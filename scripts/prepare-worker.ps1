<#
.SYNOPSIS
  Builds the Vulkan ASR worker and places it next to the bundled sidecar.

.DESCRIPTION
  ggml builds its Vulkan shader generator as a nested CMake ExternalProject, and that nested
  configure fails with "No CMAKE_C_COMPILER could be found" whenever the build path contains a
  space. The repository lives at "C:\Local WhisperGigaAM Desktop", so the crate is staged into a
  space-free directory before building. A repository path without spaces builds in place and
  costs no extra disk.

  Requires the Vulkan SDK; VULKAN_SDK must be set.
#>
param(
  [string]$StagingRoot = "C:\wigigadict-worker-build"
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$repoRoot = Split-Path -Parent $PSScriptRoot
$source = Join-Path $repoRoot "tools\asr-benchmark"
$destinationDir = Join-Path $repoRoot "apps\desktop\src-tauri\binaries"
$destination = Join-Path $destinationDir "wigigadict-asr-worker-x86_64-pc-windows-msvc.exe"

if (-not $env:VULKAN_SDK) {
  throw "VULKAN_SDK is not set. Install the Vulkan SDK before building the worker."
}
if (-not (Test-Path -LiteralPath (Join-Path $source "Cargo.toml"))) {
  throw "ASR worker crate not found at $source."
}

if ($repoRoot -match " ") {
  Write-Output "[worker] repository path contains a space; staging into $StagingRoot"
  New-Item -ItemType Directory -Force -Path $StagingRoot | Out-Null
  if ($StagingRoot -match " ") {
    throw "Staging root must not contain a space: $StagingRoot"
  }
  Copy-Item -LiteralPath (Join-Path $source "Cargo.toml") -Destination $StagingRoot -Force
  $lock = Join-Path $source "Cargo.lock"
  if (Test-Path -LiteralPath $lock) {
    Copy-Item -LiteralPath $lock -Destination $StagingRoot -Force
  }
  Copy-Item -LiteralPath (Join-Path $source "src") -Destination $StagingRoot -Recurse -Force
  $manifest = Join-Path $StagingRoot "Cargo.toml"
  $built = Join-Path $StagingRoot "target\release\wigigadict-asr-benchmark.exe"
}
else {
  Write-Output "[worker] repository path is space-free; building in place"
  $manifest = Join-Path $source "Cargo.toml"
  $built = Join-Path $source "target\release\wigigadict-asr-benchmark.exe"
}

cargo build --manifest-path $manifest --release --features whisper-vulkan --locked

if (-not (Test-Path -LiteralPath $built)) {
  throw "Worker binary was not produced at $built."
}

New-Item -ItemType Directory -Force -Path $destinationDir | Out-Null
Copy-Item -LiteralPath $built -Destination $destination -Force
$hash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLower()
$size = (Get-Item -LiteralPath $destination).Length
Write-Output "Prepared bundled ASR worker: $destination"
Write-Output "  size:   $size"
Write-Output "  sha256: $hash"
