import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// `npm run dev` serves the UI on 5173 and proxies the a4 dev API to the
// session given by A4_DEV_UI (default the default port base's UI port).
// Run `a4 dev up --ui-dev-url http://localhost:5173` to load it through a4.
const target = (globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env.A4_DEV_UI ?? "http://127.0.0.1:48702";

export default defineConfig({
  plugins: [react()],
  base: "/",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
    chunkSizeWarningLimit: 2000,
    assetsInlineLimit: 0,
  },
  server: {
    port: 5173,
    strictPort: true,
    hmr: { clientPort: 5173 },
    proxy: { "/dev/v1": { target, changeOrigin: true } },
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
  },
});
