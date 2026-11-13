// SPDX-License-Identifier: GPL-3.0-or-later
// What the status bar says after an undo, and when it offers one (spec D3).
import { plural } from "./format";
import type { BatchView, UndoReport } from "./types";

/** The count, plus the file left alone and why; several point to History. */
export function undoMessage(r: UndoReport): string {
  const moved = `Moved ${plural(r.trashed, "file")} to the Trash`;
  if (r.skipped.length === 1) return `${moved}; ${r.skipped[0]}`;
  if (r.skipped.length > 1) return `${moved}; ${r.skipped.length} left alone (see History)`;
  return moved;
}

/** Undo Last Batch only ever means the newest batch (newest first). */
export function lastUndoable(batches: BatchView[]): boolean {
  return batches[0]?.undoable ?? false;
}
