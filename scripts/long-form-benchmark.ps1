<#
.SYNOPSIS
  Long-form ASR benchmark и soak для будущего Notetaker (R2).

.DESCRIPTION
  Research harness, не production-модуль. Проверяет канонический контракт сегментов:
  окно/перекрытие, seam policy, монотонность интервалов, дрейф таймстемпов, checkpoint/resume
  после падения, вытеснение диктовкой и round-trip экспорта.

  Фикстура собирается конкатенацией синтезированных SAPI фраз, поэтому точное время каждого
  маркера известно по построению: эталонный декодер и ручная разметка не нужны.

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\long-form-benchmark.ps1 `
    -Minutes 1 -WindowSeconds 30 -OverlapSeconds 2
#>
param(
  [double]$Minutes = 1,
  [int]$WindowSeconds = 30,
  [int]$OverlapSeconds = 2,
  [string]$ModelPath,
  [ValidateSet("vulkan", "cpu-t16")][string]$Profile = "vulkan",
  [string]$WorkDir,
  [int]$CrashAfterChunk = 0,
  [int]$PreemptAfterChunk = 0,
  [switch]$Resume,
  [switch]$SelfTest
)

$ErrorActionPreference = "Stop"
$installRoot = Join-Path $env:LOCALAPPDATA "WiGigaDict"
$sampleRate = 16000
$bytesPerSample = 2

# Реплики фикстуры: маркер — единственная фраза с номером, всё остальное наполнение.
# Номер даёт и потерю (маркера нет), и дубль (маркер встретился дважды).
# Наполнитель должен быть разнообразным: на почти одинаковом повторяющемся звуке декодер
# whisper уходит в цикл и выдаёт сегменты за пределами чанка. Пяти фраз по кругу хватило,
# чтобы поймать это на 23-м чанке четырёхчасового прогона.
$fillerPhrases = @(
  "Сегодня обсуждаем план работ на ближайшую неделю.",
  "Нужно свести отчёт по расходам и проверить цифры до пятницы.",
  "Переходим к следующему пункту повестки нашей встречи.",
  "Уточните, кто отвечает за подготовку материалов к защите.",
  "Договорились о сроках, фиксируем решение и идём дальше.",
  "Клиент просил пересчитать смету с учётом новых требований.",
  "Разработчики закончили правки, тестирование начнётся завтра утром.",
  "На складе осталось мало комплектующих, нужен новый заказ.",
  "Отчёт за квартал показывает рост в двух направлениях.",
  "Предлагаю перенести обсуждение бюджета на следующий понедельник.",
  "Юристы прислали замечания к договору, их надо разобрать.",
  "Поставщик подтвердил отгрузку, документы придут по почте.",
  "Совещание получилось долгим, но решения наконец приняты.",
  "Аналитика показывает, что пользователи чаще заходят вечером.",
  "Сервер обновили ночью, утром проверим работу сервисов.",
  "Команда поддержки жалуется на нехватку инструкций для новичков.",
  "Согласуйте макеты с дизайнером до конца рабочего дня.",
  "Наш склад в области переезжает, адрес поменяется весной.",
  "Бухгалтерия просит закрыть авансовые отчёты за прошлый месяц.",
  "Обучение сотрудников назначили на третью неделю октября.",
  "Метрики загрузки улучшились после переноса кеша ближе к клиенту.",
  "Партнёры готовы обсудить условия, встреча состоится в четверг.",
  "Проверьте, пожалуйста, резервные копии базы за последние сутки.",
  "Планируем закупку оборудования, бюджет уже утверждён руководством."
)
# Номер маркера кодируется тремя существительными, а не числом: числительное whisper пишет
# то словом, то цифрой, и сверка превращается в угадайку. Троек хватает на 8000 маркеров.
$markerWords = @(
  "сокол", "ветер", "яблоко", "камень", "море", "поезд", "лампа", "книга", "дорога", "солнце",
  "рыба", "гора", "окно", "песок", "звезда", "город", "ключ", "дерево", "мост", "снег"
)

function Get-MarkerPhrase([int]$index) {
  $base = $markerWords.Count
  $zero = $index - 1
  $first = $markerWords[$zero % $base]
  $second = $markerWords[[math]::Floor($zero / $base) % $base]
  $third = $markerWords[[math]::Floor($zero / ($base * $base)) % $base]
  return "маркер $first $second $third"
}

function Convert-ToWords([string]$text) {
  $cleaned = ($text.ToLowerInvariant() -replace "[^\p{L}\p{Nd}]+", " ").Trim()
  if ($cleaned.Length -eq 0) { return @() }
  return @($cleaned -split "\s+")
}

# Ищет фразу как последовательность слов; возвращает индексы всех вхождений.
function Find-PhraseOccurrences([string[]]$haystack, [string[]]$needle) {
  $hits = @()
  if ($needle.Count -eq 0 -or $haystack.Count -lt $needle.Count) { return $hits }
  for ($i = 0; $i -le $haystack.Count - $needle.Count; $i++) {
    $match = $true
    for ($j = 0; $j -lt $needle.Count; $j++) {
      if ($haystack[$i + $j] -ne $needle[$j]) { $match = $false; break }
    }
    if ($match) { $hits += $i }
  }
  return $hits
}

# Seam policy v2. Каждый чанк отдаёт только сегменты, начавшиеся в его собственном шаге
# [startMs, startMs + stepMs); перекрытие нужно движку для контекста, а не для выдачи.
# Политика «перекрытие за более ранним чанком» ошибочна: у раннего чанка последняя фраза
# обрезана краем окна, и вместе с ней терялся весь хвост, отданный следующему чанку.
function Merge-Segments($chunks, [int]$stepMs) {
  $merged = @()
  $lastKept = $null
  $ordered = @($chunks | Sort-Object chunkIndex)
  foreach ($chunk in $ordered) {
    $ownedFrom = $chunk.startMs
    $ownedTo = $chunk.startMs + $stepMs
    if ($chunk.chunkIndex -eq $ordered[-1].chunkIndex) { $ownedTo = [int]::MaxValue }
    foreach ($segment in $chunk.segments) {
      $absoluteStart = $chunk.startMs + $segment.start_ms
      $absoluteEnd = $chunk.startMs + $segment.end_ms
      if ($absoluteStart -lt $ownedFrom -or $absoluteStart -ge $ownedTo) { continue }
      if ($absoluteEnd -le $absoluteStart) { continue }
      # Отбора по началу мало: сегмент раннего чанка может пересечь границу владения, а следующий
      # чанк начинает свой первый сегмент ровно с неё и повторяет ту же речь. Внутри чанка сегменты
      # не пересекаются, поэтому пересечение с уже принятым бывает только на шве. Признак дубля —
      # не доля перекрытия (её порог пришлось бы подгонять), а повтор слов на пересечении времени.
      if ($lastKept -and ([math]::Min($absoluteEnd, $lastKept.EndMs) -gt [math]::Max($absoluteStart, $lastKept.StartMs))) {
        $candidateWords = Convert-ToWords ([string]$segment.text)
        if ($candidateWords.Count -ge 3) {
          $head = $candidateWords[0..2] -join " "
          if ((Convert-ToWords $lastKept.Text) -join " " -like "*$head*") { continue }
        }
      }
      $kept = [pscustomobject]@{
        StartMs = [int]$absoluteStart
        EndMs   = [int]$absoluteEnd
        Text    = [string]$segment.text
        Chunk   = $chunk.chunkIndex
      }
      $merged += $kept
      $lastKept = $kept
    }
  }
  return @($merged | Sort-Object StartMs, EndMs)
}

function Format-Timestamp([int]$ms, [string]$separator) {
  $span = [TimeSpan]::FromMilliseconds($ms)
  return "{0:00}:{1:00}:{2:00}$separator{3:000}" -f [math]::Floor($span.TotalHours), $span.Minutes, $span.Seconds, $span.Milliseconds
}

function Export-Segments($segments, [string]$path, [string]$format) {
  $lines = @()
  $ordinal = 1
  foreach ($segment in $segments) {
    $text = $segment.Text.Trim()
    if ($format -eq "txt") {
      $lines += "[{0} – {1}] {2}" -f (Format-Timestamp $segment.StartMs "."), (Format-Timestamp $segment.EndMs "."), $text
    }
    elseif ($format -eq "srt") {
      $lines += "$ordinal"
      $lines += "{0} --> {1}" -f (Format-Timestamp $segment.StartMs ","), (Format-Timestamp $segment.EndMs ",")
      $lines += $text
      $lines += ""
    }
    else {
      if ($ordinal -eq 1) { $lines += "WEBVTT"; $lines += "" }
      $lines += "{0} --> {1}" -f (Format-Timestamp $segment.StartMs "."), (Format-Timestamp $segment.EndMs ".")
      $lines += $text
      $lines += ""
    }
    $ordinal++
  }
  Set-Content -LiteralPath $path -Value $lines -Encoding utf8
  return $lines
}

if ($SelfTest) {
  $chunks = @(
    [pscustomobject]@{ chunkIndex = 0; startMs = 0; segments = @(
        [pscustomobject]@{ start_ms = 0; end_ms = 1000; text = "маркер один" },
        [pscustomobject]@{ start_ms = 1000; end_ms = 2000; text = "хвост перекрытия" }) },
    [pscustomobject]@{ chunkIndex = 1; startMs = 1000; segments = @(
        [pscustomobject]@{ start_ms = 0; end_ms = 1000; text = "хвост перекрытия" },
        [pscustomobject]@{ start_ms = 1000; end_ms = 2000; text = "маркер два" }) }
  )
  $merged = Merge-Segments $chunks 1000  # шаг = окно 2000 минус перекрытие 1000
  if ($merged.Count -ne 3) { throw "self-test: seam policy оставила $($merged.Count) сегментов вместо 3" }
  if ($merged[2].Text -ne "маркер два") { throw "self-test: потерян сегмент после перекрытия" }
  if (@($merged | Where-Object { $_.Text -eq "хвост перекрытия" }).Count -ne 1) { throw "self-test: дубль на шве не снят" }
  for ($i = 1; $i -lt $merged.Count; $i++) {
    if ($merged[$i].StartMs -lt $merged[$i - 1].StartMs) { throw "self-test: интервалы не монотонны" }
  }
  # Регрессия: у раннего чанка последняя фраза обрезана краем окна, а у следующего она целая
  # и за ней идёт остальной хвост. Политика обязана взять версию следующего чанка.
  $seam = @(
    [pscustomobject]@{ chunkIndex = 0; startMs = 0; segments = @(
        [pscustomobject]@{ start_ms = 0; end_ms = 1000; text = "начало" },
        [pscustomobject]@{ start_ms = 1200; end_ms = 2000; text = "уточните что" }) },
    [pscustomobject]@{ chunkIndex = 1; startMs = 1000; segments = @(
        [pscustomobject]@{ start_ms = 200; end_ms = 1500; text = "уточните кто отвечает за материалы" },
        [pscustomobject]@{ start_ms = 1500; end_ms = 2500; text = "хвост после шва" }) }
  )
  $seamMerged = Merge-Segments $seam 1000
  $seamTexts = @($seamMerged | ForEach-Object { $_.Text })
  if ($seamTexts -contains "уточните что") { throw "self-test: обрезанный хвост раннего чанка не отброшен" }
  if ($seamTexts -notcontains "уточните кто отвечает за материалы") { throw "self-test: целая фраза следующего чанка потеряна" }
  if ($seamTexts -notcontains "хвост после шва") { throw "self-test: потерян хвост после шва" }

  $crossing = @(
    [pscustomobject]@{ chunkIndex = 0; startMs = 0; segments = @(
        [pscustomobject]@{ start_ms = 0; end_ms = 900; text = "до шва" },
        [pscustomobject]@{ start_ms = 900; end_ms = 1600; text = "маркер сокол ветер яблоко" }) },
    [pscustomobject]@{ chunkIndex = 1; startMs = 1000; segments = @(
        [pscustomobject]@{ start_ms = 0; end_ms = 600; text = "маркер сокол ветер яблоко" },
        [pscustomobject]@{ start_ms = 600; end_ms = 1600; text = "после шва" }) }
  )
  $crossingTexts = @((Merge-Segments $crossing 1000) | ForEach-Object { $_.Text })
  if (@($crossingTexts | Where-Object { $_ -eq "маркер сокол ветер яблоко" }).Count -ne 1) {
    throw "self-test: маркер на границе владения продублирован"
  }
  if ($crossingTexts -notcontains "после шва") { throw "self-test: потерян сегмент после дубля" }

  $words = Convert-ToWords "Маркер, один! И ещё маркер один."
  if (@(Find-PhraseOccurrences $words (Convert-ToWords "маркер один")).Count -ne 2) { throw "self-test: поиск фразы не нашёл оба вхождения" }
  if (@(Find-PhraseOccurrences $words (Convert-ToWords "маркер три")).Count -ne 0) { throw "self-test: поиск фразы выдумал вхождение" }
  if ((Format-Timestamp 3661001 ",") -ne "01:01:01,001") { throw "self-test: неверный таймстемп" }
  $emptyHits = @{}
  if (@($emptyHits[42] | Where-Object { $_ }).Count -ne 0) { throw "self-test: пустое вхождение считается найденным" }
  Write-Output "self-test ok"
  return
}

if (-not $WorkDir) { $WorkDir = Join-Path "C:\wigigadict-longform" ("run-{0:yyyyMMdd-HHmmss}" -f (Get-Date)) }
if (-not $ModelPath) {
  $installed = Join-Path $installRoot "installed"
  $candidate = $null
  if ($Profile -eq "vulkan") {
    $candidate = @(Get-ChildItem -LiteralPath $installed -Recurse -Filter "*turbo*.bin" -ErrorAction SilentlyContinue)[0]
  }
  if (-not $candidate) {
    $candidate = @(Get-ChildItem -LiteralPath $installed -Recurse -Filter *.bin -ErrorAction SilentlyContinue | Sort-Object Length)[0]
  }
  if (-not $candidate) { throw "модель не найдена: укажите -ModelPath" }
  $ModelPath = $candidate.FullName
}
$worker = Join-Path $installRoot "wigigadict-asr-worker.exe"
if (-not (Test-Path -LiteralPath $worker)) { throw "воркер не найден: $worker" }

$fixtureDir = Join-Path $WorkDir "fixture"
$chunkDir = Join-Path $WorkDir "chunks"
$outputDir = Join-Path $WorkDir "out"
foreach ($dir in @($WorkDir, $fixtureDir, $chunkDir, $outputDir)) {
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
}
$fixtureWav = Join-Path $fixtureDir "fixture.wav"
$planPath = Join-Path $fixtureDir "plan.json"

if ((-not $Resume) -or (-not (Test-Path -LiteralPath $planPath))) {
  Write-Output "[longform] синтез фикстуры на $Minutes мин"
  Add-Type -AssemblyName System.Speech
  $synthesizer = New-Object System.Speech.Synthesis.SpeechSynthesizer
  $voice = @($synthesizer.GetInstalledVoices() | Where-Object { $_.Enabled -and $_.VoiceInfo.Culture.Name -eq "ru-RU" })[0]
  if (-not $voice) { throw "нужен русский голос SAPI" }
  $synthesizer.SelectVoice($voice.VoiceInfo.Name)
  $format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo($sampleRate, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)

  $targetBytes = [int64]($Minutes * 60 * $sampleRate * $bytesPerSample)
  $pcmPath = Join-Path $fixtureDir "fixture.pcm"
  $pcm = [System.IO.File]::Create($pcmPath)
  $markers = @()
  $markerIndex = 1
  $fillerIndex = 0
  try {
    while ($pcm.Length -lt $targetBytes) {
      $isMarker = ($markerIndex -le 1) -or ($pcm.Length -ge ($markers[-1].startMs / 1000.0 * $sampleRate * $bytesPerSample + 8 * $sampleRate * $bytesPerSample))
      if ($isMarker) {
        $phrase = Get-MarkerPhrase $markerIndex
      }
      else {
        $phrase = $fillerPhrases[$fillerIndex % $fillerPhrases.Count]
        $fillerIndex++
      }
      $utterance = Join-Path $fixtureDir "utterance.wav"
      $synthesizer.SetOutputToWaveFile($utterance, $format)
      $synthesizer.Speak($phrase)
      $synthesizer.SetOutputToNull()

      $bytes = [System.IO.File]::ReadAllBytes($utterance)
      $offsetMs = [int]([math]::Round($pcm.Length / ($sampleRate * $bytesPerSample) * 1000))
      # SAPI пишет канонический 44-байтовый заголовок RIFF; данные идут следом.
      $pcm.Write($bytes, 44, $bytes.Length - 44)
      if ($isMarker) {
        $markers += [pscustomobject]@{ index = $markerIndex; phrase = $phrase; startMs = $offsetMs }
        $markerIndex++
      }
      # Пауза между репликами разной длины: швы должны попадать и в речь, и в тишину,
      # а одинаковый ритм — ещё один повод для декодера зациклиться.
      $pauseBytes = [int]($sampleRate * $bytesPerSample * (0.3 + 0.1 * ($fillerIndex % 5)))
      $pcm.Write((New-Object byte[] $pauseBytes), 0, $pauseBytes)
      Remove-Item -LiteralPath $utterance -Force -ErrorAction SilentlyContinue
    }
  }
  finally {
    $pcm.Close()
    $synthesizer.Dispose()
  }

  $dataBytes = (Get-Item $pcmPath).Length
  $header = New-Object byte[] 44
  $writer = New-Object System.IO.BinaryWriter (New-Object System.IO.MemoryStream($header, $true))
  $writer.Write([System.Text.Encoding]::ASCII.GetBytes("RIFF"))
  $writer.Write([int]($dataBytes + 36))
  $writer.Write([System.Text.Encoding]::ASCII.GetBytes("WAVEfmt "))
  $writer.Write([int]16)
  $writer.Write([int16]1)
  $writer.Write([int16]1)
  $writer.Write([int]$sampleRate)
  $writer.Write([int]($sampleRate * $bytesPerSample))
  $writer.Write([int16]$bytesPerSample)
  $writer.Write([int16]16)
  $writer.Write([System.Text.Encoding]::ASCII.GetBytes("data"))
  $writer.Write([int]$dataBytes)
  $writer.Close()

  $target = [System.IO.File]::Create($fixtureWav)
  $target.Write($header, 0, 44)
  $source = [System.IO.File]::OpenRead($pcmPath)
  $source.CopyTo($target)
  $source.Close()
  $target.Close()
  Remove-Item -LiteralPath $pcmPath -Force

  $durationMs = [int]([math]::Round($dataBytes / ($sampleRate * $bytesPerSample) * 1000))
  $plan = [pscustomobject]@{
    durationMs = $durationMs
    sampleRate = $sampleRate
    markers    = $markers
    model      = $ModelPath
    profile    = $Profile
    windowMs   = $WindowSeconds * 1000
    overlapMs  = $OverlapSeconds * 1000
  }
  $plan | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $planPath -Encoding utf8
  Write-Output ("[longform] фикстура {0:N1} с, маркеров {1}" -f ($durationMs / 1000), $markers.Count)
}

$plan = Get-Content -LiteralPath $planPath -Raw | ConvertFrom-Json
$windowMs = $plan.windowMs
$overlapMs = $plan.overlapMs
$stepMs = $windowMs - $overlapMs
if ($stepMs -le 0) { throw "перекрытие не может быть больше окна" }

$fixtureBytes = [System.IO.File]::ReadAllBytes($fixtureWav)
$dataBytes = $fixtureBytes.Length - 44
$totalMs = $plan.durationMs
$chunkCount = [int][math]::Ceiling(($totalMs - $overlapMs) / $stepMs)

Write-Output "[longform] чанков $chunkCount, окно $windowMs мс, перекрытие $overlapMs мс, профиль $Profile"
# Воркер знает профили как gpu и cpu-tN; наружу оставлено понятное «vulkan».
$threads = "0"
$workerProfile = "gpu"
if ($Profile -eq "cpu-t16") { $threads = "16"; $workerProfile = "cpu-t16" }

$chunks = @()
$preemptions = 0
$rejected = 0
$computed = 0
for ($index = 0; $index -lt $chunkCount; $index++) {
  $startMs = $index * $stepMs
  $endMs = [math]::Min($startMs + $windowMs, $totalMs)
  $resultPath = Join-Path $chunkDir ("chunk-{0:0000}.json" -f $index)

  if (Test-Path -LiteralPath $resultPath) {
    # Checkpoint: результат чанка уже зафиксирован, повторно не считаем и не дублируем.
    $existing = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
    $chunks += $existing
    continue
  }

  if ($CrashAfterChunk -gt 0 -and $index -eq $CrashAfterChunk) {
    Write-Output "[longform] имитация падения после чанка $($index - 1); повторный запуск с -Resume продолжит отсюда"
    exit 3
  }

  $memoryMb = 0
  $chunkWav = Join-Path $chunkDir ("chunk-{0:0000}.wav" -f $index)
  $workerOut = Join-Path $chunkDir ("worker-{0:0000}.json" -f $index)
  $started = Get-Date
  # Инвариант допускает один автоматический retry чанка. Повтор идёт укороченным окном: тот же
  # вход дал бы тот же результат, а урезанный хвост меняет контекст декодера. Окно всё равно
  # накрывает всю зону ответственности чанка [startMs, startMs + stepMs), поэтому речь не теряется.
  $attempt = 0
  $record = $null
  while ($attempt -lt 2 -and -not $record) {
    $attemptEndMs = $endMs
    if ($attempt -eq 1) { $attemptEndMs = [math]::Min($startMs + $stepMs + 500, $totalMs) }

    $startByte = [int]([math]::Round($startMs / 1000.0 * $sampleRate)) * $bytesPerSample
    $lengthBytes = [int]([math]::Round(($attemptEndMs - $startMs) / 1000.0 * $sampleRate)) * $bytesPerSample
    if ($startByte + $lengthBytes -gt $dataBytes) { $lengthBytes = $dataBytes - $startByte }

    $chunkHeader = New-Object byte[] 44
    [Array]::Copy($fixtureBytes, 0, $chunkHeader, 0, 44)
    [Array]::Copy([BitConverter]::GetBytes([int]($lengthBytes + 36)), 0, $chunkHeader, 4, 4)
    [Array]::Copy([BitConverter]::GetBytes([int]$lengthBytes), 0, $chunkHeader, 40, 4)
    $stream = [System.IO.File]::Create($chunkWav)
    $stream.Write($chunkHeader, 0, 44)
    $stream.Write($fixtureBytes, 44 + $startByte, $lengthBytes)
    $stream.Close()
  # Пик памяти снимается по живому процессу: после выхода воркера мерить уже нечего.
    Remove-Item -LiteralPath $workerOut -Force -ErrorAction SilentlyContinue
    $arguments = @("run-whisper", "--model", $plan.model, "--audio", $chunkWav,
      "--sample", ("chunk-{0:0000}" -f $index), "--language", "ru", "--profile", $workerProfile,
      "--mode", "cold", "--threads", $threads, "--output", $workerOut)
    $process = Start-Process -FilePath $worker -ArgumentList $arguments -PassThru -NoNewWindow
    # Обращение к Handle до выхода процесса удерживает дескриптор: иначе ExitCode приходит пустым.
    $null = $process.Handle
    while (-not $process.HasExited) {
      try {
        $process.Refresh()
        $current = [int]($process.PeakWorkingSet64 / 1MB)
        if ($current -gt $memoryMb) { $memoryMb = $current }
      }
      catch { }
      Start-Sleep -Milliseconds 50
    }
    # Без WaitForExit объект от Start-Process -PassThru не отдаёт ExitCode.
    $process.WaitForExit()
    if ($process.ExitCode -eq 0 -and (Test-Path -LiteralPath $workerOut)) {
      $record = [System.IO.File]::ReadAllText($workerOut, [System.Text.Encoding]::UTF8) | ConvertFrom-Json
    }
    else {
      $attempt++
      $rejected++
      Write-Output ("[longform] воркер отклонил чанк {0}, код {1}; попытка {2}" -f $index, $process.ExitCode, ($attempt + 1))
      if ($attempt -ge 2) { throw "чанк $index не принят после повтора: job переходит в recoverable failed" }
    }
  }

  $chunk = [pscustomobject]@{
    chunkIndex  = $index
    startMs     = $startMs
    endMs       = $endMs
    segments    = $record.segments
    text        = $record.text
    inferenceMs = $record.inference_ms
    elapsedMs   = [int]((Get-Date) - $started).TotalMilliseconds
    peakMb      = $memoryMb
  }
  # Чанк объявляется зафиксированным только после того, как файл целиком лёг на диск.
  $temporary = "$resultPath.part"
  $chunk | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $temporary -Encoding utf8
  Move-Item -LiteralPath $temporary -Destination $resultPath -Force
  $chunks += $chunk
  $computed++
  Remove-Item -LiteralPath $chunkWav -Force -ErrorAction SilentlyContinue

  if ($PreemptAfterChunk -gt 0 -and (($index + 1) % $PreemptAfterChunk) -eq 0 -and $index -lt $chunkCount - 1) {
    # Вытеснение диктовкой: runtime освобождается на границе чанка, работа продолжается после.
    $preemptions++
    Write-Output "[longform] вытеснение диктовкой на границе чанка $index"
    Start-Sleep -Milliseconds 500
  }

  Write-Output ("[longform] чанк {0}/{1}: {2} мс inference, пик {3} МБ" -f ($index + 1), $chunkCount, $chunk.inferenceMs, $chunk.peakMb)
}

$merged = Merge-Segments $chunks $stepMs

$results = @()
function Add-Gate([string]$name, [bool]$ok, [string]$detail) {
  $verdict = "FAIL"
  if ($ok) { $verdict = "PASS" }
  $script:results += [pscustomobject]@{ Gate = $name; Verdict = $verdict; Detail = $detail }
  Write-Output ("[{0}] {1} — {2}" -f $verdict, $name, $detail)
}

# Маркер — уникальная последовательность «маркер» + три существительных, поэтому вместо поиска
# каждой фразы по всему транскрипту сегменты обходятся один раз, а фразы ищутся в хеш-таблице.
# На четырёхчасовом прогоне это разница между секундами и часами.
$markerByPhrase = @{}
foreach ($marker in $plan.markers) {
  $markerByPhrase[(Convert-ToWords $marker.phrase) -join " "] = $marker
}
$hitsByIndex = @{}
foreach ($segment in $merged) {
  $words = Convert-ToWords $segment.Text
  for ($w = 0; $w -lt $words.Count; $w++) {
    if ($words[$w] -ne "маркер" -or ($w + 3) -ge $words.Count) { continue }
    $phrase = $words[$w..($w + 3)] -join " "
    $marker = $markerByPhrase[$phrase]
    if (-not $marker) { continue }
    if (-not $hitsByIndex.ContainsKey($marker.index)) { $hitsByIndex[$marker.index] = @() }
    $hitsByIndex[$marker.index] += $segment
  }
}

$lost = @()
$duplicated = @()
$drifts = @()
foreach ($marker in $plan.markers) {
  # @($hash[$отсутствующий]) даёт массив из одного $null с Count = 1: без отсева потерянный
  # маркер выглядел бы найденным, а дрейф считался бы от пустого сегмента.
  $hits = @($hitsByIndex[$marker.index] | Where-Object { $_ })
  if ($hits.Count -eq 0) { $lost += $marker.index; continue }
  if ($hits.Count -gt 1) { $duplicated += $marker.index }
  $segment = @($hits | Where-Object { $marker.startMs -ge $_.StartMs -and $marker.startMs -lt $_.EndMs })[0]
  if (-not $segment) {
    $segment = @($hits | Sort-Object { [math]::Abs($_.StartMs - $marker.startMs) })[0]
  }
  $outside = 0
  if ($marker.startMs -lt $segment.StartMs) { $outside = $segment.StartMs - $marker.startMs }
  elseif ($marker.startMs -ge $segment.EndMs) { $outside = $marker.startMs - $segment.EndMs }
  $drifts += [pscustomobject]@{ index = $marker.index; driftMs = [int]$outside }
}

# Точное совпадение фразы меряет качество распознавания, а не целостность нарезки: три редких
# существительных подряд модель нередко слышит с одной ошибкой. Поэтому потеря считается нечётко —
# по наличию «маркер» и хотя бы двух из трёх слов рядом с плановым временем, — а гейтом служит не
# сам процент, а вопрос «липнут ли промахи к швам». Ошибка сшивки давала бы всплеск именно там.
$segmentIndex = 0
$missed = @()
$seamMissed = 0
$seamTotal = 0
$fuzzyFound = 0
foreach ($marker in ($plan.markers | Sort-Object index)) {
  $from = $marker.startMs - 1000
  $to = $marker.startMs + 4000
  while ($segmentIndex -gt 0 -and $merged[$segmentIndex - 1].EndMs -gt $from) { $segmentIndex-- }
  while ($segmentIndex -lt $merged.Count -and $merged[$segmentIndex].EndMs -le $from) { $segmentIndex++ }
  $nearby = @()
  $cursor = $segmentIndex
  while ($cursor -lt $merged.Count -and $merged[$cursor].StartMs -lt $to) {
    $nearby += $merged[$cursor].Text
    $cursor++
  }
  $nearbyWords = Convert-ToWords ($nearby -join " ")
  $expected = Convert-ToWords $marker.phrase
  $matchedNouns = 0
  foreach ($noun in $expected[1..3]) { if ($nearbyWords -contains $noun) { $matchedNouns++ } }
  $isFound = ($nearbyWords -contains "маркер") -and ($matchedNouns -ge 2)
  if ($isFound) { $fuzzyFound++ } else { $missed += $marker.index }

  # Шов — окрестность границы владения между соседними чанками.
  $offset = $marker.startMs % $stepMs
  if ($offset -le 2000 -or $offset -ge ($stepMs - 2000)) {
    $seamTotal++
    if (-not $isFound) { $seamMissed++ }
  }
}
$missRate = 0.0
$seamMissRate = 0.0
$otherMissRate = 0.0
if ($plan.markers.Count -gt 0) { $missRate = $missed.Count / $plan.markers.Count }
if ($seamTotal -gt 0) { $seamMissRate = $seamMissed / $seamTotal }
$otherTotal = $plan.markers.Count - $seamTotal
if ($otherTotal -gt 0) { $otherMissRate = ($missed.Count - $seamMissed) / $otherTotal }

Add-Gate "Промахи не концентрируются на швах" ($seamMissRate -le $otherMissRate + 0.02) ("на швах {0:P1} из {1}, вне швов {2:P1} из {3}" -f $seamMissRate, $seamTotal, $otherMissRate, $otherTotal)
Add-Gate "Маркеры не продублированы" ($duplicated.Count -eq 0) ("дублей {0}" -f $duplicated.Count)

$monotone = $true
for ($i = 0; $i -lt $merged.Count; $i++) {
  if ($merged[$i].EndMs -le $merged[$i].StartMs) { $monotone = $false; break }
  if ($i -gt 0 -and $merged[$i].StartMs -lt $merged[$i - 1].StartMs) { $monotone = $false; break }
}
Add-Gate "Интервалы монотонны и полуоткрыты" $monotone ("сегментов {0}" -f $merged.Count)

$maxDrift = 0
$finalDrift = 0
if ($drifts.Count -gt 0) {
  $maxDrift = ($drifts | ForEach-Object { [math]::Abs($_.driftMs) } | Measure-Object -Maximum).Maximum
  $finalDrift = [math]::Abs(($drifts | Sort-Object index)[-1].driftMs)
}
Add-Gate "Дрейф финального маркера не больше 1 с" ($finalDrift -le 1000) ("время маркера вне несущего сегмента: финальный {0} мс, максимальный {1} мс" -f $finalDrift, $maxDrift)

$firstMarkerFound = ($missed -notcontains $plan.markers[0].index)
$lastMarkerFound = ($missed -notcontains $plan.markers[-1].index)
Add-Gate "Нет обрезки в начале и в конце" ($firstMarkerFound -and $lastMarkerFound) ("первый {0}, последний {1}" -f $firstMarkerFound, $lastMarkerFound)

$committed = @(Get-ChildItem -LiteralPath $chunkDir -Filter "chunk-*.json").Count
$partials = @(Get-ChildItem -LiteralPath $chunkDir -Filter "*.part" -ErrorAction SilentlyContinue).Count
Add-Gate "Чанки зафиксированы без потерь и дублей" (($committed -eq $chunkCount) -and ($partials -eq 0)) ("зафиксировано {0} из {1}, незавершённых {2}" -f $committed, $chunkCount, $partials)

# Отказ движка сам по себе не провал: инвариант разрешает один повтор. Провалом был бы
# чанк, не принятый и после повтора, — на нём прогон падает выше.
Add-Gate "Отказы движка восстановлены повтором" $true ("отклонённых попыток {0} из {1} чанков" -f $rejected, $chunkCount)

$peak = 0
if ($chunks.Count -gt 0) { $peak = ($chunks | Measure-Object peakMb -Maximum).Maximum }
$firstHalf = @($chunks | Where-Object { $_.chunkIndex -lt [int]($chunkCount / 2) })
$secondHalf = @($chunks | Where-Object { $_.chunkIndex -ge [int]($chunkCount / 2) })
$growth = 0
if ($firstHalf.Count -gt 0 -and $secondHalf.Count -gt 0) {
  $growth = [int](($secondHalf | Measure-Object peakMb -Average).Average - ($firstHalf | Measure-Object peakMb -Average).Average)
}
Add-Gate "Память ограничена, тренда роста нет" ($growth -le 64) ("пик {0} МБ, вторая половина прогона {1:+#;-#;0} МБ" -f $peak, $growth)

$txt = Export-Segments $merged (Join-Path $outputDir "transcript.txt") "txt"
$srt = Export-Segments $merged (Join-Path $outputDir "transcript.srt") "srt"
$vtt = Export-Segments $merged (Join-Path $outputDir "transcript.vtt") "vtt"
$srtTexts = @()
for ($i = 1; $i -lt $srt.Count; $i += 4) { $srtTexts += $srt[$i + 1] }
$roundTrip = ($txt.Count -eq $merged.Count) -and ($srtTexts.Count -eq $merged.Count)
for ($i = 0; $i -lt $merged.Count -and $roundTrip; $i++) {
  if ($srtTexts[$i] -ne $merged[$i].Text.Trim()) { $roundTrip = $false }
}
Add-Gate "Экспорт TXT/SRT/VTT не меняет текст и порядок" $roundTrip ("строк txt {0}, блоков srt {1}, строк vtt {2}" -f $txt.Count, $srtTexts.Count, $vtt.Count)

if ($PreemptAfterChunk -gt 0 -and $computed -gt 0) {
  Add-Gate "Вытеснение диктовкой на границе чанка" ($preemptions -gt 0) ("вытеснений {0} на {1} посчитанных чанках, все на границах" -f $preemptions, $computed)
}

$totalInference = ($chunks | Measure-Object inferenceMs -Sum).Sum
$rtf = 0
if ($totalMs -gt 0) { $rtf = [math]::Round($totalInference / $totalMs, 4) }
$failed = @($results | Where-Object { $_.Verdict -eq "FAIL" }).Count

$report = Join-Path $WorkDir "report.md"
$lines = @()
$lines += "# Long-form benchmark"
$lines += ""
$lines += "- Дата: {0:yyyy-MM-dd HH:mm}" -f (Get-Date)
$lines += "- Модель: {0}, профиль {1}" -f (Split-Path -Leaf $plan.model), $plan.profile
$lines += "- Длительность: {0:N1} с, маркеров {1}" -f ($totalMs / 1000), $plan.markers.Count
$lines += "- Окно {0} мс, перекрытие {1} мс, чанков {2}" -f $windowMs, $overlapMs, $chunkCount
$lines += "- Inference {0} мс, RTF {1}, пик памяти {2} МБ" -f $totalInference, $rtf, $peak
$lines += "- Сегментов после сшивки: {0}" -f $merged.Count
$lines += "- Распознавание маркеров (качество модели, не гейт): точно {0} из {1}, нечётко {2}" -f $drifts.Count, $plan.markers.Count, $fuzzyFound
$lines += "- Промахи: всего {0:P1}, на швах {1:P1}, вне швов {2:P1}" -f $missRate, $seamMissRate, $otherMissRate
$lines += "- Отказов движка, восстановленных повтором: {0}" -f $rejected
$lines += ""
$lines += "| Гейт | Итог | Детали |"
$lines += "|---|---|---|"
foreach ($result in $results) { $lines += "| {0} | {1} | {2} |" -f $result.Gate, $result.Verdict, $result.Detail }
Set-Content -LiteralPath $report -Value $lines -Encoding utf8

Write-Output ""
Write-Output ("Отчёт: {0}" -f $report)
Write-Output ("RTF {0}, пик {1} МБ, FAIL {2}" -f $rtf, $peak, $failed)
if ($failed -gt 0) { exit 1 }
