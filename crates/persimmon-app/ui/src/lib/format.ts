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

/** The target badge: the format, EXTRACT for archives, or for Compress
 * the format it keeps. */
export function target(row: RowView): string {
  if (row.op === "extract") return "EXTRACT";
  if (row.op === "compress") return badge(row.to ?? row.from);
  return badge(row.to);
}

/** The status bar's note when added files were already in the queue. */
export function alreadyMessage(n: number): string {
  return `${plural(n, "file")} already in the queue`;
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}
