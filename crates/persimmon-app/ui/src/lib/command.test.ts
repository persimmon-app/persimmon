// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { commandLine } from "./command";

const none = { quality: null, preset: null, pages: null, maxSize: null, stripMetadata: false } as const;

describe("commandLine", () => {
  it("matches presets.rs for the built-ins and the edge cases", () => {
    expect(commandLine({ mode: "convert", target: "webp", options: { ...none, quality: "medium", maxSize: 2048, stripMetadata: true } }))
      .toBe("prsm convert <files> --to webp --quality medium --max-size 2048 --strip-metadata");
    expect(commandLine({ mode: "convert", target: "mp4", options: { ...none, preset: "compatible", quality: "max" } }))
      .toBe("prsm convert <files> --to mp4 --preset compatible --quality max");
    expect(commandLine({ mode: "combine", target: "tar.gz", options: none })).toBe("prsm merge <files> --to tar.gz");
    expect(commandLine({ mode: "compress", target: null, options: none })).toBe("prsm compress <files>");
  });
});
