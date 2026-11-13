<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- The Settings window (canvas "Settings"): six panes. -->
<script lang="ts">
  import { onMount } from "svelte";
  import GeneralPane from "./components/GeneralPane.svelte";
  import OutputPane from "./components/OutputPane.svelte";
  import PresetsPane from "./components/PresetsPane.svelte";
  import EnginesPane from "./components/EnginesPane.svelte";
  import PrivacyPane from "./components/PrivacyPane.svelte";
  import AboutPane from "./components/AboutPane.svelte";
  import * as ipc from "./lib/ipc";
  import type { EnginesView, PresetView, SettingsView } from "./lib/types";

  type Pane = "general" | "output" | "presets" | "engines" | "privacy" | "about";
  const panes: { id: Pane; label: string }[] = [
    { id: "general", label: "General" }, { id: "output", label: "Output" }, { id: "presets", label: "Presets" },
    { id: "engines", label: "Engines" }, { id: "privacy", label: "Privacy" }, { id: "about", label: "About & licences" },
  ];
  let pane: Pane = $state("general");
  let settings: SettingsView | null = $state(null);
  let presets: PresetView[] = $state([]);
  let engines: EnginesView | null = $state(null);

  onMount(() => {
    ipc.getSettings().then((s) => (settings = s));
    ipc.presets().then((p) => (presets = p));
    ipc.engines().then((e) => (engines = e));
    const off = [ipc.onSettings((s) => (settings = s)), ipc.onPresets((p) => (presets = p))];
    return () => off.forEach((p) => p.then((f) => f()));
  });

  async function savePreset(p: ipc.PresetDraft): Promise<string | null> {
    try { presets = await ipc.savePreset(p); return null; } catch (e) { return String(e); }
  }
  async function deletePreset(id: string): Promise<string | null> {
    try { presets = await ipc.deletePreset(id); return null; } catch (e) { return String(e); }
  }
</script>

<div class="settings">
  <nav aria-label="Settings">
    {#each panes as p (p.id)}
      <button class:current={pane === p.id} aria-current={pane === p.id ? "page" : undefined} onclick={() => (pane = p.id)}>{p.label}</button>
    {/each}
  </nav>
  <main>
    {#if pane === "general" && settings}
      <GeneralPane {settings} onDropWindow={(on) => ipc.setDropWindowAtLaunch(on).then((s) => (settings = s))} />
    {:else if pane === "output" && settings}
      <OutputPane {settings}
        onBeside={() => ipc.setOutputBeside().then((s) => (settings = s))}
        onChooseFolder={() => ipc.chooseOutputFolder().then((s) => (settings = s))}
        onChooseFallback={() => ipc.chooseFallbackFolder().then((s) => (settings = s))} />
    {:else if pane === "presets"}
      <PresetsPane {presets} onSave={savePreset} onDelete={deletePreset} />
    {:else if pane === "engines"}
      <EnginesPane {engines} />
    {:else if pane === "privacy"}
      <PrivacyPane />
    {:else if pane === "about"}
      <AboutPane version={engines?.version ?? ""} onLicences={() => ipc.openLicences()} onSources={() => ipc.openSources()} />
    {/if}
  </main>
</div>

<style>
  .settings { display: grid; grid-template-columns: 170px 1fr; height: 100vh; }
  nav { padding: var(--space-4) var(--space-3); border-right: 1px solid var(--line); background: var(--surface); display: flex; flex-direction: column; gap: 2px; }
  nav button { text-align: left; font: inherit; color: var(--muted); background: transparent; border: 0; border-radius: var(--radius-control); padding: 6px var(--space-2); }
  nav button.current { background: var(--accent-tint); color: var(--accent-text); }
  nav button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  main { padding: var(--space-5); overflow-y: auto; }
</style>
