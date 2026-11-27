#!/usr/bin/env bash
# The licence inventory's two promises, checked:
#
#  1. THIRD-PARTY.md has a row for every direct dependency (normal and
#     build, not dev) of the crates Persimmon ships: persimmon-core,
#     persimmon-cli, persimmon-app and persimmon-mac. A row lists a crate by
#     its first cell's link text, or by naming it in backticks there.
#  2. about.toml, which writes the licence list shipped with each release,
#     accepts exactly what deny.toml allows, with the same per-crate
#     exceptions.
#
#   inventory.sh [<repo root>]
#
# Prints "inventory ok", or one "inventory: <problem>" line per problem and
# exits 1. INVENTORY_METADATA names a saved `cargo metadata --no-deps`
# output to read instead of running cargo (the script's tests use it).
set -euo pipefail
root=${1:-"$(dirname "$0")/../.."}
cd "$root"
if [[ -n "${INVENTORY_METADATA:-}" ]]; then
  metadata=$(cat "$INVENTORY_METADATA")
else
  metadata=$(cargo metadata --no-deps --format-version 1 --locked)
fi
printf '%s' "$metadata" | python3 -I -c '
import json, re, sys, tomllib

SHIPPED = {"persimmon-core", "persimmon-cli", "persimmon-app", "persimmon-mac"}
problems = []

meta = json.load(sys.stdin)
workspace = {p["name"] for p in meta["packages"]}
deps = set()
for p in meta["packages"]:
    if p["name"] in SHIPPED:
        for d in p["dependencies"]:
            if d["kind"] != "dev" and d["name"] not in workspace:
                deps.add((d["name"], p["name"]))

listed = set()
for line in open("THIRD-PARTY.md", encoding="utf-8"):
    if not line.startswith("| "):
        continue
    first = line.split("|")[1]
    listed.update(m.lower() for m in re.findall(r"^\s*\[([^\]]+)\]", first))
    listed.update(re.findall(r"`([^`]+)`", first))
for name, user in sorted(deps):
    if name not in listed and name.lower() not in listed:
        problems.append(f"{name} ({user}) has no row in THIRD-PARTY.md")

deny = tomllib.load(open("deny.toml", "rb"))
about = tomllib.load(open("about.toml", "rb"))
allowed = set(deny["licenses"]["allow"])
accepted = set(about["accepted"])
for l in sorted(allowed - accepted):
    problems.append(f"{l} is allowed by deny.toml but not accepted by about.toml")
for l in sorted(accepted - allowed):
    problems.append(f"{l} is accepted by about.toml but not allowed by deny.toml")
deny_ex = {e["crate"]: set(e["allow"]) for e in deny["licenses"].get("exceptions", [])}
about_ex = {k: set(v["accepted"]) for k, v in about.items()
            if isinstance(v, dict) and "accepted" in v}
for crate in sorted(set(deny_ex) | set(about_ex)):
    d, a = deny_ex.get(crate), about_ex.get(crate)
    if d is None:
        problems.append(f"about.toml accepts {sorted(a)} for {crate}, which deny.toml has no exception for")
    elif a is None:
        problems.append(f"deny.toml allows {sorted(d)} for {crate}, which about.toml has no [{crate}] table for")
    elif a != d:
        problems.append(f"{crate}: deny.toml allows {sorted(d)} but about.toml accepts {sorted(a)}")

for p in problems:
    print(f"inventory: {p}")
if problems:
    sys.exit(1)
print("inventory ok")
'
