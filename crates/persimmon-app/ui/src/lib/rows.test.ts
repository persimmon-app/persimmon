// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { rowToCancel, upsert, without } from "./rows";
import type { RowView } from "./types";

const r = (id: number, rev: number, state = "queued") =>
  ({ id, rev, path: "", inputs: [], name: `${id}`, from: null, to: null, op: "convert", size: 0, output: null, note: null, state }) as RowView;

describe("rows", () => {
  it("adds a new row at the end", () => {
    expect(upsert([r(1, 1)], r(2, 3)).map((x) => x.id)).toEqual([1, 2]);
  });
  it("replaces a row with a newer view", () => {
    const rows = upsert([r(1, 1)], r(1, 5, "running"));
    expect(rows[0].state).toBe("running");
  });
  it("ignores an older view, so a late event can't undo a newer one", () => {
    const rows = upsert([r(1, 9, "done")], r(1, 4, "queued"));
    expect(rows[0].state).toBe("done");
  });
  it("drops rows by id", () => {
    expect(without([r(1, 1), r(2, 1)], [1]).map((x) => x.id)).toEqual([2]);
  });
  it("rowToCancel takes the active row when it's running", () => {
    expect(rowToCancel([r(1, 1, "running"), r(2, 1, "running")], 2)?.id).toBe(2);
  });
  it("rowToCancel falls back to the first running row", () => {
    expect(rowToCancel([r(1, 1, "done"), r(2, 1, "running"), r(3, 1, "running")], 1)?.id).toBe(2);
    expect(rowToCancel([r(1, 1, "running")], null)?.id).toBe(1);
    expect(rowToCancel([r(1, 1, "done")], 1)).toBeUndefined();
  });
});
