jest.mock('react', () => ({
  useState: jest.fn(),
  useRef: jest.fn((value: unknown) => ({ current: value })),
  useCallback: jest.fn((callback: unknown) => callback),
}));
jest.mock('@usearete/sdk', () => {
  class InstructionError extends Error {
    programError: unknown;

    constructor(message: string, programError?: unknown) {
      super(message);
      this.programError = programError;
    }
  }

  class OperationExecutionError extends Error {
    completedReceipts: unknown[] = [];
    operation: unknown;
  }

  return {
    InstructionError,
    OperationExecutionError,
    getTransactionFailureOutcome: (error: unknown) =>
      error && typeof error === 'object' && 'outcome' in error
        ? (error as { outcome: unknown }).outcome
        : null,
    unwrapOperationExecutionError: (error: unknown) =>
      error && typeof error === 'object' && 'cause' in error
        ? (error as { cause: unknown }).cause
        : error,
  };
});

import { useCallback, useRef, useState } from 'react';

import {
  useInstructionMutation,
  type MutationExecutionObserver,
  type UseMutationOptions,
} from './use-mutation';
import { createProcessedSlotReconciliation } from '../reconciliation';

const mockUseState = useState as jest.Mock;
const mockUseRef = useRef as jest.Mock;
const mockUseCallback = useCallback as jest.Mock;

function renderMutation<TParams, TResult>(
  execute: (
    args: TParams,
    options?: Record<string, never>,
    observer?: MutationExecutionObserver
  ) => Promise<TResult>,
  defaults?: Partial<UseMutationOptions<Record<string, never>, TResult>>
) {
  let currentState: Record<string, unknown> = {};
  const setState = jest.fn((update: unknown) => {
    currentState = typeof update === 'function'
      ? (update as (state: Record<string, unknown>) => Record<string, unknown>)(currentState)
      : update as Record<string, unknown>;
  });
  mockUseState.mockImplementationOnce((initial: unknown) => {
    currentState = typeof initial === 'function'
      ? (initial as () => Record<string, unknown>)()
      : initial as Record<string, unknown>;
    return [currentState, setState];
  });

  return {
    mutation: useInstructionMutation<TParams, TResult>(execute, defaults),
    state: () => currentState,
  };
}

describe('useInstructionMutation', () => {
  beforeEach(() => {
    mockUseState.mockReset();
    mockUseRef.mockClear();
    mockUseRef.mockImplementation((value: unknown) => ({ current: value }));
    mockUseCallback.mockReset();
    mockUseCallback.mockImplementation((callback: unknown) => callback);
  });

  it('preserves signature conveniences while exposing confirmed result data', async () => {
    const multi = renderMutation(async (_args: undefined) => ({
      signatures: ['sig-1', 'sig-2'],
      transactions: [
        { transactionIndex: 0, transactionName: 'one', signature: 'sig-1', slot: 10 },
        { transactionIndex: 1, transactionName: 'two', signature: 'sig-2', slot: 11 },
      ],
    }));
    const result = await multi.mutation.submit(undefined);

    expect(result.signatures).toEqual(['sig-1', 'sig-2']);
    expect(multi.state()).toMatchObject({
      status: 'success',
      phase: 'confirmed',
      result,
      signatures: ['sig-1', 'sig-2'],
      outcome: { status: 'confirmed', signature: 'sig-2', slot: 11 },
    });

    multi.mutation.reset();
    expect(multi.state()).toMatchObject({
      status: 'idle',
      signatures: [],
      result: undefined,
    });
  });

  it('normalizes non-Error throws and always passes an Error to onError', async () => {
    const onError = jest.fn();
    const { mutation, state } = renderMutation(async (_args: undefined) => {
      throw 'wallet unavailable';
    });

    await expect(mutation.submit(undefined, { onError })).rejects.toEqual(
      expect.objectContaining({ message: 'wallet unavailable' })
    );
    expect(onError).toHaveBeenCalledWith(expect.any(Error));
    expect(state()).toMatchObject({
      status: 'error',
      phase: 'not-submitted',
      displayError: 'wallet unavailable',
    });
  });

  it('offers a non-throwing event form while preserving the rejecting form', async () => {
    const failure = new Error('wallet unavailable');
    const { mutation, state } = renderMutation(async (_args: undefined) => {
      throw failure;
    });

    expect(() => mutation.mutate(undefined)).not.toThrow();
    await Promise.resolve();
    await Promise.resolve();

    expect(state()).toMatchObject({
      status: 'error',
      displayError: 'wallet unavailable',
    });
    await expect(mutation.mutateAsync(undefined)).rejects.toBe(failure);
  });

  it('remains pending until success callbacks settle', async () => {
    let release!: () => void;
    const callback = new Promise<void>((resolve) => {
      release = resolve;
    });
    const { mutation, state } = renderMutation(async (_args: undefined) => ({
      signature: 'sig',
    }));

    const pending = mutation.mutateAsync(undefined, {
      onSuccess: () => callback,
    });
    await Promise.resolve();
    await Promise.resolve();

    expect(state()).toMatchObject({ status: 'pending', phase: 'confirmed' });
    release();
    await pending;
    expect(state()).toMatchObject({ status: 'success', phase: 'confirmed' });
  });

  it('remains pending until retry success callbacks settle', async () => {
    let attempts = 0;
    let release!: () => void;
    const callback = new Promise<void>((resolve) => {
      release = resolve;
    });
    const { mutation, state } = renderMutation(async (_args: undefined) => ({
      signature: 'sig',
    }));

    await mutation.mutateAsync(undefined, {
      reconcile: async () => {
        attempts += 1;
        if (attempts === 1) throw new Error('stream unavailable');
      },
      onSuccess: () => callback,
    });
    const retry = mutation.retryReconciliation();
    await Promise.resolve();
    await Promise.resolve();

    expect(state()).toMatchObject({ status: 'pending', phase: 'reconciled' });
    release();
    await retry;
    expect(state()).toMatchObject({ status: 'success', phase: 'reconciled' });
  });

  it('keeps submitted-unknown distinct from confirmed state', async () => {
    const error = Object.assign(new Error('confirmation timed out'), {
      outcome: {
        status: 'submitted-unknown' as const,
        phase: 'confirmation' as const,
        signature: 'uncertain-signature',
        cause: new Error('confirmation timed out'),
      },
    });
    const { mutation, state } = renderMutation(async (_args: undefined) => {
      throw error;
    });

    await expect(mutation.submit(undefined)).rejects.toBe(error);
    expect(state()).toMatchObject({
      status: 'error',
      phase: 'submitted-unknown',
      signatures: ['uncertain-signature'],
      failure: { status: 'submitted-unknown' },
    });
  });

  it('does not let stale completions overwrite a newer submission', async () => {
    let resolveFirst!: (value: { signature: string }) => void;
    let resolveSecond!: (value: { signature: string }) => void;
    const execute = jest.fn((name: string) => new Promise<{ signature: string }>((resolve) => {
      if (name === 'first') resolveFirst = resolve;
      else resolveSecond = resolve;
    }));
    const { mutation, state } = renderMutation(execute);

    const first = mutation.submit('first');
    const second = mutation.submit('second');
    resolveSecond({ signature: 'newer' });
    await second;
    resolveFirst({ signature: 'older' });
    await first;

    expect(state()).toMatchObject({
      status: 'success',
      result: { signature: 'newer' },
      signatures: ['newer'],
    });
    expect(execute).toHaveBeenCalledTimes(2);
  });

  it('separates callback and reconciliation errors from confirmed execution', async () => {
    const callbackError = new Error('consumer callback failed');
    const reconciliationError = new Error('stream did not catch up');
    const onSuccess = jest.fn();
    const { mutation, state } = renderMutation(async (_args: undefined) => ({
      signature: 'confirmed-signature',
    }));

    await expect(mutation.submit(undefined, {
      onConfirmed: () => {
        throw callbackError;
      },
      onSuccess,
      reconcile: async () => {
        throw reconciliationError;
      },
    })).resolves.toEqual({ signature: 'confirmed-signature' });

    expect(state()).toMatchObject({
      status: 'success',
      phase: 'confirmed-unreconciled',
      outcome: { status: 'confirmed', signature: 'confirmed-signature' },
      callbackErrors: [callbackError],
      reconciliationError,
      displayError: null,
    });
    expect(onSuccess).not.toHaveBeenCalled();
  });

  it('treats an explicit confirmed-unreconciled result as post-confirmation state', async () => {
    const reconciliationError = new Error('view refresh timed out');
    const { mutation, state } = renderMutation(async (_args: undefined) => ({
      signature: 'confirmed-signature',
    }));

    await expect(mutation.submit(undefined, {
      reconcile: async () => ({
        status: 'confirmed-unreconciled' as const,
        error: reconciliationError,
      }),
    })).resolves.toEqual({ signature: 'confirmed-signature' });

    expect(state()).toMatchObject({
      status: 'success',
      phase: 'confirmed-unreconciled',
      outcome: { status: 'confirmed', signature: 'confirmed-signature' },
      reconciliationError,
      displayError: null,
    });
  });

  it('keeps submit stable while reading the latest inline defaults', async () => {
    const refs: Array<{ current: unknown }> = [];
    const callbacks: Array<{ callback: unknown; dependencies: readonly unknown[] }> = [];
    let currentState: Record<string, unknown> = {};
    let refIndex = 0;
    let callbackIndex = 0;

    mockUseState.mockImplementation((initial: unknown) => {
      if (Object.keys(currentState).length === 0) {
        currentState = typeof initial === 'function'
          ? (initial as () => Record<string, unknown>)()
          : initial as Record<string, unknown>;
      }
      return [currentState, (update: unknown) => {
        currentState = typeof update === 'function'
          ? (update as (state: Record<string, unknown>) => Record<string, unknown>)(currentState)
          : update as Record<string, unknown>;
      }];
    });
    mockUseRef.mockImplementation((initial: unknown) => {
      const index = refIndex++;
      refs[index] ??= { current: initial };
      return refs[index];
    });
    mockUseCallback.mockImplementation((callback: unknown, dependencies: readonly unknown[]) => {
      const index = callbackIndex++;
      const previous = callbacks[index];
      if (
        previous
        && previous.dependencies.length === dependencies.length
        && previous.dependencies.every((dependency, dependencyIndex) =>
          Object.is(dependency, dependencies[dependencyIndex])
        )
      ) {
        return previous.callback;
      }
      callbacks[index] = { callback, dependencies };
      return callback;
    });

    const execute = jest.fn(async (_args: undefined) => ({ signature: 'signature' }));
    const firstSuccess = jest.fn();
    const latestSuccess = jest.fn();
    const render = (onSuccess: (result: { signature: string }) => void) => {
      refIndex = 0;
      callbackIndex = 0;
      return useInstructionMutation(execute, { onSuccess });
    };

    const first = render(firstSuccess);
    const second = render(latestSuccess);

    expect(second.submit).toBe(first.submit);
    await second.submit(undefined);
    expect(latestSuccess).toHaveBeenCalledWith({ signature: 'signature' });
    expect(firstSuccess).not.toHaveBeenCalled();
  });

  it('captures prepared operations and completed receipts from execution events', async () => {
    const prepared = { kind: 'instruction', name: 'deploy' };
    const receipt = {
      transactionIndex: 0,
      transactionName: 'deploy',
      signature: 'deploy-signature',
      slot: 55,
    };
    const { mutation, state } = renderMutation(async (
      _args: undefined,
      _options,
      observer
    ) => {
      observer?.onPrepared(prepared as never);
      observer?.onTransactionSuccess({ receipt } as never);
      return { signature: receipt.signature };
    });

    await mutation.submit(undefined);
    expect(state()).toMatchObject({
      prepared,
      completedReceipts: [receipt],
      outcome: { status: 'confirmed', slot: 55 },
    });
  });

  it('skips the reconciling phase when reconcile is false', async () => {
    const client = { waitForProcessedSlot: jest.fn(async () => 11n) };
    const { mutation, state } = renderMutation(
      async (_args: undefined) => ({
        transactions: [
          { transactionIndex: 0, transactionName: 'deploy', signature: 'sig', slot: 10 },
        ],
      }),
      { reconcile: createProcessedSlotReconciliation(client) as never },
    );

    await mutation.submit(undefined, { reconcile: false });
    expect(state()).toMatchObject({ status: 'success', phase: 'confirmed' });
    expect(client.waitForProcessedSlot).not.toHaveBeenCalled();
  });

  it('resolves per-call reconcile override objects against the marked default', async () => {
    const client = { waitForProcessedSlot: jest.fn(async () => 11n) };
    const refresh = jest.fn();
    const { mutation, state } = renderMutation(
      async (_args: undefined) => ({
        transactions: [
          { transactionIndex: 0, transactionName: 'deploy', signature: 'sig', slot: 10 },
        ],
      }),
      { reconcile: createProcessedSlotReconciliation(client) as never },
    );

    await mutation.submit(undefined, { reconcile: { refresh } });
    expect(state()).toMatchObject({ status: 'success', phase: 'reconciled' });
    expect(client.waitForProcessedSlot).toHaveBeenCalledWith(10n, expect.objectContaining({
      timeoutMs: 30_000,
    }));
    expect(refresh).toHaveBeenCalled();
  });

  it('skips default reconciliation when no receipt reports a slot', async () => {
    const client = { waitForProcessedSlot: jest.fn(async () => 11n) };
    const onSuccess = jest.fn();
    const { mutation, state } = renderMutation(
      async (_args: undefined) => ({ signature: 'sig' }),
      { reconcile: createProcessedSlotReconciliation(client) as never, onSuccess },
    );

    await expect(mutation.submit(undefined)).resolves.toEqual({ signature: 'sig' });
    expect(state()).toMatchObject({ status: 'success', phase: 'confirmed' });
    expect(client.waitForProcessedSlot).not.toHaveBeenCalled();
    expect(onSuccess).toHaveBeenCalledWith({ signature: 'sig' });
  });

  it('rejects reconcile override objects without a marked default reconciliation', async () => {
    const { mutation, state } = renderMutation(async (_args: undefined) => ({
      signature: 'sig',
    }));

    await expect(mutation.submit(undefined, { reconcile: { timeoutMs: 1 } })).rejects.toEqual(
      expect.objectContaining({
        message: expect.stringContaining('reconcile option objects require'),
      })
    );
    expect(state()).toMatchObject({ status: 'error', phase: 'not-submitted' });
  });

  it('exposes phase booleans derived from the current phase', () => {
    const cases = [
      { phase: 'preparing', isPreparing: true, isAwaitingWallet: false, isReconciling: false },
      { phase: 'awaiting-wallet', isPreparing: false, isAwaitingWallet: true, isReconciling: false },
      { phase: 'reconciling', isPreparing: false, isAwaitingWallet: false, isReconciling: true },
      { phase: 'reconciled', isPreparing: false, isAwaitingWallet: false, isReconciling: false },
    ] as const;

    for (const expected of cases) {
      mockUseState.mockImplementationOnce(() => [{
        status: 'pending',
        phase: expected.phase,
        latestEvent: null,
        displayError: null,
        failure: null,
        outcome: null,
        prepared: null,
        result: undefined,
        signatures: [],
        completedReceipts: [],
        callbackErrors: [],
        reconciliationError: null,
      }, jest.fn()]);
      const mutation = useInstructionMutation(async () => ({}));
      expect({
        isPreparing: mutation.isPreparing,
        isAwaitingWallet: mutation.isAwaitingWallet,
        isReconciling: mutation.isReconciling,
      }).toEqual({
        isPreparing: expected.isPreparing,
        isAwaitingWallet: expected.isAwaitingWallet,
        isReconciling: expected.isReconciling,
      });
    }
  });
});
