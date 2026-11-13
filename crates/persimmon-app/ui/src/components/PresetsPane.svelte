<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Settings → Presets (spec D4): the list and the editor, with the prsm line. -->
<script lang="ts">
  import { tick } from "svelte";
  import { commandLine } from "../lib/command";
  import type { FormatId, JobOptions, PresetView, QueueMode } from "../lib/types";

  type Draft = { id: string; name: string; builtin: boolean; mode: QueueMode; target: FormatId | null; options: JobOptions };
  let { presets, onSave, onDelete }: {
    presets: PresetView[];
    onSave: (p: Draft) => Promise<string | null>;
    onDelete: (id: string) => Promise<string | null>;
  } = $props();

  const blank = (): Draft => ({ id: "", name: "", builtin: false, mode: "convert", target: "jpeg",
    options: { quality: null, preset: null, pages: null, maxSize: null, stripMetadata: false } });
  let draft: Draft | null = $state(null);
  let error: string | null = $state(null);
  let copied = $state(false);

  const targetsFor: Record<QueueMode, FormatId[]> = {
    convert: ["jpeg", "png", "webp", "avif", "tiff", "pdf", "mp4", "webm", "mov", "gif", "mp3", "m4a", "wav", "flac", "opus"],
    compress: [], extract: [], combine: ["pdf", "zip", "tar.gz"],
  };
  const builtins = $derived(presets.filter((p) => p.builtin));
  const mine = $derived(presets.filter((p) => !p.builtin));
  const line = $derived(draft ? commandLine(draft) : "");

  function edit(p: PresetView) {
    // A Combine or Extract preset saved before options were refused for
    // them would never save again, with the controls hidden: drop them.
    const options = p.mode === "combine" || p.mode === "extract" ? blank().options : { ...p.options };
    draft = { id: p.id, name: p.name, builtin: p.builtin, mode: p.mode, target: p.target, options };
    error = null;
  }
  function setMode(m: QueueMode) {
    if (!draft) return;
    draft.mode = m;
    draft.target = targetsFor[m][0] ?? null;
    // Combine and Extract take none of the options (presets.rs check()).
    if (m === "combine" || m === "extract") draft.options = blank().options;
  }
  /** After a save, the editor shows what was saved: a built-in saves as a
   * new copy, so the copy (the one preset that wasn't there before) is
   * selected rather than the built-in staying open. */
  async function save() {
    if (!draft) return;
    const before = new Set(presets.map((p) => p.id));
    error = await onSave($state.snapshot(draft));
    if (error) return;
    await tick();
    const fresh = presets.find((p) => !before.has(p.id));
    if (fresh) edit(fresh);
  }
  async function remove() {
    if (draft && !draft.builtin && draft.id) { error = await onDelete(draft.id); if (!error) draft = null; }
  }
</script>

<section class="presets" aria-labelledby="presets-title">
  <h2 id="presets-title">Presets</h2>
  <div class="layout">
    <div class="list">
      <p class="label">BUILT IN</p>
      {#each builtins as p (p.id)}<button class:current={draft?.id === p.id} onclick={() => edit(p)}>{p.name}</button>{/each}
      <p class="label">MINE</p>
      {#each mine as p (p.id)}<button class:current={draft?.id === p.id} onclick={() => edit(p)}>{p.name}</button>{/each}
      <button class="new" onclick={() => { draft = blank(); error = null; }}>New preset</button>
    </div>
    {#if draft}
      <form class="editor" onsubmit={(e) => { e.preventDefault(); save(); }}>
        <label>Name <input bind:value={draft.name} /></label>
        <label>Mode
          <select value={draft.mode} onchange={(e) => setMode(e.currentTarget.value as QueueMode)}>
            <option value="convert">Convert</option><option value="compress">Compress</option>
            <option value="extract">Extract</option><option value="combine">Combine</option>
          </select>
        </label>
        {#if targetsFor[draft.mode].length}
          <label>Target
            <select bind:value={draft.target}>{#each targetsFor[draft.mode] as t (t)}<option value={t}>{t.toUpperCase()}</option>{/each}</select>
          </label>
        {/if}
        {#if draft.mode === "convert" || draft.mode === "compress"}
        <label>Quality
          <select bind:value={draft.options.quality}>
            <option value={null}>Default</option><option value="low">Low</option><option value="medium">Medium</option>
            <option value="high">High</option><option value="max">Max</option>
          </select>
        </label>
        <label>Video preset
          <select bind:value={draft.options.preset}>
            <option value={null}>Keep the codec</option><option value="compatible">Compatible</option><option value="small">Small</option>
          </select>
        </label>
        <label>Max size (px)
          <input type="number" min="1" value={draft.options.maxSize ?? ""}
            oninput={(e) => { if (draft) { const v = e.currentTarget.valueAsNumber; draft.options.maxSize = Number.isFinite(v) && v > 0 ? v : null; } }} />
        </label>
        <label class="check"><input type="checkbox" bind:checked={draft.options.stripMetadata} /> Strip metadata</label>
        {/if}
        <div class="command">
          <code>{line}</code>
          <button type="button" class="text" onclick={() => { navigator.clipboard?.writeText(line); copied = true; setTimeout(() => (copied = false), 1500); }}>{copied ? "Copied" : "Copy"}</button>
        </div>
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        <div class="actions">
          {#if !draft.builtin && draft.id}<button type="button" class="text" onclick={remove}>Delete</button>{/if}
          <span class="spacer"></span>
          <button type="submit" class="primary">{draft.builtin ? "Save as copy" : "Save"}</button>
        </div>
      </form>
    {:else}
      <p class="hint">Pick a preset to edit it, or make a new one. Built-in presets are saved as a copy.</p>
    {/if}
  </div>
</section>

<style>
  h2 { margin: 0 0 var(--space-4); font-size: var(--text-section); font-weight: 600; }
  .layout { display: grid; grid-template-columns: 180px 1fr; gap: var(--space-5); }
  .label { margin: var(--space-3) 0 var(--space-1); font-size: var(--text-label); font-weight: 500; letter-spacing: 0.14em; color: var(--faint); }
  .list button { display: block; width: 100%; text-align: left; font: inherit; color: var(--muted); background: transparent; border: 0; border-radius: var(--radius-control); padding: 5px var(--space-2); }
  .list button.current { background: var(--accent-tint); color: var(--accent-text); }
  .list .new { margin-top: var(--space-3); color: var(--text); border: 1px solid var(--control-border); text-align: center; }
  .editor { display: flex; flex-direction: column; gap: var(--space-2); }
  label { display: grid; grid-template-columns: 120px 1fr; align-items: center; gap: var(--space-2); color: var(--muted); }
  label.check { display: flex; }
  input, select { font: inherit; color: var(--text); background: var(--ground); border: 1px solid var(--control-border); border-radius: var(--radius-control); padding: 3px var(--space-2); }
  .command { display: flex; align-items: center; gap: var(--space-2); margin-top: var(--space-2); padding: var(--space-2) var(--space-3); background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-control); }
  code { flex: 1; font-family: var(--font-data); font-size: var(--text-meta); color: var(--text); overflow-wrap: anywhere; }
  .error { margin: 0; color: var(--error-text); font-size: var(--text-meta); }
  .actions { display: flex; gap: var(--space-2); margin-top: var(--space-2); }
  .spacer { flex: 1; }
  .hint { color: var(--muted); }
  button.text, button.primary { font: inherit; font-size: var(--text-meta); color: var(--text); background: transparent; border: 1px solid var(--control-border); border-radius: var(--radius-control); padding: 4px 12px; }
  button.primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); font-weight: 600; }
  button:focus-visible, input:focus-visible, select:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
