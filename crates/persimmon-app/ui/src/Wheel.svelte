<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- The format wheel (spec D4): six formats round a hub, at the pointer. -->
<script lang="ts">
  import { onMount } from "svelte";
  import { HUB, SIZE, badgeAt, hit, sector } from "./lib/wheel";
  import type { FormatId } from "./lib/types";

  type DragEvent = { payload: { type: string; paths?: string[]; position?: { x: number; y: number } } };
  let { formats, labels, hub, scale, listenDrag, onPick, onDrop }: {
    formats: FormatId[];
    labels: string[];
    hub: string;
    /** Device pixels per point: a drag's position comes in pixels. */
    scale: number;
    listenDrag: (h: (e: DragEvent) => void) => Promise<() => void>;
    /** A click: a segment's index, null for the hub, -1 to cancel. */
    onPick: (index: number | null) => void;
    /** A drop, with its point in the window's points. */
    onDrop: (paths: string[], x: number, y: number) => void;
  } = $props();

  let hovered: number | null = $state(null);
  function point(x: number, y: number) {
    const h = hit(x, y);
    hovered = typeof h === "number" && h < formats.length ? h : null;
  }
  function click(e: MouseEvent) {
    const h = hit(e.clientX, e.clientY);
    onPick(h === "hub" ? null : h === "outside" ? -1 : h);
  }

  onMount(() => {
    const off = listenDrag((e) => {
      const { type, paths, position } = e.payload;
      const p = position ? { x: position.x / scale, y: position.y / scale } : null;
      if ((type === "enter" || type === "over") && p) point(p.x, p.y);
      else if (type === "leave") hovered = null;
      else if (type === "drop") {
        hovered = null;
        if (paths?.length && p) onDrop(paths, p.x, p.y);
      }
    });
    return () => { off.then((f) => f()); };
  });
</script>

<!-- Losing the keyboard (a click in another app) cancels a wheel that
     waits for a click; a drag's wheel never has the keyboard. -->
<svelte:window onmousemove={(e) => point(e.clientX, e.clientY)} onclick={click}
  onkeydown={(e) => { if (e.key === "Escape") onPick(-1); }} onblur={() => onPick(-1)} />

<div class="wheel">
  <svg width={SIZE} height={SIZE} viewBox="0 0 {SIZE} {SIZE}" role="img" aria-label={hub}>
    {#each formats as f, i (f)}
      <path class="segment" class:hovered={hovered === i} d={sector(i)} />
      <text class="badge" x={badgeAt(i).x} y={badgeAt(i).y}>{f.toUpperCase()}</text>
    {/each}
    <circle class="hub" cx={SIZE / 2} cy={SIZE / 2} r={HUB} />
    <text class="hub-text" x={SIZE / 2} y={SIZE / 2 + (hovered === null ? 0 : -8)}>{hub}</text>
    {#if hovered !== null}<text class="label" x={SIZE / 2} y={SIZE / 2 + 12}>{labels[hovered]}</text>{/if}
  </svg>
  <p class="hint">⇧ Formats · esc Cancel</p>
</div>

<style>
  :global(html), :global(body) { margin: 0; background: transparent; overflow: hidden; }
  .wheel {
    display: flex; flex-direction: column; align-items: center;
    font-family: var(--font-ui); -webkit-user-select: none; user-select: none; cursor: default;
  }
  .segment { fill: var(--surface); stroke: var(--line); stroke-width: 1.5; }
  .segment.hovered { fill: var(--accent-tint); stroke: var(--accent); }
  .hub { fill: var(--surface); stroke: var(--line); stroke-width: 1.5; }
  text { text-anchor: middle; dominant-baseline: middle; pointer-events: none; }
  .badge { fill: var(--text); font-family: var(--font-data); font-size: var(--text-meta); font-weight: 600; }
  .hub-text { fill: var(--text); font-family: var(--font-data); font-size: var(--text-label); font-weight: 600; }
  .label { fill: var(--accent-text); font-size: var(--text-label); }
  .hint {
    margin: var(--space-2) 0 0; padding: var(--space-1) var(--space-3);
    background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-chip);
    color: var(--muted); font-size: var(--text-label);
  }
</style>
