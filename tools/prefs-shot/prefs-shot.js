// Render the History & progress window with made-up calls and save each
// page as a PNG. Run through tools/prefs-shot/run.sh, which provides an
// offscreen (broadway) display, a temporary XDG_STATE_HOME, and a copy of
// the extension with a stub ExtensionPreferences.
import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Graphene from 'gi://Graphene';
import Gtk from 'gi://Gtk';

// run.sh copies this file next to prefs.js. The import must be static: a
// top-level await would stop promise jobs from running inside app.run().
import RespirePreferences from './prefs.js';

const [out, mode] = ARGV;
const extDir = Gio.File.new_for_path(GLib.path_get_dirname(import.meta.url.slice(7)));

// Twelve weeks of calls that get a little better every week.
function fakeLog() {
    const lines = [];
    const now = Date.now() / 1000;
    const day = 86400;
    for (let d = 84; d >= 0; d--) {
        if (d % 7 === 5 || d % 7 === 6 || d % 3 === 0)
            continue;
        const progress = 1 - d / 84;
        const start = Math.floor(now - d * day - 3 * 3600);
        const minutes = 25 + (d % 4) * 10;
        const talk = Math.round(minutes * 60 * (0.55 - 0.2 * progress));
        const others = minutes * 60 - talk - 300;
        const line = {
            start, end: start + minutes * 60, apps: [d % 2 ? 'Firefox' : 'Slack'], limit_sec: 90,
            turns: 10 + (d % 5), talk_sec: talk, longest_turn_sec: Math.round(140 - 70 * progress),
            warnings: Math.round(4 - 3 * progress), alerts: Math.round(3 - 2.6 * progress),
        };
        if (d < 60) {
            Object.assign(line, {
                v: 2, median_turn_sec: Math.round(32 - 16 * progress), others_talk_sec: others,
                talk_share: Math.round(talk / (talk + others) * 100) / 100,
                interruptions: Math.round(4 - 3 * progress),
            });
        }
        lines.push(JSON.stringify(line));
    }
    return `${lines.join('\n')}\n`;
}

const stateDir = GLib.build_filenamev([GLib.get_user_state_dir(), 'respire']);
GLib.mkdir_with_parents(stateDir, 0o700);
if (mode !== 'empty')
    GLib.file_set_contents(`${stateDir}/sessions.jsonl`, fakeLog());

function save(window, file) {
    // A paintable of the toplevel itself draws nothing, so paint its child.
    const widget = window.get_first_child();
    const paintable = new Gtk.WidgetPaintable({widget});
    const snapshot = new Gtk.Snapshot();
    paintable.snapshot(snapshot, widget.get_width(), widget.get_height());
    const node = snapshot.to_node();
    if (!node)
        throw new Error(`nothing drawn for ${file}`);
    // Without a viewport the texture is cropped to what was drawn.
    const viewport = new Graphene.Rect().init(0, 0, widget.get_width(), widget.get_height());
    const texture = window.get_renderer().render_texture(node, viewport);
    texture.save_to_png(file);
}

const wait = ms => new Promise(r => GLib.timeout_add(GLib.PRIORITY_DEFAULT, ms, () => {
    r();
    return GLib.SOURCE_REMOVE;
}));

const app = new Adw.Application({application_id: 'io.github.mmontiel0904.Respire.PrefsShot'});
app.connect('activate', async () => {
    try {
        const window = new Adw.PreferencesWindow({application: app});
        const prefs = new RespirePreferences({uuid: 'respire@mmontiel0904.github.io', path: extDir.get_path(), dir: extDir});
        prefs.fillPreferencesWindow(window);
        window.set_default_size(640, 1100);
        window.present();
        await wait(800);
        for (const name of ['progress', 'history']) {
            window.visible_page_name = name;
            window.queue_draw();
            await wait(1000);
            save(window, `${out}/prefs-${mode || 'data'}-${name}.png`);
        }
        GLib.file_set_contents(`${out}/prefs-done`, 'ok');
    } catch (e) {
        GLib.file_set_contents(`${out}/prefs-done`, `error: ${e}\n${e.stack}`);
    }
    app.quit();
});
app.run([]);
