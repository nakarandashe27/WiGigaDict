$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "initialize-vsenv.ps1")
$logPath = Join-Path $repoRoot ("logs\dev-{0:yyyyMMdd-HHmmss}.transcript" -f (Get-Date))
Start-Transcript -LiteralPath $logPath | Out-Null
try {
  Set-Location -LiteralPath $repoRoot
  Initialize-VsDevEnvironment
  Write-Output "[dev] repo=$repoRoot"
  rustup show active-toolchain
  npm ci --prefix apps/desktop
  & (Join-Path $repoRoot "scripts\create-icon.ps1")
  cargo build --package wigigadict-asr-sidecar --locked
  & (Join-Path $repoRoot "scripts\prepare-sidecar.ps1") -Profile debug
  cargo build --package wigigadict-desktop --locked
  npm run tauri:dev --prefix apps/desktop
}
finally {
  Stop-Transcript | Out-Null
}
