#!/usr/bin/env bash
# Tests for inventory.sh: today's tree passes, and each kind of drift
# fails naming what drifted.
#
#   inventory-test.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
cargo metadata --no-deps --format-version 1 --locked --manifest-path "$repo/Cargo.toml" > "$scratch/metadata.json"
export INVENTORY_METADATA="$scratch/metadata.json"
fails=0

# A copy of the three files the check reads, changed by `edit`.
case_dir() {
  local d="$scratch/$1"
  mkdir -p "$d"
  cp "$repo/THIRD-PARTY.md" "$repo/about.toml" "$repo/deny.toml" "$d/"
  echo "$d"
}

expect() {
  local name=$1 dir=$2 code=$3 pattern=$4 out got
  set +e
  out=$(bash "$here/inventory.sh" "$dir" 2>&1)
  got=$?
  set -e
  if [[ $got -ne $code ]] || ! grep -q -- "$pattern" <<<"$out"; then
    echo "FAIL $name: exit $got (wanted $code), output:"
    echo "$out" | sed 's/^/  /'
    fails=$((fails + 1))
  else
    echo "ok   $name"
  fi
}

expect "today's tree" "$repo" 0 "inventory ok"

d=$(case_dir no-row)
python3 -I -c 'import sys; p=sys.argv[1]; s=open(p).read().splitlines(True); open(p,"w").writelines(l for l in s if not l.startswith("| [clap]"))' "$d/THIRD-PARTY.md"
expect "a crate with no row" "$d" 1 "inventory: clap (persimmon-cli) has no row"

d=$(case_dir no-exception)
python3 -I -c 'import sys; p=sys.argv[1]; s=open(p).read(); i=s.index("[notify]"); open(p,"w").write(s[:i])' "$d/about.toml"
expect "about.toml without notify's exception" "$d" 1 "inventory: deny.toml allows \['CC0-1.0'\] for notify"

d=$(case_dir extra-licence)
python3 -I -c 'import sys; p=sys.argv[1]; s=open(p).read(); open(p,"w").write(s.replace("accepted = [", "accepted = [\n    \"WTFPL\",", 1))' "$d/about.toml"
expect "a licence only about.toml accepts" "$d" 1 "inventory: WTFPL is accepted by about.toml but not allowed by deny.toml"

if [[ $fails -gt 0 ]]; then
  echo "$fails failed"
  exit 1
fi
echo "all passed"
