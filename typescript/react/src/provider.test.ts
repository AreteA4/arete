import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import {
  Arete,
  FrameProcessor,
  MemoryAdapter,
  QueryStore,
  SubscriptionRegistry,
  parseFrame,
  type Subscription,
} from '@usearete/sdk';
import type { AreteConfig } from './types';

import { createClientCacheKey } from './client-key';
import { trackConnectingPromise } from './provider-cache';
import { AreteProvider } from './provider';
import { useArete } from './stack';
import { initializeConnectedClient, syncClientWallets } from './wallet-sync';

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

describe('provider wallet helpers', () => {
  const wallet = {
    publicKey: 'wallet-111',
    async signAndSend() {
      return { signature: 'sig' };
    },
  } satisfies NonNullable<AreteConfig['wallet']>;

  const stack = {
    name: 'ore-stream',
    endpoints: {
      ws: 'wss://ore.example',
      http: 'https://ore.example',
    },
    views: {},
  } as const;

  it('builds stable client cache keys for the same stack and options', () => {
    expect(createClientCacheKey(stack)).toBe(createClientCacheKey(stack));
    expect(
      createClientCacheKey(stack, { url: 'ws://localhost:7777', httpUrl: 'http://localhost:7777' })
    ).toBe(
      createClientCacheKey(stack, { url: 'ws://localhost:7777', httpUrl: 'http://localhost:7777' })
    );
  });

  it('keys attached programs by behavior while allowing fresh outer maps', () => {
    const generatedProgramA = { name: 'ore', programId: 'ore-program', definitionHash: 'ore-v1' };
    const generatedProgramB = { name: 'ore', programId: 'ore-program', definitionHash: 'ore-v1' };
    const changedProgram = { name: 'ore', programId: 'ore-program', definitionHash: 'ore-v2' };
    const anonymousProgramA = { definitionHash: 'anonymous-v1' };
    const anonymousProgramB = { definitionHash: 'anonymous-v1' };
    const manualProgram = { name: 'ore', rawInstructions: { mine: jest.fn() } };

    expect(createClientCacheKey(stack, { transport: 'http' })).not.toBe(createClientCacheKey(stack));
    expect(createClientCacheKey(stack, { programs: { ore: generatedProgramA } as never })).toBe(
      createClientCacheKey(stack, { programs: { ore: generatedProgramB } as never })
    );
    expect(createClientCacheKey(stack, { programs: { ore: generatedProgramA } as never })).not.toBe(
      createClientCacheKey(stack, { programs: { ore: changedProgram } as never })
    );
    expect(createClientCacheKey(stack, { programs: { anonymous: anonymousProgramA } as never })).toBe(
      createClientCacheKey(stack, { programs: { anonymous: anonymousProgramB } as never })
    );
    expect(createClientCacheKey(stack, { programs: { ore: manualProgram } as never })).toBe(
      createClientCacheKey(stack, { programs: { ore: manualProgram } as never })
    );
    expect(createClientCacheKey(stack, { programs: { ore: manualProgram } as never })).not.toBe(
      createClientCacheKey(stack, {
        programs: { ore: { name: 'ore', rawInstructions: manualProgram.rawInstructions } } as never,
      })
    );
  });

  it('syncs the latest wallet across cached clients', () => {
    const first = { setWallet: jest.fn() };
    const second = { setWallet: jest.fn() };

    syncClientWallets(
      [
        { client: first as never, disconnect: jest.fn() },
        { client: second as never, disconnect: jest.fn() },
      ],
      wallet
    );

    expect(first.setWallet).toHaveBeenCalledWith(wallet);
    expect(second.setWallet).toHaveBeenCalledWith(wallet);
  });

  it('applies the latest wallet when a client finishes connecting', () => {
    const adapter = { setConnectionState: jest.fn() };
    let onConnectionStateChange:
      | ((state: 'connected' | 'error', error?: Error) => void)
      | undefined;
    const client = {
      connectionState: 'connected',
      setWallet: jest.fn(),
      onConnectionStateChange: jest.fn((callback) => {
        onConnectionStateChange = callback;
        return jest.fn();
      }),
    };

    initializeConnectedClient(client as never, adapter as never, wallet);

    expect(client.setWallet).toHaveBeenCalledWith(wallet);
    expect(adapter.setConnectionState).toHaveBeenCalledWith('connected');

    const error = new Error('boom');
    onConnectionStateChange?.('error', error);
    expect(adapter.setConnectionState).toHaveBeenLastCalledWith('error');
  });

  it('removes rejected connection promises so a later attempt can reconnect', async () => {
    const connecting = new Map<string, Promise<string>>();
    const rejected = Promise.reject(new Error('connection failed'));
    const rejection = expect(rejected).rejects.toThrow('connection failed');

    trackConnectingPromise(connecting, 'stack', rejected);
    expect(connecting.get('stack')).toBe(rejected);
    await rejection;
    await Promise.resolve();

    expect(connecting.has('stack')).toBe(false);
  });

  it('forwards initial connection and recovery policies independently', async () => {
    const connect = jest.spyOn(Arete, 'connect').mockReturnValue(
      new Promise(() => undefined) as never,
    );
    function Consumer() {
      useArete(stack);
      return null;
    }
    let renderer: ReactTestRenderer | undefined;

    await act(async () => {
      renderer = create(
        React.createElement(
          AreteProvider,
          { autoConnect: false, autoReconnect: true },
          React.createElement(Consumer),
        ),
      );
      await Promise.resolve();
    });

    expect(connect).toHaveBeenCalledWith(
      stack,
      expect.objectContaining({
        autoConnect: false,
        autoReconnect: true,
      }),
    );
    act(() => renderer?.unmount());
    connect.mockRestore();
  });

  it('leaves one active shared client under StrictMode', async () => {
    const clients: Array<{
      connectionState: 'connected';
      disconnect: jest.Mock;
      setWallet: jest.Mock;
      onConnectionStateChange: jest.Mock;
      onSocketIssue: jest.Mock;
      programs: undefined;
      queries: Record<string, never>;
      chain: Record<string, never>;
      store: Record<string, never>;
    }> = [];
    const connect = jest.spyOn(Arete, 'connect').mockImplementation(async () => {
      const client = {
        connectionState: 'connected' as const,
        disconnect: jest.fn(),
        setWallet: jest.fn(),
        onConnectionStateChange: jest.fn(() => jest.fn()),
        onSocketIssue: jest.fn(() => jest.fn()),
        programs: undefined,
        queries: {},
        chain: {},
        store: {},
      };
      clients.push(client);
      return client as never;
    });
    let current: { status: string } | undefined;
    function Consumer() {
      current = useArete(stack);
      return null;
    }
    let renderer: ReactTestRenderer | undefined;

    await act(async () => {
      renderer = create(
        React.createElement(
          React.StrictMode,
          null,
          React.createElement(
            AreteProvider,
            null,
            React.createElement(Consumer),
          ),
        ),
      );
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(current?.status).toBe('connected');
    expect(clients.filter((client) => client.disconnect.mock.calls.length === 0)).toHaveLength(1);

    act(() => renderer?.unmount());
    expect(clients.every((client) => client.disconnect.mock.calls.length === 1)).toBe(true);
    connect.mockRestore();
  });

  it('drives dependent generated-style view hooks across the provider boundary', async () => {
    const storage = new MemoryAdapter();
    const queries = new QueryStore(storage);
    const connection = {
      subscribe: jest.fn(),
      unsubscribe: jest.fn(),
      refresh: jest.fn(),
    };
    const registry = new SubscriptionRegistry(connection as never, queries);
    const processor = new FrameProcessor(storage, { queryStore: queries });
    const client = {
      connectionState: 'connected' as const,
      disconnect: jest.fn(),
      setWallet: jest.fn(),
      onConnectionStateChange: jest.fn(() => jest.fn()),
      onSocketIssue: jest.fn(() => jest.fn()),
      getSubscriptionRegistry: () => registry,
      programs: undefined,
      queries: {},
      chain: {},
      store: {},
    };
    const generatedStack = {
      name: 'generated-style',
      endpoints: { ws: 'wss://example.invalid' },
      views: {
        Board: {
          state: { mode: 'state', view: 'Board/state', keyFields: ['address'] },
        },
        Round: {
          state: { mode: 'state', view: 'Round/state', keyFields: ['roundId'] },
        },
      },
    } as const;
    jest.spyOn(Arete, 'connect').mockResolvedValue(client as never);

    let board: { data?: { state: { roundId: number } } } | undefined;
    let round: { data?: { id: { roundId: number } } } | undefined;
    function Consumer() {
      const arete = useArete(generatedStack);
      board = arete.views.Board.state.use({ address: 'board' }) as typeof board;
      const roundId = board?.data?.state.roundId;
      round = arete.views.Round.state.use(
        roundId === undefined ? undefined : { roundId },
      ) as typeof round;
      return null;
    }
    let renderer: ReactTestRenderer | undefined;
    await act(async () => {
      renderer = create(
        React.createElement(
          AreteProvider,
          { stack: generatedStack },
          React.createElement(Consumer),
        ),
      );
      await Promise.resolve();
      await Promise.resolve();
    });

    const active = (view: string, key: string) => connection.subscribe.mock.calls
      .map(([value]) => value as Subscription)
      .find((subscription) => (
        subscription.query.view === view && subscription.query.key === key
      ));
    const emit = (view: string, key: string, data: Record<string, unknown>) => {
      const subscription = active(view, key);
      if (!subscription) throw new Error(`Missing subscription for ${view}:${key}`);
      processor.handleFrame(parseFrame(JSON.stringify({
        protocolVersion: 2,
        subscriptionId: subscription.subscriptionId,
        snapshotId: `${view}:${key}`,
        authoritative: true,
        mode: 'state',
        entity: view,
        op: 'snapshot',
        key,
        data: [{ key, data }],
        complete: true,
      })));
    };

    expect(active('Board/state', 'board')).toBeDefined();
    act(() => emit('Board/state', 'board', { state: { roundId: 42 } }));
    expect(active('Round/state', '42')).toBeDefined();
    act(() => emit('Round/state', '42', { id: { roundId: 42 } }));
    expect(round?.data?.id.roundId).toBe(42);

    act(() => renderer?.unmount());
    expect(client.disconnect).toHaveBeenCalledTimes(1);
  });

  it('disconnects a client that resolves after its provider unmounts', async () => {
    let resolveClient!: (client: unknown) => void;
    const connection = new Promise((resolve) => {
      resolveClient = resolve;
    });
    const client = {
      connectionState: 'connected',
      disconnect: jest.fn(),
    };
    jest.spyOn(Arete, 'connect').mockReturnValue(connection as never);
    function Consumer() {
      useArete(stack);
      return null;
    }
    let renderer: ReactTestRenderer | undefined;
    await act(async () => {
      renderer = create(
        React.createElement(
          AreteProvider,
          null,
          React.createElement(Consumer),
        ),
      );
      await Promise.resolve();
    });

    act(() => renderer?.unmount());
    await act(async () => {
      resolveClient(client);
      await connection;
      await Promise.resolve();
    });

    expect(client.disconnect).toHaveBeenCalledTimes(1);
  });
});
