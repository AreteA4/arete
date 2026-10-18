import { fileURLToPath } from "node:url";

import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    // Fixtures import the package by name, as a real protocol project does.
    alias: { "@usearete/protocol": fileURLToPath(new URL("./src/index.ts", import.meta.url)) },
  },
  test: {
    include: ["src/**/*.test.ts"],
    testTimeout: 60_000,
  },
});
