<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Settings → Finder & menus (spec D3): where Finder's two Services are, and how to show or hide them. -->
<script lang="ts">
  let { onShortcuts, wheel, onWheel }: { onShortcuts: () => void; wheel: boolean; onWheel: (on: boolean) => void } = $props();
</script>

<section aria-labelledby="finder-title">
  <h2 id="finder-title">Finder & menus</h2>
  <div class="wheel">
    <div>
      <p class="name">The format wheel</p>
      <p class="what">Hold ⇧ while dragging files anywhere to pick a format at the pointer. It watches only for that, and asks for no permission.</p>
    </div>
    <button role="switch" aria-checked={wheel} aria-label="The format wheel" class="switch" class:on={wheel}
      onclick={() => onWheel(!wheel)}>{wheel ? "ON" : "OFF"}</button>
  </div>
  <p>In Finder, select files or folders, then right-click → Services → Convert with Persimmon or Open in Persimmon.</p>
  <dl>
    <dt>Convert with Persimmon</dt>
    <dd>Shows the format wheel at the pointer, for the selected files.</dd>
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
  .wheel { display: flex; align-items: flex-start; gap: var(--space-4); margin-bottom: var(--space-5); }
  .wheel p { margin: 0; }
  .name { font-weight: 600; }
  .what { color: var(--muted); margin-top: var(--space-1) !important; }
  .switch { font-family: var(--font-data); font-size: var(--text-label); min-width: 48px; }
  .switch.on { background: var(--accent-tint); border-color: var(--accent); color: var(--accent-text); }
</style>
