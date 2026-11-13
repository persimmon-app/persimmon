<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- The sidebar's modes (canvas "macOS — Main window"): how new files become rows. -->
<script lang="ts">
  import Icon from "./Icon.svelte";
  import PresetList from "./PresetList.svelte";
  import { nextRadio } from "../lib/radio";
  import type { PresetView, QueueMode } from "../lib/types";

  let { mode, onMode, preset = null, presets = [], onPreset = () => {}, view = "queue", onHistory = () => {} }: {
    mode: QueueMode; onMode: (m: QueueMode) => void;
    preset?: string | null; presets?: PresetView[]; onPreset?: (id: string) => void;
    view?: "queue" | "history"; onHistory?: () => void;
  } = $props();
  const modes: { id: QueueMode; label: string; key: string; icon: "arrow" | "compress" | "folder" | "add" }[] = [
    { id: "convert", label: "Convert", key: "⌃⌘1", icon: "arrow" },
    { id: "compress", label: "Compress", key: "⌃⌘2", icon: "compress" },
    { id: "extract", label: "Extract", key: "⌃⌘3", icon: "folder" },
    { id: "combine", label: "Combine", key: "⌃⌘4", icon: "add" },
  ];
  /** One tab stop for the group: the checked mode, or the current one
   * while a preset is active. The arrow keys move and select (macOS). */
  const buttons: HTMLButtonElement[] = $state([]);
  function key(e: KeyboardEvent, from: QueueMode) {
    const to = nextRadio(modes.map((m) => m.id), from, e.key) as QueueMode | null;
    if (!to) return;
    e.preventDefault(); e.stopPropagation();
    onMode(to);
    buttons[modes.findIndex((m) => m.id === to)]?.focus();
  }
</script>

<nav class="sidebar" aria-label="Sidebar">
  <p class="label" id="mode-label">MODE</p>
  <div role="radiogroup" aria-labelledby="mode-label">
    {#each modes as m, i (m.id)}
      <button role="radio" aria-checked={mode === m.id && !preset} class:current={mode === m.id && !preset && view === "queue"}
        tabindex={mode === m.id ? 0 : -1} bind:this={buttons[i]} onkeydown={(e) => key(e, m.id)}
        title="{m.label} ({m.key})" onclick={() => onMode(m.id)}>
        <Icon name={m.icon} size={16} /> <span>{m.label}</span> <span class="key">{m.key}</span>
      </button>
    {/each}
  </div>
  {#if presets.length}<PresetList {presets} active={preset} {onPreset} />{/if}
  <p class="label">AUTOMATION</p>
  <button aria-pressed={view === "history"} class:current={view === "history"} onclick={onHistory}>
    <Icon name="queued" size={16} /> <span>History</span>
  </button>
</nav>

<style>
  .sidebar { width: var(--sidebar); flex-shrink: 0; overflow-y: auto; padding: 0 var(--space-3) var(--space-4); border-right: 1px solid var(--line); background: var(--surface); }
  .label { margin: var(--space-4) var(--space-2) var(--space-2); font-size: var(--text-label); font-weight: 500; letter-spacing: 0.14em; color: var(--faint); }
  button {
    width: 100%; display: flex; align-items: center; gap: var(--space-2);
    font: inherit; font-size: var(--text-body); color: var(--muted); text-align: left;
    background: transparent; border: 0; border-radius: var(--radius-control); padding: 6px var(--space-2);
  }
  button.current { background: var(--accent-tint); color: var(--accent-text); }
  .key { margin-left: auto; font-family: var(--font-data); font-size: var(--text-label); color: var(--faint); }
  button.current .key { color: inherit; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
