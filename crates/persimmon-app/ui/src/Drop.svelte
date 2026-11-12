<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- The drop window (DD-41): a small target that stays on top. -->
<script lang="ts">
  import { onMount } from "svelte";

  type DragEvent = { payload: { type: string; paths?: string[] } };
  let { listenDrag, onDrop, running }: {
    listenDrag: (h: (e: DragEvent) => void) => Promise<() => void>;
    onDrop: (paths: string[]) => void;
    running: number;
  } = $props();

  let hover = $state(false);
  onMount(() => {
    const off = listenDrag((e) => {
      const t = e.payload.type;
      if (t === "enter" || t === "over") hover = true;
      else if (t === "leave") hover = false;
      else if (t === "drop") { hover = false; if (e.payload.paths?.length) onDrop(e.payload.paths); }
    });
    return () => { off.then((f) => f()); };
  });
</script>

<div class="zone" class:hover role="region" aria-label="Drop files here to convert them">
  <span class="label">Drop here</span>
  {#if running > 0}<span class="running">{running} running</span>{/if}
</div>

<style>
  :global(html), :global(body) { height: 100%; background: transparent; }
  .zone {
    height: 100vh; box-sizing: border-box; display: flex; flex-direction: column;
    align-items: center; justify-content: center; gap: var(--space-1);
    background: var(--surface); border: 1.5px solid var(--line); border-radius: var(--radius-card);
    color: var(--text); font-family: var(--font-ui); -webkit-user-select: none; user-select: none; cursor: default;
  }
  .zone.hover { background: var(--accent-tint); border-color: var(--accent); }
  .label { font-weight: 600; }
  .running { font-family: var(--font-data); font-size: var(--text-label); color: var(--accent-text); }
</style>
