$ErrorActionPreference = "Stop"

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

# Compile GSettings schemas
$shareDir = "target\share\glib-2.0\schemas"
New-Item -ItemType Directory -Force -Path $shareDir | Out-Null
Copy-Item "resources\gschemas\org.gnome.meld-rs.gschema.xml" $shareDir
glib-compile-schemas $shareDir
Write-Host "Schemas compiled to $shareDir"

# Build
cargo build @args
