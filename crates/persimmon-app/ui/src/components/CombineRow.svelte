<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- A Combine row: one output from many inputs, listed in order (spec D1). -->
<script lang="ts">
  import QueueRow from "./QueueRow.svelte";
  import Icon from "./Icon.svelte";
  import type { RowId, RowView } from "../lib/types";

  let {
    row, selected, onCancel, onRetry, onReveal, onSelect, onReorder, onRemoveInput,
  }: {
    row: RowView; selected: boolean;
    onCancel: (id: RowId) => void; onRetry: (id: RowId) => void; onReveal: (id: RowId) => void;
    onSelect: (id: RowId, e: MouseEvent) => void;
    onReorder: (id: RowId, from: number, to: number) => void;
    onRemoveInput: (id: RowId, index: number) => void;
  } = $props();

  let open = $state(false);
  const editable = $derived(row.state === "queued" || row.state === "failed");
  const base = (p: string) => p.split("/").pop() ?? p;

  function key(e: KeyboardEvent, i: number) {
    if (!e.altKey || !editable) return;
    if (e.key === "ArrowDown" && i < row.inputs.length - 1) { e.preventDefault(); onReorder(row.id, i, i + 1); }
    if (e.key === "ArrowUp" && i > 0) { e.preventDefault(); onReorder(row.id, i, i - 1); }
  }

  let dragging: number | null = $state(null);
</script>

<div class="combine">
  <button class="disclose" aria-expanded={open} aria-label="Show the {row.inputs.length} files in {row.name}"
    onclick={() => (open = !open)}>
    <span class="chev" class:open>›</span>
  </button>
  <div class="main"><QueueRow {row} {selected} {onCancel} {onRetry} {onReveal} {onSelect} /></div>
</div>
{#if open}
  <ol class="inputs" aria-label="Files in {row.name}, in order">
    {#each row.inputs as input, i (input)}
      <li>
        <span class="n">{i + 1}</span>
        <span class="name handle" role="button" tabindex="0" draggable={editable}
          aria-label="Move {base(input)}, {i + 1} of {row.inputs.length}: ⌥↑ or ⌥↓"
          onkeydown={(e) => key(e, i)}
          ondragstart={() => (dragging = i)}
          ondragover={(e) => e.preventDefault()}
          ondrop={(e) => { e.preventDefault(); if (dragging !== null && dragging !== i) onReorder(row.id, dragging, i); dragging = null; }}>{base(input)}</span>
        {#if editable}
          <button class="remove" aria-label="Remove {base(input)}" onclick={() => onRemoveInput(row.id, i)}><Icon name="close" size={12} /></button>
        {/if}
      </li>
    {/each}
  </ol>
{/if}

<style>
  .combine { display: flex; align-items: stretch; }
  .main { flex: 1; min-width: 0; }
  .disclose {
    width: 24px; border: 0; border-bottom: 1px solid var(--line); background: transparent;
    color: var(--muted); font: inherit; font-size: 16px; padding: 0;
  }
  .disclose:focus-visible, .remove:focus-visible, .handle:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .chev { display: inline-block; transition: transform 0.15s; }
  .chev.open { transform: rotate(90deg); }
  @media (prefers-reduced-motion: reduce) { .chev { transition: none; } }
  .inputs { list-style: none; margin: 0; padding: var(--space-1) 0 var(--space-2) 64px; border-bottom: 1px solid var(--line); background: var(--surface); }
  li { display: flex; align-items: center; gap: var(--space-3); padding: 3px var(--space-3); border-radius: var(--radius-control); }
  .n { font-family: var(--font-data); font-size: var(--text-label); color: var(--faint); width: 2ch; text-align: right; }
  .name { font-family: var(--font-data); font-size: var(--text-meta); color: var(--muted); flex: 1; }
  .remove { display: flex; padding: 2px; color: var(--muted); background: transparent; border: 1px solid var(--control-border); border-radius: var(--radius-control); }
</style>
