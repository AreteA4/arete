import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// `a4 dev up`'s dev API: the UI port, 48702 unless it was busy (see `uiUrl`
// in .arete/dev/session.json). Only the dev panel calls it.
const devApi = process.env.ARETE_DEV_UI_URL ?? "http://127.0.0.1:48702";

// `npm run app` serves app/. In development /dev/v1 is proxied to the dev
// API, which accepts requests from its own origin only, so the proxy sends
// that origin. `vite build` has no proxy: a production build cannot reach it.
export default defineConfig({
  root: "app",
  envDir: "..",
  plugins: [react()],
  server: {
    proxy: {
      "/dev/v1": { target: devApi, changeOrigin: true, headers: { origin: new URL(devApi).origin } },
    },
  },
  build: { outDir: "../dist/app", emptyOutDir: true },
});
