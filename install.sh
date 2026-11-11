#!/bin/sh
# Build and install Stales.
#
#   ./install.sh                  installs for you only, into ~/.local (no sudo needed)
#   sudo PREFIX=/usr/local ./install.sh   installs for everyone
#   ./install.sh uninstall        removes it again
#
# Needs: Rust (cargo), GTK 4.14+ and WebKitGTK 6.0 (2.40+) with their development files.
#   Arch:   sudo pacman -S --needed rust gtk4 webkitgtk-6.0
#   Ubuntu: sudo apt install cargo libgtk-4-dev libwebkitgtk-6.0-dev
set -eu

PREFIX="${PREFIX:-$HOME/.local}"
APP_ID="dev.stales.Stales"
cd "$(dirname "$0")"

if [ "${1:-}" = "uninstall" ]; then
    rm -f "$PREFIX/bin/stales" \
          "$PREFIX/share/applications/$APP_ID.desktop" \
          "$PREFIX/share/icons/hicolor/scalable/apps/$APP_ID.svg" \
          "$PREFIX/share/metainfo/$APP_ID.metainfo.xml"
    echo "Stales removed from $PREFIX. Your settings and tabs are still in ~/.config/stales and ~/.local/share/stales."
    exit 0
fi

if [ "$(id -u)" -eq 0 ] && [ -n "${SUDO_USER:-}" ] && [ ! -x target/release/stales ]; then
    echo "Build first without sudo (cargo build --release), then run this with sudo." >&2
    exit 1
fi
[ "$(id -u)" -eq 0 ] || cargo build --release --locked

install -Dm755 target/release/stales "$PREFIX/bin/stales"
install -Dm644 "data/$APP_ID.desktop" "$PREFIX/share/applications/$APP_ID.desktop"
install -Dm644 "data/icons/$APP_ID.svg" "$PREFIX/share/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm644 "data/$APP_ID.metainfo.xml" "$PREFIX/share/metainfo/$APP_ID.metainfo.xml"
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true

echo "Installed Stales to $PREFIX/bin/stales"
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "Note: $PREFIX/bin is not in your PATH." ;; esac
echo "Make it your default browser: xdg-settings set default-web-browser $APP_ID.desktop"
