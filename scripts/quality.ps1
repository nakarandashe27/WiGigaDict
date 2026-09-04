param(
  [switch]$SkipNpmCi
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$repoRoot = Split-Path -Parent $PSScriptRoot
$logPath = Join-Path $repoRoot ("logs\quality-{0:yyyyMMdd-HHmmss}.transcript" -f (Get-Date))

Start-Transcript -LiteralPath $logPath | Out-Null
try {
  Set-Location -LiteralPath $repoRoot
  . (Join-Path $PSScriptRoot "initialize-vsenv.ps1")
  Initialize-VsDevEnvironment

  Write-Output "[quality] rust format"
  cargo fmt --all -- --check
  Write-Output "[quality] rust clippy"
  cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
  Write-Output "[quality] rust unit/integration/fault tests"
  cargo test --workspace --all-targets --all-features --locked

  Write-Output "[quality] golden-flow frozen threshold contract"
  & (Join-Path $PSScriptRoot "golden-flow.ps1") -CheckThresholdsOnly -SkipToolchainInit

  Write-Output "[quality] offline deny-all and marker audit"
  & (Join-Path $PSScriptRoot "offline-audit.ps1") -SkipToolchainInit

  if (-not $SkipNpmCi) {
    Write-Output "[quality] npm clean install"
    npm ci --prefix apps/desktop
  }
  Write-Output "[quality] TypeScript format/lint/unit/integration/build"
  npm run check --prefix apps/desktop
  Write-Output "[quality] completed"
}
finally {
  Stop-Transcript | Out-Null
}
