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

  if (-not $SkipNpmCi) {
    Write-QualityStage "npm clean install"
    npm ci --prefix apps/desktop
  }
  Write-QualityStage "frontend bundle prerequisite"
  npm run build --prefix apps/desktop

  Write-QualityStage "rust format"
  cargo fmt --all -- --check
  Write-QualityStage "prepare clean-checkout bundle inputs"
  cargo build --package wigigadict-asr-sidecar --locked
  & (Join-Path $PSScriptRoot "prepare-sidecar.ps1") -Profile debug
  & (Join-Path $PSScriptRoot "prepare-worker.ps1")
  $clippyPackages = @(
    "wigigadict-protocol",
    "wigigadict-storage",
    "wigigadict-test-support",
    "wigigadict-asr-sidecar",
    "wigigadict-win32-spike"
  )
  foreach ($package in $clippyPackages) {
    Write-QualityStage "rust clippy: $package"
    cargo clippy --package $package --all-targets --all-features --locked -- -D warnings
  }
  Write-QualityStage "rust clippy: wigigadict-desktop lib"
  cargo clippy --package wigigadict-desktop --lib --all-features --locked -- -D warnings
  Write-QualityStage "rust clippy: wigigadict-desktop bin"
  $nativePreference = $PSNativeCommandUseErrorActionPreference
  $errorPreference = $ErrorActionPreference
  try {
    $PSNativeCommandUseErrorActionPreference = $false
    $ErrorActionPreference = "Continue"
    $desktopBinOutput = & cargo clippy --package wigigadict-desktop --bin wigigadict-desktop --all-features --locked -- -D warnings 2>&1
    $desktopBinExitCode = $LASTEXITCODE
  }
  finally {
    $PSNativeCommandUseErrorActionPreference = $nativePreference
    $ErrorActionPreference = $errorPreference
  }
  foreach ($line in $desktopBinOutput) {
    Write-Output $line.ToString()
  }
  if ($desktopBinExitCode -ne 0) {
    $desktopBinText = $desktopBinOutput -join [Environment]::NewLine
    $category = if ($desktopBinText -match '(?i)(frontendDist|frontend dist)') {
      "missing_frontend"
    }
    elseif ($desktopBinText -match '(?i)(externalBin|resource path|does not exist|doesn''t exist)') {
      "missing_resource"
    }
    elseif ($desktopBinText -match '(?i)(LNK\d+|linking with .* failed|linker .* failed)') {
      "linker"
    }
    elseif ($desktopBinText -match '(?m)^error\[E\d+\]') {
      "rust_compile"
    }
    elseif ($desktopBinText -match '(?m)^error:') {
      "compiler_or_lint"
    }
    else {
      "unclassified"
    }
    if ($env:GITHUB_ACTIONS) {
      Write-Output "::error title=WiGigaDict desktop bin category::$category"
    }
    throw "Desktop binary Clippy failed ($category)."
  }
  Write-QualityStage "rust clippy: wigigadict-desktop tests"
  cargo clippy --package wigigadict-desktop --tests --all-features --locked -- -D warnings
  Write-QualityStage "rust unit/integration/fault tests"
  cargo test --workspace --all-targets --all-features --locked

  Write-QualityStage "golden-flow frozen threshold contract"
  & (Join-Path $PSScriptRoot "golden-flow.ps1") -CheckThresholdsOnly -SkipToolchainInit

  Write-QualityStage "offline deny-all and marker audit"
  & (Join-Path $PSScriptRoot "offline-audit.ps1") -SkipToolchainInit

  Write-QualityStage "TypeScript format/lint/unit/integration/build"
  npm run check --prefix apps/desktop
  Write-Output "[quality] completed"
}
finally {
  Stop-Transcript | Out-Null
}
