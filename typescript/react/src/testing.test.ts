import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { extendStack, getTransactionFailureOutcome } from '@usearete/sdk';

import type { MutationPhase } from './hooks';
import { AreteProvider } from './provider';
import { useArete, type UseAreteResult } from './stack';
import {
  MUTATION_PHASES,
  createAreteTestConfig,
  createMutationResultFixture,
  createWebSocketHarness,
  frames,
  type WebSocketHarness,
} from './testing';

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

function render(element: React.ReactElement): () => void {
  let renderer: ReactTestRenderer | undefined;
  act(() => {
    renderer = create(element);
  });
  return () => act(() => renderer?.unmount());
}

async function until(check: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100 && !check(); attempt += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
  }
  expect(check()).toBe(true);
}

describe('createAreteTestConfig with the WebSocket harness', () => {
  let ws: WebSocketHarness;
  beforeEach(() => {
    ws = createWebSocketHarness();
  });
  afterEach(() => ws.restore());

  it('renders view data the harness serves through a real provider client', async () => {
    let things: ReturnType<UseAreteResult<typeof STACK>['views']['Thing']['list']['use']> | undefined;
    function Things() {
      things = useArete(STACK).views.Thing.list.use();
      return null;
    }
    const unmount = render(
      React.createElement(AreteProvider, createAreteTestConfig(ws), React.createElement(Things))
    );

    // Let React commit between polls: an `act` scope defers renders to its end.
    await until(() => ws.sockets.length > 0 && ws.subscriptions().length > 0);
    const subscription = ws.subscription('Thing/list');
    act(() => ws.serve(subscription, [{ key: 'a', data: { id: 'a', size: 1 } }]));
    await until(() => things?.status === 'ready');
    expect(things?.data).toEqual([{ id: 'a', size: 1 }]);

    // A patch for a key the client does not hold never becomes an entity.
    act(() => ws.send(frames.patch(subscription, 'b', { size: 9 })));
    act(() => ws.send(frames.patch(subscription, 'a', { size: 2 })));
    expect(things?.data).toEqual([{ id: 'a', size: 2 }]);
    unmount();
  });

  it('keeps stack reads when the generated stack is spread', async () => {
    const extended = extendStack(STACK, {
      readArgCounts: { ping: 0 },
      createRead: () => ({ ping: async () => 'pong' }),
    });
    const spread = { ...extended };
    let ping: { status: string; data?: unknown } | undefined;
    function Reader() {
      ping = useArete(spread).read.ping.use();
      return null;
    }
    const unmount = render(
      React.createElement(AreteProvider, createAreteTestConfig(ws), React.createElement(Reader))
    );

    await until(() => ping?.status === 'ready');
    expect(ping?.data).toBe('pong');
    unmount();
  });

  it('reports a program key conflict from useArete(stack, { programs })', async () => {
    const programs = {
      things: { name: 'other', programId: 'Other11111111111111111111111111111111111111' },
    };
    let arete: { error: Error | null } | undefined;
    function Conflicted() {
      arete = useArete(STACK, { programs });
      return null;
    }
    const unmount = render(
      React.createElement(AreteProvider, createAreteTestConfig(ws), React.createElement(Conflicted))
    );

    await until(() => arete?.error != null);
    expect(arete?.error).toMatchObject({ code: 'PROGRAM_KEY_CONFLICT' });
    expect(ws.sockets).toHaveLength(0);
    unmount();
  });

  it('uses an attached program with the same program spec, warning once', async () => {
    const stack = {
      ...STACK,
      name: 'things-local',
      programs: { things: { ...STACK.programs.things, programSpecHash: 'spec-things' } },
    } as const;
    // A local standalone build of the stack's program: same spec, no release.
    const programs = { things: { ...stack.programs.things, name: 'things-standalone' } };
    const warn = jest.spyOn(console, 'warn').mockImplementation(() => undefined);
    let arete: { error: Error | null; programs: Record<string, { programId?: string }> } | undefined;
    function Attached() {
      arete = useArete(stack, { programs }) as never;
      return null;
    }
    const unmount = render(
      React.createElement(AreteProvider, createAreteTestConfig(ws), React.createElement(Attached))
    );

    try {
      await until(() => arete?.programs?.things !== undefined);
      expect(arete?.error).toBeNull();
      expect(warn.mock.calls.filter(([message]) => /could not be proven identical/.test(String(message))))
        .toHaveLength(1);
    } finally {
      warn.mockRestore();
      unmount();
    }
  });
});

describe('createMutationResultFixture', () => {
  it('lists every mutation phase', () => {
    type Missing = Exclude<MutationPhase, (typeof MUTATION_PHASES)[number]>;
    const exhaustive: [Missing] extends [never] ? true : false = true;
    expect(exhaustive).toBe(true);
    expect(new Set(MUTATION_PHASES).size).toBe(11);
  });

  it.each(MUTATION_PHASES)('builds a consistent %s result', async (phase) => {
    const result = createMutationResultFixture(phase);

    expect(result.phase).toBe(phase);
    expect(result.isPreparing).toBe(phase === 'preparing');
    expect(result.isAwaitingWallet).toBe(phase === 'awaiting-wallet');
    expect(result.isReconciling).toBe(phase === 'reconciling');
    expect(result.isLoading).toBe(result.status === 'pending');
    expect(result.canRetryReconciliation).toBe(phase === 'confirmed-unreconciled');
    expect(result.reconciliationError !== null).toBe(phase === 'confirmed-unreconciled');

    const failed = ['not-submitted', 'submitted-unknown', 'chain-failed'].includes(phase);
    expect(result.status).toBe(
      phase === 'idle'
        ? 'idle'
        : failed
          ? 'error'
          : phase === 'reconciled' || phase === 'confirmed-unreconciled'
            ? 'success'
            : 'pending'
    );
    expect(result.failure?.status ?? null).toBe(failed ? phase : null);
    expect(result.displayError !== null).toBe(failed);
    expect(result.isConfirmed).toBe(
      ['confirmed', 'reconciling', 'reconciled', 'confirmed-unreconciled'].includes(phase)
    );
    expect(result.isSubmittedUnknown).toBe(phase === 'submitted-unknown');
    expect(result.signatures.length > 0).toBe(
      !['idle', 'preparing', 'awaiting-wallet', 'not-submitted'].includes(phase)
    );

    if (failed) {
      const error = await result.submit({}).catch((value: unknown) => value);
      expect(getTransactionFailureOutcome(error)?.status).toBe(phase);
    }
  });

  it('applies overrides', () => {
    expect(createMutationResultFixture('reconciled', { signature: 'custom' }).signature).toBe('custom');
  });
});
