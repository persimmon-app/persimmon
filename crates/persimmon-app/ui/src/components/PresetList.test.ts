// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import PresetList from "./PresetList.svelte";
import { presets } from "../test/fixtures";

describe("PresetList", () => {
  it("marks the active preset, shows its ⌘ number, and picks on click", async () => {
    const onPreset = vi.fn();
    render(PresetList, { presets, active: "email-video", onPreset });
    expect(screen.getByRole("radio", { name: /Email video/ }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByRole("radio", { name: /Web image/ }).getAttribute("aria-checked")).toBe("false");
    expect(screen.getByText("⌘1")).toBeTruthy();
    await fireEvent.click(screen.getByRole("radio", { name: /Thumbs/ }));
    expect(onPreset).toHaveBeenCalledWith("thumbs");
  });
});
