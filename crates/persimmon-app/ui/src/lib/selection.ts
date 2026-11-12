// SPDX-License-Identifier: GPL-3.0-or-later
// Row selection, the macOS way: click selects one, ⇧-click a range from
// the anchor, ⌘-click toggles one; arrows move from the last selected row.
import type { RowId } from "./types";

export interface Selection {
  selected: RowId[];
  anchor: RowId | null;
}

export function clicked(ids: RowId[], s: Selection, id: RowId, mode: "replace" | "range" | "toggle"): Selection {
  if (mode === "toggle") {
    const selected = s.selected.includes(id) ? s.selected.filter((x) => x !== id) : [...s.selected, id];
    return { selected: ids.filter((x) => selected.includes(x)), anchor: id };
  }
  if (mode === "range" && s.anchor !== null && ids.includes(s.anchor)) {
    const [a, b] = [ids.indexOf(s.anchor), ids.indexOf(id)].sort((x, y) => x - y);
    return { selected: ids.slice(a, b + 1), anchor: s.anchor };
  }
  return { selected: [id], anchor: id };
}

export function moved(ids: RowId[], s: Selection, by: number, extend: boolean): Selection {
  if (!ids.length) return s;
  const last = s.selected.at(-1);
  const at = last === undefined || !ids.includes(last)
    ? (by > 0 ? 0 : ids.length - 1)
    : Math.min(Math.max(ids.indexOf(last) + by, 0), ids.length - 1);
  return clicked(ids, s, ids[at], extend ? "range" : "replace");
}
