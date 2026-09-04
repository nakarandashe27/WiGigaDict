#requires -Version 7.4

param(
  [ValidatePattern("^[a-z0-9][a-z0-9-]{0,31}$")]
  [string]$SpeakerId,
  [ValidateRange(1, 20)]
  [int]$Take = 1,
  [string]$DeviceName,
  [ValidateRange(0, 10)]
  [int]$TailPaddingSeconds = 0,
  [string]$Manifest = "tests/asr-benchmark/corpus/smoke-manifest.json",
  [switch]$ListDevices,
  [switch]$ManifestBindingOnly
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$repoRoot = Split-Path -Parent $PSScriptRoot

function Get-ManifestBinding {
  param([string]$ManifestArgument)
  $candidate = if ([IO.Path]::IsPathRooted($ManifestArgument)) {
    $ManifestArgument
  } else {
    Join-Path $repoRoot $ManifestArgument
  }
  $path = (Resolve-Path -LiteralPath $candidate).Path
  $item = Get-Item -LiteralPath $path
  [pscustomobject]@{
    path = $path
    relative_path = [IO.Path]::GetRelativePath($repoRoot, $path).Replace([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    bytes = $item.Length
    sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    manifest = Get-Content -Raw -LiteralPath $path -Encoding UTF8 | ConvertFrom-Json
  }
}

if ($ManifestBindingOnly) {
  $binding = Get-ManifestBinding $Manifest
  [ordered]@{
    corpus_reference = $binding.manifest.corpus_id
    manifest_relative_path = $binding.relative_path
    manifest_bytes = $binding.bytes
    manifest_sha256 = $binding.sha256
  } | ConvertTo-Json
  exit 0
}

$ffmpeg = (Get-Command ffmpeg.exe -ErrorAction Stop).Source
$ffprobe = Join-Path (Split-Path -Parent $ffmpeg) "ffprobe.exe"
if (-not (Test-Path -LiteralPath $ffprobe -PathType Leaf)) { throw "ffprobe.exe not found beside ffmpeg.exe" }

if ($ListDevices) {
  Write-Warning "Device listing intentionally ends after FFmpeg reports DirectShow devices. No audio is recorded."
  $PSNativeCommandUseErrorActionPreference = $false
  & $ffmpeg -hide_banner -list_devices true -f dshow -i dummy
  exit 0
}

if (-not $SpeakerId) { throw "-SpeakerId is required unless -ListDevices is used" }
if (-not $DeviceName) { throw "-DeviceName is required unless -ListDevices is used" }

$manifestBinding = Get-ManifestBinding $Manifest
$manifestPath = $manifestBinding.path
$manifestObject = $manifestBinding.manifest
$outputDirectory = Join-Path $repoRoot ("tests/asr-benchmark/private/human/{0}/take-{1:D2}" -f $SpeakerId, $Take)
$provenancePath = Join-Path $outputDirectory "capture-provenance.json"
$logPath = Join-Path $repoRoot ("logs/asr-human-capture-{0}-take-{1:D2}-{2:yyyyMMdd-HHmmss}.transcript" -f $SpeakerId, $Take, (Get-Date))
if (Test-Path -LiteralPath $provenancePath) { throw "This speaker/take already has provenance: $provenancePath" }
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
$preflightPath = $null

Start-Transcript -LiteralPath $logPath | Out-Null
try {
  $version = @(& $ffmpeg -hide_banner -version)
  $buildConfiguration = @($version | Where-Object { $_ -like "configuration:*" })
  $ffmpegHash = (Get-FileHash -LiteralPath $ffmpeg -Algorithm SHA256).Hash
  Write-Warning "This locally installed FFmpeg build is benchmark capture tooling only. It is not a WiGigaDict production or bundled FFmpeg artifact."
  if ($buildConfiguration -match "--enable-gpl") {
    Write-Warning "The capture-only FFmpeg reports --enable-gpl and is prohibited from product bundling."
  }
  Write-Host "FFmpeg: $($version[0])"
  Write-Host "FFmpeg SHA-256: $ffmpegHash"
  Write-Host "Capture device: $DeviceName"
  Write-Host "Output: $outputDirectory"

  $preflightDirectory = Join-Path $repoRoot ".t/asr-microphone-preflight"
  New-Item -ItemType Directory -Path $preflightDirectory -Force | Out-Null
  $preflightPath = Join-Path $preflightDirectory ("{0}-take-{1:D2}-{2}.wav" -f $SpeakerId, $Take, [Guid]::NewGuid().ToString("N"))
  Write-Host ""
  Write-Host "Microphone signal preflight (5 seconds)" -ForegroundColor Cyan
  Write-Host "Say continuously: Проверка микрофона WiGigaDict, один, два, три."
  $null = Read-Host "Press Enter when ready; signal test starts after a three-second countdown"
  foreach ($second in 3..1) {
    Write-Host $second
    Start-Sleep -Seconds 1
  }
  Write-Host "SIGNAL TEST" -ForegroundColor Red
  & $ffmpeg -hide_banner -loglevel warning -nostdin -f dshow -thread_queue_size 512 `
    -i "audio=$DeviceName" -t 5 -vn -ac 1 -ar 16000 -c:a pcm_s16le `
    -map_metadata -1 -fflags +bitexact -flags:a +bitexact -n $preflightPath
  if ($LASTEXITCODE -ne 0) { throw "FFmpeg microphone preflight failed: exit $LASTEXITCODE" }

  $PSNativeCommandUseErrorActionPreference = $false
  $volumeLines = @(& $ffmpeg -hide_banner -nostdin -i $preflightPath -af volumedetect -f null NUL 2>&1 | ForEach-Object ToString)
  $volumeExitCode = $LASTEXITCODE
  $PSNativeCommandUseErrorActionPreference = $true
  if ($volumeExitCode -ne 0) { throw "FFmpeg volumedetect failed: exit $volumeExitCode" }
  $meanLine = $volumeLines | Where-Object { $_ -match "mean_volume:" } | Select-Object -Last 1
  $maxLine = $volumeLines | Where-Object { $_ -match "max_volume:" } | Select-Object -Last 1
  $meanMatch = [regex]::Match($meanLine, "mean_volume:\s+(-?(?:inf|[0-9.]+))\s+dB")
  $maxMatch = [regex]::Match($maxLine, "max_volume:\s+(-?(?:inf|[0-9.]+))\s+dB")
  if (-not $meanMatch.Success -or -not $maxMatch.Success) { throw "Unable to parse microphone preflight volume" }
  $meanVolume = $meanMatch.Groups[1].Value
  $maxVolume = $maxMatch.Groups[1].Value
  Write-Host "Signal level: mean $meanVolume dB, max $maxVolume dB"
  if ($maxVolume -eq "-inf" -or [double]$maxVolume -lt -50.0) {
    throw "Microphone signal is too quiet (max $maxVolume dB; required >= -50 dB). Check the selected input, mute switch and Windows microphone level before retrying."
  }
  Write-Host "Microphone signal preflight passed." -ForegroundColor Green

  $records = foreach ($sample in $manifestObject.samples) {
    $outputPath = Join-Path $outputDirectory ("$($sample.id).wav")
    if (Test-Path -LiteralPath $outputPath) { throw "Refusing to overwrite existing recording: $outputPath" }
    $targetSeconds = [double]$sample.target_duration_ms / 1000.0
    $captureSeconds = $targetSeconds + $TailPaddingSeconds
    $captureDurationMilliseconds = [int]$sample.target_duration_ms + ($TailPaddingSeconds * 1000)
    Write-Host ""
    Write-Host "[$($sample.id)] speech target $targetSeconds seconds; capture $captureSeconds seconds" -ForegroundColor Cyan
    Write-Host $sample.reference
    if ($TailPaddingSeconds -gt 0) {
      Write-Host "Finish the prompt naturally, then stay silent for the $TailPaddingSeconds-second tail padding." -ForegroundColor Yellow
    }
    $null = Read-Host "Press Enter when ready; recording starts after a three-second countdown"
    foreach ($second in 3..1) {
      Write-Host $second
      Start-Sleep -Seconds 1
    }
    Write-Host "RECORDING" -ForegroundColor Red
    & $ffmpeg -hide_banner -loglevel warning -nostdin -f dshow -thread_queue_size 512 `
      -i "audio=$DeviceName" -t $captureSeconds -vn -ac 1 -ar 16000 -c:a pcm_s16le `
      -map_metadata -1 -fflags +bitexact -flags:a +bitexact -n $outputPath
    if ($LASTEXITCODE -ne 0) { throw "FFmpeg capture failed for $($sample.id): exit $LASTEXITCODE" }

    $probeJson = & $ffprobe -v error -select_streams a:0 `
      -show_entries stream=codec_name,sample_rate,channels,bits_per_sample,duration `
      -of json $outputPath
    if ($LASTEXITCODE -ne 0) { throw "ffprobe failed for $($sample.id): exit $LASTEXITCODE" }
    $probe = ($probeJson | ConvertFrom-Json).streams[0]
    $durationMilliseconds = [Math]::Round([double]::Parse($probe.duration, [Globalization.CultureInfo]::InvariantCulture) * 1000.0)
    if ($probe.codec_name -ne "pcm_s16le" -or [int]$probe.sample_rate -ne 16000 -or [int]$probe.channels -ne 1 -or [int]$probe.bits_per_sample -ne 16) {
      throw "Unexpected PCM format for $($sample.id): $($probe | ConvertTo-Json -Compress)"
    }
    if ([Math]::Abs($durationMilliseconds - $captureDurationMilliseconds) -gt 50.0) {
      throw "Duration mismatch for $($sample.id): expected $captureDurationMilliseconds ms, got $durationMilliseconds ms"
    }
    $item = Get-Item -LiteralPath $outputPath
    $hash = (Get-FileHash -LiteralPath $outputPath -Algorithm SHA256).Hash
    Write-Host "Saved $($sample.id): $durationMilliseconds ms, $($item.Length) bytes, SHA-256 $hash"
    [ordered]@{
      sample_id = $sample.id
      relative_path = $item.Name
      target_duration_ms = $captureDurationMilliseconds
      prompt_target_duration_ms = [int]$sample.target_duration_ms
      tail_padding_ms = ($TailPaddingSeconds * 1000)
      measured_duration_ms = [int]$durationMilliseconds
      bytes = $item.Length
      sha256 = $hash
    }
  }

  $provenance = [ordered]@{
    schema_version = 3
    purpose = "private_human_asr_benchmark_only"
    corpus_reference = $manifestObject.corpus_id
    manifest_relative_path = $manifestBinding.relative_path
    manifest_bytes = $manifestBinding.bytes
    manifest_sha256 = $manifestBinding.sha256
    speaker_id = $SpeakerId
    take = $Take
    tail_padding_ms = ($TailPaddingSeconds * 1000)
    recorded_at = [DateTimeOffset]::Now.ToString("O")
    device_name = $DeviceName
    ffmpeg_path = $ffmpeg
    ffmpeg_sha256 = $ffmpegHash
    ffmpeg_version = $version[0]
    ffmpeg_configuration = $buildConfiguration
    production_bundle_approved = $false
    samples = @($records)
  }
  $provenance | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $provenancePath -Encoding utf8
  Write-Host "Human corpus take ready: $provenancePath"
}
finally {
  if ($preflightPath -and (Test-Path -LiteralPath $preflightPath -PathType Leaf)) {
    Remove-Item -LiteralPath $preflightPath -Force
  }
  Stop-Transcript | Out-Null
}


