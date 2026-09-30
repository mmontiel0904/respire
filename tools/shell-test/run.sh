#!/bin/bash
# Load the extension in a throwaway headless GNOME Shell with a mock daemon,
# cycle through every state, and save screenshots.
#
#   tools/shell-test/run.sh [output-dir]
#
# Needs gnome-shell (same major version as the host) and gjs. Touches
# nothing in your real session: separate D-Bus, in-memory GSettings.
set -euo pipefail

if [ -z "${RESPIRE_IN_TEST_BUS:-}" ]; then
    export RESPIRE_IN_TEST_BUS=1
    exec timeout 90 dbus-run-session -- "$0" "$@" 2>/dev/null
fi

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
OUT=$(realpath -m "${1:-$REPO/build/shots}")
WORK=$(mktemp -d)
trap 'kill $(jobs -p) 2>/dev/null; rm -rf "$WORK"' EXIT

mkdir -p "$OUT" "$WORK/extensions"
rm -f "$OUT"/*.png "$OUT/done"
cp -r "$REPO/extension" "$WORK/extensions/respire@mmontiel0904.github.io"
cp -r "$HERE/harness@respire" "$WORK/extensions/"
mkdir -p "$WORK/data/gnome-shell"
mv "$WORK/extensions" "$WORK/data/gnome-shell/extensions"

export GSETTINGS_BACKEND=memory XDG_DATA_HOME="$WORK/data" RESPIRE_SHOTS="$OUT"
gnome-shell --headless --virtual-monitor 1280x720 > "$OUT/shell.log" 2>&1 &
for _ in $(seq 1 30); do
    gdbus introspect --session --dest org.gnome.Shell.Extensions \
        --object-path /org/gnome/Shell/Extensions > /dev/null 2>&1 && break
    sleep 0.5
done

gjs -m "$HERE/mock-daemon.js" &
sleep 1
gnome-extensions enable respire@mmontiel0904.github.io
sleep 1
gnome-extensions info respire@mmontiel0904.github.io | grep -E 'State|Error'
gnome-extensions enable harness@respire
for _ in $(seq 1 60); do [ -f "$OUT/done" ] && break; sleep 0.5; done

result=$(cat "$OUT/done" 2>/dev/null || echo "timed out")
echo "harness: $result"
grep -E 'JS ERROR|JS WARNING' "$OUT/shell.log" && exit 1
[ "$result" = ok ]
echo "screenshots in $OUT"
