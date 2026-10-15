#!/usr/bin/env bash
# Build a Linux AppImage for meld-rs.
#
# Produces: publish/meld-rs-<version>-<arch>.AppImage
#
# The AppDir is assembled by hand (binary, desktop entry, icon, GSettings
# schemas, gettext catalogs and the GTK runtime data dirs), then linuxdeploy
# bundles the shared libraries and generates the AppImage itself.
#
# Prerequisites (see PLAN.md Step 0): cargo, patchelf, glib-compile-schemas,
# wget.  linuxdeploy is downloaded into target/appimage-tools/ on first run.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd "${script_dir}/.." && pwd)"
cd "${repo_dir}"

app_id="org.gnome.meld-rs"
desktop_src="resources/${app_id}.desktop"
icon_src="resources/icons/scalable/apps/${app_id}.svg"
apprun_src="packaging/AppRun"

# Version comes straight from Cargo.toml so the artifact name never drifts.
version="$(sed -n 's/^version = "\([^"]*\)".*/\1/p' Cargo.toml)"
arch="$(uname -m)"

tools_dir="${repo_dir}/target/appimage-tools"
build_dir="${repo_dir}/target/appimage"
appdir="${build_dir}/Meld-rs.AppDir"
output_dir="${repo_dir}/publish"
output_file="${output_dir}/meld-rs-${version}-${arch}.AppImage"

# ── Fail fast if a required tool is missing ───────────────────────────────
for tool in cargo patchelf glib-compile-schemas wget; do
    if ! command -v "${tool}" >/dev/null 2>&1; then
        echo "ERROR: required tool '${tool}' not found in PATH." >&2
        echo "See PLAN.md Step 0 for the prerequisites." >&2
        exit 1
    fi
done

if [ ! -f "${apprun_src}" ]; then
    echo "ERROR: missing AppRun source: ${apprun_src}" >&2
    exit 1
fi
for src in "${desktop_src}" "${icon_src}"; do
    if [ ! -f "${src}" ]; then
        echo "ERROR: missing source file: ${src}" >&2
        exit 1
    fi
done

# ── 1. Release build ──────────────────────────────────────────────────────
echo "==> Building release binary"
cargo build --release

binary="${repo_dir}/target/release/meld-rs"
if [ ! -x "${binary}" ]; then
    echo "ERROR: release binary not found at ${binary}" >&2
    exit 1
fi

# ── 2. Fetch linuxdeploy (cached under target/) ───────────────────────────
mkdir -p "${tools_dir}"
linuxdeploy="${tools_dir}/linuxdeploy-x86_64.AppImage"
if [ ! -x "${linuxdeploy}" ]; then
    echo "==> Downloading linuxdeploy"
    wget -q --show-progress -O "${linuxdeploy}" \
        "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage"
    chmod +x "${linuxdeploy}"
fi

# ── 3. Assemble the AppDir ────────────────────────────────────────────────
echo "==> Assembling AppDir at ${appdir}"
rm -rf "${appdir}"
mkdir -p \
    "${appdir}/usr/bin" \
    "${appdir}/usr/share/applications" \
    "${appdir}/usr/share/glib-2.0/schemas"

# Payload binary.
install -m 0755 "${binary}" "${appdir}/usr/bin/meld-rs"

# GTK runtime data dirs (mirrors scripts/copy-dlls.sh): the Adwaita/hicolor
# icon themes and the GtkSourceView language specs.  Copied before our own
# icon so the merge does not clobber it.
for name in icons gtksourceview-5 gtk-4.0; do
    src="/usr/share/${name}"
    if [ -d "${src}" ]; then
        mkdir -p "${appdir}/usr/share/${name}"
        cp -rf "${src}/." "${appdir}/usr/share/${name}/"
    fi
done

# Desktop entry + application icon (also used for AppImage desktop integration).
mkdir -p "${appdir}/usr/share/icons/hicolor/scalable/apps"
install -m 0644 "${desktop_src}" "${appdir}/usr/share/applications/${app_id}.desktop"
install -m 0644 "${icon_src}" "${appdir}/usr/share/icons/hicolor/scalable/apps/${app_id}.svg"

# Copies at the AppDir root: appimagetool uses these for the AppImage's own
# embedded desktop entry and icon.
install -m 0644 "${desktop_src}" "${appdir}/${app_id}.desktop"
install -m 0644 "${icon_src}" "${appdir}/${app_id}.svg"

# GSettings schemas.  Our own schema is compiled together with the system set
# so GTK4's org.gtk.gtk4.Settings.* schemas (needed by the file chooser) are
# present on hosts without GTK installed.
schema_dir="${appdir}/usr/share/glib-2.0/schemas"
if [ -d /usr/share/glib-2.0/schemas ]; then
    find /usr/share/glib-2.0/schemas -maxdepth 1 \
        \( -name '*.gschema.xml' -o -name '*.override' \) \
        -exec cp -f {} "${schema_dir}/" \;
fi
install -m 0644 "resources/gschemas/${app_id}.gschema.xml" "${schema_dir}/"
glib-compile-schemas "${schema_dir}"

# gettext catalogs compiled by build.rs into target/release/share/locale.
if [ -d target/release/share/locale ]; then
    mkdir -p "${appdir}/usr/share/locale"
    cp -rf target/release/share/locale/. "${appdir}/usr/share/locale/"
fi

# Entrypoint.
install -m 0755 "${apprun_src}" "${appdir}/AppRun"

# ── 4. Run linuxdeploy: bundle libraries + build the AppImage ─────────────
echo "==> Running linuxdeploy"
mkdir -p "${output_dir}"
rm -f "${output_file}"

export VERSION="${version}"
export ARCH="${arch}"
export OUTPUT="${output_file}"

"${linuxdeploy}" \
    --appdir "${appdir}" \
    -e "${appdir}/usr/bin/meld-rs" \
    -d "${desktop_src}" \
    -i "${icon_src}" \
    --output appimage

# ── 5. Report ─────────────────────────────────────────────────────────────
if [ ! -f "${output_file}" ]; then
    echo "ERROR: expected AppImage not found at ${output_file}" >&2
    exit 1
fi

chmod +x "${output_file}"
echo "==> Done: ${output_file}"
