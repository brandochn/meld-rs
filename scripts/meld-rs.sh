#!/usr/bin/env bash
# meld-rs launcher — ensures the correct GTK4 runtime is on PATH.
#
# On Windows with Git for Windows, `C:\Program Files\Git\mingw64\bin`
# sits at the front of PATH and contains older GLib/Cairo/Pango DLLs
# that are incompatible with GTK4.  The Windows DLL loader picks the
# first match, so it never reaches the correct DLLs in MSYS2's
# `C:\msys64\mingw64\bin`.
#
# This script fixes the PATH ordering before launching meld-rs.
# On Linux / macOS it simply execs the binary directly.

set -euo pipefail

# Directory containing this script
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Platform-specific binary name.
case "$(uname -s)" in
MINGW* | MSYS* | CYGWIN*)
    EXE="${SCRIPT_DIR}/meld-rs.exe"
    ;;
*)
    EXE="${SCRIPT_DIR}/meld-rs"
    ;;
esac

# Platform-specific PATH fixup.
case "$(uname -s)" in
MINGW* | MSYS* | CYGWIN*)
    # Windows: put MSYS2 MINGW64 bin at the front of PATH so the
    # correct GTK4 DLLs are found before Git for Windows' copies.
    MINGW64_BIN="/c/msys64/mingw64/bin"
    if [ -d "$MINGW64_BIN" ]; then
        export PATH="${MINGW64_BIN}:${PATH}"
    fi

    # GTK4 GSettings schemas — compiled by build.rs into target/share/
    SCHEMA_DIR="${SCRIPT_DIR}/share/glib-2.0/schemas"
    MINGW64_SHARE="/c/msys64/mingw64/share"
    if [ -n "${XDG_DATA_DIRS:-}" ]; then
        export XDG_DATA_DIRS="${SCHEMA_DIR}:${MINGW64_SHARE}:${XDG_DATA_DIRS}"
    else
        export XDG_DATA_DIRS="${SCHEMA_DIR}:${MINGW64_SHARE}:/usr/local/share:/usr/share"
    fi
    ;;
*)
    # Linux / macOS: no special setup needed.
    ;;
esac

exec "$EXE" "$@"
