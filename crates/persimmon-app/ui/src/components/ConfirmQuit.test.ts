// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import ConfirmQuit from "./ConfirmQuit.svelte";

// jsdom has no modal dialogs.
HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) { this.open = true; };
HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) { this.open = false; };

describe("ConfirmQuit", () => {
  it("counts the running conversions", () => {
    render(ConfirmQuit, { open: true, running: 3, onCancel: vi.fn(), onQuit: vi.fn() });
    expect(screen.getByText("3 conversions are running. Quit and cancel them?")).toBeTruthy();
  });
  it("keeps the singular for one", () => {
    render(ConfirmQuit, { open: true, running: 1, onCancel: vi.fn(), onQuit: vi.fn() });
    expect(screen.getByText("A conversion is running. Quit and cancel it?")).toBeTruthy();
  });

  it("says watch folders stop, with nothing running", () => {
    render(ConfirmQuit, { open: true, running: 0, rules: 1, onCancel: vi.fn(), onQuit: vi.fn() });
    expect(screen.getByText("Watch folders stop while Persimmon is closed. Quit anyway?")).toBeTruthy();
  });
  it("adds it to the running sentence", () => {
    render(ConfirmQuit, { open: true, running: 2, rules: 1, onCancel: vi.fn(), onQuit: vi.fn() });
    expect(screen.getByText("2 conversions are running. Quit and cancel them? Watch folders stop too.")).toBeTruthy();
  });
});
