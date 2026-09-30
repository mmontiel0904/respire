// Test-only extension: drives the mock daemon through every state and
// saves screenshots of the indicator and the popover to $RESPIRE_SHOTS.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const OUT = GLib.getenv('RESPIRE_SHOTS');
const sleep = ms => new Promise(r => GLib.timeout_add(GLib.PRIORITY_DEFAULT, ms, () => { r(); return GLib.SOURCE_REMOVE; }));
function mockCall(method, args) {
  return new Promise((resolve, reject) => Gio.DBus.session.call('io.github.mmontiel0904.Respire',
    '/io/github/mmontiel0904/Respire', 'io.github.mmontiel0904.Respire1', method,
    args, null, Gio.DBusCallFlags.NONE, -1, null,
    (c, res) => { try { c.call_finish(res); resolve(); } catch (e) { reject(e); } }));
}
const mockSet = (state, talking) => mockCall('Set', new GLib.Variant('(su)', [state, talking]));
async function shot(name, x, y, w, h) {
  const file = Gio.File.new_for_path(`${OUT}/${name}.png`);
  const stream = file.replace(null, false, Gio.FileCreateFlags.NONE, null);
  await new Shell.Screenshot().screenshot_area(x, y, w, h, stream);
  stream.close(null);
}
async function run() {
  try {
    Main.overview.hide();
    await sleep(1500);
    // Close the first-login welcome dialog, which dims the whole screen.
    for (const a of Main.layoutManager.modalDialogGroup.get_children()) a.destroy();
    await sleep(500);
    const W = global.stage.width;
    const steps = [['standby', 0], ['idle', 0], ['speaking', 30], ['warning', 72], ['alert', 95], ['snoozed', 0]];
    for (const [state, t] of steps) {
      await mockSet(state, t);
      await sleep(800);
      await shot(`bar-${state}`, W - 260, 0, 260, 36);
    }
    await mockSet('speaking', 42);
    const ind = Main.panel.statusArea['respire@mmontiel0904.github.io'];
    ind.menu.open();
    await sleep(800);
    await shot('menu', W - 460, 0, 460, 520);
    ind.menu.close();
    await mockSet('alert', 95);
    await sleep(600);
    await shot('alert-notification', W / 2 - 300, 0, 600, 160);
    // Headless, the alert banner never times out. Drop it so the next one shows.
    for (const source of Main.messageTray.getSources()) source.destroy();
    await sleep(800);
    await mockCall('Finish', null);
    await sleep(1200);
    await shot('summary-notification', W / 2 - 300, 0, 600, 160);
    const summary = Main.messageTray.getSources().flatMap(s => s.notifications)
      .find(n => n.title === 'Respire · call summary');
    if (!summary) throw new Error('no call summary notification');
    GLib.file_set_contents(`${OUT}/summary.txt`, summary.body);
    GLib.file_set_contents(`${OUT}/done`, 'ok');
  } catch (e) {
    GLib.file_set_contents(`${OUT}/done`, `error: ${e}\n${e.stack}`);
  }
}
export default class { enable() { run(); } disable() {} }
