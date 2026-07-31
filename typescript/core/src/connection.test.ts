import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ConnectionManager, isHostedAreteEndpoint } from './connection';
import { SubscriptionRegistry } from './subscription';
import { QueryStore } from './query-store';
import { MemoryAdapter } from './storage/memory-adapter';
import {
  AreteError,
  type AuthTokenRequest,
  type ProgramReadBindingAuthTarget,
  type SolanaGatewayBindingAuthTarget,
  type Subscription,
  type SubscriptionQuery,
} from './types';

const PROGRAM_READ_BINDING_1 = 'prb_00000000000000000000000000000001';
const PROGRAM_READ_BINDING_2 = 'prb_00000000000000000000000000000002';
const SOLANA_GATEWAY_BINDING = 'sgb_00000000000000000000000000000001';

function toBase64Url(value: string): string {
  return Buffer.from(value, 'utf-8')
    .toString('base64')
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/g, '');
}

function makeJwt(exp: number): string {
  const header = toBase64Url(JSON.stringify({ alg: 'none', typ: 'JWT' }));
  const payload = toBase64Url(JSON.stringify({ exp }));
  return `${header}.${payload}.signature`;
}

function programReadTarget(
  targetId = PROGRAM_READ_BINDING_1,
  programReleaseHash = 'release-1'
): ProgramReadBindingAuthTarget {
  return {
    targetKind: 'program-read-binding',
    targetId,
    programReleaseHash,
  };
}

function solanaGatewayTarget(): SolanaGatewayBindingAuthTarget {
  return {
    targetKind: 'solana-gateway-binding',
    targetId: SOLANA_GATEWAY_BINDING,
  };
}

function subscription(
  subscriptionId: string,
  query: SubscriptionQuery,
  snapshotEnabled = true
): Subscription {
  return {
    type: 'subscribe',
    protocolVersion: 2,
    subscriptionId,
    query,
    snapshot: { enabled: snapshotEnabled },
  };
}

function makeErrorResponse(
  status: number,
  body: { error: string; code?: string } | string,
  headerCode?: string
) {
  const rawBody = typeof body === 'string' ? body : JSON.stringify(body);
  const headers = new Headers();

  if (headerCode) {
    headers.set('X-Error-Code', headerCode);
  }

  return {
    ok: false,
    status,
    statusText: 'Request failed',
    headers,
    text: async () => rawBody,
  };
}

class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  static instances: MockWebSocket[] = [];

  readyState = MockWebSocket.CONNECTING;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void | Promise<void>) | null = null;
  onerror: (() => void) | null = null;
  onclose: ((event: { code: number; reason: string }) => void) | null = null;
  sent: string[] = [];

  constructor(public readonly url: string) {
    MockWebSocket.instances.push(this);
    queueMicrotask(() => {
      this.readyState = MockWebSocket.OPEN;
      this.onopen?.();
    });
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(code = 1000, reason = ''): void {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.({ code, reason });
  }
}

class FactoryWebSocket extends MockWebSocket {
  constructor(
    url: string,
    public readonly init?: { headers?: Record<string, string> }
  ) {
    super(url);
  }
}

describe('hosted endpoint classification', () => {
  it('recognizes hosted stack endpoints without accepting suffix lookalikes', () => {
    expect(isHostedAreteEndpoint('wss://ore.stack.arete.run')).toBe(true);
    expect(isHostedAreteEndpoint('https://ore.stack.arete.run')).toBe(true);
    expect(isHostedAreteEndpoint('wss://stack.arete.run.example.com')).toBe(false);
    expect(isHostedAreteEndpoint('ws://127.0.0.1:8877')).toBe(false);
    expect(isHostedAreteEndpoint('not-a-url')).toBe(false);
  });
});

describe('ConnectionManager auth', () => {
  beforeEach(() => {
    MockWebSocket.instances = [];
    vi.stubGlobal('WebSocket', MockWebSocket as unknown as typeof WebSocket);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('mints an anonymous hosted session token when no publishable key is configured', async () => {
    const nowSeconds = Math.floor(Date.now() / 1000);
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        token: makeJwt(nowSeconds + 300),
        expires_at: nowSeconds + 300,
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const manager = new ConnectionManager({
      websocketUrl: 'wss://demo.stack.arete.run',
    });

    await manager.connect();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith(
      'https://api.arete.run/ws/sessions',
      expect.objectContaining({ method: 'POST' })
    );

    const requestInit = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(JSON.parse(String(requestInit.body))).toEqual({
      websocket_url: 'wss://demo.stack.arete.run',
      scopes: ['read'],
    });
    expect(requestInit.headers).not.toMatchObject({
      Authorization: expect.anything(),
    });
    expect(MockWebSocket.instances[0]?.url).toContain('hs_token=');
  });

  it('fetches a hosted session token when a publishable key is configured', async () => {
    const nowSeconds = Math.floor(Date.now() / 1000);
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        token: makeJwt(nowSeconds + 300),
        expires_at: nowSeconds + 300,
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const manager = new ConnectionManager({
      websocketUrl: 'wss://demo.stack.arete.run',
      auth: { publishableKey: 'hspk_test_123' },
    });

    await manager.connect();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith(
      'https://api.arete.run/ws/sessions',
      expect.objectContaining({ method: 'POST' })
    );

    const requestInit = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(JSON.parse(String(requestInit.body))).toEqual({
      websocket_url: 'wss://demo.stack.arete.run',
      scopes: ['read'],
    });
    expect(requestInit.headers).toMatchObject({
      Authorization: 'Bearer hspk_test_123',
    });
    expect(MockWebSocket.instances[0]?.url).toContain('hs_token=');
  });

  it('sends the publishable key when provided for hosted auth', async () => {
    const nowSeconds = Math.floor(Date.now() / 1000);
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        token: makeJwt(nowSeconds + 300),
        expires_at: nowSeconds + 300,
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const manager = new ConnectionManager({
      websocketUrl: 'wss://global.stack.arete.run',
      auth: { publishableKey: 'hspk_test_123' },
    });

    await manager.connect();

    const requestInit = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(requestInit.headers).toMatchObject({
      Authorization: 'Bearer hspk_test_123',
    });
  });

  it('fails clearly when the hosted auth server rejects the request', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      makeErrorResponse(401, {
        error: 'Authentication required to mint websocket session tokens.',
        code: 'auth-required',
      })
    );
    vi.stubGlobal('fetch', fetchMock);

    const manager = new ConnectionManager({
      websocketUrl: 'wss://global.stack.arete.run',
      auth: { publishableKey: 'hspk_test_123' },
    });

    await expect(manager.connect()).rejects.toMatchObject<Partial<AreteError>>({
      code: 'AUTH_REQUIRED',
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('surfaces platform origin-required errors from the token endpoint', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      makeErrorResponse(
        403,
        {
          error: 'Publishable key requires Origin header',
          code: 'origin-required',
        },
        'origin-required'
      )
    );
    vi.stubGlobal('fetch', fetchMock);

    const manager = new ConnectionManager({
      websocketUrl: 'wss://global.stack.arete.run',
      auth: { publishableKey: 'hspk_test_123' },
    });

    await expect(manager.connect()).rejects.toMatchObject<Partial<AreteError>>({
      code: 'ORIGIN_REQUIRED',
      details: expect.objectContaining({ wireErrorCode: 'origin-required' }),
    });
  });

  it('surfaces platform websocket session rate-limit errors from the token endpoint', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      makeErrorResponse(
        429,
        {
          error: 'WebSocket session mint rate limit exceeded',
          code: 'websocket-session-rate-limit-exceeded',
        },
        'websocket-session-rate-limit-exceeded'
      )
    );
    vi.stubGlobal('fetch', fetchMock);

    const manager = new ConnectionManager({
      websocketUrl: 'wss://global.stack.arete.run',
      auth: { publishableKey: 'hspk_test_123' },
    });

    await expect(manager.connect()).rejects.toMatchObject<Partial<AreteError>>({
      code: 'WEBSOCKET_SESSION_RATE_LIMIT_EXCEEDED',
      details: expect.objectContaining({
        wireErrorCode: 'websocket-session-rate-limit-exceeded',
      }),
    });
  });

  it('refreshes expiring tokens in the background via in-band refresh', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-03-28T12:00:00Z'));

    const nowSeconds = Math.floor(Date.now() / 1000);
    const newToken = makeJwt(nowSeconds + 3600);
    const getToken = vi
      .fn<[], Promise<{ token: string }>>()
      .mockResolvedValueOnce({ token: makeJwt(nowSeconds + 61) })
      .mockResolvedValueOnce({ token: newToken });

    const manager = new ConnectionManager({
      websocketUrl: 'wss://refresh.stack.arete.run',
      auth: { getToken },
    });

    await manager.connect();
    expect(getToken).toHaveBeenCalledTimes(1);
    expect(MockWebSocket.instances).toHaveLength(1);

    const ws = MockWebSocket.instances[0]!;
    expect(ws.sent).toHaveLength(0);

    await vi.advanceTimersByTimeAsync(1_100);

    // Should refresh token but NOT reconnect - use in-band refresh instead
    expect(getToken).toHaveBeenCalledTimes(2);
    expect(MockWebSocket.instances).toHaveLength(1); // Still only 1 WebSocket

    // Should have sent refresh_auth message
    expect(ws.sent).toHaveLength(1);
    const sentMsg = JSON.parse(ws.sent[0]!);
    expect(sentMsg).toEqual({
      type: 'refresh_auth',
      token: newToken,
    });
  });

  it('does not let an abandoned token refresh block refreshes after reconnecting', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-03-28T12:00:00Z'));

    const nowSeconds = Math.floor(Date.now() / 1000);
    const getToken = vi
      .fn<[], Promise<{ token: string }>>()
      .mockResolvedValueOnce({ token: makeJwt(nowSeconds + 61) })
      .mockImplementationOnce(() => new Promise(() => undefined))
      .mockResolvedValueOnce({ token: makeJwt(nowSeconds + 62) })
      .mockResolvedValueOnce({ token: makeJwt(nowSeconds + 3600) });
    const manager = new ConnectionManager({
      websocketUrl: 'wss://refresh.stack.arete.run',
      auth: { getToken },
    });

    await manager.connect();
    await vi.advanceTimersByTimeAsync(1_100);
    expect(getToken).toHaveBeenCalledTimes(2);

    manager.disconnect();
    await manager.connect();
    await vi.advanceTimersByTimeAsync(1_100);

    expect(getToken).toHaveBeenCalledTimes(4);
    expect(MockWebSocket.instances).toHaveLength(2);
    expect(manager.getState()).toBe('connected');
  });

  it('reuses a program-read token for the same exact target and scopes', async () => {
    const getToken = vi.fn(async (request?: AuthTokenRequest) => ({
      token: 'program-read-token',
      scopes: request?.scopes,
    }));
    const manager = new ConnectionManager({ websocketUrl: null, auth: { getToken } });
    const request = {
      ...programReadTarget(),
      scopes: ['read'],
    } as const;

    await expect(manager.getHttpAuthToken(request)).resolves.toBe('program-read-token');
    await expect(manager.getHttpAuthToken(request)).resolves.toBe('program-read-token');

    expect(getToken).toHaveBeenCalledTimes(1);
    expect(getToken).toHaveBeenCalledWith({
      targetKind: 'program-read-binding',
      targetId: PROGRAM_READ_BINDING_1,
      programReleaseHash: 'release-1',
      scopes: ['read'],
    });
  });

  it('does not reuse a program-read token across bindings', async () => {
    const getToken = vi.fn(async (request?: AuthTokenRequest) => ({
      token: `token-${request?.targetId}`,
      scopes: request?.scopes,
    }));
    const manager = new ConnectionManager({ websocketUrl: null, auth: { getToken } });

    await expect(manager.getHttpAuthToken(['read'], false, programReadTarget(PROGRAM_READ_BINDING_1)))
      .resolves.toBe(`token-${PROGRAM_READ_BINDING_1}`);
    await expect(manager.getHttpAuthToken(programReadTarget(PROGRAM_READ_BINDING_2)))
      .resolves.toBe(`token-${PROGRAM_READ_BINDING_2}`);
    await expect(manager.getHttpAuthToken(programReadTarget(PROGRAM_READ_BINDING_1)))
      .resolves.toBe(`token-${PROGRAM_READ_BINDING_1}`);

    expect(getToken).toHaveBeenCalledTimes(2);
  });

  it('does not reuse a program-read token across exact releases', async () => {
    const getToken = vi.fn(async (request?: AuthTokenRequest) => ({
      token: `token-${request?.programReleaseHash}`,
      scopes: request?.scopes,
    }));
    const manager = new ConnectionManager({ websocketUrl: null, auth: { getToken } });

    await expect(manager.getHttpAuthToken(programReadTarget(PROGRAM_READ_BINDING_1, 'release-1')))
      .resolves.toBe('token-release-1');
    await expect(manager.getHttpAuthToken(programReadTarget(PROGRAM_READ_BINDING_1, 'release-2')))
      .resolves.toBe('token-release-2');
    await expect(manager.getHttpAuthToken(programReadTarget(PROGRAM_READ_BINDING_1, 'release-1')))
      .resolves.toBe('token-release-1');

    expect(getToken).toHaveBeenCalledTimes(2);
  });

  it('normalizes targeted scope order and duplicates for cache and in-flight identity', async () => {
    let resolveToken!: (result: { token: string; scopes: readonly string[] }) => void;
    const getToken = vi.fn((request?: AuthTokenRequest) =>
      new Promise<{ token: string; scopes: readonly string[] }>((resolve) => {
        resolveToken = resolve;
      }));
    const manager = new ConnectionManager({ websocketUrl: null, auth: { getToken } });
    const target = programReadTarget();

    const first = manager.getHttpAuthToken({
      ...target,
      scopes: ['read:batch', 'read', 'read:batch'],
    });
    const second = manager.getHttpAuthToken({
      ...target,
      scopes: ['read', 'read:batch'],
    });
    await vi.waitFor(() => expect(getToken).toHaveBeenCalledTimes(1));
    expect(getToken.mock.calls[0]?.[0]?.scopes).toEqual(['read', 'read:batch']);

    resolveToken({ token: 'normalized-token', scopes: ['read:batch', 'read'] });
    await expect(Promise.all([first, second]))
      .resolves.toEqual(['normalized-token', 'normalized-token']);
  });

  it('keeps in-flight program-read token requests isolated by target identity', async () => {
    const resolvers = new Map<
      string,
      (result: { token: string; scopes: readonly string[] }) => void
    >();
    const getToken = vi.fn((request?: AuthTokenRequest) =>
      new Promise<{ token: string; scopes: readonly string[] }>((resolve) => {
        resolvers.set(request?.targetId ?? 'legacy', resolve);
      }));
    const manager = new ConnectionManager({ websocketUrl: null, auth: { getToken } });

    const first = manager.getHttpAuthToken(programReadTarget(PROGRAM_READ_BINDING_1));
    const second = manager.getHttpAuthToken(programReadTarget(PROGRAM_READ_BINDING_2));
    await vi.waitFor(() => expect(getToken).toHaveBeenCalledTimes(2));

    resolvers.get(PROGRAM_READ_BINDING_2)?.({ token: 'token-2', scopes: ['read'] });
    resolvers.get(PROGRAM_READ_BINDING_1)?.({ token: 'token-1', scopes: ['read'] });
    await expect(Promise.all([first, second])).resolves.toEqual(['token-1', 'token-2']);
  });

  it('sends camelCase target fields to token endpoints while preserving legacy websocket_url', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ token: 'session-token', scopes: ['read'] }),
    });
    vi.stubGlobal('fetch', fetchMock);
    const manager = new ConnectionManager({
      websocketUrl: 'wss://demo.stack.arete.run',
      auth: { tokenEndpoint: 'https://auth.example/sessions' },
    });

    await manager.getHttpAuthToken(programReadTarget());
    await manager.getHttpAuthToken(['read']);

    const targetedInit = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(JSON.parse(String(targetedInit.body))).toEqual({
      targetKind: 'program-read-binding',
      targetId: PROGRAM_READ_BINDING_1,
      programReleaseHash: 'release-1',
      scopes: ['read'],
    });
    const legacyInit = fetchMock.mock.calls[1]?.[1] as RequestInit;
    expect(JSON.parse(String(legacyInit.body))).toEqual({
      websocket_url: 'wss://demo.stack.arete.run',
      scopes: ['read'],
    });
  });

  it('serializes and caches Solana gateway tokens by exact target and scope set', async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      const request = JSON.parse(String(init?.body)) as AuthTokenRequest;
      return new Response(JSON.stringify({
        token: `gateway-${request.scopes.join('+')}`,
        scopes: request.scopes,
      }), { status: 200 });
    });
    const manager = new ConnectionManager({
      websocketUrl: null,
      auth: { tokenEndpoint: 'https://auth.example/sessions' },
      fetch: fetchMock as typeof fetch,
    });
    const target = solanaGatewayTarget();

    await expect(manager.getHttpAuthToken(target, ['read']))
      .resolves.toBe('gateway-read');
    await expect(manager.getHttpAuthToken(['read'], target))
      .resolves.toBe('gateway-read');
    await expect(manager.getHttpAuthToken(target, ['transaction:inspect']))
      .resolves.toBe('gateway-transaction:inspect');
    await expect(manager.getHttpAuthToken({
      ...target,
      scopes: ['transaction:send'],
    })).resolves.toBe('gateway-transaction:send');

    expect(fetchMock).toHaveBeenCalledTimes(3);
    expect(fetchMock.mock.calls.map(([, init]) => JSON.parse(String(init?.body)))).toEqual([
      {
        targetKind: 'solana-gateway-binding',
        targetId: SOLANA_GATEWAY_BINDING,
        scopes: ['read'],
      },
      {
        targetKind: 'solana-gateway-binding',
        targetId: SOLANA_GATEWAY_BINDING,
        scopes: ['transaction:inspect'],
      },
      {
        targetKind: 'solana-gateway-binding',
        targetId: SOLANA_GATEWAY_BINDING,
        scopes: ['transaction:send'],
      },
    ]);
  });

  it('does not expose program-read tokens to websocket or transaction token calls', async () => {
    const getToken = vi.fn(async (request?: AuthTokenRequest) => ({
      token: request?.targetKind
        ? 'program-read-token'
        : `legacy-${request?.scopes.join('+')}`,
      scopes: request?.scopes,
    }));
    const manager = new ConnectionManager({
      websocketUrl: 'ws://localhost:8878',
      auth: { getToken },
    });

    await expect(manager.getHttpAuthToken(programReadTarget()))
      .resolves.toBe('program-read-token');
    await manager.connect();
    expect(MockWebSocket.instances[0]?.url).toContain('hs_token=legacy-read');
    expect(MockWebSocket.instances[0]?.url).not.toContain('program-read-token');
    await expect(manager.getHttpAuthToken(['transaction:send']))
      .resolves.toBe('legacy-read+transaction:send');

    expect(getToken.mock.calls.map(([request]) => request)).toEqual([
      {
        targetKind: 'program-read-binding',
        targetId: PROGRAM_READ_BINDING_1,
        programReleaseHash: 'release-1',
        scopes: ['read'],
      },
      { scopes: ['read'] },
      { scopes: ['read', 'transaction:send'] },
    ]);
    manager.disconnect();
  });

  it('clears only the requested targeted HTTP token identity', async () => {
    const getToken = vi.fn(async (request?: AuthTokenRequest) => ({
      token: `${request?.targetId}-${getToken.mock.calls.length}`,
      scopes: request?.scopes,
    }));
    const manager = new ConnectionManager({ websocketUrl: null, auth: { getToken } });
    const firstTarget = programReadTarget(PROGRAM_READ_BINDING_1);
    const secondTarget = programReadTarget(PROGRAM_READ_BINDING_2);

    await manager.getHttpAuthToken(firstTarget);
    await manager.getHttpAuthToken(secondTarget);
    manager.clearHttpAuthToken(firstTarget);

    await expect(manager.getHttpAuthToken(secondTarget))
      .resolves.toBe(`${PROGRAM_READ_BINDING_2}-2`);
    await expect(manager.getHttpAuthToken(firstTarget))
      .resolves.toBe(`${PROGRAM_READ_BINDING_1}-3`);
    expect(getToken).toHaveBeenCalledTimes(3);
  });

  it('reuses and atomically upgrades the shared token scope cache', async () => {
    const getToken = vi.fn(async (request?: { scopes: readonly string[] }) => ({
      token: `token-${request?.scopes.join('+')}`,
      scopes: request?.scopes,
      expiresAt: Math.floor(Date.now() / 1000) + 300,
    }));
    const manager = new ConnectionManager({ websocketUrl: null, auth: { getToken } });

    await expect(manager.getHttpAuthToken(['read'])).resolves.toBe('token-read');
    await expect(manager.getHttpAuthToken(['read'])).resolves.toBe('token-read');
    expect(getToken).toHaveBeenCalledTimes(1);

    const [inspect, send] = await Promise.all([
      manager.getHttpAuthToken(['transaction:inspect']),
      manager.getHttpAuthToken(['transaction:send']),
    ]);
    expect(inspect).toBe(send);
    expect(getToken).toHaveBeenCalledTimes(2);
    expect(getToken.mock.calls[1]?.[0]?.scopes).toEqual([
      'read', 'transaction:inspect', 'transaction:send',
    ]);
  });

  it('shares an unauthenticated local token lookup without requiring scopes', async () => {
    const manager = new ConnectionManager({ websocketUrl: null });

    await expect(Promise.all([
      manager.getHttpAuthToken(['read']),
      manager.getHttpAuthToken(['transaction:inspect']),
      manager.getHttpAuthToken(['transaction:send']),
    ])).resolves.toEqual([undefined, undefined, undefined]);
  });

  it('handles refresh_auth success responses as control messages', async () => {
    const nowSeconds = Math.floor(Date.now() / 1000);
    const manager = new ConnectionManager({
      websocketUrl: 'wss://refresh.stack.arete.run',
      auth: {
        token: makeJwt(nowSeconds + 300),
      },
    });

    const states: string[] = [];
    manager.onStateChange((state) => {
      states.push(state);
    });

    const frameHandler = vi.fn();
    manager.onFrame(frameHandler);

    await manager.connect();

    const ws = MockWebSocket.instances[0]!;
    await ws.onmessage?.({
      data: JSON.stringify({
        success: true,
        expires_at: nowSeconds + 600,
      }),
    });

    expect(frameHandler).not.toHaveBeenCalled();
    expect(states.at(-1)).toBe('connected');
  });

  it('emits socket issues from server error control messages', async () => {
    const nowSeconds = Math.floor(Date.now() / 1000);
    const manager = new ConnectionManager({
      websocketUrl: 'wss://limits.stack.arete.run',
      auth: {
        token: makeJwt(nowSeconds + 300),
      },
    });

    const issueHandler = vi.fn();
    const frameHandler = vi.fn();
    manager.onSocketIssue(issueHandler);
    manager.onFrame(frameHandler);

    await manager.connect();

    const ws = MockWebSocket.instances[0]!;
    await ws.onmessage?.({
      data: JSON.stringify({
        type: 'error',
        error: 'subscription-limit-exceeded',
        message: 'Subscription limit exceeded',
        code: 'subscription-limit-exceeded',
        retryable: false,
        suggested_action: 'Unsubscribe first',
        fatal: false,
      }),
    });

    expect(frameHandler).not.toHaveBeenCalled();
    expect(issueHandler).toHaveBeenCalledWith({
      error: 'subscription-limit-exceeded',
      message: 'Subscription limit exceeded',
      code: 'SUBSCRIPTION_LIMIT_EXCEEDED',
      retryable: false,
      retryAfter: undefined,
      suggestedAction: 'Unsubscribe first',
      docsUrl: undefined,
      fatal: false,
    });
  });

  it('supports bearer-token websocket transport via a custom factory', async () => {
    const socketFactory = vi.fn((url: string, init?: { headers?: Record<string, string> }) => {
      return new FactoryWebSocket(url, init) as unknown as WebSocket;
    });

    const manager = new ConnectionManager({
      websocketUrl: 'wss://private.stack.arete.run',
      auth: {
        token: 'server-side-token',
        tokenTransport: 'bearer',
        websocketFactory: socketFactory,
      },
    });

    await manager.connect();

    expect(socketFactory).toHaveBeenCalledWith('wss://private.stack.arete.run', {
      headers: {
        Authorization: 'Bearer server-side-token',
      },
    });
    expect(MockWebSocket.instances[0]?.url).toBe('wss://private.stack.arete.run');
  });

  it('accepts a null websocketUrl but refuses to connect or subscribe', async () => {
    const manager = new ConnectionManager({ websocketUrl: null });

    await expect(manager.connect()).rejects.toMatchObject({ code: 'WEBSOCKET_DISABLED' });
    expect(() => manager.subscribe(subscription('things:all', { view: 'Thing/list' }))).toThrowError(
      expect.objectContaining({ code: 'WEBSOCKET_DISABLED' })
    );
    expect(manager.isConnected()).toBe(false);
  });

  it('preserves the full v2 query and stable ID when reconnecting', async () => {
    const manager = new ConnectionManager({ websocketUrl: 'ws://localhost:8878' });
    const active = subscription('rounds:page-2', {
      view: 'OreRound/latest',
      take: 1,
      skip: 2,
    }, false);

    await manager.connect();
    manager.subscribe(active);

    expect(JSON.parse(MockWebSocket.instances[0]!.sent[0]!)).toEqual(active);

    manager.disconnect();
    await manager.connect();

    expect(JSON.parse(MockWebSocket.instances[1]!.sent[0]!)).toEqual(active);
  });

  it('reports an established socket error as reconnecting rather than terminal', async () => {
    const manager = new ConnectionManager({ websocketUrl: 'ws://localhost:8878' });
    const states: string[] = [];
    manager.onStateChange((state) => { states.push(state); });

    await manager.connect();
    MockWebSocket.instances[0]!.onerror?.();

    expect(states.at(-1)).toBe('reconnecting');
    manager.disconnect();
  });

  it('makes an unexpected close terminal when automatic reconnection is disabled', async () => {
    const manager = new ConnectionManager({
      websocketUrl: 'ws://localhost:8878',
      autoReconnect: false,
      reconnectIntervals: [0],
    });

    await manager.connect();
    MockWebSocket.instances[0]!.close(1006, 'network lost');

    expect(manager.getState()).toBe('error');
    expect(MockWebSocket.instances).toHaveLength(1);

    await manager.connect();
    expect(manager.getState()).toBe('connected');
    expect(MockWebSocket.instances).toHaveLength(2);
    manager.disconnect();
  });

  it('orders refresh unsubscribe before resubscribe with the same opaque ID', async () => {
    const manager = new ConnectionManager({ websocketUrl: 'ws://localhost:8878' });
    const active = subscription('miner:wallet', {
      view: 'OreMiner/state',
      key: 'wallet',
      filters: { status: 'open', owner: 'wallet' },
      after: '20:1',
      snapshotLimit: 1,
    });

    await manager.connect();
    manager.subscribe(active);
    const ws = MockWebSocket.instances[0]!;
    ws.sent = [];

    manager.refresh(active);

    expect(ws.sent.map((message) => JSON.parse(message))).toEqual([
      { type: 'unsubscribe', protocolVersion: 2, subscriptionId: 'miner:wallet' },
      active,
    ]);

    manager.disconnect();
    await manager.connect();
    expect(JSON.parse(MockWebSocket.instances[1]!.sent[0]!)).toEqual(active);
  });

  it('retains a refresh registration when the replacement send fails', async () => {
    const manager = new ConnectionManager({ websocketUrl: 'ws://localhost:8878' });
    const active = subscription('miner:wallet', {
      view: 'OreMiner/state',
      key: 'wallet',
    });

    await manager.connect();
    manager.subscribe(active);
    const ws = MockWebSocket.instances[0]!;
    ws.sent = [];
    const send = vi.spyOn(ws, 'send');
    send.mockImplementationOnce((data) => { ws.sent.push(data); });
    send.mockImplementationOnce((data) => {
      ws.sent.push(data);
      throw new Error('refresh send failed');
    });

    expect(() => manager.refresh(active)).toThrowError('refresh send failed');

    manager.disconnect();
    await manager.connect();
    expect(JSON.parse(MockWebSocket.instances[1]!.sent[0]!)).toEqual(active);
  });

  it('rejects reuse of an active opaque ID for a different query', async () => {
    const manager = new ConnectionManager({ websocketUrl: 'ws://localhost:8878' });

    await manager.connect();
    manager.subscribe(subscription('positions:window', { view: 'Position/list', take: 1 }));

    expect(() =>
      manager.subscribe(subscription('positions:window', { view: 'Position/list', take: 2 }))
    ).toThrowError(/already registered/);
    expect(MockWebSocket.instances[0]!.sent).toHaveLength(1);
  });

  it('rejects refresh for an inactive wire identity without sending messages', async () => {
    const manager = new ConnectionManager({ websocketUrl: 'ws://localhost:8878' });

    await manager.connect();

    expect(() =>
      manager.refresh(subscription('miner:wallet', { view: 'OreMiner/state', key: 'wallet' }))
    ).toThrowError(/Cannot refresh inactive subscription/);
    expect(MockWebSocket.instances[0]!.sent).toHaveLength(0);
  });

  it('compensates for a subscribe send that throws after reaching the socket', async () => {
    const manager = new ConnectionManager({ websocketUrl: 'ws://localhost:8878' });
    const registry = new SubscriptionRegistry(manager, new QueryStore(new MemoryAdapter()));

    await manager.connect();
    const ws = MockWebSocket.instances[0]!;
    vi.spyOn(ws, 'send').mockImplementationOnce((data: string) => {
      ws.sent.push(data);
      throw new Error('socket send failed');
    });

    expect(() =>
      registry.subscribe({ view: 'OreMiner/state', key: 'wallet' })
    ).toThrowError(/socket send failed/);
    expect(ws.sent.map((message) => JSON.parse(message))).toEqual([
      expect.objectContaining({
        type: 'subscribe',
        protocolVersion: 2,
        query: { view: 'OreMiner/state', key: 'wallet' },
      }),
      expect.objectContaining({
        type: 'unsubscribe',
        protocolVersion: 2,
      }),
    ]);
    expect(registry.getRefCount({ view: 'OreMiner/state', key: 'wallet' })).toBe(0);
    expect(registry.getActiveSubscriptions()).toEqual([]);
  });

  it('does not send unsubscribe for an inactive subscription ID', async () => {
    const manager = new ConnectionManager({ websocketUrl: 'ws://localhost:8878' });

    await manager.connect();
    manager.unsubscribe('miner:not-active');

    expect(MockWebSocket.instances[0]!.sent).toHaveLength(0);
  });

  it('removes queued subscriptions before they connect', async () => {
    const manager = new ConnectionManager({ websocketUrl: 'ws://localhost:8878' });

    manager.subscribe(subscription('miner:wallet', { view: 'OreMiner/state', key: 'wallet' }));
    manager.unsubscribe('miner:wallet');
    await manager.connect();

    expect(MockWebSocket.instances[0]!.sent).toHaveLength(0);
  });

  it('ignores callbacks and frames from a stale socket generation', async () => {
    vi.useFakeTimers();
    const manager = new ConnectionManager({
      websocketUrl: 'ws://localhost:8878',
      reconnectIntervals: [0],
    });
    const frameHandler = vi.fn();
    manager.onFrame(frameHandler);

    const firstConnect = manager.connect();
    await vi.runAllTicks();
    await firstConnect;
    const oldSocket = MockWebSocket.instances[0]!;
    const staleMessageHandler = oldSocket.onmessage!;
    oldSocket.close(1006, 'network lost');
    await vi.advanceTimersByTimeAsync(0);
    await vi.runAllTicks();
    expect(MockWebSocket.instances).toHaveLength(2);

    await staleMessageHandler({
      data: JSON.stringify({
        protocolVersion: 2,
        subscriptionId: 'things:all',
        mode: 'list',
        entity: 'Thing/list',
        op: 'upsert',
        key: 'stale',
        data: { id: 'stale' },
      }),
    });

    expect(frameHandler).not.toHaveBeenCalled();
  });

  it('continues reconnecting when socket construction fails before onclose', async () => {
    vi.useFakeTimers();
    let attempts = 0;
    function IntermittentWebSocket(url: string): MockWebSocket {
      attempts++;
      if (attempts === 2) throw new Error('constructor unavailable');
      return new MockWebSocket(url);
    }
    Object.assign(IntermittentWebSocket, {
      CONNECTING: MockWebSocket.CONNECTING,
      OPEN: MockWebSocket.OPEN,
      CLOSING: MockWebSocket.CLOSING,
      CLOSED: MockWebSocket.CLOSED,
    });
    vi.stubGlobal('WebSocket', IntermittentWebSocket as unknown as typeof WebSocket);
    const manager = new ConnectionManager({
      websocketUrl: 'ws://localhost:8878',
      reconnectIntervals: [0],
    });
    const states: string[] = [];
    manager.onStateChange((state) => { states.push(state); });

    const firstConnect = manager.connect();
    await vi.runAllTicks();
    await firstConnect;
    MockWebSocket.instances[0]!.close(1006, 'network lost');

    await vi.advanceTimersToNextTimerAsync();
    await vi.runAllTicks();
    await vi.advanceTimersToNextTimerAsync();
    await vi.runAllTicks();

    expect(attempts).toBe(3);
    expect(manager.getState()).toBe('connected');
    expect(states).not.toContain('error');
  });

  it('rejects an in-flight connection when disconnected before open', async () => {
    const socket = {
      readyState: MockWebSocket.CONNECTING,
      onopen: null as (() => void) | null,
      onmessage: null,
      onerror: null,
      onclose: null as ((event: { code: number; reason: string }) => void) | null,
      send: vi.fn(),
      close() {
        this.readyState = MockWebSocket.CLOSED;
        this.onclose?.({ code: 1000, reason: '' });
      },
    };
    const manager = new ConnectionManager({
      websocketUrl: 'ws://localhost:8878',
      auth: { websocketFactory: () => socket as unknown as WebSocket },
    });

    const connecting = manager.connect();
    await Promise.resolve();
    manager.disconnect();

    await expect(connecting).rejects.toMatchObject({ code: 'CONNECTION_CANCELLED' });
    expect(manager.getState()).toBe('disconnected');
  });

  it('rejects an in-flight connection when disconnected during authentication', async () => {
    const getToken = vi
      .fn<[], Promise<string>>()
      .mockImplementationOnce(() => new Promise<string>(() => undefined))
      .mockResolvedValue('fresh-token');
    const manager = new ConnectionManager({
      websocketUrl: 'ws://localhost:8878',
      auth: { getToken },
    });

    const connecting = manager.connect();
    await Promise.resolve();
    expect(getToken).toHaveBeenCalledOnce();
    manager.disconnect();

    await expect(connecting).rejects.toMatchObject({ code: 'CONNECTION_CANCELLED' });
    expect(MockWebSocket.instances).toHaveLength(0);
    expect(manager.getState()).toBe('disconnected');

    await manager.connect();
    expect(getToken).toHaveBeenCalledTimes(2);
    expect(MockWebSocket.instances).toHaveLength(1);
    expect(manager.getState()).toBe('connected');
  });

  it('allows a connecting state handler to cancel the connection synchronously', async () => {
    const manager = new ConnectionManager({ websocketUrl: 'ws://localhost:8878' });
    manager.onStateChange((state) => {
      if (state === 'connecting') manager.disconnect();
    });

    await expect(manager.connect()).rejects.toMatchObject({ code: 'CONNECTION_CANCELLED' });
    expect(MockWebSocket.instances).toHaveLength(0);
    expect(manager.getState()).toBe('disconnected');
  });

  it('allows a disconnected state handler to reconnect synchronously', async () => {
    const manager = new ConnectionManager({ websocketUrl: 'ws://localhost:8878' });
    await manager.connect();
    let reconnecting: Promise<void> | undefined;
    manager.onStateChange((state) => {
      if (state === 'disconnected') reconnecting = manager.connect();
    });

    manager.disconnect();
    await reconnecting;

    expect(MockWebSocket.instances).toHaveLength(2);
    expect(manager.getState()).toBe('connected');
  });

  it('treats the legacy disabled sentinel URL as a null websocketUrl', async () => {
    const manager = new ConnectionManager({ websocketUrl: 'ws://127.0.0.1/__arete_disabled__' });

    await expect(manager.connect()).rejects.toMatchObject({ code: 'WEBSOCKET_DISABLED' });
    expect(MockWebSocket.instances).toHaveLength(0);
  });
});
