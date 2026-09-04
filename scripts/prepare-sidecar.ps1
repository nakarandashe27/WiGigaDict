param(
  [ValidateSet("debug", "release")]
  [string]$Profile = "debug"
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$targetDir = Join-Path $repoRoot "target\$Profile"
$source = Join-Path $targetDir "wigigadict-asr-sidecar.exe"
$destinationDir = Join-Path $repoRoot "apps\desktop\src-tauri\binaries"
$destination = Join-Path $destinationDir "wigigadict-asr-sidecar-x86_64-pc-windows-msvc.exe"

if (-not (Test-Path -LiteralPath $source)) {
  throw "Sidecar binary not found at $source. Build the Cargo workspace first."
}

New-Item -ItemType Directory -Force -Path $destinationDir | Out-Null
Copy-Item -LiteralPath $source -Destination $destination -Force
Write-Output "Prepared bundled sidecar: $destination"

