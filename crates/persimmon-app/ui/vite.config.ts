// SPDX-License-Identifier: GPL-3.0-or-later
import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri serves the built files; in development it loads this dev server.
export default defineConfig(({ mode }) => ({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  // assetsInlineLimit 0: no font or image is inlined as a data: URL, which
  // the production CSP (font-src 'self') would block.
  build: {
    target: "safari17", outDir: "dist", emptyOutDir: true, assetsInlineLimit: 0,
    // Three windows, three pages: the queue, the drop window, Settings.
    rollupOptions: { input: { main: "index.html", drop: "drop.html", settings: "settings.html" } },
  },
  // Svelte 5 components need its browser build under Vitest.
  resolve: mode === "test" ? { conditions: ["browser"] } : undefined,
  test: { environment: "jsdom", include: ["src/**/*.test.ts"], setupFiles: ["src/test-setup.ts"] },
}));
