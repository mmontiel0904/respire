// Respire: History & progress window.
//
// Reads the per-call log that respired writes to
// $XDG_STATE_HOME/respire/sessions.jsonl and shows how you are doing over
// time. Nothing here talks to the daemon; the file is the only input.

import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk';
import PangoCairo from 'gi://PangoCairo';

import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

import {METRICS, compare, describe, parseSessions, plural, recentVsBaseline, weekly} from './history.js';

const HISTORY_DAYS = 60;
const TREND_WEEKS = 12;

function logFile() {
    return Gio.File.new_for_path(
        GLib.build_filenamev([GLib.get_user_state_dir(), 'respire', 'sessions.jsonl']));
}

function readSessions() {
    try {
        const [, bytes] = logFile().load_contents(null);
        return parseSessions(new TextDecoder().decode(bytes));
    } catch (e) {
        if (!e.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.NOT_FOUND))
            console.warn(`Respire: cannot read call log: ${e.message}`);
        return [];
    }
}

function localTime(unixSec) {
    return GLib.DateTime.new_from_unix_local(unixSec);
}

function dayTitle(dt) {
    const today = GLib.DateTime.new_now_local();
    const days = Math.round((
        GLib.DateTime.new_local(today.get_year(), today.get_month(), today.get_day_of_month(), 0, 0, 0).to_unix() -
        GLib.DateTime.new_local(dt.get_year(), dt.get_month(), dt.get_day_of_month(), 0, 0, 0).to_unix()
    ) / 86400);
    if (days === 0)
        return 'Today';
    if (days === 1)
        return 'Yesterday';
    return dt.format('%A, %e %B').replace(/\s+/g, ' ');
}

function emptyState(description) {
    return new Adw.StatusPage({
        icon_name: 'audio-input-microphone-symbolic',
        title: 'No calls yet',
        description,
        hexpand: true,
        css_classes: ['compact'],
    });
}

// A page whose groups are rebuilt from scratch on every refresh.
class RebuildablePage {
    constructor(props) {
        this.page = new Adw.PreferencesPage(props);
        this._groups = [];
    }

    clear() {
        this._groups.forEach(g => this.page.remove(g));
        this._groups = [];
    }

    add(group) {
        this.page.add(group);
        this._groups.push(group);
        return group;
    }
}

class ProgressPage extends RebuildablePage {
    constructor() {
        super({name: 'progress', title: 'Progress', icon_name: 'document-open-recent-symbolic'});
        this._sessions = [];
        this._metric = METRICS[1];
    }

    refresh(sessions) {
        this._sessions = sessions;
        this.clear();
        if (sessions.length === 0) {
            const group = this.add(new Adw.PreferencesGroup());
            group.add(emptyState('After your first call, your progress shows here.'));
            return;
        }
        this._addHeadline();
        this._addTrend();
    }

    _addHeadline() {
        const {recent, baseline} = recentVsBaseline(this._sessions);
        const hours = recent.callMinutes / 60;
        const group = this.add(new Adw.PreferencesGroup({
            title: 'Last 7 days',
            description: recent.calls > 0
                ? `${plural(recent.calls, 'call')}, ${hours.toFixed(1)} h in total. Compared with the 4 weeks before.`
                : 'No calls in the last 7 days.',
        }));
        for (const metric of METRICS) {
            const value = recent[metric.key];
            const base = baseline[metric.key];
            const row = new Adw.ActionRow({
                title: metric.label,
                subtitle: base === null ? 'No earlier data' : `Before: ${metric.format(base)}`,
                use_markup: false,
            });
            const change = compare(metric, recent, baseline);
            if (change && change.verdict !== 'neutral') {
                const arrow = change.delta < 0 ? '↓' : '↑';
                const label = new Gtk.Label({
                    label: arrow,
                    css_classes: [change.verdict === 'better' ? 'success' : 'error'],
                    tooltip_text: change.verdict === 'better' ? 'Better than before' : 'Worse than before',
                });
                row.add_suffix(label);
            }
            row.add_suffix(new Gtk.Label({
                label: value === null ? '—' : metric.format(value),
                css_classes: ['title-4'],
                valign: Gtk.Align.CENTER,
            }));
            group.add(row);
        }
    }

    _addTrend() {
        const group = this.add(new Adw.PreferencesGroup({
            title: 'Weekly trend',
            description: `The last ${TREND_WEEKS} weeks. The dashed line is the average.`,
        }));
        const picker = new Adw.ComboRow({
            title: 'Metric',
            model: Gtk.StringList.new(METRICS.map(m => m.label)),
            selected: METRICS.indexOf(this._metric),
        });
        group.add(picker);

        const chart = new Gtk.DrawingArea({
            content_height: 220,
            margin_top: 12,
            hexpand: true,
        });
        chart.set_draw_func((area, cr, width, height) => {
            this._drawChart(area, cr, width, height);
            cr.$dispose();
        });
        picker.connect('notify::selected', () => {
            this._metric = METRICS[picker.selected];
            chart.queue_draw();
        });
        group.add(chart);
    }

    _drawChart(area, cr, width, height) {
        const metric = this._metric;
        const weeks = weekly(this._sessions, TREND_WEEKS);
        const values = weeks.map(w => w.stats[metric.key]);
        const known = values.filter(v => v !== null);

        const fg = area.get_color();
        const accent = Adw.StyleManager.get_default().get_accent_color_rgba();
        const labelHeight = 20;
        const top = 18;
        const plotHeight = height - labelHeight - top;
        const slot = width / weeks.length;
        const barWidth = Math.max(4, slot * 0.6);
        const max = Math.max(...known, metric.key === 'talkShare' ? 1 : 0) || 1;
        const y = v => top + plotHeight * (1 - v / max);

        const text = (str, x, yPos, alpha) => {
            const layout = area.create_pango_layout(str);
            const [, logical] = layout.get_pixel_extents();
            cr.setSourceRGBA(fg.red, fg.green, fg.blue, alpha);
            cr.moveTo(x - logical.width / 2, yPos);
            PangoCairo.show_layout(cr, layout);
        };

        // Axis.
        cr.setSourceRGBA(fg.red, fg.green, fg.blue, 0.2);
        cr.setLineWidth(1);
        cr.moveTo(0, top + plotHeight + 0.5);
        cr.lineTo(width, top + plotHeight + 0.5);
        cr.stroke();

        values.forEach((v, i) => {
            const cx = slot * (i + 0.5);
            if (v === null) {
                // No calls that week: a small gap marker on the axis.
                cr.setSourceRGBA(fg.red, fg.green, fg.blue, 0.25);
                cr.rectangle(cx - barWidth / 2, top + plotHeight - 2, barWidth, 2);
                cr.fill();
            } else {
                const current = i === values.length - 1;
                cr.setSourceRGBA(accent.red, accent.green, accent.blue, current ? 1 : 0.7);
                cr.rectangle(cx - barWidth / 2, y(v), barWidth, top + plotHeight - y(v));
                cr.fill();
                if (current || slot > 44)
                    text(metric.format(v), cx, y(v) - 17, 0.8);
            }
            if (i % 2 === (values.length - 1) % 2) {
                const start = weeks[i].start;
                const dt = GLib.DateTime.new_local(start.getFullYear(), start.getMonth() + 1, start.getDate(), 0, 0, 0);
                text(dt.format('%e %b').trim(), cx, top + plotHeight + 3, 0.6);
            }
        });

        if (known.length > 1) {
            const average = known.reduce((a, b) => a + b, 0) / known.length;
            cr.setSourceRGBA(fg.red, fg.green, fg.blue, 0.5);
            cr.setDash([4, 4], 0);
            cr.moveTo(0, Math.round(y(average)) + 0.5);
            cr.lineTo(width, Math.round(y(average)) + 0.5);
            cr.stroke();
        }
    }
}

class HistoryPage extends RebuildablePage {
    constructor(window) {
        super({name: 'history', title: 'History', icon_name: 'view-list-symbolic'});
        this._window = window;
    }

    refresh(sessions) {
        this.clear();
        const cutoff = Date.now() / 1000 - HISTORY_DAYS * 86400;
        const recent = sessions.filter(s => s.start >= cutoff).reverse();
        if (recent.length === 0) {
            const group = this.add(new Adw.PreferencesGroup());
            group.add(emptyState(`Calls from the last ${HISTORY_DAYS} days show here.`));
        }

        let group = null;
        let day = null;
        for (const s of recent) {
            const start = localTime(s.start);
            const key = start.format('%F');
            if (key !== day) {
                day = key;
                group = this.add(new Adw.PreferencesGroup({title: dayTitle(start)}));
            }
            const minutes = Math.max(1, Math.round((s.end - s.start) / 60));
            const title = [start.format('%H:%M'), s.apps.join(', ') || 'Call', `${minutes} min`].join(' · ');
            const subtitle = describe(s);
            group.add(new Adw.ActionRow({
                title,
                subtitle: subtitle[0].toUpperCase() + subtitle.slice(1),
                use_markup: false,
            }));
        }

        if (sessions.length > 0)
            this._addClear(sessions.length);
    }

    _addClear(count) {
        const group = this.add(new Adw.PreferencesGroup({
            description: 'Only call summaries are stored: no audio and no words.',
        }));
        const row = new Adw.ButtonRow({title: 'Clear History', css_classes: ['destructive-action']});
        row.connect('activated', () => {
            const dialog = new Adw.AlertDialog({
                heading: 'Clear call history?',
                body: `This deletes the summaries of ${plural(count, 'call')}. You cannot undo this.`,
                close_response: 'cancel',
                default_response: 'cancel',
            });
            dialog.add_response('cancel', 'Cancel');
            dialog.add_response('clear', 'Clear');
            dialog.set_response_appearance('clear', Adw.ResponseAppearance.DESTRUCTIVE);
            dialog.connect('response', (_dialog, response) => {
                if (response !== 'clear')
                    return;
                try {
                    logFile().delete(null);
                } catch (e) {
                    console.warn(`Respire: cannot clear call log: ${e.message}`);
                }
            });
            dialog.present(this._window);
        });
        group.add(row);
    }
}

export default class RespirePreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        window.set_default_size(640, 760);
        window.search_enabled = false;

        const progress = new ProgressPage();
        const history = new HistoryPage(window);
        window.add(progress.page);
        window.add(history.page);

        const refresh = () => {
            const sessions = readSessions();
            progress.refresh(sessions);
            history.refresh(sessions);
        };
        refresh();

        // Refresh when a call ends (the daemon appends a line) or the log is cleared.
        let timeoutId = 0;
        // The directory must exist for the monitor to see the file appear.
        GLib.mkdir_with_parents(logFile().get_parent().get_path(), 0o700);
        const monitor = logFile().monitor_file(Gio.FileMonitorFlags.NONE, null);
        monitor.connect('changed', () => {
            if (timeoutId)
                GLib.source_remove(timeoutId);
            timeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 300, () => {
                timeoutId = 0;
                refresh();
                return GLib.SOURCE_REMOVE;
            });
        });
        window.connect('close-request', () => {
            monitor.cancel();
            if (timeoutId)
                GLib.source_remove(timeoutId);
            timeoutId = 0;
            return false;
        });
        window._respireMonitor = monitor;
    }
}
