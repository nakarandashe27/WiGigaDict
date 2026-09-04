param(
  [string]$ReportDirectory = "artifacts/reports"
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$repoRoot = Split-Path -Parent $PSScriptRoot
$reportRoot = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $ReportDirectory))
$logPath = Join-Path $repoRoot ("logs\supply-chain-{0:yyyyMMdd-HHmmss}.transcript" -f (Get-Date))

Start-Transcript -LiteralPath $logPath | Out-Null
try {
  Set-Location -LiteralPath $repoRoot
  . (Join-Path $PSScriptRoot "initialize-vsenv.ps1")
  Initialize-VsDevEnvironment
  New-Item -ItemType Directory -Force -Path $reportRoot | Out-Null

  $denyVersion = cargo deny --version
  if ($denyVersion -ne "cargo-deny 0.20.2") {
    throw "Expected cargo-deny 0.20.2, got: $denyVersion"
  }
  $cycloneDxVersion = cargo cyclonedx --version
  if ($cycloneDxVersion -ne "cargo-cyclonedx-cyclonedx 0.5.9") {
    throw "Expected cargo-cyclonedx 0.5.9, got: $cycloneDxVersion"
  }

  Write-Output "[supply-chain] npm vulnerability audit"
  $nativePreference = $PSNativeCommandUseErrorActionPreference
  $PSNativeCommandUseErrorActionPreference = $false
  $npmAudit = npm audit --prefix apps/desktop --audit-level=high --json
  $npmAuditExitCode = $LASTEXITCODE
  $PSNativeCommandUseErrorActionPreference = $nativePreference
  [System.IO.File]::WriteAllLines((Join-Path $reportRoot "npm-audit.json"), $npmAudit)
  if ($npmAuditExitCode -ne 0) {
    throw "npm audit failed with exit code $npmAuditExitCode; report was preserved"
  }

  Write-Output "[supply-chain] npm license policy and inventory"
  node scripts/check-npm-licenses.mjs (Join-Path $reportRoot "npm-licenses.json")

  Write-Output "[supply-chain] npm CycloneDX SBOM"
  $npmSbom = npm sbom --prefix apps/desktop --sbom-format cyclonedx
  [System.IO.File]::WriteAllLines((Join-Path $reportRoot "npm.cdx.json"), $npmSbom)

  Write-Output "[supply-chain] Cargo advisory/license/source/bans policy"
  cargo deny --locked check advisories bans licenses sources --hide-inclusion-graph
  $rustLicenses = cargo deny list --format json
  [System.IO.File]::WriteAllLines((Join-Path $reportRoot "rust-licenses.json"), $rustLicenses)

  Write-Output "[supply-chain] Cargo CycloneDX SBOMs"
  $generatedSbomName = "wigigadict.cdx.json"
  cargo cyclonedx --manifest-path Cargo.toml --format json --all --all-features --spec-version 1.5 --override-filename "wigigadict.cdx"
  $manifests = @(
    @{ Name = "desktop"; Path = "apps/desktop/src-tauri/Cargo.toml" },
    @{ Name = "protocol"; Path = "crates/protocol/Cargo.toml" },
    @{ Name = "asr-sidecar"; Path = "crates/asr-sidecar/Cargo.toml" },
    @{ Name = "test-support"; Path = "crates/test-support/Cargo.toml" },
    @{ Name = "win32-spike"; Path = "crates/win32-spike/Cargo.toml" }
  )
  foreach ($manifest in $manifests) {
    $outputName = "rust-$($manifest.Name).cdx.json"
    $generated = [System.IO.Path]::GetFullPath((Join-Path (Split-Path -Parent (Join-Path $repoRoot $manifest.Path)) $generatedSbomName))
    $repoPrefix = $repoRoot.TrimEnd('\') + '\'
    if (-not $generated.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
      throw "Generated SBOM escaped repository root: $generated"
    }
    Copy-Item -LiteralPath $generated -Destination (Join-Path $reportRoot $outputName) -Force
    Remove-Item -LiteralPath $generated -Force
  }

  $versions = @(
    (node --version),
    (npm --version),
    (cargo --version),
    $denyVersion,
    $cycloneDxVersion
  )
  [System.IO.File]::WriteAllLines((Join-Path $reportRoot "tool-versions.txt"), $versions)
  Write-Output "[supply-chain] reports=$reportRoot"
}
finally {
  Stop-Transcript | Out-Null
}
