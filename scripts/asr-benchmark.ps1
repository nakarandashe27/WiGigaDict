param(
  [ValidateSet("Test", "Probe", "BuildCpu", "SmokeCpu")]
  [string]$Action = "Test",
  [string]$Model,
  [ValidateSet("en-005", "ru-005")]
  [string]$Sample = "en-005",
  [string]$EvidenceOutput
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$repoRoot = Split-Path -Parent $PSScriptRoot
$toolManifest = Join-Path $repoRoot "tools/asr-benchmark/Cargo.toml"
$logPath = Join-Path $repoRoot ("logs/asr-benchmark-{0:yyyyMMdd-HHmmss}.transcript" -f (Get-Date))
$llvmBin = "C:\Program Files\LLVM\bin"
$expectedClang = "clang version 22.1.8"
$expectedLibclangSha256 = "51FED10C43C3D31C1FE5BFE76BAC60150970961E9B9B23CF014DBFCB5398BBFC"

Start-Transcript -LiteralPath $logPath | Out-Null
try {
  Set-Location -LiteralPath $repoRoot
  . (Join-Path $PSScriptRoot "initialize-vsenv.ps1")
  Initialize-VsDevEnvironment
  $clangVersion = (& (Join-Path $llvmBin "clang.exe") --version | Select-Object -First 1)
  if (-not $clangVersion.StartsWith($expectedClang)) { throw "Expected $expectedClang, got $clangVersion" }
  $libclangHash = (Get-FileHash -LiteralPath (Join-Path $llvmBin "libclang.dll") -Algorithm SHA256).Hash
  if ($libclangHash -ne $expectedLibclangSha256) { throw "libclang.dll SHA-256 mismatch" }
  $env:LIBCLANG_PATH = $llvmBin

  switch ($Action) {
    "Test" { cargo test --manifest-path $toolManifest --locked }
    "Probe" {
      if (-not $Model) { throw "-Model is required" }
      cargo run --manifest-path $toolManifest --locked -- probe-whisper --model $Model
    }
    "BuildCpu" { cargo build --manifest-path $toolManifest --locked --features whisper-cpu }
    "SmokeCpu" {
      if (-not $Model) { throw "-Model is required" }
      $language = $Sample.Substring(0, 2)
      $audio = Join-Path $repoRoot "tests/asr-benchmark/generated/$Sample.wav"
      $output = if ($EvidenceOutput) {
        if ([IO.Path]::IsPathRooted($EvidenceOutput)) { $EvidenceOutput } else { Join-Path $repoRoot $EvidenceOutput }
      } else {
        Join-Path $repoRoot ("tests/asr-benchmark/evidence/local-smoke-{0:yyyyMMdd-HHmmss-fffffff}.ndjson" -f (Get-Date))
      }
      if (Test-Path -LiteralPath $output) {
        throw "Refusing ambiguous evidence append; output already exists: $output"
      }
      cargo run --manifest-path $toolManifest --locked --features whisper-cpu -- run-whisper --model $Model --audio $audio --sample $Sample --language $language --profile cpu --mode cold --output $output
    }
  }
}
finally { Stop-Transcript | Out-Null }

