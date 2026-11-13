#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# The macOS app's licence list, Contents/Resources/THIRD-PARTY-LICENSES.html:
# every Rust crate compiled in (cargo-about, accepting what about.toml
# accepts), every bundled native library and FFmpeg (their licence files,
# kept by build-libs.sh and build-ffmpeg.sh), and every npm package in the
# built interface (ui/dist/bundled-packages.json, written by the Vite build).
#
#   licences.sh <native-dir> <out.html>
set -euo pipefail
if [ $# -ne 2 ]; then echo "usage: licences.sh <native-dir> <out.html>" >&2; exit 2; fi
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
native=$(cd "$1" && pwd) out=$2
ui=$root/crates/persimmon-app/ui
[ -f "$ui/dist/bundled-packages.json" ] || { echo "licences.sh: build the interface first (npm run build)" >&2; exit 1; }
crates=$(mktemp)
trap 'rm -f "$crates"' EXIT
(cd "$root" && cargo about generate --locked -c about.toml -m crates/persimmon-app/Cargo.toml \
  --target aarch64-apple-darwin crates/persimmon-app/packaging/about-app.hbs -o "$crates")
python3 -I - "$crates" "$here/versions.sh" "$native/prefix/share/licenses" "$ui" "$out" <<'PY'
import html, json, pathlib, re, sys
crates, versions, licences, ui, out = (pathlib.Path(a) for a in sys.argv[1:])
page = crates.read_text()
if "<!-- more -->" not in page:
    sys.exit("licences.sh: the template lost its <!-- more --> marker")
known = {}
for line in versions.read_text().splitlines():
    f = line.split()
    if len(f) == 4 and f[2].startswith("http"):
        known[f[0]] = f[1]
def block(title, texts):
    return f"<h3>{html.escape(title)}</h3>\n" + "".join(f"<pre>{html.escape(t)}</pre>\n" for t in texts)
parts = ["<h2>Bundled libraries and FFmpeg</h2>\n"]
for d in sorted(p for p in licences.iterdir() if p.is_dir()):
    files = sorted(f for f in d.iterdir() if f.is_file())
    if not files:
        sys.exit(f"licences.sh: no licence text for {d.name}")
    parts.append(block(f"{d.name} {known.get(d.name, '')}".strip(),
                       [f.read_text(errors="replace") for f in files]))
parts.append("<h2>Interface</h2>\n")
for name in json.loads((ui / "dist/bundled-packages.json").read_text()):
    pkg = ui / "node_modules" / name
    meta = json.loads((pkg / "package.json").read_text())
    texts = [f.read_text(errors="replace") for f in sorted(pkg.iterdir())
             if f.is_file() and re.match(r"(?i)licen[cs]e|copying", f.name)]
    if not texts:
        sys.exit(f"licences.sh: {name} ships no licence file")
    parts.append(block(f"{name} {meta.get('version', '')} ({meta.get('license', '?')})", texts))
pathlib.Path(out).write_text(page.replace("<!-- more -->", "".join(parts)))
PY
echo "licences: $out"
