#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Tests for verify.sh and mode.sh. Each case builds a small fake
# Persimmon.app (a C binary linking one @rpath dylib, script stand-ins for
# ffmpeg and ffprobe, the licence list), breaks it one way, and checks that
# verify.sh names the problem. Prints "ok <case>" or "FAIL <case>".
#
#   bash crates/persimmon-app/packaging/tests/run.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
verify=$here/../verify.sh
mode=$here/../mode.sh
ID=io.github.persimmon-app.Persimmon
work=$(mktemp -d "${TMPDIR:-/tmp}/prsm-packaging-tests.XXXXXX")
trap 'rm -rf "$work"' EXIT
failed=0

BUILDCONF="--enable-gpl --disable-network --enable-libx264 --enable-libx265 --enable-libvpx --enable-libopus --enable-libmp3lame --enable-libdav1d --enable-libaom"
ENCODERS="libx264 libx265 libvpx-vp9 libopus libmp3lame libaom-av1 aac"
PROTOCOLS="file pipe"

# A stand-in for ffmpeg or ffprobe that answers -buildconf, -encoders and
# -protocols the way the real one lays them out.
stand_in() { # path buildconf encoders protocols
  {
    echo '#!/bin/sh'
    echo 'case "$2" in'
    echo '  -buildconf) echo "  configuration:"'
    for f in $2; do echo "    echo '    $f'"; done
    echo '  ;;'
    echo '  -encoders) echo "Encoders:"'
    for e in $3; do echo "    echo ' V....D $e              $e encoder'"; done
    echo '  ;;'
    echo '  -protocols) echo "Supported file protocols:"; echo "Input:"'
    for p in $4; do echo "    echo '  $p'"; done
    echo '    echo "Output:"'
    for p in $4; do echo "    echo '  $p'"; done
    echo '  ;;'
    echo 'esac'
  } > "$1"
  chmod +x "$1"
}

# make_app <dir> [minos]: an unsigned fake app at <dir>/Persimmon.app.
make_app() {
  local app=$1/Persimmon.app minos=${2:-14.0}
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks" "$app/Contents/Resources"
  cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>persimmon-app</string>
<key>CFBundleIdentifier</key><string>$ID</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
PLIST
  clang -arch arm64 -mmacosx-version-min="$minos" -dynamiclib -install_name @rpath/libfake.1.dylib \
    -o "$app/Contents/Frameworks/libfake.1.dylib" "$here/fakelib.c"
  clang -arch arm64 -mmacosx-version-min="$minos" -o "$app/Contents/MacOS/persimmon-app" "$here/fake.c" \
    -L"$app/Contents/Frameworks" -lfake.1 -Wl,-rpath,@executable_path/../Frameworks
  stand_in "$app/Contents/MacOS/ffmpeg" "$BUILDCONF" "$ENCODERS" "$PROTOCOLS"
  stand_in "$app/Contents/MacOS/ffprobe" "$BUILDCONF" "$ENCODERS" "$PROTOCOLS"
  echo '<html></html>' > "$app/Contents/Resources/THIRD-PARTY-LICENSES.html"
}

# sign_app <app> [identifier]: inside-out, ad hoc, as bundle.sh does.
sign_app() {
  local app=$1 id=${2:-$ID} f
  for f in "$app"/Contents/Frameworks/* "$app/Contents/MacOS/ffmpeg" "$app/Contents/MacOS/ffprobe"; do
    [ -e "$f" ] && codesign --force -s - "$f" 2>/dev/null
  done
  codesign --force -s - --identifier "$id" "$app" 2>/dev/null
}

# expect <case> <want-exit> <want-text> <app>
expect() {
  local name=$1 want=$2 text=$3 out code=0
  out=$("$verify" "$4" 2>&1) || code=$?
  if [ "$code" = "$want" ] && grep -qF -- "$text" <<<"$out"; then
    echo "ok $name"
  else
    echo "FAIL $name (exit $code, wanted $want and \"$text\"):"; sed 's/^/    /' <<<"$out"; failed=1
  fi
}

case_dir() { local d=$work/$1; mkdir -p "$d"; echo "$d"; }

d=$(case_dir good); make_app "$d"; sign_app "$d/Persimmon.app"
expect good 0 "verified (ad-hoc)" "$d/Persimmon.app"

d=$(case_dir homebrew-path); make_app "$d"
install_name_tool -change @rpath/libfake.1.dylib /opt/homebrew/lib/libfake.1.dylib "$d/Persimmon.app/Contents/MacOS/persimmon-app" 2>/dev/null
sign_app "$d/Persimmon.app"
expect homebrew-path 1 "/opt/homebrew/lib/libfake.1.dylib" "$d/Persimmon.app"

d=$(case_dir missing-rpath); make_app "$d"; rm "$d/Persimmon.app/Contents/Frameworks/libfake.1.dylib"; sign_app "$d/Persimmon.app"
expect missing-rpath 1 "not in the bundle" "$d/Persimmon.app"

d=$(case_dir minos); make_app "$d" 15.0; sign_app "$d/Persimmon.app"
expect minos 1 "minos 15.0" "$d/Persimmon.app"

d=$(case_dir tampered); make_app "$d"; sign_app "$d/Persimmon.app"; echo extra > "$d/Persimmon.app/Contents/Resources/extra.txt"
expect tampered 1 "signature" "$d/Persimmon.app"

d=$(case_dir identifier); make_app "$d"; sign_app "$d/Persimmon.app" persimmon-app
expect identifier 1 "signed as persimmon-app" "$d/Persimmon.app"

d=$(case_dir network); make_app "$d"
stand_in "$d/Persimmon.app/Contents/MacOS/ffmpeg" "${BUILDCONF/--disable-network /}" "$ENCODERS" "$PROTOCOLS"; sign_app "$d/Persimmon.app"
expect network 1 "--disable-network" "$d/Persimmon.app"

d=$(case_dir encoder); make_app "$d"
stand_in "$d/Persimmon.app/Contents/MacOS/ffmpeg" "$BUILDCONF" "${ENCODERS/libx265 /}" "$PROTOCOLS"; sign_app "$d/Persimmon.app"
expect encoder 1 "libx265" "$d/Persimmon.app"

d=$(case_dir protocols); make_app "$d"
stand_in "$d/Persimmon.app/Contents/MacOS/ffmpeg" "$BUILDCONF" "$ENCODERS" "file pipe https"; sign_app "$d/Persimmon.app"
expect protocols 1 "https" "$d/Persimmon.app"

d=$(case_dir licences); make_app "$d"; rm "$d/Persimmon.app/Contents/Resources/THIRD-PARTY-LICENSES.html"; sign_app "$d/Persimmon.app"
expect licences 1 "THIRD-PARTY-LICENSES.html" "$d/Persimmon.app"

# A Developer ID app before notarisation: Gatekeeper and the ticket can't
# pass yet, so bundle.sh checks it with --before-notarisation. A fake
# bundle can't carry a Developer ID, so the mode is forced.
d=$(case_dir developer-id); make_app "$d"; sign_app "$d/Persimmon.app"
out=$(PERSIMMON_VERIFY_MODE=developer-id "$verify" "$d/Persimmon.app" 2>&1) && code=0 || code=$?
if [ "$code" = 1 ] && grep -qF "Gatekeeper refuses it" <<<"$out"; then echo "ok developer-id"
else echo "FAIL developer-id (exit $code):"; sed 's/^/    /' <<<"$out"; failed=1; fi
out=$(PERSIMMON_VERIFY_MODE=developer-id "$verify" --before-notarisation "$d/Persimmon.app" 2>&1) && code=0 || code=$?
if [ "$code" = 0 ] && [ "$out" = "verified (developer-id, before notarisation)" ]; then echo "ok before-notarisation"
else echo "FAIL before-notarisation (exit $code):"; sed 's/^/    /' <<<"$out"; failed=1; fi

# The native build refuses a folder whose path has a space (CFLAGS and
# the install-name pass would split it), at once and downloading nothing.
spaced="$work/a b"
out=$(PATH=/usr/bin:/bin bash "$here/../build-libs.sh" "$spaced" 2>&1) && code=0 || code=$?
if [ "$code" = 1 ] && grep -qF "lib.sh: the native folder's path can't contain spaces: $spaced" <<<"$out" && [ ! -e "$spaced/sources" ]; then
  echo "ok native-dir-with-space"
else
  echo "FAIL native-dir-with-space (exit $code):"; sed 's/^/    /' <<<"$out"; failed=1
fi

# bundle.sh names a missing source before building anything: without
# cargo on PATH, reaching the build would fail differently.
fake=$work/fake-native
mkdir -p "$fake/prefix/bin" "$fake/prefix/lib" "$fake/prefix/share" "$fake/sources"
ffv=$(awk '$1 == "ffmpeg" {print $2}' <<<"$(. "$here/../versions.sh"; echo "$SOURCES")")
touch "$fake/prefix/bin/ffmpeg" "$fake/prefix/bin/ffprobe" "$fake/prefix/lib/libvips.42.dylib" "$fake/prefix/share/ffmpeg-$ffv-configure.txt"
out=$(PATH=/usr/bin:/bin bash "$here/../bundle.sh" "$fake" "$work/fake-out" 2>&1) && code=0 || code=$?
if [ "$code" = 1 ] && grep -qF "bundle.sh: no downloaded source for libffi 3.8.0" <<<"$out"; then
  echo "ok missing-source"
else
  echo "FAIL missing-source (exit $code):"; sed 's/^/    /' <<<"$out"; failed=1
fi

# mode.sh: which signing steps the release workflow runs.
SECRETS="APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID"
# A success prints the mode on stdout; a failure prints nothing there (CI
# captures stdout) and names what's missing on stderr, where the log shows it.
mode_case() { # name want-exit want-text VAR...
  local name=$1 want=$2 text=$3 out err code=0 v; shift 3
  local args=()
  for v in $SECRETS; do args+=(-u "$v"); done
  for v in "$@"; do args+=("$v=x"); done
  out=$(env "${args[@]}" bash "$mode" 2>"$work/mode.err") || code=$?
  err=$(cat "$work/mode.err")
  if [ "$want" = 0 ]; then got=$out; else got=$err; [ -z "$out" ] || got="stdout: $out"; fi
  if [ "$code" = "$want" ] && [ "$got" = "$text" ]; then echo "ok mode-$name"
  else echo "FAIL mode-$name (exit $code, got \"$got\"; wanted $want and \"$text\")"; failed=1; fi
}
NOTARY="APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID"
mode_case none 0 "ad-hoc"
mode_case all 0 "notarise" $SECRETS
# Signing without notarising would ship a DMG Gatekeeper still blocks.
mode_case signing-only 1 "mode: missing $NOTARY" APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY
mode_case partial 1 "mode: missing APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY $NOTARY" APPLE_CERTIFICATE
mode_case notarise-only 1 "mode: missing APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY" $NOTARY

exit $failed
