jest.mock('react', () => ({
  useState: jest.fn(),
  useEffect: jest.fn((effect: () => void | (() => void)) => effect()),
  useCallback: jest.fn((callback: unknown) => callback),
  useSyncExternalStore: jest.fn(
    (subscribe: (callback: () => void) => () => void, getSnapshot: () => unknown) => {
      subscribe(() => {});
      return getSnapshot();
    }
  ),
  useRef: jest.fn((value: unknown) => ({ current: value })),
}));

import { useState } from 'react';

import { useListView, useStateView } from './view-hooks';

const mockUseState = useState as jest.Mock;

function createHookState() {
  const setIsLoading = jest.fn();
  const setError = jest.fn();
  mockUseState
    .mockImplementationOnce((value: unknown) => [value, setIsLoading])
    .mockImplementationOnce((value: unknown) => [value, setError]);
  return { setIsLoading, setError };
}

function createClient() {
  let frameHandler: ((frame: never) => void) | undefined;
  const registry = {
    subscribe: jest.fn(() => jest.fn()),
    refresh: jest.fn(),
  };
  const client = {
    getSubscriptionRegistry: jest.fn(() => registry),
    onFrame: jest.fn((handler: (frame: never) => void) => {
      frameHandler = handler;
      return jest.fn();
    }),
    store: {
      onUpdate: jest.fn(() => jest.fn()),
      getSync: jest.fn(() => undefined),
      getAllSync: jest.fn(() => undefined),
      getAll: jest.fn(() => []),
    },
  };

  return {
    client,
    registry,
    emitFrame(frame: unknown) {
      frameHandler?.(frame as never);
    },
  };
}

describe('view hook refresh', () => {
  beforeEach(() => {
    mockUseState.mockReset();
  });

  it('refreshes a state view through the registry without changing its refcount', () => {
    const { setIsLoading } = createHookState();
    const { client, registry } = createClient();
    const timeoutSpy = jest.spyOn(globalThis, 'setTimeout');

    const result = useStateView(
      { mode: 'state', view: 'OreMiner/state' },
      client as never,
      { authority: 'wallet' },
      { snapshotLimit: 1 }
    );
    setIsLoading.mockClear();
    result.refresh();

    expect(registry.refresh).toHaveBeenCalledWith({
      view: 'OreMiner/state',
      key: 'wallet',
      withSnapshot: undefined,
      after: undefined,
      snapshotLimit: 1,
    });
    expect(registry.subscribe).toHaveBeenCalledTimes(1);
    expect(setIsLoading).toHaveBeenCalledWith(true);
    expect(timeoutSpy).not.toHaveBeenCalled();
    timeoutSpy.mockRestore();
  });

  it('refreshes a list view with the exact active options', () => {
    createHookState();
    const { client, registry } = createClient();

    const result = useListView(
      { mode: 'list', view: 'Position/list' },
      client as never,
      {
        filters: { status: 'open', owner: 'wallet' },
        take: 10,
        skip: 2,
        after: '20:1',
      }
    );
    result.refresh();

    expect(registry.refresh).toHaveBeenCalledWith({
      view: 'Position/list',
      key: undefined,
      filters: { status: 'open', owner: 'wallet' },
      take: 10,
      skip: 2,
      withSnapshot: undefined,
      after: '20:1',
      snapshotLimit: undefined,
    });
    expect(registry.subscribe).toHaveBeenCalledTimes(1);
  });

  it('reports refresh failures and stops loading', () => {
    const { setIsLoading, setError } = createHookState();
    const { client, registry } = createClient();
    registry.refresh.mockImplementation(() => {
      throw new Error('inactive subscription');
    });

    const result = useStateView(
      { mode: 'state', view: 'OreMiner/state' },
      client as never,
      { authority: 'wallet' }
    );
    setIsLoading.mockClear();
    setError.mockClear();
    result.refresh();

    expect(setError).toHaveBeenCalledWith(expect.objectContaining({
      message: 'inactive subscription',
    }));
    expect(setIsLoading).toHaveBeenCalledWith(false);
  });

  it('completes keyed absent-state loading only for the exact wire identity', () => {
    const { setIsLoading } = createHookState();
    const { client, emitFrame } = createClient();

    useStateView(
      { mode: 'state', view: 'OreMiner/state' },
      client as never,
      { authority: 'wallet' }
    );
    setIsLoading.mockClear();

    emitFrame({
      mode: 'state',
      entity: 'OreMiner/state',
      op: 'snapshot',
      key: 'another-wallet',
      data: [],
      complete: true,
    });

    expect(setIsLoading).not.toHaveBeenCalled();

    emitFrame({
      mode: 'state',
      entity: 'OreMiner/state',
      op: 'snapshot',
      key: 'wallet',
      data: [],
      complete: true,
    });

    expect(setIsLoading).toHaveBeenCalledWith(false);
  });

  it('completes keyless list loading from a keyless empty snapshot', () => {
    const { setIsLoading } = createHookState();
    const { client, emitFrame } = createClient();

    useListView({ mode: 'list', view: 'Automation/list' }, client as never);
    setIsLoading.mockClear();

    emitFrame({
      mode: 'list',
      entity: 'Automation/list',
      op: 'snapshot',
      data: [],
      complete: true,
    });

    expect(setIsLoading).toHaveBeenCalledWith(false);
  });
});
