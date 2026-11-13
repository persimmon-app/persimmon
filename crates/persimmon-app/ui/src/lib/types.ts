// SPDX-License-Identifier: GPL-3.0-or-later
// Mirrors of the Rust types the app sends (crates/persimmon-app/src/queue.rs
// and wording.rs). Keep in step with the serde shapes there.

export type FormatId = string;
export type RowId = number;
export type Operation = "convert" | "compress" | "extract" | "merge" | "split";

export interface Shown {
  code: string;
  file: string | null;
  reason: string;
  fix: string | null;
}

export type RowState =
  | { state: "queued" }
  | { state: "running"; fraction: number | null }
  | { state: "done"; outputSize: number | null }
  | { state: "notSmaller" }
  | { state: "failed"; error: Shown }
  | { state: "cancelled" };

export type RowView = {
  id: RowId;
  path: string;
  inputs: string[];
  name: string;
  from: FormatId | null;
  to: FormatId | null;
  op: Operation;
  size: number;
  output: string | null;
  note: string | null;
  rev: number;
} & RowState;

export type QueueMode = "convert" | "compress" | "extract" | "combine";

export type OutputMode = { mode: "beside" } | { mode: "folder"; folder: string };

/** The settings as the app sends them (settings.rs, SettingsView). */
export interface SettingsView {
  output: OutputMode;
  fallback: string;
  mode: QueueMode;
  dropWindow: boolean;
  preset: string | null;
  outputLabel: string;
  fallbackLabel: string;
}

export interface SetTarget {
  changed: RowView[];
  skipped: number;
}

export interface JobOptions {
  quality: "low" | "medium" | "high" | "max" | null;
  preset: "compatible" | "small" | null;
  pages: string | null;
  maxSize: number | null;
  stripMetadata: boolean;
}

/** A preset as the app sends it (app.rs, PresetView). */
export interface PresetView {
  id: string;
  name: string;
  builtin: boolean;
  mode: QueueMode;
  target: FormatId | null;
  options: JobOptions;
  command: string;
  kinds: string;
  shortcut: number | null;
}

export type ItemOutcome =
  | { outcome: "done" }
  | { outcome: "notSmaller" }
  | { outcome: "failed"; error: Shown }
  | { outcome: "cancelled" };

export type Item = {
  inputs: string[];
  output: string | null;
  outputSize: number | null;
  outputMtime: number | null;
  size: number;
  undone: boolean;
} & ItemOutcome;

export type HistoryFilter = "all" | "undoable" | "failed";

/** A batch as the app sends it (history.rs, app.rs BatchView). */
export interface BatchView {
  id: string;
  started: number;
  finished: number;
  source: "window" | "drop" | "opened";
  preset: string | null;
  mode: QueueMode;
  items: Item[];
  undone: boolean;
  undoable: boolean;
}

export interface UndoReport {
  trashed: number;
  skipped: string[];
}

export interface EnginesView {
  ffmpeg: { path: string; version: string } | null;
  ffmpegError: Shown | null;
  images: boolean;
  rar: boolean;
  version: string;
}
