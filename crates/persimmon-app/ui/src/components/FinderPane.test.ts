// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import FinderPane from "./FinderPane.svelte";

describe("FinderPane", () => {
  it("says where the two Services are and what each does", () => {
    render(FinderPane, { onShortcuts: vi.fn(), wheel: false, onWheel: vi.fn() });
    expect(screen.getByRole("heading", { name: "Finder & menus" })).toBeTruthy();
    for (const text of [
      "In Finder, select files or folders, then right-click → Services → Convert with Persimmon or Open in Persimmon.",
      "Shows the format wheel at the pointer, for the selected files.",
      "Adds the files to the queue and brings the window forward.",
      "To show or hide them: System Settings → Keyboard → Keyboard Shortcuts → Services.",
    ]) {
      expect(screen.getByText(text), text).toBeTruthy();
    }
    expect(screen.getByText("Convert with Persimmon", { selector: "dt" })).toBeTruthy();
    expect(screen.getByText("Open in Persimmon", { selector: "dt" })).toBeTruthy();
  });

  it("opens Keyboard Shortcuts", async () => {
    const onShortcuts = vi.fn();
    render(FinderPane, { onShortcuts, wheel: false, onWheel: vi.fn() });
    await fireEvent.click(screen.getByRole("button", { name: "Open Keyboard Shortcuts…" }));
    expect(onShortcuts).toHaveBeenCalled();
  });

  it("has the wheel's switch, with what it watches for", async () => {
    const onWheel = vi.fn();
    const { rerender } = render(FinderPane, { onShortcuts: vi.fn(), wheel: false, onWheel });
    expect(screen.getByText("The format wheel")).toBeTruthy();
    expect(screen.getByText("Hold ⇧ while dragging files anywhere to pick a format at the pointer. It watches only for that, and asks for no permission.")).toBeTruthy();
    const sw = screen.getByRole("switch", { name: "The format wheel" });
    expect(sw.getAttribute("aria-checked")).toBe("false");
    await fireEvent.click(sw);
    expect(onWheel).toHaveBeenCalledWith(true);
    await rerender({ wheel: true });
    expect(screen.getByRole("switch", { name: "The format wheel" }).getAttribute("aria-checked")).toBe("true");
    await fireEvent.click(screen.getByRole("switch", { name: "The format wheel" }));
    expect(onWheel).toHaveBeenLastCalledWith(false);
  });
});
