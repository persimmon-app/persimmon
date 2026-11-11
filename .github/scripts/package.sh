#!/usr/bin/env bash
# Package one release archive (DD-30).
#
#   package.sh <target> <version> <bin-dir> <out-dir>
#
# Writes <out-dir>/prsm-<version>-<target>.tar.gz (.zip for Windows) and a
# .sha256 file beside it. The archive holds one folder,
# prsm-<version>-<target>/, with the binary, README.md, LICENSE,
# THIRD-PARTY.md, CHANGELOG.md and THIRD-PARTY-LICENSES.html. The licence
# inventory must already be at <bin-dir>/THIRD-PARTY-LICENSES.html.
#
# Uses only what every runner has: tar on macOS and Linux, 7z on Windows
# (Git Bash has no zip), and sha256sum or shasum.
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: package.sh <target> <version> <bin-dir> <out-dir>" >&2
  exit 2
fi
target=$1 version=$2 bin_dir=$3 out_dir=$4
root=$(cd "$(dirname "$0")/../.." && pwd)
name="prsm-$version-$target"

case "$target" in
  *windows*) exe=prsm.exe ;;
  *) exe=prsm ;;
esac

mkdir -p "$out_dir"
out_dir=$(cd "$out_dir" && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/$name"

for f in "$bin_dir/$exe" "$bin_dir/THIRD-PARTY-LICENSES.html" \
         "$root/README.md" "$root/LICENSE" "$root/THIRD-PARTY.md" "$root/CHANGELOG.md"; do
  if [[ ! -f "$f" ]]; then
    echo "package.sh: missing $f" >&2
    exit 1
  fi
  cp "$f" "$stage/$name/"
done

cd "$stage"
case "$target" in
  *windows*)
    archive="$name.zip"
    7z a -tzip -bso0 -bsp0 "$out_dir/$archive" "$name" ;;
  *)
    archive="$name.tar.gz"
    # No macOS extended attributes or ._ files in the archive.
    COPYFILE_DISABLE=1 tar -czf "$out_dir/$archive" "$name" ;;
esac

cd "$out_dir"
if command -v sha256sum >/dev/null; then
  sha256sum "$archive" > "$archive.sha256"
else
  shasum -a 256 "$archive" > "$archive.sha256"
fi
echo "$out_dir/$archive"
