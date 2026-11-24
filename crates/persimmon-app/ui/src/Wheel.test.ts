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

function setup(over: Partial<{ formats: FormatId[]; labels: string[] }> = {}) {
  let handler: Handler = () => {};
  const onPick = vi.fn();
  const onDrop = vi.fn();
  const listenDrag = (h: Handler) => { handler = h; return Promise.resolve(() => {}); };
  const view = render(Wheel, { formats, labels, hub: "MOV · 1 FILE", scale: 1, listenDrag, onPick, onDrop, ...over });
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
