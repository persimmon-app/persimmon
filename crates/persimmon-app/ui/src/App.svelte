<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- The main window: toolbar, queue, status bar (canvas "macOS — Main window"). -->
<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import Toolbar from "./components/Toolbar.svelte";
  import QueueRow from "./components/QueueRow.svelte";
  import StatusBar from "./components/StatusBar.svelte";
  import ConfirmQuit from "./components/ConfirmQuit.svelte";
  import * as ipc from "./lib/ipc";
  import { badge, plural } from "./lib/format";
  import type { FormatId, RowId, RowView } from "./lib/types";

  let rows: RowView[] = $state([]);
  let selected: RowId[] = $state([]);
  let anchor: RowId | null = $state(null);
  let targets: FormatId[] = $state([]);
  let message: string | null = $state(null);
  let confirmQuit = $state(false);

  /** The rows the toolbar acts on: the selection, else every row. */
  const scope = $derived(selected.length ? selected : rows.map((r) => r.id));
  const current = $derived.by(() => {
    const tos = new Set(rows.filter((r) => scope.includes(r.id) && r.op === "convert").map((r) => r.to));
    return tos.size === 1 ? ([...tos][0] ?? null) : null;
  });
  const canStart = $derived(rows.some((r) => r.state === "queued"));
  const totalBytes = $derived(rows.reduce((n, r) => n + r.size, 0));

  function upsert(row: RowView) {
    const i = rows.findIndex((r) => r.id === row.id);
    if (i === -1) rows.push(row);
    else rows[i] = row;
  }

  $effect(() => {
    const ids = $state.snapshot(scope);
    ipc.targets(ids).then((t) => (targets = t), () => (targets = []));
  });

  async function add(paths: string[]) {
    if (paths.length) (await ipc.addPaths(paths)).forEach(upsert);
  }

  async function pick(to: FormatId) {
    const result = await ipc.setTarget($state.snapshot(scope), to);
    result.changed.forEach(upsert);
    message = result.skipped ? `${plural(result.skipped, "file")} can't become ${badge(to)}` : null;
  }

  async function remove(ids: RowId[]) {
    const gone = await ipc.removeRows(ids);
    rows = rows.filter((r) => !gone.includes(r.id));
    selected = selected.filter((id) => !gone.includes(id));
  }

  function select(id: RowId, extend: boolean) {
    if (extend && anchor !== null) {
      const ids = rows.map((r) => r.id);
      const [a, b] = [ids.indexOf(anchor), ids.indexOf(id)].sort((x, y) => x - y);
      selected = ids.slice(a, b + 1);
    } else {
      selected = [id];
      anchor = id;
    }
  }

  function move(by: number, extend: boolean) {
    if (!rows.length) return;
    const ids = rows.map((r) => r.id);
    const last = selected.at(-1);
    const at = last === undefined
      ? (by > 0 ? 0 : ids.length - 1)
      : Math.min(Math.max(ids.indexOf(last) + by, 0), ids.length - 1);
    select(ids[at], extend);
  }

  const running = () => rows.find((r) => r.state === "running");
  const chosen = () => rows.filter((r) => selected.includes(r.id));

  function onKey(e: KeyboardEvent) {
    const cmd = e.metaKey;
    if (cmd && e.key === "o") { e.preventDefault(); ipc.pickFiles().then((r) => r.forEach(upsert)); }
    else if (cmd && e.key === "Enter") { e.preventDefault(); if (canStart) ipc.start(); }
    else if (cmd && e.key === "a") { e.preventDefault(); selected = rows.map((r) => r.id); }
    else if (cmd && e.key === ".") { e.preventDefault(); const r = running(); if (r) ipc.cancelRow(r.id); }
    else if (cmd && e.shiftKey && e.key.toLowerCase() === "r") { e.preventDefault(); chosen().filter((r) => r.state === "done").forEach((r) => ipc.revealRow(r.id)); }
    else if (cmd && e.key === "r") { e.preventDefault(); chosen().filter((r) => r.state === "failed" || r.state === "cancelled").forEach((r) => ipc.retryRow(r.id)); }
    else if (e.key === "Backspace" || e.key === "Delete") { if (selected.length) { e.preventDefault(); remove(selected); } }
    else if (e.key === "ArrowDown") { e.preventDefault(); move(1, e.shiftKey); }
    else if (e.key === "ArrowUp") { e.preventDefault(); move(-1, e.shiftKey); }
  }

  onMount(() => {
    ipc.queueRows().then((r) => (rows = r));
    const off = [
      ipc.onRow(upsert),
      ipc.onConfirmQuit(() => (confirmQuit = true)),
      getCurrentWebview().onDragDropEvent((e) => {
        if (e.payload.type === "drop") add(e.payload.paths);
      }),
    ];
    return () => off.forEach((p) => p.then((f) => f()));
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="window">
  <Toolbar count={rows.length} {totalBytes} {targets} {current} {canStart}
    onAdd={() => ipc.pickFiles().then((r) => r.forEach(upsert))} onPick={pick} onStart={() => ipc.start()} />
  <div class="queue" role="listbox" aria-label="Queue" aria-multiselectable="true">
    {#each rows as row (row.id)}
      <QueueRow {row} selected={selected.includes(row.id)} onSelect={select}
        onCancel={(id) => ipc.cancelRow(id)} onRetry={(id) => ipc.retryRow(id)} onReveal={(id) => ipc.revealRow(id)} />
    {:else}
      <p class="empty">Drop files here, or choose Add files (⌘O).</p>
    {/each}
  </div>
  <StatusBar {rows} {message} />
</div>
<ConfirmQuit open={confirmQuit} onCancel={() => (confirmQuit = false)} onQuit={() => ipc.quitNow()} />

<style>
  .window { height: 100vh; display: flex; flex-direction: column; }
  .queue { flex: 1; overflow-y: auto; }
  .empty { margin: var(--space-6); color: var(--muted); }
</style>
