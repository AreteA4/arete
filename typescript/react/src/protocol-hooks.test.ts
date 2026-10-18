import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';

import {
  useAllowedSteps,
  useProtocolEvents,
  useProtocolInstance,
  useProtocolParticipant,
  useWalletActions,
} from './protocol-hooks';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const originalConsoleError = console.error;
beforeAll(() => {
  jest.spyOn(console, 'error').mockImplementation((message: unknown, ...args: unknown[]) => {
    if (typeof message === 'string' && message.startsWith('react-test-renderer is deprecated')) return;
    originalConsoleError(message, ...args);
  });
});
afterAll(() => jest.restoreAllMocks());

const WALLET = 'So11111111111111111111111111111111111111112';

function fakeHandle(id: string) {
  const listeners = new Set<() => void>();
  let snapshot: { instance: { id: string; state: string }; live: unknown } | undefined;
  let error: Error | undefined;
  const handle = {
    instanceId: id,
    get: () => snapshot,
    getError: () => error,
    subscribe: jest.fn((listener: () => void) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    }),
    refresh: jest.fn(async () => snapshot),
    close: jest.fn(() => listeners.clear()),
    listeners,
    set(state: string, live?: unknown) {
      snapshot = { instance: { id, state }, live };
      for (const listener of [...listeners]) listener();
    },
    fail(next: Error) {
      error = next;
      for (const listener of [...listeners]) listener();
    },
  };
  return handle;
}

function fakeClient() {
  const handles: ReturnType<typeof fakeHandle>[] = [];
  const streams: { instanceId: string; options: { onEvent(event: unknown): void; lastEventId?: string }; close: jest.Mock }[] = [];
  const client = {
    instances: {
      use: jest.fn((id: string) => {
        const handle = fakeHandle(id);
        handles.push(handle);
        return handle;
      }),
    },
    events: jest.fn((instanceId: string, options: { onEvent(event: unknown): void; lastEventId?: string }) => {
      const stream = { instanceId, options, close: jest.fn() };
      streams.push(stream);
      return { close: stream.close, lastEventId: undefined, closed: false };
    }),
    allowedSteps: jest.fn(async () => [{ step: 'contribute', allowed: true }]),
    participants: {
      get: jest.fn(async () => ({ instanceId: 'inst-1', wallet: WALLET, state: 'contributed' })),
    },
    actions: jest.fn(async (input: { wallet: string; cursor?: string }) => ({
      deploymentId: 'dep-1',
      wallet: input.wallet,
      createSteps: [{ step: 'create', allowed: true, scope: 'create', args: [] }],
      instances: [{
        instanceId: input.cursor ? 'inst-2' : 'inst-1',
        state: 'open',
        relation: ['participant'],
        participantState: 'contributed',
        steps: [{ step: 'contribute', allowed: true, scope: 'participant' }],
      }],
      nextCursor: input.cursor ? null : 'c2',
    })),
  };
  return { client, handles, streams };
}

async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe('useProtocolInstance', () => {
  it('follows the instance handle and closes it on unmount', async () => {
    const { client, handles } = fakeClient();
    const seen: unknown[] = [];
    function Probe({ id }: { id: string }) {
      const result = useProtocolInstance(client as never, id, { wallet: WALLET });
      seen.push({ state: result.instance?.state, live: result.live, loading: result.isLoading, error: result.error?.message });
      return null;
    }

    let renderer: ReactTestRenderer | undefined;
    await act(async () => {
      renderer = create(React.createElement(Probe, { id: 'inst-1' }));
    });
    expect(client.instances.use).toHaveBeenCalledWith('inst-1', { wallet: WALLET });
    expect(seen[seen.length - 1]).toEqual({ state: undefined, live: undefined, loading: true, error: undefined });

    act(() => handles[0]!.set('open', { raised: '1' }));
    expect(seen[seen.length - 1]).toEqual({ state: 'open', live: { raised: '1' }, loading: false, error: undefined });

    act(() => handles[0]!.fail(new Error('stream down')));
    expect(seen[seen.length - 1]).toMatchObject({ state: 'open', error: 'stream down' });

    await act(async () => {
      renderer!.update(React.createElement(Probe, { id: 'inst-2' }));
    });
    expect(handles[0]!.close).toHaveBeenCalled();
    expect(handles[1]!.instanceId).toBe('inst-2');

    act(() => renderer!.unmount());
    expect(handles[1]!.close).toHaveBeenCalled();
    expect(handles[1]!.listeners.size).toBe(0);
  });

  it('stays idle without a client or instance', async () => {
    let result: ReturnType<typeof useProtocolInstance> | undefined;
    function Probe() {
      result = useProtocolInstance(null, 'inst-1');
      return null;
    }
    await act(async () => {
      create(React.createElement(Probe));
    });
    expect(result).toMatchObject({ instance: undefined, isLoading: false });
  });
});

describe('protocol read hooks', () => {
  it('reads allowed steps and refreshes them on stream events', async () => {
    const { client, streams } = fakeClient();
    let result: ReturnType<typeof useAllowedSteps> | undefined;
    function Probe() {
      result = useAllowedSteps(client as never, { instance: 'inst-1', wallet: WALLET }, { refreshOnEvents: true });
      return null;
    }
    let renderer: ReactTestRenderer | undefined;
    await act(async () => {
      renderer = create(React.createElement(Probe));
    });
    await flush();
    expect(result!.data).toEqual([{ step: 'contribute', allowed: true }]);
    expect(client.allowedSteps).toHaveBeenCalledWith(expect.objectContaining({ instance: 'inst-1', wallet: WALLET }));
    expect(streams).toHaveLength(1);

    await act(async () => {
      streams[0]!.options.onEvent({ type: 'transition', id: '1', data: {} });
    });
    await flush();
    expect(client.allowedSteps).toHaveBeenCalledTimes(2);

    act(() => renderer!.unmount());
    expect(streams[0]!.close).toHaveBeenCalled();
  });

  it('reads a participant without opening a stream by default', async () => {
    const { client, streams } = fakeClient();
    let result: ReturnType<typeof useProtocolParticipant> | undefined;
    function Probe() {
      result = useProtocolParticipant(client as never, { instance: 'inst-1', wallet: WALLET });
      return null;
    }
    await act(async () => {
      create(React.createElement(Probe));
    });
    await flush();
    expect(result!.data).toMatchObject({ state: 'contributed' });
    expect(streams).toHaveLength(0);
  });

  it('is disabled until the wallet is known', async () => {
    const { client } = fakeClient();
    let result: ReturnType<typeof useAllowedSteps> | undefined;
    function Probe() {
      result = useAllowedSteps(client as never, { instance: 'inst-1', wallet: null });
      return null;
    }
    await act(async () => {
      create(React.createElement(Probe));
    });
    expect(result!.status).toBe('disabled');
    expect(client.allowedSteps).not.toHaveBeenCalled();
  });
});

describe('useWalletActions', () => {
  it('reads one page and re-reads when the cursor changes', async () => {
    const { client, streams } = fakeClient();
    let result: ReturnType<typeof useWalletActions> | undefined;
    function Probe({ cursor }: { cursor?: string | null }) {
      result = useWalletActions(client as never, { wallet: WALLET, cursor, limit: 10 });
      return null;
    }
    let renderer: ReactTestRenderer | undefined;
    await act(async () => {
      renderer = create(React.createElement(Probe, { cursor: null }));
    });
    await flush();
    expect(result!.data).toMatchObject({
      createSteps: [{ step: 'create', allowed: true }],
      instances: [{ instanceId: 'inst-1' }],
      nextCursor: 'c2',
    });
    expect(client.actions).toHaveBeenCalledWith(expect.objectContaining({
      wallet: WALLET,
      instance: undefined,
      cursor: undefined,
      limit: 10,
    }));
    // No instance: no stream to follow.
    expect(streams).toHaveLength(0);

    await act(async () => {
      renderer!.update(React.createElement(Probe, { cursor: 'c2' }));
    });
    await flush();
    expect(client.actions).toHaveBeenLastCalledWith(expect.objectContaining({ cursor: 'c2' }));
    expect(result!.data).toMatchObject({ instances: [{ instanceId: 'inst-2' }], nextCursor: null });
  });

  it('refreshes on the instance stream when scoped to one instance', async () => {
    const { client, streams } = fakeClient();
    function Probe() {
      useWalletActions(client as never, { wallet: WALLET, instance: 'inst-1' }, { refreshOnEvents: true });
      return null;
    }
    let renderer: ReactTestRenderer | undefined;
    await act(async () => {
      renderer = create(React.createElement(Probe));
    });
    await flush();
    expect(client.actions).toHaveBeenCalledWith(expect.objectContaining({ instance: 'inst-1' }));
    expect(streams).toHaveLength(1);
    await act(async () => {
      streams[0]!.options.onEvent({ type: 'transition', id: '1', data: {} });
    });
    await flush();
    expect(client.actions).toHaveBeenCalledTimes(2);
    act(() => renderer!.unmount());
    expect(streams[0]!.close).toHaveBeenCalled();
  });

  it('stays idle until the wallet is known and surfaces read errors', async () => {
    const { client } = fakeClient();
    let result: ReturnType<typeof useWalletActions> | undefined;
    function Probe({ wallet }: { wallet?: string }) {
      result = useWalletActions(client as never, { wallet });
      return null;
    }
    let renderer: ReactTestRenderer | undefined;
    await act(async () => {
      renderer = create(React.createElement(Probe, {}));
    });
    expect(result!.status).toBe('disabled');
    expect(client.actions).not.toHaveBeenCalled();

    client.actions.mockRejectedValueOnce(new Error('session expired'));
    await act(async () => {
      renderer!.update(React.createElement(Probe, { wallet: WALLET }));
    });
    await flush();
    expect(result!.error?.message).toBe('session expired');
    expect(result!.status).toBe('error');
  });
});

describe('useProtocolEvents', () => {
  it('uses the latest callback and reopens only when the instance changes', async () => {
    const { client, streams } = fakeClient();
    const received: string[] = [];
    function Probe({ id, tag }: { id: string; tag: string }) {
      useProtocolEvents(client as never, id, {
        lastEventId: '5',
        onEvent: (event) => received.push(`${tag}:${event.id}`),
      });
      return null;
    }
    let renderer: ReactTestRenderer | undefined;
    await act(async () => {
      renderer = create(React.createElement(Probe, { id: 'inst-1', tag: 'a' }));
    });
    expect(streams).toHaveLength(1);
    expect(streams[0]!.options.lastEventId).toBe('5');

    await act(async () => {
      renderer!.update(React.createElement(Probe, { id: 'inst-1', tag: 'b' }));
    });
    expect(streams).toHaveLength(1);
    streams[0]!.options.onEvent({ type: 'transition', id: '6', data: {} });
    expect(received).toEqual(['b:6']);

    await act(async () => {
      renderer!.update(React.createElement(Probe, { id: 'inst-2', tag: 'b' }));
    });
    expect(streams[0]!.close).toHaveBeenCalled();
    expect(streams[1]!.instanceId).toBe('inst-2');
    act(() => renderer!.unmount());
    expect(streams[1]!.close).toHaveBeenCalled();
  });
});
