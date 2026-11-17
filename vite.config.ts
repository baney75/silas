import { readFile } from "node:fs/promises";
import { basename, join } from "node:path";
import { defineConfig, type Plugin } from "vite";

// Vite type-checks and runs this file with Node; tsc only checks src/.
declare const process: { env: Record<string, string | undefined> };
const host = process.env.TAURI_DEV_HOST;

// Dev server only. Serves theme files from the folder in SILAS_THEME_DIR at /__theme/<name>, so the
// browser mock can show a theme (src/mock.ts, `?themeFile=`). Not part of `vite build`.
function themePreview(): Plugin {
  const dir = process.env.SILAS_THEME_DIR;
  return {
    name: "silas-theme-preview",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/__theme", async (req, res) => {
        try {
          const name = basename(decodeURIComponent((req.url ?? "").split("?")[0]));
          if (!dir || !/^[\w.-]+\.json$/.test(name)) throw new Error("no");
          res.setHeader("content-type", "application/json");
          res.end(await readFile(join(dir, name)));
        } catch {
          res.statusCode = 404;
          res.end("not found");
        }
      });
    },
  };
}

export default defineConfig({
  plugins: [themePreview()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: process.env.TAURI_ENV_DEBUG ? false : "esbuild",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    rollupOptions: {
      input: {
        main: "index.html",
        overlay: "overlay.html",
      },
    },
  },
});
