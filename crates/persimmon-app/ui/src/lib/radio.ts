// SPDX-License-Identifier: GPL-3.0-or-later
// Arrow keys in a radio group, as macOS moves them: down and right to the
// next, up and left to the previous, wrapping around; Home and End to the
// ends.

/** The radio `key` moves to from `current`, or null for any other key. */
export function nextRadio(ids: string[], current: string, key: string): string | null {
  if (!ids.length) return null;
  const i = ids.indexOf(current);
  switch (key) {
    case "ArrowDown": case "ArrowRight": return ids[(i + 1) % ids.length];
    case "ArrowUp": case "ArrowLeft": return ids[(i - 1 + ids.length) % ids.length];
    case "Home": return ids[0];
    case "End": return ids[ids.length - 1];
    default: return null;
  }
}
