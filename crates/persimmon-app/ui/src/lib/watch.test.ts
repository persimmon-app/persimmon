// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { failedLine, midnight, statusLine, summary } from "./watch";
import { view } from "../test/fixtures";

describe("watch lines", () => {
  it("says what a rule is doing", () => {
    expect(statusLine(view({ today: 24 }))).toBe("24 files today");
    expect(statusLine(view({ today: 1 }))).toBe("1 file today");
    expect(statusLine(view())).toBe("No files today");
    expect(statusLine(view({}, { state: "converting", done: 2, total: 7 } as never))).toBe("Converting 3 of 7…");
    expect(statusLine(view({}, { state: "converting", done: 7, total: 7 } as never))).toBe("Converting 7 of 7…");
    expect(statusLine(view({}, { state: "missing" }))).toBe("Folder not found — waiting for it");
    expect(statusLine(view({}, { state: "unreadable", error: { code: "UNSUPPORTED_INPUT", file: null, reason: "Persimmon can't read this folder", fix: "Allow…" } } as never)))
      .toBe("Persimmon can't read this folder");
    expect(statusLine(view({ enabled: false }, { state: "paused" }))).toBe("Paused");
  });

  it("names the last failure", () => {
    expect(failedLine(view())).toBeNull();
    expect(failedLine(view({}, { lastFailed: { name: "scan-0231.jpg", reason: "it's damaged" } }))).toBe("Last failed: scan-0231.jpg — it's damaged");
  });

  it("sums up every rule", () => {
    expect(summary([])).toBe("No rules yet");
    expect(summary([view({ today: 1 })])).toBe("1 active · 1 file today");
    expect(summary([view({ today: 20 }), view({ id: "r2", today: 4 }), view({ id: "r3", enabled: false, today: 3 })])).toBe("2 active · 27 files today");
    expect(summary([view({ enabled: false })])).toBe("0 active · no files today");
  });

  it("finds local midnight", () => {
    const at = new Date(2026, 10, 24, 15, 30, 12);
    expect(midnight(at)).toBe(new Date(2026, 10, 24, 0, 0, 0).getTime() / 1000);
  });
});
