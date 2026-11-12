<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<script lang="ts">
  import type { RowView } from "../lib/types";

  let { rows, message }: { rows: RowView[]; message: string | null } = $props();
  const count = (s: string) => rows.filter((r) => r.state === s).length;
  const done = $derived(count("done") + count("notSmaller"));
  const running = $derived(count("running"));
  const queued = $derived(count("queued"));
  const failed = $derived(count("failed"));
</script>

<div class="status" role="status">
  {#if rows.length === 0}
    <span>Queue empty</span>
  {:else}
    {#if done}<span>{done} done</span>{/if}
    {#if running}<span class="running">{running} running</span>{/if}
    {#if queued}<span>{queued} queued</span>{/if}
    {#if failed}<span class="failed">{failed} failed</span>{/if}
  {/if}
  {#if message}<span class="message">{message}</span>{/if}
  <span class="spacer"></span>
  <span>Output → beside original</span>
</div>

<style>
  .status {
    height: var(--statusbar);
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0 var(--space-4);
    border-top: 1px solid var(--line);
    background: var(--surface);
    font-size: var(--text-meta);
    color: var(--muted);
  }
  .running { color: var(--accent-text); }
  .failed { color: var(--error-text); }
  .message { color: var(--text); }
  .spacer { flex: 1; }
</style>
