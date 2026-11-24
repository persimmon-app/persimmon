// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import FirstRun from "./FirstRun.svelte";

describe("FirstRun", () => {
  it("introduces the wheel and the three ways in", () => {
    render(FirstRun, { onWheel: vi.fn(), onDismiss: vi.fn() });
    expect(screen.getByText("Hold ⇧ while dragging a file in Finder and pick a format at the cursor")).toBeTruthy();
    for (const way of ["⇧ Drag & hold", "Right-click → Services → Convert with Persimmon", "Terminal"]) {
      expect(screen.getByText(way), way).toBeTruthy();
    }
    expect(screen.getByRole("button", { name: "Show me" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Not now" })).toBeTruthy();
  });

  it("Show me turns the wheel on and says so", async () => {
    const onWheel = vi.fn();
    const onDismiss = vi.fn();
    render(FirstRun, { onWheel, onDismiss });
    await fireEvent.click(screen.getByRole("button", { name: "Show me" }));
    expect(onWheel).toHaveBeenCalledWith(true);
    expect(onDismiss).toHaveBeenCalled();
    expect(screen.getByText("The wheel is on. Hold ⇧ while dragging files from Finder.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Show me" })).toBeNull();
  });

  it("Not now only dismisses, and the card goes", async () => {
    const onWheel = vi.fn();
    const onDismiss = vi.fn();
    render(FirstRun, { onWheel, onDismiss });
    await fireEvent.click(screen.getByRole("button", { name: "Not now" }));
    expect(onDismiss).toHaveBeenCalled();
    expect(onWheel).not.toHaveBeenCalled();
    expect(screen.queryByText("Hold ⇧ while dragging a file in Finder and pick a format at the cursor")).toBeNull();
  });
});
