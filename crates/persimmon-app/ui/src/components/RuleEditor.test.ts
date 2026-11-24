// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import RuleEditor from "./RuleEditor.svelte";
import type { Rule, RuleDraftView } from "../lib/types";
import { presets } from "../test/fixtures";

HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) { this.open = true; };
HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) { this.open = false; };

const rule: Rule = {
  id: "r1", folder: "/Users/me/Inbox", target: "webp", preset: null, patterns: "*.jpg,*.png",
  results: "/Users/me/Inbox/converted", done: null, recursive: false, settle: true, notifyEach: false, enabled: true,
};
const draft = (existing = 0): RuleDraftView => ({ rule, command: "prsm watch ~/Inbox --to webp", existing });

function props(over: Record<string, unknown> = {}) {
  return {
    draft: draft(), isNew: true, targets: ["jpeg", "png", "webp"], presets,
    onDraft: vi.fn((r: Rule) => Promise.resolve({ rule: r, command: "prsm watch ~/Inbox --to png", existing: 0 })),
    onDefaults: vi.fn((t: string) => Promise.resolve(t === "png" ? "*.jpg,*.webp" : "*.jpg,*.png")),
    onChoose: vi.fn().mockResolvedValue(null),
    onSave: vi.fn().mockResolvedValue(null),
    onCancel: vi.fn(),
    ...over,
  };
}

const flush = async () => { for (let i = 0; i < 5; i++) await tick(); };

describe("RuleEditor", () => {
  it("has the canvas's fields", () => {
    render(RuleEditor, props());
    for (const label of ["Match files", "Convert to", "Preset", "Include subfolders", "Wait until the file stops growing", "Notify on every file", "Leave where it is", "Move to"]) {
      expect(screen.getByLabelText(new RegExp(label)), label).toBeTruthy();
    }
    expect(screen.getByText("prsm watch ~/Inbox --to webp")).toBeTruthy();
  });

  it("resets untouched patterns when the target changes, and keeps edited ones", async () => {
    const p = props();
    render(RuleEditor, p);
    const patterns = screen.getByLabelText(/Match files/) as HTMLInputElement;
    await fireEvent.change(screen.getByLabelText(/Convert to/), { target: { value: "png" } });
    await flush();
    expect(patterns.value).toBe("*.jpg,*.webp");
    await fireEvent.input(patterns, { target: { value: "*.heic" } });
    await fireEvent.change(screen.getByLabelText(/Convert to/), { target: { value: "jpeg" } });
    await flush();
    expect(patterns.value).toBe("*.heic");
  });

  it("shows a refusal's reason and fix, and stays open", async () => {
    const p = props({ onSave: vi.fn().mockResolvedValue({ code: "USAGE", file: null, reason: "Results would be converted again", fix: "Choose another folder for results." }) });
    render(RuleEditor, p);
    await fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await flush();
    expect(screen.getByText("Results would be converted again")).toBeTruthy();
    expect(screen.getByText("Choose another folder for results.")).toBeTruthy();
    expect(p.onCancel).not.toHaveBeenCalled();
  });

  it("offers the files already there only for a new rule that has some", async () => {
    const { unmount } = render(RuleEditor, props());
    expect(screen.queryByLabelText(/already here/)).toBeNull();
    unmount();
    const p = props({ draft: draft(14) });
    render(RuleEditor, p);
    const box = screen.getByLabelText("Also convert the 14 files already here") as HTMLInputElement;
    expect(box.checked).toBe(false);
    await fireEvent.click(box);
    await fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await flush();
    expect(p.onSave).toHaveBeenCalledWith(expect.objectContaining({ id: "r1" }), true);
  });

  it("doesn't offer them when editing", () => {
    render(RuleEditor, props({ draft: draft(3), isNew: false }));
    expect(screen.queryByLabelText(/already here/)).toBeNull();
  });

  it("moves originals to <folder>/done when chosen", async () => {
    const p = props();
    render(RuleEditor, p);
    await fireEvent.click(screen.getByLabelText(/Move to/));
    await fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await flush();
    expect(p.onSave).toHaveBeenCalledWith(expect.objectContaining({ done: "/Users/me/Inbox/done" }), false);
  });

  it("offers only the presets that fit the target", () => {
    render(RuleEditor, props());
    const options = [...(screen.getByLabelText(/Preset/) as HTMLSelectElement).options].map((o) => o.textContent);
    expect(options).toContain("None");
    expect(options).toContain("Web image");
    expect(options).not.toContain("Email video");
  });

  it("cancels on Escape", async () => {
    const p = props();
    render(RuleEditor, p);
    await fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }));
    expect(p.onCancel).toHaveBeenCalled();
  });

  it("counts the files already here again when the folder changes", async () => {
    const p = props({
      onChoose: vi.fn().mockResolvedValue("/Users/me/Scans"),
      onDraft: vi.fn((r: Rule) => Promise.resolve({ rule: r, command: "c", existing: r.folder === "/Users/me/Scans" ? 5 : 0 })),
    });
    render(RuleEditor, p);
    await flush();
    expect(screen.queryByLabelText(/already here/)).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Choose the watched folder" }));
    await flush();
    expect(screen.getByLabelText("Also convert the 5 files already here")).toBeTruthy();
  });
});
