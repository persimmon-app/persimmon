<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- The first run (canvas "First run", spec D4): the wheel, and the three ways in. -->
<script lang="ts">
  let { onWheel, onDismiss }: { onWheel: (on: boolean) => void; onDismiss: () => void } = $props();
  let step: "ask" | "on" | "gone" = $state("ask");
</script>

{#if step !== "gone"}
  <section class="first" aria-label="Getting started">
    {#if step === "ask"}
      <p class="line">Hold ⇧ while dragging a file in Finder and pick a format at the cursor</p>
      <div class="actions">
        <button class="primary" onclick={() => { onWheel(true); onDismiss(); step = "on"; }}>Show me</button>
        <button onclick={() => { onDismiss(); step = "gone"; }}>Not now</button>
      </div>
    {:else}
      <p class="line" role="status">The wheel is on. Hold ⇧ while dragging files from Finder.</p>
    {/if}
    <ul class="ways">
      <li>⇧ Drag & hold</li>
      <li>Right-click → Services → Convert with Persimmon</li>
      <li>Terminal</li>
    </ul>
  </section>
{/if}

<style>
  .first {
    max-width: 640px; margin: var(--space-6) auto 0; padding: var(--space-4) var(--space-5);
    border: 1px solid var(--accent); border-radius: var(--radius-card); background: var(--accent-tint);
  }
  .line { margin: 0 0 var(--space-3); font-weight: 600; color: var(--text); }
  .actions { display: flex; gap: var(--space-2); margin-bottom: var(--space-3); }
  button {
    font: inherit; color: var(--text); background: transparent;
    border: 1px solid var(--control-border); border-radius: var(--radius-control);
    height: var(--button); padding: 0 var(--space-4);
  }
  button.primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); font-weight: 600; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .ways { display: flex; flex-wrap: wrap; gap: var(--space-2); margin: 0; padding: 0; list-style: none; }
  .ways li {
    font-family: var(--font-data); font-size: var(--text-label); color: var(--muted);
    border: 1px solid var(--line); border-radius: var(--radius-chip); padding: 2px var(--space-2); background: var(--surface);
  }
</style>
