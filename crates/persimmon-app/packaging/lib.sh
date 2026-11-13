# SPDX-License-Identifier: GPL-3.0-or-later
# Shared by build-libs.sh and build-ffmpeg.sh: where things go, fetching a
# pinned source, and the build steps. Sourced, never run.
#
# <native-dir> (default target/native) holds:
#   sources/  the downloaded tarballs, each checked against versions.sh
#   build/    unpacked trees, deleted when a script finishes
#   prefix/   what's bundled: lib/*.dylib, bin/ffmpeg, bin/ffprobe,
#             share/licenses/<name>/, share/ffmpeg-<version>-configure.txt
# A finished step leaves prefix/.done-<name>, so a rerun (or the release
# workflow's cached prefix) skips it.
set -euo pipefail
PACKAGING=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=versions.sh
. "$PACKAGING/versions.sh"
NATIVE=${1:-$PACKAGING/../../../target/native}
mkdir -p "$NATIVE"
NATIVE=$(cd "$NATIVE" && pwd)
SRC=$NATIVE/sources BUILD=$NATIVE/build P=$NATIVE/prefix
mkdir -p "$SRC" "$BUILD" "$P/lib/pkgconfig" "$P/share/licenses"

export MACOSX_DEPLOYMENT_TARGET=14.0
unset PKG_CONFIG_PATH
export PKG_CONFIG_LIBDIR=$P/lib/pkgconfig:$P/share/pkgconfig
export CFLAGS="-O2 -arch arm64 -mmacosx-version-min=14.0 -I$P/include"
export CXXFLAGS="$CFLAGS" CPPFLAGS="-I$P/include"
export LDFLAGS="-arch arm64 -mmacosx-version-min=14.0 -L$P/lib"
J=$(sysctl -n hw.ncpu)

# The versions.sh line for <name>: "name version url sha256".
source_line() {
  local line; line=$(awk -v n="$1" '$1 == n' <<<"$SOURCES")
  [ -n "$line" ] || { echo "lib.sh: no source named $1 in versions.sh" >&2; exit 1; }
  echo "$line"
}
version_of() { source_line "$1" | awk '{print $2}'; }

# fetch <name>: download once into sources/, check the SHA-256, unpack a
# fresh tree into build/<name>, and print its folder. An "unpinned"
# checksum prints the file's own and stops, so it's pinned by hand once.
fetch() {
  local name=$1 version url sha ext file got
  read -r _ version url sha <<<"$(source_line "$name")"
  case "$url" in *.tar.gz | *.tgz) ext=tar.gz ;; *.tar.xz) ext=tar.xz ;; *.tar.bz2) ext=tar.bz2 ;; *) ext=tar ;; esac
  file=$SRC/$name-$version.$ext
  if [ ! -f "$file" ]; then
    curl -fsSL --retry 3 -o "$file.part" "$url"
    mv "$file.part" "$file"
  fi
  got=$(shasum -a 256 "$file" | awk '{print $1}')
  if [ "$sha" != "$got" ]; then
    if [ "$sha" = unpinned ]; then echo "pin: $name $got" >&2; else echo "fetch: $name: SHA-256 $got, expected $sha" >&2; fi
    exit 1
  fi
  rm -rf "${BUILD:?}/$name"; mkdir -p "$BUILD/$name"
  tar -xf "$file" -C "$BUILD/$name" --strip-components=1
  echo "$BUILD/$name"
}

# licences <name> <source-dir>: keep the project's licence and patent files
# for the bundled licence list.
licences() {
  local name=$1 dir=$2 f found=0
  mkdir -p "$P/share/licenses/$name"
  for f in "$dir"/COPYING* "$dir"/LICENSE* "$dir"/LICENCE* "$dir"/COPYRIGHT* "$dir"/PATENTS*; do
    [ -f "$f" ] || continue
    cp "$f" "$P/share/licenses/$name/"; found=1
  done
  [ "$found" = 1 ] || { echo "licences: $name has no licence file" >&2; exit 1; }
}

# step <name> <builder> [args…]: fetch, build and keep the licences, once.
step() {
  local name=$1 builder=$2 t0=$SECONDS dir; shift 2
  if [ -f "$P/.done-$name" ]; then echo "=== $name (done)"; return; fi
  echo "=== $name"
  dir=$(fetch "$name")
  "$builder" "$dir" "$@"
  licences "$name" "$dir"
  touch "$P/.done-$name"
  echo "=== $name $((SECONDS - t0))s"
}

CMAKE=(cmake -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$P" -DCMAKE_INSTALL_LIBDIR=lib
  -DCMAKE_OSX_DEPLOYMENT_TARGET=14.0 -DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_PREFIX_PATH="$P"
  "-DCMAKE_IGNORE_PREFIX_PATH=/opt/homebrew;/usr/local" -DCMAKE_INSTALL_NAME_DIR=@rpath
  -DCMAKE_POLICY_VERSION_MINIMUM=3.5)
cmake_build() { local d=$1; shift; "${CMAKE[@]}" -DBUILD_SHARED_LIBS=ON -S "$d" -B "$d/_b" "$@" && ninja -C "$d/_b" && ninja -C "$d/_b" install; }
meson_build() { local d=$1; shift; meson setup --wrap-mode=nodownload --prefix="$P" --libdir=lib --buildtype=release -Ddefault_library=shared "$@" "$d/_b" "$d" && ninja -C "$d/_b" && ninja -C "$d/_b" install; }
auto_build() { local d=$1; shift; (cd "$d" && ./configure --prefix="$P" --disable-static "$@" && make -j"$J" && make install); }

# fix_names: every dylib in the prefix is named @rpath/<file>, its
# dependencies inside the prefix are @rpath too, rpaths into the prefix go,
# and each is re-signed ad hoc (an edited arm64 binary won't load unsigned).
fix_names() {
  local f id dep rp
  for f in $(find "$P/lib" -maxdepth 1 -name '*.dylib' -type f); do
    id=$(otool -D "$f" | tail -1)
    install_name_tool -id "@rpath/$(basename "$id")" "$f" 2>/dev/null
    for dep in $(otool -L "$f" | tail -n +2 | awk '{print $1}'); do
      case "$dep" in "$P"/*) install_name_tool -change "$dep" "@rpath/$(basename "$dep")" "$f" 2>/dev/null ;; esac
    done
    for rp in $(otool -l "$f" | awk '/LC_RPATH/ {getline; getline; print $2}'); do
      case "$rp" in "$P"*) install_name_tool -delete_rpath "$rp" "$f" 2>/dev/null ;; esac
    done
    codesign --force -s - "$f" 2>/dev/null
  done
}
