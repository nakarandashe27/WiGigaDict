param(
  [ValidateSet("LargeTurboQ5", "LargeTurboQ8", "LargeV3Q5", "SmallQ5")]
  [string]$Variant = "LargeTurboQ5"
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$repoRoot = Split-Path -Parent $PSScriptRoot
$revision = "5359861c739e955e79d9a303bcbc70fb988958b1"
$modelDir = Join-Path $repoRoot "tests/asr-benchmark/private/models/whisper.cpp/$revision"
$logPath = Join-Path $repoRoot ("logs/prepare-whisper-benchmark-{0:yyyyMMdd-HHmmss}.transcript" -f (Get-Date))

$artifact = switch ($Variant) {
  "LargeTurboQ5" {
    @{ Filename = "ggml-large-v3-turbo-q5_0.bin"; Bytes = 574041195L; Sha256 = "394221709CD5AD1F40C46E6031CA61BCE88931E6E088C188294C6D5A55FFA7E2" }
  }
  "LargeTurboQ8" {
    @{ Filename = "ggml-large-v3-turbo-q8_0.bin"; Bytes = 874188075L; Sha256 = "317EB69C11673C9DE1E1F0D459B253999804EC71AC4C23C17ECF5FBE24E259A1" }
  }
  "LargeV3Q5" {
    @{ Filename = "ggml-large-v3-q5_0.bin"; Bytes = 1081140203L; Sha256 = "D75795ECFF3F83B5FAA89D1900604AD8C780ABD5739FAE406DE19F23ECD98AD1" }
  }
  "SmallQ5" {
    @{ Filename = "ggml-small-q5_1.bin"; Bytes = 190085487L; Sha256 = "AE85E4A935D7A567BD102FE55AFC16BB595BDB618E11B2FC7591BC08120411BB" }
  }
}

function Test-Artifact {
  param([string]$Path, [long]$Bytes, [string]$Sha256)
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
  if ((Get-Item -LiteralPath $Path).Length -ne $Bytes) { return $false }
  return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -eq $Sha256
}

Start-Transcript -LiteralPath $logPath | Out-Null
try {
  New-Item -ItemType Directory -Path $modelDir -Force | Out-Null
  $final = Join-Path $modelDir $artifact.Filename
  if (-not (Test-Artifact -Path $final -Bytes $artifact.Bytes -Sha256 $artifact.Sha256)) {
    $part = "$final.part"
    $url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/$revision/$($artifact.Filename)"
    curl.exe --fail --location --retry 3 --retry-all-errors --continue-at - --output $part $url
    if (-not (Test-Artifact -Path $part -Bytes $artifact.Bytes -Sha256 $artifact.Sha256)) {
      throw "Artifact validation failed: $($artifact.Filename)"
    }
    Move-Item -LiteralPath $part -Destination $final -Force
  }
  Write-Host "Verified $($artifact.Filename)"
}
finally { Stop-Transcript | Out-Null }

Write-Host "Whisper benchmark model ready: $final"
