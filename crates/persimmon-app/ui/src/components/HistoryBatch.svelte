<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- One batch in History (canvas "History"; spec D3). -->
<script lang="ts">
  import { size, plural } from "../lib/format";
  import type { BatchView } from "../lib/types";

  let { batch, onUndo, onReveal, onRetry = () => {} }: {
    batch: BatchView; onUndo: (id: string) => void; onReveal: (path: string) => void;
    /** Retry from History (spec D1): the batch and the item's index. */
    onRetry?: (batch: string, index: number) => void;
  } = $props();
  let open = $state(false);

  const base = (p: string | null) => (p ? (p.split("/").pop() ?? p) : "");
  const time = $derived(new Date(batch.finished * 1000).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" }));
  const target = $derived.by(() => {
    const out = batch.items.find((i) => i.output)?.output;
    const name = base(out ?? null);
    if (!name) return batch.mode.toUpperCase();
    const dot = name.indexOf(".");
    return dot > 0 ? name.slice(dot + 1).toUpperCase() : "FOLDER";
  });
  const source = $derived({ window: "WINDOW", drop: "DROP", opened: "OPEN WITH" }[batch.source]);
  const inBytes = $derived(batch.items.reduce((n, i) => n + i.size, 0));
  const outBytes = $derived(batch.items.reduce((n, i) => n + (i.outputSize ?? 0), 0));
  const done = $derived(batch.items.filter((i) => i.outcome === "done").length);
  const failed = $derived(batch.items.filter((i) => i.outcome === "failed").length);
  const duration = $derived.by(() => {
    const s = Math.max(0, batch.finished - batch.started);
    return s >= 60 ? `${Math.floor(s / 60)} min ${s % 60} s` : `${s} s`;
  });
</script>

<article class="batch" class:undone={batch.undone}>
  <div class="head">
    <button class="disclose" aria-expanded={open} aria-label="Show the {batch.items.length} files in this batch"
      onclick={() => (open = !open)}><span class="chev" class:open>›</span></button>
    <span class="time">{time}</span>
    <span class="what">{plural(batch.items.length, "file")} → {target}</span>
    <span class="tag">{source}</span>
    {#if batch.preset}<span class="preset">{batch.preset}</span>{/if}
    <span class="spacer"></span>
    {#if batch.undone}
      <span class="state">Undone</span>
    {:else if batch.undoable}
      <button class="text" aria-label="Undo batch from {time}" onclick={() => onUndo(batch.id)}>Undo batch</button>
    {/if}
  </div>
  <p class="summary">{size(inBytes)} → {size(outBytes)} · {done} done, {failed} failed · {duration}</p>
  {#if open}
    <ul class="files">
      {#each batch.items as item, i (i)}
        <li>
          <span class="names">{base(item.inputs[0])}{#if item.output}{" → "}{base(item.output)}{/if}</span>
          {#if item.outcome === "done"}<span class="sizes">{size(item.size)} → {size(item.outputSize ?? 0)}</span>{/if}
          {#if item.outcome === "failed"}<span class="why">{item.error.reason}</span>{/if}
          {#if item.outcome === "cancelled"}<span class="state">Cancelled</span>{/if}
          {#if item.outcome === "failed" || item.outcome === "cancelled"}
            <button class="text" aria-label="Retry {base(item.inputs[0])}" onclick={() => onRetry(batch.id, i)}>Retry</button>
          {/if}
          {#if item.undone}<span class="state">In the Trash</span>
          {:else if item.output && item.outcome === "done"}
            <button class="text" aria-label="Reveal {base(item.output)} in Finder" onclick={() => onReveal(item.output!)}>Reveal</button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</article>

<style>
  .batch { border-bottom: 1px solid var(--line); padding: var(--space-3) var(--space-4); }
  .batch.undone { color: var(--faint); }
  .head { display: flex; align-items: center; gap: var(--space-3); }
  .time, .sizes, .names { font-family: var(--font-data); font-size: var(--text-meta); }
  .what { font-weight: 600; }
  .tag { font-family: var(--font-data); font-size: var(--text-label); letter-spacing: 0.08em; padding: 1px 6px; border: 1px solid var(--line); border-radius: var(--radius-badge); color: var(--muted); }
  .preset { color: var(--muted); font-size: var(--text-meta); }
  .spacer { flex: 1; }
  .state { font-size: var(--text-meta); color: var(--faint); }
  .summary { margin: var(--space-1) 0 0 36px; font-family: var(--font-data); font-size: var(--text-meta); color: var(--muted); }
  .files { list-style: none; margin: var(--space-2) 0 0 36px; padding: 0; }
  li { display: flex; align-items: center; gap: var(--space-3); padding: 3px 0; }
  .names { flex: 1; color: var(--text); }
  .why { font-size: var(--text-meta); color: var(--error-text); }
  .disclose { width: 24px; border: 0; background: transparent; color: var(--muted); font-size: 16px; padding: 0; }
  .chev { display: inline-block; transition: transform 0.15s; }
  .chev.open { transform: rotate(90deg); }
  @media (prefers-reduced-motion: reduce) { .chev { transition: none; } }
  button.text { font: inherit; font-size: var(--text-meta); color: var(--text); background: transparent; border: 1px solid var(--control-border); border-radius: var(--radius-control); padding: 3px 10px; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
