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
});
