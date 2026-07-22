import { afterEach, describe, expect, it, vi } from 'vitest';

afterEach(() => {
  vi.unstubAllEnvs();
});

describe('appConfig', () => {
  it('requires a publishable key for the default hosted endpoints', async () => {
    vi.resetModules();
    const { appConfig } = await import('./config');
    expect(appConfig.areteOptions).toBeUndefined();
    expect(appConfig.publishableKey).toBeUndefined();
    expect(appConfig.configurationError).toContain('VITE_ARETE_PUBLISHABLE_KEY');
  });

  it('allows credential-free local endpoints', async () => {
    vi.resetModules();
    vi.stubEnv('VITE_ARETE_WS_URL', 'ws://127.0.0.1:8877');
    vi.stubEnv('VITE_ARETE_HTTP_URL', 'http://127.0.0.1:8877');
    const { appConfig } = await import('./config');

    expect(appConfig.publishableKey).toBeUndefined();
    expect(appConfig.configurationError).toBeNull();
  });

  it('does not let blank overrides or credentials bypass hosted authentication', async () => {
    vi.resetModules();
    vi.stubEnv('VITE_ARETE_WS_URL', '  ');
    vi.stubEnv('VITE_ARETE_HTTP_URL', '');
    vi.stubEnv('VITE_ARETE_PUBLISHABLE_KEY', '   ');
    const { appConfig } = await import('./config');

    expect(appConfig.areteOptions).toBeUndefined();
    expect(appConfig.publishableKey).toBeUndefined();
    expect(appConfig.configurationError).toContain('VITE_ARETE_PUBLISHABLE_KEY');
  });

  it('honours env overrides', async () => {
    vi.resetModules();
    vi.stubEnv('VITE_ARETE_WS_URL', 'wss://example.test');
    vi.stubEnv('VITE_ARETE_HTTP_URL', 'https://example.test');
    vi.stubEnv('VITE_ARETE_PUBLISHABLE_KEY', 'pk_test');
    const { appConfig } = await import('./config');
    expect(appConfig.areteOptions).toEqual({
      url: 'wss://example.test',
      httpUrl: 'https://example.test',
    });
    expect(appConfig.publishableKey).toBe('pk_test');
    expect(appConfig.configurationError).toBeNull();
  });

  it('rejects malformed endpoint overrides before connecting', async () => {
    vi.resetModules();
    vi.stubEnv('VITE_ARETE_WS_URL', 'not-a-url');
    vi.stubEnv('VITE_ARETE_HTTP_URL', 'ftp://example.test');
    const { appConfig } = await import('./config');

    expect(appConfig.configurationError).toBe(
      'VITE_ARETE_WS_URL must be a valid URL.',
    );
  });

  it('rejects endpoint overrides with the wrong protocol', async () => {
    vi.resetModules();
    vi.stubEnv('VITE_ARETE_WS_URL', 'https://example.test');
    vi.stubEnv('VITE_ARETE_HTTP_URL', 'wss://example.test');
    const { appConfig } = await import('./config');

    expect(appConfig.configurationError).toBe(
      'VITE_ARETE_WS_URL must use ws: or wss:.',
    );
  });
});
