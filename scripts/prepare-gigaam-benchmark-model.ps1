param(
  [ValidateSet("Int8", "Fp32")]
  [string]$Variant = "Int8"
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$repoRoot = Split-Path -Parent $PSScriptRoot
$revision = "322c3b29492673eb7d0b434bfa9dfb8653e34d02"
$modelDir = Join-Path $repoRoot "tests/asr-benchmark/private/models/gigaam-v3-onnx/$revision"
$logPath = Join-Path $repoRoot ("logs/prepare-gigaam-benchmark-{0:yyyyMMdd-HHmmss}.transcript" -f (Get-Date))

$artifacts = @(
  @{ Source = "v3_vocab.txt"; Target = "vocab.txt"; Bytes = 198L; Sha256 = "A9143C30844D3C0BEE3E9E927E4084774EB1B9EEAAFC473B2C4521E4911A7C07" },
  @{ Source = "LICENSE.txt"; Target = "LICENSE.txt"; Bytes = 1070L; Sha256 = "F00DE6715714C7A63D08639CDBFAA40224EEFC407302614BD19F1A8B98C875AA" }
)
if ($Variant -eq "Int8") {
  $artifacts += @{ Source = "v3_ctc.int8.onnx"; Target = "model.int8.onnx"; Bytes = 224721181L; Sha256 = "CEB61454E2E1A2DEC5872CBAC1DE0FE0A4271D1148F6B26B5BDA53FF30A12ACD" }
}
else {
  $artifacts += @{ Source = "v3_ctc.onnx"; Target = "model.onnx"; Bytes = 885264128L; Sha256 = "1FB978D4F41E1334003FAE9D29F2D9844D4132315A90553A4B986839051BA9D3" }
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
  foreach ($artifact in $artifacts) {
    $final = Join-Path $modelDir $artifact.Target
    if (Test-Artifact -Path $final -Bytes $artifact.Bytes -Sha256 $artifact.Sha256) {
      Write-Host "Verified existing $($artifact.Target)"
      continue
    }
    $part = "$final.part"
    $url = "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/$revision/$($artifact.Source)"
    curl.exe --fail --location --retry 3 --retry-all-errors --continue-at - --output $part $url
    if (-not (Test-Artifact -Path $part -Bytes $artifact.Bytes -Sha256 $artifact.Sha256)) {
      throw "Artifact validation failed: $($artifact.Target)"
    }
    Move-Item -LiteralPath $part -Destination $final -Force
    Write-Host "Downloaded and verified $($artifact.Target)"
  }
}
finally {
  Stop-Transcript | Out-Null
}

Write-Host "GigaAM benchmark model ready: $modelDir"
