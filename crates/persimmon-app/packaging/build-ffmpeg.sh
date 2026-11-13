#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Build the macOS app's FFmpeg and ffprobe from source (D5, DD-04, DD-44):
# GPL, static, with x264, x265, libvpx, opus, LAME, dav1d and libaom, no
# network protocols (DD-11) and nothing autodetected. The codec libraries
# are built static into <native-dir>/ffprefix, apart from the shared dav1d
# and libaom that libheif uses, so ffmpeg and ffprobe link only system
# libraries. They land in <native-dir>/prefix/bin, with the configure line
# in prefix/share/ffmpeg-<version>-configure.txt.
#
#   build-ffmpeg.sh [native-dir]      (default target/native)
# shellcheck source=lib.sh
. "$(cd "$(dirname "$0")" && pwd)/lib.sh"

FP=$NATIVE/ffprefix
mkdir -p "$FP/lib/pkgconfig" "$P/bin"
export PKG_CONFIG_LIBDIR=$FP/lib/pkgconfig
export CFLAGS="-O2 -arch arm64 -mmacosx-version-min=14.0 -I$FP/include"
export CXXFLAGS="$CFLAGS" CPPFLAGS="-I$FP/include"
export LDFLAGS="-arch arm64 -mmacosx-version-min=14.0 -L$FP/lib"

# Like lib.sh's step, under its own marker: dav1d and libaom are also
# built shared for libheif.
fstep() {
  local name=$1 builder=$2 t0=$SECONDS dir; shift 2
  if [ -f "$P/.done-ffmpeg-$name" ]; then echo "=== $name, static (done)"; return; fi
  echo "=== $name, static"
  dir=$(fetch "$name")
  "$builder" "$dir" "$@"
  licences "$name" "$dir"
  touch "$P/.done-ffmpeg-$name"
  echo "=== $name, static $((SECONDS - t0))s"
}
static_cmake() { local d=$1 src=$1; shift; [ "${CMAKE_SUBDIR:-}" ] && src=$d/$CMAKE_SUBDIR
  "${CMAKE[@]}" -DCMAKE_INSTALL_PREFIX="$FP" -DCMAKE_PREFIX_PATH="$FP" -DBUILD_SHARED_LIBS=OFF -S "$src" -B "$d/_b" "$@" \
    && ninja -C "$d/_b" && ninja -C "$d/_b" install; }
static_meson() { local d=$1; shift; meson setup --wrap-mode=nodownload --prefix="$FP" --libdir=lib --buildtype=release -Ddefault_library=static "$@" "$d/_b" "$d" \
  && ninja -C "$d/_b" && ninja -C "$d/_b" install; }
static_auto() { local d=$1; shift; (cd "$d" && ./configure --prefix="$FP" "$@" && make -j"$J" && make install); }

fstep x264 static_auto --enable-static --disable-cli --enable-pic --host=aarch64-apple-darwin \
  --extra-cflags="-mmacosx-version-min=14.0" --extra-ldflags="-mmacosx-version-min=14.0"
CMAKE_SUBDIR=source fstep x265 static_cmake -DENABLE_SHARED=OFF -DENABLE_CLI=OFF
# libvpx names its targets by Darwin version, and each sets that version's
# minimum macOS: Darwin 23 is macOS 14 (DD-27).
vpx() { local d=$1; shift
  static_auto "$d" --target=arm64-darwin23-gcc --disable-shared --enable-static --enable-pic --enable-vp9-highbitdepth \
    --disable-examples --disable-tools --disable-docs --disable-unit-tests "$@"; }
fstep libvpx vpx
fstep opus static_cmake -DOPUS_BUILD_PROGRAMS=OFF -DOPUS_BUILD_TESTING=OFF
fstep lame static_auto --disable-shared --enable-static --disable-frontend
fstep dav1d static_meson -Denable_tools=false -Denable_tests=false
fstep libaom static_cmake -DENABLE_DOCS=0 -DENABLE_EXAMPLES=0 -DENABLE_TESTS=0 -DENABLE_TOOLS=0 -DCONFIG_AV1_DECODER=0

# libaom is built to encode only, so FFmpeg's libaom decoder is left out:
# dav1d decodes AV1.
if [ ! -f "$P/.done-ffmpeg" ]; then
  echo "=== ffmpeg"
  t0=$SECONDS
  d=$(fetch ffmpeg)
  (cd "$d" && ./configure --prefix="$P" --cc=clang --arch=arm64 \
    --enable-gpl --disable-network --disable-autodetect --enable-zlib \
    --enable-libx264 --enable-libx265 --enable-libvpx --enable-libopus --enable-libmp3lame \
    --enable-libdav1d --enable-libaom --disable-decoder=libaom_av1 \
    --disable-ffplay --disable-doc --disable-debug \
    --pkg-config-flags=--static \
    --extra-cflags="-mmacosx-version-min=14.0 -I$FP/include" \
    --extra-ldflags="-mmacosx-version-min=14.0 -L$FP/lib" \
    --extra-libs=-lc++ \
    && make -j"$J")
  cp "$d/ffmpeg" "$d/ffprobe" "$P/bin/"
  codesign --force -s - "$P/bin/ffmpeg" "$P/bin/ffprobe" 2>/dev/null
  licences ffmpeg "$d"
  "$P/bin/ffmpeg" -hide_banner -buildconf > "$P/share/ffmpeg-$(version_of ffmpeg)-configure.txt"
  touch "$P/.done-ffmpeg"
  echo "=== ffmpeg $((SECONDS - t0))s"
fi

rm -rf "${BUILD:?}" "${FP:?}"
echo "=== built"; "$P/bin/ffmpeg" -hide_banner -version | head -1
