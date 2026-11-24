<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<script lang="ts">
  import type { SettingsView } from "../lib/types";
  let { settings, onDropWindow, onStartAtLogin = () => {}, loginError = null }: {
    settings: SettingsView; onDropWindow: (on: boolean) => void;
    /** Start at login (spec D2): Persimmon opens with no window and its rules run. */
    onStartAtLogin?: (on: boolean) => void; loginError?: string | null;
  } = $props();
</script>

<section aria-labelledby="general-title">
  <h2 id="general-title">General</h2>
  <label><input type="checkbox" checked={settings.dropWindow} onchange={(e) => onDropWindow(e.currentTarget.checked)} /> Open the drop window at launch</label>
  <label><input type="checkbox" checked={settings.startAtLogin} onchange={(e) => onStartAtLogin(e.currentTarget.checked)} /> Start at login</label>
  {#if loginError}<p class="error">{loginError}</p>{/if}
</section>

<style>
  h2 { margin: 0 0 var(--space-4); font-size: var(--text-section); font-weight: 600; }
  label { display: flex; align-items: center; gap: var(--space-2); margin-bottom: var(--space-2); }
  .error { margin: 0 0 0 24px; font-size: var(--text-meta); color: var(--error-text); }
  input:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
