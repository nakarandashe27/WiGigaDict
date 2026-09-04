param([string]$Manifest = "tests/asr-benchmark/corpus/smoke-manifest.json")

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $repoRoot $Manifest
$data = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$outputRoot = Join-Path $repoRoot "tests/asr-benchmark/generated"
$workRoot = Join-Path $repoRoot "tests/asr-benchmark/.work"
New-Item -ItemType Directory -Force -Path $outputRoot, $workRoot | Out-Null
Add-Type -AssemblyName System.Speech
$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)

function Read-Pcm([string]$Path) {
  $bytes = [IO.File]::ReadAllBytes($Path)
  for ($i = 12; $i -le $bytes.Length - 8;) {
    $id = [Text.Encoding]::ASCII.GetString($bytes, $i, 4)
    $size = [BitConverter]::ToUInt32($bytes, $i + 4)
    if ($id -eq "data") {
      $pcm = New-Object byte[] $size
      [Array]::Copy($bytes, $i + 8, $pcm, 0, $size)
      return $pcm
    }
    $i += 8 + $size + ($size % 2)
  }
  throw "Missing WAV data chunk: $Path"
}

function Write-Wav([string]$Path, [byte[]]$Pcm, [int]$TargetBytes) {
  if ($Pcm.Length -gt $TargetBytes) { throw "Speech exceeds target duration: $Path" }
  $stream = [IO.File]::Create($Path)
  $writer = New-Object IO.BinaryWriter($stream)
  try {
    $writer.Write([Text.Encoding]::ASCII.GetBytes("RIFF"))
    $writer.Write([uint32](36 + $TargetBytes))
    $writer.Write([Text.Encoding]::ASCII.GetBytes("WAVEfmt "))
    $writer.Write([uint32]16); $writer.Write([uint16]1); $writer.Write([uint16]1)
    $writer.Write([uint32]16000); $writer.Write([uint32]32000)
    $writer.Write([uint16]2); $writer.Write([uint16]16)
    $writer.Write([Text.Encoding]::ASCII.GetBytes("data")); $writer.Write([uint32]$TargetBytes)
    $writer.Write($Pcm)
    $writer.Write((New-Object byte[] ($TargetBytes - $Pcm.Length)))
  } finally { $writer.Dispose(); $stream.Dispose() }
}

$provenance = @()
foreach ($sample in $data.samples) {
  $culture = if ($sample.language -eq "ru") { "ru-RU" } else { "en-US" }
  $raw = Join-Path $workRoot ($sample.id + ".wav")
  $synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
  try {
    $voice = $synth.GetInstalledVoices() | Where-Object { $_.VoiceInfo.Culture.Name -eq $culture } | Select-Object -First 1
    if (-not $voice) { throw "No SAPI voice for $culture" }
    $synth.SelectVoice($voice.VoiceInfo.Name)
    $targetBytes = [int]($sample.target_duration_ms * 32)
    $pcm = $null
    foreach ($rate in 0..10) {
      $synth.Rate = $rate
      $synth.SetOutputToWaveFile($raw, $format)
      $synth.Speak($sample.reference)
      $synth.SetOutputToNull()
      $pcm = Read-Pcm $raw
      if ($pcm.Length -le $targetBytes) { break }
    }
    if ($pcm.Length -gt $targetBytes) { throw "Speech exceeds target duration even at SAPI rate 10: $($sample.id)" }
    $output = Join-Path $outputRoot ($sample.id + ".wav")
    Write-Wav $output $pcm $targetBytes
    $provenance += [ordered]@{ sample_id=$sample.id; voice=$voice.VoiceInfo.Name; rate=$synth.Rate; target_duration_ms=$sample.target_duration_ms; sha256=(Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant() }
  } finally { $synth.Dispose() }
}
[ordered]@{ schema_version=1; purpose="harness_smoke_only"; generator="Windows System.Speech"; generated_at=(Get-Date).ToUniversalTime().ToString("o"); samples=$provenance } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $outputRoot "provenance.json") -Encoding utf8
Write-Output "Generated $($provenance.Count) exact-duration smoke WAV files."
