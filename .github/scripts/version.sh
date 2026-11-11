#!/usr/bin/env bash
# Print the workspace version. Given a tag, fail unless it is v<version>,
# so a mistyped tag never builds a release.
#
#   version.sh [tag]
set -euo pipefail
cd "$(dirname "$0")/../.."
version=$(cargo metadata --no-deps --format-version 1 --locked \
  | jq -r '.packages[] | select(.name == "persimmon-cli") | .version')
if [[ -z "$version" ]]; then
  echo "version.sh: no persimmon-cli package found" >&2
  exit 1
fi
if [[ $# -gt 0 && "$1" != "v$version" ]]; then
  echo "version.sh: tag $1 doesn't match the workspace version $version (expected v$version)" >&2
  exit 1
fi
echo "$version"
