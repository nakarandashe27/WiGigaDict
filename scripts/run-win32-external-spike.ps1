param(
  [Parameter(Mandatory = $true)]
  [ValidateSet("vscode_codex", "terminal_claude_code", "browser")]
  [string]$Surface,

  [Parameter(Mandatory = $true)]
  [string]$ExpectedProcess,

  [string]$ActivationTitle,
  [string]$ReportPath,
  [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($ReportPath)) {
  $ReportPath = "artifacts/win32-spike/manual-$Surface-$('{0:yyyyMMdd-HHmmss}' -f (Get-Date)).json"
}
$resolvedReport = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $ReportPath))
$repoPrefix = $repoRoot.TrimEnd('\') + '\'
if (-not $resolvedReport.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "Report path escaped repository root: $resolvedReport"
}
$logPath = Join-Path $repoRoot ("logs\win32-external-spike-{0:yyyyMMdd-HHmmss}.transcript" -f (Get-Date))

Start-Transcript -LiteralPath $logPath | Out-Null
try {
  Set-Location -LiteralPath $repoRoot
  . (Join-Path $PSScriptRoot "initialize-vsenv.ps1")
  Initialize-VsDevEnvironment

  if (-not $SkipBuild) {
    cargo build --locked -p wigigadict-win32-spike
  }
  $binary = Join-Path $repoRoot "target\debug\wigigadict-win32-spike.exe"
  if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
    throw "Win32 spike binary is missing: $binary"
  }

  Write-Output "[external-spike] surface=$Surface expected_process=$ExpectedProcess"
  if ([string]::IsNullOrWhiteSpace($ActivationTitle)) {
    Write-Output "[external-spike] focus the disposable target within 60 seconds; do not use a live prompt"
  }
  else {
    Write-Output "[external-spike] activating the unique disposable title prefix: $ActivationTitle"
  }

  $arguments = @(
    "external",
    $Surface,
    $ExpectedProcess,
    ('"' + $resolvedReport + '"')
  )
  if (-not [string]::IsNullOrWhiteSpace($ActivationTitle)) {
    $arguments += ('"' + $ActivationTitle + '"')
  }
  $process = Start-Process -FilePath $binary -ArgumentList $arguments -PassThru -Wait
  if ($process.ExitCode -ne 0) {
    throw "External Win32 spike failed with exit code $($process.ExitCode); no report is accepted as evidence"
  }
  Write-Output "[external-spike] report=$resolvedReport"
}
finally {
  Stop-Transcript | Out-Null
}
