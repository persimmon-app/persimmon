// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import Wheel from "./Wheel.svelte";
import type { FormatId } from "./lib/types";

const formats: FormatId[] = ["mp4", "gif", "mp3", "webm", "mov", "m4a"];
const labels = ["MPEG-4 · H.264", "GIF · animated", "MP3 · audio", "WebM · VP9", "QuickTime · H.264", "MPEG-4 audio · AAC"];
type Handler = (e: { payload: { type: string; paths?: string[]; position?: { x: number; y: number } } }) => void;

/** A point in segment `i`'s middle, in the window's points. */
const segment = (i: number) => {
  const a = (i * 60 * Math.PI) / 180;
  return { clientX: 160 + 112 * Math.sin(a), clientY: 160 - 112 * Math.cos(a) };
};

function setup(over: Partial<{ formats: FormatId[]; labels: string[]; listenHover: (h: (i: number | null) => void) => Promise<() => void> }> = {}) {
  let handler: Handler = () => {};
  const onPick = vi.fn();
  const onDrop = vi.fn();
  const listenDrag = (h: Handler) => { handler = h; return Promise.resolve(() => {}); };
  const view = render(Wheel, { formats, labels, hub: "MOV · 1 FILE", listenDrag, onPick, onDrop, ...over });
  return { ...view, onPick, onDrop, drag: (e: Parameters<Handler>[0]) => handler(e) };
}

describe("the wheel", () => {
  it("draws six labelled segments and the hub", () => {
    const { container } = setup();
    expect(container.querySelectorAll("path.segment").length).toBe(6);
    for (const badge of ["MP4", "GIF", "MP3", "WEBM", "MOV", "M4A"]) expect(screen.getByText(badge)).toBeTruthy();
    expect(screen.getByText("MOV · 1 FILE")).toBeTruthy();
  });

  it("names the hovered segment under the hub", async () => {
    setup();
    await fireEvent.mouseMove(window, segment(0));
    expect(screen.getByText("MPEG-4 · H.264")).toBeTruthy();
    await fireEvent.mouseMove(window, segment(3));
    expect(screen.getByText("WebM · VP9")).toBeTruthy();
    expect(screen.queryByText("MPEG-4 · H.264")).toBeNull();
  });

  it("hovers with a drag too, at its point", async () => {
    const { drag } = setup();
    drag({ payload: { type: "over", position: { x: 160, y: 40 } } });
    await tick();
    expect(screen.getByText("MPEG-4 · H.264")).toBeTruthy();
  });

  it("picks the clicked segment, or the hub", async () => {
    const { onPick } = setup();
    await fireEvent.click(window, segment(2));
    expect(onPick).toHaveBeenLastCalledWith(2);
    await fireEvent.click(window, { clientX: 160, clientY: 160 });
    expect(onPick).toHaveBeenLastCalledWith(null);
  });

  it("cancels on Esc and on a click outside the circle", async () => {
    const { onPick } = setup();
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(onPick).toHaveBeenLastCalledWith(-1);
    onPick.mockClear();
    await fireEvent.click(window, { clientX: 5, clientY: 340 });
    expect(onPick).toHaveBeenLastCalledWith(-1);
  });

  it("cancels when it loses the keyboard (a click elsewhere)", async () => {
    const { onPick } = setup();
    await fireEvent.blur(window);
    expect(onPick).toHaveBeenLastCalledWith(-1);
  });

  it("hands a drop to the app with its point, in points", () => {
    const { drag, onDrop } = setup({ });
    drag({ payload: { type: "drop", paths: ["/x/a.mov"], position: { x: 160, y: 40 } } });
    expect(onDrop).toHaveBeenCalledWith(["/x/a.mov"], 160, 40);
  });

  it("reads a drag's position as points, on a Retina display too", async () => {
    // macOS's webview reports a drag's place in points, though Tauri calls
    // it physical: dividing by the display's scale halves it.
    Object.defineProperty(window, "devicePixelRatio", { value: 2, configurable: true });
    const { drag, onDrop } = setup();
    drag({ payload: { type: "over", position: { x: 160, y: 280 } } });
    await tick();
    expect(screen.getByText("WebM · VP9")).toBeTruthy();
    drag({ payload: { type: "drop", paths: ["/x/a.mov"], position: { x: 160, y: 280 } } });
    expect(onDrop).toHaveBeenCalledWith(["/x/a.mov"], 160, 280);
    Object.defineProperty(window, "devicePixelRatio", { value: 1, configurable: true });
  });

  /** A point `deg` clockwise from 12 o'clock, in the ring. */
  const around = (deg: number) => {
    const a = (deg * Math.PI) / 180;
    return { clientX: 160 + 112 * Math.sin(a), clientY: 160 - 112 * Math.cos(a) };
  };

  it("shares the whole circle among five formats", async () => {
    const { container, onPick } = setup({ formats: formats.slice(0, 5), labels: labels.slice(0, 5) });
    expect(container.querySelectorAll("path.segment").length).toBe(5);
    await fireEvent.click(window, around(33));
    expect(onPick).toHaveBeenLastCalledWith(0);
    await fireEvent.click(window, around(300));
    expect(onPick).toHaveBeenLastCalledWith(4);
    await fireEvent.mouseMove(window, around(170));
    expect(screen.getByText("MP3 · audio")).toBeTruthy();
  });

  it("gives one format the whole ring", async () => {
    const { container, onPick } = setup({ formats: ["pdf"], labels: ["PDF"] });
    const paths = container.querySelectorAll("path.segment");
    expect(paths.length).toBe(1);
    // Two half-circle arcs each way: an SVG arc can't go all the way round.
    expect((paths[0].getAttribute("d") ?? "").match(/A /g)?.length).toBe(4);
    await fireEvent.click(window, around(200));
    expect(onPick).toHaveBeenLastCalledWith(0);
  });

  it("lights the segment the app says the pointer is over", async () => {
    let tell: (i: number | null) => void = () => {};
    const listenHover = (h: (i: number | null) => void) => { tell = h; return Promise.resolve(() => {}); };
    setup({ listenHover });
    tell(3);
    await tick();
    expect(screen.getByText("WebM · VP9")).toBeTruthy();
    tell(null);
    await tick();
    expect(screen.queryByText("WebM · VP9")).toBeNull();
  });

  it("shows the hint bar", () => {
    setup();
    expect(screen.getByText("⇧ Formats · esc Cancel")).toBeTruthy();
  });

  it("with no formats, draws the hub alone", () => {
    const { container } = setup({ formats: [], labels: [] });
    expect(container.querySelectorAll("path.segment").length).toBe(0);
    expect(screen.getByText("MOV · 1 FILE")).toBeTruthy();
  });
});
