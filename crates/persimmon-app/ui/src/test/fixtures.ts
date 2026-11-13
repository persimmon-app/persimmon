// SPDX-License-Identifier: GPL-3.0-or-later
// Shared test data in the app's JSON shapes.
import type { BatchView, PresetView } from "../lib/types";

const none = { quality: null, preset: null, pages: null, maxSize: null, stripMetadata: false } as const;

export const presets: PresetView[] = [
  { id: "web-image", name: "Web image", builtin: true, mode: "convert", target: "webp",
    options: { ...none, quality: "medium", maxSize: 2048, stripMetadata: true },
    command: "prsm convert <files> --to webp --quality medium --max-size 2048 --strip-metadata", kinds: "images", shortcut: 1 },
  { id: "email-video", name: "Email video", builtin: true, mode: "convert", target: "mp4",
    options: { ...none, preset: "compatible", quality: "low", maxSize: 1280, stripMetadata: true },
    command: "prsm convert <files> --to mp4 --preset compatible --quality low --max-size 1280 --strip-metadata", kinds: "video", shortcut: 2 },
  { id: "thumbs", name: "Thumbs", builtin: false, mode: "convert", target: "jpeg",
    options: { ...none, maxSize: 320 }, command: "prsm convert <files> --to jpeg --max-size 320", kinds: "images", shortcut: 3 },
];

/** 12 November 2026, 15:00 local time. */
export const NOW = new Date(2026, 10, 12, 15, 0, 0).getTime() / 1000;

export function batch(over: Partial<BatchView> & { id: string }): BatchView {
  return {
    started: NOW - 200, finished: NOW - 8, source: "window", preset: "Web image", mode: "convert", undone: false, undoable: true,
    items: [
      { inputs: ["/x/hero.png"], output: "/x/hero.webp", outputSize: 480_000, outputMtime: 1, size: 4_200_000, undone: false, outcome: "done" },
      { inputs: ["/x/scan.png"], output: null, outputSize: null, outputMtime: null, size: 1_000_000, undone: false, outcome: "failed",
        error: { code: "CORRUPT_INPUT", file: "/x/scan.png", reason: "it's damaged", fix: null } },
    ],
    ...over,
  };
}
