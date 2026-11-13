// SPDX-License-Identifier: GPL-3.0-or-later
// Which npm packages end up in the built interface, for the app's licence
// list: the Vite build writes them to dist/bundled-packages.json.
import type { Plugin } from "vite";

/** The npm packages (scoped names whole) of these module IDs, innermost
 * install first, sorted and once each. Vite's virtual modules (`\0…`)
 * aren't packages. */
export function packagesOf(moduleIds: string[]): string[] {
  const names = new Set<string>();
  for (const id of moduleIds) {
    if (id.startsWith("\0")) continue;
    const parts = id.split("?")[0].split("/node_modules/");
    if (parts.length < 2) continue;
    const rest = parts[parts.length - 1].split("/");
    names.add(rest[0].startsWith("@") ? `${rest[0]}/${rest[1]}` : rest[0]);
  }
  return [...names].sort();
}

/** Writes dist/bundled-packages.json: every package whose code is in a
 * chunk, or whose files (fonts) are assets of the build. */
export function bundledPackages(): Plugin {
  return {
    name: "persimmon-bundled-packages",
    apply: "build",
    generateBundle(_options, bundle) {
      const ids: string[] = [];
      for (const out of Object.values(bundle)) {
        if (out.type === "chunk") ids.push(...Object.keys(out.modules));
        else ids.push(...(out.originalFileNames ?? []));
      }
      this.emitFile({ type: "asset", fileName: "bundled-packages.json", source: JSON.stringify(packagesOf(ids), null, 2) + "\n" });
    },
  };
}
