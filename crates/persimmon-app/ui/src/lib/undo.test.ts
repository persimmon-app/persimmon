// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { lastUndoable, undoMessage } from "./undo";
import { batch } from "../test/fixtures";

const LEFT = "a.jpg was changed or moved since it was converted, so it was left alone";

describe("undo", () => {
  it("counts what went to the Trash", () => {
    expect(undoMessage({ trashed: 2, restored: 0, skipped: [] })).toBe("Moved 2 files to the Trash");
  });

  it("names the one file it left alone, and why", () => {
    expect(undoMessage({ trashed: 1, restored: 0, skipped: [LEFT] })).toBe(`Moved 1 file to the Trash; ${LEFT}`);
  });

  it("points to History when it left several alone", () => {
    expect(undoMessage({ trashed: 0, restored: 0, skipped: [LEFT, LEFT] })).toBe("Moved 0 files to the Trash; 2 left alone (see History)");
  });

  it("offers only the newest batch", () => {
    expect(lastUndoable([batch({ id: "new", undoable: false }), batch({ id: "old", undoable: true })])).toBe(false);
    expect(lastUndoable([batch({ id: "new", undoable: true })])).toBe(true);
    expect(lastUndoable([])).toBe(false);
  });

  it("never offers a watch batch, which History undoes", () => {
    expect(lastUndoable([batch({ id: "w", source: "watch", undoable: true }), batch({ id: "mine", undoable: true })])).toBe(true);
    expect(lastUndoable([batch({ id: "w", source: "watch", undoable: true }), batch({ id: "mine", undoable: false })])).toBe(false);
    expect(lastUndoable([batch({ id: "w", source: "watch", undoable: true })])).toBe(false);
  });

  it("counts the originals it put back", () => {
    expect(undoMessage({ trashed: 2, restored: 2, skipped: [] })).toBe("Moved 2 files to the Trash and put 2 originals back");
    expect(undoMessage({ trashed: 1, restored: 1, skipped: [] })).toBe("Moved 1 file to the Trash and put 1 original back");
    expect(undoMessage({ trashed: 0, restored: 0, skipped: [] })).toBe("Moved 0 files to the Trash");
  });
});
