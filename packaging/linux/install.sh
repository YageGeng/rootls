#!/usr/bin/env bash
# Installs rtools for the current user: the release binary on PATH, the desktop
# entry, and the app icon under the name both the entry and the windows' Wayland
# app id (dev.rtools.rtools) refer to, so menus and the task manager show it.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
id=dev.rtools.rtools

cargo build --release --manifest-path "$root/Cargo.toml"
install -Dm755 -s "$root/target/release/rtools" "$bin/rtools"
install -Dm644 "$root/packaging/linux/$id.desktop" "$data/applications/$id.desktop"
install -Dm644 "$root/assets/icons/rtools.svg" "$data/icons/hicolor/scalable/apps/$id.svg"
for size in 32 256; do
    install -Dm644 "$root/assets/icons/rtools-$size.png" "$data/icons/hicolor/${size}x${size}/apps/$id.png"
done
echo "installed $bin/rtools; double-tap Ctrl needs your user in the input group"
