jest.mock('react', () => ({
  useState: jest.fn(),
  useRef: jest.fn((value: unknown) => ({ current: value })),
  useCallback: jest.fn((callback: unknown) => callback),
  useEffect: jest.fn(),
}));

import { useEffect, useState } from 'react';

import { buildReadHookInterfaces } from './read-hooks';

const mockUseState = useState as jest.Mock;
const mockUseEffect = useEffect as jest.Mock;

function renderHook<T>(fn: () => T): { result: T; state: () => Record<string, unknown> } {
  let currentState: Record<string, unknown> = {};
  mockUseState.mockImplementationOnce((initial: unknown) => {
    currentState = typeof initial === 'function'
      ? (initial as () => Record<string, unknown>)()
      : initial as Record<string, unknown>;
    return [currentState, (update: unknown) => {
      currentState = typeof update === 'function'
        ? (update as (state: Record<string, unknown>) => Record<string, unknown>)(currentState)
        : update as Record<string, unknown>;
    }];
  });
  mockUseEffect.mockImplementation((effect: () => void | (() => void)) => {
    effect();
  });
  return { result: fn(), state: () => currentState };
}

describe('buildReadHookInterfaces', () => {
  beforeEach(() => {
    mockUseState.mockReset();
    mockUseEffect.mockReset();
  });

  it('runs the read when every argument is present', async () => {
    const solClaimPreview = jest.fn(async (authority: string) => ({ authority }));
    const reads = buildReadHookInterfaces({ solClaimPreview }, { solClaimPreview: 1 });

    const { result } = renderHook(() => reads.solClaimPreview.use('wallet'));
    await expect(result.refresh()).resolves.toEqual({ authority: 'wallet' });

    expect(solClaimPreview).toHaveBeenCalledWith('wallet');
  });

  it('stays disabled while any argument is nullish', () => {
    const solClaimPreview = jest.fn(async () => ({}));
    const reads = buildReadHookInterfaces({ solClaimPreview }, { solClaimPreview: 1 });

    const undefinedArg = renderHook(() => reads.solClaimPreview.use(undefined as never));
    expect(solClaimPreview).not.toHaveBeenCalled();
    expect(undefinedArg.result.isLoading).toBe(false);
    expect(undefinedArg.result.status).toBe('disabled');

    const nullArg = renderHook(() => reads.solClaimPreview.use(null as never));
    expect(solClaimPreview).not.toHaveBeenCalled();
    expect(nullArg.result.isLoading).toBe(false);
  });

  it('stays disabled while a required argument is omitted', () => {
    const solClaimPreview = jest.fn(async (authority: string) => ({ authority }));
    const reads = buildReadHookInterfaces({ solClaimPreview }, { solClaimPreview: 1 });
    const useWithoutArgs = reads.solClaimPreview.use as unknown as () => {
      isLoading: boolean;
      status: string;
    };

    const omittedArg = renderHook(useWithoutArgs);

    expect(solClaimPreview).not.toHaveBeenCalled();
    expect(omittedArg.result.isLoading).toBe(false);
    expect(omittedArg.result.status).toBe('disabled');
  });

  it('stays disabled without throwing when the client is not connected', () => {
    const reads = buildReadHookInterfaces(null, { anything: 1 });

    const { result } = renderHook(() => reads.anything.use('wallet'));

    expect(result.isLoading).toBe(false);
    expect(result.status).toBe('connecting');
    expect(result.data).toBeUndefined();
  });

  it('accepts options for zero-argument reads', () => {
    jest.useFakeTimers();
    const currentRound = jest.fn(async () => ({ roundId: 42n }));
    const reads = buildReadHookInterfaces({ currentRound }, { currentRound: 0 });

    const { result } = renderHook(() => reads.currentRound.use(
      { debounceMs: 300 },
    ));

    expect(result.status).toBe('loading');
    expect(currentRound).not.toHaveBeenCalled();
    jest.advanceTimersByTime(300);
    expect(currentRound).toHaveBeenCalledWith();
    jest.useRealTimers();
  });

  it('accepts options last through use', () => {
    jest.useFakeTimers();
    const solClaimPreview = jest.fn(async (authority: string) => ({ authority }));
    const reads = buildReadHookInterfaces({ solClaimPreview }, { solClaimPreview: 1 });

    const { result } = renderHook(() => reads.solClaimPreview.use(
      'wallet',
      { debounceMs: 300 },
    ));

    expect(result.isPending).toBe(true);
    expect(solClaimPreview).not.toHaveBeenCalled();
    jest.advanceTimersByTime(300);
    expect(solClaimPreview).toHaveBeenCalledWith('wallet');
    jest.useRealTimers();
  });

  it('keeps imperative and hook reads on the same function', async () => {
    const solClaimPreview = jest.fn(async (authority: string) => ({ authority }));
    const reads = buildReadHookInterfaces({ solClaimPreview }, { solClaimPreview: 1 });

    await expect(reads.solClaimPreview('wallet')).resolves.toEqual({ authority: 'wallet' });
    expect(() => buildReadHookInterfaces(null, { anything: 1 }).anything('wallet')).toThrow(
      'Arete client is not connected',
    );
  });

  it('keys reads by name and arguments so different args refetch', async () => {
    const seen: string[] = [];
    const solClaimPreview = jest.fn(async (authority: string) => {
      seen.push(authority);
      return { authority };
    });
    const reads = buildReadHookInterfaces({ solClaimPreview }, { solClaimPreview: 1 });

    const alice = renderHook(() => reads.solClaimPreview.use('alice'));
    const bob = renderHook(() => reads.solClaimPreview.use('bob'));
    await alice.result.refresh();
    await bob.result.refresh();

    expect(seen).toEqual(['alice', 'bob', 'alice', 'bob']);
  });

  it('supports multi-arg and bigint args', async () => {
    const preview = jest.fn(async (authority: string, roundId: bigint) => ({ authority, roundId }));
    const reads = buildReadHookInterfaces({ preview }, { preview: 2 });

    const { result } = renderHook(() => reads.preview.use('wallet', 42n));
    await expect(result.refresh()).resolves.toEqual({ authority: 'wallet', roundId: 42n });

    expect(preview).toHaveBeenCalledWith('wallet', 42n);
  });

  it('does not mistake a binary read argument for unary hook options', async () => {
    const preview = jest.fn(async (
      authority: string,
      input: { debounceMs: number },
    ) => ({ authority, input }));
    const reads = buildReadHookInterfaces({ preview }, { preview: 2 });

    const { result } = renderHook(() => reads.preview.use('wallet', { debounceMs: 42 }));
    await expect(result.refresh()).resolves.toEqual({
      authority: 'wallet',
      input: { debounceMs: 42 },
    });
    expect(preview).toHaveBeenCalledWith('wallet', { debounceMs: 42 });
  });

  it('keeps omitted optional read arguments before hook options', async () => {
    const preview = jest.fn(async (
      authority: string,
      bps: number | undefined = 10_000,
    ) => ({ authority, bps }));
    const reads = buildReadHookInterfaces({ preview }, { preview: [1, 2] });

    const { result } = renderHook(() => reads.preview.use(
      'wallet',
      undefined,
      { initialData: { authority: 'seed', bps: 0 } },
    ));
    await expect(result.refresh()).resolves.toEqual({ authority: 'wallet', bps: 10_000 });
    expect(preview).toHaveBeenCalledWith('wallet', undefined);
  });

  it('fails clearly when read metadata is missing', () => {
    const preview = jest.fn(async () => null);
    const reads = buildReadHookInterfaces({ preview }, {} as never);

    expect(() => reads.preview.use()).toThrow(
      'Missing read argument count metadata for "preview"',
    );
  });

  it('keeps read options while the client is connecting', () => {
    const reads = buildReadHookInterfaces(null, { anything: 1 });

    const { result } = renderHook(() => reads.anything.use(
      'wallet',
      { initialData: 'seed', debounceMs: 300 },
    ));

    expect(result).toMatchObject({
      data: 'seed',
      status: 'connecting',
      isLoading: false,
    });
  });

  it('records the read result in hook state', async () => {
    const solClaimPreview = jest.fn(async () => ({ total: 5n }));
    const reads = buildReadHookInterfaces({ solClaimPreview }, { solClaimPreview: 1 });

    const { result, state } = renderHook(() => reads.solClaimPreview.use('wallet'));
    await result.refresh();

    expect(state()).toMatchObject({ data: { total: 5n }, isLoading: false, error: null });
  });
});
