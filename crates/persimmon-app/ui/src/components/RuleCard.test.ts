// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import RuleCard from "./RuleCard.svelte";
import { view } from "../test/fixtures";

HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) { this.open = true; };
HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) { this.open = false; };

const handlers = () => ({ onToggle: vi.fn(), onCopy: vi.fn(), onEdit: vi.fn(), onScan: vi.fn(), onRemove: vi.fn() });

describe("RuleCard", () => {
  it("is a switch that reads ACTIVE and pauses", async () => {
    const h = handlers();
    render(RuleCard, { view: view({ today: 24 }), ...h });
    const sw = screen.getByRole("switch", { name: /~\/Inbox/ });
    expect(sw.getAttribute("aria-checked")).toBe("true");
    expect(sw.textContent).toContain("ACTIVE");
    expect(screen.getByText("24 files today")).toBeTruthy();
    await fireEvent.click(sw);
    expect(h.onToggle).toHaveBeenCalledWith("r1", false);
  });

  it("reads PAUSED when off", () => {
    render(RuleCard, { view: view({ enabled: false }, { state: "paused" }), ...handlers() });
    expect(screen.getByRole("switch").textContent).toContain("PAUSED");
  });

  it("shows its command, and copies it", async () => {
    const h = handlers();
    render(RuleCard, { view: view(), ...h });
    expect(screen.getByText(/prsm watch ~\/Inbox --to webp/)).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: /Copy/ }));
    expect(h.onCopy).toHaveBeenCalledWith("r1");
  });

  it("asks before it removes", async () => {
    const h = handlers();
    render(RuleCard, { view: view(), ...h });
    await fireEvent.click(screen.getByRole("button", { name: /Remove/ }));
    expect(h.onRemove).not.toHaveBeenCalled();
    expect(screen.getByText("Stop watching ~/Inbox? Files there stay as they are.")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Stop watching" }));
    expect(h.onRemove).toHaveBeenCalledWith("r1");
  });

  it("says why it can't read its folder, and how to fix it", () => {
    render(RuleCard, {
      view: view({}, { state: "unreadable", error: { code: "UNSUPPORTED_INPUT", file: null, reason: "Persimmon can't read this folder", fix: "Allow Persimmon in System Settings → Privacy & Security → Files and Folders" } } as never),
      ...handlers(),
    });
    expect(screen.getByText("Persimmon can't read this folder")).toBeTruthy();
    expect(screen.getByText("Allow Persimmon in System Settings → Privacy & Security → Files and Folders")).toBeTruthy();
  });

  it("names the last failure and any note", () => {
    render(RuleCard, { view: view({}, { lastFailed: { name: "scan-0231.jpg", reason: "it's damaged" }, note: "Couldn't read what this rule had converted, so files already here were left alone" }), ...handlers() });
    expect(screen.getByText("Last failed: scan-0231.jpg — it's damaged")).toBeTruthy();
    expect(screen.getByText(/files already here were left alone/)).toBeTruthy();
  });

  it("edits and scans", async () => {
    const h = handlers();
    render(RuleCard, { view: view(), ...h });
    await fireEvent.click(screen.getByRole("button", { name: /Edit/ }));
    await fireEvent.click(screen.getByRole("button", { name: /Scan now/ }));
    expect(h.onEdit).toHaveBeenCalledWith("r1");
    expect(h.onScan).toHaveBeenCalledWith("r1");
  });
});
