<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- The canvas's icon style: a 20 px grid, 1.6 stroke, round caps, never filled. -->
<script lang="ts">
  let { name, size = 16 }: { name: "done" | "error" | "stop" | "queued" | "arrow" | "add" | "close" | "folder" | "retry"; size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 20 20" fill="none" stroke="currentColor"
  stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#if name === "done"}<path d="M4.5 10.5l3.5 3.5 7.5-8" />
  {:else if name === "error"}<circle cx="10" cy="10" r="7" /><path d="M10 6.5v4.5M10 13.5v.01" />
  {:else if name === "stop"}<rect x="5" y="5" width="10" height="10" rx="1.5" />
  {:else if name === "queued"}<circle cx="10" cy="10" r="7" /><path d="M10 6.5V10l2.5 1.5" />
  {:else if name === "arrow"}<path d="M4 10h11M11 6l4 4-4 4" />
  {:else if name === "add"}<path d="M10 4v12M4 10h12" />
  {:else if name === "close"}<path d="M5.5 5.5l9 9M14.5 5.5l-9 9" />
  {:else if name === "folder"}<path d="M3 6.5V15a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V8a1 1 0 0 0-1-1H9.5L8 5H4a1 1 0 0 0-1 1.5z" />
  {:else if name === "retry"}<path d="M15.5 10a5.5 5.5 0 1 1-1.6-3.9M15.5 4.5v3h-3" />
  {/if}
</svg>
