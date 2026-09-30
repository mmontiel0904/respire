//! The per-call summary: one line in `sessions.jsonl`, and the payload of
//! the `SessionFinished` D-Bus signal.
//!
//! Version 2 adds talk share, interruptions and the median turn. The
//! version 1 fields keep their names, so readers can handle both.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::tracker::SessionStats;

pub const VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionSummary {
    pub v: u32,
    pub start: u64,
    pub end: u64,
    pub apps: BTreeSet<String>,
    pub limit_sec: u32,
    pub turns: u32,
    pub talk_sec: u64,
    pub longest_turn_sec: u64,
    pub median_turn_sec: u64,
    pub warnings: u32,
    pub alerts: u32,
    /// The fields below are `null` when the other people in the call were not tracked.
    pub others_talk_sec: Option<u64>,
    /// Your talk time divided by everyone's talk time, 0.0 to 1.0.
    pub talk_share: Option<f64>,
    pub interruptions: Option<u32>,
}

impl SessionSummary {
    pub fn new(
        start: u64,
        end: u64,
        apps: BTreeSet<String>,
        limit_sec: u32,
        mine: &SessionStats,
        others: Option<&SessionStats>,
    ) -> Self {
        Self {
            v: VERSION,
            start,
            end,
            apps,
            limit_sec,
            turns: mine.turns,
            talk_sec: mine.talk_ms / 1000,
            longest_turn_sec: mine.longest_turn_ms / 1000,
            median_turn_sec: mine.median_turn_ms() / 1000,
            warnings: mine.warnings,
            alerts: mine.alerts,
            others_talk_sec: others.map(|o| o.talk_ms / 1000),
            talk_share: others.and_then(|o| talk_share(mine.talk_ms, o.talk_ms)),
            interruptions: others.map(|_| mine.interruptions),
        }
    }
}

/// `mine / (mine + others)`, rounded to two decimals; `None` when nobody talked.
pub fn talk_share(mine_ms: u64, others_ms: u64) -> Option<f64> {
    let total = mine_ms + others_ms;
    (total > 0).then(|| (mine_ms as f64 / total as f64 * 100.0).round() / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(talk_ms: u64, interruptions: u32) -> SessionStats {
        SessionStats {
            turns: 3,
            talk_ms,
            longest_turn_ms: 40_500,
            warnings: 1,
            alerts: 0,
            interruptions,
            turn_ms: vec![2_000, 40_500, 9_000],
        }
    }

    #[test]
    fn share_is_rounded() {
        assert_eq!(talk_share(1, 2), Some(0.33));
        assert_eq!(talk_share(10_000, 0), Some(1.0));
        assert_eq!(talk_share(0, 0), None);
    }

    #[test]
    fn with_others() {
        let s = SessionSummary::new(
            100,
            700,
            BTreeSet::from(["Firefox".to_string()]),
            90,
            &stats(60_000, 2),
            Some(&stats(140_000, 0)),
        );
        assert_eq!(s.talk_sec, 60);
        assert_eq!(s.median_turn_sec, 9);
        assert_eq!(s.others_talk_sec, Some(140));
        assert_eq!(s.talk_share, Some(0.3));
        assert_eq!(s.interruptions, Some(2));
    }

    #[test]
    fn without_others_the_new_fields_are_null() {
        let s = SessionSummary::new(0, 1, BTreeSet::new(), 90, &stats(5_000, 1), None);
        let json = serde_json::to_value(&s).unwrap();
        assert!(json["talk_share"].is_null());
        assert!(json["interruptions"].is_null());
        assert!(json["others_talk_sec"].is_null());
    }

    #[test]
    fn json_keys_are_stable() {
        let s = SessionSummary::new(0, 1, BTreeSet::new(), 90, &stats(5_000, 0), None);
        let json = serde_json::to_value(&s).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "alerts",
                "apps",
                "end",
                "interruptions",
                "limit_sec",
                "longest_turn_sec",
                "median_turn_sec",
                "others_talk_sec",
                "start",
                "talk_sec",
                "talk_share",
                "turns",
                "v",
                "warnings",
            ]
        );
    }
}
