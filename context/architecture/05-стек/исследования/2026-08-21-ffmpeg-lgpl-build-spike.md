# Research spike: воспроизводимая LGPL-сборка FFmpeg для Notetaker

- Дата проверки: 2026-08-21
- Статус: **завершено 2 сентября 2026: сборка воспроизводима побайтово, media matrix и compliance bundle зелёные**
- Область: M2 Notetaker; не production-код и не runtime MVP/M1

## Вопрос

Можно ли распространять вместе с WiGigaDict точную Windows CLI-сборку FFmpeg/ffprobe, которая покрывает проверенные media formats и сохраняет весь bundle в LGPL-профиле?

## Проверенные факты

- Базовый FFmpeg распространяется по LGPL 2.1+, но включение GPL-компонентов переводит соответствующую сборку в GPL. Официальный compliance checklist требует не использовать `--enable-gpl` и `--enable-nonfree` и отдельно проверять лицензии external libraries.
- Для Windows официальный checklist рекомендует shared build/DLL. Static linking создаёт дополнительное LGPL-бремя relinkability/object files.
- Вместе с binary distribution нужны exact corresponding source, license/notice, полный configure/build recipe и `changes.diff`, если исходники изменялись. Source должен быть доступен там же, где binary distribution.
- Проверенные официальные источники: [FFmpeg Legal](https://ffmpeg.org/legal.html), [Windows build notes](https://www.ffmpeg.org/platform.html), [FFmpeg license](https://www.ffmpeg.org/doxygen/7.0/md_LICENSE.html), [LGPL 2.1](https://www.gnu.org/licenses/old-licenses/lgpl-2.1).

## Локальное evidence

Установленные `ffmpeg` и `ffprobe` — `8.1.1-full_build-www.gyan.dev`, GCC/MSYS2 static build. `ffmpeg -buildconf` содержит `--enable-gpl`, `--enable-version3`, `libx264`, `libx265` и другие расширения. Этот binary пригоден только как локальный decode/probe oracle и **запрещён как production bundle WiGigaDict**.

## Принятое направление

- Bundled exact-pinned CLI `ffmpeg.exe` + `ffprobe.exe` и необходимые DLL собственной воспроизводимой сборки.
- Shared/DLL LGPL profile; `--enable-gpl` и `--enable-nonfree` отсутствуют; external dependencies работают по allowlist, а не через случайный autodetect/PATH.
- Release payload содержит source archive/tag+commit, SHA-256 source/binary/DLL, toolchain versions, configure line, dependency/license inventory, `changes.diff`, LGPL text, notices и build recipe.
- Поддержка форматов публикуется только по проверенной container/codec matrix конкретной сборки. Возможность FFmpeg декодировать что-то ещё не становится продуктовым обещанием.

## Build spike protocol

1. Выбрать один exact FFmpeg source tag/commit и один Windows toolchain profile; не смешивать MSVC и MinGW artifacts.
2. Запустить hermetic shared build без GPL/nonfree и без неразрешённых external libraries.
3. Повторить build из чистого окружения и сравнить manifest/hashes; если bit-for-bit пока недостижим, документировать детерминированные входы и объяснимые различия, но release artifact строить только в одном pinned environment.
4. Проверить PE dependencies, `ffmpeg -buildconf`, license inventory и отсутствие `libx264`/других GPL/nonfree частей.
5. Прогнать media matrix: WAV/PCM, MP3, FLAC, AAC/M4A и выбранные MP4/MKV/WebM/MOV combinations; проверить probe, выбор audio stream, decode в PCM S16LE 16 kHz mono, cancel и corrupt/no-audio failures.
6. Собрать compliance bundle и доказать exact source/binary correspondence.

## Результаты сборки (2 сентября 2026)

Рецепт: `scripts/build-ffmpeg-lgpl.sh`, матрица: `scripts/ffmpeg-media-matrix.sh`, bundle: `scripts/ffmpeg-compliance-bundle.sh`.

**Источник.** Tag `n8.1.2`, тарбол с ffmpeg.org, sha256 `464beb5e7bf0c311e68b45ae2f04e9cc2af88851abb4082231742a74d97b524c`. Подпись `.asc` проверена ключом `FCF986EA15E6E293A5644F10B4322F04D67658D8` («FFmpeg release signing key»); ключ взят с keyserver.ubuntu.com, то есть не с того же сайта, что и архив. Изменённых файлов апстрима — 0, `changes.diff` пуст.

**Профиль.** Shared LGPL 2.1: `--enable-shared --disable-static --disable-autodetect --disable-network --disable-doc --disable-debug --disable-ffplay --enable-w32threads`, ldflags `-static-libgcc -Wl,--no-insert-timestamp`. Ни `--enable-gpl`, ни `--enable-nonfree`, ни `--enable-version3`. Внешних библиотек нет вообще: все нужные форматы покрывают родные декодеры, поэтому allowlist пуст, а `--disable-autodetect` не даёт сборке подхватить что-либо из окружения. Скрипт после сборки сам отвергает запрещённые флаги в `-buildconf` и требует баннер LGPL 2.1.

**Тулчейн.** MSYS2 mingw-w64 gcc 16.2.0, nasm 3.02, GNU make 4.4.1.

**Поставка.** `ffmpeg.exe` 471 552 Б, `ffprobe.exe` 214 016 Б и семь DLL — **29,8 МБ**. PE-зависимости только собственные DLL плюс `KERNEL32`, `msvcrt`, `SHELL32`: ни `libgcc`, ни `libwinpthread`, ни одной внешней библиотеки. Import-библиотеки `.lib` в поставку не входят.

**Воспроизводимость — побайтовая.** Две независимые чистые сборки в разных каталогах дают одинаковые sha256 всех девяти файлов. Далось это тремя правками, каждая закрывает конкретную недетерминированную входную величину: фиксированный `--prefix` с раскладкой через `DESTDIR` (FFmpeg вшивает строку configure целиком, и путь сборки попадал в бинарь), `-Wl,--no-insert-timestamp` (штамп в таблице экспорта) и `SOURCE_DATE_EPOCH` (штамп в PE-заголовке — последние 4 расходившихся байта).

**Media matrix — 11/11.** WAV/PCM и FLAC декодируются в PCM S16LE 16 kHz mono байт-в-байт с эталоном; MP3, AAC в M4A/MOV/MP4, Opus в WebM и Vorbis в MKV сходятся по длине с отклонением не более 32 байт (padding кодеров). Файл без звуковой дорожки: probe пуст, decode отказывает. Битый MP3 отвергается с `Header missing`, а не «успехом» с пустым PCM. Убитый на длинном декодировании процесс умирает с кодом 137 и оборванным выводом. Фикстуры кодирует установленный полный build как oracle, проверяемая сборка выступает только потребителем.

**Compliance bundle.** `artifacts/ffmpeg-lgpl/bundle/` (41 МБ): бинарники, исходный тарбол с подписью, `LICENSE.md`, `COPYING.LGPLv2.1`, `CREDITS`, пустой `changes.diff`, манифест сборки со строкой configure, версиями тулчейна, суммами и PE-зависимостями, а также `SHA256SUMS.txt` и `README.md` с командой проверки подписи.

**Что осталось за рамками.** Версии пакетов MSYS2 не запинены: побайтовое совпадение доказано для gcc 16.2.0 и nasm 3.02, другой компилятор даст другие байты — при смене тулчейна проверку повторить. Набор DLL не урезан: `--disable-encoders`/`--disable-filters` заметно уменьшат 29,8 МБ, но это оптимизация размера, а не лицензии, и делать её нужно вместе с сокращением обещанной матрицы форматов. Подпись бинарников Authenticode не делалась — общий вопрос публичной раздачи, см. BL-049.

## Go/no-go

**Go получено 2 сентября 2026:** clean rebuild побайтово воспроизводим, license/dependency audit пуст по внешним библиотекам, media matrix 11/11, compliance bundle собран. FFmpeg остаётся зависимостью M2 Notetaker и в installer M1 не входит; установленный Gyan build по-прежнему используется только как oracle для фикстур и не копируется ни в repository, ни в installer.

Связанные решения: [ADR-003](../../06-решения/журнал-решений/2026-08-21-003-bundled-lgpl-ffmpeg.md) · [технологии](../технологии.md) · [Notetaker roadmap](../../08-дорожная-карта/roadmap.md#research-track-notetaker-до-m2)
