// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import FinderPane from "./FinderPane.svelte";

describe("FinderPane", () => {
  it("says where the two Services are and what each does", () => {
    render(FinderPane, { onShortcuts: vi.fn() });
    expect(screen.getByRole("heading", { name: "Finder & menus" })).toBeTruthy();
    for (const text of [
      "In Finder, select files or folders, then right-click → Services → Convert with Persimmon or Open in Persimmon.",
      "Converts the files at once, with the mode or preset you use now.",
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
    render(FinderPane, { onShortcuts });
    await fireEvent.click(screen.getByRole("button", { name: "Open Keyboard Shortcuts…" }));
    expect(onShortcuts).toHaveBeenCalled();
  });
});
