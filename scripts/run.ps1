$ErrorActionPreference = "Stop"

Write-Host "=== Meld-rs build & run script ===" -ForegroundColor Cyan

# Make sure no previous instance is running (binary would be locked)
$exePath = ".\target\debug\meld-rs.exe"
if (Test-Path $exePath) {
    try {
        $file = [System.IO.File]::OpenWrite($exePath)
        $file.Close()
    } catch {
        Write-Host "ERROR: The binary is locked. Close all running meld-rs instances and try again." -ForegroundColor Red
        exit 1
    }
}

# Locate MSYS2 MINGW64 and add to PATH
$msys2Dir = "C:\msys64\mingw64\bin"
if (-not (Test-Path "$msys2Dir\libgtk-4-1.dll")) {
    $msys2Dir = "C:\msys2\mingw64\bin"
}
if (Test-Path "$msys2Dir\libgtk-4-1.dll") {
    $env:PATH = "$msys2Dir;$env:PATH"
    $env:MINGW_PREFIX = (Split-Path -Parent $msys2Dir)
    Write-Host "MSYS2 MINGW64 found at $msys2Dir"
}
else {
    Write-Host "WARNING: MSYS2 MINGW64 not found - GTK4 DLLs may not be available"
    Write-Host "Install with: pacman -S mingw-w64-x86_64-gtk4"
}

# Compile GSettings schemas (if not already compiled)
$shareDir = "target\share\glib-2.0\schemas"
if (-not (Test-Path "$shareDir\gschemas.compiled")) {
    New-Item -ItemType Directory -Force -Path $shareDir | Out-Null
    Copy-Item "resources\gschemas\org.gnome.meld-rs.gschema.xml" $shareDir
    glib-compile-schemas $shareDir
    Write-Host "Schemas compiled to $shareDir"
}

# Enable debug logging so we can see diff computation output
$env:RUST_LOG = "debug"

Write-Host "Running cargo run..." -ForegroundColor Green
cargo run -- $args
if ($LASTEXITCODE -ne 0) {
    Write-Host "ERROR: Application exited with code $LASTEXITCODE" -ForegroundColor Red
}
