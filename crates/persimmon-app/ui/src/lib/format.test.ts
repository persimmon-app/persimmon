// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { alreadyMessage, target } from "./format";
import type { RowView } from "./types";

const row = (over: Partial<RowView>) => ({ op: "convert", from: "png", to: "jpeg", ...over }) as RowView;

describe("format", () => {
  it("a Compress row keeps its format", () => {
    expect(target(row({ op: "compress", from: "png", to: null }))).toBe("PNG");
  });
  it("names the target, or EXTRACT", () => {
    expect(target(row({}))).toBe("JPEG");
    expect(target(row({ op: "extract", to: null }))).toBe("EXTRACT");
  });
  it("says how many files were already queued", () => {
    expect(alreadyMessage(1)).toBe("1 file already in the queue");
    expect(alreadyMessage(2)).toBe("2 files already in the queue");
  });
});
