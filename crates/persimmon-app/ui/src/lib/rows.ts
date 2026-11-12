// SPDX-License-Identifier: GPL-3.0-or-later
// Keeping the window's rows: every view carries a revision, and an older
// view never replaces a newer one (events and command replies can arrive
// out of order).
import type { RowId, RowView } from "./types";

export function upsert(rows: RowView[], row: RowView): RowView[] {
  const i = rows.findIndex((r) => r.id === row.id);
  if (i === -1) return [...rows, row];
  if (rows[i].rev > row.rev) return rows;
  const next = rows.slice();
  next[i] = row;
  return next;
}

export function without(rows: RowView[], ids: RowId[]): RowView[] {
  return rows.filter((r) => !ids.includes(r.id));
}
