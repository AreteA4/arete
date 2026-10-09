import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  ARETE_API_KEY_ENV,
  classifyApiKey,
  isBrowserEnvironment,
  resetAuthKeyWarningsForTesting,
  resolveAuthConfig,
} from './auth-keys';
import { ConnectionManager } from './connection';
import { AreteError } from './types';

const SECRET = 'a4_sk_supersecretvalue';
const AGENT = 'a4_ak_agentsecretvalue';
const PUBLISHABLE = 'a4_pk_publicvalue';

function makeJwt(exp: number): string {
  const encode = (value: unknown) =>
    Buffer.from(JSON.stringify(value), 'utf-8').toString('base64url');
  return `${encode({ alg: 'none', typ: 'JWT' })}.${encode({ exp })}.signature`;
}

class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  readyState = 0;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: ((event: { code: number; reason: string }) => void) | null = null;

  constructor(public readonly url: string) {
    queueMicrotask(() => {
      this.readyState = MockWebSocket.OPEN;
      this.onopen?.();
    });
  }

  send(): void {}

  close(code = 1000, reason = ''): void {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.({ code, reason });
  }
}

function stubBrowser(): void {
  vi.stubGlobal('window', {});
  vi.stubGlobal('document', {});
}

function catchError(run: () => unknown): AreteError {
  try {
    run();
  } catch (error) {
    expect(error).toBeInstanceOf(AreteError);
    return error as AreteError;
  }
  throw new Error('expected an error');
}

function expectNoKeyMaterial(text: string): void {
  for (const key of [SECRET, AGENT, PUBLISHABLE]) {
    expect(text).not.toContain(key);
    expect(text).not.toContain(key.slice(6));
  }
}

async function connectAndCaptureAuthHeader(
  auth: ConstructorParameters<typeof ConnectionManager>[0]['auth']
): Promise<string | undefined> {
  const nowSeconds = Math.floor(Date.now() / 1000);
  const fetchMock = vi.fn().mockResolvedValue({
    ok: true,
    json: async () => ({ token: makeJwt(nowSeconds + 300), expires_at: nowSeconds + 300 }),
  });
  const manager = new ConnectionManager({
    websocketUrl: 'wss://demo.stack.arete.run',
    auth,
    fetch: fetchMock,
  });
  await manager.connect();
  manager.disconnect();
  const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
  const headers = init.headers as Record<string, string>;
  if (url !== 'https://api.arete.run/ws/sessions' || 'Origin' in headers) {
    throw new Error(`unexpected token request to ${url}: ${Object.keys(headers).join(',')}`);
  }
  return headers.Authorization;
}

describe('classifyApiKey', () => {
  it('classifies key prefixes', () => {
    expect(classifyApiKey(PUBLISHABLE)).toBe('publishable');
    expect(classifyApiKey('hspk_legacy')).toBe('publishable');
    expect(classifyApiKey(SECRET)).toBe('secret');
    expect(classifyApiKey(AGENT)).toBe('secret');
    expect(classifyApiKey('hsk_legacy')).toBe('secret');
    expect(classifyApiKey('custom-key')).toBe('unknown');
  });
});

describe('resolveAuthConfig', () => {
  beforeEach(() => {
    resetAuthKeyWarningsForTesting();
    vi.stubEnv(ARETE_API_KEY_ENV, '');
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
  });

  it('treats a page with window and document as a browser, and workers/SSR as servers', () => {
    expect(isBrowserEnvironment()).toBe(false);
    vi.stubGlobal('window', {});
    expect(isBrowserEnvironment()).toBe(false);
    vi.stubGlobal('document', {});
    expect(isBrowserEnvironment()).toBe(true);
  });

  it('keeps an explicit secretKey', () => {
    expect(resolveAuthConfig({ secretKey: SECRET })).toEqual({ secretKey: SECRET });
    expect(resolveAuthConfig({ secretKey: AGENT })).toEqual({ secretKey: AGENT });
  });

  it('falls back to ARETE_API_KEY when no auth is configured', () => {
    vi.stubEnv(ARETE_API_KEY_ENV, `  ${AGENT}\n`);
    expect(resolveAuthConfig(undefined)).toEqual({ secretKey: AGENT });
    expect(resolveAuthConfig({ tokenTransport: 'bearer' })).toEqual({
      tokenTransport: 'bearer',
      secretKey: AGENT,
    });
  });

  it('prefers any explicit auth option over ARETE_API_KEY', () => {
    vi.stubEnv(ARETE_API_KEY_ENV, AGENT);
    const getToken = async () => 'token';
    for (const auth of [
      { secretKey: SECRET },
      { publishableKey: PUBLISHABLE },
      { token: 'static' },
      { getToken },
      { tokenEndpoint: 'https://auth.example.com/token' },
    ]) {
      expect(resolveAuthConfig(auth)).toEqual(auth);
    }
  });

  it('ignores ARETE_API_KEY in a browser', () => {
    vi.stubEnv(ARETE_API_KEY_ENV, SECRET);
    stubBrowser();
    expect(resolveAuthConfig(undefined)).toBeUndefined();
  });

  it('ignores a publishable key in ARETE_API_KEY with a warning', () => {
    vi.stubEnv(ARETE_API_KEY_ENV, PUBLISHABLE);
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    expect(resolveAuthConfig(undefined)).toBeUndefined();
    expect(warn).toHaveBeenCalledTimes(1);
    expect(String(warn.mock.calls[0]?.[0])).toContain('auth.publishableKey');
    expectNoKeyMaterial(String(warn.mock.calls[0]?.[0]));
  });

  it('refuses secretKey in a browser and points at publishable keys', () => {
    stubBrowser();
    const error = catchError(() => resolveAuthConfig({ secretKey: SECRET }));
    expect(error.code).toBe('INVALID_CONFIG');
    expect(error.message).toContain('auth.publishableKey');
    expect(error.message).toContain('a4 auth keys create-publishable --origin');
    expectNoKeyMaterial(error.message);
  });

  it('refuses a publishable key passed as secretKey', () => {
    const error = catchError(() => resolveAuthConfig({ secretKey: PUBLISHABLE }));
    expect(error.code).toBe('INVALID_CONFIG');
    expect(error.message).toContain('Pass it as auth.publishableKey');
    expectNoKeyMaterial(error.message);
  });

  it('refuses an empty secretKey and secretKey combined with publishableKey', () => {
    expect(catchError(() => resolveAuthConfig({ secretKey: '  ' })).code).toBe('INVALID_CONFIG');
    const error = catchError(() =>
      resolveAuthConfig({ secretKey: SECRET, publishableKey: PUBLISHABLE })
    );
    expect(error.message).toContain('not both');
    expectNoKeyMaterial(error.message);
  });

  it('refuses a secret-class key passed as publishableKey in a browser', () => {
    stubBrowser();
    for (const key of [SECRET, AGENT]) {
      const error = catchError(() => resolveAuthConfig({ publishableKey: key }));
      expect(error.code).toBe('INVALID_CONFIG');
      expect(error.message).toContain('a4 auth keys create-publishable');
      expectNoKeyMaterial(error.message);
    }
  });

  it('warns once about a secret-class key passed as publishableKey on a server', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    expect(resolveAuthConfig({ publishableKey: SECRET })).toEqual({ publishableKey: SECRET });
    resolveAuthConfig({ publishableKey: AGENT });
    expect(warn).toHaveBeenCalledTimes(1);
    expect(String(warn.mock.calls[0]?.[0])).toContain('auth.secretKey');
    expectNoKeyMaterial(String(warn.mock.calls[0]?.[0]));
  });

  it('accepts a publishable key in a browser', () => {
    stubBrowser();
    expect(resolveAuthConfig({ publishableKey: PUBLISHABLE })).toEqual({
      publishableKey: PUBLISHABLE,
    });
  });
});

describe('ConnectionManager secretKey', () => {
  beforeEach(() => {
    resetAuthKeyWarningsForTesting();
    vi.stubEnv(ARETE_API_KEY_ENV, '');
    vi.stubGlobal('WebSocket', MockWebSocket as unknown as typeof WebSocket);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
  });

  it('sends secretKey as the token endpoint bearer credential', async () => {
    await expect(connectAndCaptureAuthHeader({ secretKey: SECRET })).resolves.toBe(
      `Bearer ${SECRET}`
    );
  });

  it('sends ARETE_API_KEY when no auth is configured', async () => {
    vi.stubEnv(ARETE_API_KEY_ENV, AGENT);
    await expect(connectAndCaptureAuthHeader(undefined)).resolves.toBe(`Bearer ${AGENT}`);
  });

  it('prefers an explicit secretKey over ARETE_API_KEY', async () => {
    vi.stubEnv(ARETE_API_KEY_ENV, AGENT);
    await expect(connectAndCaptureAuthHeader({ secretKey: SECRET })).resolves.toBe(
      `Bearer ${SECRET}`
    );
  });

  it('prefers an explicit publishableKey over ARETE_API_KEY', async () => {
    vi.stubEnv(ARETE_API_KEY_ENV, AGENT);
    await expect(connectAndCaptureAuthHeader({ publishableKey: PUBLISHABLE })).resolves.toBe(
      `Bearer ${PUBLISHABLE}`
    );
  });

  it('refuses to construct with secretKey in a browser', () => {
    stubBrowser();
    expect(
      () => new ConnectionManager({
        websocketUrl: 'wss://demo.stack.arete.run',
        auth: { secretKey: SECRET },
      })
    ).toThrow(/cannot be used in a browser/);
  });
});
