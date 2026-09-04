<#
.SYNOPSIS
  Installs the pinned Vulkan SDK needed to build the ggml Vulkan backend.

.DESCRIPTION
  The ASR worker links ggml's Vulkan backend, which needs the SDK at build time (the runtime only
  needs the loader that ships with GPU drivers). The version and the SHA-256 come from LunarG's
  published manifest and are pinned here like every other dependency: a checksum mismatch aborts
  the install instead of continuing with an unverified installer.

  A machine that already has the pinned version installed is left untouched.
#>
param(
  [string]$Version = "1.4.357.0",
  [string]$ExpectedSha256 = "81f474711e9042f4cd22b31b2f7a8870db2e428b21586fb43dd80150be97310d",
  [long]$ExpectedSize = 287971024,
  [string]$InstallRoot = "C:\VulkanSDK"
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

$target = Join-Path $InstallRoot $Version
if (Test-Path -LiteralPath (Join-Path $target "Include\vulkan\vulkan.h")) {
  Write-Output "[vulkan] $Version is already installed at $target"
  if ($env:GITHUB_ENV) {
    "VULKAN_SDK=$target" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
  }
  $env:VULKAN_SDK = $target
  return
}

$installer = Join-Path $env:TEMP "vulkansdk-windows-X64-$Version.exe"
$uri = "https://sdk.lunarg.com/sdk/download/$Version/windows/vulkansdk-windows-X64-$Version.exe"
Write-Output "[vulkan] downloading $uri"
Invoke-WebRequest -Uri $uri -OutFile $installer -UseBasicParsing

$size = (Get-Item -LiteralPath $installer).Length
if ($size -ne $ExpectedSize) {
  throw "Vulkan SDK installer size is $size, expected $ExpectedSize. Refusing to install."
}
$hash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLower()
if ($hash -ne $ExpectedSha256.ToLower()) {
  throw "Vulkan SDK installer SHA-256 is $hash, expected $ExpectedSha256. Refusing to install."
}
Write-Output "[vulkan] checksum verified"

# LunarG ships a Qt Installer Framework package; these switches make it non-interactive.
& $installer --root $target --accept-licenses --default-answer --confirm-command install
if ($LASTEXITCODE -ne 0) {
  throw "Vulkan SDK installer exited with $LASTEXITCODE."
}
if (-not (Test-Path -LiteralPath (Join-Path $target "Include\vulkan\vulkan.h"))) {
  throw "Vulkan SDK headers were not found under $target after installation."
}

if ($env:GITHUB_ENV) {
  "VULKAN_SDK=$target" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
}
$env:VULKAN_SDK = $target
Write-Output "[vulkan] installed $Version at $target"
