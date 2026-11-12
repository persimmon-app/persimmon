// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import OutputPane from "./OutputPane.svelte";
import type { SettingsView } from "../lib/types";

const beside: SettingsView = {
  output: { mode: "beside" }, fallback: "/Users/me/Downloads/Persimmon", mode: "convert", dropWindow: false,
  outputLabel: "beside original", fallbackLabel: "~/Downloads/Persimmon",
};

function show(settings: SettingsView) {
  const h = { onBeside: vi.fn(), onChooseFolder: vi.fn(), onChooseFallback: vi.fn() };
  render(OutputPane, { settings, ...h });
  return h;
}

describe("OutputPane", () => {
  it("reflects saving beside the original", () => {
    show(beside);
    expect((screen.getByRole("radio", { name: "Beside the original" }) as HTMLInputElement).checked).toBe(true);
    expect(screen.getByText("~/Downloads/Persimmon")).toBeTruthy();
  });

  it("reflects a folder, and Choose… picks one", async () => {
    const h = show({ ...beside, output: { mode: "folder", folder: "/Users/me/Converted" }, outputLabel: "~/Converted" });
    expect((screen.getByRole("radio", { name: /In a folder/ }) as HTMLInputElement).checked).toBe(true);
    expect(screen.getByText("~/Converted")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Choose output folder" }));
    expect(h.onChooseFolder).toHaveBeenCalled();
  });

  it("changes the fallback folder", async () => {
    const h = show(beside);
    await fireEvent.click(screen.getByRole("button", { name: "Change fallback folder" }));
    expect(h.onChooseFallback).toHaveBeenCalled();
  });
});
