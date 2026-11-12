// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { queueOwnsKey } from "./keys";

const key = (target: Element, metaKey = false) => ({ target, metaKey }) as unknown as KeyboardEvent;

describe("queueOwnsKey", () => {
  it("handles keys when nothing else should", () => {
    expect(queueOwnsKey(key(document.body), false)).toBe(true);
  });
  it("leaves every key to the quit dialog while it's open", () => {
    expect(queueOwnsKey(key(document.body), true)).toBe(false);
    expect(queueOwnsKey(key(document.body, true), true)).toBe(false);
  });
  it("leaves plain keys to a focused form control, but not ⌘ shortcuts", () => {
    const select = document.createElement("select");
    expect(queueOwnsKey(key(select), false)).toBe(false);
    expect(queueOwnsKey(key(select, true), false)).toBe(true);
  });
});
