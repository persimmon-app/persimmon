// SPDX-License-Identifier: GPL-3.0-or-later
// Whether the queue's shortcuts should act on a key press.

/** Not while the quit dialog is open; and plain keys (⌫, arrows) belong to
 * a focused form control such as the Output menu. ⌘ shortcuts still work. */
export function queueOwnsKey(e: KeyboardEvent, dialogOpen: boolean): boolean {
  if (dialogOpen) return false;
  const t = e.target as Element | null;
  const inControl = !!t && ["INPUT", "SELECT", "TEXTAREA", "BUTTON"].includes(t.tagName);
  return !inControl || e.metaKey;
}
