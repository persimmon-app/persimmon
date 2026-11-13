// SPDX-License-Identifier: GPL-3.0-or-later
// The queue as VoiceOver and the keyboard meet it: a grid that holds focus
// and points at the active row.
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import type { RowView } from "../lib/types";

const h = vi.hoisted(() => {
  const r = (id: number, name: string, state: Record<string, unknown>): unknown => ({
    id, rev: 1, path: `/x/${name}`, inputs: [`/x/${name}`], name, from: "png", to: "jpeg", op: "convert",
    size: 1000, output: null, note: null, preset: null, ...state,
  });
  return {
    rows: [
      r(1, "a.png", { state: "queued" }),
      r(2, "b.png", { state: "running", fraction: 0.2 }),
      r(3, "c.png", { state: "failed", error: { code: "CORRUPT_INPUT", file: "/x/c.png", reason: "it's damaged", fix: "Remove this file from the queue." } }),
    ],
    handlers: {} as Record<string, (x: unknown) => void>,
  };
});

vi.mock("../lib/ipc", () => {
  const listen = (name: string) => (f: (x: unknown) => void) => { h.handlers[name] = f; return Promise.resolve(() => {}); };
  const empty = () => Promise.resolve([]);
  return {
    queueRows: () => Promise.resolve(h.rows), getSettings: () => Promise.resolve(null), presets: empty,
    history: empty, targets: empty, onHistory: listen("history"), onHistoryCleared: listen("cleared"),
    onPresets: listen("presets"), onUndoReport: listen("undo"), onRow: listen("row"),
    onConfirmQuit: listen("quit"), onSettings: listen("settings"), onQueueNote: listen("note"),
  };
});
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: () => Promise.resolve(() => {}) }),
}));

import App from "../App.svelte";

async function grid() {
  render(App);
  await screen.findAllByRole("row");
  return screen.getByRole("grid", { name: "Queue" });
}

describe("the queue grid", () => {
  it("is a focusable grid of rows", async () => {
    const g = await grid();
    expect(g.getAttribute("tabindex")).toBe("0");
    expect(screen.getAllByRole("row")).toHaveLength(3);
  });

  it("arrow keys move the active row; progress doesn't", async () => {
    const g = await grid();
    await fireEvent.click(screen.getAllByRole("row")[0]);
    g.focus();
    await fireEvent.keyDown(g, { key: "ArrowDown" });
    expect(g.getAttribute("aria-activedescendant")).toBe("row-2");
    h.handlers.row({ ...(h.rows[1] as object), rev: 2, fraction: 0.6 });
    await tick();
    expect(g.getAttribute("aria-activedescendant")).toBe("row-2");
    expect(document.activeElement).toBe(g);
  });

  it("only the active row's buttons are in the tab order", async () => {
    const g = await grid();
    await fireEvent.click(screen.getAllByRole("row")[1]);
    expect(g.getAttribute("aria-activedescendant")).toBe("row-2");
    expect(screen.getByRole("button", { name: "Cancel b.png" }).getAttribute("tabindex")).toBe("0");
    expect(screen.getByRole("button", { name: "Retry c.png" }).getAttribute("tabindex")).toBe("-1");
  });
});
