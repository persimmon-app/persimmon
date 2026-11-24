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
  import FirstRun from "./components/FirstRun.svelte";
  import { rowToCancel, upsert as merge, without } from "./lib/rows";
  import StatusBar from "./components/StatusBar.svelte";
  import ConfirmQuit from "./components/ConfirmQuit.svelte";
  import HistoryView from "./components/HistoryView.svelte";
  import WatchView from "./components/WatchView.svelte";
  import RuleEditor from "./components/RuleEditor.svelte";
  import { midnight } from "./lib/watch";
  import * as ipc from "./lib/ipc";
  import { badge, noteMessage, plural } from "./lib/format";
  import { lastUndoable as newestUndoable, undoMessage } from "./lib/undo";
  import { clicked, moved } from "./lib/selection";
  import { queueOwnsKey } from "./lib/keys";
  import type { BatchView, FormatId, HistoryFilter, NotificationsState, PresetView, QueueMode, Rule, RowId, RowView, RuleDraftView, RuleView, SettingsView, Shown, UndoReport } from "./lib/types";

  let rows: RowView[] = $state([]);
  let selected: RowId[] = $state([]);
  let anchor: RowId | null = $state(null);
  let targets: FormatId[] = $state([]);
  let message: string | null = $state(null);
  let confirmQuit = $state(false);
  let quitRules = $state(0);
  let focusBatch: string | null = $state(null);
  // A notification's batch is shown once: leaving History forgets it.
  $effect(() => { if (view !== "history") focusBatch = null; });
  let settings = $state<SettingsView | null>(null);
  /** The first-run card, decided once when the settings first arrive, so
   * its confirmation stays after it's been answered. */
  let firstRun = $state(false);
  /** The wheel tip, for someone who had Persimmon before the wheel. */
  const showTip = $derived(!!settings && !settings.newInstall && !settings.wheelTipSeen && !settings.wheel);
  const mode: QueueMode = $derived(settings?.mode ?? "convert");
  let picking = false;
  let presets: PresetView[] = $state([]);
  let view: "queue" | "history" | "watch" = $state("queue");
  let rules: RuleView[] = $state([]);
  let notifications: NotificationsState = $state("allowed");
  let editor: { draft: RuleDraftView; isNew: boolean } | null = $state(null);
  let watchTargets: FormatId[] = $state([]);
  let loginError: string | null = $state(null);
  const activeRules = $derived(rules.filter((r) => r.enabled).length);
  let batches: BatchView[] = $state([]);
  let historyText = $state("");
  let historyFilter: HistoryFilter = $state("all");
  let report: UndoReport | null = $state(null);
  /** Whether the newest batch can still be undone, for the status bar. */
  let lastUndoable = $state(false);

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
    view = "queue";
  }

  /** A preset becomes active for new files and applies to the selected
   * rows, or every row when none is selected (spec D4). */
  async function choosePreset(id: string) {
    settings = await ipc.usePreset(id);
    view = "queue";
    const ids = $state.snapshot(scope);
    if (!ids.length) { message = null; return; }
    const changed = await ipc.applyPreset(ids, id);
    changed.forEach(upsert);
    const misfits = changed.filter((r) => r.state === "failed").length;
    const name = presets.find((p) => p.id === id)?.name ?? "";
    message = misfits ? `${plural(misfits, "file")} ${misfits === 1 ? "doesn't" : "don't"} fit ${name}` : null;
  }

  async function refreshWatch() {
    rules = await ipc.watchRules(midnight(new Date()));
    notifications = await ipc.notifications();
  }

  /** A new rule: pick its folder, then edit it before it's saved. */
  async function newRule() {
    const folder = await ipc.pickWatchFolder();
    if (!folder) return;
    watchTargets = await ipc.watchTargets();
    const target = watchTargets.includes("webp") ? "webp" : (watchTargets[0] ?? "webp");
    editor = { draft: await ipc.newRule(folder, target), isNew: true };
  }

  async function editRule(id: string) {
    const v = rules.find((r) => r.id === id);
    if (!v) return;
    watchTargets = await ipc.watchTargets();
    const rule: Rule = { id: v.id, folder: v.folder, target: v.target, preset: v.preset, patterns: v.patterns, results: v.results,
      done: v.done, recursive: v.recursive, settle: v.settle, notifyEach: v.notifyEach, enabled: v.enabled };
    editor = { draft: { rule, command: v.command, existing: 0 }, isNew: false };
  }

  async function saveRule(rule: Rule, existing: boolean): Promise<Shown | null> {
    try { rules = await ipc.saveRule(rule, existing); editor = null; return null; } catch (e) { return e as Shown; }
  }

  async function startAtLogin(on: boolean) {
    try { settings = await ipc.setStartAtLogin(on); loginError = null; } catch (e) { loginError = String(e); }
  }

  async function refreshHistory() {
    batches = await ipc.history(historyText, historyFilter);
    lastUndoable = newestUndoable(await ipc.history("", "all"));
  }

  async function undo(id: string | null) {
    showReport(id ? await ipc.undoBatch(id) : await ipc.undoLast());
    await refreshHistory();
  }

  function showReport(r: UndoReport | null) {
    report = r;
    if (r) message = undoMessage(r);
  }

  /** Reveal in Finder, or say why it can't. */
  async function reveal(next: Promise<string | null>) {
    const why = await next;
    if (why) message = why;
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

  let grid: HTMLDivElement | undefined = $state();
  function select(id: RowId, mode: "replace" | "range" | "toggle") {
    ({ selected, anchor } = clicked(rows.map((r) => r.id), { selected, anchor }, id, mode));
    // Back to the grid, even from a row's button: VoiceOver follows the
    // grid's active row only while the grid has focus.
    grid?.focus();
  }

  function move(by: number, extend: boolean) {
    ({ selected, anchor } = moved(rows.map((r) => r.id), { selected, anchor }, by, extend));
  }

  const chosen = () => rows.filter((r) => selected.includes(r.id));

  function onKey(e: KeyboardEvent) {
    if (!queueOwnsKey(e, confirmQuit)) return;
    const cmd = e.metaKey;
    if (cmd && e.ctrlKey && ["1", "2", "3", "4"].includes(e.key)) {
      e.preventDefault();
      chooseMode((["convert", "compress", "extract", "combine"] as const)[Number(e.key) - 1]);
    }
    else if (cmd && !e.ctrlKey && /^[1-9]$/.test(e.key)) {
      const p = presets.find((x) => x.shortcut === Number(e.key));
      if (p) { e.preventDefault(); choosePreset(p.id); }
    }
    else if (view !== "queue" && e.key === "Escape" && !editor) { view = "queue"; }
    else if (cmd && e.key === "o") { e.preventDefault(); if (!e.repeat) pick(); }
    else if (cmd && e.key === "Enter") { e.preventDefault(); if (canStart) ipc.start(); }
    else if (cmd && e.key === "a") { e.preventDefault(); selected = rows.map((r) => r.id); }
    else if (cmd && e.key === ".") { e.preventDefault(); const r = rowToCancel(rows, anchor); if (r) ipc.cancelRow(r.id); }
    else if (cmd && e.shiftKey && e.key.toLowerCase() === "r") { e.preventDefault(); chosen().filter((r) => r.state === "done").forEach((r) => reveal(ipc.revealRow(r.id))); }
    else if (cmd && e.key === "r") { e.preventDefault(); chosen().filter((r) => (r.state === "failed" || r.state === "cancelled") && r.inputs.length > 0).forEach((r) => ipc.retryRow(r.id)); }
    else if (e.key === "Backspace" || e.key === "Delete") { if (selected.length) { e.preventDefault(); remove(selected); } }
    else if (e.key === "ArrowDown") { e.preventDefault(); move(1, e.shiftKey); }
    else if (e.key === "ArrowUp") { e.preventDefault(); move(-1, e.shiftKey); }
    else if (e.key === "Escape") { selected = []; anchor = null; }
  }

  onMount(() => {
    ipc.queueRows().then((r) => r.forEach(upsert));
    ipc.getSettings().then((s) => { settings = s; firstRun = !!s && s.newInstall && !s.firstRunSeen; });
    ipc.presets().then((p) => (presets = p));
    refreshHistory();
    refreshWatch();
    const off = [
      ipc.onWatch(() => refreshWatch()),
      ipc.onHistory(() => refreshHistory()),
      ipc.onHistoryCleared(() => refreshHistory()),
      ipc.onPresets((p) => (presets = p)),
      ipc.onQueueNote((note) => (message = noteMessage(note))),
      // ⌥⌘Z is the Edit menu's (Undo Last Batch), from any window.
      ipc.onUndoReport((r) => { showReport(r); refreshHistory(); }),
      ipc.onRow(upsert),
      ipc.onConfirmQuit((p) => { quitRules = p?.rules ?? 0; confirmQuit = true; }),
      // A notification's Undo opens History at that batch (spec D9).
      ipc.onShowBatch((id) => { historyText = ""; historyFilter = "all"; focusBatch = id; view = "history"; refreshHistory(); }),
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
  <Sidebar {mode} onMode={chooseMode} preset={settings?.preset ?? null} {presets} onPreset={choosePreset}
    {view} onHistory={() => { view = view === "history" ? "queue" : "history"; refreshHistory(); }}
    onWatch={() => { view = view === "watch" ? "queue" : "watch"; refreshWatch(); }} watchBadge={activeRules} />
  <div class="main">
    <Toolbar count={rows.length} {totalBytes} {targets} {current} {canStart}
      onAdd={pick} onPick={pickTarget} onStart={() => ipc.start()} />
    {#if view === "watch"}
      <WatchView views={rules} {notifications} startAtLogin={settings?.startAtLogin ?? false} {loginError}
        onNew={newRule} onEdit={editRule}
        onToggle={async (id, on) => (rules = await ipc.toggleRule(id, on))}
        onCopy={(id) => ipc.copyCommand(id).then((ok) => { message = ok ? "Command copied" : null; })}
        onScan={(id) => ipc.scanRule(id)}
        onRemove={async (id) => (rules = await ipc.removeRule(id))}
        onStartAtLogin={startAtLogin} onOpenNotifications={() => ipc.openNotificationSettings()} />
    {:else if view === "history"}
      <HistoryView {batches} text={historyText} filter={historyFilter} {report} focus={focusBatch}
        onSearch={(t) => { historyText = t; refreshHistory(); }}
        onFilter={(f) => { historyFilter = f; refreshHistory(); }}
        onUndo={(id) => undo(id)} onReveal={(p) => reveal(ipc.revealPath(p))}
        onRetry={async (b, i) => { const r = await ipc.retryItem(b, i); r.forEach(upsert); if (r.length) view = "queue"; }}
        onClear={async () => { await ipc.clearHistory(); report = null; refreshHistory(); }}
        onDismissReport={() => (report = null)} />
    {:else if rows.length === 0}
      <div class="queue">
        {#if firstRun}
          <FirstRun onWheel={(on) => ipc.setWheel(on).then((s) => (settings = s))}
            onDismiss={() => ipc.dismissCard("firstRun").then((s) => (settings = s))} />
        {/if}
        <EmptyState onAdd={pick} showTip={showTip && !firstRun}
          onTurnOn={async () => { settings = await ipc.setWheel(true); settings = await ipc.dismissCard("wheelTip"); }}
          onDismissTip={() => ipc.dismissCard("wheelTip").then((s) => (settings = s))} />
      </div>
    {:else}
      <!-- A click on the queue's empty space clears the selection, so the
           Output menu acts on every row again (spec D1). -->
      <!-- A grid, not a listbox: rows hold buttons, which an option can't.
           It keeps focus and points at the active row, so VoiceOver reads
           each row the arrow keys reach. -->
      <div class="queue" role="grid" aria-label="Queue" aria-multiselectable="true" tabindex="0" bind:this={grid}
        aria-activedescendant={anchor !== null && rows.some((r) => r.id === anchor) ? `row-${anchor}` : undefined}
        onclick={(e) => { if (e.target === e.currentTarget) { selected = []; anchor = null; } }}
        onkeydown={() => {}}>
        {#each rows as row (row.id)}
          {#if row.op === "merge"}
            <CombineRow {row} selected={selected.includes(row.id)} active={anchor === row.id}
              onSelect={(id, e) => select(id, e.shiftKey ? "range" : e.metaKey ? "toggle" : "replace")}
              onCancel={(id) => ipc.cancelRow(id)} onRetry={(id) => ipc.retryRow(id)} onReveal={(id) => reveal(ipc.revealRow(id))}
              onReorder={reorder} onRemoveInput={removeInput} />
          {:else}
            <QueueRow {row} selected={selected.includes(row.id)} active={anchor === row.id}
              onSelect={(id, e) => select(id, e.shiftKey ? "range" : e.metaKey ? "toggle" : "replace")}
              onCancel={(id) => ipc.cancelRow(id)} onRetry={(id) => ipc.retryRow(id)} onReveal={(id) => reveal(ipc.revealRow(id))} />
          {/if}
        {/each}
      </div>
    {/if}
    <StatusBar {rows} {message} output={settings?.outputLabel ?? "beside original"} onOutput={() => ipc.openSettings()}
      undoable={lastUndoable} onUndoLast={() => undo(null)} />
  </div>
</div>
{#if editor}
  <RuleEditor draft={editor.draft} isNew={editor.isNew} targets={watchTargets} {presets}
    onDraft={(r) => ipc.ruleDraft(r)} onDefaults={(t) => ipc.watchPatterns(t)}
    onChoose={() => ipc.pickWatchFolder()} onSave={saveRule} onCancel={() => (editor = null)} />
{/if}
<ConfirmQuit open={confirmQuit} running={rows.filter((r) => r.state === "running").length} rules={quitRules} onCancel={() => (confirmQuit = false)} onQuit={() => ipc.quitNow()} />

<style>
  .window { height: 100vh; display: flex; }
  .main { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .queue { flex: 1; overflow-y: auto; }
</style>
