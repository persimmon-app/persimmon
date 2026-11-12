<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<script lang="ts">
  import type { RowView } from "../lib/types";

  let { rows, message, output, onOutput }: {
    rows: RowView[]; message: string | null; output: string; onOutput: () => void;
  } = $props();
  const count = (s: string) => rows.filter((r) => r.state === s).length;
  const parts = $derived(
    [
      { n: count("done") + count("notSmaller"), word: "done", cls: "" },
      { n: count("running"), word: "running", cls: "running" },
      { n: count("queued"), word: "queued", cls: "" },
      { n: count("failed"), word: "failed", cls: "failed" },
    ].filter((p) => p.n > 0),
  );
</script>

<div class="status" role="status">
  {#if rows.length === 0}
    <span>Queue empty</span>
  {:else}
    {#each parts as p, i (p.word)}{#if i > 0}<span class="sep">{" · "}</span>{/if}<span class={p.cls}>{p.n} {p.word}</span>{/each}
  {/if}
  {#if message}<span class="message">{message}</span>{/if}
  <span class="spacer"></span>
  {#if rows.length === 0}
    <span class="offline">
      <svg width="14" height="14" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M10 2.5l6 2.5v4.5c0 4-2.6 6.8-6 8-3.4-1.2-6-4-6-8V5z" /></svg>
      Offline — no network access
    </span>
  {/if}
  <button class="output" onclick={onOutput}>Output → {output}</button>
</div>

<style>
  .status {
    height: var(--statusbar); display: flex; align-items: center; gap: var(--space-3);
    padding: 0 var(--space-4); border-top: 1px solid var(--line); background: var(--surface);
    font-size: var(--text-meta); color: var(--muted);
  }
  .sep { margin: 0 calc(-1 * var(--space-2)); }
  .running { color: var(--accent-text); }
  .failed { color: var(--error-text); }
  .message { color: var(--text); }
  .spacer { flex: 1; }
  .offline { display: inline-flex; align-items: center; gap: var(--space-1); }
  .output { font: inherit; color: var(--muted); background: transparent; border: 0; padding: 0; cursor: default; }
  .output:hover { color: var(--text); }
  .output:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
