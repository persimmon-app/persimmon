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
  | { state: "cancelled" }
  /** A file still being handed over; `photos` when Photos is handing it
   * over, `slow` once it has taken a while (for Photos, iCloud). */
  | { state: "receiving"; slow: boolean; photos: boolean };

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
  /** Read from macOS's Login Items each time (not stored). */
  startAtLogin: boolean;
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
  hardware: boolean;
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
  /** The row's target and mode, for Retry; older history has neither. */
  to?: FormatId | null;
  mode?: QueueMode | null;
  undone: boolean;
  /** The original a watch rule moved after converting it (history.rs, MovedOriginal). */
  moved?: { from: string; to: string; stamp: { size: number; secs: number; nanos: number } } | null;
  /** Whether undo put that original back. */
  restored?: boolean;
} & ItemOutcome;

export type HistoryFilter = "all" | "undoable" | "failed";

/** A batch as the app sends it (history.rs, app.rs BatchView). */
export interface BatchView {
  id: string;
  started: number;
  finished: number;
  source: "window" | "drop" | "opened" | "watch" | "finder" | "wheel";
  /** A watch batch's rule and folder, and the folder as shown (~/Inbox). */
  rule?: string | null;
  folder?: string | null;
  folderLabel?: string | null;
  preset: string | null;
  mode: QueueMode;
  items: Item[];
  undone: boolean;
  undoable: boolean;
}

export interface UndoReport {
  trashed: number;
  /** Originals a watch rule had moved, put back where they were. */
  restored: number;
  skipped: string[];
}

export interface EnginesView {
  ffmpeg: { path: string; version: string } | null;
  ffmpegError: Shown | null;
  images: boolean;
  rar: boolean;
  /** The codecs VideoToolbox makes in this FFmpeg: "H.264", "HEVC". */
  hardware: string[];
  version: string;
}

/** A watch rule (watch/rule.rs, Rule). */
export interface Rule {
  id: string;
  folder: string;
  target: FormatId;
  preset: string | null;
  patterns: string;
  results: string;
  done: string | null;
  recursive: boolean;
  settle: boolean;
  notifyEach: boolean;
  enabled: boolean;
}

export type WatchState =
  | { state: "watching" }
  | { state: "converting"; done: number; total: number }
  | { state: "missing" }
  | { state: "unreadable"; error: Shown }
  | { state: "paused" };

/** What a rule is doing (watch/runner.rs, RuleStatus). */
export type RuleStatus = {
  id: string;
  burstDone: number;
  lastFailed: { name: string; reason: string } | null;
  note: string | null;
} & WatchState;

/** A rule as the Watch folders view shows it (app.rs, RuleView). */
export type RuleView = Rule & {
  command: string;
  status: RuleStatus;
  today: number;
  folderLabel: string;
  resultsLabel: string;
  doneLabel: string | null;
};

/** A rule being made or edited (app.rs, RuleDraftView). */
export interface RuleDraftView {
  rule: Rule;
  command: string;
  existing: number;
}

export type NotificationsState = "allowed" | "off" | "notAsked";

/** What the wheel shows (app/wheel.rs): its formats, their hover labels,
 * and the hub's text. */
export interface WheelShow {
  formats: FormatId[];
  labels: string[];
  hub: string;
}
