// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { packagesOf } from "./bundled";

describe("packagesOf", () => {
  it("names each npm package whose code is in the bundle, once, sorted", () => {
    expect(packagesOf([
      "/x/node_modules/svelte/src/internal/client/index.js",
      "/x/node_modules/@tauri-apps/api/core.js",
      "/x/src/App.svelte",
      "/x/node_modules/svelte/src/index.js",
    ])).toEqual(["@tauri-apps/api", "svelte"]);
  });

  it("takes the innermost package of a nested install", () => {
    expect(packagesOf(["/x/node_modules/a/node_modules/b/x.js"])).toEqual(["b"]);
  });

  it("ignores Vite's virtual modules", () => {
    expect(packagesOf(["\0vite/preload-helper.js", "\0/x/node_modules/c/y.js?commonjs"])).toEqual([]);
  });
});
