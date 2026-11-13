<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- The sidebar's presets (spec D4): ⌘1–⌘9, active for new files. -->
<script lang="ts">
  import type { PresetView } from "../lib/types";
  import { nextRadio } from "../lib/radio";
  let { presets, active, onPreset }: { presets: PresetView[]; active: string | null; onPreset: (id: string) => void } = $props();

  /** One tab stop: the active preset, else the first. Arrows move and
   * select, as the modes above do. */
  const stop = $derived(presets.some((p) => p.id === active) ? active : (presets[0]?.id ?? null));
  const buttons: HTMLButtonElement[] = $state([]);
  function key(e: KeyboardEvent, from: string) {
    const to = nextRadio(presets.map((p) => p.id), from, e.key);
    if (!to) return;
    e.preventDefault(); e.stopPropagation();
    onPreset(to);
    buttons[presets.findIndex((p) => p.id === to)]?.focus();
  }
</script>

<p class="label" id="presets-label">PRESETS</p>
<div role="radiogroup" aria-labelledby="presets-label">
  {#each presets as p, i (p.id)}
    <button role="radio" aria-checked={active === p.id} class:current={active === p.id}
      tabindex={stop === p.id ? 0 : -1} bind:this={buttons[i]} onkeydown={(e) => key(e, p.id)}
      title="{p.name} ({p.command})" onclick={() => onPreset(p.id)}>
      <span class="name">{p.name}</span>
      {#if p.shortcut}<span class="key">⌘{p.shortcut}</span>{/if}
    </button>
  {/each}
</div>

<style>
  .label { margin: var(--space-4) var(--space-2) var(--space-2); font-size: var(--text-label); font-weight: 500; letter-spacing: 0.14em; color: var(--faint); }
  button {
    width: 100%; display: flex; align-items: center; gap: var(--space-2);
    font: inherit; font-size: var(--text-body); color: var(--muted); text-align: left;
    background: transparent; border: 0; border-radius: var(--radius-control); padding: 6px var(--space-2);
  }
  button.current { background: var(--accent-tint); color: var(--accent-text); }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .key { margin-left: auto; font-family: var(--font-data); font-size: var(--text-label); color: var(--faint); }
  button.current .key { color: inherit; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
