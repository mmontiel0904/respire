//! Microphone capture through `pw-record`.
//!
//! Running `pw-record` as a child process keeps the daemon free of native
//! build dependencies (no libpipewire headers, no bindgen). It delivers
//! 16 kHz mono s16 PCM on stdout; each 100 ms frame becomes one level
//! reading. Capture only runs while the engine asks for it.
//!
//! There are two captures: the microphone (you) and the monitor of the
//! default sink (the other people in the call).

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, watch};

use crate::vad::level_dbfs;

/// Prefix of every node Respire creates. Meeting detection skips these.
pub const OWN_NODE_NAME: &str = "respired-capture";
pub const REMOTE_NODE_NAME: &str = "respired-capture-remote";
pub const SAMPLE_RATE: u32 = 16_000;
pub const FRAME_MS: u64 = 100;
const FRAME_SAMPLES: usize = (SAMPLE_RATE as u64 * FRAME_MS / 1000) as usize;
const FRAME_BYTES: usize = FRAME_SAMPLES * 2;
const RESTART_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct CaptureTarget {
    pub node_name: &'static str,
    pub media_name: &'static str,
    /// PipeWire node to record from. Empty means the default.
    pub source: String,
    /// Record the monitor of the default sink instead of a source.
    pub capture_sink: bool,
}

impl CaptureTarget {
    pub fn mic(source: String) -> Self {
        Self {
            node_name: OWN_NODE_NAME,
            media_name: "Respire voice monitor",
            source,
            capture_sink: false,
        }
    }

    pub fn others() -> Self {
        Self {
            node_name: REMOTE_NODE_NAME,
            media_name: "Respire call monitor",
            source: String::new(),
            capture_sink: true,
        }
    }
}

/// Runs forever: starts `pw-record` while `want` is true, stops it when false,
/// and sends one dBFS level per 100 ms frame to `levels`.
pub async fn capture(
    target: CaptureTarget,
    mut want: watch::Receiver<bool>,
    levels: mpsc::Sender<f32>,
) {
    loop {
        while !*want.borrow_and_update() {
            if want.changed().await.is_err() {
                return;
            }
        }

        let mut child = match spawn(&target) {
            Ok(child) => child,
            Err(e) => {
                log::error!("cannot start pw-record (install pipewire-utils): {e}");
                tokio::time::sleep(RESTART_DELAY).await;
                continue;
            }
        };
        log::info!("capture started: {}", target.node_name);
        let mut stdout = child.stdout.take().expect("stdout is piped");
        let mut pending: Vec<u8> = Vec::with_capacity(FRAME_BYTES * 2);
        let mut chunk = [0u8; 4096];

        let stopped_on_request = loop {
            tokio::select! {
                // `read` is cancel safe; partial frames stay in `pending`.
                read = stdout.read(&mut chunk) => match read {
                    Ok(0) | Err(_) => break false,
                    Ok(n) => {
                        pending.extend_from_slice(&chunk[..n]);
                        while pending.len() >= FRAME_BYTES {
                            let samples: Vec<i16> = pending[..FRAME_BYTES]
                                .as_chunks::<2>()
                                .0
                                .iter()
                                .map(|&b| i16::from_le_bytes(b))
                                .collect();
                            pending.drain(..FRAME_BYTES);
                            if levels.send(level_dbfs(&samples)).await.is_err() {
                                return;
                            }
                        }
                    }
                },
                changed = want.changed() => {
                    if changed.is_err() { return; }
                    if !*want.borrow_and_update() { break true; }
                }
            }
        };

        let _ = child.kill().await;
        if stopped_on_request {
            log::info!("capture stopped: {}", target.node_name);
        } else {
            log::warn!(
                "pw-record ({}) exited unexpectedly; restarting",
                target.node_name
            );
            tokio::time::sleep(RESTART_DELAY).await;
        }
    }
}

fn spawn(target: &CaptureTarget) -> std::io::Result<Child> {
    let mut cmd = Command::new("pw-record");
    cmd.args([
        "--rate",
        &SAMPLE_RATE.to_string(),
        "--channels",
        "1",
        "--format",
        "s16",
    ])
    .args(["--latency", &format!("{FRAME_MS}ms")])
    .args([
        "-P",
        &format!(
            "{{ node.name = {} application.name = Respire media.name = \"{}\" \
                 stream.capture.sink = {} }}",
            target.node_name, target.media_name, target.capture_sink
        ),
    ]);
    if !target.source.is_empty() {
        cmd.args(["--target", &target.source]);
    }
    cmd.arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
}
