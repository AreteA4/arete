import { createRequire } from 'node:module';
import { defineConfig } from 'vitest/config';

const require = createRequire(import.meta.url);
const fromSdk = createRequire(require.resolve('@usearete/sdk'));

export default defineConfig({
  resolve: {
    alias: { zod: fromSdk.resolve('zod') },
    dedupe: ['@usearete/sdk'],
  },
});
