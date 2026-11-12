// SPDX-License-Identifier: GPL-3.0-or-later
// Development only: when the UI runs in a plain browser (`npm run dev`,
// outside Tauri), fake the app's commands with one row in every state, so
// the queue can be looked at. Production builds leave this file out.
import type { RowView, SettingsView } from "./types";

const rows: RowView[] = [
  { id: 1, rev: 1, note: null, inputs: ["/Users/me/Movies/interview-raw.mov"], path: "/Users/me/Movies/interview-raw.mov", name: "interview-raw.mov", from: "mov", to: "mp4", op: "convert", size: 1_400_000_000, output: "/Users/me/Movies/interview-raw.mp4", state: "running", fraction: 0.62 },
  { id: 2, rev: 1, note: null, inputs: ["/Users/me/Pictures/hero.png"], path: "/Users/me/Pictures/hero.png", name: "hero.png", from: "png", to: "webp", op: "convert", size: 4_200_000, output: "/Users/me/Pictures/hero.webp", state: "done", outputSize: 480_000 },
  { id: 3, rev: 1, note: null, inputs: ["/Users/me/Music/memo.wav"], path: "/Users/me/Music/memo.wav", name: "memo.wav", from: "wav", to: "mp3", op: "convert", size: 86_000_000, output: "/Users/me/Music/memo.mp3", state: "queued" },
  { id: 4, rev: 1, note: null, inputs: ["/Users/me/Downloads/scan.tif"], path: "/Users/me/Downloads/scan.tif", name: "scan.tif", from: "tiff", to: "jpeg", op: "convert", size: 12_000_000, output: null, state: "failed", error: { code: "INPUT_NOT_FOUND", file: "/Users/me/Downloads/scan.tif", reason: "no such file", fix: null } },
  { id: 5, rev: 1, note: null, inputs: ["/Users/me/Downloads/photos.zip"], path: "/Users/me/Downloads/photos.zip", name: "photos.zip", from: "zip", to: null, op: "extract", size: 31_000_000, output: "/Users/me/Downloads/photos", state: "cancelled" },
  { id: 6, rev: 1, note: null, inputs: ["/Users/me/Pictures/huge.png"], path: "/Users/me/Pictures/huge.png", name: "huge.png", from: "png", to: "webp", op: "convert", size: 9_800_000, output: null, state: "failed", error: { code: "UNSUPPORTED_INPUT", file: "/Users/me/Pictures/huge.png", reason: "WebP can't be wider or taller than 16383 pixels (this image is 20000×12000)", fix: "Set a size limit of 16383 px or less, or pick PNG or JPEG." } },
  { id: 8, rev: 1, note: null, inputs: ["/Users/me/Desktop/scan-1.png", "/Users/me/Desktop/scan-2.png", "/Users/me/Desktop/notes.pdf"], path: "/Users/me/Desktop/scan-1.png", name: "merged.pdf", from: null, to: "pdf", op: "merge", size: 5_200_000, output: "/Users/me/Desktop/merged.pdf", state: "queued" },
  { id: 9, rev: 1, note: "Saved to ~/Downloads/Persimmon — the original's folder isn't writable", inputs: ["/Users/me/Library/Mail/x/Invoice.png"], path: "/Users/me/Library/Mail/x/Invoice.png", name: "Invoice.png", from: "png", to: "jpeg", op: "convert", size: 900_000, output: "/Users/me/Downloads/Persimmon/Invoice.jpg", state: "done", outputSize: 210_000 },
  { id: 7, rev: 1, note: null, inputs: ["/Users/me/Pictures/small.jpg"], path: "/Users/me/Pictures/small.jpg", name: "small.jpg", from: "jpeg", to: "jpeg", op: "compress", size: 120_000, output: null, state: "notSmaller" },
];

const settings: SettingsView = {
  output: { mode: "beside" }, fallback: "/Users/me/Downloads/Persimmon", mode: "convert", dropWindow: false,
  outputLabel: "beside original", fallbackLabel: "~/Downloads/Persimmon",
};

const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
  get_settings: () => settings,
  set_mode: (a) => ({ ...settings, mode: a.mode }),
  queue_rows: () => rows,
  targets: () => ["avif", "jpeg", "mp3", "mp4", "png", "tiff", "webp"],
  set_target: () => ({ changed: [], skipped: 2 }),
  remove_rows: (a) => a.ids,
  add_paths: () => [],
  pick_files: () => [],
};

export function installDevMock() {
  const w = window as unknown as Record<string, unknown>;
  if (w.__TAURI_INTERNALS__) return;
  let next = 1;
  w.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
    transformCallback: () => next++,
    invoke: async (cmd: string, args: Record<string, unknown>) => {
      if (cmd.startsWith("plugin:event|")) return next++;
      return commands[cmd]?.(args ?? {}) ?? null;
    },
  };
}
