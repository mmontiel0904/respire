# Project Specification: **Respire** 
*(GNOME Shell Extension & Rust Audio Daemon)*

**Respire** (from *respiro* - to take a breather) is a minimalist, hybrid-architecture tool tailored for Fedora. It monitors continuous speech via PipeWire and displays dynamic visual feedback directly in the GNOME top status bar. Its primary goal is to help users improve their meeting etiquette by alerting them when they have talked too long without yielding the floor.

To ensure zero desktop stuttering and maximum stability, Respire is split into two components: a highly efficient background **Rust Daemon** for audio processing, and a lightweight **GNOME Shell Extension** for the UI.

---

## 1. System Architecture (Hybrid Rust + GJS)

By offloading audio buffer processing to Rust, the GNOME Shell JavaScript thread remains completely free of heavy computations.

```text
+-----------------------------------------------------------------+
|                       GNOME Shell Top Bar                       |
|   [Wi-Fi]  [Volume]  [  Respire Icon (D-Bus Consumer / GJS)  ]   |
+--------------------------------+--------------------------------+
                                 | D-Bus IPC (org.gnome.Respire)
+--------------------------------v--------------------------------+
|                       respired (Rust Daemon)                    |
|  - pipewire / cpal   (PipeWire Audio Stream Listener)           |
|  - MicroVAD / RMS    (Voice Activity Detection)                 |
|  - zbus              (D-Bus IPC Server)                         |
|  - Timer Logic       (State Machine: Idle -> Talk -> Alert)     |
+-----------------------------------------------------------------+
```

---

## 2. Top-Bar Indicator & Animation Specification

The indicator lives directly in the GNOME Shell aggregate area (top-right corner). Visual updates are triggered purely by D-Bus signals.

| State | Top-Bar Icon Appearance | Animation / Visual Behavior |
| :--- | :--- | :--- |
| **Idle / Listening** | Grayed-out mic icon (`respire-idle`) | Static icon. Monitors input levels in background. |
| **Active Speech** | Colored mic with dynamic progress ring | The outer ring fills clockwise as continuous speaking time accrues (0% to 75%). |
| **Warning Threshold** | Amber mic icon + pulsing ring | Gentle breathing/pulse animation every 1.5 seconds to subtly warn the speaker (75% to 99%). |
| **Limit Exceeded** | Flashing Red/Amber mic badge | Active flashing animation (1 Hz rate). Displays a system notification toast: *"Tómate un respiro: time to pause for questions!"* |

---

## 3. D-Bus Interface Specification

The Rust daemon exposes the `org.gnome.Respire` service over the Session Bus, acting as the single source of truth.

### **Properties (Readable by Extension)**
* `State` (`s`): `"idle"`, `"speaking"`, `"warning"`, or `"alert"`.
* `TalkingTime` (`u`): Continuous speech duration in seconds.
* `TimeLimit` (`u`): Configured limit before triggering alert (default: 90).
* `Progress` (`d`): Ratio calculated as $\frac{\text{TalkingTime}}{\text{TimeLimit}}$ (ranges from $0.0$ to $1.0$).

### **Signals (Emitted to Extension)**
* `StateChanged(s state)`: Emitted instantly when the state shifts.
* `Tick(u seconds, d progress)`: Emitted every second during active speech to update top-bar ring animations smoothly.

### **Methods (Callable from Extension Popover)**
* `SetLimit(u seconds)`: Update maximum speaking time.
* `Snooze(u minutes)`: Pause voice detection temporarily.

---

## 4. Core Rust Logic Flow

The Rust daemon captures small audio frames (e.g., 100ms) and processes the RMS (Root Mean Square) volume against a calibrated noise floor.

```rust
// Conceptual State Machine
pub struct SpeechTracker {
    silence_threshold_rms: f32, // Calibrated noise floor
    silence_reset_sec: f32,     // Default: 3.0 seconds
    speech_limit_sec: u32,      // Default: 90 seconds
    current_talk_time: f32,
    current_silence_time: f32,
}

impl SpeechTracker {
    pub fn process_audio_frame(&mut self, rms: f32) -> State {
        if rms > self.silence_threshold_rms {
            self.current_silence_time = 0.0;
            self.current_talk_time += 0.1; 
        } else {
            self.current_silence_time += 0.1;
            if self.current_silence_time >= self.silence_reset_sec {
                self.current_talk_time = 0.0; // Reset after enough silence
            }
        }

        // Return current state based on thresholds
        match self.current_talk_time {
            t if t >= self.speech_limit_sec as f32 => State::Alert,
            t if t >= (self.speech_limit_sec as f32 * 0.75) => State::Warning,
            t if t > 0.0 => State::Speaking,
            _ => State::Idle,
        }
    }
}
```

---

## 5. UI Interactions (GNOME Popover Menu)

Clicking the top-bar icon opens a native GNOME Quick-Settings popover, which communicates with the Rust daemon via D-Bus method calls:

* **Current Talking Time:** Real-time counter (e.g., `00:42 / 01:30`).
* **Threshold Quick-Select:** Segmented buttons (`45s`, `60s`, `90s`, `120s`).
* **Snooze / Mute Button:** Pause tracking for 15m, 30m, or for the current meeting.
* **Sensitivity Slider:** Adjust microphone activation threshold (updates `silence_threshold_rms` in Rust).

---

## 6. Project Directory Layout

```text
respire/
├── respired/                     # Rust Daemon (Backend)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs               # Async entrypoint & Tokio loop
│       ├── audio.rs              # PipeWire stream & RMS VAD logic
│       ├── dbus.rs               # zbus implementation
│       └── config.rs             # Config loading (~/.config/respire.toml)
├── extension/                    # GNOME Shell Extension (Frontend)
│   ├── metadata.json
│   ├── extension.js              # Ultra-thin D-Bus client & UI rendering
│   └── stylesheet.css
└── systemd/
    └── respired.service          # User systemd service unit for the daemon
```

---

## 7. Minimal `metadata.json` (Extension)

```json
{
  "uuid": "respire@yourdomain.com",
  "name": "Respire",
  "description": "Un respiro para tus reuniones. Monitors microphone activity and alerts you when you talk continuously without pausing.",
  "shell-version": ["45", "46", "47"],
  "url": "https://github.com/yourusername/respire"
}
```