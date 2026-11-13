<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<script lang="ts">
  import type { SettingsView } from "../lib/types";
  let { settings, onDropWindow }: { settings: SettingsView; onDropWindow: (on: boolean) => void } = $props();
</script>

<section aria-labelledby="general-title">
  <h2 id="general-title">General</h2>
  <label><input type="checkbox" checked={settings.dropWindow} onchange={(e) => onDropWindow(e.currentTarget.checked)} /> Open the drop window at launch</label>
</section>

<style>
  h2 { margin: 0 0 var(--space-4); font-size: var(--text-section); font-weight: 600; }
  label { display: flex; align-items: center; gap: var(--space-2); }
  input:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
