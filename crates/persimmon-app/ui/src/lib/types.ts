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
  outputLabel: string;
  fallbackLabel: string;
}

export interface SetTarget {
  changed: RowView[];
  skipped: number;
}
