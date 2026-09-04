param(
  [Parameter(Mandatory = $true)]
  [string]$Evidence,
  [string]$Manifest = "tests/asr-benchmark/corpus/smoke-manifest.json",
  [Parameter(Mandatory = $true)]
  [string]$Output
)

$ErrorActionPreference = "Stop"

function ConvertTo-NormalizedText {
  param([string]$Text)
  return (($Text.ToLowerInvariant() -replace "[^\p{L}\p{N}]+", " ").Trim() -replace "\s+", " ")
}

function Test-TerminalMarker {
  param([string]$Text, [object[]]$Markers)
  $normalizedText = ConvertTo-NormalizedText $Text
  foreach ($marker in $Markers) {
    $normalizedMarker = ConvertTo-NormalizedText ([string]$marker)
    if ($normalizedMarker -and (
        $normalizedText -ceq $normalizedMarker -or
        $normalizedText.EndsWith(" $normalizedMarker", [StringComparison]::Ordinal)
      )) {
      return $true
    }
  }
  return $false
}

function Assert-RecordContract {
  param($Record, $Reference)

  if ($Record.mode -notin @("cold", "warm")) {
    throw "Unsupported mode '$($Record.mode)' for sample $($Record.sample_id)"
  }
  if ($Record.profile -notmatch '^(cpu|gpu|cpu-t([1-9]|[1-5][0-9]|6[0-4]))$') {
    throw "Unsupported profile '$($Record.profile)' for sample $($Record.sample_id)"
  }
  if ($Record.profile -like "cpu-t*") {
    if ($Record.engine -ne "whisper") {
      throw "Thread-pinned profile '$($Record.profile)' is only valid for Whisper"
    }
    $declaredThreads = [int]$Matches[2]
    if ($null -eq $Record.n_threads -or [int]$Record.n_threads -ne $declaredThreads) {
      throw "Profile '$($Record.profile)' does not match n_threads=$($Record.n_threads)"
    }
  }
  elseif ($Record.engine -eq "whisper" -and $Record.profile -eq "cpu" -and
      $null -ne $Record.n_threads -and [int]$Record.n_threads -ne 0) {
    throw "Whisper profile cpu requires n_threads=0; use cpu-tN for a pinned count"
  }
  if ($Record.language -notin @("ru", "en") -or $Record.language -ne $Reference.language) {
    throw "Language '$($Record.language)' does not match manifest for sample $($Record.sample_id)"
  }
  if ([long]$Record.audio_duration_ms -le 0) {
    throw "Invalid audio_duration_ms for sample $($Record.sample_id)"
  }

  $index = 0
  foreach ($segment in @($Record.segments)) {
    if ([long]$segment.start_ms -lt 0 -or [long]$segment.end_ms -gt [long]$Record.audio_duration_ms) {
      throw "Segment $index for sample $($Record.sample_id) is outside WAV duration $($Record.audio_duration_ms) ms"
    }
    $index++
  }
}

function Get-LevenshteinDistance {
  param([object[]]$Left, [object[]]$Right)
  $leftItems = @($Left)
  $rightItems = @($Right)
  $previous = [int[]](0..$rightItems.Count)
  for ($i = 1; $i -le $leftItems.Count; $i++) {
    $current = [int[]]::new($rightItems.Count + 1)
    $current[0] = $i
    for ($j = 1; $j -le $rightItems.Count; $j++) {
      $cost = if ($leftItems[$i - 1] -ceq $rightItems[$j - 1]) { 0 } else { 1 }
      $current[$j] = [Math]::Min(
        [Math]::Min($current[$j - 1] + 1, $previous[$j] + 1),
        $previous[$j - 1] + $cost
      )
    }
    $previous = $current
  }
  return $previous[$rightItems.Count]
}

function Get-NearestRankPercentile {
  param([double[]]$Values, [double]$Percentile)
  $sorted = @($Values | Sort-Object)
  $index = [Math]::Max(0, [Math]::Ceiling($Percentile * $sorted.Count) - 1)
  return $sorted[$index]
}

$manifestObject = Get-Content -Raw -LiteralPath $Manifest | ConvertFrom-Json
$references = @{}
foreach ($sample in $manifestObject.samples) { $references[$sample.id] = $sample }
$records = @(Get-Content -LiteralPath $Evidence | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json })
if ($records.Count -eq 0) { throw "No benchmark records in $Evidence" }

$scored = foreach ($record in $records) {
  $reference = $references[$record.sample_id]
  if (-not $reference) { throw "Missing reference for $($record.sample_id)" }
  Assert-RecordContract $record $reference
  $referenceText = ConvertTo-NormalizedText $reference.reference
  $hypothesisText = ConvertTo-NormalizedText $record.text
  $referenceWords = @($referenceText -split " " | Where-Object { $_ })
  $hypothesisWords = @($hypothesisText -split " " | Where-Object { $_ })
  $referenceChars = @($referenceText.ToCharArray())
  $hypothesisChars = @($hypothesisText.ToCharArray())
  $missingTokens = @($reference.technical_tokens | Where-Object {
      $token = ConvertTo-NormalizedText $_
      -not $hypothesisText.Contains($token)
    })
  $markers = @($reference.final_marker) + @($reference.final_marker_aliases)
  $markerPresent = Test-TerminalMarker $record.text $markers
  $monotonic = $true
  $previousEnd = 0
  foreach ($segment in $record.segments) {
    if ($segment.start_ms -lt $previousEnd -or $segment.end_ms -lt $segment.start_ms) { $monotonic = $false }
    $previousEnd = $segment.end_ms
  }
  [pscustomobject]@{
    engine = $record.engine
    profile = $record.profile
    mode = $record.mode
    sample_id = $record.sample_id
    inference_ms = [double]$record.inference_ms
    rtf = [double]$record.rtf
    peak_working_set_bytes = [double]$record.peak_working_set_bytes
    wer = (Get-LevenshteinDistance $referenceWords $hypothesisWords) / [Math]::Max(1, $referenceWords.Count)
    cer = (Get-LevenshteinDistance $referenceChars $hypothesisChars) / [Math]::Max(1, $referenceChars.Count)
    technical_token_errors = $missingTokens.Count
    technical_token_total = @($reference.technical_tokens).Count
    marker_present = $markerPresent
    monotonic_segments = $monotonic
  }
}

$groups = @($scored | Group-Object engine,profile,mode | ForEach-Object {
    $items = @($_.Group)
    [pscustomobject]@{
      engine = $items[0].engine
      profile = $items[0].profile
      mode = $items[0].mode
      count = $items.Count
      inference_p50_ms = Get-NearestRankPercentile @($items.inference_ms) 0.50
      inference_p95_ms = Get-NearestRankPercentile @($items.inference_ms) 0.95
      rtf_p50 = Get-NearestRankPercentile @($items.rtf) 0.50
      rtf_p95 = Get-NearestRankPercentile @($items.rtf) 0.95
      peak_ram_max_bytes = ($items.peak_working_set_bytes | Measure-Object -Maximum).Maximum
      mean_wer = ($items.wer | Measure-Object -Average).Average
      mean_cer = ($items.cer | Measure-Object -Average).Average
      technical_token_errors = ($items.technical_token_errors | Measure-Object -Sum).Sum
      technical_token_total = ($items.technical_token_total | Measure-Object -Sum).Sum
      final_marker_miss_count = @($items | Where-Object { -not $_.marker_present }).Count
      non_monotonic_count = @($items | Where-Object { -not $_.monotonic_segments }).Count
    }
  })

$result = [ordered]@{
  schema_version = 2
  evidence = $Evidence
  corpus_id = $manifestObject.corpus_id
  record_count = $records.Count
  model_hashes = @($records.model_sha256 | Sort-Object -Unique)
  audio_hashes = @($records.audio_sha256 | Sort-Object -Unique)
  model_bytes = @($records.model_bytes | Sort-Object -Unique)
  percentile_method = "nearest_rank"
  groups = $groups
  samples = @($scored)
}
$parent = Split-Path -Parent $Output
if ($parent) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Output -Encoding utf8NoBOM
Write-Host "Analyzed $($records.Count) records into $Output"

