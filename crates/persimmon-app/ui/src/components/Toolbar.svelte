<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<script lang="ts">
  import Icon from "./Icon.svelte";
  import { badge, plural, size } from "../lib/format";
  import type { FormatId } from "../lib/types";

  let {
    count, totalBytes, targets, current, canStart, onAdd, onPick, onStart,
  }: {
    count: number; totalBytes: number; targets: FormatId[]; current: FormatId | null;
    canStart: boolean; onAdd: () => void; onPick: (to: FormatId) => void; onStart: () => void;
  } = $props();
</script>

<div class="toolbar">
  <button onclick={onAdd} title="Add files (⌘O)"><Icon name="add" size={14} /> Add files</button>
  <label class="output">
    <span>Output</span>
    <select aria-label="Output format" disabled={targets.length === 0}
      value={current ?? ""} onchange={(e) => { const v = e.currentTarget.value; if (v) onPick(v); }}>
      <option value="" disabled>{current === null && targets.length > 0 ? "Mixed" : "—"}</option>
      {#each targets as t (t)}<option value={t}>{badge(t)}</option>{/each}
    </select>
  </label>
  <span class="spacer"></span>
  {#if count > 0}<span class="total">{plural(count, "file")} · {size(totalBytes)}</span>{/if}
  <button class="start" disabled={!canStart} onclick={onStart} title="Start (⌘↩)">Start</button>
</div>

<style>
  .toolbar {
    height: var(--toolbar);
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0 var(--space-4);
    border-bottom: 1px solid var(--line);
    background: var(--surface);
  }
  .output { display: flex; align-items: center; gap: var(--space-2); font-size: var(--text-meta); color: var(--muted); }
  select {
    font-family: var(--font-data);
    font-size: var(--text-meta);
    color: var(--text);
    background: var(--ground);
    border: 1px solid var(--control-border);
    border-radius: var(--radius-control);
    height: var(--button-toolbar);
    padding: 0 var(--space-2);
  }
  .spacer { flex: 1; }
  .total { font-family: var(--font-data); font-size: var(--text-meta); color: var(--muted); }
  button {
    display: inline-flex; align-items: center; gap: var(--space-1);
    font: inherit; font-size: var(--text-body); color: var(--text);
    height: var(--button-toolbar); padding: 0 var(--space-3);
    background: transparent; border: 1px solid var(--control-border); border-radius: var(--radius-control);
  }
  .start { background: var(--accent); border-color: var(--accent); color: var(--on-accent); font-weight: 600; }
  .start:disabled { background: transparent; border-color: var(--line); color: var(--faint); }
  button:focus-visible, select:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
