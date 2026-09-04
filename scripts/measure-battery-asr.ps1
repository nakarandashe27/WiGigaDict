param(
  [string]$Binary,
  [string[]]$BenchmarkArguments = @(),
  [string]$Output,
  [string]$ProcessLogDirectory,
  [ValidateRange(1, 10)]
  [int]$Repetitions = 3,
  [ValidateRange(5, 120)]
  [int]$IdleSamples = 15,
  [ValidateRange(500, 5000)]
  [int]$PollMilliseconds = 1000,
  [switch]$PreflightOnly
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

function Get-BatterySample {
  $rows = @(Get-CimInstance -Namespace root/wmi -ClassName BatteryStatus -ErrorAction Stop)
  if ($rows.Count -eq 0) { throw "BatteryStatus returned no battery instances" }
  $unknownRate = @($rows | Where-Object { [double]$_.DischargeRate -lt 0 -or [double]$_.DischargeRate -ge 4294967295 })
  if ($unknownRate.Count -gt 0) { throw "BatteryStatus returned an unavailable discharge rate" }
  $dischargeMilliwatts = ($rows.DischargeRate | Measure-Object -Sum).Sum
  [pscustomobject]@{
    captured_at_utc = [DateTimeOffset]::UtcNow.ToString("O")
    battery_count = $rows.Count
    power_online = @($rows | Where-Object PowerOnline).Count -gt 0
    charging = @($rows | Where-Object Charging).Count -gt 0
    discharging = @($rows | Where-Object Discharging).Count -gt 0
    critical = @($rows | Where-Object Critical).Count -gt 0
    discharge_milliwatts = [double]$dischargeMilliwatts
    power_watts = [double]$dischargeMilliwatts / 1000.0
    remaining_capacity_reported = [double](($rows.RemainingCapacity | Measure-Object -Sum).Sum)
    voltage_millivolts = [double](($rows.Voltage | Measure-Object -Average).Average)
  }
}

function Assert-MeasurementReady($Sample, [string]$Stage) {
  if ($Sample.power_online) { throw "AC power is connected during $Stage; battery telemetry run refused" }
  if ($Sample.charging) { throw "Battery is charging during $Stage; battery telemetry run refused" }
  if (-not $Sample.discharging) { throw "Battery is not discharging during $Stage" }
  if ($Sample.critical) { throw "Battery is critical during $Stage" }
  if ($Sample.discharge_milliwatts -le 0) { throw "Battery discharge rate is not positive during $Stage" }
}

$preflight = Get-BatterySample
$preflightResult = [ordered]@{
  schema_version = 1
  telemetry_source = "root/wmi BatteryStatus.DischargeRate"
  measurement_ready = (-not $preflight.power_online -and -not $preflight.charging -and $preflight.discharging -and -not $preflight.critical -and $preflight.discharge_milliwatts -gt 0)
  sample = $preflight
  limitation = "Whole-system battery discharge, not CPU package power. Microsoft documents milliwatts unless the battery reports relative units; BatteryStaticData capability lookup fails on this machine, so absolute-unit capability is not independently verified."
}

if ($PreflightOnly) {
  $preflightResult | ConvertTo-Json -Depth 6
  return
}

if (-not $Binary) { throw "-Binary is required unless -PreflightOnly is used" }
if (-not $Output) { throw "-Output is required unless -PreflightOnly is used" }
if ($BenchmarkArguments.Count -eq 0) { throw "-BenchmarkArguments must not be empty" }
if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) { throw "Benchmark binary not found: $Binary" }
if (Test-Path -LiteralPath $Output) { throw "Telemetry output already exists: $Output" }
Assert-MeasurementReady $preflight "preflight"

$outputDirectory = Split-Path -Parent $Output
if (-not $outputDirectory) { $outputDirectory = (Get-Location).Path }
$logDirectory = if ($ProcessLogDirectory) {
  $ProcessLogDirectory
} else {
  Join-Path $outputDirectory ((Split-Path -LeafBase $Output) + "-process-logs")
}
if (Test-Path -LiteralPath $logDirectory) { throw "Process log directory already exists: $logDirectory" }
New-Item -ItemType Directory -Path $logDirectory | Out-Null

$idle = for ($index = 0; $index -lt $IdleSamples; $index++) {
  $sample = Get-BatterySample
  Assert-MeasurementReady $sample "idle sample $($index + 1)"
  $sample
  Start-Sleep -Milliseconds $PollMilliseconds
}
$idlePowerWatts = ($idle.power_watts | Measure-Object -Average).Average

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
  try {
    while (-not $process.HasExited) {
      $sample = Get-BatterySample
      Assert-MeasurementReady $sample "benchmark repetition $repetition"
      $samples.Add([pscustomobject]@{
          elapsed_ms = $timer.Elapsed.TotalMilliseconds
          power_watts = $sample.power_watts
          remaining_capacity_reported = $sample.remaining_capacity_reported
          voltage_millivolts = $sample.voltage_millivolts
        })
      Start-Sleep -Milliseconds $PollMilliseconds
    }
  } finally {
    if (-not $process.HasExited) { $process.Kill($true) }
  }
  $process.WaitForExit()
  $timer.Stop()
  $stdout = $stdoutTask.GetAwaiter().GetResult()
  $stderr = $stderrTask.GetAwaiter().GetResult()
  [IO.File]::WriteAllText($stdoutPath, $stdout, [Text.UTF8Encoding]::new($false))
  [IO.File]::WriteAllText($stderrPath, $stderr, [Text.UTF8Encoding]::new($false))
  if ($process.ExitCode -ne 0) { throw "Benchmark repetition $repetition exited $($process.ExitCode)" }
  if ($samples.Count -eq 0) { throw "No battery telemetry samples captured for repetition $repetition" }

  $wholeSystemWattSeconds = 0.0
  $incrementalWattSeconds = 0.0
  for ($sampleIndex = 1; $sampleIndex -lt $samples.Count; $sampleIndex++) {
    $seconds = ($samples[$sampleIndex].elapsed_ms - $samples[$sampleIndex - 1].elapsed_ms) / 1000.0
    $leftPower = $samples[$sampleIndex - 1].power_watts
    $rightPower = $samples[$sampleIndex].power_watts
    $wholeSystemWattSeconds += (($leftPower + $rightPower) / 2.0) * $seconds
    $leftIncremental = [Math]::Max(0.0, $leftPower - $idlePowerWatts)
    $rightIncremental = [Math]::Max(0.0, $rightPower - $idlePowerWatts)
    $incrementalWattSeconds += (($leftIncremental + $rightIncremental) / 2.0) * $seconds
  }
  if ($samples.Count -eq 1) {
    $wholeSystemWattSeconds = $samples[0].power_watts * $timer.Elapsed.TotalSeconds
    $incrementalWattSeconds = [Math]::Max(0.0, $samples[0].power_watts - $idlePowerWatts) * $timer.Elapsed.TotalSeconds
  }
  $averagePowerWatts = ($samples.power_watts | Measure-Object -Average).Average
  [pscustomobject]@{
    repetition = $repetition
    started_at_utc = $startedAt.ToString("O")
    elapsed_ms = [Math]::Round($timer.Elapsed.TotalMilliseconds, 3)
    telemetry_samples = $samples.Count
    average_whole_system_watts = [Math]::Round($averagePowerWatts, 4)
    average_incremental_watts = [Math]::Round($incrementalWattSeconds / [Math]::Max($timer.Elapsed.TotalSeconds, 0.001), 4)
    whole_system_energy_kwh = [Math]::Round($wholeSystemWattSeconds / 3600000.0, 10)
    incremental_energy_kwh = [Math]::Round($incrementalWattSeconds / 3600000.0, 10)
    capacity_delta_reported = [Math]::Max(0.0, $samples[0].remaining_capacity_reported - $samples[$samples.Count - 1].remaining_capacity_reported)
    stdout_log = $stdoutPath
    stderr_log = $stderrPath
  }
}

$report = [ordered]@{
  schema_version = 1
  measurement_scope = "whole benchmark worker process on battery; model load, optional warmup and measured inference"
  telemetry_source = "root/wmi BatteryStatus.DischargeRate"
  units = "battery-reported milliwatts converted to watts"
  limitation = $preflightResult.limitation
  idle = [ordered]@{
    sample_count = $IdleSamples
    average_whole_system_watts = [Math]::Round($idlePowerWatts, 4)
  }
  poll_milliseconds = $PollMilliseconds
  benchmark_binary = $Binary
  benchmark_arguments = $BenchmarkArguments
  runs = @($runs)
}

$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Output -Encoding utf8
Write-Host "Captured $($runs.Count) battery-discharge telemetry runs into $Output"
