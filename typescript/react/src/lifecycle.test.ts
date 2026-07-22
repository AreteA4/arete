import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';

import { useInstructionMutation, type UseMutationResult } from './hooks/use-mutation';
import { useAsyncRead, type UseAsyncReadResult } from './hooks/use-async-read';
import { buildProgramHookInterfaces } from './program-hooks';
import { buildReadInterfaces } from './read-hooks';
import { useListView, useStateView } from './view-hooks';
import type { ViewHookResult } from './types';
import {
  FrameProcessor,
  MemoryAdapter,
  QueryStore,
  SubscriptionRegistry,
  parseFrame,
  type OperationExecutionOptions,
  type PreparedOperation,
  type Subscription,
} from '@usearete/sdk';

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

function createViewClient() {
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
    getSubscriptionRegistry: () => registry,
  };
  let snapshotId = 0;
  let sequence = 0;
  const active = (view: string, key?: string) => connection.subscribe.mock.calls
    .map(([value]) => value as Subscription)
    .find((subscription) =>
      subscription.query.view === view && subscription.query.key === key
    );
  const emit = (view: string, values: Array<{ id: string }>, key?: string) => {
    const subscription = active(view, key);
    if (!subscription) throw new Error(`Missing subscription for ${view}`);
    snapshotId++;
    processor.handleFrame(parseFrame(JSON.stringify({
      protocolVersion: 2,
      subscriptionId: subscription.subscriptionId,
      snapshotId: `snapshot-${snapshotId}`,
      authoritative: true,
      mode: key === undefined ? 'list' : 'state',
      entity: view,
      op: 'snapshot',
      ...(key === undefined ? {} : { key }),
      data: values.map((value) => ({ key: value.id, data: value })),
      complete: true,
    })));
  };
  const emitPatch = (view: string, key: string, value: Record<string, unknown>) => {
    const subscription = active(view, key);
    if (!subscription) throw new Error(`Missing subscription for ${view}`);
    sequence++;
    processor.handleFrame(parseFrame(JSON.stringify({
      protocolVersion: 2,
      subscriptionId: subscription.subscriptionId,
      mode: 'state',
      entity: view,
      op: 'patch',
      key,
      data: value,
      seq: `${sequence}:000000000000`,
    })));
  };
  return {
    client,
    connection,
    registry,
    setList(value: Array<{ id: string }>) {
      emit('Item/list', value);
    },
    emitEmpty(view: string, key?: string) {
      emit(view, [], key);
    },
    setStateSnapshot(view: string, key: string, value: { id: string; [key: string]: unknown }) {
      emit(view, [value], key);
    },
    patchState(view: string, key: string, value: Record<string, unknown>) {
      emitPatch(view, key, value);
    },
  };
}

describe('real React lifecycle integration', () => {
  it('reruns a keyed read when a replacement client supplies a new read function', async () => {
    const firstRead = jest.fn(async (_authority: string) => 'first');
    const secondRead = jest.fn(async (_authority: string) => 'second');
    let read = firstRead;
    let current: UseAsyncReadResult<string> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      const reads = buildReadInterfaces({ preview: read }, { preview: 1 });
      current = reads.preview.use('wallet');
      return null;
    }

    await act(async () => {
      renderer = create(React.createElement(Harness));
      await Promise.resolve();
    });
    expect(current?.data).toBe('first');

    read = secondRead;
    await act(async () => {
      renderer?.update(React.createElement(Harness));
      await Promise.resolve();
    });

    expect(secondRead).toHaveBeenCalledWith('wallet');
    expect(current?.data).toBe('second');
    act(() => renderer?.unmount());
  });

  it('treats initial list data as a seed and does not restore it after live removal', () => {
    const { client, setList } = createViewClient();
    let current: ViewHookResult<Array<{ id: string }>> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      current = useListView(
        { mode: 'list', view: 'Item/list' },
        client as never,
        undefined,
        { initialData: [{ id: 'seed' }] },
      );
      return null;
    }
    act(() => {
      renderer = create(React.createElement(Harness));
    });
    expect(current?.data).toEqual([{ id: 'seed' }]);
    expect(current?.isLoading).toBe(false);

    act(() => setList([{ id: 'live' }]));
    expect(current?.data).toEqual([{ id: 'live' }]);

    act(() => setList([]));
    expect(current?.data).toEqual([]);
    act(() => renderer?.unmount());
  });

  it('publishes identifiable empty snapshots and leaves disabled hooks idle', () => {
    const { client, connection, emitEmpty, setList } = createViewClient();
    let list: ViewHookResult<Array<{ id: string }>> | undefined;
    let state: ViewHookResult<{ id: string }> | undefined;
    let disabledState: ViewHookResult<{ id: string }> | undefined;
    let disabledList: ViewHookResult<Array<{ id: string }>> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      list = useListView(
        { mode: 'list', view: 'Item/list' },
        client as never,
        undefined,
        { initialData: [{ id: 'seed' }] },
      );
      state = useStateView(
        { mode: 'state', view: 'Item/state', keyFields: ['id'] },
        client as never,
        { id: 'missing' },
        { initialData: { id: 'seed' } },
      );
      disabledState = useStateView(
        { mode: 'state', view: 'Disabled/state', keyFields: ['id'] },
        client as never,
        { id: 'missing' },
        { enabled: false },
      );
      disabledList = useListView(
        { mode: 'list', view: 'Disabled/list' },
        client as never,
        undefined,
        { enabled: false },
      );
      return null;
    }
    act(() => {
      renderer = create(React.createElement(Harness));
    });
    expect(disabledState).toMatchObject({ data: undefined, isLoading: false });
    expect(disabledList).toMatchObject({ data: undefined, isLoading: false });
    expect(connection.subscribe).toHaveBeenCalledTimes(2);

    act(() => setList([{ id: 'stale' }]));
    expect(list?.data).toEqual([{ id: 'stale' }]);
    act(() => emitEmpty('Item/list'));
    expect(list?.data).toEqual([]);
    expect(list?.isLoading).toBe(false);
    expect(list?.isEmpty).toBe(true);
    act(() => emitEmpty('Item/state', 'missing'));
    expect(state?.data).toBeUndefined();
    expect(state?.isLoading).toBe(false);
    expect(state?.isEmpty).toBe(true);
    act(() => renderer?.unmount());
  });

  it('clears state data immediately when an active hook is disabled', () => {
    const { client, setStateSnapshot } = createViewClient();
    let current: ViewHookResult<{ id: string }> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness({ enabled }: { enabled: boolean }) {
      current = useStateView(
        { mode: 'state', view: 'Item/state', keyFields: ['id'] },
        client as never,
        { id: 'item' },
        { enabled },
      );
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness, { enabled: true })); });
    act(() => setStateSnapshot('Item/state', 'item', { id: 'item' }));
    expect(current?.data).toEqual({ id: 'item' });

    act(() => renderer?.update(React.createElement(Harness, { enabled: false })));
    expect(current).toMatchObject({
      data: undefined,
      status: 'disabled',
      isEmpty: false,
    });
    act(() => renderer?.unmount());
  });

  it('keeps empty reads ready while refreshing an undefined result', async () => {
    let resolveRefresh: (() => void) | undefined;
    let calls = 0;
    const execute = jest.fn(async () => {
      calls += 1;
      if (calls === 1) return undefined;
      await new Promise<void>((resolve) => { resolveRefresh = resolve; });
      return undefined;
    });
    let current: UseAsyncReadResult<undefined> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      current = useAsyncRead('empty', execute);
      return null;
    }
    await act(async () => {
      renderer = create(React.createElement(Harness));
      await Promise.resolve();
    });
    expect(current).toMatchObject({ status: 'ready', isReady: true, isEmpty: true });

    let refresh!: Promise<undefined>;
    await act(async () => {
      refresh = current!.refresh();
      await Promise.resolve();
    });
    expect(current).toMatchObject({
      status: 'refreshing',
      isLoading: false,
      isReady: true,
      isEmpty: true,
    });
    await act(async () => {
      resolveRefresh?.();
      await refresh;
    });
    act(() => renderer?.unmount());
  });

  it('re-renders keyed state hooks for repeated same-key live patches', () => {
    const { client, patchState, setStateSnapshot } = createViewClient();
    let current: ViewHookResult<{ id: string; deployCount?: number }> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      current = useStateView(
        { mode: 'state', view: 'Round/state', keyFields: ['id'] },
        client as never,
        { id: 'round-1' },
      );
      return null;
    }
    act(() => {
      renderer = create(React.createElement(Harness));
    });

    act(() => setStateSnapshot('Round/state', 'round-1', {
      id: 'round-1',
      deployCount: 0,
    }));
    act(() => patchState('Round/state', 'round-1', { deployCount: 1 }));
    expect(current?.data).toEqual({ id: 'round-1', deployCount: 1 });

    act(() => patchState('Round/state', 'round-1', { deployCount: 2 }));
    expect(current?.data).toEqual({ id: 'round-1', deployCount: 2 });
    act(() => renderer?.unmount());
  });

  it('keeps mutations pending until reconciliation and orders callbacks', async () => {
    let finishReconciliation: (() => void) | undefined;
    const reconciliation = new Promise<void>((resolve) => {
      finishReconciliation = resolve;
    });
    const calls: string[] = [];
    let mutation: UseMutationResult<undefined, { signature: string }> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      mutation = useInstructionMutation(async () => ({ signature: 'sig' }), {
        onConfirmed: () => { calls.push('confirmed'); },
        onSuccess: () => { calls.push('success'); },
        reconcile: async () => {
          calls.push('reconciling');
          await reconciliation;
        },
      });
      return null;
    }
    act(() => {
      renderer = create(React.createElement(Harness));
    });

    let submission!: Promise<{ signature: string }>;
    await act(async () => {
      submission = mutation!.submit(undefined);
      await Promise.resolve();
    });
    expect(mutation).toMatchObject({
      status: 'pending',
      phase: 'reconciling',
      isLoading: true,
      isConfirmed: true,
    });
    expect(calls).toEqual(['confirmed', 'reconciling']);

    await act(async () => {
      finishReconciliation?.();
      await submission;
    });
    expect(mutation).toMatchObject({ status: 'success', phase: 'reconciled', isLoading: false });
    expect(calls).toEqual(['confirmed', 'reconciling', 'success']);
    act(() => renderer?.unmount());
  });

  it('receives incremental operation receipts through generated mutation hooks', async () => {
    const prepared = {
      kind: 'transaction' as const,
      name: 'deploy',
      transaction: {
        name: 'deploy',
        instructions: [],
        requiredSignerAddresses: [],
        errors: [],
      },
      plan: {
        name: 'deploy',
        artifacts: {},
        transactions: [{
          name: 'deploy',
          instructions: [],
          requiredSignerAddresses: [],
          errors: [],
        }],
      },
      artifacts: {},
    };
    const receipt = {
      transactionIndex: 0,
      transactionName: 'deploy',
      signature: 'deploy-signature',
      slot: 55,
    };
    let releaseExecution: (() => void) | undefined;
    let reportSubmitted: (() => void) | undefined;
    const executionReleased = new Promise<void>((resolve) => { releaseExecution = resolve; });
    const submitted = new Promise<void>((resolve) => { reportSubmitted = resolve; });
    const callerStart = jest.fn();
    const callerSuccess = jest.fn();
    const client = {
      transaction: jest.fn(),
      execute: jest.fn(async (
        _prepared: PreparedOperation,
        options?: OperationExecutionOptions,
      ) => {
        const transaction = prepared.plan.transactions[0]!;
        await options?.onTransactionStart?.({
          operation: prepared as PreparedOperation,
          transaction,
          transactionIndex: 0,
        });
        await options?.onTransactionSuccess?.({
          operation: prepared as PreparedOperation,
          transaction,
          transactionIndex: 0,
          receipt,
        });
        reportSubmitted?.();
        await executionReleased;
        return {
          kind: 'transaction',
          operationName: 'deploy',
          artifacts: {},
          signatures: [receipt.signature],
          transaction: receipt,
        };
      }),
    };
    const programs = buildProgramHookInterfaces({
      ore: {
        name: 'ore',
        programId: 'ore-program',
        schemas: {},
        pdas: {},
        accounts: {},
        queries: {},
        raw: {},
        instructions: {},
        transactions: {
          mining: {
            deploy: { prepare: jest.fn(async () => prepared) },
          },
        },
        flows: {},
      },
    } as never, client as never, useInstructionMutation as never, {
      defaultReconciliation: false,
    });
    let mutation: ReturnType<typeof programs.ore.transactions.mining.deploy.useMutation> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      mutation = programs.ore.transactions.mining.deploy.useMutation();
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });

    let submission!: Promise<unknown>;
    await act(async () => {
      submission = mutation!.submit({}, {
        onTransactionStart: callerStart,
        onTransactionSuccess: callerSuccess,
      } as never);
      await submitted;
    });
    expect(mutation).toMatchObject({
      phase: 'submitted',
      signatures: ['deploy-signature'],
      completedReceipts: [receipt],
    });
    expect(callerStart).toHaveBeenCalledTimes(1);
    expect(callerSuccess).toHaveBeenCalledTimes(1);

    await act(async () => {
      releaseExecution?.();
      await submission;
    });
    expect(mutation).toMatchObject({ phase: 'confirmed', status: 'success' });
    act(() => renderer?.unmount());
  });

  it('resolves confirmed results without onSuccess when reconciliation fails', async () => {
    const onConfirmed = jest.fn();
    const onSuccess = jest.fn();
    let mutation: UseMutationResult<undefined, { signature: string }> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      mutation = useInstructionMutation(async () => ({ signature: 'sig' }), {
        onConfirmed,
        onSuccess,
        reconcile: async () => {
          throw new Error('stream unavailable');
        },
      });
      return null;
    }
    act(() => {
      renderer = create(React.createElement(Harness));
    });

    let result: { signature: string } | undefined;
    await act(async () => {
      result = await mutation!.submit(undefined);
    });
    expect(result).toEqual({ signature: 'sig' });
    expect(mutation).toMatchObject({
      status: 'success',
      phase: 'confirmed-unreconciled',
      reconciliationError: expect.objectContaining({ message: 'stream unavailable' }),
    });
    expect(onConfirmed).toHaveBeenCalledWith({ signature: 'sig' });
    expect(onSuccess).not.toHaveBeenCalled();
    act(() => renderer?.unmount());
  });

  it('retries reconciliation without executing the mutation again', async () => {
    const execute = jest.fn(async () => ({ signature: 'sig' }));
    const onSuccess = jest.fn();
    let attempts = 0;
    let mutation: UseMutationResult<undefined, { signature: string }> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      mutation = useInstructionMutation(execute, {
        onSuccess,
        reconcile: async () => {
          attempts += 1;
          if (attempts === 1) throw new Error('stream unavailable');
        },
      });
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });

    await act(async () => { await mutation!.submit(undefined); });
    expect(mutation).toMatchObject({
      phase: 'confirmed-unreconciled',
      canRetryReconciliation: true,
    });

    await act(async () => { await mutation!.retryReconciliation(); });
    expect(mutation).toMatchObject({
      phase: 'reconciled',
      canRetryReconciliation: false,
      reconciliationError: null,
    });
    expect(execute).toHaveBeenCalledTimes(1);
    expect(onSuccess).toHaveBeenCalledWith({ signature: 'sig' });
    act(() => renderer?.unmount());
  });

  it('keeps retry reconciliation bound to the latest overlapping submission', async () => {
    let resolveFirst: ((value: { signature: string }) => void) | undefined;
    let resolveSecond: ((value: { signature: string }) => void) | undefined;
    const execute = jest.fn((name: string) => new Promise<{ signature: string }>((resolve) => {
      if (name === 'first') resolveFirst = resolve;
      else resolveSecond = resolve;
    }));
    const reconciled: string[] = [];
    let secondAttempts = 0;
    let mutation: UseMutationResult<string, { signature: string }> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      mutation = useInstructionMutation(execute, {
        reconcile: async ({ result }) => {
          reconciled.push(result.signature);
          if (result.signature === 'second' && secondAttempts++ === 0) {
            throw new Error('retry second');
          }
        },
      });
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });

    let first!: Promise<{ signature: string }>;
    let second!: Promise<{ signature: string }>;
    act(() => {
      first = mutation!.submit('first');
      second = mutation!.submit('second');
    });
    await act(async () => {
      resolveSecond?.({ signature: 'second' });
      await second;
    });
    await act(async () => {
      resolveFirst?.({ signature: 'first' });
      await first;
    });
    await act(async () => { await mutation!.retryReconciliation(); });

    expect(reconciled).toEqual(['second', 'second']);
    expect(execute).toHaveBeenCalledTimes(2);
    expect(mutation?.phase).toBe('reconciled');
    act(() => renderer?.unmount());
  });

  it('aborts reconciliation on reset without firing success callbacks', async () => {
    let reconciliationSignal: AbortSignal | undefined;
    const onSuccess = jest.fn();
    let mutation: UseMutationResult<undefined, { signature: string }> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      mutation = useInstructionMutation(async () => ({ signature: 'sig' }), {
        onSuccess,
        reconcile: ({ signal }) => new Promise<void>((_resolve, reject) => {
          reconciliationSignal = signal;
          signal.addEventListener('abort', () => reject(new Error('aborted')), { once: true });
        }),
      });
      return null;
    }
    act(() => {
      renderer = create(React.createElement(Harness));
    });

    let submission!: Promise<{ signature: string }>;
    await act(async () => {
      submission = mutation!.submit(undefined);
      await Promise.resolve();
    });
    act(() => mutation!.reset());
    expect(reconciliationSignal?.aborted).toBe(true);
    await expect(submission).resolves.toEqual({ signature: 'sig' });
    expect(mutation).toMatchObject({ status: 'idle', phase: 'idle' });
    expect(onSuccess).not.toHaveBeenCalled();
    act(() => renderer?.unmount());
  });
});
