// SPDX-License-Identifier: GPL-3.0-or-later
// The prsm line a preset amounts to (DD-18), for the editor's live preview.
// Mirrors presets.rs `command()`; command.test.ts pins the same strings as
// the Rust tests, so the two can't drift apart unnoticed.
import type { FormatId, JobOptions, QueueMode } from "./types";

const verbs: Record<QueueMode, string> = { convert: "convert", compress: "compress", extract: "extract", combine: "merge" };

export function commandLine(p: { mode: QueueMode; target: FormatId | null; options: JobOptions }): string {
  let line = `prsm ${verbs[p.mode]} <files>`;
  if (p.target) line += ` --to ${p.target}`;
  if (p.options.preset) line += ` --preset ${p.options.preset}`;
  if (p.options.quality) line += ` --quality ${p.options.quality}`;
  if (p.options.maxSize) line += ` --max-size ${p.options.maxSize}`;
  if (p.options.stripMetadata) line += " --strip-metadata";
  return line;
}
