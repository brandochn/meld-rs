# Install Meld-rs desktop-integration files — the application icon and the
# .desktop launcher entry — into an MSYS2 MINGW64 prefix so the icon is
# available to GTK and to launchers.
#
# The icon is installed to <prefix>\share\icons\hicolor\scalable\apps\, matching
# the `Icon=org.gnome.meld-rs` key in the desktop entry.  The binary itself is
# not installed by this script.
#
# Usage:
#   scripts\install-desktop.ps1                       # auto-detect MSYS2 MINGW64
#   scripts\install-desktop.ps1 -Prefix C:\msys64\mingw64
param(
    [string]$Prefix
)

$ErrorActionPreference = "Stop"

# Auto-detect MSYS2 MINGW64 (same locations as scripts/run.ps1).
if (-not $Prefix) {
    foreach ($cand in @("C:\msys64\mingw64", "C:\msys2\mingw64")) {
        if (Test-Path "$cand\bin\libgtk-4-1.dll") { $Prefix = $cand; break }
    }
}
if (-not $Prefix -or -not (Test-Path $Prefix)) {
    Write-Error "MSYS2 MINGW64 prefix not found. Pass -Prefix <path>."
    exit 1
}

$repo = (Resolve-Path "$PSScriptRoot\..").Path
$share = Join-Path $Prefix "share"
$iconDir = Join-Path $share "icons\hicolor\scalable\apps"
$appsDir = Join-Path $share "applications"

New-Item -ItemType Directory -Force -Path $iconDir, $appsDir | Out-Null
Copy-Item "$repo\resources\icons\scalable\apps\org.gnome.meld-rs.svg" -Destination $iconDir -Force

$desktopTemplate = "$repo\resources\org.gnome.meld-rs.desktop.in"
$desktopOut = Join-Path $appsDir "org.gnome.meld-rs.desktop"

$msgfmt = Get-Command msgfmt -ErrorAction SilentlyContinue
if ($msgfmt) {
    & $msgfmt.Source --desktop "--template=$desktopTemplate" -d "$repo\po" -o $desktopOut
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "msgfmt failed; installing the English template"
        Copy-Item $desktopTemplate -Destination $desktopOut -Force
    }
} else {
    Write-Warning "msgfmt not found; installing the English template"
    Copy-Item $desktopTemplate -Destination $desktopOut -Force
}

$validator = Get-Command desktop-file-validate -ErrorAction SilentlyContinue
if ($validator) {
    & $validator.Source $desktopOut
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "desktop-file-validate reported problems"
    }
}

Write-Host "Installed icon     -> $iconDir\org.gnome.meld-rs.svg"
Write-Host "Installed launcher -> $desktopOut"

# Refresh the icon cache if the tool is available.
$iconCacheDir = Join-Path $share "icons\hicolor"
if ((Test-Path "$iconCacheDir\index.theme") -and (Get-Command gtk4-update-icon-cache -ErrorAction SilentlyContinue)) {
    & gtk4-update-icon-cache -q -t -f $iconCacheDir 2>$null
}

Write-Host "Done."
