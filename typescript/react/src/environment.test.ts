/**
 * `<AreteProvider environment>`: every client the provider creates goes to
 * the dev edge with no credential, and the badge says so.
 */

import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';

import { AreteEnvironmentBadge, useAreteEnvironment } from './environment';
import { AreteProvider } from './provider';
import { useArete } from './stack';
import { createWebSocketHarness, type WebSocketHarness } from './testing';

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

const EDGE = 'http://127.0.0.1:8911';
const stack = {
  name: 'demo-stream',
  endpoints: { ws: 'wss://demo.stack.arete.run', http: 'https://demo.stack.arete.run' },
  release: {
    stackManifestHash: 'arete:h1:stack-manifest:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
    liveAlias: 'default',
  },
  views: {},
} as const;

function edgeFetch(urls: string[], authorizations: (string | null)[]): typeof fetch {
  return (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    urls.push(url);
    authorizations.push(new Headers(init?.headers).get('Authorization'));
    return new Response(JSON.stringify({ edge: 1 }), {
      status: 200,
      headers: { 'Content-Type': 'application/json', 'X-Arete-Dev-Edge': '1' },
    });
  }) as typeof fetch;
}

async function settle() {
  await act(async () => {
    for (let i = 0; i < 20; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

describe('AreteProvider environment', () => {
  let ws: WebSocketHarness;
  beforeEach(() => {
    ws = createWebSocketHarness();
  });
  afterEach(() => {
    ws.restore();
  });

  it('streams through the edge, without a credential, and shows the badge', async () => {
    const urls: string[] = [];
    const authorizations: (string | null)[] = [];
    let environment: string | undefined;
    function Probe() {
      useArete(stack);
      environment = useAreteEnvironment(stack);
      return React.createElement(AreteEnvironmentBadge, { stack });
    }
    let renderer!: ReactTestRenderer;
    await act(async () => {
      renderer = create(
        React.createElement(
          AreteProvider,
          {
            environment: { kind: 'dev', url: EDGE },
            fetch: edgeFetch(urls, authorizations),
            auth: { websocketFactory: ws.websocketFactory },
          },
          React.createElement(Probe)
        )
      );
    });
    await settle();

    expect(urls).toEqual([`${EDGE}/.well-known/arete-dev-edge`]);
    expect(authorizations).toEqual([null]);
    expect(ws.sockets.map((socket) => socket.url)).toEqual([
      `ws://127.0.0.1:8911/stacks/default?stackManifestHash=${stack.release.stackManifestHash}`,
    ]);
    expect(environment).toBe('dev');
    expect(renderer.root.findByProps({ 'data-arete-environment': 'dev' }).props.children)
      .toBe('Dev environment');
    await act(async () => renderer.unmount());
  });

  it('renders no badge in production', async () => {
    let renderer!: ReactTestRenderer;
    await act(async () => {
      renderer = create(
        React.createElement(
          AreteProvider,
          { environment: 'production', autoConnect: false },
          React.createElement(AreteEnvironmentBadge)
        )
      );
    });
    expect(renderer.toJSON()).toBeNull();
    await act(async () => renderer.unmount());
  });
});
