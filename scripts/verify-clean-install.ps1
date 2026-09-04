<#
.SYNOPSIS
  Проверка clean install personal alpha на чистой Windows-VM (Step 17).

.DESCRIPTION
  Запускается ВНУТРИ виртуальной машины, от обычного пользователя, без репозитория.
  Скопируйте в VM: этот скрипт, WiGigaDict_<версия>_x64-setup.exe и ggml-модель.
  Речевой образец для проверки распознавания скрипт синтезирует сам через встроенный в
  Windows SAPI — микрофон и запись не нужны; -SpeechWav переопределяет его своим файлом
  (16 kHz mono s16).

  Механические проверки автоматизированы; микрофонная диктовка и вставка в редактор
  остаются ручными — их строки попадают в отчёт со статусом MANUAL.

  PowerShell 5.1: pwsh на чистой VM нет.

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File .\verify-clean-install.ps1 `
    -Installer .\WiGigaDict_0.0.4_x64-setup.exe -ModelPath .\ggml-base.bin
#>
param(
  [string]$Installer,
  [string]$ModelPath,
  [string]$SpeechWav,
  [string]$PreviousInstaller,
  [string]$Report,
  [switch]$SkipUninstallCycle,
  [switch]$SelfTest
)

$ErrorActionPreference = "Stop"
$installRoot = Join-Path $env:LOCALAPPDATA "WiGigaDict"
$dataDirs = @("storage", "audio", "logs", "installed", "quarantine")
$appFiles = @(
  "wigigadict-desktop.exe",
  "wigigadict-asr-sidecar.exe",
  "wigigadict-asr-worker.exe",
  "catalog.json",
  "catalog.sig",
  "uninstall.exe"
)
if (-not $Report) {
  $Report = Join-Path (Get-Location) ("clean-install-report-{0:yyyyMMdd-HHmmss}.md" -f (Get-Date))
}

$script:results = @()

function Add-Result([string]$name, [bool]$ok, [string]$detail) {
  $verdict = "FAIL"
  if ($ok) { $verdict = "PASS" }
  $script:results += [pscustomobject]@{ Check = $name; Verdict = $verdict; Detail = $detail }
  Write-Output ("[{0}] {1} — {2}" -f $verdict, $name, $detail)
}

function Add-Manual([string]$name, [string]$detail) {
  $script:results += [pscustomobject]@{ Check = $name; Verdict = "MANUAL"; Detail = $detail }
  Write-Output ("[MANUAL] {0} — {1}" -f $name, $detail)
}

function Wait-For([scriptblock]$condition, [int]$timeoutSeconds) {
  $deadline = (Get-Date).AddSeconds($timeoutSeconds)
  while ((Get-Date) -lt $deadline) {
    if (& $condition) { return $true }
    Start-Sleep -Milliseconds 500
  }
  return (& $condition)
}

function Get-DataInventory {
  $inventory = @{}
  foreach ($dir in $dataDirs) {
    $path = Join-Path $installRoot $dir
    if (Test-Path -LiteralPath $path) {
      $inventory[$dir] = @(Get-ChildItem -LiteralPath $path -Recurse -File -ErrorAction SilentlyContinue).Count
    }
  }
  return $inventory
}

function Format-Inventory($inventory) {
  $parts = @()
  foreach ($key in $inventory.Keys) { $parts += ("{0}={1}" -f $key, $inventory[$key]) }
  if ($parts.Count -eq 0) { return "пусто" }
  return ($parts -join ", ")
}

# Возвращает список каталогов, где файлов стало меньше, чем было.
function Get-LostDirectories($before, $after) {
  $lost = @()
  foreach ($dir in $before.Keys) {
    if (-not $after.ContainsKey($dir) -or $after[$dir] -lt $before[$dir]) { $lost += $dir }
  }
  return $lost
}

# Речевой образец синтезируется встроенным в Windows SAPI: микрофон и запись для проверки
# тракта распознавания не нужны, а на белом шуме whisper галлюцинирует и ничего не доказывает.
function New-SpeechSample([string]$path) {
  Add-Type -AssemblyName System.Speech
  $synthesizer = New-Object System.Speech.Synthesis.SpeechSynthesizer
  $voices = @($synthesizer.GetInstalledVoices() | Where-Object { $_.Enabled })
  $voice = @($voices | Where-Object { $_.VoiceInfo.Culture.Name -eq "ru-RU" })[0]
  $phrase = "Проверка установки на чистой машине. Распознавание работает на процессоре."
  $language = "ru"
  if (-not $voice) {
    $voice = $voices[0]
    $phrase = "This is a clean install check. Recognition runs on the processor."
    $language = "en"
  }
  if (-not $voice) {
    $synthesizer.Dispose()
    return $null
  }
  $synthesizer.SelectVoice($voice.VoiceInfo.Name)
  $format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
  $synthesizer.SetOutputToWaveFile($path, $format)
  $synthesizer.Speak($phrase)
  $synthesizer.SetOutputToNull()
  $synthesizer.Dispose()
  return [pscustomobject]@{ Path = $path; Phrase = $phrase; Language = $language; Voice = $voice.VoiceInfo.Name }
}

function Split-Words([string]$text) {
  $cleaned = ($text.ToLowerInvariant() -replace "[^\p{L}\p{Nd}]+", " ").Trim()
  if ($cleaned.Length -eq 0) { return @() }
  return @($cleaned -split "\s+")
}

# Доля слов синтезированной фразы, найденных в распознанном тексте.
function Get-WordRecall([string]$expected, [string]$actual) {
  $expectedWords = Split-Words $expected
  if ($expectedWords.Count -eq 0) { return 0.0 }
  $actualWords = Split-Words $actual
  $matched = @($expectedWords | Where-Object { $actualWords -contains $_ }).Count
  return [math]::Round($matched / $expectedWords.Count, 2)
}

# Модель берётся явным путём, иначе самая маленькая из установленных приложением.
function Resolve-ModelPath([string]$explicit) {
  if ($explicit) { return (Resolve-Path -LiteralPath $explicit).Path }
  $installed = Join-Path $installRoot "installed"
  if (-not (Test-Path -LiteralPath $installed)) { return $null }
  $candidate = @(Get-ChildItem -LiteralPath $installed -Recurse -Filter *.bin -ErrorAction SilentlyContinue | Sort-Object Length)[0]
  if ($candidate) { return $candidate.FullName }
  return $null
}

function Install-Package([string]$path, [string]$label) {
  $started = Get-Date
  $process = Start-Process -FilePath $path -ArgumentList "/S" -PassThru -Wait
  $elapsed = [int]((Get-Date) - $started).TotalSeconds
  $ok = ($process.ExitCode -eq 0) -and (Wait-For { Test-Path -LiteralPath (Join-Path $installRoot "wigigadict-desktop.exe") } 120)
  Add-Result $label $ok ("exit={0}, {1} с" -f $process.ExitCode, $elapsed)
  return $ok
}

function Start-AppSoak([string]$label, [int]$soakSeconds) {
  $exe = Join-Path $installRoot "wigigadict-desktop.exe"
  $app = Start-Process -FilePath $exe -PassThru
  $alive = Wait-For { $app.Refresh(); -not $app.HasExited } 5
  Start-Sleep -Seconds $soakSeconds
  $app.Refresh()
  $alive = $alive -and (-not $app.HasExited)
  $memoryMb = 0
  if ($alive) { $memoryMb = [int]($app.WorkingSet64 / 1MB) }
  $storageOk = Test-Path -LiteralPath (Join-Path $installRoot "storage")
  Add-Result $label ($alive -and $storageOk) ("жив после {0} с: {1}, storage: {2}, working set: {3} МБ" -f $soakSeconds, $alive, $storageOk, $memoryMb)
  if ($alive) {
    Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
  }
  return ($alive -and $storageOk)
}

function Uninstall-Package([string]$label) {
  $started = Get-Date
  $process = Start-Process -FilePath (Join-Path $installRoot "uninstall.exe") -ArgumentList "/S" -PassThru -Wait
  $null = Wait-For { -not (Test-Path -LiteralPath (Join-Path $installRoot "wigigadict-desktop.exe")) } 60
  $elapsed = [int]((Get-Date) - $started).TotalSeconds
  $leftovers = @()
  foreach ($file in $appFiles) {
    if ($file -eq "uninstall.exe") { continue }
    if (Test-Path -LiteralPath (Join-Path $installRoot $file)) { $leftovers += $file }
  }
  $detail = "exit={0}, {1} с" -f $process.ExitCode, $elapsed
  if ($leftovers.Count -gt 0) { $detail = $detail + ("; остались: " + ($leftovers -join ", ")) }
  Add-Result $label ($leftovers.Count -eq 0) $detail
}

if ($SelfTest) {
  $full = @{ storage = 3; audio = 2 }
  if ((Get-LostDirectories $full $full).Count -ne 0) { throw "self-test: неизменный набор не должен считаться потерей" }
  if ((Get-LostDirectories $full @{ storage = 3; audio = 5 }).Count -ne 0) { throw "self-test: рост числа файлов не потеря" }
  if ((Get-LostDirectories $full @{ storage = 3 }) -join "," -ne "audio") { throw "self-test: исчезнувший каталог не пойман" }
  if ((Get-LostDirectories $full @{ storage = 1; audio = 2 }) -join "," -ne "storage") { throw "self-test: усохший каталог не пойман" }
  if ((Format-Inventory @{}) -ne "пусто") { throw "self-test: пустая опись" }
  if ((Get-WordRecall "Проверка на чистой машине" "проверка, на чистой машине!") -ne 1) { throw "self-test: пунктуация и регистр не должны мешать" }
  if ((Get-WordRecall "Проверка на чистой машине" "проверка на грязной тачке") -ne 0.5) { throw "self-test: частичное совпадение считается неверно" }
  if ((Get-WordRecall "Проверка на чистой машине" "") -ne 0) { throw "self-test: пустой ответ не может дать совпадение" }
  Write-Output "self-test ok"
  return
}
if (-not $Installer) { throw "укажите -Installer (или -SelfTest)" }

Write-Output "== 1. Снимок среды до установки =="
$os = Get-CimInstance Win32_OperatingSystem
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltinRole]::Administrator)
Add-Result "Запуск без прав администратора" (-not $isAdmin) ("elevated={0}" -f $isAdmin)
Add-Result "Windows x64" ($env:PROCESSOR_ARCHITECTURE -eq "AMD64") ("{0} build {1}, {2}" -f $os.Caption, $os.BuildNumber, $env:PROCESSOR_ARCHITECTURE)

$webviewKeys = @(
  "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
  "HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
)
$webview = "отсутствует"
foreach ($key in $webviewKeys) {
  try {
    $pv = (Get-ItemProperty -Path $key -Name pv -ErrorAction Stop).pv
    if ($pv) { $webview = $pv; break }
  }
  catch { }
}
Add-Manual "WebView2 до установки" ("версия: {0}; при отсутствии установщик тянет bootstrapper из сети" -f $webview)

$vcRuntime = (Test-Path "$env:SystemRoot\System32\vcruntime140.dll") -and (Test-Path "$env:SystemRoot\System32\msvcp140.dll")
Add-Manual "VC++ runtime до установки" ("vcruntime140 + msvcp140: {0}" -f $vcRuntime)
$python = Get-Command python -ErrorAction SilentlyContinue
Add-Result "Python не требуется" ($null -eq $python) ("python в PATH: {0}" -f ($null -ne $python))
$vulkan = Test-Path "$env:SystemRoot\System32\vulkan-1.dll"
Add-Manual "Vulkan loader до установки" ("vulkan-1.dll: {0}; CPU-профиль от него не зависит" -f $vulkan)
$systemDrive = $env:SystemDrive.TrimEnd(":")
$freeGb = [math]::Round((Get-PSDrive -Name $systemDrive).Free / 1GB, 1)
Add-Result "Свободного места не меньше 5 ГБ" ($freeGb -ge 5) ("{0} ГБ" -f $freeGb)
Add-Result "Машина чистая: приложение не установлено" (-not (Test-Path -LiteralPath $installRoot)) $installRoot

Write-Output "== 2. Тихая установка =="
$installerPath = (Resolve-Path -LiteralPath $Installer).Path
if (-not (Install-Package $installerPath "Установка /S")) {
  throw "установка не завершилась; дальнейшие проверки бессмысленны"
}

$missing = @()
foreach ($file in $appFiles) {
  if (-not (Test-Path -LiteralPath (Join-Path $installRoot $file))) { $missing += $file }
}
$missingDetail = "все файлы на месте: " + ($appFiles -join ", ")
if ($missing.Count -gt 0) { $missingDetail = "нет: " + ($missing -join ", ") }
Add-Result "Состав установки" ($missing.Count -eq 0) $missingDetail
Add-Result "Установка per-user" (Test-Path -LiteralPath (Join-Path $installRoot "wigigadict-desktop.exe")) $installRoot
Add-Result "Ничего не легло в Program Files" (-not (Test-Path -LiteralPath (Join-Path $env:ProgramFiles "WiGigaDict"))) (Join-Path $env:ProgramFiles "WiGigaDict")

Write-Output "== 3. Первый запуск =="
$null = Start-AppSoak "Первый запуск, soak 30 с" 30

Write-Output "== 4. CPU-распознавание =="
$model = Resolve-ModelPath $ModelPath
if (-not $model) {
  Add-Manual "CPU-распознавание воркером" "пропущено: модель не найдена; укажите -ModelPath или сначала установите модель в приложении"
}
else {
  $sample = $null
  $expected = $null
  if ($SpeechWav) {
    $sample = [pscustomobject]@{ Path = (Resolve-Path -LiteralPath $SpeechWav).Path; Phrase = $null; Language = "ru"; Voice = "файл владельца" }
  }
  else {
    $sample = New-SpeechSample (Join-Path $env:TEMP ("wigigadict-speech-{0}.wav" -f (Get-Random)))
    if ($sample) { $expected = $sample.Phrase }
  }
  if (-not $sample) {
    Add-Manual "CPU-распознавание воркером" "пропущено: в системе нет ни одного голоса SAPI"
  }
  else {
    $worker = Join-Path $installRoot "wigigadict-asr-worker.exe"
    $out = Join-Path $env:TEMP ("wigigadict-cpu-run-{0}.json" -f (Get-Random))
    $started = Get-Date
    & $worker run-whisper --model $model --audio $sample.Path --sample clean-install `
      --language $sample.Language --profile cpu-t16 --mode cold --threads 16 --output $out
    $elapsed = [int]((Get-Date) - $started).TotalSeconds
    $ok = $false
    $detail = "воркер не создал отчёт"
    if (Test-Path -LiteralPath $out) {
      # Отчёт воркера — UTF-8 без BOM; Get-Content в PS 5.1 прочитал бы его как ANSI.
      $record = [System.IO.File]::ReadAllText($out, [System.Text.Encoding]::UTF8) | ConvertFrom-Json
      $text = [string]$record.text
      $ok = -not [string]::IsNullOrWhiteSpace($text)
      # В отчёт идут только метрики: содержимое распознанного текста не публикуется.
      $detail = "модель: {0}, голос: {1}, символов: {2}, inference: {3} мс, всего: {4} с" -f (Split-Path -Leaf $model), $sample.Voice, $text.Length, $record.inference_ms, $elapsed
      if ($expected) {
        $recall = Get-WordRecall $expected $text
        $ok = $ok -and ($recall -ge 0.6)
        $detail = $detail + ("; слов фразы узнано: {0}" -f $recall)
      }
      Remove-Item -LiteralPath $out -Force -ErrorAction SilentlyContinue
    }
    Add-Result "CPU-распознавание воркером (cpu-t16)" $ok $detail
    if (-not $SpeechWav) { Remove-Item -LiteralPath $sample.Path -Force -ErrorAction SilentlyContinue }
  }
}

Add-Manual "Модель установлена" "экран «Модели»: загрузка из каталога либо импорт из папки; отметьте, что использовали"
Add-Manual "Первая микрофонная диктовка" "hotkey, речь, отпустить: overlay Recording / Processing / Delivered, текст вставлен"
Add-Manual "Вставка в целевое приложение" "проверить в Блокноте и в VS Code/Codex"

if (-not $SkipUninstallCycle) {
  Write-Output "== 5. Деинсталляция и сохранность данных =="
  $before = Get-DataInventory
  Uninstall-Package "Деинсталляция /S удалила бинарники"
  $lost = Get-LostDirectories $before (Get-DataInventory)
  $lostDetail = "сохранены: " + (Format-Inventory $before)
  if ($lost.Count -gt 0) { $lostDetail = "потеряны: " + ($lost -join ", ") }
  Add-Result "Данные пережили деинсталляцию" ($lost.Count -eq 0) $lostDetail

  Write-Output "== 6. Повторная установка (repair) =="
  if (Install-Package $installerPath "Повторная установка поверх данных") {
    $lostAfterRepair = Get-LostDirectories $before (Get-DataInventory)
    $repairDetail = "данные на месте"
    if ($lostAfterRepair.Count -gt 0) { $repairDetail = "потеряны: " + ($lostAfterRepair -join ", ") }
    Add-Result "Repair сохранил данные" ($lostAfterRepair.Count -eq 0) $repairDetail
    $null = Start-AppSoak "Запуск после repair, soak 30 с" 30
  }

  if ($PreviousInstaller) {
    Write-Output "== 7. Rollback на предыдущую версию =="
    Uninstall-Package "Деинсталляция перед rollback"
    if (Install-Package (Resolve-Path -LiteralPath $PreviousInstaller).Path "Установка предыдущей версии") {
      $lostAfterRollback = Get-LostDirectories $before (Get-DataInventory)
      $rollbackDetail = "данные на месте"
      if ($lostAfterRollback.Count -gt 0) { $rollbackDetail = "потеряны: " + ($lostAfterRollback -join ", ") }
      Add-Result "Rollback сохранил данные" ($lostAfterRollback.Count -eq 0) $rollbackDetail
      $null = Start-AppSoak "Запуск предыдущей версии, soak 30 с" 30
    }
  }
  else {
    Add-Manual "Rollback на предыдущую версию" "пропущено: не задан -PreviousInstaller"
  }
}

$failed = @($script:results | Where-Object { $_.Verdict -eq "FAIL" }).Count
$manual = @($script:results | Where-Object { $_.Verdict -eq "MANUAL" }).Count
$lines = @()
$lines += "# Clean install VM report"
$lines += ""
$lines += "Дата: {0:yyyy-MM-dd HH:mm}" -f (Get-Date)
$lines += "ОС: {0}, build {1}" -f $os.Caption, $os.BuildNumber
$lines += "Установщик: {0}" -f (Split-Path -Leaf $installerPath)
$lines += "Итог: FAIL {0}, MANUAL {1}, всего проверок {2}" -f $failed, $manual, $script:results.Count
$lines += ""
$lines += "| Проверка | Итог | Детали |"
$lines += "|---|---|---|"
foreach ($result in $script:results) {
  $lines += "| {0} | {1} | {2} |" -f $result.Check, $result.Verdict, $result.Detail
}
$lines += ""
$lines += "Строки MANUAL заполняются владельцем вручную."
Set-Content -LiteralPath $Report -Value $lines -Encoding utf8

Write-Output ""
Write-Output ("Отчёт: {0}" -f $Report)
Write-Output ("FAIL: {0}, MANUAL: {1}" -f $failed, $manual)
if ($failed -gt 0) { exit 1 }
