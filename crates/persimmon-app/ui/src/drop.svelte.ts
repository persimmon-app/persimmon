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
import Drop from "./Drop.svelte";
import * as ipc from "./lib/ipc";
import type { RowView } from "./lib/types";

const target = document.getElementById("app");
if (!target) throw new Error("no #app element");
const props = $state({
  listenDrag: (h: (e: { payload: { type: string; paths?: string[] } }) => void) =>
    getCurrentWebview().onDragDropEvent((e) => h(e as unknown as { payload: { type: string; paths?: string[] } })),
  onDrop: (paths: string[]) => { ipc.dropPaths(paths); },
  running: 0,
});
const rows = new Map<number, RowView>();
const count = () => { props.running = [...rows.values()].filter((r) => r.state === "running").length; };
ipc.queueRows().then((all) => { all.forEach((r) => rows.set(r.id, r)); count(); });
ipc.onRow((r) => { const old = rows.get(r.id); if (!old || old.rev <= r.rev) rows.set(r.id, r); count(); });
export default mount(Drop, { target, props });
