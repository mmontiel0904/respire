#!/bin/bash
# Render the History & progress window offscreen (GTK broadway backend)
# with made-up calls, and save each page as a PNG.
#
#   tools/prefs-shot/run.sh [output-dir]
#
# Needs gjs, libadwaita and gtk4-broadwayd.
# Touches nothing in your real session: temporary XDG_STATE_HOME, private D-Bus.
set -euo pipefail

if [ -z "${RESPIRE_IN_TEST_BUS:-}" ]; then
    export RESPIRE_IN_TEST_BUS=1
    exec timeout 60 dbus-run-session -- "$0" "$@" 2> >(grep -vE '^(dbus-daemon|fusermount)' >&2)
fi

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
OUT=$(realpath -m "${1:-$REPO/build/shots}")
WORK=$(mktemp -d)
trap 'kill $(jobs -p) 2>/dev/null; rm -rf "$WORK"' EXIT
mkdir -p "$OUT"
rm -f "$OUT"/prefs-*.png "$OUT/prefs-done"

DISPLAY_NUM=:$((50 + RANDOM % 40))
gtk4-broadwayd "$DISPLAY_NUM" > /dev/null 2>&1 &
sleep 0.5

export GDK_BACKEND=broadway BROADWAY_DISPLAY="$DISPLAY_NUM" GSETTINGS_BACKEND=memory GIO_USE_VFS=local \
    ADW_DISABLE_PORTAL=1 GTK_A11Y=none GDK_DEBUG=no-portals
# The shell's own prefs module only works inside its D-Bus service, so use a
# copy of the extension whose ExtensionPreferences import points at a stub.
cp -r "$REPO/extension" "$WORK/extension"
cp "$HERE/stub-prefs.js" "$HERE/prefs-shot.js" "$WORK/extension/"
sed -i "s#'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js'#'./stub-prefs.js'#" \
    "$WORK/extension/prefs.js"

for mode in data empty; do
    rm -rf "$WORK/state"
    XDG_STATE_HOME="$WORK/state" gjs -m "$WORK/extension/prefs-shot.js" "$OUT" "$mode" 2>&1 \
        | grep -vE 'Gtk-WARNING.*(Theme parser|Unable to acquire)' || true
    result=$(cat "$OUT/prefs-done" 2>/dev/null || echo "no result")
    echo "prefs ($mode): $result"
    [ "$result" = ok ] || exit 1
    rm -f "$OUT/prefs-done"
done
echo "screenshots in $OUT"
