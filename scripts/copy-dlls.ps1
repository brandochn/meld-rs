# Copy GTK4 runtime DLLs to the build output directory
# Required because C:\Program Files\Meld\ has conflicting GTK3 DLLs in the system PATH
#
# Uses the MINGW64 runtime set, matching scripts/run.ps1 and scripts/build.ps1.
# (The UCRT64 set was observed to crash with STATUS_ENTRYPOINT_NOT_FOUND.)
$msys2 = "C:\msys64\mingw64\bin"
if (-not (Test-Path "$msys2\libgtk-4-1.dll")) {
    $msys2 = "C:\msys2\mingw64\bin"
}
$target = "$PSScriptRoot\..\target\release"

if (-not (Test-Path $msys2)) {
    Write-Error "MSYS2 MINGW64 not found at $msys2"
    exit 1
}

if (-not (Test-Path $target)) {
    Write-Error "Build output not found at $target. Run cargo build first."
    exit 1
}

Write-Host "Copying GTK4 DLLs from $msys2 to $target..."
Copy-Item "$msys2\*.dll" -Destination $target -Force

# Copy runtime data (icon theme, GSettings schemas, GTK4 data, and
# GtkSourceView language specs/style schemes) so the release binary is
# self-contained. GTK resolves standard/symbolic icon names (e.g.
# "document-save-symbolic") against the icon theme found in
# $XDG_DATA_DIRS/icons — without this the Adwaita icons are missing when the
# exe runs outside an MSYS2 environment.
$msys2Share = Join-Path (Split-Path -Parent $msys2) "share"
$targetShare = Join-Path $target "share"

if (Test-Path $msys2Share) {
    Write-Host "Copying runtime data from $msys2Share to $targetShare..."
    New-Item -ItemType Directory -Force -Path $targetShare | Out-Null
    foreach ($name in @("icons", "glib-2.0", "gtk-4.0", "gtksourceview-5")) {
        $src = Join-Path $msys2Share $name
        if (Test-Path $src) {
            Copy-Item $src -Destination $targetShare -Recurse -Force
        }
    }
}

Write-Host "Done."

# The application's own translation catalogs are produced by build.rs into
# $targetShare\locale (e.g. .../share/locale/de/LC_MESSAGES/meld-rs.mo) and are
# looked up at runtime next to the executable. Warn loudly if they are missing
# so a distributed build is not silently English-only.
$meldRsCatalog = Join-Path $targetShare "locale\de\LC_MESSAGES\meld-rs.mo"
if (-not (Test-Path $meldRsCatalog)) {
    Write-Warning "Translation catalogs not found at $targetShare\locale. Run 'cargo build --release' (build.rs compiles po/*.po with msgfmt) before distributing a portable build."
}
