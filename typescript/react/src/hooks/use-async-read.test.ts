jest.mock('react', () => ({
  useState: jest.fn(),
  useRef: jest.fn((value: unknown) => ({ current: value })),
  useCallback: jest.fn((callback: unknown) => callback),
  useEffect: jest.fn(),
}));

import { useEffect, useRef, useState } from 'react';

import { useAsyncRead, type AsyncReadContext } from './use-async-read';

const mockUseState = useState as jest.Mock;
const mockUseRef = useRef as jest.Mock;
const mockUseEffect = useEffect as jest.Mock;

function renderRead<T>(
  read: (context: AsyncReadContext) => Promise<T>,
  options?: { enabled?: boolean; initialData?: T }
) {
  let currentState: Record<string, unknown> = {};
  const cleanups: Array<() => void> = [];
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
    const cleanup = effect();
    if (cleanup) cleanups.push(cleanup);
  });

  return {
    read: useAsyncRead('stable-key', read, options),
    state: () => currentState,
    unmount: () => cleanups.reverse().forEach((cleanup) => cleanup()),
  };
}

describe('useAsyncRead', () => {
  beforeEach(() => {
    mockUseState.mockReset();
    mockUseRef.mockReset();
    mockUseRef.mockImplementation((value: unknown) => ({ current: value }));
    mockUseEffect.mockReset();
  });

  it('suppresses stale results and aborts the superseded request', async () => {
    const requests: Array<{
      context: AsyncReadContext;
      resolve: (value: string) => void;
    }> = [];
    const execute = jest.fn((context: AsyncReadContext) => new Promise<string>((resolve) => {
      requests.push({ context, resolve });
    }));
    const rendered = renderRead(execute);

    expect(requests).toHaveLength(1);
    const refresh = rendered.read.refresh();
    expect(requests).toHaveLength(2);
    expect(requests[0]!.context.signal.aborted).toBe(true);

    requests[1]!.resolve('newer');
    await expect(refresh).resolves.toBe('newer');
    requests[0]!.resolve('older');
    await Promise.resolve();

    expect(rendered.state()).toMatchObject({
      data: 'newer',
      error: null,
      isLoading: false,
      isRefreshing: false,
    });
  });

  it('distinguishes initial loading from refreshing existing data', () => {
    const pending = () => new Promise<string>(() => undefined);
    const loading = renderRead(pending);
    expect(loading.state()).toMatchObject({ isLoading: true, isRefreshing: false });

    mockUseState.mockReset();
    mockUseRef.mockReset();
    mockUseRef.mockImplementation((value: unknown) => ({ current: value }));
    const refreshing = renderRead(pending, { initialData: 'cached' });
    expect(refreshing.state()).toMatchObject({
      data: 'cached',
      isLoading: false,
      isRefreshing: true,
    });
  });

  it('does not run while disabled and aborts an active read on unmount', async () => {
    const disabledRead = jest.fn(async () => 'unused');
    const disabled = renderRead(disabledRead, { enabled: false });
    await expect(disabled.read.refresh()).resolves.toBeUndefined();
    expect(disabledRead).not.toHaveBeenCalled();
    expect(disabled.state()).toMatchObject({ isLoading: false, isRefreshing: false });

    mockUseState.mockReset();
    mockUseRef.mockReset();
    mockUseRef.mockImplementation((value: unknown) => ({ current: value }));
    let signal: AbortSignal | undefined;
    const active = renderRead(async (context) => {
      signal = context.signal;
      return new Promise<string>(() => undefined);
    });
    active.unmount();
    expect(signal?.aborted).toBe(true);
  });
});
