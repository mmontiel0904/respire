//! respired: listens to your microphone during calls and tells the GNOME
//! Shell extension how long you have been holding the floor.
//!
//! Tasks:
//! - `meeting::watch_meetings` polls PipeWire for other apps capturing the mic.
//! - `audio::capture` runs `pw-record` while the engine wants audio: once for
//!   the microphone and, with `track_others`, once for the speaker output.
//! - the main loop (the engine) turns levels into turns and publishes over D-Bus.
//!
//! Audio is processed in memory, 100 ms at a time, and never stored.

mod audio;
mod config;
mod dbus;
mod meeting;
mod summary;
mod tracker;
mod vad;

use std::collections::BTreeSet;
use std::io::Write as _;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{mpsc, watch};

use audio::CaptureTarget;
use config::{Config, Mode};
use dbus::{BUS_NAME, Command, OBJECT_PATH, Respire, Snapshot};
use meeting::MeetingInfo;
use summary::SessionSummary;
use tracker::{Limits, Tracker};
use vad::{EnergyVad, Vad};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Snooze {
    Off,
    Until(Instant),
    UntilMeetingEnds,
}

struct Session {
    started: SystemTime,
    apps: BTreeSet<String>,
}

struct Engine {
    cfg: Config,
    vad: EnergyVad,
    tracker: Tracker,
    /// The other people in the call, heard through the speaker output.
    others_vad: EnergyVad,
    others: Tracker,
    others_ms: u64,
    meeting: MeetingInfo,
    snooze: Snooze,
    session: Option<Session>,
    audio_ms: u64,
    last_level_db: f32,
    want_capture: watch::Sender<bool>,
    /// Summary of a session that just ended, waiting to be signalled.
    finished: Option<String>,
}

fn limits(cfg: &Config) -> Limits {
    Limits {
        limit_ms: cfg.time_limit_sec as u64 * 1000,
        warning_ratio: cfg.warning_ratio,
        silence_reset_ms: (cfg.silence_reset_sec * 1000.0) as u64,
    }
}

/// Others hold the floor until 1.5 s of silence. They have no limit.
fn others_limits() -> Limits {
    Limits {
        limit_ms: u64::MAX,
        warning_ratio: 1.0,
        silence_reset_ms: 1_500,
    }
}

fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Engine {
    fn new(cfg: Config, want_capture: watch::Sender<bool>) -> Self {
        Self {
            vad: EnergyVad::new(cfg.sensitivity),
            tracker: Tracker::new(limits(&cfg)),
            others_vad: EnergyVad::new(0.5),
            others: Tracker::new(others_limits()),
            others_ms: 0,
            cfg,
            meeting: MeetingInfo::default(),
            snooze: Snooze::Off,
            session: None,
            audio_ms: 0,
            last_level_db: -100.0,
            want_capture,
            finished: None,
        }
    }

    fn session_wanted(&self) -> bool {
        self.cfg.mode == Mode::Always || self.meeting.active()
    }

    fn capturing(&self) -> bool {
        self.session.is_some() && self.snooze == Snooze::Off
    }

    /// Start or finish the session and capture to match the current conditions.
    fn sync(&mut self) {
        match (self.session.is_some(), self.session_wanted()) {
            (false, true) => {
                self.tracker.reset_session();
                self.others.reset_session();
                self.session = Some(Session {
                    started: SystemTime::now(),
                    apps: BTreeSet::new(),
                });
            }
            (true, false) => {
                self.finished = self.finish_session();
                if self.snooze == Snooze::UntilMeetingEnds {
                    self.snooze = Snooze::Off;
                }
            }
            _ => {}
        }
        if let Some(session) = &mut self.session {
            session.apps.extend(self.meeting.apps.iter().cloned());
        }
        let capturing = self.capturing();
        if !capturing {
            self.tracker.end_turn();
            self.others.end_turn();
        }
        self.want_capture
            .send_if_modified(|want| std::mem::replace(want, capturing) != capturing);
    }

    /// End the session, log it, and return its summary as JSON.
    /// Returns `None` when there is no session or you did not speak.
    fn finish_session(&mut self) -> Option<String> {
        let session = self.session.take()?;
        let mine = self.tracker.reset_session();
        let others = self.others.reset_session();
        if mine.turns == 0 {
            return None;
        }
        let summary = SessionSummary::new(
            unix_secs(session.started),
            unix_secs(SystemTime::now()),
            session.apps,
            self.cfg.time_limit_sec,
            &mine,
            self.cfg.track_others.then_some(&others),
        );
        let line = match serde_json::to_string(&summary) {
            Ok(line) => line,
            Err(e) => {
                log::warn!("cannot serialize session summary: {e}");
                return None;
            }
        };
        if self.cfg.log_sessions
            && let Err(e) = append_line(&config::sessions_log_path(), &line)
        {
            log::warn!("cannot write session log: {e:#}");
        }
        Some(line)
    }

    fn on_level(&mut self, level_db: f32) {
        if !self.capturing() {
            return;
        }
        self.last_level_db = level_db;
        let voiced = self.vad.process(level_db);
        self.audio_ms += audio::FRAME_MS;
        self.tracker.set_others_talking(self.others.in_turn());
        self.tracker.update(self.audio_ms, voiced);
    }

    fn on_others_level(&mut self, level_db: f32) {
        if !self.capturing() {
            return;
        }
        let voiced = self.others_vad.process(level_db);
        self.others_ms += audio::FRAME_MS;
        self.others.update(self.others_ms, voiced);
    }

    fn on_meeting(&mut self, info: MeetingInfo) {
        self.meeting = info;
        self.sync();
    }

    fn on_tick(&mut self) {
        if let Snooze::Until(t) = self.snooze
            && Instant::now() >= t
        {
            log::info!("snooze over");
            self.snooze = Snooze::Off;
        }
        self.sync();
        if self.capturing() {
            log::debug!(
                "level {:.1} dBFS, noise floor {:.1} dBFS, state {}",
                self.last_level_db,
                self.vad.noise_floor_db().unwrap_or(f32::NAN),
                self.tracker.state().as_str()
            );
        }
    }

    fn on_command(&mut self, cmd: Command) {
        log::info!("command: {cmd:?}");
        match cmd {
            Command::SetLimit(seconds) => {
                self.cfg.time_limit_sec = seconds;
                self.tracker.set_limits(limits(&self.cfg));
                self.save_config();
            }
            Command::SetSensitivity(value) => {
                self.cfg.sensitivity = value;
                self.vad.set_sensitivity(value);
                self.save_config();
            }
            // Nothing to snooze until a call starts.
            Command::Snooze(0) if self.session.is_none() => {}
            Command::Snooze(0) => self.snooze = Snooze::UntilMeetingEnds,
            Command::Snooze(minutes) => {
                self.snooze =
                    Snooze::Until(Instant::now() + Duration::from_secs(minutes as u64 * 60));
            }
            Command::Resume => self.snooze = Snooze::Off,
        }
        self.sync();
    }

    fn save_config(&self) {
        if let Err(e) = self.cfg.save() {
            log::warn!("cannot save config: {e:#}");
        }
    }

    fn snapshot(&self) -> Snapshot {
        let state = if self.session.is_none() {
            "standby"
        } else if self.snooze != Snooze::Off {
            "snoozed"
        } else {
            self.tracker.state().as_str()
        };
        let talking_time = (self.tracker.talking_ms() / 1000) as u32;
        let stats = self.tracker.stats();
        let talk_share = if self.cfg.track_others && self.session.is_some() {
            summary::talk_share(self.tracker.total_talk_ms(), self.others.total_talk_ms())
        } else {
            None
        };
        Snapshot {
            state: state.to_string(),
            talking_time,
            time_limit: self.cfg.time_limit_sec,
            progress: (talking_time as f64 / self.cfg.time_limit_sec as f64).min(1.0),
            sensitivity: self.cfg.sensitivity,
            meeting_apps: self.meeting.apps.clone(),
            snooze_remaining: match self.snooze {
                Snooze::Until(t) => t.saturating_duration_since(Instant::now()).as_secs() as u32,
                _ => 0,
            },
            turn_count: stats.turns,
            longest_turn: (stats.longest_turn_ms / 1000) as u32,
            alert_count: stats.alerts,
            talk_share: talk_share.unwrap_or(-1.0),
            interruption_count: stats.interruptions,
        }
    }
}

fn append_line(path: &std::path::Path, line: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")?;
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cfg = Config::load()?;
    log::info!("config: {cfg:?}");

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
    let (want_tx, want_rx) = watch::channel(false);
    let (level_tx, mut level_rx) = mpsc::channel(64);
    let (others_tx, mut others_rx) = mpsc::channel(64);
    let (meeting_tx, mut meeting_rx) = watch::channel(MeetingInfo::default());

    let mut engine = Engine::new(cfg.clone(), want_tx);
    engine.sync();

    let conn = zbus::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, Respire::new(engine.snapshot(), cmd_tx))?
        .build()
        .await
        .context("connecting to the session bus (is another respired running?)")?;
    let iface = conn
        .object_server()
        .interface::<_, Respire>(OBJECT_PATH)
        .await?;
    log::info!("serving {BUS_NAME} at {OBJECT_PATH}");

    tokio::spawn(audio::capture(
        CaptureTarget::mic(cfg.source.clone()),
        want_rx.clone(),
        level_tx,
    ));
    if cfg.track_others {
        tokio::spawn(audio::capture(CaptureTarget::others(), want_rx, others_tx));
    } else {
        drop(others_tx);
    }
    tokio::spawn(meeting::watch_meetings(cfg.ignore_apps.clone(), meeting_tx));

    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    loop {
        tokio::select! {
            Some(level) = level_rx.recv() => engine.on_level(level),
            Some(level) = others_rx.recv() => engine.on_others_level(level),
            Ok(()) = meeting_rx.changed() => {
                let info = meeting_rx.borrow_and_update().clone();
                engine.on_meeting(info);
            }
            Some(cmd) = cmd_rx.recv() => engine.on_command(cmd),
            _ = tick.tick() => engine.on_tick(),
            _ = sigterm.recv() => break,
            _ = sigint.recv() => break,
        }
        if let Some(summary) = engine.finished.take()
            && let Err(e) = Respire::session_finished(iface.signal_emitter(), &summary).await
        {
            log::warn!("cannot signal session summary: {e}");
        }
        if let Err(e) = dbus::publish(&iface, engine.snapshot()).await {
            log::warn!("cannot publish state: {e}");
        }
    }

    log::info!("shutting down");
    engine.finish_session();
    Ok(())
}
