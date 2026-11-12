<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- The main window: toolbar, queue, status bar (canvas "macOS — Main window"). -->
<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import Toolbar from "./components/Toolbar.svelte";
  import QueueRow from "./components/QueueRow.svelte";
  import CombineRow from "./components/CombineRow.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import EmptyState from "./components/EmptyState.svelte";
  import { upsert as merge, without } from "./lib/rows";
  import StatusBar from "./components/StatusBar.svelte";
  import ConfirmQuit from "./components/ConfirmQuit.svelte";
  import * as ipc from "./lib/ipc";
  import { badge, plural } from "./lib/format";
  import { clicked, moved } from "./lib/selection";
  import { queueOwnsKey } from "./lib/keys";
  import type { FormatId, QueueMode, RowId, RowView, SettingsView } from "./lib/types";

  let rows: RowView[] = $state([]);
  let selected: RowId[] = $state([]);
  let anchor: RowId | null = $state(null);
  let targets: FormatId[] = $state([]);
  let message: string | null = $state(null);
  let confirmQuit = $state(false);
  let settings = $state<SettingsView | null>(null);
  const mode: QueueMode = $derived(settings?.mode ?? "convert");
  let picking = false;

  /** The rows the toolbar acts on: the selection, else every row. */
  const scope = $derived(selected.length ? selected : rows.map((r) => r.id));
  const current = $derived.by(() => {
    const tos = new Set(rows.filter((r) => scope.includes(r.id) && r.op === "convert").map((r) => r.to));
    return tos.size === 1 ? ([...tos][0] ?? null) : null;
  });
  const canStart = $derived(rows.some((r) => r.state === "queued"));
  const totalBytes = $derived(rows.reduce((n, r) => n + r.size, 0));

  function upsert(row: RowView) {
    rows = merge(rows, row);
  }

  /** The open panel, once at a time: a repeated ⌘O or a second click while
   * it's open does nothing. */
  async function pick() {
    if (picking) return;
    picking = true;
    try { (await ipc.pickFiles()).forEach(upsert); } finally { picking = false; }
  }

  async function chooseMode(m: QueueMode) {
    settings = await ipc.setMode(m);
  }

  function reorder(id: RowId, from: number, to: number) {
    ipc.reorderInput(id, from, to).then((r) => r && upsert(r));
  }

  function removeInput(id: RowId, index: number) {
    ipc.removeInput(id, index).then((r) => { if (r) upsert(r); else rows = without(rows, [id]); });
  }

  $effect(() => {
    const ids = $state.snapshot(scope);
    ipc.targets(ids).then((t) => (targets = t), () => (targets = []));
  });

  async function add(paths: string[]) {
    if (paths.length) (await ipc.addPaths(paths)).forEach(upsert);
  }

  async function pickTarget(to: FormatId) {
    const result = await ipc.setTarget($state.snapshot(scope), to);
    result.changed.forEach(upsert);
    message = result.skipped ? `${plural(result.skipped, "file")} can't become ${badge(to)}` : null;
  }

  async function remove(ids: RowId[]) {
    const gone = await ipc.removeRows(ids);
    rows = without(rows, gone);
    selected = selected.filter((id) => !gone.includes(id));
  }

  function select(id: RowId, mode: "replace" | "range" | "toggle") {
    ({ selected, anchor } = clicked(rows.map((r) => r.id), { selected, anchor }, id, mode));
  }

  function move(by: number, extend: boolean) {
    ({ selected, anchor } = moved(rows.map((r) => r.id), { selected, anchor }, by, extend));
  }

  const running = () => rows.find((r) => r.state === "running");
  const chosen = () => rows.filter((r) => selected.includes(r.id));

  function onKey(e: KeyboardEvent) {
    if (!queueOwnsKey(e, confirmQuit)) return;
    const cmd = e.metaKey;
    if (cmd && e.ctrlKey && ["1", "2", "3", "4"].includes(e.key)) {
      e.preventDefault();
      chooseMode((["convert", "compress", "extract", "combine"] as const)[Number(e.key) - 1]);
    }
    else if (cmd && e.key === "o") { e.preventDefault(); if (!e.repeat) pick(); }
    else if (cmd && e.key === "Enter") { e.preventDefault(); if (canStart) ipc.start(); }
    else if (cmd && e.key === "a") { e.preventDefault(); selected = rows.map((r) => r.id); }
    else if (cmd && e.key === ".") { e.preventDefault(); const r = running(); if (r) ipc.cancelRow(r.id); }
    else if (cmd && e.shiftKey && e.key.toLowerCase() === "r") { e.preventDefault(); chosen().filter((r) => r.state === "done").forEach((r) => ipc.revealRow(r.id)); }
    else if (cmd && e.key === "r") { e.preventDefault(); chosen().filter((r) => r.state === "failed" || r.state === "cancelled").forEach((r) => ipc.retryRow(r.id)); }
    else if (e.key === "Backspace" || e.key === "Delete") { if (selected.length) { e.preventDefault(); remove(selected); } }
    else if (e.key === "ArrowDown") { e.preventDefault(); move(1, e.shiftKey); }
    else if (e.key === "ArrowUp") { e.preventDefault(); move(-1, e.shiftKey); }
    else if (e.key === "Escape") { selected = []; anchor = null; }
  }

  onMount(() => {
    ipc.queueRows().then((r) => r.forEach(upsert));
    ipc.getSettings().then((s) => (settings = s));
    const off = [
      ipc.onRow(upsert),
      ipc.onConfirmQuit(() => (confirmQuit = true)),
      ipc.onSettings((s) => (settings = s)),
      getCurrentWebview().onDragDropEvent((e) => {
        if (e.payload.type === "drop") add(e.payload.paths);
      }),
    ];
    return () => off.forEach((p) => p.then((f) => f()));
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="window">
  <Sidebar {mode} onMode={chooseMode} />
  <div class="main">
    <Toolbar count={rows.length} {totalBytes} {targets} {current} {canStart}
      onAdd={pick} onPick={pickTarget} onStart={() => ipc.start()} />
    {#if rows.length === 0}
      <div class="queue"><EmptyState onAdd={pick} /></div>
    {:else}
      <!-- A click on the queue's empty space clears the selection, so the
           Output menu acts on every row again (spec D1). -->
      <div class="queue" role="listbox" aria-label="Queue" aria-multiselectable="true" tabindex="-1"
        onclick={(e) => { if (e.target === e.currentTarget) { selected = []; anchor = null; } }}
        onkeydown={() => {}}>
        {#each rows as row (row.id)}
          {#if row.op === "merge"}
            <CombineRow {row} selected={selected.includes(row.id)}
              onSelect={(id, e) => select(id, e.shiftKey ? "range" : e.metaKey ? "toggle" : "replace")}
              onCancel={(id) => ipc.cancelRow(id)} onRetry={(id) => ipc.retryRow(id)} onReveal={(id) => ipc.revealRow(id)}
              onReorder={reorder} onRemoveInput={removeInput} />
          {:else}
            <QueueRow {row} selected={selected.includes(row.id)}
              onSelect={(id, e) => select(id, e.shiftKey ? "range" : e.metaKey ? "toggle" : "replace")}
              onCancel={(id) => ipc.cancelRow(id)} onRetry={(id) => ipc.retryRow(id)} onReveal={(id) => ipc.revealRow(id)} />
          {/if}
        {/each}
      </div>
    {/if}
    <StatusBar {rows} {message} output={settings?.outputLabel ?? "beside original"} onOutput={() => ipc.openSettings()} />
  </div>
</div>
<ConfirmQuit open={confirmQuit} onCancel={() => (confirmQuit = false)} onQuit={() => ipc.quitNow()} />

<style>
  .window { height: 100vh; display: flex; }
  .main { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .queue { flex: 1; overflow-y: auto; }
</style>
