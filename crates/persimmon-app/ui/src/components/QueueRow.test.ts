// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import QueueRow from "./QueueRow.svelte";
import type { RowView } from "../lib/types";

const base = {
  id: 1, path: "/x/a.png", inputs: ["/x/a.png"], name: "a.png", from: "png", to: "jpeg",
  op: "convert", size: 1_400_000_000, output: "/x/a.jpg", note: null, rev: 1,
};

function row(state: object): RowView {
  return { ...base, ...state } as unknown as RowView;
}

function show(r: RowView) {
  const handlers = { onCancel: vi.fn(), onRetry: vi.fn(), onReveal: vi.fn(), onSelect: vi.fn() };
  const { container } = render(QueueRow, { row: r, selected: false, ...handlers });
  return { container, ...handlers };
}

describe("QueueRow", () => {
  it("shows a queued row", () => {
    const { container } = show(row({ state: "queued" }));
    expect(screen.getByText("Queued")).toBeTruthy();
    expect(screen.getByText("a.png")).toBeTruthy();
    expect(screen.getByText("PNG")).toBeTruthy();
    expect(screen.getByText("JPEG")).toBeTruthy();
    expect(screen.getByText("1.4 GB")).toBeTruthy();
    expect(container.querySelector(".running")).toBeNull();
  });

  it("shows a running row with its percentage and a cancel button", async () => {
    const { container, onCancel } = show(row({ state: "running", fraction: 0.62 }));
    expect(screen.getByText("62%")).toBeTruthy();
    expect(container.querySelector(".running")).not.toBeNull();
    const bar = screen.getByRole("progressbar");
    expect(bar.getAttribute("aria-valuenow")).toBe("62");
    await fireEvent.click(screen.getByLabelText("Cancel a.png"));
    expect(onCancel).toHaveBeenCalledWith(1);
  });

  it("shows an indeterminate bar when the backend can't tell", () => {
    show(row({ state: "running", fraction: null }));
    expect(screen.getByRole("progressbar").hasAttribute("aria-valuenow")).toBe(false);
    expect(screen.getByText("Working")).toBeTruthy();
  });

  it("shows a done row with both sizes and Reveal", async () => {
    const { container, onReveal } = show(row({ state: "done", outputSize: 340_000_000 }));
    expect(screen.getByText("Done")).toBeTruthy();
    expect(screen.getByText("1.4 GB → 340 MB")).toBeTruthy();
    expect(container.querySelector(".running")).toBeNull();
    await fireEvent.click(screen.getByLabelText("Reveal a.jpg in Finder"));
    expect(onReveal).toHaveBeenCalledWith(1);
  });

  it("shows where a fallen-back output went", () => {
    show(row({ state: "done", outputSize: 3, note: "Saved to ~/Downloads/Persimmon — the original's folder isn't writable" }));
    expect(screen.getByText("Saved to ~/Downloads/Persimmon — the original's folder isn't writable")).toBeTruthy();
  });

  it("shows a row that was already small", () => {
    show(row({ state: "notSmaller" }));
    expect(screen.getByText("Already small")).toBeTruthy();
  });

  it("shows a failed row with its reason, fix and Retry", async () => {
    const { onRetry } = show(row({
      state: "failed",
      error: { code: "INPUT_NOT_FOUND", file: "/x/a.png", reason: "no such file", fix: "Pick an output format." },
    }));
    expect(screen.getByText("Failed")).toBeTruthy();
    expect(screen.getByText(/no such file/)).toBeTruthy();
    expect(screen.getByText(/Pick an output format\./)).toBeTruthy();
    await fireEvent.click(screen.getByLabelText("Retry a.png"));
    expect(onRetry).toHaveBeenCalledWith(1);
  });

  it("gives VoiceOver the reason and fix of a failure, and a running row's progress", () => {
    show(row({
      state: "failed",
      error: { code: "INPUT_NOT_FOUND", file: "/x/a.png", reason: "no such file", fix: "Pick an output format." },
    }));
    expect(screen.getByRole("option", { name: "a.png, failed: no such file. Pick an output format." })).toBeTruthy();
  });

  it("names a running row's progress for VoiceOver", () => {
    show(row({ state: "running", fraction: 0.62 }));
    expect(screen.getByRole("option", { name: "a.png, running, 62%" })).toBeTruthy();
  });

  it("names the file that failed in a Combine row", () => {
    show(row({
      op: "merge", from: null, to: "pdf", name: "merged.pdf", inputs: ["/x/a.png", "/x/b.jpeg"],
      state: "failed",
      error: { code: "UNSUPPORTED_INPUT", file: "/x/Photos Library.photoslibrary/b.jpeg", reason: "This is Photos' preview, not your original", fix: null },
    }));
    expect(screen.getByText(/b\.jpeg: This is Photos' preview/)).toBeTruthy();
  });

  it("shows a cancelled row with Retry", () => {
    show(row({ state: "cancelled" }));
    expect(screen.getByText("Cancelled")).toBeTruthy();
    expect(screen.getByLabelText("Retry a.png")).toBeTruthy();
  });

  it("names EXTRACT as the target of an archive", () => {
    show({ ...base, from: "zip", to: null, op: "extract", state: "queued" } as unknown as RowView);
    expect(screen.getByText("EXTRACT")).toBeTruthy();
  });
});
