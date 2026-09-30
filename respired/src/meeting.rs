//! Meeting detection: "is some other application capturing the microphone?"
//!
//! Polls `pw-dump` and looks for audio capture streams. This is what
//! makes Respire listen only during calls, and it also means the GNOME
//! microphone privacy indicator is only on when it would be on anyway.

use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::watch;

use crate::audio::OWN_NODE_NAME;

const POLL_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeetingInfo {
    /// Applications currently capturing a microphone, deduplicated.
    pub apps: Vec<String>,
}

impl MeetingInfo {
    pub fn active(&self) -> bool {
        !self.apps.is_empty()
    }
}

pub async fn watch_meetings(ignore_apps: Vec<String>, tx: watch::Sender<MeetingInfo>) {
    let mut interval = tokio::time::interval(POLL_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        match poll(&ignore_apps).await {
            Ok(info) => {
                tx.send_if_modified(|cur| {
                    if *cur != info {
                        log::info!("capture streams: {:?}", info.apps);
                        *cur = info;
                        true
                    } else {
                        false
                    }
                });
            }
            Err(e) => log::warn!("meeting detection failed: {e:#}"),
        }
        if tx.is_closed() {
            return;
        }
    }
}

async fn poll(ignore_apps: &[String]) -> Result<MeetingInfo> {
    let out = Command::new("pw-dump")
        .arg("--no-colors")
        .output()
        .await
        .context("running pw-dump (install pipewire-utils)")?;
    anyhow::ensure!(out.status.success(), "pw-dump exited with {}", out.status);
    let dump: Value = serde_json::from_slice(&out.stdout).context("parsing pw-dump output")?;
    Ok(capture_apps(&dump, ignore_apps))
}

fn capture_apps(dump: &Value, ignore_apps: &[String]) -> MeetingInfo {
    let mut apps: Vec<String> = dump
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|obj| obj.pointer("/info/props"))
        .filter(|props| is_mic_capture(props, ignore_apps))
        .map(|props| {
            str_prop(props, "application.name")
                .or_else(|| str_prop(props, "node.name"))
                .unwrap_or("unknown")
                .to_string()
        })
        .collect();
    apps.sort();
    apps.dedup();
    MeetingInfo { apps }
}

fn is_mic_capture(props: &Value, ignore_apps: &[String]) -> bool {
    if str_prop(props, "media.class") != Some("Stream/Input/Audio") {
        return false;
    }
    // Level meters and desktop-audio recorders (capturing a sink monitor) are not calls.
    if bool_prop(props, "stream.monitor") || bool_prop(props, "stream.capture.sink") {
        return false;
    }
    if str_prop(props, "node.name").is_some_and(|n| n.starts_with(OWN_NODE_NAME)) {
        return false;
    }
    let names = [
        str_prop(props, "application.name"),
        str_prop(props, "application.process.binary"),
    ];
    !names.into_iter().flatten().any(|name| {
        let name = name.to_lowercase();
        ignore_apps
            .iter()
            .any(|ignored| name.contains(&ignored.to_lowercase()))
    })
}

fn str_prop<'a>(props: &'a Value, key: &str) -> Option<&'a str> {
    props.get(key).and_then(Value::as_str)
}

fn bool_prop(props: &Value, key: &str) -> bool {
    match props.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s == "true",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(props: Value) -> Value {
        json!({ "type": "PipeWire:Interface:Node", "info": { "props": props } })
    }

    #[test]
    fn finds_browser_capture_and_skips_the_rest() {
        let dump = json!([
            node(json!({ "media.class": "Audio/Source", "node.name": "alsa_input.usb" })),
            node(json!({ "media.class": "Stream/Output/Audio", "application.name": "Spotify" })),
            node(json!({ "media.class": "Stream/Input/Audio", "application.name": "Firefox" })),
            node(json!({ "media.class": "Stream/Input/Audio", "application.name": "Firefox" })),
            node(json!({ "media.class": "Stream/Input/Audio", "node.name": "respired-capture" })),
            node(
                json!({ "media.class": "Stream/Input/Audio", "node.name": "respired-capture-remote" })
            ),
            node(
                json!({ "media.class": "Stream/Input/Audio", "application.name": "PulseAudio Volume Control", "application.process.binary": "pavucontrol" })
            ),
            node(
                json!({ "media.class": "Stream/Input/Audio", "application.name": "OBS", "stream.capture.sink": true })
            ),
            node(
                json!({ "media.class": "Stream/Input/Audio", "application.name": "Meter", "stream.monitor": "true" })
            ),
            json!({ "type": "PipeWire:Interface:Core" }),
        ]);
        let info = capture_apps(&dump, &Config::default().ignore_apps);
        assert_eq!(info.apps, vec!["Firefox".to_string()]);
        assert!(info.active());
    }

    #[test]
    fn no_capture_streams_means_no_meeting() {
        let dump = json!([node(json!({ "media.class": "Audio/Source" }))]);
        assert!(!capture_apps(&dump, &[]).active());
    }

    use crate::config::Config;
}
