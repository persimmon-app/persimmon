#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# The release check for the packaged app (Phase 2 spec, "Testing → The
# packaged app"):
#
#   verify.sh [--before-notarisation] <Persimmon.app> [Persimmon.dmg]
#
# Prints "verified (ad-hoc)" or "verified (developer-id)", or one
# "verify: <file>: <problem>" line per problem and exits 1. It checks the
# signature and identifier; that every Mach-O file is arm64 only for
# macOS 14; that nothing links outside the bundle and the system; that
# every @rpath library is in Contents/Frameworks; the bundled FFmpeg's
# configure line, encoders and protocols; the licence list; and, for a
# Developer ID signature only, Gatekeeper and the notarisation ticket:
# --before-notarisation leaves those two out, for the check bundle.sh makes
# before the release workflow notarises. PERSIMMON_VERIFY_MODE=developer-id
# forces the Developer ID checks (the tests' fake bundles can't carry one).
set -euo pipefail
ID=io.github.persimmon-app.Persimmon
FLAGS="--enable-gpl --disable-network --enable-libx264 --enable-libx265 --enable-libvpx --enable-libopus --enable-libmp3lame --enable-libdav1d --enable-libaom"
ENCODERS="libx264 libx265 libvpx-vp9 libopus libmp3lame libaom-av1"
NETWORK="tcp http https udp rtmp"

before=0
if [ "${1:-}" = --before-notarisation ]; then before=1; shift; fi
if [ $# -lt 1 ] || [ ! -d "$1" ]; then echo "usage: verify.sh [--before-notarisation] <Persimmon.app> [Persimmon.dmg]" >&2; exit 2; fi
app=$(cd "$1" && pwd) dmg=${2:-}
problems=0
problem() { echo "verify: ${1#"$app"/}: $2"; problems=$((problems + 1)); }

# The signature, and whose it is.
if ! out=$(codesign --verify --deep --strict "$app" 2>&1); then
  problem "$app" "the signature doesn't verify ($(head -1 <<<"$out"))"
fi
info=$(codesign -dv --verbose=2 "$app" 2>&1 || true)
id=$(sed -n 's/^Identifier=//p' <<<"$info")
[ "$id" = "$ID" ] || problem "$app" "signed as ${id:-nothing}, not $ID"
if grep -q '^Authority=Developer ID Application' <<<"$info" || [ "${PERSIMMON_VERIFY_MODE:-}" = developer-id ]; then
  mode=developer-id
else
  mode=ad-hoc
fi

# Every Mach-O file: arm64 only, macOS 14, linking only the bundle and the system.
while IFS= read -r -d '' f; do
  file -b "$f" | grep -q Mach-O || continue
  archs=$(lipo -archs "$f" 2>/dev/null || echo unknown)
  [ "$archs" = arm64 ] || problem "$f" "built for $archs, not arm64 only"
  minos=$(vtool -show-build "$f" 2>/dev/null | awk '$1 == "minos" {print $2; exit}')
  [ "$minos" = 14.0 ] || problem "$f" "minos ${minos:-unknown}, not 14.0"
  self=$(otool -D "$f" | tail -n +2 | head -1)
  while read -r dep; do
    [ -z "$dep" ] || [ "$dep" = "$self" ] && continue
    case "$dep" in
      /usr/lib/* | /System/*) ;;
      @rpath/*) [ -f "$app/Contents/Frameworks/${dep#@rpath/}" ] \
        || problem "$f" "needs ${dep#@rpath/}, which is not in the bundle (Contents/Frameworks)" ;;
      *) problem "$f" "links $dep, outside the bundle and the system" ;;
    esac
  done < <(otool -L "$f" | tail -n +2 | awk '{print $1}')
  while read -r rp; do
    case "$rp" in
      @executable_path/../Frameworks | @loader_path | @loader_path/../Frameworks) ;;
      *) problem "$f" "has the rpath $rp" ;;
    esac
  done < <(otool -l "$f" | awk '/LC_RPATH/ {getline; getline; print $2}')
done < <(find "$app/Contents" -type f -print0)

# The bundled FFmpeg: GPL, no network, every encoder the engine asks for.
ff=$app/Contents/MacOS/ffmpeg
for p in ffmpeg ffprobe; do [ -x "$app/Contents/MacOS/$p" ] || problem "$app/Contents/MacOS/$p" "is missing"; done
if [ -x "$ff" ]; then
  conf=$("$ff" -hide_banner -buildconf 2>&1 || true)
  for flag in $FLAGS; do
    grep -Eq -- "^[[:space:]]*$flag\$" <<<"$conf" || problem "$ff" "ffmpeg wasn't configured with $flag"
  done
  encoders=$("$ff" -hide_banner -encoders 2>&1 | awk '{print $2}' || true)
  for e in $ENCODERS; do grep -qx -- "$e" <<<"$encoders" || problem "$ff" "ffmpeg has no $e encoder"; done
  protocols=$("$ff" -hide_banner -protocols 2>&1 | awk '{print $1}' || true)
  for p in $NETWORK; do grep -qx -- "$p" <<<"$protocols" && problem "$ff" "ffmpeg can open $p"; done
fi

[ -f "$app/Contents/Resources/THIRD-PARTY-LICENSES.html" ] \
  || problem "$app/Contents/Resources/THIRD-PARTY-LICENSES.html" "is missing"

# Gatekeeper and the ticket, only for a Developer ID signature.
if [ "$mode" = developer-id ] && [ "$before" = 0 ]; then
  spctl -a -vv "$app" > /dev/null 2>&1 || problem "$app" "Gatekeeper refuses it (spctl -a -vv)"
  ticket=${dmg:-$app}
  xcrun stapler validate "$ticket" > /dev/null 2>&1 || problem "$ticket" "has no notarisation ticket (stapler validate)"
fi

if [ "$problems" -gt 0 ]; then exit 1; fi
if [ "$mode" = developer-id ] && [ "$before" = 1 ]; then echo "verified (developer-id, before notarisation)"; else echo "verified ($mode)"; fi
