<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  let engine = $state("…");
  invoke<string>("engine_version").then(
    (v) => (engine = v),
    () => (engine = "unavailable"),
  );
</script>

<main>
  <h1>Persimmon</h1>
  <p class="data">engine {engine}</p>
</main>

<style>
  main {
    padding: var(--space-6);
  }
  h1 {
    margin: 0 0 var(--space-3);
    font-size: var(--text-display);
    font-weight: 600;
    letter-spacing: -0.02em;
  }
  .data {
    margin: 0;
    font-family: var(--font-data);
    font-size: var(--text-meta);
    color: var(--muted);
  }
</style>
