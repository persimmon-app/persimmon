<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Making or editing a watch rule (canvas "Watch folders"; spec D2), with its command line. -->
<script lang="ts">
  import { onMount } from "svelte";
  import { badge } from "../lib/format";
  import type { FormatId, PresetView, Rule, RuleDraftView, Shown } from "../lib/types";

  let { draft, isNew, targets, presets, onCommand, onDefaults, onChoose, onSave, onCancel }: {
    draft: RuleDraftView; isNew: boolean; targets: FormatId[]; presets: PresetView[];
    /** The `prsm watch` line for the rule as it stands. */
    onCommand: (rule: Rule) => Promise<string>;
    /** Match files' defaults for a target. */
    onDefaults: (target: FormatId) => Promise<string>;
    /** The open panel, for the watched, results or done folder. */
    onChoose: (which: "folder" | "results" | "done") => Promise<string | null>;
    /** Null when saved; otherwise why not. */
    onSave: (rule: Rule, existing: boolean) => Promise<Shown | null>;
    onCancel: () => void;
  } = $props();

  // Edited in place; `draft` is only where it starts.
  // svelte-ignore state_referenced_locally
  let rule: Rule = $state({ ...draft.rule });
  // svelte-ignore state_referenced_locally
  let command = $state(draft.command);
  let touched = $state(false);
  let existing = $state(false);
  let error: Shown | null = $state(null);
  let saving = $state(false);
  let dialog: HTMLDialogElement | undefined = $state();

  const fitting = $derived(presets.filter((p) => p.mode === "convert" && (p.target === null || p.target === rule.target)));
  // svelte-ignore state_referenced_locally
  const count = draft.existing;

  onMount(() => dialog?.showModal());

  $effect(() => {
    const snapshot = $state.snapshot(rule);
    onCommand(snapshot).then((c) => (command = c), () => {});
  });

  async function retarget(to: FormatId) {
    rule.target = to;
    if (rule.preset && !fitting.some((p) => p.id === rule.preset)) rule.preset = null;
    if (!touched) rule.patterns = await onDefaults(to);
  }

  async function choose(which: "folder" | "results" | "done") {
    const path = await onChoose(which);
    if (!path) return;
    if (which === "folder") {
      // Results and done folders that followed the old folder follow the new one.
      if (rule.results === `${rule.folder}/converted`) rule.results = `${path}/converted`;
      if (rule.done === `${rule.folder}/done`) rule.done = `${path}/done`;
      rule.folder = path;
    } else if (which === "results") rule.results = path;
    else rule.done = path;
  }

  async function save() {
    if (saving) return;
    saving = true;
    try { error = await onSave($state.snapshot(rule), existing); } finally { saving = false; }
  }
</script>

<dialog bind:this={dialog} aria-labelledby="rule-title" oncancel={(e) => { e.preventDefault(); onCancel(); }}>
  <h2 id="rule-title">{isNew ? "New watch rule" : "Edit watch rule"}</h2>
  <div class="grid">
    <span class="label" id="folder-label">Folder</span>
    <div class="path"><code aria-labelledby="folder-label">{rule.folder}</code>
      <button class="text" aria-label="Choose the watched folder" onclick={() => choose("folder")}>Choose…</button></div>

    <label for="patterns">Match files</label>
    <input id="patterns" value={rule.patterns} oninput={(e) => { touched = true; rule.patterns = e.currentTarget.value; }} />

    <label for="target">Convert to</label>
    <select id="target" value={rule.target} onchange={(e) => retarget(e.currentTarget.value)}>
      {#each targets as t (t)}<option value={t}>{badge(t)}</option>{/each}
    </select>

    <label for="preset">Preset</label>
    <select id="preset" value={rule.preset ?? ""} onchange={(e) => (rule.preset = e.currentTarget.value || null)}>
      <option value="">None</option>
      {#each fitting as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
    </select>

    <span class="label" id="results-label">Put results in</span>
    <div class="path"><code aria-labelledby="results-label">{rule.results}</code>
      <button class="text" aria-label="Choose the results folder" onclick={() => choose("results")}>Choose…</button></div>

    <span class="label" id="then-label">Then the original</span>
    <div role="radiogroup" aria-labelledby="then-label" class="then">
      <label><input type="radio" name="then" checked={rule.done === null} onchange={() => (rule.done = null)} /> Leave where it is</label>
      <label><input type="radio" name="then" checked={rule.done !== null} onchange={() => (rule.done ??= `${rule.folder}/done`)} /> Move to</label>
      {#if rule.done !== null}
        <div class="path"><code>{rule.done}</code>
          <button class="text" aria-label="Choose the folder for finished originals" onclick={() => choose("done")}>Choose…</button></div>
      {/if}
    </div>
  </div>
  <label class="check"><input type="checkbox" bind:checked={rule.recursive} /> Include subfolders</label>
  <label class="check"><input type="checkbox" bind:checked={rule.settle} /> Wait until the file stops growing</label>
  <label class="check"><input type="checkbox" bind:checked={rule.notifyEach} /> Notify on every file</label>
  {#if isNew && count > 0}
    <label class="check"><input type="checkbox" bind:checked={existing} /> Also convert the {count === 1 ? "1 file" : `${count} files`} already here</label>
  {/if}
  <code class="command">{command}</code>
  {#if error}
    <p class="error" role="alert">{error.reason}</p>
    {#if error.fix}<p class="fix">{error.fix}</p>{/if}
  {/if}
  <div class="buttons">
    <button onclick={onCancel}>Cancel</button>
    <button class="primary" disabled={saving} onclick={save}>Save</button>
  </div>
</dialog>

<style>
  dialog { background: var(--surface); color: var(--text); border: 1px solid var(--line); border-radius: var(--radius-card); padding: var(--space-5); width: min(560px, 90vw); }
  dialog::backdrop { background: var(--backdrop); }
  h2 { margin: 0 0 var(--space-4); font-size: var(--text-section); font-weight: 600; }
  .grid { display: grid; grid-template-columns: 140px 1fr; gap: var(--space-2) var(--space-3); align-items: center; margin-bottom: var(--space-3); }
  .label, .grid > label { color: var(--muted); }
  .path { display: flex; align-items: center; gap: var(--space-2); min-width: 0; }
  .path code { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  code { font-family: var(--font-data); font-size: var(--text-meta); }
  .then { display: flex; flex-direction: column; gap: var(--space-1); }
  .then label, .check { display: flex; align-items: center; gap: var(--space-2); }
  .check { margin-bottom: var(--space-2); }
  .command { display: block; margin: var(--space-3) 0; padding: var(--space-2); border: 1px solid var(--line); border-radius: var(--radius-control); color: var(--muted); overflow-x: auto; white-space: nowrap; }
  .error { margin: 0; color: var(--error-text); }
  .fix { margin: var(--space-1) 0 0; font-size: var(--text-meta); color: var(--muted); }
  input:not([type]), select { font: inherit; color: var(--text); background: var(--surface); border: 1px solid var(--control-border); border-radius: var(--radius-control); padding: 3px 6px; }
  .buttons { display: flex; justify-content: flex-end; gap: var(--space-2); margin-top: var(--space-4); }
  button { font: inherit; color: var(--text); background: transparent; border: 1px solid var(--control-border); border-radius: var(--radius-control); height: var(--button-toolbar); padding: 0 var(--space-3); }
  button.text { height: auto; padding: 3px 10px; font-size: var(--text-meta); }
  .primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); font-weight: 600; }
  button:focus-visible, input:focus-visible, select:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
