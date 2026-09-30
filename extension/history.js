// Respire call history: parse sessions.jsonl and aggregate it.
//
// Pure JavaScript with no GNOME imports, so the shell (extension.js), the
// preferences window (prefs.js) and the tests (tools/history-test.mjs, under
// gjs or node) can all use it.

const HOUR = 3600;
const DAY_MS = 24 * HOUR * 1000;

export function formatTime(seconds) {
    const m = Math.floor(seconds / 60);
    const s = Math.floor(seconds % 60);
    return `${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
}

export function plural(n, word) {
    return `${n} ${word}${n === 1 ? '' : 's'}`;
}

export function percent(share) {
    return `${Math.round(share * 100)}%`;
}

function has(value) {
    return value !== null && value !== undefined;
}

/**
 * Parse sessions.jsonl text. Bad lines are skipped. Version 1 lines have no
 * talk share, interruptions or median turn; those fields become null.
 */
export function parseSessions(text) {
    const sessions = [];
    for (const line of text.split('\n')) {
        if (!line.trim())
            continue;
        let s;
        try {
            s = JSON.parse(line);
        } catch {
            continue;
        }
        if (typeof s !== 'object' || s === null || !Number.isFinite(s.start) || !Number.isFinite(s.end))
            continue;
        sessions.push({
            start: s.start,
            end: Math.max(s.end, s.start),
            apps: Array.isArray(s.apps) ? s.apps : [],
            limitSec: s.limit_sec ?? null,
            turns: s.turns ?? 0,
            talkSec: s.talk_sec ?? 0,
            longestTurnSec: s.longest_turn_sec ?? 0,
            medianTurnSec: s.median_turn_sec ?? null,
            warnings: s.warnings ?? 0,
            alerts: s.alerts ?? 0,
            othersTalkSec: s.others_talk_sec ?? null,
            talkShare: s.talk_share ?? null,
            interruptions: s.interruptions ?? null,
        });
    }
    sessions.sort((a, b) => a.start - b.start);
    return sessions;
}

function median(values) {
    if (values.length === 0)
        return null;
    const sorted = [...values].sort((a, b) => a - b);
    const mid = Math.floor(sorted.length / 2);
    return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

function sum(values) {
    return values.reduce((a, b) => a + b, 0);
}

function perHour(count, seconds) {
    return seconds > 0 ? count / (seconds / HOUR) : null;
}

/** Totals and rates over a list of sessions. A metric with no data is null. */
export function aggregate(sessions) {
    const duration = s => s.end - s.start;
    const withOthers = sessions.filter(s => has(s.othersTalkSec));
    const everyone = sum(withOthers.map(s => s.talkSec + s.othersTalkSec));
    const withInterruptions = sessions.filter(s => has(s.interruptions));
    const medians = sessions.filter(s => has(s.medianTurnSec) && s.turns > 0);
    return {
        calls: sessions.length,
        callMinutes: sum(sessions.map(duration)) / 60,
        talkShare: everyone > 0 ? sum(withOthers.map(s => s.talkSec)) / everyone : null,
        medianTurnSec: median(medians.map(s => s.medianTurnSec)),
        alertsPerHour: perHour(sum(sessions.map(s => s.alerts)), sum(sessions.map(duration))),
        interruptionsPerHour: perHour(
            sum(withInterruptions.map(s => s.interruptions)),
            sum(withInterruptions.map(duration))),
        longestTurnSec: sessions.length ? Math.max(...sessions.map(s => s.longestTurnSec)) : null,
    };
}

/**
 * The metrics the Progress page shows. `lower` means a lower value is
 * better. Talk share is neutral: the right share depends on how many
 * people are in the call.
 */
export const METRICS = [
    {key: 'talkShare', label: 'Your talk share', better: 'neutral', format: percent},
    {key: 'medianTurnSec', label: 'Median turn', better: 'lower', format: formatTime},
    {key: 'alertsPerHour', label: 'Alerts per hour of calls', better: 'lower', format: v => v.toFixed(1)},
    {key: 'interruptionsPerHour', label: 'Interruptions per hour of calls', better: 'lower', format: v => v.toFixed(1)},
];

/** Sessions that started in [fromMs, toMs). */
export function between(sessions, fromMs, toMs) {
    return sessions.filter(s => s.start * 1000 >= fromMs && s.start * 1000 < toMs);
}

/** Local midnight of the Monday that starts the week containing `date`. */
export function weekStart(date) {
    const d = new Date(date);
    d.setHours(0, 0, 0, 0);
    d.setDate(d.getDate() - (d.getDay() + 6) % 7);
    return d;
}

/**
 * The last `weeks` calendar weeks (Monday to Monday, local time), oldest
 * first, each with its sessions and aggregate.
 */
export function weekly(sessions, weeks, now = new Date()) {
    const buckets = [];
    const current = weekStart(now);
    for (let i = weeks - 1; i >= 0; i--) {
        const start = new Date(current);
        start.setDate(start.getDate() - 7 * i);
        const end = new Date(start);
        end.setDate(end.getDate() + 7);
        const inWeek = between(sessions, start.getTime(), end.getTime());
        buckets.push({start, end, sessions: inWeek, stats: aggregate(inWeek)});
    }
    return buckets;
}

/**
 * Last 7 days against the 28 days before them. Used for the headline rows
 * of the Progress page.
 */
export function recentVsBaseline(sessions, now = new Date()) {
    const t = now.getTime();
    return {
        recent: aggregate(between(sessions, t - 7 * DAY_MS, t + 1)),
        baseline: aggregate(between(sessions, t - 35 * DAY_MS, t - 7 * DAY_MS)),
    };
}

/**
 * Compare one metric. Returns null when either side has no data; otherwise
 * `{delta, verdict}` where verdict is `better`, `worse` or `neutral`.
 * Changes under 5% of the baseline count as neutral.
 */
export function compare(metric, recent, baseline) {
    const cur = recent[metric.key];
    const base = baseline[metric.key];
    if (!has(cur) || !has(base))
        return null;
    const delta = cur - base;
    const noise = Math.abs(base) * 0.05;
    let verdict = 'neutral';
    if (metric.better === 'lower' && Math.abs(delta) > noise)
        verdict = delta < 0 ? 'better' : 'worse';
    return {delta, verdict};
}

/** One-line description of a call, from a parsed session. */
export function describe(s) {
    const parts = [];
    if (has(s.talkShare))
        parts.push(`you ${percent(s.talkShare)}`);
    parts.push(plural(s.turns, 'turn'));
    parts.push(`longest ${formatTime(s.longestTurnSec)}`);
    if (s.alerts > 0)
        parts.push(plural(s.alerts, 'alert'));
    if (s.interruptions > 0)
        parts.push(plural(s.interruptions, 'interruption'));
    return parts.join(' · ');
}

/** Text for the post-call notification, from the SessionFinished JSON. */
export function formatSummary(json) {
    const [s] = parseSessions(json);
    if (!s)
        return null;
    const minutes = Math.max(1, Math.round((s.end - s.start) / 60));
    return `${minutes} min · ${describe(s)}`;
}
