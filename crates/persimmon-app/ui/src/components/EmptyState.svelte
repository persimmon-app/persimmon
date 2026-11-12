<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- The empty queue (canvas "First run"): no wizard, just ways in. -->
<script lang="ts">
  import Icon from "./Icon.svelte";
  let { onAdd }: { onAdd: () => void } = $props();
</script>

<div class="empty">
  <div class="zone">
    <Icon name="add" size={24} />
    <p class="title">Drop files here, or folders to combine</p>
    <p class="sub">Images, video, audio, PDF and archives</p>
    <button onclick={onAdd}>Choose files… <span class="key">⌘O</span></button>
  </div>
  <p class="label">WAYS IN — ALL THE SAME ENGINE</p>
  <div class="cards">
    <div class="card">
      <p class="card-title">Drop window <span class="key">⌥⌘D</span></p>
      <p>A small target that stays on top.</p>
    </div>
    <div class="card">
      <p class="card-title">Terminal</p>
      <p class="cmd">prsm convert a.mov --to mp4</p>
      <p>Install prsm: see the README.</p>
    </div>
  </div>
</div>

<style>
  .empty { padding: var(--space-6); max-width: 640px; margin: 0 auto; }
  .zone {
    display: flex; flex-direction: column; align-items: center; gap: var(--space-2);
    padding: var(--space-6) var(--space-5); border: 1.5px dashed var(--control-border);
    border-radius: var(--radius-card); color: var(--muted); text-align: center;
  }
  .title { margin: 0; font-size: var(--text-section); font-weight: 600; color: var(--text); }
  .sub { margin: 0 0 var(--space-3); }
  button {
    font: inherit; color: var(--text); background: transparent;
    border: 1px solid var(--control-border); border-radius: var(--radius-control);
    height: var(--button); padding: 0 var(--space-4);
  }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .key { font-family: var(--font-data); font-size: var(--text-label); color: var(--faint); margin-left: var(--space-1); }
  .label { margin: var(--space-6) 0 var(--space-3); font-size: var(--text-label); font-weight: 500; letter-spacing: 0.14em; color: var(--faint); }
  .cards { display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-3); }
  .card { padding: var(--space-4); border: 1px solid var(--line); border-radius: var(--radius-card); background: var(--surface); color: var(--muted); }
  .card p { margin: 0 0 var(--space-1); }
  .card-title { color: var(--text); font-weight: 600; }
  .cmd { font-family: var(--font-data); font-size: var(--text-meta); color: var(--text); }
</style>
