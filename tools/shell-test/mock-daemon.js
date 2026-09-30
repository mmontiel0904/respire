// Mock respired: same interface plus a Set() method the test harness drives.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
const XML = `<node><interface name="io.github.mmontiel0904.Respire1">
<method name="SetLimit"><arg type="u" direction="in"/></method>
<method name="SetSensitivity"><arg type="d" direction="in"/></method>
<method name="Snooze"><arg type="u" direction="in"/></method>
<method name="Resume"/>
<property name="State" type="s" access="read"/><property name="TalkingTime" type="u" access="read"/>
<property name="TimeLimit" type="u" access="read"/><property name="Progress" type="d" access="read"/>
<property name="Sensitivity" type="d" access="read"/><property name="MeetingApps" type="as" access="read"/>
<property name="SnoozeRemaining" type="u" access="read"/><property name="TurnCount" type="u" access="read"/>
<property name="LongestTurn" type="u" access="read"/><property name="AlertCount" type="u" access="read"/>
<property name="TalkShare" type="d" access="read"/><property name="InterruptionCount" type="u" access="read"/>
<signal name="SessionFinished"><arg type="s"/></signal>
<method name="Finish"/>
<method name="Set"><arg type="s" direction="in"/><arg type="u" direction="in"/></method>
</interface></node>`;
const props = {State: 'standby', TalkingTime: 0, TimeLimit: 90, Progress: 0, Sensitivity: 0.5,
  MeetingApps: [], SnoozeRemaining: 0, TurnCount: 3, LongestTurn: 72, AlertCount: 1,
  TalkShare: 0.42, InterruptionCount: 1};
const types = {State: 's', TalkingTime: 'u', TimeLimit: 'u', Progress: 'd', Sensitivity: 'd',
  MeetingApps: 'as', SnoozeRemaining: 'u', TurnCount: 'u', LongestTurn: 'u', AlertCount: 'u',
  TalkShare: 'd', InterruptionCount: 'u'};
const obj = {
  SetLimit(s) { update({TimeLimit: s}); }, SetSensitivity(v) { update({Sensitivity: v}); },
  Snooze(m) { update({State: 'snoozed', SnoozeRemaining: m * 60}); }, Resume() { update({State: 'idle'}); },
  // Pretend a 38-minute call just ended.
  Finish() {
    const end = Math.floor(Date.now() / 1000);
    const summary = {v: 2, start: end - 38 * 60, end, apps: ['Firefox'], limit_sec: 90, turns: 12,
      talk_sec: 610, longest_turn_sec: 112, median_turn_sec: 24, warnings: 3, alerts: 2,
      others_talk_sec: 845, talk_share: 0.42, interruptions: 1};
    update({State: 'standby', MeetingApps: []});
    exported.emit_signal('SessionFinished', new GLib.Variant('(s)', [JSON.stringify(summary)]));
  },
  Set(state, talking) {
    update({State: state, TalkingTime: talking, Progress: Math.min(1, talking / props.TimeLimit),
      MeetingApps: state === 'standby' ? [] : ['Firefox']});
  },
};
for (const k of Object.keys(props)) Object.defineProperty(obj, k, {get: () => props[k]});
const exported = Gio.DBusExportedObject.wrapJSObject(XML, obj);
function update(changes) {
  Object.assign(props, changes);
  for (const k of Object.keys(changes)) exported.emit_property_changed(k, new GLib.Variant(types[k], props[k]));
  exported.flush();
}
exported.export(Gio.DBus.session, '/io/github/mmontiel0904/Respire');
Gio.DBus.session.own_name('io.github.mmontiel0904.Respire', Gio.BusNameOwnerFlags.NONE, null, null);
new GLib.MainLoop(null, false).run();
