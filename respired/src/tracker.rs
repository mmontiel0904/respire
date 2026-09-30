//! Turn tracking: turns voice/no-voice frames into "how long have I been
//! holding the floor".
//!
//! A turn starts on the first voiced frame and ends after `silence_reset`
//! of continuous silence. Short pauses between words and sentences stay
//! inside the turn, so the timer measures wall-clock time holding the
//! floor, not only the frames with sound in them.
//!
//! Time is passed in explicitly (milliseconds of audio processed), which
//! keeps this module pure and easy to test.
//!
//! The same tracker also follows the other people in the call (the speaker
//! output), which gives talk share. A turn that starts while others are
//! still talking, and then lasts `INTERRUPTION_MIN_MS`, is an interruption.

use serde::Serialize;

/// A turn started over someone else counts as an interruption only when it
/// lasts this long, so short backchannel words ("yeah", "right") do not count.
pub const INTERRUPTION_MIN_MS: u64 = 1_500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    Idle,
    Speaking,
    Warning,
    Alert,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Speaking => "speaking",
            State::Warning => "warning",
            State::Alert => "alert",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub limit_ms: u64,
    pub warning_ratio: f64,
    pub silence_reset_ms: u64,
}

/// Aggregates for the current meeting, used for the popover and the session log.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SessionStats {
    pub turns: u32,
    pub talk_ms: u64,
    pub longest_turn_ms: u64,
    pub warnings: u32,
    pub alerts: u32,
    pub interruptions: u32,
    /// Duration of every closed turn, in order.
    pub turn_ms: Vec<u64>,
}

impl SessionStats {
    pub fn median_turn_ms(&self) -> u64 {
        let mut sorted = self.turn_ms.clone();
        sorted.sort_unstable();
        match sorted.len() {
            0 => 0,
            n if n % 2 == 1 => sorted[n / 2],
            n => (sorted[n / 2 - 1] + sorted[n / 2]) / 2,
        }
    }
}

pub struct Tracker {
    limits: Limits,
    turn_start_ms: Option<u64>,
    last_voice_ms: u64,
    now_ms: u64,
    turn_peak: State,
    others_talking: bool,
    started_over_others: bool,
    stats: SessionStats,
}

impl Tracker {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            turn_start_ms: None,
            last_voice_ms: 0,
            now_ms: 0,
            turn_peak: State::Idle,
            others_talking: false,
            started_over_others: false,
            stats: SessionStats::default(),
        }
    }

    pub fn set_limits(&mut self, limits: Limits) {
        self.limits = limits;
    }

    /// Whether someone else is holding the floor right now. Set before `update`.
    pub fn set_others_talking(&mut self, talking: bool) {
        self.others_talking = talking;
    }

    /// Advance to `now_ms` with the latest voice decision.
    pub fn update(&mut self, now_ms: u64, voiced: bool) {
        self.now_ms = now_ms;
        if voiced {
            if self.turn_start_ms.is_none() {
                self.turn_start_ms = Some(now_ms);
                self.turn_peak = State::Speaking;
                self.started_over_others = self.others_talking;
            }
            self.last_voice_ms = now_ms;
        } else if self.turn_start_ms.is_some()
            && now_ms.saturating_sub(self.last_voice_ms) >= self.limits.silence_reset_ms
        {
            self.end_turn();
            return;
        }

        let state = self.state();
        if state > self.turn_peak {
            match state {
                State::Warning => self.stats.warnings += 1,
                State::Alert => self.stats.alerts += 1,
                _ => {}
            }
            self.turn_peak = state;
        }
    }

    /// Close any open turn, e.g. when capture stops.
    pub fn end_turn(&mut self) {
        if let Some(start) = self.turn_start_ms.take() {
            let duration = self.last_voice_ms.saturating_sub(start);
            self.stats.turns += 1;
            self.stats.talk_ms += duration;
            self.stats.longest_turn_ms = self.stats.longest_turn_ms.max(duration);
            self.stats.turn_ms.push(duration);
            if self.started_over_others && duration >= INTERRUPTION_MIN_MS {
                self.stats.interruptions += 1;
            }
        }
        self.turn_peak = State::Idle;
        self.started_over_others = false;
    }

    /// Start a fresh session: close the turn and clear statistics.
    pub fn reset_session(&mut self) -> SessionStats {
        self.end_turn();
        std::mem::take(&mut self.stats)
    }

    pub fn talking_ms(&self) -> u64 {
        self.turn_start_ms
            .map_or(0, |start| self.now_ms.saturating_sub(start))
    }

    pub fn in_turn(&self) -> bool {
        self.turn_start_ms.is_some()
    }

    /// Talk time of the session so far, including the open turn.
    pub fn total_talk_ms(&self) -> u64 {
        let open = self
            .turn_start_ms
            .map_or(0, |start| self.last_voice_ms.saturating_sub(start));
        self.stats.talk_ms + open
    }

    pub fn state(&self) -> State {
        if self.turn_start_ms.is_none() {
            return State::Idle;
        }
        let talking = self.talking_ms() as f64;
        let limit = self.limits.limit_ms as f64;
        if talking >= limit {
            State::Alert
        } else if talking >= limit * self.limits.warning_ratio {
            State::Warning
        } else {
            State::Speaking
        }
    }

    pub fn stats(&self) -> &SessionStats {
        &self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: u64 = 100;

    fn tracker() -> Tracker {
        Tracker::new(Limits {
            limit_ms: 10_000,
            warning_ratio: 0.75,
            silence_reset_ms: 3_000,
        })
    }

    /// Feed `ms` of audio, voiced or silent, in 100 ms frames.
    fn run(t: &mut Tracker, now: &mut u64, ms: u64, voiced: bool) {
        for _ in 0..ms / FRAME {
            *now += FRAME;
            t.update(*now, voiced);
        }
    }

    #[test]
    fn starts_idle() {
        let t = tracker();
        assert_eq!(t.state(), State::Idle);
        assert_eq!(t.talking_ms(), 0);
    }

    #[test]
    fn goes_through_speaking_warning_alert() {
        let mut t = tracker();
        let mut now = 0;
        run(&mut t, &mut now, 1_000, true);
        assert_eq!(t.state(), State::Speaking);
        run(&mut t, &mut now, 6_600, true);
        assert_eq!(t.state(), State::Warning);
        run(&mut t, &mut now, 2_500, true);
        assert_eq!(t.state(), State::Alert);
        assert_eq!(t.talking_ms(), 10_000);
    }

    #[test]
    fn short_pauses_stay_in_the_turn() {
        let mut t = tracker();
        let mut now = 0;
        run(&mut t, &mut now, 2_000, true);
        run(&mut t, &mut now, 1_500, false);
        run(&mut t, &mut now, 2_000, true);
        assert_eq!(t.talking_ms(), 5_400);
        assert_eq!(t.stats().turns, 0);
    }

    #[test]
    fn long_silence_ends_the_turn() {
        let mut t = tracker();
        let mut now = 0;
        run(&mut t, &mut now, 4_000, true);
        run(&mut t, &mut now, 3_000, false);
        assert_eq!(t.state(), State::Idle);
        assert_eq!(t.talking_ms(), 0);
        assert_eq!(t.stats().turns, 1);
        assert_eq!(t.stats().longest_turn_ms, 3_900);
    }

    #[test]
    fn warnings_and_alerts_count_once_per_turn() {
        let mut t = tracker();
        let mut now = 0;
        run(&mut t, &mut now, 12_000, true);
        run(&mut t, &mut now, 3_000, false);
        run(&mut t, &mut now, 8_000, true);
        let s = t.reset_session();
        assert_eq!(s.turns, 2);
        assert_eq!(s.warnings, 2);
        assert_eq!(s.alerts, 1);
        assert_eq!(t.stats(), &SessionStats::default());
    }

    #[test]
    fn end_turn_records_duration() {
        let mut t = tracker();
        let mut now = 0;
        run(&mut t, &mut now, 2_000, true);
        t.end_turn();
        assert_eq!(t.state(), State::Idle);
        assert_eq!(t.stats().talk_ms, 1_900);
    }

    #[test]
    fn long_turn_over_others_is_an_interruption() {
        let mut t = tracker();
        let mut now = 0;
        t.set_others_talking(true);
        run(&mut t, &mut now, 3_000, true);
        t.set_others_talking(false);
        run(&mut t, &mut now, 3_000, false);
        assert_eq!(t.stats().interruptions, 1);
    }

    #[test]
    fn short_overlap_is_not_an_interruption() {
        let mut t = tracker();
        let mut now = 0;
        t.set_others_talking(true);
        run(&mut t, &mut now, 800, true);
        run(&mut t, &mut now, 3_000, false);
        t.set_others_talking(false);
        run(&mut t, &mut now, 3_000, true);
        t.end_turn();
        assert_eq!(t.stats().turns, 2);
        assert_eq!(t.stats().interruptions, 0);
    }

    #[test]
    fn turn_started_before_others_is_not_an_interruption() {
        let mut t = tracker();
        let mut now = 0;
        run(&mut t, &mut now, 500, true);
        t.set_others_talking(true);
        run(&mut t, &mut now, 3_000, true);
        t.end_turn();
        assert_eq!(t.stats().interruptions, 0);
    }

    #[test]
    fn median_turn() {
        let mut s = SessionStats::default();
        assert_eq!(s.median_turn_ms(), 0);
        s.turn_ms = vec![5_000, 1_000, 3_000];
        assert_eq!(s.median_turn_ms(), 3_000);
        s.turn_ms.push(9_000);
        assert_eq!(s.median_turn_ms(), 4_000);
    }

    #[test]
    fn total_talk_includes_open_turn() {
        let mut t = tracker();
        let mut now = 0;
        run(&mut t, &mut now, 2_000, true);
        run(&mut t, &mut now, 3_000, false);
        run(&mut t, &mut now, 1_000, true);
        assert!(t.in_turn());
        assert_eq!(t.total_talk_ms(), 1_900 + 900);
    }
}
