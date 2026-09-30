#!/bin/bash
# Try the installed extension in a nested GNOME Shell window, without logging out.
#
#   make install && tools/nested.sh
#
# Needs: sudo dnf install mutter-devkit
# Runs on a private D-Bus with in-memory GSettings, so your real session and
# settings are untouched. A second respired runs inside it; it uses the same
# config file and detects calls in your real session (PipeWire is shared).
set -euo pipefail

if [ -z "${RESPIRE_NESTED_BUS:-}" ]; then
    export RESPIRE_NESTED_BUS=1
    exec dbus-run-session -- "$0" "$@"
fi

UUID=respire@mmontiel0904.github.io
RESPIRED=${RESPIRED:-$HOME/.local/bin/respired}
export GSETTINGS_BACKEND=memory

trap 'kill $(jobs -p) 2>/dev/null' EXIT

RUST_LOG=${RUST_LOG:-info} "$RESPIRED" &
gnome-shell --devkit &
SHELL_PID=$!

for _ in $(seq 1 60); do
    gdbus introspect --session --dest org.gnome.Shell.Extensions \
        --object-path /org/gnome/Shell/Extensions > /dev/null 2>&1 && break
    sleep 0.5
done
gnome-extensions enable "$UUID"
gnome-extensions info "$UUID" | grep -E 'State|Error'
echo "Nested shell running. Close its window or press Ctrl+C to stop."
wait "$SHELL_PID"
