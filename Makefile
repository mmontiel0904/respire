PREFIX      ?= $(HOME)/.local
BINDIR      ?= $(PREFIX)/bin
DATADIR     ?= $(PREFIX)/share
SYSTEMD_DIR ?= $(HOME)/.config/systemd/user
UUID        := respire@mmontiel0904.github.io
EXT_DIR     := $(DATADIR)/gnome-shell/extensions/$(UUID)
BUS_NAME    := io.github.mmontiel0904.Respire

.PHONY: all build test lint shell-test prefs-shot nested install install-daemon install-extension uninstall pack run

all: build

build:
	cargo build --release --manifest-path respired/Cargo.toml

test:
	cargo test --manifest-path respired/Cargo.toml
	gjs -m tools/history-test.mjs

lint:
	cargo fmt --manifest-path respired/Cargo.toml --check
	cargo clippy --manifest-path respired/Cargo.toml -- -D warnings

# Load the extension in a headless GNOME Shell against a mock daemon; screenshots go to build/shots.
shell-test:
	tools/shell-test/run.sh

# Render the History & progress window offscreen with made-up calls; screenshots go to build/shots.
prefs-shot:
	tools/prefs-shot/run.sh

# Try the installed extension in a nested GNOME Shell window (needs mutter-devkit).
nested: install-extension
	tools/nested.sh

# Run the daemon in the foreground with debug logs (stop the service first).
run:
	RUST_LOG=respired=debug cargo run --manifest-path respired/Cargo.toml

install: install-daemon install-extension
	@echo
	@echo "Done. Log out and back in (Wayland), then run:"
	@echo "  gnome-extensions enable $(UUID)"

install-daemon: build
	install -Dm755 respired/target/release/respired $(BINDIR)/respired
	mkdir -p $(SYSTEMD_DIR) $(DATADIR)/dbus-1/services
	sed 's|@BINDIR@|$(BINDIR)|g' systemd/respired.service.in > $(SYSTEMD_DIR)/respired.service
	sed 's|@BINDIR@|$(BINDIR)|g' dbus/$(BUS_NAME).service.in > $(DATADIR)/dbus-1/services/$(BUS_NAME).service
	systemctl --user daemon-reload
	systemctl --user enable --now respired.service
	systemctl --user restart respired.service

install-extension:
	install -Dm644 -t $(EXT_DIR) extension/metadata.json extension/extension.js extension/prefs.js \
		extension/history.js extension/stylesheet.css

uninstall:
	-systemctl --user disable --now respired.service
	rm -f $(BINDIR)/respired $(SYSTEMD_DIR)/respired.service $(DATADIR)/dbus-1/services/$(BUS_NAME).service
	rm -rf $(EXT_DIR)
	systemctl --user daemon-reload

# Zip for extensions.gnome.org (the daemon is installed separately).
pack:
	mkdir -p build
	gnome-extensions pack --force --out-dir=build --extra-source=history.js extension
