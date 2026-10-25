/**
 * `useArete(member)` over real sessions: `createSession` connects each stack
 * member to the WebSocket harness, and the hook must read that member's
 * connection and store without connecting again, never disconnect it, render
 * placeholders while the session opens, and move to the new member when the
 * session reopens.
 */

import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { createSession, type AuthConfig, type Session, type WalletAdapter } from '@usearete/sdk';

import * as programHooks from './program-hooks';
import { AreteProvider } from './provider';
import { AreteSessionProvider, useAreteSession, type AreteSessionProviderProps } from './session-provider';
import { useArete, type UseAreteResult } from './stack';
import { createAreteTestConfig, createWebSocketHarness, frames, type WebSocketHarness } from './testing';

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

const STACK = {
  name: 'things',
  endpoints: { ws: 'ws://arete.test', http: 'http://arete.test' },
  views: {
    Thing: { list: { mode: 'list', view: 'Thing/list' } },
  },
  programs: {
    things: { name: 'things', programId: 'Things1111111111111111111111111111111111111' },
  },
} as const;
const DEFINITION = { stacks: { things: STACK } };
type ThingsSession = Session<typeof DEFINITION>;
type ThingsResult = UseAreteResult<typeof STACK>;
type ThingsList = ReturnType<ThingsResult['views']['Thing']['list']['use']>;

function wallet(publicKey: string): WalletAdapter {
  return { publicKey, signAndSend: async () => ({ signature: `sig-${publicKey}` }) };
}

const optionsCache = new Map<string, unknown>();

/**
 * The provider's options for one auth, built once: a new options object of
 * the same shape (three levels deep with member options) would reopen the
 * session.
 */
function sessionOptions(ws: WebSocketHarness, auth: Partial<AuthConfig> = {}) {
  const key = JSON.stringify(auth);
  let options = optionsCache.get(key);
  if (!options) {
    options = {
      auth: { ...auth, websocketFactory: ws.websocketFactory },
      stacks: { things: { autoReconnect: false } },
    };
    optionsCache.set(key, options);
  }
  return options as {
    auth: AuthConfig;
    stacks: { things: { autoReconnect: boolean } };
  };
}

async function subscribed(ws: WebSocketHarness, view: string, socketCount = 1) {
  await until(() => ws.sockets.length === socketCount
    && ws.subscriptions().some((subscription) => subscription.query.view === view));
  return ws.subscription(view);
}

interface Seen {
  arete: ThingsResult;
  things: ThingsList;
  session: ThingsSession | null;
}

/** Reads the provider's session and its `things` member, as an app would. */
function Things({ seen }: { seen: Seen[] }) {
  const { session } = useAreteSession<typeof DEFINITION>();
  const arete = useArete(session?.stacks.things, { stack: STACK });
  const things = arete.views.Thing.list.use();
  seen.push({ arete, things, session });
  return null;
}

function render(element: React.ReactElement) {
  let renderer!: ReactTestRenderer;
  act(() => {
    renderer = create(element);
  });
  return {
    update(next: React.ReactElement) {
      act(() => {
        renderer.update(next);
      });
    },
    unmount() {
      act(() => {
        renderer.unmount();
      });
    },
  };
}

async function until(check: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100 && !check(); attempt += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
  }
  expect(check()).toBe(true);
}

function latest(seen: Seen[]): Seen {
  const last = seen[seen.length - 1];
  if (!last) throw new Error('Nothing rendered yet');
  return last;
}

describe('useArete(member)', () => {
  let ws: WebSocketHarness;
  beforeEach(() => {
    ws = createWebSocketHarness();
    optionsCache.clear();
  });
  afterEach(() => ws.restore());

  function provider(seen: Seen[], auth: Partial<AuthConfig> = {}, children?: React.ReactNode) {
    const props: AreteSessionProviderProps<typeof DEFINITION> = {
      definition: DEFINITION,
      options: sessionOptions(ws, auth) as AreteSessionProviderProps<typeof DEFINITION>['options'],
      children: children ?? React.createElement(Things, { seen }),
    };
    return React.createElement(AreteSessionProvider, props as AreteSessionProviderProps);
  }

  it('renders the connecting result while the session opens, then the member', async () => {
    const seen: Seen[] = [];
    const view = render(provider(seen));

    const opening = seen[0]!;
    expect(opening.session).toBeNull();
    expect(opening.arete.client).toBeNull();
    expect(opening.arete.status).toBe('connecting');
    expect(opening.arete.isLoading).toBe(true);
    expect(opening.arete.isConnected).toBe(false);
    expect(opening.arete.connectionState).toBe('disconnected');
    expect(opening.arete.error).toBeNull();
    expect(opening.arete.canRetry).toBe(false);
    // Placeholders come from the stack definition: hooks run unconditionally.
    expect(opening.things.status).toBe('connecting');
    expect(opening.things.data).toEqual([]);
    expect(opening.arete.programs.things.programId).toBe(STACK.programs.things.programId);

    await until(() => latest(seen).arete.status === 'connected');
    const open = latest(seen);
    expect(open.arete.client).toBe(open.session?.stacks.things);
    expect(open.arete.isLoading).toBe(false);
    expect(open.arete.isConnected).toBe(true);
    view.unmount();
  });

  it('reads views from the member it was given, over the member’s one connection', async () => {
    const seen: Seen[] = [];
    // An enclosing AreteProvider must not open a connection of its own.
    const view = render(React.createElement(
      AreteProvider,
      createAreteTestConfig(ws),
      provider(seen),
    ));

    const subscription = await subscribed(ws, 'Thing/list');
    expect(ws.sockets).toHaveLength(1);
    act(() => ws.serve(subscription, [{ key: 'a', data: { id: 'a', size: 1 } }]));
    await until(() => latest(seen).things.status === 'ready');
    expect(latest(seen).things.data).toEqual([{ id: 'a', size: 1 }]);

    // The rows are the member's own store, which the hook reads as it is.
    const member = latest(seen).session!.stacks.things;
    expect(member.store.getAll('Thing/list')).toEqual([{ id: 'a', size: 1 }]);

    act(() => ws.send(frames.patch(subscription, 'a', { size: 2 })));
    expect(latest(seen).things.data).toEqual([{ id: 'a', size: 2 }]);
    expect(ws.sockets).toHaveLength(1);
    expect(ws.subscriptions()).toHaveLength(1);
    view.unmount();
  });

  it('never disconnects the member when the component unmounts', async () => {
    const seen: Seen[] = [];
    function Toggle({ show }: { show: boolean }) {
      return show ? React.createElement(Things, { seen }) : null;
    }
    const view = render(provider(seen, {}, React.createElement(Toggle, { show: true })));
    await subscribed(ws, 'Thing/list');
    await until(() => latest(seen).arete.status === 'connected');
    const session = latest(seen).session!;
    const member = session.stacks.things;
    const disconnect = jest.spyOn(member, 'disconnect');

    view.update(provider(seen, {}, React.createElement(Toggle, { show: false })));
    expect(disconnect).not.toHaveBeenCalled();
    expect(member.connectionState).toBe('connected');
    expect(ws.latest().readyState).toBe(ws.latest().OPEN);

    // The session owns the member: closing the provider closes it.
    view.unmount();
    expect(disconnect).toHaveBeenCalledTimes(1);
  });

  it('moves to the new member when the session reopens, with nothing from the old one', async () => {
    const seen: Seen[] = [];
    const view = render(provider(seen, { publishableKey: 'pk-a' }));
    const firstSubscription = await subscribed(ws, 'Thing/list');
    act(() => ws.serve(firstSubscription, [{ key: 'a', data: { id: 'a', size: 1 } }]));
    await until(() => latest(seen).things.status === 'ready');
    const first = latest(seen).arete.client!;
    const firstSocket = ws.latest();

    view.update(provider(seen, { publishableKey: 'pk-b' }));
    expect(latest(seen).arete.client).toBeNull();
    expect(latest(seen).arete.status).toBe('connecting');
    expect(latest(seen).things.data).toEqual([]);
    expect(firstSocket.readyState).toBe(firstSocket.CLOSED);

    const secondSubscription = await subscribed(ws, 'Thing/list', 2);
    await until(() => latest(seen).arete.status === 'connected');
    const second = latest(seen).arete.client!;
    expect(second).not.toBe(first);
    expect(second).toBe(latest(seen).session?.stacks.things);
    // Subscribed again on the new member; the old rows are gone.
    expect(latest(seen).things.status).toBe('subscribing');
    expect(latest(seen).things.data).toEqual([]);

    act(() => ws.serve(secondSubscription, [{ key: 'b', data: { id: 'b', size: 5 } }]));
    await until(() => latest(seen).things.status === 'ready');
    expect(latest(seen).things.data).toEqual([{ id: 'b', size: 5 }]);
    expect(ws.sockets).toHaveLength(2);
    view.unmount();
  });

  it('takes a member without a provider and reads its stack from it', async () => {
    const session = await createSession(DEFINITION, {
      ...sessionOptions(ws),
      wallet: wallet('alice'),
    });
    let arete: ThingsResult | undefined;
    let things: ThingsList | undefined;
    await until(() => session.stacks.things.connectionState === 'connected');
    function Member() {
      arete = useArete(session.stacks.things);
      things = arete.views.Thing.list.use();
      return null;
    }
    const view = render(React.createElement(Member));
    try {
      const subscription = await subscribed(ws, 'Thing/list');
      act(() => ws.serve(subscription, [{ key: 'a', data: { id: 'a', size: 1 } }]));
      await until(() => things?.status === 'ready');
      expect(things?.data).toEqual([{ id: 'a', size: 1 }]);
      expect(arete?.client).toBe(session.stacks.things);
      expect(arete?.programs.things.programId).toBe(STACK.programs.things.programId);
      expect(ws.sockets).toHaveLength(1);

      // Mutations sign with the member's wallet, which is the session's.
      expect(session.stacks.things.wallet?.publicKey).toBe('alice');
      session.setWallet(wallet('bob'));
      expect(arete?.client?.wallet?.publicKey).toBe('bob');

      // The session closing the member under a mounted component shows as a
      // disconnected stack whose views have lost their subscription, and
      // renders settle (no update loop on the released lease).
      act(() => session.close());
      await until(() => arete?.status === 'disconnected');
      expect(arete?.isConnected).toBe(false);
      expect(things?.status).toBe('error');
    } finally {
      view.unmount();
      session.close();
    }
  });

  it('waits for the streamed slot only for a member its session connected over the socket', async () => {
    const buildPrograms = jest.spyOn(programHooks, 'buildProgramHookInterfaces');
    async function reconcilesOver(transport: 'ws' | 'http') {
      const session = await createSession(DEFINITION, { ...sessionOptions(ws), transport });
      const member = session.stacks.things;
      buildPrograms.mockClear();
      function Member() {
        useArete(member);
        return null;
      }
      const view = render(React.createElement(Member));
      try {
        expect(member.transport).toBe(transport);
        const calls = buildPrograms.mock.calls.filter(([, client]) => client === member);
        expect(calls.length).toBeGreaterThan(0);
        return calls[calls.length - 1]?.[3]?.defaultReconciliation;
      } finally {
        view.unmount();
        session.close();
      }
    }
    try {
      await expect(reconcilesOver('ws')).resolves.toBe(true);
      await expect(reconcilesOver('http')).resolves.toBe(false);
    } finally {
      buildPrograms.mockRestore();
    }
  });

  it('reports a session that failed to open, and retry reopens it', async () => {
    const seen: Seen[] = [];
    let attempts = 0;
    function failingFactory(url: string, init?: Parameters<WebSocketHarness['websocketFactory']>[1]) {
      attempts += 1;
      if (attempts === 1) throw new Error('socket refused');
      return ws.websocketFactory(url, init);
    }
    const props = {
      definition: DEFINITION,
      options: {
        auth: { websocketFactory: failingFactory },
        stacks: { things: { autoReconnect: false } },
      },
      children: React.createElement(Things, { seen }),
    } as unknown as AreteSessionProviderProps;
    const view = render(React.createElement(AreteSessionProvider, props));

    await until(() => latest(seen).arete.status === 'error');
    expect(latest(seen).arete.error).toBeInstanceOf(Error);
    expect(latest(seen).arete.canRetry).toBe(true);
    expect(latest(seen).arete.isLoading).toBe(false);

    await act(async () => {
      await latest(seen).arete.retry();
    });
    await until(() => latest(seen).arete.status === 'connected');
    expect(latest(seen).arete.error).toBeNull();
    expect(latest(seen).arete.canRetry).toBe(false);
    view.unmount();
  });
});
