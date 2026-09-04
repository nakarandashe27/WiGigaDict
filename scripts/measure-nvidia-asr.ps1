param(
  [Parameter(Mandatory = $true)]
  [string]$Binary,
  [Parameter(Mandatory = $true)]
  [string[]]$BenchmarkArguments,
  [Parameter(Mandatory = $true)]
  [string]$Output,
  [string]$ProcessLogDirectory,
  [ValidateRange(1, 20)]
  [int]$Repetitions = 5,
  [ValidateRange(3, 100)]
  [int]$IdleSamples = 10,
  [ValidateRange(50, 2000)]
  [int]$PollMilliseconds = 100,
  [ValidateRange(0, 31)]
  [int]$NvidiaIndex = 0
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$culture = [Globalization.CultureInfo]::InvariantCulture
$mib = 1MB

if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) { throw "Benchmark binary not found: $Binary" }
if (Test-Path -LiteralPath $Output) { throw "Telemetry output already exists: $Output" }
$nvidiaSmi = (Get-Command nvidia-smi.exe -ErrorAction Stop).Source
$outputDirectory = Split-Path -Parent $Output
if (-not $outputDirectory) { $outputDirectory = (Get-Location).Path }
$logDirectory = if ($ProcessLogDirectory) {
  $ProcessLogDirectory
} else {
  Join-Path $outputDirectory ((Split-Path -LeafBase $Output) + "-process-logs")
}
New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null

function Get-NvidiaSample {
  $line = @(& $nvidiaSmi -i $NvidiaIndex --query-gpu=power.draw,memory.used --format=csv,noheader,nounits)
  if ($LASTEXITCODE -ne 0 -or -not $line) { throw "nvidia-smi telemetry query failed" }
  $fields = @($line[0].Split(",") | ForEach-Object { $_.Trim() })
  if ($fields.Count -ne 2) { throw "Unexpected nvidia-smi telemetry row: $($line[0])" }
  [pscustomobject]@{
    power_watts = [double]::Parse($fields[0], $culture)
    memory_mib = [double]::Parse($fields[1], $culture)
  }
}

$identityLine = @(& $nvidiaSmi -i $NvidiaIndex --query-gpu=index,name,uuid,driver_version --format=csv,noheader)
if ($LASTEXITCODE -ne 0 -or -not $identityLine) { throw "nvidia-smi identity query failed" }
$identity = @($identityLine[0].Split(",") | ForEach-Object { $_.Trim() })
if ($identity.Count -ne 4) { throw "Unexpected nvidia-smi identity row: $($identityLine[0])" }

$idle = for ($index = 0; $index -lt $IdleSamples; $index++) {
  Get-NvidiaSample
  Start-Sleep -Milliseconds $PollMilliseconds
}
$idlePowerWatts = ($idle.power_watts | Measure-Object -Average).Average
$idleMemoryMib = ($idle.memory_mib | Measure-Object -Average).Average

$runs = for ($repetition = 1; $repetition -le $Repetitions; $repetition++) {
  $stdoutPath = Join-Path $logDirectory ("run-{0:D2}.stdout.log" -f $repetition)
  $stderrPath = Join-Path $logDirectory ("run-{0:D2}.stderr.log" -f $repetition)
  $processInfo = [Diagnostics.ProcessStartInfo]::new($Binary)
  $processInfo.UseShellExecute = $false
  $processInfo.CreateNoWindow = $true
  $processInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
  $processInfo.RedirectStandardOutput = $true
  $processInfo.RedirectStandardError = $true
  foreach ($argument in $BenchmarkArguments) { $processInfo.ArgumentList.Add($argument) }
  $process = [Diagnostics.Process]::new()
  $process.StartInfo = $processInfo
  $startedAt = [DateTimeOffset]::UtcNow
  $timer = [Diagnostics.Stopwatch]::StartNew()
  if (-not $process.Start()) { throw "Failed to start benchmark process" }
  $stdoutTask = $process.StandardOutput.ReadToEndAsync()
  $stderrTask = $process.StandardError.ReadToEndAsync()
  $samples = [Collections.Generic.List[object]]::new()
  while (-not $process.HasExited) {
    $sample = Get-NvidiaSample
    $samples.Add([pscustomobject]@{
        elapsed_ms = $timer.Elapsed.TotalMilliseconds
        power_watts = $sample.power_watts
        memory_mib = $sample.memory_mib
      })
    Start-Sleep -Milliseconds $PollMilliseconds
  }
  $process.WaitForExit()
  $timer.Stop()
  $stdout = $stdoutTask.GetAwaiter().GetResult()
  $stderr = $stderrTask.GetAwaiter().GetResult()
  [IO.File]::WriteAllText($stdoutPath, $stdout, [Text.UTF8Encoding]::new($false))
  [IO.File]::WriteAllText($stderrPath, $stderr, [Text.UTF8Encoding]::new($false))
  if ($process.ExitCode -ne 0) { throw "Benchmark repetition $repetition exited $($process.ExitCode)" }
  if ($samples.Count -eq 0) { throw "No NVIDIA telemetry samples captured for repetition $repetition" }

  $incrementalWattSeconds = 0.0
  for ($sampleIndex = 1; $sampleIndex -lt $samples.Count; $sampleIndex++) {
    $seconds = ($samples[$sampleIndex].elapsed_ms - $samples[$sampleIndex - 1].elapsed_ms) / 1000.0
    $left = [Math]::Max(0.0, $samples[$sampleIndex - 1].power_watts - $idlePowerWatts)
    $right = [Math]::Max(0.0, $samples[$sampleIndex].power_watts - $idlePowerWatts)
    $incrementalWattSeconds += (($left + $right) / 2.0) * $seconds
  }
  if ($samples.Count -eq 1) {
    $incrementalWattSeconds = [Math]::Max(0.0, $samples[0].power_watts - $idlePowerWatts) * $timer.Elapsed.TotalSeconds
  }
  $averagePowerWatts = ($samples.power_watts | Measure-Object -Average).Average
  $peakMemoryMib = ($samples.memory_mib | Measure-Object -Maximum).Maximum
  [pscustomobject]@{
    repetition = $repetition
    started_at_utc = $startedAt.ToString("O")
    elapsed_ms = [Math]::Round($timer.Elapsed.TotalMilliseconds, 3)
    telemetry_samples = $samples.Count
    average_power_watts = [Math]::Round($averagePowerWatts, 4)
    average_incremental_watts = [Math]::Round($incrementalWattSeconds / [Math]::Max($timer.Elapsed.TotalSeconds, 0.001), 4)
    incremental_energy_kwh = [Math]::Round($incrementalWattSeconds / 3600000.0, 10)
    peak_memory_mib = $peakMemoryMib
    peak_incremental_vram_bytes = [Math]::Max(0, [long][Math]::Round(($peakMemoryMib - $idleMemoryMib) * $mib))
    stdout_log = $stdoutPath
    stderr_log = $stderrPath
  }
}

$report = [ordered]@{
  schema_version = 1
  measurement_scope = "whole benchmark worker process; model load, optional warmup and measured inference"
  telemetry_source = "nvidia-smi"
  gpu = [ordered]@{
    index = [int]$identity[0]
    name = $identity[1]
    uuid = $identity[2]
    driver_version = $identity[3]
  }
  idle = [ordered]@{
    sample_count = $IdleSamples
    average_power_watts = [Math]::Round($idlePowerWatts, 4)
    average_memory_mib = [Math]::Round($idleMemoryMib, 4)
  }
  poll_milliseconds = $PollMilliseconds
  benchmark_binary = $Binary
  benchmark_arguments = $BenchmarkArguments
  runs = @($runs)
}

$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Output -Encoding utf8
Write-Host "Captured $($runs.Count) process-level NVIDIA telemetry runs into $Output"
