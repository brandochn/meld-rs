#!/usr/bin/env bash
# Install Meld-rs desktop-integration files — the application icon and the
# .desktop launcher entry — so the app shows up with its own icon in the
# desktop environment (launcher, task bar, and application menu).
#
# The icon is installed to <data>/icons/hicolor/scalable/apps/, matching the
# `Icon=org.gnome.meld-rs` key in the desktop entry.  The binary itself is not
# installed by this script.
#
# Usage:
#   ./scripts/install-desktop.sh [PREFIX]
#
# With no argument the files are installed per-user into $XDG_DATA_HOME
# (default ~/.local/share), which needs no privileges.  Pass a prefix
# (e.g. /usr/local) to install system-wide instead.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd "$script_dir/.." && pwd)"

if [ "$#" -ge 1 ]; then
    data_dir="$1/share"
else
    data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
fi

icon_src="$repo_dir/resources/icons/scalable/apps/org.gnome.meld-rs.svg"
desktop_template="$repo_dir/resources/org.gnome.meld-rs.desktop.in"

icon_dir="$data_dir/icons/hicolor/scalable/apps"
apps_dir="$data_dir/applications"
desktop_out="$apps_dir/org.gnome.meld-rs.desktop"

mkdir -p "$icon_dir" "$apps_dir"
cp -f "$icon_src" "$icon_dir/org.gnome.meld-rs.svg"

if command -v msgfmt >/dev/null 2>&1; then
    if ! msgfmt --desktop --template="$desktop_template" -d "$repo_dir/po" -o "$desktop_out"; then
        echo "warning: msgfmt failed; installing the English template" >&2
        cp -f "$desktop_template" "$desktop_out"
    fi
else
    echo "warning: msgfmt not found; installing the English template" >&2
    cp -f "$desktop_template" "$desktop_out"
fi

if command -v desktop-file-validate >/dev/null 2>&1; then
    if ! desktop-file-validate "$desktop_out"; then
        echo "warning: desktop-file-validate reported problems" >&2
    fi
fi

echo "Installed icon     -> $icon_dir/org.gnome.meld-rs.svg"
echo "Installed launcher -> $desktop_out"

# Refresh desktop caches so the new icon and entry are picked up without
# requiring a re-login.
icon_cache_dir="$data_dir/icons/hicolor"
if [ -f "$icon_cache_dir/index.theme" ]; then
    if command -v gtk4-update-icon-cache >/dev/null 2>&1; then
        gtk4-update-icon-cache -q -t -f "$icon_cache_dir" 2>/dev/null || true
    elif command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t -f "$icon_cache_dir" 2>/dev/null || true
    fi
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$apps_dir" 2>/dev/null || true
fi

echo "Done."
