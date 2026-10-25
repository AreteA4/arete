/**
 * `AreteProvider` with an API credential (`auth.tokenKind: 'api'`), as a
 * dashboard signed in to Arete uses it: the credential is fetched from the
 * app (getToken or its own token route), sent only to the Arete API's
 * session endpoint and account routes, and the stack socket only ever sees
 * the session minted with it.
 */

import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import type { AuthConfig, PlatformHttpClient } from '@usearete/sdk';

import { AreteProvider } from './provider';
import { usePlatformHttpClient } from './platform-http-hooks';
import { useArete } from './stack';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const originalConsoleError = console.error;
beforeAll(() => {
  jest.spyOn(console, 'error').mockImplementation((message: unknown, ...args: unknown[]) => {
    if (typeof message === 'string' && message.startsWith('react-test-renderer is deprecated')) return;
    originalConsoleError(message, ...args);
  });
});
afterAll(() => {
  jest.restoreAllMocks();
});

const CREDENTIAL = 'dashboard-login-credential';
const SESSION = 'minted-stack-session';

const stack = {
  name: 'demo-stream',
  endpoints: { ws: 'wss://demo.stack.arete.run', http: 'https://demo.stack.arete.run' },
  views: {},
} as const;

class FakeSocket {
  static instances: FakeSocket[] = [];
  readyState = 0;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: ((event: { code: number; reason: string }) => void) | null = null;

  constructor(
    readonly url: string,
    readonly init?: { headers?: Record<string, string> }
  ) {
    FakeSocket.instances.push(this);
    queueMicrotask(() => {
      this.readyState = 1;
      this.onopen?.();
    });
  }

  send(): void {}

  close(code = 1000, reason = ''): void {
    this.readyState = 3;
    this.onclose?.({ code, reason });
  }
}

interface Recorded {
  url: string;
  authorization: string | null;
  body: unknown;
}

function recordingFetch(requests: Recorded[]): typeof fetch {
  return (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const headers = new Headers(init?.headers);
    requests.push({
      url,
      authorization: headers.get('Authorization'),
      body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined,
    });
    const json = (value: unknown) => new Response(JSON.stringify(value), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
    if (url === '/api/account/api-token') return json({ token: CREDENTIAL });
    if (url.endsWith('/ws/sessions')) {
      return json({ token: SESSION, expires_at: Math.floor(Date.now() / 1000) + 3600 });
    }
    if (url.endsWith('/api/account/squads')) return json({ squads: [] });
    return new Response('not found', { status: 404 });
  }) as typeof fetch;
}

async function settle() {
  await act(async () => {
    for (let i = 0; i < 20; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

describe('AreteProvider with tokenKind api', () => {
  beforeEach(() => {
    FakeSocket.instances = [];
  });

  it.each([
    ['getToken', (getToken: jest.Mock): AuthConfig => ({ tokenKind: 'api', getToken })],
    ['tokenEndpoint', (): AuthConfig => ({
      tokenKind: 'api',
      tokenEndpoint: '/api/account/api-token',
      tokenEndpointCredentials: 'same-origin',
    })],
  ])('connects a hosted stack with a session minted from the credential (%s)', async (_name, authFor) => {
    const requests: Recorded[] = [];
    const getToken = jest.fn(async () => CREDENTIAL);
    const auth: AuthConfig = {
      ...authFor(getToken),
      websocketFactory: (url, init) => new FakeSocket(url, init) as unknown as WebSocket,
    };
    let connection: string | undefined;
    function Consumer() {
      connection = useArete(stack).connectionState;
      return null;
    }
    let renderer!: ReactTestRenderer;
    act(() => {
      renderer = create(React.createElement(
        AreteProvider,
        { auth, fetch: recordingFetch(requests), children: React.createElement(Consumer) }
      ));
    });
    await settle();

    const mint = requests.find((request) => request.url.endsWith('/ws/sessions'));
    expect(mint?.authorization).toBe(`Bearer ${CREDENTIAL}`);
    expect(FakeSocket.instances).toHaveLength(1);
    const socket = FakeSocket.instances[0];
    const sentToSocket = `${socket.url} ${JSON.stringify(socket.init ?? {})}`;
    expect(sentToSocket).toContain(SESSION);
    // The API credential never reaches the stack.
    expect(sentToSocket).not.toContain(CREDENTIAL);
    expect(connection).toBe('connected');
    if (_name === 'getToken') {
      expect(getToken).toHaveBeenCalledWith({ scopes: [], forceRefresh: false });
    } else {
      expect(requests[0]).toMatchObject({ url: '/api/account/api-token' });
    }

    act(() => renderer.unmount());
  });

  it('hands components an Arete API client authenticated with the same login', async () => {
    const requests: Recorded[] = [];
    const auth: AuthConfig = { tokenKind: 'api', tokenEndpoint: '/api/account/api-token' };
    let http!: PlatformHttpClient;
    const seen = new Set<PlatformHttpClient>();
    function Consumer() {
      http = usePlatformHttpClient({ apiUrl: 'https://api.example.test' });
      seen.add(http);
      return null;
    }
    let renderer!: ReactTestRenderer;
    const element = () => React.createElement(
      AreteProvider,
      { auth, fetch: fetchImpl, children: React.createElement(Consumer) }
    );
    const fetchImpl = recordingFetch(requests);
    act(() => {
      renderer = create(element());
    });
    act(() => {
      renderer.update(element());
    });
    expect(seen.size).toBe(1);

    const response = await http.request<{ squads: unknown[] }>({ path: '/account/squads' });

    expect(response.body).toEqual({ squads: [] });
    expect(http.apiUrl).toBe('https://api.example.test/api');
    expect(requests.map((request) => [request.url, request.authorization])).toEqual([
      ['/api/account/api-token', null],
      ['https://api.example.test/api/account/squads', `Bearer ${CREDENTIAL}`],
    ]);
    act(() => renderer.unmount());
  });
});
