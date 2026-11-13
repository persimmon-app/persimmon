// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import HistoryView from "./HistoryView.svelte";
import { NOW, batch } from "../test/fixtures";

function show(over = {}) {
  const h = { onSearch: vi.fn(), onFilter: vi.fn(), onUndo: vi.fn(), onReveal: vi.fn(), onClear: vi.fn(), onDismissReport: vi.fn() };
  const batches = [
    batch({ id: "today" }),
    batch({ id: "yesterday", finished: NOW - 86_400, started: NOW - 86_500, undoable: false, undone: true }),
    batch({ id: "older", finished: NOW - 3 * 86_400, started: NOW - 3 * 86_400 - 60, source: "drop" }),
  ];
  render(HistoryView, { batches, text: "", filter: "all", report: null, now: NOW, ...h, ...over });
  return h;
}

describe("HistoryView", () => {
  it("groups batches by day", () => {
    show();
    expect(screen.getByText("TODAY")).toBeTruthy();
    expect(screen.getByText("YESTERDAY")).toBeTruthy();
    expect(screen.getByText("9 NOVEMBER")).toBeTruthy();
    expect(screen.getByText("6 files · 30 days")).toBeTruthy();
  });

  it("searches and filters", async () => {
    const h = show();
    await fireEvent.input(screen.getByLabelText("Search history"), { target: { value: "hero" } });
    expect(h.onSearch).toHaveBeenCalledWith("hero");
    await fireEvent.click(screen.getByRole("button", { name: "Failed" }));
    expect(h.onFilter).toHaveBeenCalledWith("failed");
  });

  it("offers Undo only where it can still undo", () => {
    show();
    expect(screen.getAllByRole("button", { name: /^Undo batch/ })).toHaveLength(2);
    expect(screen.getByText("Undone")).toBeTruthy();
  });

  it("asks before clearing, and says what undo does", async () => {
    const h = show();
    expect(screen.getByText("Undo moves the files a batch made to the Trash")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Clear history…" }));
    expect(h.onClear).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(h.onClear).toHaveBeenCalled();
  });

  it("lists what undo left alone", () => {
    show({ report: { trashed: 1, skipped: ["hero.webp was changed or moved since it was converted, so it was left alone"] } });
    expect(screen.getByText(/hero\.webp was changed or moved/)).toBeTruthy();
  });
});
