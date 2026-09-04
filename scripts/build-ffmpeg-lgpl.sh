#!/usr/bin/env bash
# Воспроизводимая shared LGPL-сборка FFmpeg для Notetaker (R1, ADR-003).
#
# Запускать под MSYS2 MINGW64:
#   MSYSTEM=MINGW64 C:/msys64/usr/bin/bash.exe -l scripts/build-ffmpeg-lgpl.sh <build-root>
#
# Путь репозитория содержит пробел, а build-система FFmpeg на пробелах ломается, поэтому
# сборка идёт в свободный от пробелов каталог, а артефакты копируются обратно.
set -euo pipefail

FFMPEG_VERSION="8.1.2"
# Пин источника: тарбол с ffmpeg.org, подпись проверена ключом FFmpeg release signing key
# FCF986EA15E6E293A5644F10B4322F04D67658D8 (импортирован с keyserver.ubuntu.com).
SOURCE_SHA256="464beb5e7bf0c311e68b45ae2f04e9cc2af88851abb4082231742a74d97b524c"

BUILD_ROOT="${1:-/c/wigigadict-ffmpeg-build/build-1}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_TARBALL="$REPO_ROOT/artifacts/ffmpeg-lgpl/source/ffmpeg-$FFMPEG_VERSION.tar.xz"

export PATH="/mingw64/bin:$PATH"
# Штамп времени в PE-заголовке — единственное, что иначе различает две сборки одного
# исходника. binutils берёт его из SOURCE_DATE_EPOCH; константа — дата подписи релиза 8.1.2.
export SOURCE_DATE_EPOCH=1781675339

case "$BUILD_ROOT" in
  *\ *) echo "build root must not contain a space: $BUILD_ROOT" >&2; exit 1 ;;
esac

echo "[ffmpeg] verifying pinned source"
actual="$(sha256sum "$SOURCE_TARBALL" | cut -d' ' -f1)"
if [ "$actual" != "$SOURCE_SHA256" ]; then
  echo "source sha256 mismatch: expected $SOURCE_SHA256, got $actual" >&2
  exit 1
fi

SOURCE_DIR="$BUILD_ROOT/ffmpeg-$FFMPEG_VERSION"
# Путь сборки не должен попадать в бинарь: FFmpeg вшивает строку configure целиком, поэтому
# prefix фиксирован, а раскладка делается через DESTDIR. Иначе две сборки различаются всегда.
VIRTUAL_PREFIX="/wigigadict-ffmpeg"
DESTDIR_ROOT="$BUILD_ROOT/install"
PREFIX="$DESTDIR_ROOT$VIRTUAL_PREFIX"

# FFMPEG_REUSE_BUILD=1 повторяет только проверки и манифест поверх готовой сборки.
if [ "${FFMPEG_REUSE_BUILD:-0}" = "1" ] && [ -x "$PREFIX/bin/ffmpeg.exe" ]; then
  echo "[ffmpeg] reusing existing build in $PREFIX"
else

rm -rf "$BUILD_ROOT"
mkdir -p "$BUILD_ROOT"
tar -xf "$SOURCE_TARBALL" -C "$BUILD_ROOT"

cd "$SOURCE_DIR"
echo "[ffmpeg] configure"
# LGPL 2.1 профиль: без --enable-gpl, --enable-nonfree и --enable-version3.
# --disable-autodetect запрещает подхватывать внешние библиотеки из окружения: allowlist пуст,
# все нужные форматы покрываются родными декодерами FFmpeg.
# -static-libgcc убирает из поставки runtime-DLL компилятора.
./configure \
  --prefix="$VIRTUAL_PREFIX" \
  --arch=x86_64 \
  --target-os=mingw32 \
  --enable-shared \
  --disable-static \
  --disable-autodetect \
  --disable-network \
  --disable-doc \
  --disable-debug \
  --disable-ffplay \
  --enable-w32threads \
  --extra-ldflags="-static-libgcc -Wl,--no-insert-timestamp"

echo "[ffmpeg] build"
make -j"$(nproc)"
make install DESTDIR="$DESTDIR_ROOT"

fi

BIN="$PREFIX/bin"
echo "[ffmpeg] license gate"
buildconf="$("$BIN/ffmpeg.exe" -hide_banner -buildconf 2>&1)"
for forbidden in "--enable-gpl" "--enable-nonfree" "--enable-version3"; do
  if printf '%s' "$buildconf" | grep -q -- "$forbidden"; then
    echo "forbidden configure flag in build: $forbidden" >&2
    exit 1
  fi
done
license="$("$BIN/ffmpeg.exe" -hide_banner -L 2>&1 | head -8)"
# Баннер LGPL 2.1 разбит на строки, поэтому проверяются обе половины утверждения,
# а отсутствие GPL уже доказано отсутствием флагов в buildconf выше.
if ! printf '%s' "$license" | grep -q "Lesser General Public"; then
  echo "license banner is not LGPL: $license" >&2
  exit 1
fi
if ! printf '%s' "$license" | grep -q "version 2.1"; then
  echo "license banner is not version 2.1: $license" >&2
  exit 1
fi

echo "[ffmpeg] manifest"
MANIFEST="$BUILD_ROOT/manifest.txt"
{
  echo "ffmpeg $FFMPEG_VERSION"
  echo "source sha256 $SOURCE_SHA256"
  echo "built $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "build root $BUILD_ROOT"
  echo
  echo "== toolchain =="
  gcc --version | head -1
  nasm --version
  make --version | head -1
  echo
  echo "== configure =="
  printf '%s\n' "$buildconf"
  echo
  echo "== license =="
  printf '%s\n' "$license"
  echo
  echo "== installed files =="
  (cd "$PREFIX" && find . -type f | sort | xargs sha256sum)
  echo
  echo "== PE dependencies =="
  for exe in "$BIN"/*.exe "$BIN"/*.dll; do
    [ -e "$exe" ] || continue
    echo "-- $(basename "$exe")"
    objdump -p "$exe" | grep "DLL Name:" | sort -u
  done
} > "$MANIFEST"

echo "[ffmpeg] done: $PREFIX"
echo "[ffmpeg] manifest: $MANIFEST"
