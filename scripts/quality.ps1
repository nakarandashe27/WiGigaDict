param(
  [switch]$SkipNpmCi
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$repoRoot = Split-Path -Parent $PSScriptRoot
$logPath = Join-Path $repoRoot ("logs\quality-{0:yyyyMMdd-HHmmss}.transcript" -f (Get-Date))

function Write-QualityStage {
  param([Parameter(Mandatory = $true)][string]$Name)

  Write-Output "[quality] $Name"
  if ($env:GITHUB_ACTIONS) {
    Write-Output "::notice title=WiGigaDict quality stage::$Name"
  }
}

Start-Transcript -LiteralPath $logPath | Out-Null
try {
  Set-Location -LiteralPath $repoRoot
  . (Join-Path $PSScriptRoot "initialize-vsenv.ps1")
  Initialize-VsDevEnvironment

  Write-QualityStage "rust format"
  cargo fmt --all -- --check
  Write-QualityStage "prepare clean-checkout bundle inputs"
  cargo build --package wigigadict-asr-sidecar --locked
  & (Join-Path $PSScriptRoot "prepare-sidecar.ps1") -Profile debug
  & (Join-Path $PSScriptRoot "prepare-worker.ps1")
  Write-QualityStage "rust clippy"
  cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
  Write-QualityStage "rust unit/integration/fault tests"
  cargo test --workspace --all-targets --all-features --locked

  Write-QualityStage "golden-flow frozen threshold contract"
  & (Join-Path $PSScriptRoot "golden-flow.ps1") -CheckThresholdsOnly -SkipToolchainInit

  Write-QualityStage "offline deny-all and marker audit"
  & (Join-Path $PSScriptRoot "offline-audit.ps1") -SkipToolchainInit

  if (-not $SkipNpmCi) {
    Write-QualityStage "npm clean install"
    npm ci --prefix apps/desktop
  }
  Write-QualityStage "TypeScript format/lint/unit/integration/build"
  npm run check --prefix apps/desktop
  Write-Output "[quality] completed"
}
finally {
  Stop-Transcript | Out-Null
}
