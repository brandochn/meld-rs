#!/bin/bash
# Copy GTK4 runtime DLLs and data files next to the release binary so it can
# run standalone (Bash equivalent of scripts/copy-dlls.ps1).
#
# Intended for Git Bash / MSYS2 MINGW64 on Windows, where the Adwaita icon
# theme and GSettings schemas must be bundled next to the executable because
# GTK resolves icon names against $XDG_DATA_DIRS/icons at runtime.
set -e

# Detect the MINGW64 prefix that actually contains the GTK4 runtime.
#
# In a standalone MSYS2 MINGW64 shell this is $MINGW_PREFIX or /mingw64, but
# in Git for Windows (Git Bash) those point at Git's own tree, so we also
# check the common standalone MSYS2 install locations.
candidates=()

# Standalone MSYS2 installs (the usual case on Windows).
candidates+=(/c/msys64/mingw64 /c/msys2/mingw64)

# If running inside an MSYS2 MINGW64 shell, these point at the real prefix.
if [ -n "$MINGW_PREFIX" ]; then
    candidates+=("$MINGW_PREFIX")
fi
candidates+=(/mingw64)

prefix=""
for cand in "${candidates[@]}"; do
    if [ -f "$cand/bin/libgtk-4-1.dll" ]; then
        prefix="$cand"
        break
    fi
done

if [ -z "$prefix" ]; then
    echo "ERROR: MINGW64 with GTK4 not found. Checked:" >&2
    for cand in "${candidates[@]}"; do
        echo "  - $cand" >&2
    done
    echo "Install GTK4 with: pacman -S mingw-w64-x86_64-gtk4" >&2
    exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
target="$script_dir/../target/release"

if [ ! -d "$target" ]; then
    echo "ERROR: Build output not found at $target. Run 'cargo build --release' first." >&2
    exit 1
fi

echo "Copying GTK4 DLLs from $prefix/bin to $target..."
cp -f "$prefix"/bin/*.dll "$target"/

echo "Copying runtime data from $prefix/share to $target/share..."
mkdir -p "$target/share"
for name in icons glib-2.0 gtk-4.0 gtksourceview-5; do
    src="$prefix/share/$name"
    if [ -d "$src" ]; then
        # Remove the destination first so repeated runs don't nest the copy.
        rm -rf "$target/share/$name"
        cp -rf "$src" "$target/share/"
    fi
done

echo "Done."
