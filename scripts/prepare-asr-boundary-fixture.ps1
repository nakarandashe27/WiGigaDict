[CmdletBinding()]
param(
    [string]$FfmpegPath = "ffmpeg",
    [string]$CalibrationSpeech = "tests/asr-benchmark/generated/ru-005.wav",
    [string]$HeldoutSpeech = "tests/asr-benchmark/generated/en-005.wav",
    [string]$OutputDirectory = "tests/asr-benchmark/generated/boundary-fixture"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
if (Test-Path -LiteralPath $FfmpegPath) {
    $ffmpeg = (Resolve-Path -LiteralPath $FfmpegPath).Path
}
else {
    $ffmpegCommand = Get-Command -Name $FfmpegPath -CommandType Application -ErrorAction Stop
    $ffmpeg = $ffmpegCommand.Source
}
$calibrationSource = (Resolve-Path -LiteralPath (Join-Path $repoRoot $CalibrationSpeech)).Path
$heldoutSource = (Resolve-Path -LiteralPath (Join-Path $repoRoot $HeldoutSpeech)).Path
$outputRoot = Join-Path $repoRoot $OutputDirectory
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null

function Invoke-Ffmpeg {
    param([string[]]$Arguments)
    & $ffmpeg -hide_banner -loglevel error -y @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "ffmpeg failed with exit code $LASTEXITCODE"
    }
}

function New-SpeechFixture {
    param(
        [string]$Id,
        [string]$Source,
        [string]$Gain
    )
    $target = Join-Path $outputRoot "$Id.wav"
    $filter = "[1:a]silenceremove=start_periods=1:start_threshold=-50dB:start_silence=0.02,atrim=duration=0.70,asetpts=PTS-STARTPTS,volume=$Gain[speech];[0:a][speech]concat=n=2:v=0:a=1,apad=whole_dur=3,atrim=duration=3[out]"
    Invoke-Ffmpeg -Arguments @("-f", "lavfi", "-i", "anullsrc=r=16000:cl=mono:d=2.3", "-i", $Source, "-filter_complex", $filter, "-map", "[out]", "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le", $target)
}

function New-LavfiFixture {
    param(
        [string]$Id,
        [string]$Expression
    )
    $target = Join-Path $outputRoot "$Id.wav"
    Invoke-Ffmpeg -Arguments @("-f", "lavfi", "-i", $Expression, "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le", "-t", "3.000", $target)
}

$definitions = @(
    [pscustomobject]@{ id = "cal-speech-ru-normal"; split = "calibration"; label = "speech"; subtype = "tts-speech-normal"; kind = "speech"; source = $calibrationSource; value = "1.0" },
    [pscustomobject]@{ id = "cal-speech-ru-low"; split = "calibration"; label = "speech"; subtype = "tts-speech-low"; kind = "speech"; source = $calibrationSource; value = "0.35" },
    [pscustomobject]@{ id = "cal-silence"; split = "calibration"; label = "non_speech"; subtype = "silence"; kind = "lavfi"; source = "generated"; value = "anullsrc=r=16000:cl=mono:d=3" },
    [pscustomobject]@{ id = "cal-impulse-borderline"; split = "calibration"; label = "non_speech"; subtype = "isolated-impulse"; kind = "lavfi"; source = "generated"; value = "aevalsrc=if(eq(n\,44000)\,0.008\,0):s=16000:d=3" },
    [pscustomobject]@{ id = "cal-white-noise"; split = "calibration"; label = "non_speech"; subtype = "stationary-white-noise"; kind = "lavfi"; source = "generated"; value = "anoisesrc=color=white:amplitude=0.006:seed=1101:r=16000:d=3" },
    [pscustomobject]@{ id = "cal-handling"; split = "calibration"; label = "non_speech"; subtype = "low-frequency-handling"; kind = "lavfi"; source = "generated"; value = "aevalsrc=if(between(t\,2.70\,2.80)\,0.008*sin(2*PI*80*t)\,0):s=16000:d=3" },
    [pscustomobject]@{ id = "held-speech-en-normal"; split = "heldout"; label = "speech"; subtype = "tts-speech-normal"; kind = "speech"; source = $heldoutSource; value = "1.0" },
    [pscustomobject]@{ id = "held-speech-en-low"; split = "heldout"; label = "speech"; subtype = "tts-speech-low"; kind = "speech"; source = $heldoutSource; value = "0.35" },
    [pscustomobject]@{ id = "held-silence"; split = "heldout"; label = "non_speech"; subtype = "silence"; kind = "lavfi"; source = "generated"; value = "anullsrc=r=16000:cl=mono:d=3" },
    [pscustomobject]@{ id = "held-click-train"; split = "heldout"; label = "non_speech"; subtype = "click-train"; kind = "lavfi"; source = "generated"; value = "aevalsrc=if(gt(between(n\,43200\,43208)+between(n\,44800\,44808)+between(n\,46400\,46408)\,0)\,0.008\,0):s=16000:d=3" },
    [pscustomobject]@{ id = "held-breath-noise"; split = "heldout"; label = "non_speech"; subtype = "band-limited-pink-noise"; kind = "lavfi"; source = "generated"; value = "anoisesrc=color=pink:amplitude=0.010:seed=2202:r=16000:d=3,highpass=f=150,lowpass=f=1200" },
    [pscustomobject]@{ id = "held-handling"; split = "heldout"; label = "non_speech"; subtype = "low-frequency-handling"; kind = "lavfi"; source = "generated"; value = "aevalsrc=if(between(t\,2.62\,2.76)\,0.008*sin(2*PI*63*t)\,0):s=16000:d=3" }
)

$samples = foreach ($definition in $definitions) {
    if ($definition.kind -eq "speech") {
        New-SpeechFixture -Id $definition.id -Source $definition.source -Gain $definition.value
    }
    else {
        New-LavfiFixture -Id $definition.id -Expression $definition.value
    }
    $fileName = "$($definition.id).wav"
    $filePath = Join-Path $outputRoot $fileName
    [pscustomobject]@{
        id = $definition.id
        split = $definition.split
        label = $definition.label
        subtype = $definition.subtype
        path = $fileName
        sha256 = (Get-FileHash -LiteralPath $filePath -Algorithm SHA256).Hash.ToLowerInvariant()
        construction = if ($definition.kind -eq "speech") { "existing-tts-tail:$($definition.value)" } else { $definition.value }
    }
}

$ffmpegVersion = (& $ffmpeg -version | Select-Object -First 1)
$manifest = [ordered]@{
    schema_version = 1
    contract = "../../boundary-contract.md"
    generator = "scripts/prepare-asr-boundary-fixture.ps1"
    ffmpeg = $ffmpegVersion
    samples = @($samples)
}
$manifestPath = Join-Path $outputRoot "manifest.json"
$json = $manifest | ConvertTo-Json -Depth 6
[IO.File]::WriteAllText($manifestPath, $json + "`n", [Text.UTF8Encoding]::new($false))
Write-Output $manifestPath
