import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri serves the built files; in development it loads this dev server.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    // The oldest web views the app runs in (ADR 0027): Svelte 5 needs
    // Chrome 88 (`:where`, `replaceAll`); Safari 14 is macOS 11's. An older
    // one says so (public/old-web-view.js).
    target: ["chrome88", "safari14"],
    // No inline data: URLs for assets, so the CSP can stay strict.
    assetsInlineLimit: 0,
  },
});
