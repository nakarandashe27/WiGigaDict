#!/usr/bin/env bash
# Compliance bundle для собственной LGPL-сборки FFmpeg (R1, ADR-003).
#
#   MSYSTEM=MINGW64 C:/msys64/usr/bin/bash.exe -l scripts/ffmpeg-compliance-bundle.sh <build-root> <out-dir>
#
# LGPL 2.1 требует распространять вместе с бинарником точный соответствующий исходник, полный
# рецепт сборки и тексты лицензий. Bundle собирается из уже проверенной сборки и ничего не пересобирает.
set -euo pipefail

BUILD_ROOT="${1:-/c/wigigadict-ffmpeg-build/build-5}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${2:-$REPO_ROOT/artifacts/ffmpeg-lgpl/bundle}"
FFMPEG_VERSION="8.1.2"

export PATH="/mingw64/bin:$PATH"

SOURCE_DIR="$BUILD_ROOT/ffmpeg-$FFMPEG_VERSION"
BIN="$BUILD_ROOT/install/wigigadict-ffmpeg/bin"
SOURCE_FILES="$REPO_ROOT/artifacts/ffmpeg-lgpl/source"

for required in "$SOURCE_DIR" "$BIN/ffmpeg.exe" "$BUILD_ROOT/manifest.txt"; do
  if [ ! -e "$required" ]; then echo "missing: $required" >&2; exit 1; fi
done

rm -rf "$OUT"
mkdir -p "$OUT/binaries" "$OUT/licenses" "$OUT/source"

echo "[bundle] binaries"
cp "$BIN"/*.exe "$BIN"/*.dll "$OUT/binaries/"

echo "[bundle] source and signature"
cp "$SOURCE_FILES/ffmpeg-$FFMPEG_VERSION.tar.xz" "$SOURCE_FILES/ffmpeg-$FFMPEG_VERSION.tar.xz.asc" "$OUT/source/"

echo "[bundle] licenses"
for name in LICENSE.md COPYING.LGPLv2.1 CREDITS; do
  cp "$SOURCE_DIR/$name" "$OUT/licenses/"
done

echo "[bundle] changes.diff"
# Вопрос LGPL — «менялся ли исходник», а не «что сборка сгенерировала рядом». Поэтому сравниваются
# только файлы, пришедшие в тарболе: появившиеся config.h, списки кодеков и объектники к делу
# не относятся и в обход апстрима просто не попадают.
work="$(mktemp -d)"
tar -xf "$SOURCE_FILES/ffmpeg-$FFMPEG_VERSION.tar.xz" -C "$work"
pristine="$work/ffmpeg-$FFMPEG_VERSION"
: > "$OUT/changes.diff"
changed_files=0
while IFS= read -r file; do
  if ! cmp -s "$pristine/$file" "$SOURCE_DIR/$file"; then
    changed_files=$((changed_files + 1))
    diff -u "$pristine/$file" "$SOURCE_DIR/$file" >> "$OUT/changes.diff" || true
  fi
done < <(cd "$pristine" && find . -type f | sort)
rm -rf "$work"

echo "[bundle] manifest"
cp "$BUILD_ROOT/manifest.txt" "$OUT/build-manifest.txt"
(cd "$OUT" && find . -type f ! -name SHA256SUMS.txt | sort | xargs sha256sum > SHA256SUMS.txt)

{
  echo "# FFmpeg LGPL compliance bundle"
  echo
  echo "- Версия: $FFMPEG_VERSION, tag n$FFMPEG_VERSION"
  echo "- Исходник: \`source/ffmpeg-$FFMPEG_VERSION.tar.xz\` с ffmpeg.org, подпись \`.asc\` ключа FCF986EA15E6E293A5644F10B4322F04D67658D8"
  echo "- Изменённых файлов апстрима: $changed_files (\`changes.diff\` собран обходом файлов тарбола)"
  echo "- Профиль: shared LGPL 2.1, без \`--enable-gpl\`, \`--enable-nonfree\`, \`--enable-version3\`; внешних библиотек нет (\`--disable-autodetect\`)"
  echo "- Рецепт сборки: \`scripts/build-ffmpeg-lgpl.sh\`; строка configure, версии тулчейна, суммы файлов и PE-зависимости — в \`build-manifest.txt\`"
  echo "- Проверка форматов: \`scripts/ffmpeg-media-matrix.sh\`"
  echo
  echo "Проверить подпись исходника:"
  echo
  echo '```bash'
  echo "gpg --recv-keys FCF986EA15E6E293A5644F10B4322F04D67658D8"
  echo "gpg --verify source/ffmpeg-$FFMPEG_VERSION.tar.xz.asc source/ffmpeg-$FFMPEG_VERSION.tar.xz"
  echo '```'
} > "$OUT/README.md"

echo "[bundle] done: $OUT"
du -sh "$OUT"
