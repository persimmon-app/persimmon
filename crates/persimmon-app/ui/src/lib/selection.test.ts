// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { clicked, moved } from "./selection";

const ids = [1, 2, 3, 4, 5];

describe("selection", () => {
  it("a plain click selects one row and anchors there", () => {
    expect(clicked(ids, { selected: [2, 3], anchor: 2 }, 4, "replace")).toEqual({ selected: [4], anchor: 4 });
  });
  it("⇧-click selects the range from the anchor", () => {
    expect(clicked(ids, { selected: [2], anchor: 2 }, 4, "range")).toEqual({ selected: [2, 3, 4], anchor: 2 });
  });
  it("⌘-click toggles one row, the macOS way", () => {
    expect(clicked(ids, { selected: [2], anchor: 2 }, 4, "toggle")).toEqual({ selected: [2, 4], anchor: 4 });
    expect(clicked(ids, { selected: [2, 4], anchor: 4 }, 2, "toggle")).toEqual({ selected: [4], anchor: 2 });
  });
  it("⇧-click with a removed anchor acts as a plain click", () => {
    expect(clicked(ids, { selected: [], anchor: 9 }, 3, "range")).toEqual({ selected: [3], anchor: 3 });
  });
  it("arrows move from the last selected row, and start at an end", () => {
    expect(moved(ids, { selected: [], anchor: null }, 1, false)).toEqual({ selected: [1], anchor: 1 });
    expect(moved(ids, { selected: [], anchor: null }, -1, false)).toEqual({ selected: [5], anchor: 5 });
    expect(moved(ids, { selected: [3], anchor: 3 }, 1, false)).toEqual({ selected: [4], anchor: 4 });
    expect(moved(ids, { selected: [5], anchor: 5 }, 1, false)).toEqual({ selected: [5], anchor: 5 });
    expect(moved(ids, { selected: [3], anchor: 3 }, 1, true)).toEqual({ selected: [3, 4], anchor: 3 });
  });
});
