// Tests for extension/history.js. Runs under gjs or node:
//   gjs -m tools/history-test.mjs
//   node tools/history-test.mjs
import * as H from '../extension/history.js';

let failures = 0;
function test(name, fn) {
    try {
        fn();
        print(`ok   ${name}`);
    } catch (e) {
        failures++;
        print(`FAIL ${name}: ${e.message}`);
    }
}
// gjs has print() on stdout; node has console.log.
const print = typeof globalThis.print === 'function' ? globalThis.print : console.log;
function eq(actual, expected, what = '') {
    const a = JSON.stringify(actual);
    const e = JSON.stringify(expected);
    if (a !== e)
        throw new Error(`${what} expected ${e}, got ${a}`);
}
function near(actual, expected, what = '') {
    if (actual === null || Math.abs(actual - expected) > 1e-9)
        throw new Error(`${what} expected ${expected}, got ${actual}`);
}

// Local-time unix seconds, so week buckets match wherever the test runs.
const at = (y, mo, d, h = 10) => new Date(y, mo - 1, d, h).getTime() / 1000;

const v1 = start => JSON.stringify({
    start, end: start + 1800, apps: ['Firefox'], limit_sec: 90, turns: 10,
    talk_sec: 300, longest_turn_sec: 95, warnings: 2, alerts: 1,
});
const v2 = (start, extra = {}) => JSON.stringify({
    v: 2, start, end: start + 3600, apps: ['Firefox'], limit_sec: 90, turns: 12,
    talk_sec: 600, longest_turn_sec: 80, median_turn_sec: 20, warnings: 1, alerts: 2,
    others_talk_sec: 1400, talk_share: 0.3, interruptions: 3, ...extra,
});

test('parses v1 and v2, skips bad lines, sorts by start', () => {
    const text = [v2(2000), 'not json', '', v1(1000), '{"start": "x"}', '[]'].join('\n');
    const s = H.parseSessions(text);
    eq(s.map(x => x.start), [1000, 2000]);
    eq(s[0].talkShare, null, 'v1 share');
    eq(s[0].interruptions, null, 'v1 interruptions');
    eq(s[0].medianTurnSec, null, 'v1 median');
    eq(s[1].talkShare, 0.3);
    eq(s[1].othersTalkSec, 1400);
});

test('aggregate weights share by talk time and uses v2 only for new metrics', () => {
    const s = H.parseSessions([
        v1(0),
        v2(10000),
        v2(20000, {talk_sec: 1000, others_talk_sec: 1000, talk_share: 0.5, median_turn_sec: 40, interruptions: 1}),
    ].join('\n'));
    const a = H.aggregate(s);
    eq(a.calls, 3);
    near(a.callMinutes, 30 + 60 + 60);
    near(a.talkShare, 1600 / 4000, 'share');
    near(a.medianTurnSec, 30, 'median of medians');
    near(a.alertsPerHour, 5 / 2.5, 'alerts/h over all calls');
    near(a.interruptionsPerHour, 4 / 2, 'interruptions/h over v2 calls');
    eq(a.longestTurnSec, 95);
});

test('aggregate of nothing is all null', () => {
    const a = H.aggregate([]);
    eq([a.calls, a.talkShare, a.medianTurnSec, a.alertsPerHour, a.interruptionsPerHour, a.longestTurnSec],
        [0, null, null, null, null, null]);
});

test('weekStart is local Monday midnight', () => {
    const sunday = new Date(2026, 8, 27, 23, 30);
    eq(H.weekStart(sunday).getTime(), new Date(2026, 8, 21).getTime(), 'sunday');
    const monday = new Date(2026, 8, 28, 0, 0);
    eq(H.weekStart(monday).getTime(), new Date(2026, 8, 28).getTime(), 'monday');
});

test('weekly buckets are calendar weeks, oldest first, across a DST change', () => {
    // Late October / early November crosses DST in both Europe and the US.
    const now = new Date(2026, 10, 12, 12);
    const s = H.parseSessions([
        v2(at(2026, 10, 19)), v2(at(2026, 10, 25, 23)), v2(at(2026, 10, 26, 0)),
        v2(at(2026, 11, 2)), v2(at(2026, 11, 9)), v2(at(2026, 11, 12)),
    ].join('\n'));
    const weeks = H.weekly(s, 4, now);
    eq(weeks.length, 4);
    eq(weeks.map(w => w.start.getDay()), [1, 1, 1, 1], 'all Mondays');
    eq(weeks.map(w => w.start.getHours()), [0, 0, 0, 0], 'all midnight');
    eq(weeks.map(w => w.sessions.length), [2, 1, 1, 2]);
});

test('recent vs baseline windows', () => {
    const now = new Date(2026, 8, 30, 12);
    const day = 86400;
    const t = now.getTime() / 1000;
    const s = H.parseSessions([v2(t - day), v2(t - 10 * day), v2(t - 20 * day), v2(t - 40 * day)].join('\n'));
    const {recent, baseline} = H.recentVsBaseline(s, now);
    eq(recent.calls, 1);
    eq(baseline.calls, 2);
});

test('compare directions', () => {
    const median = H.METRICS.find(m => m.key === 'medianTurnSec');
    const share = H.METRICS.find(m => m.key === 'talkShare');
    eq(H.compare(median, {medianTurnSec: 15}, {medianTurnSec: 20}).verdict, 'better');
    eq(H.compare(median, {medianTurnSec: 25}, {medianTurnSec: 20}).verdict, 'worse');
    eq(H.compare(median, {medianTurnSec: 20.5}, {medianTurnSec: 20}).verdict, 'neutral', 'within noise');
    eq(H.compare(share, {talkShare: 0.2}, {talkShare: 0.5}).verdict, 'neutral');
    eq(H.compare(median, {medianTurnSec: null}, {medianTurnSec: 20}), null);
});

test('describe and formatSummary', () => {
    eq(H.formatSummary(v2(0)), '60 min · you 30% · 12 turns · longest 01:20 · 2 alerts · 3 interruptions');
    eq(H.formatSummary(v1(0)), '30 min · 10 turns · longest 01:35 · 1 alert');
    eq(H.formatSummary('garbage'), null);
    eq(H.formatTime(0), '00:00');
    eq(H.formatTime(125), '02:05');
});

if (failures > 0) {
    print(`${failures} failed`);
    // gjs has no process; throwing makes both runtimes exit non-zero.
    throw new Error(`${failures} history tests failed`);
}
print('all history tests passed');
