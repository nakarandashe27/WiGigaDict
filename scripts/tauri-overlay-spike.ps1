param(
  [string]$ReportPath = "artifacts/win32-spike/tauri-overlay-manual.json",
  [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$resolvedReport = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $ReportPath))
$repoPrefix = $repoRoot.TrimEnd('\') + '\'
if (-not $resolvedReport.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "Report path escaped repository root: $resolvedReport"
}
$logPath = Join-Path $repoRoot ("logs\tauri-overlay-spike-{0:yyyyMMdd-HHmmss}.transcript" -f (Get-Date))

Start-Transcript -LiteralPath $logPath | Out-Null
try {
  Set-Location -LiteralPath $repoRoot
  if (-not $SkipBuild) {
    & (Join-Path $PSScriptRoot "build.ps1") -SkipNpmCi
  }
  $source = Join-Path $repoRoot "target\release\wigigadict-desktop.exe"
  $binary = Join-Path $repoRoot "target\release\wigigadict-overlay-spike.exe"
  Copy-Item -LiteralPath $source -Destination $binary -Force

  Write-Output "[tauri-overlay-spike] after one second, focus 'WiGigaDict' once; the process times out safely after 10 seconds"
  $reportArgument = '"--m0-overlay-report=' + $resolvedReport + '"'
  $process = Start-Process -FilePath $binary -ArgumentList $reportArgument -PassThru
  Wait-Process -Id $process.Id
  $process.Refresh()
  if ($process.ExitCode -ne 0) {
    throw "Tauri overlay spike failed with exit code $($process.ExitCode)"
  }
  $report = Get-Content -LiteralPath $resolvedReport -Raw | ConvertFrom-Json
  if (-not $report.passed -or $report.cycles -ne 100 -or $report.focus_steals -ne 0) {
    throw "Tauri overlay report did not meet the 100-cycle zero-focus-steal contract"
  }
  Write-Output "[tauri-overlay-spike] report=$resolvedReport"
}
finally {
  Stop-Transcript | Out-Null
}
