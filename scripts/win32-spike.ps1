param(
  [string]$ReportPath = "artifacts/win32-spike/interactive.json",
  [switch]$BuildOnly
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$repoRoot = Split-Path -Parent $PSScriptRoot
$resolvedReport = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $ReportPath))
$repoPrefix = $repoRoot.TrimEnd('\') + '\'
if (-not $resolvedReport.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "Report path escaped repository root: $resolvedReport"
}
$logPath = Join-Path $repoRoot ("logs\win32-spike-{0:yyyyMMdd-HHmmss}.transcript" -f (Get-Date))

Start-Transcript -LiteralPath $logPath | Out-Null
try {
  Set-Location -LiteralPath $repoRoot
  . (Join-Path $PSScriptRoot "initialize-vsenv.ps1")
  Initialize-VsDevEnvironment

  Write-Output "[win32-spike] build isolated harness"
  cargo build --locked -p wigigadict-win32-spike
  if ($BuildOnly) {
    Write-Output "[win32-spike] build-only completed"
    return
  }
  Write-Output "[win32-spike] focus the window named 'WiGigaDict M0 target fixture' within 60 seconds"
  $quotedReport = '"' + $resolvedReport + '"'
  $process = Start-Process `
    -FilePath (Join-Path $repoRoot "target\debug\wigigadict-win32-spike.exe") `
    -ArgumentList $quotedReport `
    -PassThru `
    -Wait
  if ($process.ExitCode -ne 0) {
    throw "Win32 spike failed with exit code $($process.ExitCode); no input is injected unless the fixture owns foreground"
  }
  Write-Output "[win32-spike] report=$resolvedReport"
}
finally {
  Stop-Transcript | Out-Null
}
