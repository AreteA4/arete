import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ConnectionManager } from './connection';
import { AreteError } from './types';

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

describe('ConnectionManager auth', () => {
  beforeEach(() => {
    MockWebSocket.instances = [];
    vi.stubGlobal('WebSocket', MockWebSocket as unknown as typeof WebSocket);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('fails clearly when hosted auth metadata is missing', async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);

    const manager = new ConnectionManager({
      websocketUrl: 'wss://demo.stack.arete.run',
    });

    await expect(manager.connect()).rejects.toMatchObject<Partial<AreteError>>({
      code: 'AUTH_REQUIRED',
    });
    expect(fetchMock).not.toHaveBeenCalled();
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
});
