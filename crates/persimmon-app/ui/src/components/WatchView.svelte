<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Watch folders (canvas "Watch folders"; spec D2, D9): every rule, a new one, and start at login. -->
<script lang="ts">
  import RuleCard from "./RuleCard.svelte";
  import { summary } from "../lib/watch";
  import type { NotificationsState, RuleView } from "../lib/types";

  let { views, notifications, startAtLogin, loginError = null, onNew, onEdit, onToggle, onCopy, onScan, onRemove, onStartAtLogin, onOpenNotifications }: {
    views: RuleView[]; notifications: NotificationsState; startAtLogin: boolean; loginError?: string | null;
    onNew: () => void; onEdit: (id: string) => void; onToggle: (id: string, on: boolean) => void;
    onCopy: (id: string) => void; onScan: (id: string) => void; onRemove: (id: string) => void;
    onStartAtLogin: (on: boolean) => void; onOpenNotifications: () => void;
  } = $props();
</script>

<section class="watch" aria-labelledby="watch-title">
  <header>
    <h2 id="watch-title">Watch folders</h2>
    <span class="summary">{summary(views)}</span>
    <span class="spacer"></span>
    <button class="primary" onclick={onNew}>New rule</button>
  </header>
  {#if notifications === "off" && views.length}
    <p class="banner">Notifications are off — <button class="link" onclick={onOpenNotifications}>Open System Settings</button></p>
  {/if}
  <div class="rules">
    {#each views as v (v.id)}
      <RuleCard view={v} {onToggle} {onCopy} {onEdit} {onScan} {onRemove} />
    {/each}
  </div>
  <footer>
    <span>Rules keep working when this window is closed</span>
    <label><input type="checkbox" checked={startAtLogin} onchange={(e) => onStartAtLogin(e.currentTarget.checked)} /> Start at login</label>
    {#if loginError}<span class="error">{loginError}</span>{/if}
  </footer>
</section>

<style>
  .watch { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  header { display: flex; align-items: center; gap: var(--space-3); padding: var(--space-3) var(--space-4); border-bottom: 1px solid var(--line); }
  h2 { margin: 0; font-size: var(--text-section); font-weight: 600; }
  .summary { font-family: var(--font-data); font-size: var(--text-meta); color: var(--muted); }
  .spacer { flex: 1; }
  .banner { margin: 0; padding: var(--space-2) var(--space-4); font-size: var(--text-meta); color: var(--muted); border-bottom: 1px solid var(--line); }
  .rules { flex: 1; overflow-y: auto; }
  footer { display: flex; align-items: center; gap: var(--space-4); padding: var(--space-2) var(--space-4); border-top: 1px solid var(--line); font-size: var(--text-meta); color: var(--muted); }
  footer label { display: flex; align-items: center; gap: var(--space-2); color: var(--text); }
  .error { color: var(--error-text); }
  .primary { font: inherit; font-size: var(--text-meta); background: var(--accent); border: 1px solid var(--accent); color: var(--on-accent); font-weight: 600; border-radius: var(--radius-control); padding: 3px 10px; }
  .link { font: inherit; color: var(--accent-text); background: transparent; border: 0; padding: 0; text-decoration: underline; }
  button:focus-visible, input:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
