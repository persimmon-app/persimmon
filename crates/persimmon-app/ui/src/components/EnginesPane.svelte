<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<script lang="ts">
  import type { EnginesView } from "../lib/types";
  let { engines }: { engines: EnginesView | null } = $props();
</script>

<section aria-labelledby="engines-title">
  <h2 id="engines-title">Engines</h2>
  {#if engines}
    <dl>
      <dt>FFmpeg</dt>
      <dd>
        {#if engines.ffmpeg}<span class="data">{engines.ffmpeg.version}</span> <span class="data path">{engines.ffmpeg.path}</span>
        {:else if engines.ffmpegError}<span class="why">{engines.ffmpegError.reason}.{#if engines.ffmpegError.fix}{" "}{engines.ffmpegError.fix}{/if}</span>{/if}
      </dd>
      <dt>Images (libvips)</dt><dd>{engines.images ? "Built in" : "Not in this build"}</dd>
      <dt>RAR (libarchive)</dt><dd>{engines.rar ? "Built in" : "Not in this build"}</dd>
      <dt>Persimmon</dt><dd class="data">{engines.version}</dd>
    </dl>
  {/if}
</section>

<style>
  h2 { margin: 0 0 var(--space-4); font-size: var(--text-section); font-weight: 600; }
  dl { display: grid; grid-template-columns: 160px 1fr; gap: var(--space-2) var(--space-4); margin: 0; }
  dt { color: var(--muted); }
  dd { margin: 0; }
  .data { font-family: var(--font-data); font-size: var(--text-meta); }
  .path { color: var(--muted); margin-left: var(--space-2); }
  .why { color: var(--error-text); font-size: var(--text-meta); }
</style>
