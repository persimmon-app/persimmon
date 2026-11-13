<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- History (canvas "History"; spec D3): 30 days of batches, search, filters, undo. -->
<script lang="ts">
  import HistoryBatch from "./HistoryBatch.svelte";
  import { plural } from "../lib/format";
  import type { BatchView, HistoryFilter, UndoReport } from "../lib/types";

  let { batches, text, filter, report, now = Date.now() / 1000, onSearch, onFilter, onUndo, onReveal, onClear, onDismissReport }: {
    batches: BatchView[]; text: string; filter: HistoryFilter; report: UndoReport | null; now?: number;
    onSearch: (t: string) => void; onFilter: (f: HistoryFilter) => void; onUndo: (id: string) => void;
    onReveal: (path: string) => void; onClear: () => void; onDismissReport: () => void;
  } = $props();

  let confirming = $state(false);
  const files = $derived(batches.reduce((n, b) => n + b.items.length, 0));
  const day = (t: number) => { const d = new Date(t * 1000); return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime(); };
  const groups = $derived.by(() => {
    const today = day(now);
    const out: { label: string; batches: BatchView[] }[] = [];
    for (const b of batches) {
      const d = day(b.finished);
      const label = d === today ? "TODAY" : d === today - 86_400_000 ? "YESTERDAY"
        : new Date(d).toLocaleDateString("en-GB", { day: "numeric", month: "long" }).toUpperCase();
      const last = out.at(-1);
      if (last?.label === label) last.batches.push(b); else out.push({ label, batches: [b] });
    }
    return out;
  });
  const chips: { id: HistoryFilter; label: string }[] = [
    { id: "all", label: "All" }, { id: "undoable", label: "Undoable" }, { id: "failed", label: "Failed" },
  ];
</script>

<section class="history" aria-label="History">
  <div class="bar">
    <input type="search" placeholder="Search history" aria-label="Search history" value={text}
      oninput={(e) => onSearch(e.currentTarget.value)} />
    {#each chips as c (c.id)}
      <button class="chip" aria-pressed={filter === c.id} class:on={filter === c.id} onclick={() => onFilter(c.id)}>{c.label}</button>
    {/each}
    <span class="spacer"></span>
    <span class="count">{plural(files, "file")} · 30 days</span>
  </div>
  {#if report && report.skipped.length}
    <div class="report" role="status">
      <ul>{#each report.skipped as s (s)}<li>{s}</li>{/each}</ul>
      <button class="text" onclick={onDismissReport}>OK</button>
    </div>
  {/if}
  <div class="list">
    {#each groups as g (g.label)}
      <h3>{g.label}</h3>
      {#each g.batches as b (b.id)}<HistoryBatch batch={b} {onUndo} {onReveal} />{/each}
    {:else}
      <p class="empty">No batches{text || filter !== "all" ? " match" : " yet"}.</p>
    {/each}
  </div>
  <footer>
    <span>Undo moves the files a batch made to the Trash</span>
    <span class="spacer"></span>
    {#if confirming}
      <span>Clear all history? Files stay where they are.</span>
      <button class="text" onclick={() => { confirming = false; onClear(); }}>Clear</button>
      <button class="text" onclick={() => (confirming = false)}>Cancel</button>
    {:else}
      <button class="text" onclick={() => (confirming = true)}>Clear history…</button>
    {/if}
  </footer>
</section>

<style>
  .history { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  .bar { display: flex; align-items: center; gap: var(--space-2); padding: var(--space-3) var(--space-4); border-bottom: 1px solid var(--line); background: var(--surface); }
  input { font: inherit; color: var(--text); background: var(--ground); border: 1px solid var(--control-border); border-radius: var(--radius-control); height: var(--button-toolbar); padding: 0 var(--space-2); width: 220px; }
  .chip { font: inherit; font-size: var(--text-meta); color: var(--muted); background: transparent; border: 1px solid var(--line); border-radius: var(--radius-chip); padding: 3px 12px; }
  .chip.on { background: var(--accent-tint); color: var(--accent-text); border-color: transparent; }
  .count { font-family: var(--font-data); font-size: var(--text-meta); color: var(--muted); }
  .spacer { flex: 1; }
  .list { flex: 1; overflow-y: auto; }
  h3 { margin: 0; padding: var(--space-3) var(--space-4) var(--space-1); font-size: var(--text-label); font-weight: 500; letter-spacing: 0.14em; color: var(--faint); }
  .empty { margin: var(--space-6); color: var(--muted); }
  .report { display: flex; align-items: flex-start; gap: var(--space-3); margin: var(--space-3) var(--space-4) 0; padding: var(--space-3); border: 1px solid var(--line); border-radius: var(--radius-card); background: var(--surface); font-size: var(--text-meta); }
  .report ul { margin: 0; padding-left: var(--space-4); flex: 1; }
  footer { display: flex; align-items: center; gap: var(--space-2); padding: var(--space-2) var(--space-4); border-top: 1px solid var(--line); background: var(--surface); font-size: var(--text-meta); color: var(--muted); }
  button.text { font: inherit; font-size: var(--text-meta); color: var(--text); background: transparent; border: 1px solid var(--control-border); border-radius: var(--radius-control); padding: 3px 10px; }
  button:focus-visible, input:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
