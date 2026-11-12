<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Settings → Output (spec D3): where converted files go, and the fallback. -->
<script lang="ts">
  import type { SettingsView } from "../lib/types";

  let { settings, onBeside, onChooseFolder, onChooseFallback }: {
    settings: SettingsView; onBeside: () => void; onChooseFolder: () => void; onChooseFallback: () => void;
  } = $props();
  const folder = $derived(settings.output.mode === "folder");
</script>

<section aria-labelledby="output-title">
  <h2 id="output-title">Output</h2>
  <fieldset>
    <legend>Save converted files</legend>
    <!-- A radio only shows what the settings say: a click asks for the
         change (a folder panel the user may cancel), and the ticked radio
         follows the settings that come back. -->
    <label><input type="radio" name="output" checked={!folder}
      onclick={(e) => { e.preventDefault(); if (folder) onBeside(); }} /> Beside the original</label>
    <label class="folder">
      <input type="radio" name="output" checked={folder}
        onclick={(e) => { e.preventDefault(); onChooseFolder(); }} /> In a folder:
      {#if folder}<span class="path">{settings.outputLabel}</span>{/if}
      <button aria-label="Choose output folder" onclick={onChooseFolder}>Choose…</button>
    </label>
  </fieldset>
  <fieldset>
    <legend>When the original's folder can't be written, save to</legend>
    <div class="fallback">
      <span class="path">{settings.fallbackLabel}</span>
      <button aria-label="Change fallback folder" onclick={onChooseFallback}>Change…</button>
    </div>
    <p class="hint">Files from the Photos library, Mail or a read-only disk can't be saved beside the original. Persimmon saves them here and says so on the row.</p>
  </fieldset>
</section>

<style>
  h2 { margin: 0 0 var(--space-4); font-size: var(--text-section); font-weight: 600; }
  fieldset { border: 0; padding: 0; margin: 0 0 var(--space-5); }
  legend { font-size: var(--text-label); font-weight: 500; letter-spacing: 0.14em; text-transform: uppercase; color: var(--faint); margin-bottom: var(--space-2); }
  label { display: flex; align-items: center; gap: var(--space-2); margin-bottom: var(--space-2); }
  .fallback { display: flex; align-items: center; gap: var(--space-2); }
  .path { font-family: var(--font-data); font-size: var(--text-meta); color: var(--text); }
  .hint { color: var(--muted); font-size: var(--text-meta); margin: var(--space-2) 0 0; }
  button {
    font: inherit; font-size: var(--text-meta); color: var(--text); background: transparent;
    border: 1px solid var(--control-border); border-radius: var(--radius-control); padding: 3px 10px;
  }
  button:focus-visible, input:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
