// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import Toolbar from "./Toolbar.svelte";

function show(over: Record<string, unknown> = {}) {
  const props = {
    count: 6, totalBytes: 1_500_000_000, targets: ["jpeg", "png", "webp"], current: null,
    canStart: true, onAdd: vi.fn(), onPick: vi.fn(), onStart: vi.fn(), ...over,
  };
  render(Toolbar, props);
  return props;
}

describe("Toolbar", () => {
  it("shows the file count and total size", () => {
    show();
    expect(screen.getByText("6 files · 1.5 GB")).toBeTruthy();
  });

  it("disables Start with nothing queued", () => {
    show({ canStart: false });
    expect((screen.getByRole("button", { name: "Start" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("lists the targets it's given and reports the one picked", async () => {
    const p = show();
    const select = screen.getByLabelText("Output format") as HTMLSelectElement;
    const values = [...select.options].map((o) => o.value).filter(Boolean);
    expect(values).toEqual(["jpeg", "png", "webp"]);
    await fireEvent.change(select, { target: { value: "webp" } });
    expect(p.onPick).toHaveBeenCalledWith("webp");
  });

  it("adds files", async () => {
    const p = show();
    await fireEvent.click(screen.getByRole("button", { name: "Add files" }));
    expect(p.onAdd).toHaveBeenCalled();
  });
});
