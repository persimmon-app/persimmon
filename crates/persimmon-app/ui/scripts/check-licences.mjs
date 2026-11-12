// SPDX-License-Identifier: GPL-3.0-or-later
// The npm counterpart of deny.toml: every installed package must carry a
// licence compatible with GPL-3.0-or-later. Widening this list is the
// owner's decision.
import { readFileSync } from "node:fs";

const ALLOWED = new Set([
  "MIT", "ISC", "BSD-2-Clause", "BSD-3-Clause", "Apache-2.0", "0BSD",
  "OFL-1.1", "CC0-1.0", "BlueOak-1.0.0", "MPL-2.0",
]);

// "A OR B" passes if either does; "A AND B" only if both do.
function allowed(expr) {
  const e = expr.trim().replace(/^\((.*)\)$/, "$1");
  if (e.includes(" OR ")) return e.split(" OR ").some(allowed);
  if (e.includes(" AND ")) return e.split(" AND ").every(allowed);
  return ALLOWED.has(e);
}

const lock = JSON.parse(readFileSync(new URL("../package-lock.json", import.meta.url)));
const bad = [];
for (const [path, pkg] of Object.entries(lock.packages)) {
  if (path === "") continue;
  const licence = pkg.license;
  if (!licence || !allowed(licence)) bad.push(`${path}: ${licence ?? "no licence"}`);
}
if (bad.length) {
  console.error(`licences not allowed:\n  ${bad.join("\n  ")}`);
  process.exit(1);
}
console.log(`licences ok (${Object.keys(lock.packages).length - 1} packages)`);
