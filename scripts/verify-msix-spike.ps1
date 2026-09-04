<#
.SYNOPSIS
  Спайк упаковки в MSIX: переживают ли контейнер хоткей, микрофон, вставка и данные (BL-049).

.DESCRIPTION
  Запускается ВНУТРИ одноразовой виртуальной машины, **от администратора**: установка тестового
  сертификата в машинное хранилище доверия иначе невозможна. На рабочей машине не запускать.

  Спайк отвечает на вопрос, можно ли раздавать приложение через Microsoft Store: только там
  предупреждение SmartScreen не показывается вовсе. Проверяется не красота упаковки, а выживание
  тех возможностей, которые в контейнере ограничены.

  PowerShell 5.1.

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File .\verify-msix-spike.ps1 `
    -Package .\WiGigaDict-spike.msix -Certificate .\spike.cer
#>
param(
  [string]$Package = ".\WiGigaDict-spike.msix",
  [string]$Certificate = ".\spike.cer",
  [string]$Report,
  [switch]$KeepInstalled,
  [switch]$SelfTest
)

$ErrorActionPreference = "Stop"
$packageName = "WiGigaDict.Spike"
$appId = "WiGigaDict"
$classicRoot = Join-Path $env:LOCALAPPDATA "WiGigaDict"
$dataDirs = @("storage", "audio", "logs", "installed", "quarantine")

$script:results = @()
function Add-Result([string]$name, [bool]$ok, [string]$detail) {
  $verdict = "FAIL"
  if ($ok) { $verdict = "PASS" }
  $script:results += [pscustomobject]@{ Check = $name; Verdict = $verdict; Detail = $detail }
  Write-Output ("[{0}] {1} — {2}" -f $verdict, $name, $detail)
}
function Add-Note([string]$name, [string]$detail) {
  $script:results += [pscustomobject]@{ Check = $name; Verdict = "ФАКТ"; Detail = $detail }
  Write-Output ("[ФАКТ] {0} — {1}" -f $name, $detail)
}
function Add-Manual([string]$name, [string]$detail) {
  $script:results += [pscustomobject]@{ Check = $name; Verdict = "MANUAL"; Detail = $detail }
  Write-Output ("[MANUAL] {0} — {1}" -f $name, $detail)
}

# Данные приложения под MSIX могут оказаться в контейнере пакета вместо обычного пути.
# Именно это и надо выяснить: от ответа зависят retention, recovery и удаление.
function Get-DataLocations([string]$familyName) {
  $found = @()
  if (Test-Path -LiteralPath $classicRoot) {
    $count = @(Get-ChildItem -LiteralPath $classicRoot -Recurse -File -ErrorAction SilentlyContinue).Count
    $found += [pscustomobject]@{ Kind = "обычный путь"; Path = $classicRoot; Files = $count }
  }
  if ($familyName) {
    $container = Join-Path $env:LOCALAPPDATA "Packages\$familyName\LocalCache\Local\WiGigaDict"
    if (Test-Path -LiteralPath $container) {
      $count = @(Get-ChildItem -LiteralPath $container -Recurse -File -ErrorAction SilentlyContinue).Count
      $found += [pscustomobject]@{ Kind = "контейнер пакета"; Path = $container; Files = $count }
    }
  }
  return $found
}

function Format-Locations($locations) {
  if ($locations.Count -eq 0) { return "не найдено ни одного" }
  return (($locations | ForEach-Object { "{0}: {1} файлов" -f $_.Kind, $_.Files }) -join "; ")
}

if ($SelfTest) {
  if ((Format-Locations @()) -ne "не найдено ни одного") { throw "self-test: пустой список" }
  $sample = @([pscustomobject]@{ Kind = "обычный путь"; Path = "x"; Files = 3 })
  if ((Format-Locations $sample) -ne "обычный путь: 3 файлов") { throw "self-test: описание пути" }
  Write-Output "self-test ok"
  return
}

if (-not $Report) {
  $Report = Join-Path (Get-Location) ("msix-spike-report-{0:yyyyMMdd-HHmmss}.md" -f (Get-Date))
}

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltinRole]::Administrator)
if (-not $isAdmin) { throw "запустите от администратора: без этого сертификат в машинное хранилище не встанет" }
if (Get-Process -Name "wigigadict-desktop" -ErrorAction SilentlyContinue) {
  throw "обычная версия приложения запущена; закройте её — single-instance mutex общий на пользователя"
}

$packagePath = (Resolve-Path -LiteralPath $Package).Path
$certificatePath = (Resolve-Path -LiteralPath $Certificate).Path

Write-Output "== 1. Доверие тестовому сертификату =="
# Сертификат одноразовый и годится только для этой виртуалки: в реальной раздаче пакет
# подписывает Microsoft при публикации в Store.
Import-Certificate -FilePath $certificatePath -CertStoreLocation "Cert:\LocalMachine\TrustedPeople" | Out-Null
Add-Note "Тестовый сертификат" "импортирован в LocalMachine\TrustedPeople; вне этой VM бессмыслен"

Write-Output "== 2. Установка пакета =="
$started = Get-Date
Add-AppxPackage -Path $packagePath
$elapsed = [int]((Get-Date) - $started).TotalSeconds
$installed = Get-AppxPackage -Name $packageName
Add-Result "MSIX ставится" ($null -ne $installed) ("{0} с, {1}" -f $elapsed, $installed.PackageFullName)
if (-not $installed) { throw "пакет не установился" }
$familyName = $installed.PackageFamilyName
Add-Note "Расположение пакета" $installed.InstallLocation

$payload = @("wigigadict-desktop.exe", "wigigadict-asr-sidecar.exe", "wigigadict-asr-worker.exe", "catalog.json", "catalog.sig")
$missing = @()
foreach ($file in $payload) {
  if (-not (Test-Path -LiteralPath (Join-Path $installed.InstallLocation $file))) { $missing += $file }
}
Add-Result "Состав пакета на месте" ($missing.Count -eq 0) $(if ($missing.Count -eq 0) { "все файлы, включая воркер и каталог" } else { "нет: " + ($missing -join ", ") })

Write-Output "== 3. Запуск из контейнера =="
$beforeLocations = Get-DataLocations $familyName
Start-Process "shell:AppsFolder\$familyName!$appId"
Start-Sleep -Seconds 5
$process = Get-Process -Name "wigigadict-desktop" -ErrorAction SilentlyContinue
$alive = $null -ne $process
if ($alive) {
  Start-Sleep -Seconds 30
  $process = Get-Process -Name "wigigadict-desktop" -ErrorAction SilentlyContinue
  $alive = $null -ne $process
}
$memoryMb = 0
if ($alive) { $memoryMb = [int](($process | Measure-Object WorkingSet64 -Maximum).Maximum / 1MB) }
Add-Result "Приложение живёт в контейнере 30 с" $alive ("рабочее множество {0} МБ" -f $memoryMb)

Write-Output "== 4. Куда легли данные =="
# Главный технический вопрос спайка: MSIX перенаправляет запись в AppData в приватный
# контейнер пакета. Если данные ушли туда, меняются retention, recovery и смысл удаления.
$afterLocations = Get-DataLocations $familyName
Add-Note "Каталоги данных" (Format-Locations $afterLocations)
$inContainer = @($afterLocations | Where-Object { $_.Kind -eq "контейнер пакета" })
$inClassic = @($afterLocations | Where-Object { $_.Kind -eq "обычный путь" })
if ($inContainer.Count -gt 0 -and $inClassic.Count -eq 0) {
  Add-Note "Вывод по данным" "запись перенаправлена в контейнер: удаление пакета унесёт историю диктовок и модели"
}
elseif ($inClassic.Count -gt 0 -and $inContainer.Count -eq 0) {
  Add-Note "Вывод по данным" "запись идёт по обычному пути: поведение как у NSIS-версии"
}
else {
  Add-Note "Вывод по данным" "данные в обоих местах — разобрать вручную, что именно куда пишется"
}

Add-Manual "Глобальный хоткей" "нажать сочетание: overlay должен появиться, запись начаться"
Add-Manual "Микрофон" "первый запуск должен спросить доступ; проверить, что запись идёт"
Add-Manual "Вставка в чужое окно" "Блокнот и VS Code: текст вставляется, фокус не уводится"
Add-Manual "Оверлей поверх окон" "не забирает фокус, не появляется в панели задач"
Add-Manual "Sidecar и воркер" "распознавание проходит: дочерние процессы из пакета запускаются"

if ($alive) {
  Stop-Process -Name "wigigadict-desktop" -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 3
}

if (-not $KeepInstalled) {
  Write-Output "== 5. Удаление пакета =="
  $dataBefore = Get-DataLocations $familyName
  Remove-AppxPackage -Package $installed.PackageFullName
  Start-Sleep -Seconds 3
  $gone = $null -eq (Get-AppxPackage -Name $packageName)
  Add-Result "Пакет удаляется" $gone $installed.PackageFullName
  $dataAfter = Get-DataLocations $familyName
  $survived = @($dataAfter | Where-Object { $_.Files -gt 0 }).Count -gt 0
  Add-Note "Данные после удаления" ("было — {0}; стало — {1}" -f (Format-Locations $dataBefore), (Format-Locations $dataAfter))
  if (-not $survived -and @($dataBefore | Where-Object { $_.Files -gt 0 }).Count -gt 0) {
    Add-Note "Вывод по удалению" "удаление пакета стёрло данные пользователя: для NSIS-версии это не так, придётся решать отдельно"
  }
}

$failed = @($script:results | Where-Object { $_.Verdict -eq "FAIL" }).Count
$manual = @($script:results | Where-Object { $_.Verdict -eq "MANUAL" }).Count
$os = Get-CimInstance Win32_OperatingSystem
$lines = @()
$lines += "# MSIX spike report"
$lines += ""
$lines += "Дата: {0:yyyy-MM-dd HH:mm}" -f (Get-Date)
$lines += "ОС: {0}, build {1}" -f $os.Caption, $os.BuildNumber
$lines += "Пакет: {0}" -f (Split-Path -Leaf $packagePath)
$lines += "Итог: FAIL {0}, MANUAL {1}" -f $failed, $manual
$lines += ""
$lines += "| Проверка | Итог | Детали |"
$lines += "|---|---|---|"
foreach ($result in $script:results) { $lines += "| {0} | {1} | {2} |" -f $result.Check, $result.Verdict, $result.Detail }
$lines += ""
$lines += "Строки MANUAL заполняются вручную. Строки ФАКТ — наблюдения, а не приговор."
Set-Content -LiteralPath $Report -Value $lines -Encoding utf8

Write-Output ""
Write-Output ("Отчёт: {0}" -f $Report)
Write-Output ("FAIL: {0}, MANUAL: {1}" -f $failed, $manual)
if ($failed -gt 0) { exit 1 }
