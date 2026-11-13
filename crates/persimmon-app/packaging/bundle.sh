#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Build, finish, sign and package the macOS app (D5, DD-44):
#
#   bundle.sh <native-dir> <out-dir>
#
# Tauri builds Persimmon.app against <native-dir>/prefix (build-libs.sh and
# build-ffmpeg.sh). This script then copies in every library the app needs
# (Contents/Frameworks), FFmpeg and ffprobe (Contents/MacOS) and the licence
# list (Contents/Resources); signs inside-out; checks the result with
# verify.sh; and writes to <out-dir>:
#   Persimmon-<version>-arm64.dmg            and its .sha256
#   Persimmon-<version>-sources.tar          and its .sha256: every source
#                                            tarball, these scripts, FFmpeg's
#                                            configure line, PATCHES.txt
#   ffmpeg-<ffmpeg version>-configure.txt
#
# Signing: with APPLE_SIGNING_IDENTITY set (a Developer ID Application
# identity in the keychain), everything is signed with it, the hardened
# runtime and a timestamp, and so is the DMG. Without it, the signatures
# are ad hoc and there's no hardened runtime: library validation would
# refuse ad-hoc libraries. Notarising is the release workflow's step.
set -euo pipefail
if [ $# -ne 2 ]; then echo "usage: bundle.sh <native-dir> <out-dir>" >&2; exit 2; fi
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
native=$(cd "$1" && pwd)
mkdir -p "$2"; out=$(cd "$2" && pwd)
P=$native/prefix
ID=io.github.persimmon-app.Persimmon
identity=${APPLE_SIGNING_IDENTITY:--}
sign_opts=()
[ "$identity" = - ] || sign_opts=(--options runtime --timestamp)
# shellcheck source=versions.sh
. "$here/versions.sh"
ffversion=$(awk '$1 == "ffmpeg" {print $2}' <<<"$SOURCES")
for f in "$P/bin/ffmpeg" "$P/bin/ffprobe" "$P/lib/libvips.42.dylib" "$P/share/ffmpeg-$ffversion-configure.txt"; do
  [ -f "$f" ] || { echo "bundle.sh: missing $f (run build-libs.sh and build-ffmpeg.sh)" >&2; exit 1; }
done
# Every source the release must carry, found before the long build starts.
source_files=()
while read -r src v _; do
  [ -n "$src" ] || continue
  found=""
  for f in "$native/sources/$src-$v".tar*; do [ -f "$f" ] && found=$f && break; done
  [ -n "$found" ] || { echo "bundle.sh: no downloaded source for $src $v" >&2; exit 1; }
  source_files+=("$found")
done <<<"$SOURCES"
version=$("$root/.github/scripts/version.sh")
work=$(mktemp -d "${TMPDIR:-/tmp}/prsm-bundle.XXXXXX")
trap 'rm -rf "$work"' EXIT

echo "=== the app"
# Without the Apple variables: Tauri would import, sign and notarise on its
# own (and an empty APPLE_CERTIFICATE, as CI passes an absent secret, makes
# it fail). This script signs, and the release workflow notarises.
(cd "$root/crates/persimmon-app" && env -u APPLE_CERTIFICATE -u APPLE_CERTIFICATE_PASSWORD -u APPLE_SIGNING_IDENTITY \
  -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH \
  PKG_CONFIG_LIBDIR="$P/lib/pkgconfig" ui/node_modules/.bin/tauri build --bundles app)
ditto "$root/target/release/bundle/macos/Persimmon.app" "$work/Persimmon.app"
app=$work/Persimmon.app
mkdir -p "$app/Contents/Frameworks"

# Every @rpath library the app reaches, under its install name, as a real file.
add_libs() {
  local dep name
  for dep in $(otool -L "$1" | tail -n +2 | awk '{print $1}'); do
    case "$dep" in @rpath/*) ;; *) continue ;; esac
    name=${dep#@rpath/}
    [ -f "$app/Contents/Frameworks/$name" ] && continue
    [ -f "$P/lib/$name" ] || { echo "bundle.sh: $1 needs $name, which the prefix doesn't have" >&2; exit 1; }
    cp -L "$P/lib/$name" "$app/Contents/Frameworks/$name"
    chmod u+w "$app/Contents/Frameworks/$name"
    add_libs "$app/Contents/Frameworks/$name"
  done
}
add_libs "$app/Contents/MacOS/persimmon-app"
cp "$P/bin/ffmpeg" "$P/bin/ffprobe" "$app/Contents/MacOS/"
"$here/licences.sh" "$native" "$app/Contents/Resources/THIRD-PARTY-LICENSES.html"

echo "=== signing ($([ "$identity" = - ] && echo ad hoc || echo "$identity"))"
for f in "$app"/Contents/Frameworks/*.dylib "$app/Contents/MacOS/ffmpeg" "$app/Contents/MacOS/ffprobe"; do
  codesign --force ${sign_opts[@]+"${sign_opts[@]}"} -s "$identity" "$f"
done
codesign --force ${sign_opts[@]+"${sign_opts[@]}"} -s "$identity" --identifier "$ID" "$app"
# Gatekeeper and the ticket are checked once the workflow has notarised.
"$here/verify.sh" --before-notarisation "$app"

echo "=== the DMG"
dmg=$out/Persimmon-$version-arm64.dmg
mkdir "$work/dmg"
mv "$app" "$work/dmg/"
ln -s /Applications "$work/dmg/Applications"
rm -f "$dmg"
hdiutil create -quiet -volname Persimmon -srcfolder "$work/dmg" -fs HFS+ -format UDZO "$dmg"
[ "$identity" = - ] || codesign --force --timestamp -s "$identity" "$dmg"

echo "=== the sources"
name=Persimmon-$version-sources
mkdir -p "$work/$name/sources"
cp "${source_files[@]}" "$work/$name/sources/"
rsync -a "$here/" "$work/$name/packaging/"
cp "$P/share/ffmpeg-$ffversion-configure.txt" "$work/$name/"
cp "$P/share/ffmpeg-$ffversion-configure.txt" "$out/"
echo "No patches are applied to any of these sources." > "$work/$name/PATCHES.txt"
(cd "$work" && COPYFILE_DISABLE=1 tar -cf "$out/$name.tar" "$name")

cd "$out"
for f in "$(basename "$dmg")" "$name.tar"; do shasum -a 256 "$f" > "$f.sha256"; done
ls -l "$out"
