param(
  [switch]$SkipToolchainInit,
  [switch]$SkipProcessHarness
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$repoRoot = Split-Path -Parent $PSScriptRoot

Set-Location -LiteralPath $repoRoot
if (-not $SkipToolchainInit) {
  . (Join-Path $PSScriptRoot "initialize-vsenv.ps1")
  Initialize-VsDevEnvironment
}

Write-Output "[offline-audit] static deny-all boundary"
$primaryRuntimeSources = @(
  "apps/desktop/src-tauri/src/lib.rs",
  "apps/desktop/src-tauri/src/capture.rs",
  "apps/desktop/src-tauri/src/asr_service.rs",
  "apps/desktop/src-tauri/src/insertion.rs",
  "apps/desktop/src-tauri/src/ipc.rs",
  "crates/asr-sidecar/src/main.rs",
  "crates/asr-sidecar/src/engine.rs",
  "crates/protocol/src/asr.rs",
  "crates/protocol/src/message.rs"
)
$forbiddenNetworkPattern = '(reqwest|TcpStream|UdpSocket|WebSocket|ureq|hyper::|Client::builder)'
foreach ($relativePath in $primaryRuntimeSources) {
  $source = Get-Content -Raw -LiteralPath (Join-Path $repoRoot $relativePath)
  if ($source -match $forbiddenNetworkPattern) {
    throw "Primary offline runtime contains a network client symbol: $relativePath"
  }
}

$desktopManifest = Get-Content -Raw -LiteralPath (Join-Path $repoRoot "apps/desktop/src-tauri/Cargo.toml")
if ($desktopManifest -match '(?m)^[ ]*(reqwest|hyper|ureq|tokio-tungstenite)[ ]*=') {
  throw "Desktop shell must not depend directly on a network client"
}

$nativePreference = $PSNativeCommandUseErrorActionPreference
$PSNativeCommandUseErrorActionPreference = $false
$networkSites = rg -l --glob "*.rs" '(reqwest|TcpStream|UdpSocket|WebSocket|ureq|hyper::)' crates apps/desktop/src-tauri/src
$rgExitCode = $LASTEXITCODE
$PSNativeCommandUseErrorActionPreference = $nativePreference

if ($rgExitCode -notin @(0, 1)) {
  throw "Network source inventory failed"
}
$unexpectedSites = @($networkSites | Where-Object {
  $_.Replace('\', '/') -ne "crates/storage/src/model_manager.rs"
})
if ($unexpectedSites.Count -ne 0) {
  throw "Network code escaped the explicit model manager boundary: $($unexpectedSites -join ', ')"
}

if ($SkipProcessHarness) {
  Write-Output "[offline-audit] passed: static deny-all boundary"
  return
}

Write-Output "[offline-audit] process network deny harness"
$marker = "WIGIGA_MARKER_SECRET_7C49E2A1"
$previousCargoOffline = $env:CARGO_NET_OFFLINE
$previousHttpProxy = $env:HTTP_PROXY
$previousHttpsProxy = $env:HTTPS_PROXY
$previousMarker = $env:WIGIGADICT_AUDIT_MARKER
$previousErrorAction = $ErrorActionPreference
try {
  $env:CARGO_NET_OFFLINE = "true"
  $env:HTTP_PROXY = "http://127.0.0.1:9"
  $env:HTTPS_PROXY = "http://127.0.0.1:9"
  $env:WIGIGADICT_AUDIT_MARKER = $marker
  # Аудиту нужен и stderr: маркер ищется во всём выводе. Windows PowerShell 5.1 при 2>&1
  # заворачивает каждую строку stderr нативной команды в ErrorRecord, и на Stop обычное
  # cargo "Compiling ..." роняет шаг. Настоящая проверка — $LASTEXITCODE сразу ниже.
  $ErrorActionPreference = "Continue"
  $auditOutput = & cmd.exe /d /s /c "cargo test -p wigigadict-storage --test diagnostics --test recovery --locked 2>&1"
  $ErrorActionPreference = $previousErrorAction
  if ($LASTEXITCODE -ne 0) {
    foreach ($line in @($auditOutput)) {
      Write-Host ([string]$line)
    }
    throw "Offline local-flow tests failed"
  }
  $auditText = $auditOutput -join [Environment]::NewLine
  if ($auditText.IndexOf($marker, [StringComparison]::Ordinal) -ge 0) {
    throw "Marker secret leaked into offline audit output"
  }
  foreach ($line in @($auditOutput)) {
    Write-Host ([string]$line)
  }
}
finally {
  $ErrorActionPreference = $previousErrorAction
  $env:CARGO_NET_OFFLINE = $previousCargoOffline
  $env:HTTP_PROXY = $previousHttpProxy
  $env:HTTPS_PROXY = $previousHttpsProxy
  $env:WIGIGADICT_AUDIT_MARKER = $previousMarker
}

Write-Output "[offline-audit] passed: primary runtime has no network client and local flow is green under deny proxy/offline Cargo"
