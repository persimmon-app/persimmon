// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import Drop from "./Drop.svelte";

type Handler = (e: { payload: { type: string; paths?: string[] } }) => void;

describe("Drop window", () => {
  it("lights up while a drag hovers and drops into the queue", async () => {
    let handler: Handler = () => {};
    const listenDrag = (h: Handler) => { handler = h; return Promise.resolve(() => {}); };
    const onDrop = vi.fn();
    render(Drop, { listenDrag, onDrop, running: 0 });
    const zone = screen.getByText("Drop here").closest(".zone")!;
    expect(zone.classList.contains("hover")).toBe(false);
    handler({ payload: { type: "enter", paths: ["/x/a.png"] } });
    await tick();
    expect(zone.classList.contains("hover")).toBe(true);
    handler({ payload: { type: "leave" } });
    await tick();
    expect(zone.classList.contains("hover")).toBe(false);
    handler({ payload: { type: "drop", paths: ["/x/a.png"] } });
    expect(onDrop).toHaveBeenCalledWith(["/x/a.png"]);
  });

  it("says how many are running", () => {
    render(Drop, { listenDrag: () => Promise.resolve(() => {}), onDrop: vi.fn(), running: 2 });
    expect(screen.getByText("2 running")).toBeTruthy();
  });
});
