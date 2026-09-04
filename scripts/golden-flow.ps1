param(
  [switch]$CheckThresholdsOnly,
  [switch]$SkipToolchainInit,
  [string]$Evidence,
  [string]$Output
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$repoRoot = Split-Path -Parent $PSScriptRoot
$thresholds = Join-Path $repoRoot "tests\golden-flow\thresholds-v1.json"

Set-Location -LiteralPath $repoRoot
if (-not $SkipToolchainInit) {
  . (Join-Path $PSScriptRoot "initialize-vsenv.ps1")
  Initialize-VsDevEnvironment
}

Write-Output "[golden-flow] validate frozen thresholds"
& cargo run -q -p wigigadict-test-support --bin golden-flow-gate --locked --offline -- `
  check-thresholds $thresholds
if ($LASTEXITCODE -ne 0) {
  throw "golden-flow threshold validation failed"
}

if ($CheckThresholdsOnly) {
  return
}
if ([string]::IsNullOrWhiteSpace($Evidence) -or [string]::IsNullOrWhiteSpace($Output)) {
  throw "Evidence and Output are required unless -CheckThresholdsOnly is used"
}

$evidencePath = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $Evidence))
$outputPath = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $Output))
if (-not (Test-Path -LiteralPath $evidencePath -PathType Leaf)) {
  throw "golden-flow evidence file does not exist"
}
$outputParent = Split-Path -Parent $outputPath
if (-not (Test-Path -LiteralPath $outputParent -PathType Container)) {
  New-Item -ItemType Directory -Path $outputParent | Out-Null
}

Write-Output "[golden-flow] evaluate content-free owner run"
& cargo run -q -p wigigadict-test-support --bin golden-flow-gate --locked --offline -- `
  evaluate $thresholds $evidencePath $outputPath
if ($LASTEXITCODE -ne 0) {
  throw "golden-flow gate did not pass; inspect the new aggregate report"
}
