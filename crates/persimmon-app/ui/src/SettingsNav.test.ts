// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";

vi.mock("./lib/ipc", () => {
  const off = () => Promise.resolve(() => {});
  return {
    getSettings: () => Promise.resolve(null), presets: () => Promise.resolve([]), engines: () => Promise.resolve(null),
    onSettings: off, onPresets: off,
  };
});

import Settings from "./Settings.svelte";

describe("Settings", () => {
  it("lists Finder & menus between Engines and Privacy", () => {
    render(Settings);
    const names = screen.getAllByRole("button").map((b) => b.textContent?.trim());
    const at = (n: string) => names.indexOf(n);
    expect(at("Finder & menus")).toBeGreaterThan(at("Engines"));
    expect(at("Finder & menus")).toBeLessThan(at("Privacy"));
  });
});
