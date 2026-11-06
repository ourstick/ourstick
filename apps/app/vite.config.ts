import { readFileSync } from "node:fs";
import { defineConfig, type Plugin } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

/** "&", "<" and ">" as HTML text. */
function escaped(text: string): string {
  return text.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

/** OurStick's licence as a page, in the style of THIRD-PARTY-LICENSES.html. */
function licencePage(text: string): string {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>OurStick's licence</title>
<style>
  body { font: 14px/1.5 system-ui, sans-serif; max-width: 52em; margin: 2em auto; padding: 0 1em; color: #222; background: #fff; }
  pre { white-space: pre-wrap; background: #f6f6f6; padding: 1em; border-radius: 6px; font-size: 12px; }
</style>
</head>
<body>
<h1>OurStick's licence</h1>
<pre>${escaped(text)}</pre>
</body>
</html>
`;
}

// The licences that About shows in a window of their own (about.rs), made
// from the files at the top of the repository, the ones the installers carry
// beside the app. Their own window, because the system's default app may be
// unable to read a file installed with OurStick: on Ubuntu the Firefox snap
// cannot read /usr/lib.
const LICENCES: Record<string, () => string | Buffer> = {
  "LICENSE.html": () => licencePage(readFileSync(new URL("../../LICENSE", import.meta.url), "utf8")),
  "THIRD-PARTY-LICENSES.html": () => readFileSync(new URL("../../THIRD-PARTY-LICENSES.html", import.meta.url)),
};

function licencePages(): Plugin {
  return {
    name: "ourstick-licence-pages",
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const page = LICENCES[(request.url ?? "").slice(1)];
        if (!page) return next();
        response.setHeader("Content-Type", "text/html; charset=utf-8");
        response.end(page());
      });
    },
    generateBundle() {
      for (const [fileName, page] of Object.entries(LICENCES)) {
        this.emitFile({ type: "asset", fileName, source: page() });
      }
    },
  };
}

// Tauri serves the built files; in development it loads this dev server.
export default defineConfig({
  plugins: [svelte(), licencePages()],
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
