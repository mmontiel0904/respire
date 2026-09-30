//! Configuration loaded from `$XDG_CONFIG_HOME/respire/config.toml`.
//!
//! Every field has a default, so a missing file or a partial file is fine.
//! Values changed over D-Bus (limit, sensitivity) are written back here.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Listen only while another application is capturing the microphone
    /// (a call in the browser, Zoom, Slack huddle, ...). Recommended.
    Meeting,
    /// Listen all the time.
    Always,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Continuous talking time, in seconds, before the alert fires.
    pub time_limit_sec: u32,
    /// Fraction of the limit at which the warning state starts.
    pub warning_ratio: f64,
    /// Seconds of silence that end your turn and reset the timer.
    pub silence_reset_sec: f64,
    /// 0.0 (needs loud speech) .. 1.0 (triggers on quiet speech).
    pub sensitivity: f64,
    pub mode: Mode,
    /// PipeWire node name of the microphone to monitor. Empty means the
    /// default source. Point this at an echo-cancel source if you use speakers.
    pub source: String,
    /// Capture streams from these applications do not count as a meeting
    /// (case-insensitive substring match on the application name).
    pub ignore_apps: Vec<String>,
    /// Also listen to the speaker output (the default sink monitor) during
    /// calls, to measure how much the other people talk. This gives talk
    /// share and interruptions. The audio is processed in memory, like the mic.
    pub track_others: bool,
    /// Append a summary line per meeting to `$XDG_STATE_HOME/respire/sessions.jsonl`.
    pub log_sessions: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            time_limit_sec: 90,
            warning_ratio: 0.75,
            silence_reset_sec: 3.0,
            sensitivity: 0.5,
            mode: Mode::Meeting,
            source: String::new(),
            ignore_apps: vec![
                "pavucontrol".into(),
                "gnome-control-center".into(),
                "easyeffects".into(),
            ],
            track_others: true,
            log_sessions: true,
        }
    }
}

pub const MIN_LIMIT_SEC: u32 = 10;
pub const MAX_LIMIT_SEC: u32 = 3600;

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        let cfg = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str::<Config>(&text)
                .with_context(|| format!("parsing {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(cfg.sanitized())
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    fn sanitized(mut self) -> Self {
        self.time_limit_sec = self.time_limit_sec.clamp(MIN_LIMIT_SEC, MAX_LIMIT_SEC);
        self.warning_ratio = self.warning_ratio.clamp(0.1, 1.0);
        self.silence_reset_sec = self.silence_reset_sec.clamp(0.5, 30.0);
        self.sensitivity = self.sensitivity.clamp(0.0, 1.0);
        self
    }
}

fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback),
    }
}

pub fn config_path() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config").join("respire/config.toml")
}

pub fn sessions_log_path() -> PathBuf {
    xdg_dir("XDG_STATE_HOME", ".local/state").join("respire/sessions.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_file_uses_defaults() {
        let cfg: Config = toml::from_str("time_limit_sec = 60\nmode = \"always\"").unwrap();
        assert_eq!(cfg.time_limit_sec, 60);
        assert_eq!(cfg.mode, Mode::Always);
        assert_eq!(cfg.silence_reset_sec, 3.0);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let cfg: Config = toml::from_str("time_limit_sec = 1\nsensitivity = 7.0").unwrap();
        let cfg = cfg.sanitized();
        assert_eq!(cfg.time_limit_sec, MIN_LIMIT_SEC);
        assert_eq!(cfg.sensitivity, 1.0);
    }

    #[test]
    fn round_trips_through_toml() {
        let cfg = Config::default();
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(cfg, back);
    }
}
