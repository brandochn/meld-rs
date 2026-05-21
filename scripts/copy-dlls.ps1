# Copy GTK4 runtime DLLs to the build output directory
# Required because C:\Program Files\Meld\ has conflicting GTK3 DLLs in the system PATH

$msys2 = "C:\msys64\ucrt64\bin"
$target = "$PSScriptRoot\..\target\release"

if (-not (Test-Path $msys2)) {
    Write-Error "MSYS2 UCRT64 not found at $msys2"
    exit 1
}

if (-not (Test-Path $target)) {
    Write-Error "Build output not found at $target. Run cargo build first."
    exit 1
}

Write-Host "Copying GTK4 DLLs from $msys2 to $target..."
Copy-Item "$msys2\*.dll" -Destination $target -Force
Write-Host "Done."
