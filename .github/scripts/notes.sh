#!/usr/bin/env bash
# Print the CHANGELOG.md section for one version, without its heading: the
# release notes.
#
#   notes.sh <version>
set -euo pipefail
cd "$(dirname "$0")/../.."
notes=$(awk -v v="$1" '
  index($0, "## [" v "]") == 1 { on = 1; next }
  on && /^## \[/ { exit }
  on { print }
' CHANGELOG.md)
if [[ -z "${notes//[[:space:]]/}" ]]; then
  echo "notes.sh: no CHANGELOG.md section for $1" >&2
  exit 1
fi
printf '%s\n' "$notes"
