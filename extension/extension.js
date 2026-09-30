// Respire: top-bar indicator for the respired daemon.
//
// All audio work happens in respired. This extension only reads D-Bus
// properties, draws the progress ring, and sends the popover's settings back.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {Slider} from 'resource:///org/gnome/shell/ui/slider.js';

import {formatSummary, formatTime, parseSessions, percent, plural} from './history.js';

const BUS_NAME = 'io.github.mmontiel0904.Respire';
const OBJECT_PATH = '/io/github/mmontiel0904/Respire';

const RespireIface = `
<node>
  <interface name="io.github.mmontiel0904.Respire1">
    <method name="SetLimit"><arg type="u" direction="in"/></method>
    <method name="SetSensitivity"><arg type="d" direction="in"/></method>
    <method name="Snooze"><arg type="u" direction="in"/></method>
    <method name="Resume"/>
    <property name="State" type="s" access="read"/>
    <property name="TalkingTime" type="u" access="read"/>
    <property name="TimeLimit" type="u" access="read"/>
    <property name="Progress" type="d" access="read"/>
    <property name="Sensitivity" type="d" access="read"/>
    <property name="MeetingApps" type="as" access="read"/>
    <property name="SnoozeRemaining" type="u" access="read"/>
    <property name="TurnCount" type="u" access="read"/>
    <property name="LongestTurn" type="u" access="read"/>
    <property name="AlertCount" type="u" access="read"/>
    <property name="TalkShare" type="d" access="read"/>
    <property name="InterruptionCount" type="u" access="read"/>
    <signal name="SessionFinished"><arg type="s"/></signal>
  </interface>
</node>`;
const RespireProxy = Gio.DBusProxy.makeProxyWrapper(RespireIface);

const LIMIT_CHOICES = [45, 60, 90, 120];
const ALERT_MESSAGE = 'Tómate un respiro: time to pause for questions!';

// Calls shorter than this, or with fewer turns, get no summary notification.
const SUMMARY_MIN_SEC = 120;
const SUMMARY_MIN_TURNS = 2;

// Ring colors per state, as [r, g, b] in 0..1.
const RING_COLORS = {
    speaking: [0.34, 0.89, 0.54],
    warning: [0.97, 0.71, 0.18],
    alert: [0.93, 0.20, 0.23],
};
const RING_TRACK_ALPHA = 0.25;

// Icon opacity (0..255) for states where Respire is not actively listening.
const ICON_OPACITY = {offline: 110, standby: 110, snoozed: 110, idle: 200};

const RespireIndicator = GObject.registerClass(
class RespireIndicator extends PanelMenu.Button {
    _init(extension) {
        super._init(0.5, 'Respire');

        this._extension = extension;
        this._state = 'offline';
        this._progress = 0;
        this._animatedState = null;
        this._sensitivityTimeoutId = 0;
        this._proxy = null;

        const box = new St.Widget({
            layout_manager: new Clutter.BinLayout(),
            style_class: 'respire-indicator',
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._ring = new St.DrawingArea({style_class: 'respire-ring'});
        this._ring.connect('repaint', area => this._drawRing(area));
        this._icon = new St.Icon({
            icon_name: 'audio-input-microphone-symbolic',
            style_class: 'system-status-icon respire-icon',
            x_align: Clutter.ActorAlign.CENTER,
            y_align: Clutter.ActorAlign.CENTER,
        });
        box.add_child(this._ring);
        box.add_child(this._icon);
        this._box = box;
        this.add_child(box);

        this._buildMenu();
        this._connectProxy();
        this._sync();
    }

    _buildMenu() {
        this._timeItem = new PopupMenu.PopupMenuItem('', {reactive: false});
        this._timeItem.label.add_style_class_name('respire-time');
        this.menu.addMenuItem(this._timeItem);

        this._statusItem = new PopupMenu.PopupMenuItem('', {reactive: false});
        this._statusItem.label.add_style_class_name('respire-status');
        this.menu.addMenuItem(this._statusItem);

        this._statsItem = new PopupMenu.PopupMenuItem('', {reactive: false});
        this._statsItem.label.add_style_class_name('respire-status');
        this.menu.addMenuItem(this._statsItem);

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem('Speaking limit'));

        const limitItem = new PopupMenu.PopupBaseMenuItem({reactive: false, can_focus: false});
        const limitBox = new St.BoxLayout({style_class: 'respire-limits', x_expand: true});
        this._limitButtons = LIMIT_CHOICES.map(seconds => {
            const button = new St.Button({
                label: `${seconds}s`,
                style_class: 'button respire-limit-button',
                toggle_mode: true,
                can_focus: true,
                x_expand: true,
            });
            button.connect('clicked', () => this._call('SetLimitAsync', seconds));
            limitBox.add_child(button);
            return {seconds, button};
        });
        limitItem.add_child(limitBox);
        this.menu.addMenuItem(limitItem);

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem('Microphone sensitivity'));

        const sliderItem = new PopupMenu.PopupBaseMenuItem({activate: false});
        this._slider = new Slider(0.5);
        this._slider.accessible_name = 'Microphone sensitivity';
        this._slider.connect('notify::value', () => this._queueSensitivity());
        sliderItem.add_child(this._slider);
        this.menu.addMenuItem(sliderItem);

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        this._snoozeItems = [
            [15, 'Snooze for 15 minutes'],
            [30, 'Snooze for 30 minutes'],
            [0, 'Snooze for this call'],
        ].map(([minutes, label]) => {
            const item = new PopupMenu.PopupMenuItem(label);
            item.connect('activate', () => this._call('SnoozeAsync', minutes));
            this.menu.addMenuItem(item);
            return item;
        });
        this._resumeItem = new PopupMenu.PopupMenuItem('Resume listening');
        this._resumeItem.connect('activate', () => this._call('ResumeAsync'));
        this.menu.addMenuItem(this._resumeItem);

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        const historyItem = new PopupMenu.PopupMenuItem('History & progress…');
        historyItem.connect('activate', () => this._extension.openPreferences());
        this.menu.addMenuItem(historyItem);
    }

    _connectProxy() {
        // Proxy construction also D-Bus-activates respired if it is installed but not running.
        new RespireProxy(Gio.DBus.session, BUS_NAME, OBJECT_PATH, (proxy, error) => {
            if (error) {
                console.warn(`Respire: cannot reach ${BUS_NAME}: ${error.message}`);
                return;
            }
            if (!this._box) // Destroyed while connecting.
                return;
            this._proxy = proxy;
            this._proxyIds = [
                proxy.connect('g-properties-changed', () => this._sync()),
                proxy.connect('notify::g-name-owner', () => this._sync()),
            ];
            this._summaryId = proxy.connectSignal('SessionFinished',
                (_proxy, _sender, [json]) => this._notifySummary(json));
            this._sync();
        });
    }

    _call(method, ...args) {
        if (!this._proxy?.g_name_owner)
            return;
        this._proxy[method](...args).catch(e => console.warn(`Respire: ${method} failed: ${e.message}`));
    }

    _notifySummary(json) {
        const [session] = parseSessions(json);
        if (!session || session.end - session.start < SUMMARY_MIN_SEC || session.turns < SUMMARY_MIN_TURNS)
            return;
        Main.notify('Respire · call summary', formatSummary(json));
    }

    _queueSensitivity() {
        if (this._syncingSlider)
            return;
        if (this._sensitivityTimeoutId)
            GLib.source_remove(this._sensitivityTimeoutId);
        this._sensitivityTimeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 300, () => {
            this._sensitivityTimeoutId = 0;
            this._call('SetSensitivityAsync', Math.round(this._slider.value * 100) / 100);
            return GLib.SOURCE_REMOVE;
        });
    }

    _sync() {
        const p = this._proxy;
        const online = Boolean(p?.g_name_owner) && p.State !== null;
        const prevState = this._state;
        this._state = online ? p.State : 'offline';
        this._progress = online ? p.Progress : 0;

        for (const s of ['offline', 'standby', 'snoozed', 'idle', 'speaking', 'warning', 'alert'])
            this._box.remove_style_class_name(`respire-${s}`);
        this._box.add_style_class_name(`respire-${this._state}`);
        this._icon.opacity = ICON_OPACITY[this._state] ?? 255;
        this._icon.icon_name = this._state === 'snoozed'
            ? 'microphone-disabled-symbolic'
            : 'audio-input-microphone-symbolic';
        this._ring.queue_repaint();
        this._animate();

        if (this._state === 'alert' && prevState !== 'alert')
            Main.notify('Respire', ALERT_MESSAGE);

        this._syncMenu(online);
    }

    _syncMenu(online) {
        const p = this._proxy;
        if (!online) {
            this._timeItem.label.text = 'Respire daemon is not running';
            this._statusItem.label.text = 'Start it with: systemctl --user start respired';
            this._statsItem.visible = false;
            return;
        }

        this._timeItem.label.text = `${formatTime(p.TalkingTime)} / ${formatTime(p.TimeLimit)}`;

        const apps = p.MeetingApps ?? [];
        const status = {
            standby: 'Waiting for a call to start',
            snoozed: p.SnoozeRemaining > 0
                ? `Snoozed, ${Math.ceil(p.SnoozeRemaining / 60)} min left`
                : 'Snoozed until this call ends',
            idle: 'Listening',
            speaking: 'You are speaking',
            warning: 'Getting long, find a place to pause',
            alert: ALERT_MESSAGE,
        }[this._state] ?? this._state;
        this._statusItem.label.text = apps.length > 0 ? `${status} · ${apps.join(', ')}` : status;

        this._statsItem.visible = p.TurnCount > 0;
        const stats = [
            plural(p.TurnCount, 'turn'),
            `longest ${formatTime(p.LongestTurn)}`,
            plural(p.AlertCount, 'alert'),
        ];
        if (p.TalkShare >= 0)
            stats.unshift(`you ${percent(p.TalkShare)}`);
        if (p.InterruptionCount > 0)
            stats.push(plural(p.InterruptionCount, 'interruption'));
        this._statsItem.label.text = `This call: ${stats.join(' · ')}`;

        for (const {seconds, button} of this._limitButtons)
            button.checked = seconds === p.TimeLimit;

        if (!this._sensitivityTimeoutId && Math.abs(this._slider.value - p.Sensitivity) > 0.005) {
            this._syncingSlider = true;
            this._slider.value = p.Sensitivity;
            this._syncingSlider = false;
        }

        const snoozed = this._state === 'snoozed';
        const listening = this._state !== 'standby';
        this._snoozeItems.forEach((item, i) => {
            // "For this call" only makes sense during a call.
            item.visible = !snoozed && (i < 2 || listening);
        });
        this._resumeItem.visible = snoozed;
    }

    _animate() {
        const kind = {warning: 'pulse', alert: 'flash'}[this._state] ?? null;
        if (kind === this._animatedState)
            return;
        this._animatedState = kind;
        this._box.remove_all_transitions();
        this._box.opacity = 255;
        if (kind === 'pulse') {
            // Gentle breathing, one cycle every 1.5 s.
            this._box.ease({
                opacity: 110,
                duration: 750,
                mode: Clutter.AnimationMode.EASE_IN_OUT_SINE,
                repeatCount: -1,
                autoReverse: true,
            });
        } else if (kind === 'flash') {
            // 1 Hz flash.
            this._box.ease({
                opacity: 40,
                duration: 500,
                mode: Clutter.AnimationMode.EASE_IN_OUT_QUAD,
                repeatCount: -1,
                autoReverse: true,
            });
        }
    }

    _drawRing(area) {
        const color = RING_COLORS[this._state];
        if (!color)
            return;
        const cr = area.get_context();
        const [width, height] = area.get_surface_size();
        const lineWidth = Math.max(2, Math.round(height / 10));
        const radius = Math.min(width, height) / 2 - lineWidth / 2;
        const [cx, cy] = [width / 2, height / 2];

        cr.setLineWidth(lineWidth);
        cr.setSourceRGBA(...color, RING_TRACK_ALPHA);
        cr.arc(cx, cy, radius, 0, 2 * Math.PI);
        cr.stroke();

        if (this._progress > 0) {
            const start = -Math.PI / 2;
            cr.setSourceRGBA(...color, 1);
            cr.arc(cx, cy, radius, start, start + this._progress * 2 * Math.PI);
            cr.stroke();
        }
        cr.$dispose();
    }

    destroy() {
        if (this._sensitivityTimeoutId) {
            GLib.source_remove(this._sensitivityTimeoutId);
            this._sensitivityTimeoutId = 0;
        }
        if (this._proxy) {
            this._proxyIds.forEach(id => this._proxy.disconnect(id));
            this._proxy.disconnectSignal(this._summaryId);
            this._proxy = null;
        }
        this._box = null;
        super.destroy();
    }
});

export default class RespireExtension extends Extension {
    enable() {
        this._indicator = new RespireIndicator(this);
        Main.panel.addToStatusArea(this.uuid, this._indicator);
    }

    disable() {
        this._indicator?.destroy();
        this._indicator = null;
    }
}
