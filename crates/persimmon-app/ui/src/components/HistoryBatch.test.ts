// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import HistoryBatch from "./HistoryBatch.svelte";
import { batch } from "../test/fixtures";

describe("HistoryBatch", () => {
  it("summarises the batch and expands to its files", async () => {
    const onReveal = vi.fn();
    const onUndo = vi.fn();
    render(HistoryBatch, { batch: batch({ id: "a", source: "drop" }), onReveal, onUndo });
    expect(screen.getByText("2 files → WEBP")).toBeTruthy();
    expect(screen.getByText("DROP")).toBeTruthy();
    expect(screen.getByText(/1 done, 1 failed · 3 min 12 s/)).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: /Show the 2 files/ }));
    expect(screen.getByText("hero.png → hero.webp")).toBeTruthy();
    expect(screen.getByText(/it's damaged/)).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Reveal hero.webp in Finder" }));
    expect(onReveal).toHaveBeenCalledWith("/x/hero.webp");
    await fireEvent.click(screen.getByRole("button", { name: /^Undo batch/ }));
    expect(onUndo).toHaveBeenCalledWith("a");
  });

  it("offers Retry on failed and cancelled files only", async () => {
    const onRetry = vi.fn();
    const b = batch({ id: "a" });
    b.items.push({ inputs: ["/x/late.mov"], output: null, outputSize: null, outputMtime: null, size: 9, undone: false, outcome: "cancelled" });
    render(HistoryBatch, { batch: b, onReveal: vi.fn(), onUndo: vi.fn(), onRetry });
    await fireEvent.click(screen.getByRole("button", { name: /Show the 3 files/ }));
    expect(screen.queryByRole("button", { name: "Retry hero.png" })).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Retry scan.png" }));
    expect(onRetry).toHaveBeenCalledWith("a", 1);
    await fireEvent.click(screen.getByRole("button", { name: "Retry late.mov" }));
    expect(onRetry).toHaveBeenCalledWith("a", 2);
  });
});
