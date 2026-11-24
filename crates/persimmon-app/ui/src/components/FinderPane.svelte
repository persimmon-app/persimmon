<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Settings → Finder & menus (spec D3): where Finder's two Services are, and how to show or hide them. -->
<script lang="ts">
  let { onShortcuts }: { onShortcuts: () => void } = $props();
</script>

<section aria-labelledby="finder-title">
  <h2 id="finder-title">Finder & menus</h2>
  <p>In Finder, select files or folders, then right-click → Services → Convert with Persimmon or Open in Persimmon.</p>
  <dl>
    <dt>Convert with Persimmon</dt>
    <dd>Converts the files at once, with the mode or preset you use now.</dd>
    <dt>Open in Persimmon</dt>
    <dd>Adds the files to the queue and brings the window forward.</dd>
  </dl>
  <p>To show or hide them: System Settings → Keyboard → Keyboard Shortcuts → Services.</p>
  <button onclick={onShortcuts}>Open Keyboard Shortcuts…</button>
</section>

<style>
  h2 { margin: 0 0 var(--space-4); font-size: var(--text-section); font-weight: 600; }
  p, dd { max-width: 52ch; color: var(--text); }
  dl { margin: var(--space-3) 0; }
  dt { font-weight: 600; margin-top: var(--space-2); }
  dd { margin: var(--space-1) 0 0; color: var(--muted); }
  button { font: inherit; color: var(--text); background: transparent; border: 1px solid var(--control-border); border-radius: var(--radius-control); padding: 3px 10px; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
