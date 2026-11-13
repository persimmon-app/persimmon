<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<script lang="ts">
  let { version, onLicences, onSources }: { version: string; onLicences: () => Promise<boolean>; onSources: () => void } = $props();
  let missing = $state(false);
</script>

<section aria-labelledby="about-title">
  <h2 id="about-title">About & licences</h2>
  <p>Persimmon <span class="data">{version}</span></p>
  <p>GPL-3.0-or-later</p>
  <button onclick={async () => { missing = !(await onLicences()); }}>Licences…</button>
  {#if missing}<p class="hint">The full licence list comes with the released app.</p>{/if}
  <button onclick={onSources}>Source code…</button>
  <p class="hint">The sources of FFmpeg, libvips and the libraries they use, with FFmpeg's configure line, are attached to this version's release.</p>
</section>

<style>
  h2 { margin: 0 0 var(--space-4); font-size: var(--text-section); font-weight: 600; }
  .data { font-family: var(--font-data); }
  .hint { color: var(--muted); font-size: var(--text-meta); }
  button { font: inherit; color: var(--text); background: transparent; border: 1px solid var(--control-border); border-radius: var(--radius-control); padding: 4px 12px; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
