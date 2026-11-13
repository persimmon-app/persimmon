// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { lastUndoable, undoMessage } from "./undo";
import { batch } from "../test/fixtures";

const LEFT = "a.jpg was changed or moved since it was converted, so it was left alone";

describe("undo", () => {
  it("counts what went to the Trash", () => {
    expect(undoMessage({ trashed: 2, skipped: [] })).toBe("Moved 2 files to the Trash");
  });

  it("names the one file it left alone, and why", () => {
    expect(undoMessage({ trashed: 1, skipped: [LEFT] })).toBe(`Moved 1 file to the Trash; ${LEFT}`);
  });

  it("points to History when it left several alone", () => {
    expect(undoMessage({ trashed: 0, skipped: [LEFT, LEFT] })).toBe("Moved 0 files to the Trash; 2 left alone (see History)");
  });

  it("offers only the newest batch", () => {
    expect(lastUndoable([batch({ id: "new", undoable: false }), batch({ id: "old", undoable: true })])).toBe(false);
    expect(lastUndoable([batch({ id: "new", undoable: true })])).toBe(true);
    expect(lastUndoable([])).toBe(false);
  });
});
