#!/usr/bin/env bash
# Media matrix для собственной LGPL-сборки FFmpeg (R1, ADR-003).
#
#   MSYSTEM=MINGW64 C:/msys64/usr/bin/bash.exe -l scripts/ffmpeg-media-matrix.sh <prefix-bin> <work-dir>
#
# Фикстуры кодирует установленный полный build (oracle), проверяемая сборка выступает только
# потребителем: probe, выбор audio stream, decode в PCM S16LE 16 kHz mono, отказы и отмена.
set -uo pipefail

BIN="${1:-/c/wigigadict-ffmpeg-build/build-1/install/bin}"
WORK="${2:-/c/wigigadict-ffmpeg-build/matrix}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_WAV="$REPO_ROOT/artifacts/ffmpeg-lgpl/fixtures/speech.wav"
# Oracle — установленный полный build; в MSYS2-оболочке его нет в PATH, поэтому путь передаётся
# третьим аргументом либо ищется там, куда его кладёт winget.
ORACLE="${3:-$(command -v ffmpeg || true)}"
if [ -z "$ORACLE" ]; then
  ORACLE="$(ls -1 "$USERPROFILE"/AppData/Local/Microsoft/WinGet/Packages/Gyan.FFmpeg*/ffmpeg*/bin/ffmpeg.exe 2>/dev/null | head -1)"
fi

FFMPEG="$BIN/ffmpeg.exe"
FFPROBE="$BIN/ffprobe.exe"
REPORT="$WORK/matrix-report.txt"

for required in "$FFMPEG" "$FFPROBE" "$SOURCE_WAV" "$ORACLE"; do
  if [ ! -e "$required" ]; then echo "missing: $required" >&2; exit 1; fi
done

rm -rf "$WORK"
mkdir -p "$WORK/fixtures" "$WORK/out"
F="$WORK/fixtures"

echo "[matrix] encoding fixtures with oracle: $ORACLE"
q="-hide_banner -loglevel error -y"
"$ORACLE" $q -i "$SOURCE_WAV" -c:a copy "$F/pcm.wav"
"$ORACLE" $q -i "$SOURCE_WAV" -c:a libmp3lame -b:a 128k "$F/audio.mp3"
"$ORACLE" $q -i "$SOURCE_WAV" -c:a flac "$F/audio.flac"
"$ORACLE" $q -i "$SOURCE_WAV" -c:a aac -b:a 128k "$F/audio.m4a"
"$ORACLE" $q -i "$SOURCE_WAV" -c:a aac -b:a 128k "$F/audio.mov"
"$ORACLE" $q -i "$SOURCE_WAV" -c:a libopus -b:a 64k "$F/audio.webm"
"$ORACLE" $q -i "$SOURCE_WAV" -c:a libvorbis -q:a 4 "$F/audio.mkv"
# Видео должно быть не короче речи, иначе -shortest обрежет звук и сравнение длины PCM
# будет мерить фикстуру, а не декодер.
source_seconds="$("${ORACLE%ffmpeg.exe}ffprobe.exe" -v error -show_entries format=duration -of csv=p=0 "$SOURCE_WAV")"
video_seconds="$(awk -v d="$source_seconds" 'BEGIN { printf "%d", d + 2 }')"
"$ORACLE" $q -f lavfi -i "testsrc=size=160x120:rate=10:duration=$video_seconds" -i "$SOURCE_WAV" \
  -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest "$F/video.mp4"
"$ORACLE" $q -f lavfi -i testsrc=size=160x120:rate=10:duration=1 -c:v libx264 -pix_fmt yuv420p "$F/no-audio.mp4"
# Битый файл: середина mp3 забита нулями, заголовок цел — отказ должен быть на декодировании.
cp "$F/audio.mp3" "$F/corrupt.mp3"
dd if=/dev/zero of="$F/corrupt.mp3" bs=1 seek=2000 count=6000 conv=notrunc status=none

pass=0
fail=0
{
  echo "media matrix"
  echo "build   $BIN"
  echo "oracle  $ORACLE"
  echo "source  $SOURCE_WAV"
  echo "date    $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo
} > "$REPORT"

record() { # verdict name detail
  if [ "$1" = "PASS" ]; then pass=$((pass + 1)); else fail=$((fail + 1)); fi
  printf '%-4s  %-22s  %s\n' "$1" "$2" "$3" | tee -a "$REPORT"
}

# Эталон: PCM, полученный oracle из исходного WAV. Для lossless-контейнеров декод обязан совпасть
# побайтово, для lossy сравнивается длина: сами кодеки разные, а вот раскладка кадров — нет.
"$ORACLE" $q -i "$SOURCE_WAV" -ac 1 -ar 16000 -f s16le "$WORK/out/reference.raw"
reference_md5="$(md5sum "$WORK/out/reference.raw" | cut -d' ' -f1)"
reference_bytes="$(stat -c %s "$WORK/out/reference.raw")"

decode() { # fixture lossless
  local fixture="$1" lossless="$2"
  local name; name="$(basename "$fixture")"
  local raw="$WORK/out/$name.raw"

  if ! "$FFPROBE" -hide_banner -loglevel error -select_streams a:0 -show_entries stream=codec_name,sample_rate,channels -of csv=p=0 "$fixture" > "$WORK/out/$name.probe" 2>"$WORK/out/$name.probe.err"; then
    record FAIL "probe $name" "$(head -1 "$WORK/out/$name.probe.err")"
    return
  fi
  local probe; probe="$(tr -d '\r\n' < "$WORK/out/$name.probe")"
  if [ -z "$probe" ]; then
    record FAIL "probe $name" "audio stream not found"
    return
  fi

  if ! "$FFMPEG" $q -i "$fixture" -map 0:a:0 -ac 1 -ar 16000 -f s16le "$raw" 2>"$WORK/out/$name.err"; then
    record FAIL "decode $name" "$(head -1 "$WORK/out/$name.err")"
    return
  fi
  local bytes; bytes="$(stat -c %s "$raw")"
  if [ "$bytes" -eq 0 ]; then
    record FAIL "decode $name" "empty PCM"
    return
  fi

  if [ "$lossless" = "lossless" ]; then
    local md5; md5="$(md5sum "$raw" | cut -d' ' -f1)"
    if [ "$md5" = "$reference_md5" ]; then
      record PASS "decode $name" "$probe, PCM байт-в-байт совпал с эталоном ($bytes)"
    else
      record FAIL "decode $name" "lossless PCM разошёлся с эталоном: $md5 против $reference_md5"
    fi
  else
    # Кодеры добавляют padding, поэтому допускается расхождение до 0,25 с при 16 kHz mono.
    local delta=$((bytes - reference_bytes))
    local limit=8000
    if [ "${delta#-}" -le "$limit" ]; then
      record PASS "decode $name" "$probe, PCM $bytes байт, отклонение $delta"
    else
      record FAIL "decode $name" "длина PCM разошлась на $delta байт"
    fi
  fi
}

echo "[matrix] decoding with the build under test"
decode "$F/pcm.wav" lossless
decode "$F/audio.flac" lossless
decode "$F/audio.mp3" lossy
decode "$F/audio.m4a" lossy
decode "$F/audio.mov" lossy
decode "$F/audio.webm" lossy
decode "$F/audio.mkv" lossy
decode "$F/video.mp4" lossy

# Файл без звука: probe не должен найти audio stream, а decode обязан отказать.
if "$FFPROBE" -hide_banner -loglevel error -select_streams a:0 -show_entries stream=codec_name -of csv=p=0 "$F/no-audio.mp4" 2>/dev/null | grep -q .; then
  record FAIL "no-audio.mp4" "probe выдумал audio stream"
else
  if "$FFMPEG" $q -i "$F/no-audio.mp4" -map 0:a:0 -ac 1 -ar 16000 -f s16le "$WORK/out/no-audio.raw" 2>/dev/null; then
    record FAIL "no-audio.mp4" "decode завершился успехом без звуковой дорожки"
  else
    record PASS "no-audio.mp4" "probe пуст, decode отказал"
  fi
fi

# Битый файл: успех с пустым или обрезанным PCM недопустим, нужен явный отказ либо честная ошибка.
if "$FFMPEG" $q -xerror -i "$F/corrupt.mp3" -map 0:a:0 -ac 1 -ar 16000 -f s16le "$WORK/out/corrupt.raw" 2>"$WORK/out/corrupt.err"; then
  record FAIL "corrupt.mp3" "decode повреждённого файла вернул успех"
else
  record PASS "corrupt.mp3" "$(head -1 "$WORK/out/corrupt.err")"
fi

# Отмена: процесс убивается на длинном декодировании и обязан умереть, не оставив успеха.
# Петля должна быть заведомо длиннее паузы до kill, иначе проверяется гонка, а не отмена.
"$ORACLE" $q -stream_loop 600 -i "$F/audio.mp3" -c:a copy "$F/long.mp3"
"$FFMPEG" $q -i "$F/long.mp3" -ac 1 -ar 16000 -f s16le "$WORK/out/cancel.raw" 2>/dev/null &
cancel_pid=$!
sleep 0.3
kill -9 "$cancel_pid" 2>/dev/null
wait "$cancel_pid" 2>/dev/null
cancel_status=$?
cancel_bytes=0
[ -f "$WORK/out/cancel.raw" ] && cancel_bytes="$(stat -c %s "$WORK/out/cancel.raw")"
full_bytes=$((reference_bytes * 601))
if [ "$cancel_status" -eq 0 ]; then
  record FAIL "cancel" "убитый процесс отчитался успехом"
elif [ "$cancel_bytes" -ge "$full_bytes" ]; then
  record FAIL "cancel" "процесс успел выдать полный PCM: $cancel_bytes"
else
  record PASS "cancel" "процесс снят с кодом $cancel_status, PCM оборван на $cancel_bytes из $full_bytes"
fi

{
  echo
  echo "PASS $pass, FAIL $fail"
} | tee -a "$REPORT"

echo "[matrix] report: $REPORT"
[ "$fail" -eq 0 ]
