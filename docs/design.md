# Design notes

The first spec is in [spec-draft.md](spec-draft.md). This document records
where the implementation differs from it, and why.

## Viability

The idea is viable on Fedora 44 with GNOME 50, and the MVP in this repository
works end to end:

- `pw-record` delivers 16 kHz mono PCM from the default source without extra
  setup.
- `pw-dump` shows the capture streams of other applications, so the daemon can
  tell when a call is in progress.
- A zbus service on the session bus is visible to a GJS `Gio.DBusProxy`. Its
  property changes arrive as `g-properties-changed`.
- The extension loads cleanly in GNOME Shell 50.5 (`make shell-test`), and all
  six states render as expected.

The most important risk is accuracy, not feasibility. See "Known limitations"
in the README.

## Changes from the draft

| Draft | Now | Why |
| --- | --- | --- |
| Bus name `org.gnome.Respire` | `io.github.mmontiel0904.Respire` | The `org.gnome` namespace belongs to GNOME projects, and extensions.gnome.org reviewers flag it. Use a domain you control. |
| Always listening | Listens only while another app captures the mic (`mode = "meeting"`) | A permanent capture stream keeps GNOME's microphone privacy indicator lit all day, which trains you to ignore it. Gating on calls also saves CPU, avoids alerts when you talk to someone in the room, and gives a natural unit (one call) for statistics. |
| `pipewire` or `cpal` crates | `pw-record` child process | The `pipewire` crate needs `pipewire-devel` and clang for bindgen. `cpal` goes through ALSA and cannot see the PipeWire graph that meeting detection needs. With a child process the build is pure Rust. The capture code can move to native bindings later without affecting the engine. |
| "MicroVAD / RMS" | Energy VAD with adaptive noise floor and 2-frame onset, behind a `Vad` trait | No crate is called MicroVAD. RMS against a fixed threshold fails when room noise changes. The trait lets us add WebRTC VAD or Silero later. |
| `current_talk_time += 0.1` per voiced frame | Turn wall time from first voice to now; turn ends after `silence_reset_sec` of silence | Speech is full of short gaps. Counting only voiced frames undercounts how long you have held the floor, and the error depends on how fast you talk. Time is an integer millisecond counter driven by audio frames, so it cannot drift and does not depend on pipe buffering. |
| `Tick(u, d)` signal each second | `PropertiesChanged` on `TalkingTime` and `Progress` | Same traffic, one mechanism. GJS proxies handle property changes natively, and D-Bus tools like `busctl monitor` show them. |
| 4 states | Adds `standby` (no call) and `snoozed` | The indicator must show whether the daemon is listening at all. |
| Quick Settings popover | `PanelMenu.Button` with its own menu | Quick Settings is the shared system menu with toggle tiles. A standalone indicator can draw the ring and have its own menu. |
| `shell-version` 45–47 | 48–50 | Fedora 44 ships GNOME 50. Only 50 is tested. The ESM APIs in use have been stable since 45. |
| `~/.config/respire.toml` | `~/.config/respire/config.toml`, and `~/.local/state/respire/sessions.jsonl` | Follows XDG. The per-call log is what turns a nag into feedback you can learn from. |
| Plain systemd unit | `Type=dbus` unit, D-Bus activation, `RestrictAddressFamilies=AF_UNIX` | The extension starts the daemon on demand. The sandbox makes the privacy claim something you can check. |

## Metrics and history

- **Talk share uses the speaker output.** A second `pw-record` captures the
  default sink's monitor (`stream.capture.sink = true`) during calls. The same
  energy VAD and turn tracker run on it, with a 1.5 s silence reset and no
  limit. Your share is your talk time divided by everyone's. A sink monitor
  does not light the microphone indicator. `track_others = false` turns it off.
- **Interruptions need a minimum length.** A turn that you start while the
  others are mid-turn counts only if it lasts 1.5 s or more. Otherwise every
  "yeah" or "right" would count.
- **Per-call summaries only.** The log keeps one line per call (version 2 adds
  `median_turn_sec`, `others_talk_sec`, `talk_share`, `interruptions`). A
  per-turn timeline would add detail that nobody reviews, and a bigger log.
- **The window reads the log file.** GNOME runs extension preferences in a
  separate process, so `prefs.js` reads `sessions.jsonl` and watches it with
  a file monitor. The daemon needs no history API. `history.js` holds the
  parsing and aggregation. It has no GNOME imports, so it is tested under gjs
  and node.
- **Talk share has no "better" direction.** The right share depends on how
  many people are in the call, which Respire does not know. The Progress page
  marks lower median turn, alerts per hour and interruptions per hour as
  better, and shows talk share without a verdict.
- **Post-call notification.** Calls shorter than 2 minutes, or with fewer than
  2 turns, get no notification, so short tests and dictation do not spam you.

## Roadmap

Ordered by value for the goal ("be a better communicator"):

1. ~~**Talk share.**~~ Done: speaker-output capture, live `TalkShare`, and a
   field in the log.
2. ~~**Interruptions.**~~ Done: `InterruptionCount`, and a field in the log.
3. ~~**`respire stats`.**~~ Replaced by the History & progress window.
4. **Better VAD.** Silero through `ort`, or WebRTC VAD, to reject typing and
   background talkers.
5. **Per-call limits.** For example, a longer limit when the call app is a
   presentation tool, and a shorter one in stand-ups.
6. **Personal goals.** Targets for median turn and alerts per hour, drawn on
   the Progress chart. Needs a GSettings schema. The same schema would allow
   a toggle for the post-call notification and a Monday weekly digest.
7. **Quiet nudge.** A gentle hint when you have not spoken for a long time in
   a call, for the opposite problem.
8. **Response gap.** How fast you start talking after the others stop, as a
   sign of rushing in.
9. **Packaging.** A COPR RPM for the daemon, and the extension on
   extensions.gnome.org. The EGO listing must say that the daemon is installed
   separately.

## Not doing

- **Transcription or filler-word detection.** Both need ASR, which changes the
  privacy story.
- **LLM coaching.** Without transcripts, an LLM can only rephrase numbers that
  the window already shows. With transcripts, it needs ASR, which breaks the
  privacy story, and a network connection, which the daemon's sandbox
  forbids. Revisit only if on-device ASR becomes cheap and you opt in.
