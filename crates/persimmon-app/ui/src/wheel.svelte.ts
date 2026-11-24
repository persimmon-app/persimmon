// SPDX-License-Identifier: GPL-3.0-or-later
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import "./tokens.css";
import { mount } from "svelte";

if (import.meta.env.DEV) {
  const { installDevMock } = await import("./lib/devmock");
  installDevMock();
}
import { getCurrentWebview } from "@tauri-apps/api/webview";
import Wheel from "./Wheel.svelte";
import * as ipc from "./lib/ipc";
import type { FormatId } from "./lib/types";

type DragEvent = { payload: { type: string; paths?: string[]; position?: { x: number; y: number } } };
const target = document.getElementById("app");
if (!target) throw new Error("no #app element");
const props = $state({
  formats: [] as FormatId[],
  labels: [] as string[],
  hub: "",
  scale: window.devicePixelRatio || 1,
  listenDrag: (h: (e: DragEvent) => void) =>
    getCurrentWebview().onDragDropEvent((e) => h(e as unknown as DragEvent)),
  onPick: (index: number | null) => { ipc.wheelPick(index); },
  onDrop: (paths: string[], x: number, y: number) => { ipc.wheelDrop(paths, x, y); },
});
ipc.onWheelShow((w) => {
  props.formats = w.formats;
  props.labels = w.labels;
  props.hub = w.hub;
  props.scale = window.devicePixelRatio || 1;
});
export default mount(Wheel, { target, props });
