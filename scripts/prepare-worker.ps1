<#
.SYNOPSIS
  Builds the Vulkan ASR worker and places it next to the bundled sidecar.

.DESCRIPTION
  ggml builds its Vulkan shader generator as a nested CMake ExternalProject, and that nested
  configure can lose the MSVC compiler when it creates a second Visual Studio generator below the
  Cargo build. The worker crate is therefore always staged into one short, space-free directory
  and both CMake levels use Ninja with the initialized MSVC environment. Keeping a stable staging
  root also preserves Cargo/CMake incremental build caches.

  Requires the Vulkan SDK; VULKAN_SDK must be set.
#>
param(
  [string]$StagingRoot = "C:\wgd-worker"
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

if (-not [System.IO.Path]::IsPathFullyQualified($StagingRoot) -or $StagingRoot -match " ") {
  throw "Staging root must be an absolute path without spaces: $StagingRoot"
}
if ($StagingRoot.Length -gt 20) {
  throw "Staging root must be at most 20 characters because nested CMake paths approach the Windows limit: $StagingRoot"
}

Write-Output "[worker] staging into short build root $StagingRoot"
New-Item -ItemType Directory -Force -Path $StagingRoot | Out-Null
Copy-Item -LiteralPath (Join-Path $source "Cargo.toml") -Destination $StagingRoot -Force
$lock = Join-Path $source "Cargo.lock"
if (Test-Path -LiteralPath $lock) {
  Copy-Item -LiteralPath $lock -Destination $StagingRoot -Force
}
Copy-Item -LiteralPath (Join-Path $source "src") -Destination $StagingRoot -Recurse -Force
$manifest = Join-Path $StagingRoot "Cargo.toml"
$built = Join-Path $StagingRoot "target\release\wigigadict-asr-benchmark.exe"

$previousPath = $env:Path
$ninja = Get-Command ninja.exe -ErrorAction SilentlyContinue
if (-not $ninja) {
  $ninjaCandidates = @("C:\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja\ninja.exe")
  if ($env:VSINSTALLDIR) {
    $ninjaCandidates = @(
      (Join-Path $env:VSINSTALLDIR "Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja\ninja.exe")
    ) + $ninjaCandidates
  }
  $ninjaPath = $ninjaCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
  if (-not $ninjaPath) {
    throw "ninja.exe was not found. Install Visual Studio 2022 Build Tools with the C++ CMake tools."
  }
  $env:Path = "$(Split-Path -Parent $ninjaPath);$env:Path"
}
$hadCmakeGenerator = Test-Path Env:CMAKE_GENERATOR
$previousCmakeGenerator = $env:CMAKE_GENERATOR
try {
  $env:CMAKE_GENERATOR = "Ninja"
  cargo build --manifest-path $manifest --release --features whisper-vulkan --locked
}
finally {
  if ($hadCmakeGenerator) {
    $env:CMAKE_GENERATOR = $previousCmakeGenerator
  }
  else {
    Remove-Item Env:CMAKE_GENERATOR -ErrorAction SilentlyContinue
  }
  $env:Path = $previousPath
}

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
