// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import CombineRow from "./CombineRow.svelte";
import type { RowView } from "../lib/types";

const row = {
  id: 7, path: "/x/a.png", inputs: ["/x/a.png", "/x/b.png", "/x/c.pdf"], name: "merged.pdf",
  from: null, to: "pdf", op: "merge", size: 3000, output: "/x/merged.pdf", note: null, rev: 1, state: "queued",
} as RowView;

function show() {
  const h = { onCancel: vi.fn(), onRetry: vi.fn(), onReveal: vi.fn(), onSelect: vi.fn(), onReorder: vi.fn(), onRemoveInput: vi.fn() };
  render(CombineRow, { row, selected: false, ...h });
  return h;
}

describe("CombineRow", () => {
  it("says how many files it combines and into what", () => {
    show();
    expect(screen.getByText("3 FILES")).toBeTruthy();
    expect(screen.getByText("merged.pdf")).toBeTruthy();
  });

  it("lists its inputs in order when expanded", async () => {
    show();
    const toggle = screen.getByRole("button", { name: "Show the 3 files in merged.pdf" });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    await fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    const items = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(items[0]).toContain("a.png");
    expect(items[2]).toContain("c.pdf");
  });

  it("reorders with ⌥↓ and removes an input", async () => {
    const h = show();
    await fireEvent.click(screen.getByRole("button", { name: "Show the 3 files in merged.pdf" }));
    await fireEvent.keyDown(screen.getByRole("button", { name: /^Move a\.png/ }), { key: "ArrowDown", altKey: true });
    expect(h.onReorder).toHaveBeenCalledWith(7, 0, 1);
    await fireEvent.click(screen.getByRole("button", { name: "Remove b.png" }));
    expect(h.onRemoveInput).toHaveBeenCalledWith(7, 1);
  });

  it("moves an input with ⌥↓ without moving the queue's selection", async () => {
    const onReorder = vi.fn();
    const parent = vi.fn();
    render(CombineRow, { row, selected: false, active: true, onCancel() {}, onRetry() {}, onReveal() {}, onSelect() {}, onReorder, onRemoveInput() {} });
    window.addEventListener("keydown", parent);
    await fireEvent.click(screen.getByRole("button", { name: /Show the/ }));
    const handle = screen.getAllByRole("button", { name: /^Move / })[0];
    await fireEvent.keyDown(handle, { key: "ArrowDown", altKey: true });
    window.removeEventListener("keydown", parent);
    expect(onReorder).toHaveBeenCalledTimes(1);
    expect(parent).not.toHaveBeenCalled();
  });

  it("picks an input up with Enter and puts it down with Enter", async () => {
    render(CombineRow, { row, selected: false, active: true, onCancel() {}, onRetry() {}, onReveal() {}, onSelect() {}, onReorder() {}, onRemoveInput() {} });
    await fireEvent.click(screen.getByRole("button", { name: /Show the/ }));
    const handle = screen.getAllByRole("button", { name: /^Move / })[0];
    await fireEvent.keyDown(handle, { key: "Enter" });
    expect(handle.getAttribute("aria-pressed")).toBe("true");
    await fireEvent.keyDown(handle, { key: "Enter" });
    expect(handle.getAttribute("aria-pressed")).toBe("false");
  });
});
