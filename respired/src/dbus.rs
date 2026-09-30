//! D-Bus interface `io.github.mmontiel0904.Respire1` on the session bus.
//!
//! The daemon is the single source of truth. Clients read properties and
//! listen to `org.freedesktop.DBus.Properties.PropertiesChanged`; while you
//! talk, `TalkingTime` and `Progress` change once per second.

use tokio::sync::mpsc;
use zbus::fdo;
use zbus::interface;
use zbus::object_server::{InterfaceRef, SignalEmitter};

use crate::config::{MAX_LIMIT_SEC, MIN_LIMIT_SEC};

pub const BUS_NAME: &str = "io.github.mmontiel0904.Respire";
pub const OBJECT_PATH: &str = "/io/github/mmontiel0904/Respire";

#[derive(Debug)]
pub enum Command {
    SetLimit(u32),
    SetSensitivity(f64),
    /// Minutes; 0 means "until the current meeting ends".
    Snooze(u32),
    Resume,
}

/// Everything the daemon publishes. The engine replaces it and the
/// differences are emitted as property changes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    /// "standby", "snoozed", "idle", "speaking", "warning" or "alert".
    pub state: String,
    pub talking_time: u32,
    pub time_limit: u32,
    pub progress: f64,
    pub sensitivity: f64,
    pub meeting_apps: Vec<String>,
    /// Seconds left on a timed snooze; 0 when not snoozed or snoozed until the meeting ends.
    pub snooze_remaining: u32,
    pub turn_count: u32,
    pub longest_turn: u32,
    pub alert_count: u32,
    /// Your share of the talk time in this call, 0.0 to 1.0; -1.0 when unknown.
    pub talk_share: f64,
    pub interruption_count: u32,
}

pub struct Respire {
    pub snapshot: Snapshot,
    commands: mpsc::UnboundedSender<Command>,
}

impl Respire {
    pub fn new(snapshot: Snapshot, commands: mpsc::UnboundedSender<Command>) -> Self {
        Self { snapshot, commands }
    }

    fn send(&self, cmd: Command) -> fdo::Result<()> {
        self.commands
            .send(cmd)
            .map_err(|_| fdo::Error::Failed("daemon is shutting down".into()))
    }
}

#[interface(name = "io.github.mmontiel0904.Respire1")]
impl Respire {
    /// Maximum continuous talking time in seconds.
    async fn set_limit(&self, seconds: u32) -> fdo::Result<()> {
        if !(MIN_LIMIT_SEC..=MAX_LIMIT_SEC).contains(&seconds) {
            return Err(fdo::Error::InvalidArgs(format!(
                "limit must be between {MIN_LIMIT_SEC} and {MAX_LIMIT_SEC} seconds"
            )));
        }
        self.send(Command::SetLimit(seconds))
    }

    /// 0.0 (only loud speech counts) .. 1.0 (quiet speech counts).
    async fn set_sensitivity(&self, value: f64) -> fdo::Result<()> {
        if !(0.0..=1.0).contains(&value) {
            return Err(fdo::Error::InvalidArgs(
                "sensitivity must be between 0 and 1".into(),
            ));
        }
        self.send(Command::SetSensitivity(value))
    }

    /// Stop listening for `minutes`, or until the current meeting ends when 0.
    async fn snooze(&self, minutes: u32) -> fdo::Result<()> {
        self.send(Command::Snooze(minutes))
    }

    async fn resume(&self) -> fdo::Result<()> {
        self.send(Command::Resume)
    }

    #[zbus(property)]
    fn state(&self) -> String {
        self.snapshot.state.clone()
    }

    #[zbus(property)]
    fn talking_time(&self) -> u32 {
        self.snapshot.talking_time
    }

    #[zbus(property)]
    fn time_limit(&self) -> u32 {
        self.snapshot.time_limit
    }

    #[zbus(property)]
    fn progress(&self) -> f64 {
        self.snapshot.progress
    }

    #[zbus(property)]
    fn sensitivity(&self) -> f64 {
        self.snapshot.sensitivity
    }

    #[zbus(property)]
    fn meeting_apps(&self) -> Vec<String> {
        self.snapshot.meeting_apps.clone()
    }

    #[zbus(property)]
    fn snooze_remaining(&self) -> u32 {
        self.snapshot.snooze_remaining
    }

    #[zbus(property)]
    fn turn_count(&self) -> u32 {
        self.snapshot.turn_count
    }

    #[zbus(property)]
    fn longest_turn(&self) -> u32 {
        self.snapshot.longest_turn
    }

    #[zbus(property)]
    fn alert_count(&self) -> u32 {
        self.snapshot.alert_count
    }

    #[zbus(property)]
    fn talk_share(&self) -> f64 {
        self.snapshot.talk_share
    }

    #[zbus(property)]
    fn interruption_count(&self) -> u32 {
        self.snapshot.interruption_count
    }

    /// Emitted when a call ends. `summary` is the JSON line also written to
    /// `sessions.jsonl` (even when `log_sessions` is off).
    #[zbus(signal)]
    pub async fn session_finished(emitter: &SignalEmitter<'_>, summary: &str) -> zbus::Result<()>;
}

/// Replace the published snapshot and emit PropertiesChanged for what differs.
pub async fn publish(iface: &InterfaceRef<Respire>, new: Snapshot) -> zbus::Result<()> {
    let old = {
        let mut guard = iface.get_mut().await;
        if guard.snapshot == new {
            return Ok(());
        }
        std::mem::replace(&mut guard.snapshot, new.clone())
    };
    let guard = iface.get().await;
    let e: &SignalEmitter<'_> = iface.signal_emitter();
    if old.state != new.state {
        guard.state_changed(e).await?;
    }
    if old.talking_time != new.talking_time {
        guard.talking_time_changed(e).await?;
    }
    if old.time_limit != new.time_limit {
        guard.time_limit_changed(e).await?;
    }
    if old.progress != new.progress {
        guard.progress_changed(e).await?;
    }
    if old.sensitivity != new.sensitivity {
        guard.sensitivity_changed(e).await?;
    }
    if old.meeting_apps != new.meeting_apps {
        guard.meeting_apps_changed(e).await?;
    }
    if old.snooze_remaining != new.snooze_remaining {
        guard.snooze_remaining_changed(e).await?;
    }
    if old.turn_count != new.turn_count {
        guard.turn_count_changed(e).await?;
    }
    if old.longest_turn != new.longest_turn {
        guard.longest_turn_changed(e).await?;
    }
    if old.alert_count != new.alert_count {
        guard.alert_count_changed(e).await?;
    }
    if old.talk_share != new.talk_share {
        guard.talk_share_changed(e).await?;
    }
    if old.interruption_count != new.interruption_count {
        guard.interruption_count_changed(e).await?;
    }
    Ok(())
}
