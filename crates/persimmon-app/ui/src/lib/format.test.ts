// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { alreadyMessage, noteMessage, skippedMessage, target } from "./format";
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
  it("puts an add's notes in one message", () => {
    expect(noteMessage({ already: 2, skipped: null })).toBe("2 files already in the queue");
    expect(noteMessage({ already: 0, skipped: { count: 1, why: { kind: "convert" } } })).toBe("1 file skipped: can't be converted");
    expect(noteMessage({ already: 2, skipped: { count: 1, why: { kind: "convert" } } })).toBe(
      "2 files already in the queue · 1 file skipped: can't be converted",
    );
  });
  it("says what a dropped folder left out, and why", () => {
    expect(skippedMessage({ count: 1, why: { kind: "convert" } })).toBe("1 file skipped: can't be converted");
    expect(skippedMessage({ count: 3, why: { kind: "become", name: "WebP" } })).toBe("3 files skipped: can't become WebP");
    expect(skippedMessage({ count: 2, why: { kind: "compress" } })).toBe("2 files skipped: can't be compressed");
  });
});
