param(
  [switch]$SkipNpmCi
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$repoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "initialize-vsenv.ps1")
$logPath = Join-Path $repoRoot ("logs\build-{0:yyyyMMdd-HHmmss}.transcript" -f (Get-Date))
Start-Transcript -LiteralPath $logPath | Out-Null
try {
  Set-Location -LiteralPath $repoRoot
  Initialize-VsDevEnvironment
  Write-Output "[build] repo=$repoRoot"
  rustup show active-toolchain
  if (-not $SkipNpmCi) {
    npm ci --prefix apps/desktop
  }
  # `tauri::generate_context!` validates frontendDist during an ordinary Cargo build too.
  # A clean checkout therefore needs the Vite bundle before the first desktop compilation.
  npm run build --prefix apps/desktop
  & (Join-Path $repoRoot "scripts\create-icon.ps1")
  cargo build --package wigigadict-asr-sidecar --locked --release
  & (Join-Path $repoRoot "scripts\prepare-sidecar.ps1") -Profile release
  & (Join-Path $repoRoot "scripts\prepare-worker.ps1")
  cargo build --package wigigadict-desktop --locked --release
  npm run tauri:build --prefix apps/desktop
  Write-Output "[build] completed: v0.0.4-dev"
}
finally {
  Stop-Transcript | Out-Null
}
