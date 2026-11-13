<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- A Combine row: one output from many inputs, listed in order (spec D1). -->
<script lang="ts">
  import QueueRow from "./QueueRow.svelte";
  import Icon from "./Icon.svelte";
  import type { RowId, RowView } from "../lib/types";

  let {
    row, selected, active = false, onCancel, onRetry, onReveal, onSelect, onReorder, onRemoveInput,
  }: {
    row: RowView; selected: boolean; active?: boolean;
    onCancel: (id: RowId) => void; onRetry: (id: RowId) => void; onReveal: (id: RowId) => void;
    onSelect: (id: RowId, e: MouseEvent) => void;
    onReorder: (id: RowId, from: number, to: number) => void;
    onRemoveInput: (id: RowId, index: number) => void;
  } = $props();

  let open = $state(false);
  const editable = $derived(row.state === "queued" || row.state === "failed");
  const tab = $derived(active ? 0 : -1);
  const base = (p: string) => p.split("/").pop() ?? p;

  /** The input picked up with Enter or Space, moved with ↑/↓ until put
   * down; ⌥↑/⌥↓ move without picking up. None of these keys reach the
   * queue, whose own ↑/↓ move the selection. */
  let moving: number | null = $state(null);
  function key(e: KeyboardEvent, i: number) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault(); e.stopPropagation();
      if (editable) moving = moving === i ? null : i;
      return;
    }
    const down = e.key === "ArrowDown";
    if (!(down || e.key === "ArrowUp") || !(e.altKey || moving === i)) return;
    e.preventDefault(); e.stopPropagation();
    const to = down ? i + 1 : i - 1;
    if (!editable || to < 0 || to >= row.inputs.length) return;
    onReorder(row.id, i, to);
    if (moving === i) moving = to;
  }

  let dragging: number | null = $state(null);
</script>

<QueueRow {row} {selected} {active} {onCancel} {onRetry} {onReveal} {onSelect}>
  {#snippet lead()}
    <button class="disclose" tabindex={tab} aria-expanded={open} aria-label="Show the {row.inputs.length} files in {row.name}"
      onclick={(e) => { e.stopPropagation(); open = !open; }}>
      <span class="chev" class:open>›</span>
    </button>
  {/snippet}
  {#snippet after()}
    {#if open}
      <ol class="inputs" aria-label="Files in {row.name}, in order">
        {#each row.inputs as input, i (input)}
          <li>
            <span class="n">{i + 1}</span>
            <span class="name handle" class:moving={moving === i} role="button" tabindex={tab} draggable={editable}
              aria-pressed={moving === i}
              aria-label="Move {base(input)}, {i + 1} of {row.inputs.length}: ⌥↑ or ⌥↓, or Enter to pick up"
              onkeydown={(e) => key(e, i)}
              ondragstart={() => (dragging = i)}
              ondragend={() => (dragging = null)}
              ondragover={(e) => e.preventDefault()}
              ondrop={(e) => { e.preventDefault(); if (dragging !== null && dragging !== i) onReorder(row.id, dragging, i); dragging = null; }}>{base(input)}</span>
            {#if editable}
              <button class="remove" tabindex={tab} aria-label="Remove {base(input)}" onclick={(e) => { e.stopPropagation(); onRemoveInput(row.id, i); }}><Icon name="close" size={12} /></button>
            {/if}
          </li>
        {/each}
      </ol>
    {/if}
  {/snippet}
</QueueRow>

<style>
  .disclose {
    width: 24px; border: 0; border-bottom: 1px solid var(--line); background: transparent;
    color: var(--muted); font: inherit; font-size: 16px; padding: 0;
  }
  .disclose:focus-visible, .remove:focus-visible, .handle:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .handle.moving { outline: 1px dashed var(--accent); outline-offset: -2px; }
  .chev { display: inline-block; transition: transform 0.15s; }
  .chev.open { transform: rotate(90deg); }
  @media (prefers-reduced-motion: reduce) { .chev { transition: none; } }
  .inputs { list-style: none; margin: 0; padding: var(--space-1) 0 var(--space-2) 64px; border-bottom: 1px solid var(--line); background: var(--surface); }
  li { display: flex; align-items: center; gap: var(--space-3); padding: 3px var(--space-3); border-radius: var(--radius-control); }
  .n { font-family: var(--font-data); font-size: var(--text-label); color: var(--faint); width: 2ch; text-align: right; }
  .name { font-family: var(--font-data); font-size: var(--text-meta); color: var(--muted); flex: 1; }
  .remove { display: flex; padding: 2px; color: var(--muted); background: transparent; border: 1px solid var(--control-border); border-radius: var(--radius-control); }
</style>
