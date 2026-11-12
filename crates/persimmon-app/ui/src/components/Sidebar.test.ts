// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import Sidebar from "./Sidebar.svelte";

describe("Sidebar", () => {
  it("marks the current mode and switches on click", async () => {
    const onMode = vi.fn();
    render(Sidebar, { mode: "compress", onMode });
    expect(screen.getByRole("radio", { name: /Compress/ }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByRole("radio", { name: /Convert/ }).getAttribute("aria-checked")).toBe("false");
    await fireEvent.click(screen.getByRole("radio", { name: /Combine/ }));
    expect(onMode).toHaveBeenCalledWith("combine");
  });
});
