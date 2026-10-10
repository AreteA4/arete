import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    env: {
      // Keep the developer's own `a4` login out of tests: the SDK falls back
      // to it when no key is configured.
      ARETE_CREDENTIALS_PATH: '/nonexistent/arete-sdk-tests/credentials.toml',
    },
  },
});
