<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- One queue row, as the canvas's macOS window draws it (DD-13 … DD-17). -->
<script lang="ts">
  import Icon from "./Icon.svelte";
  import { badge, size, target } from "../lib/format";
  import type { RowId, RowView } from "../lib/types";

  let {
    row, selected, onCancel, onRetry, onReveal, onSelect,
  }: {
    row: RowView; selected: boolean;
    onCancel: (id: RowId) => void; onRetry: (id: RowId) => void;
    onReveal: (id: RowId) => void; onSelect: (id: RowId, e: MouseEvent) => void;
  } = $props();

  const percent = $derived(row.state === "running" && row.fraction !== null ? Math.round(row.fraction * 100) : null);
  const outputName = $derived(row.output ? row.output.split("/").pop() : "");
  /** What VoiceOver reads for the row: its state and why (DD-17). */
  const label = $derived.by(() => {
    switch (row.state) {
      case "running": return `${row.name}, running, ${percent === null ? "working" : `${percent}%`}`;
      case "failed": return `${row.name}, failed: ${row.error.reason}.${row.error.fix ? ` ${row.error.fix}` : ""}`;
      case "notSmaller": return `${row.name}, already small`;
      default: return `${row.name}, ${row.state}`;
    }
  });
  const sizes = $derived(
    row.state === "done" && row.outputSize !== null ? `${size(row.size)} → ${size(row.outputSize)}` : size(row.size),
  );
</script>

<div
  class="row {row.state}"
  class:running={row.state === "running"}
  class:selected
  role="option"
  aria-selected={selected}
  aria-label={label}
  tabindex="-1"
  onclick={(e) => onSelect(row.id, e)}
  onkeydown={() => {}}
>
  <span class="badge">{badge(row.from)}</span>
  <span class="arrow"><Icon name="arrow" size={14} /></span>
  <span class="badge to">{target(row)}</span>
  <div class="file">
    <span class="name">{row.name}</span>
    {#if row.state === "running"}
      {#if percent !== null}
        <div class="bar" role="progressbar" aria-label="Progress of {row.name}" aria-valuemin="0" aria-valuemax="100" aria-valuenow={percent}>
          <div class="fill" style:width="{percent}%"></div>
        </div>
      {:else}
        <div class="bar indeterminate" role="progressbar" aria-label="Progress of {row.name}"><div class="fill"></div></div>
      {/if}
    {/if}
    {#if row.state === "failed"}
      <span class="why">{row.error.reason}{#if row.error.fix}{" — "}{row.error.fix}{/if}</span>
    {/if}
  </div>
  <span class="sizes">{sizes}</span>
  <span class="status">
    {#if row.state === "queued"}<Icon name="queued" /> Queued
    {:else if row.state === "running"}{#if percent !== null}<span class="pct">{percent}%</span>{:else}<span class="pct">Working</span>{/if}
    {:else if row.state === "done"}<Icon name="done" /> Done
    {:else if row.state === "notSmaller"}<Icon name="done" /> Already small
    {:else if row.state === "failed"}<Icon name="error" /> Failed
    {:else}<Icon name="stop" /> Cancelled{/if}
  </span>
  <span class="action">
    {#if row.state === "running"}
      <button class="icon" aria-label="Cancel {row.name}" title="Cancel (⌘.)" onclick={(e) => { e.stopPropagation(); onCancel(row.id); }}><Icon name="close" size={14} /></button>
    {:else if row.state === "done"}
      <button class="text" aria-label="Reveal {outputName} in Finder" title="Reveal in Finder (⇧⌘R)" onclick={(e) => { e.stopPropagation(); onReveal(row.id); }}>Reveal</button>
    {:else if row.state === "failed" || row.state === "cancelled"}
      <button class="text" aria-label="Retry {row.name}" title="Retry (⌘R)" onclick={(e) => { e.stopPropagation(); onRetry(row.id); }}>Retry</button>
    {/if}
  </span>
</div>


<style>
  .row {
    display: grid;
    grid-template-columns: auto auto auto 1fr auto 104px 72px;
    align-items: center;
    gap: var(--space-3);
    padding: 13px 20px;
    border-bottom: 1px solid var(--line);
    color: var(--text);
    border-left: 2px solid transparent;
    cursor: default;
  }
  .row.selected { background: var(--surface); }
  .row.queued { color: var(--muted); }
  .row.running { background: var(--raised); }
  .row.done, .row.notSmaller, .row.cancelled { color: var(--faint); }
  .row.failed {
    border-left-color: var(--error);
    background: color-mix(in srgb, var(--error) 7%, transparent);
  }
  .badge {
    font-family: var(--font-data);
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.04em;
    padding: 2px 6px;
    border-radius: var(--radius-badge);
    border: 1px solid var(--line);
  }
  .running .badge.to { background: var(--accent-tint); color: var(--accent-text); border-color: transparent; }
  .arrow { display: flex; color: var(--faint); }
  .file { min-width: 0; display: flex; flex-direction: column; gap: 6px; }
  .name {
    font-family: var(--font-data);
    font-size: var(--text-row);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .why { font-size: var(--text-meta); color: var(--error-text); }
  .bar { height: 3px; border-radius: 2px; background: var(--line); overflow: hidden; }
  .fill { height: 100%; background: var(--accent); }
  .indeterminate .fill { width: 30%; animation: slide 1.2s ease-in-out infinite; }
  @keyframes slide { from { transform: translateX(-100%); } to { transform: translateX(340%); } }
  @media (prefers-reduced-motion: reduce) { .indeterminate .fill { animation: none; width: 100%; opacity: 0.4; } }
  .sizes { font-family: var(--font-data); font-size: var(--text-meta); white-space: nowrap; }
  .status { display: flex; align-items: center; gap: var(--space-1); font-size: var(--text-meta); white-space: nowrap; }
  .pct { font-family: var(--font-data); color: var(--accent-text); }
  .failed .status { color: var(--error-text); }
  .action { display: flex; justify-content: flex-end; }
  button {
    font: inherit;
    font-size: var(--text-meta);
    color: var(--text);
    background: transparent;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-control);
    padding: 3px 10px;
  }
  button.icon { padding: 3px; display: flex; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
