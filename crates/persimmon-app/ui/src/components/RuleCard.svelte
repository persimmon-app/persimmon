<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- One watch rule (canvas "Watch folders"; spec D2): its switch, its line and its command. -->
<script lang="ts">
  import { failedLine, statusLine } from "../lib/watch";
  import type { RuleView } from "../lib/types";

  let { view, onToggle, onCopy, onEdit, onScan, onRemove }: {
    view: RuleView; onToggle: (id: string, on: boolean) => void; onCopy: (id: string) => void;
    onEdit: (id: string) => void; onScan: (id: string) => void; onRemove: (id: string) => void;
  } = $props();
  let confirm: HTMLDialogElement | undefined = $state();
  const converting = $derived(view.status.state === "converting");
  const fix = $derived(view.status.state === "unreadable" ? view.status.error.fix : null);
  const failed = $derived(failedLine(view));
</script>

<article class="rule" class:converting aria-label="Watch rule for {view.folderLabel}">
  <div class="head">
    <span class="folder">{view.folderLabel}</span>
    <span class="target">→ {view.target.toUpperCase()}</span>
    <span class="spacer"></span>
    <button role="switch" aria-checked={view.enabled} aria-label="Watch {view.folderLabel}" class="switch" class:on={view.enabled}
      onclick={() => onToggle(view.id, !view.enabled)}>{view.enabled ? "ACTIVE" : "PAUSED"}</button>
  </div>
  <p class="line">{statusLine(view)}</p>
  {#if fix}<p class="fix">{fix}</p>{/if}
  {#if failed}<p class="failed">{failed}</p>{/if}
  {#if view.status.note}<p class="note">{view.status.note}</p>{/if}
  <div class="command">
    <code>{view.command}</code>
    <button class="text" aria-label="Copy the command for {view.folderLabel}" onclick={() => onCopy(view.id)}>Copy</button>
  </div>
  <div class="actions">
    <button class="text" aria-label="Edit the rule for {view.folderLabel}" onclick={() => onEdit(view.id)}>Edit</button>
    <button class="text" aria-label="Scan now: {view.folderLabel}" onclick={() => onScan(view.id)}>Scan now</button>
    <button class="text" aria-label="Remove the rule for {view.folderLabel}" onclick={() => confirm?.showModal()}>Remove</button>
  </div>
  <dialog bind:this={confirm} aria-label="Remove rule">
    <p>Stop watching {view.folderLabel}? Files there stay as they are.</p>
    <div class="buttons">
      <button onclick={() => confirm?.close()}>Cancel</button>
      <button class="primary" onclick={() => { confirm?.close(); onRemove(view.id); }}>Stop watching</button>
    </div>
  </dialog>
</article>

<style>
  .rule { border-bottom: 1px solid var(--line); padding: var(--space-3) var(--space-4); }
  .head { display: flex; align-items: center; gap: var(--space-3); }
  .folder { font-weight: 600; }
  .target, code { font-family: var(--font-data); font-size: var(--text-meta); color: var(--muted); }
  .spacer { flex: 1; }
  .line, .fix, .failed, .note { margin: var(--space-1) 0 0; font-size: var(--text-meta); color: var(--muted); }
  .converting .line { color: var(--accent-text); }
  .fix, .failed { color: var(--error-text); }
  .command { display: flex; align-items: center; gap: var(--space-2); margin-top: var(--space-2); }
  code { flex: 1; min-width: 0; overflow-x: auto; white-space: nowrap; }
  .actions { display: flex; gap: var(--space-2); margin-top: var(--space-2); }
  .switch { font-family: var(--font-data); font-size: var(--text-label); letter-spacing: 0.08em; padding: 2px 8px; border-radius: var(--radius-badge); border: 1px solid var(--line); background: transparent; color: var(--muted); }
  .switch.on { border-color: var(--accent); color: var(--accent-text); }
  button.text, .buttons button { font: inherit; font-size: var(--text-meta); color: var(--text); background: transparent; border: 1px solid var(--control-border); border-radius: var(--radius-control); padding: 3px 10px; }
  .buttons { display: flex; justify-content: flex-end; gap: var(--space-2); }
  .buttons .primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); font-weight: 600; }
  dialog { background: var(--surface); color: var(--text); border: 1px solid var(--line); border-radius: var(--radius-card); padding: var(--space-5); max-width: 380px; }
  dialog::backdrop { background: var(--backdrop); }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
