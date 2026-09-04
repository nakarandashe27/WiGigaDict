#requires -Version 7.4

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$analyzer = Join-Path $PSScriptRoot "analyze-asr-benchmark.ps1"
$recorder = Join-Path $PSScriptRoot "record-asr-human-corpus.ps1"
$testBase = Join-Path $repoRoot ".t/asr-benchmark-contract-tests"
$testRoot = Join-Path $testBase ([Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
$passed = 0

function Assert-True {
  param([bool]$Condition, [string]$Message)
  if (-not $Condition) { throw $Message }
  $script:passed++
}

function New-Record {
  param(
    [string]$Text = "ready",
    [string]$Profile = "cpu",
    [string]$Mode = "cold",
    [long]$SegmentEnd = 1000,
    [int]$Threads = 0
  )
  [ordered]@{
    schema_version = 1
    run_id = "contract-test"
    engine = "whisper"
    adapter = "transcribe-rs=0.3.11"
    runtime = "whisper.cpp-cpu"
    profile = $Profile
    mode = $Mode
    sample_id = "en-test"
    language = "en"
    model_sha256 = ("a" * 64)
    model_bytes = 1
    audio_sha256 = ("b" * 64)
    audio_duration_ms = 1000
    load_ms = 1
    inference_ms = 1
    total_ms = 2
    rtf = 0.001
    peak_working_set_bytes = 1
    peak_vram_bytes = $null
    average_incremental_watts = $null
    energy_kwh = $null
    n_threads = $Threads
    text = $Text
    segments = @([ordered]@{ start_ms = 0; end_ms = $SegmentEnd; text = $Text })
  }
}

function Invoke-AnalyzerCase {
  param($Record, [string]$Name)
  $evidence = Join-Path $testRoot "$Name.ndjson"
  $output = Join-Path $testRoot "$Name.json"
  $Record | ConvertTo-Json -Compress -Depth 8 | Set-Content -LiteralPath $evidence -Encoding utf8NoBOM
  & $analyzer -Evidence $evidence -Manifest $manifestPath -Output $output | Out-Null
  Get-Content -Raw -LiteralPath $output | ConvertFrom-Json
}

function Assert-AnalyzerFails {
  param($Record, [string]$Name, [string]$Message)
  $failed = $false
  try {
    Invoke-AnalyzerCase $Record $Name | Out-Null
  } catch {
    $failed = $true
  }
  Assert-True $failed $Message
}

try {
  $manifestPath = Join-Path $testRoot "manifest.json"
  [ordered]@{
    schema_version = 1
    corpus_id = "contract-test"
    samples = @(
      [ordered]@{
        id = "en-test"
        language = "en"
        target_duration_ms = 1000
        reference = "ready"
        technical_tokens = @()
        final_marker = "ready"
        final_marker_aliases = @()
      }
    )
  } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $manifestPath -Encoding utf8NoBOM

  $exact = Invoke-AnalyzerCase (New-Record -Text "ready") "marker-exact"
  Assert-True ([bool]$exact.samples[0].marker_present) "Exact terminal marker must pass"

  $substring = Invoke-AnalyzerCase (New-Record -Text "unready") "marker-substring"
  Assert-True (-not [bool]$substring.samples[0].marker_present) "Substring collision must not pass"

  $middle = Invoke-AnalyzerCase (New-Record -Text "ready trailing words") "marker-middle"
  Assert-True (-not [bool]$middle.samples[0].marker_present) "Marker in the middle must not pass"

  Assert-AnalyzerFails (New-Record -SegmentEnd 1001) "segment-out-of-bounds" "Segment beyond WAV duration must fail"
  Assert-AnalyzerFails (New-Record -Profile "gpu-human") "invalid-profile" "Arbitrary profile must fail"
  Assert-AnalyzerFails (New-Record -Mode "tepid") "invalid-mode" "Arbitrary mode must fail"

  $threadPinned = Invoke-AnalyzerCase (New-Record -Profile "cpu-t16" -Threads 16) "thread-profile-valid"
  Assert-True ($threadPinned.samples[0].profile -ceq "cpu-t16") "Pinned CPU profile must match n_threads"
  Assert-AnalyzerFails (New-Record -Profile "cpu-t16" -Threads 8) "thread-profile-mismatch" "Pinned CPU profile/thread mismatch must fail"

  $bindingJson = & pwsh -NoProfile -File $recorder -ManifestBindingOnly -Manifest $manifestPath
  if ($LASTEXITCODE -ne 0) { throw "ManifestBindingOnly failed with exit $LASTEXITCODE" }
  $binding = $bindingJson | ConvertFrom-Json
  $expectedHash = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
  Assert-True ($binding.manifest_sha256 -ceq $expectedHash) "Recorder manifest binding must contain exact SHA-256"

  Write-Host "ASR benchmark contract tests passed: $passed/9"
}
finally {
  $resolvedBase = [IO.Path]::GetFullPath($testBase).TrimEnd([IO.Path]::DirectorySeparatorChar)
  $resolvedTarget = [IO.Path]::GetFullPath($testRoot)
  if (-not $resolvedTarget.StartsWith($resolvedBase + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing unsafe test cleanup path: $resolvedTarget"
  }
  if (Test-Path -LiteralPath $resolvedTarget) {
    Remove-Item -LiteralPath $resolvedTarget -Recurse -Force
  }
}


