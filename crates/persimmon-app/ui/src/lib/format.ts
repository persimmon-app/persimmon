// SPDX-License-Identifier: GPL-3.0-or-later
import type { FormatId, RowView } from "./types";

/** Sizes in decimal units, as Finder shows them (DD-33). */
export function size(bytes: number): string {
  if (bytes < 1000) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes;
  let i = -1;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

/** A format's badge: "JPEG", "TAR.GZ". */
export function badge(id: FormatId | null): string {
  return id ? id.toUpperCase() : "—";
}

/** The target badge: the format, or EXTRACT for archives. */
export function target(row: RowView): string {
  return row.op === "extract" ? "EXTRACT" : badge(row.to);
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}
