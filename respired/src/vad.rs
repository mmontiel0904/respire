//! Energy-based voice activity detection with an adaptive noise floor.
//!
//! Deliberately simple: a frame counts as voice when its level is clearly
//! above the tracked background noise for a few frames in a row. The
//! `Vad` trait leaves room for a model-based detector (WebRTC VAD, Silero)
//! later without touching the rest of the daemon.

pub trait Vad {
    /// Feed one frame's level in dBFS; returns true if the frame is voice.
    fn process(&mut self, level_db: f32) -> bool;
    fn set_sensitivity(&mut self, sensitivity: f64);
}

/// Frames below this level are never voice, whatever the noise floor says.
const ABSOLUTE_MIN_DB: f32 = -60.0;
/// Consecutive loud frames needed before reporting voice (debounces clicks and taps).
const ONSET_FRAMES: u32 = 2;

pub struct EnergyVad {
    noise_floor_db: Option<f32>,
    margin_db: f32,
    loud_run: u32,
}

impl EnergyVad {
    pub fn new(sensitivity: f64) -> Self {
        let mut vad = Self {
            noise_floor_db: None,
            margin_db: 0.0,
            loud_run: 0,
        };
        vad.set_sensitivity(sensitivity);
        vad
    }

    pub fn noise_floor_db(&self) -> Option<f32> {
        self.noise_floor_db
    }
}

impl Vad for EnergyVad {
    fn process(&mut self, level_db: f32) -> bool {
        let floor = *self.noise_floor_db.get_or_insert(level_db);
        let loud = level_db > (floor + self.margin_db).max(ABSOLUTE_MIN_DB);

        // Track the floor: fall quickly towards quieter frames, rise slowly
        // through louder ones. While loud, rise very slowly so a new constant
        // noise source (fan, AC) is eventually absorbed, but a long
        // monologue is not.
        let rate = if !loud && level_db < floor {
            0.2
        } else if !loud {
            0.02
        } else {
            0.0002
        };
        self.noise_floor_db = Some(floor + (level_db - floor) * rate);

        self.loud_run = if loud { self.loud_run + 1 } else { 0 };
        self.loud_run >= ONSET_FRAMES
    }

    fn set_sensitivity(&mut self, sensitivity: f64) {
        // 0.0 → 20 dB above the floor, 1.0 → 4 dB above the floor.
        self.margin_db = 20.0 - 16.0 * sensitivity.clamp(0.0, 1.0) as f32;
    }
}

/// RMS level of 16-bit PCM samples in dBFS (full scale sine ≈ -3 dB).
pub fn level_dbfs(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return -100.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let rms = (sum_sq / samples.len() as f64).sqrt() / 32768.0;
    if rms <= 1e-5 {
        -100.0
    } else {
        (20.0 * rms.log10()) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(vad: &mut EnergyVad, level: f32, frames: usize) -> Vec<bool> {
        (0..frames).map(|_| vad.process(level)).collect()
    }

    #[test]
    fn silence_is_not_voice() {
        let mut vad = EnergyVad::new(0.5);
        assert!(feed(&mut vad, -55.0, 50).iter().all(|v| !v));
    }

    #[test]
    fn speech_over_noise_is_voice_after_onset() {
        let mut vad = EnergyVad::new(0.5);
        feed(&mut vad, -55.0, 30);
        let out = feed(&mut vad, -25.0, 5);
        assert_eq!(out, vec![false, true, true, true, true]);
    }

    #[test]
    fn single_click_is_ignored() {
        let mut vad = EnergyVad::new(0.5);
        feed(&mut vad, -55.0, 30);
        assert!(!vad.process(-20.0));
        assert!(!vad.process(-55.0));
    }

    #[test]
    fn quiet_absolute_levels_are_never_voice() {
        let mut vad = EnergyVad::new(1.0);
        feed(&mut vad, -90.0, 30);
        assert!(feed(&mut vad, -70.0, 10).iter().all(|v| !v));
    }

    #[test]
    fn higher_sensitivity_catches_softer_speech() {
        let mut strict = EnergyVad::new(0.0);
        let mut loose = EnergyVad::new(1.0);
        feed(&mut strict, -50.0, 30);
        feed(&mut loose, -50.0, 30);
        assert!(!feed(&mut strict, -40.0, 5).iter().any(|&v| v));
        assert!(feed(&mut loose, -40.0, 5).iter().any(|&v| v));
    }

    #[test]
    fn long_speech_stays_voice() {
        let mut vad = EnergyVad::new(0.5);
        feed(&mut vad, -55.0, 30);
        // Two minutes of continuous speech at 10 frames per second.
        assert!(feed(&mut vad, -28.0, 1200).iter().skip(1).all(|&v| v));
    }

    #[test]
    fn level_of_full_scale_square_is_zero_db() {
        let samples = vec![i16::MAX, i16::MIN, i16::MAX, i16::MIN];
        assert!(level_dbfs(&samples).abs() < 0.01);
        assert_eq!(level_dbfs(&[0; 160]), -100.0);
    }
}
