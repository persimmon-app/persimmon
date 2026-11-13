<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<script lang="ts">
  let { open, onCancel, onQuit }: { open: boolean; onCancel: () => void; onQuit: () => void } = $props();
  let dialog: HTMLDialogElement | undefined = $state();
  $effect(() => {
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  });
</script>

<dialog bind:this={dialog} oncancel={(e) => { e.preventDefault(); onCancel(); }} aria-labelledby="quit-title">
  <p id="quit-title">A conversion is running. Quit and cancel it?</p>
  <div class="buttons">
    <button onclick={onCancel}>Cancel</button>
    <button class="quit" onclick={onQuit}>Quit</button>
  </div>
</dialog>

<style>
  dialog {
    background: var(--surface); color: var(--text);
    border: 1px solid var(--line); border-radius: var(--radius-card);
    padding: var(--space-5); max-width: 360px;
  }
  dialog::backdrop { background: var(--backdrop); }
  p { margin: 0 0 var(--space-4); }
  .buttons { display: flex; justify-content: flex-end; gap: var(--space-2); }
  button {
    font: inherit; color: var(--text); background: transparent;
    border: 1px solid var(--control-border); border-radius: var(--radius-control);
    height: var(--button-toolbar); padding: 0 var(--space-3);
  }
  .quit { background: var(--accent); border-color: var(--accent); color: var(--on-accent); font-weight: 600; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
