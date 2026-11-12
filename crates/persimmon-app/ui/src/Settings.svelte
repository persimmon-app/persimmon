<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- The Settings window. One pane for now; the rest arrive with week 4. -->
<script lang="ts">
  import { onMount } from "svelte";
  import OutputPane from "./components/OutputPane.svelte";
  import * as ipc from "./lib/ipc";
  import type { SettingsView } from "./lib/types";

  let settings: SettingsView | null = $state(null);
  onMount(() => {
    ipc.getSettings().then((s) => (settings = s));
    const off = ipc.onSettings((s) => (settings = s));
    return () => { off.then((f) => f()); };
  });
</script>

<div class="settings">
  <nav aria-label="Settings"><span class="current">Output</span></nav>
  <main>
    {#if settings}
      <OutputPane {settings}
        onBeside={() => ipc.setOutputBeside().then((s) => (settings = s))}
        onChooseFolder={() => ipc.chooseOutputFolder().then((s) => (settings = s))}
        onChooseFallback={() => ipc.chooseFallbackFolder().then((s) => (settings = s))} />
    {/if}
  </main>
</div>

<style>
  .settings { display: grid; grid-template-columns: 160px 1fr; height: 100vh; }
  nav { padding: var(--space-4) var(--space-3); border-right: 1px solid var(--line); background: var(--surface); }
  .current { display: block; padding: 6px var(--space-2); border-radius: var(--radius-control); background: var(--accent-tint); color: var(--accent-text); }
  main { padding: var(--space-5); overflow-y: auto; }
</style>
