// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { nextRadio } from "./radio";

describe("nextRadio", () => {
  const ids = ["a", "b", "c"];
  it("moves down and right to the next, wrapping around", () => {
    expect(nextRadio(ids, "a", "ArrowDown")).toBe("b");
    expect(nextRadio(ids, "b", "ArrowRight")).toBe("c");
    expect(nextRadio(ids, "c", "ArrowDown")).toBe("a");
  });
  it("moves up and left to the previous, wrapping around", () => {
    expect(nextRadio(ids, "b", "ArrowUp")).toBe("a");
    expect(nextRadio(ids, "a", "ArrowLeft")).toBe("c");
  });
  it("jumps to the ends with Home and End", () => {
    expect(nextRadio(ids, "b", "Home")).toBe("a");
    expect(nextRadio(ids, "b", "End")).toBe("c");
  });
  it("ignores other keys", () => {
    expect(nextRadio(ids, "b", "Enter")).toBeNull();
  });
});
