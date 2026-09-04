[CmdletBinding()]
param(
  [ValidatePattern("^(latest|v[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?)$")]
  [string]$Version = "latest",
  [string]$DestinationDirectory = (Join-Path ([Environment]::GetFolderPath("UserProfile")) "Downloads"),
  [switch]$Silent,
  [switch]$DownloadOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

if ($env:OS -ne "Windows_NT" -or -not [Environment]::Is64BitOperatingSystem) {
  throw "WiGigaDict release installer requires 64-bit Windows."
}

[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$repository = "nakarandashe27/WiGigaDict"
$headers = @{
  Accept = "application/vnd.github+json"
  "User-Agent" = "WiGigaDict-release-installer"
  "X-GitHub-Api-Version" = "2022-11-28"
}
if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_TOKEN)) {
  $headers.Authorization = "Bearer $($env:GITHUB_TOKEN)"
}

if ($Version -eq "latest") {
  $releases = Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/$repository/releases?per_page=20"
  $release = @($releases | Where-Object { -not $_.draft }) | Select-Object -First 1
}
else {
  $escapedVersion = [Uri]::EscapeDataString($Version)
  $release = Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/$repository/releases/tags/$escapedVersion"
}

if ($null -eq $release) {
  throw "No published WiGigaDict release was found."
}

$installerAssets = @($release.assets | Where-Object { $_.name -match '^WiGigaDict_.+_x64-setup\.exe$' })
$checksumAssets = @($release.assets | Where-Object { $_.name -eq "SHA256SUMS.txt" })
if ($installerAssets.Count -ne 1 -or $checksumAssets.Count -ne 1) {
  throw "Release $($release.tag_name) must contain exactly one x64 installer and one SHA256SUMS.txt."
}

$destination = New-Item -ItemType Directory -Force -Path $DestinationDirectory
$installerPath = Join-Path $destination.FullName $installerAssets[0].name
$checksumPath = Join-Path $destination.FullName "SHA256SUMS-$($release.tag_name).txt"

Invoke-WebRequest -Headers $headers -Uri $installerAssets[0].browser_download_url -OutFile $installerPath
Invoke-WebRequest -Headers $headers -Uri $checksumAssets[0].browser_download_url -OutFile $checksumPath

$escapedName = [Regex]::Escape($installerAssets[0].name)
$checksumLine = Get-Content -LiteralPath $checksumPath | Where-Object {
  $_ -match "^([0-9a-fA-F]{64})\s+$escapedName$"
} | Select-Object -First 1
if ($null -eq $checksumLine) {
  throw "SHA256SUMS.txt has no checksum for $($installerAssets[0].name)."
}

$expectedHash = ([Regex]::Match($checksumLine, "^[0-9a-fA-F]{64}")).Value.ToLowerInvariant()
$actualHash = (Get-FileHash -LiteralPath $installerPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualHash -ne $expectedHash) {
  throw "Installer SHA-256 mismatch. Expected $expectedHash, got $actualHash. The installer was not started."
}

Write-Output "Release: $($release.tag_name)"
Write-Output "Release page: $($release.html_url)"
Write-Output "Installer: $installerPath"
Write-Output "SHA-256 verified: $actualHash"

if ($DownloadOnly) {
  Write-Output "DownloadOnly requested; installer was not started."
  return
}

$process = if ($Silent) {
  Start-Process -FilePath $installerPath -ArgumentList "/S" -Wait -PassThru
}
else {
  Start-Process -FilePath $installerPath -Wait -PassThru
}
if ($process.ExitCode -ne 0) {
  throw "Installer exited with code $($process.ExitCode)."
}
Write-Output "Installation completed successfully. Open WiGigaDict and choose a speech model before the first dictation."
