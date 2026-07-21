import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';

import type { SocketIssue } from '@usearete/sdk';
import { createClientCacheKey } from './client-key';

interface MockClientChange {
  cacheKey: string;
  status: 'connecting' | 'connected' | 'error';
  error?: Error;
}

const mockAreteContext = {
  getClient: jest.fn(),
  getOrCreateClient: jest.fn(),
  retryClient: jest.fn(),
  subscribeToClientChanges: jest.fn(),
  config: {} as import('./types').AreteConfig,
};
const clientChangeListeners = new Set<(change?: MockClientChange) => void>();

jest.mock('./provider', () => ({
  useAreteContext: () => mockAreteContext,
}));

import { useArete, type UseAreteResult } from './stack';

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

const stack = {
  name: 'extended-stack',
  endpoints: { ws: 'wss://example.invalid', http: 'https://example.invalid' },
  views: {},
  addresses: { board: () => 'board-address' },
  constants: { tileCount: 25 },
  math: { double: (value: number) => value * 2 },
} as const;
const stackCacheKey = createClientCacheKey(stack, {
  url: undefined,
  httpUrl: undefined,
  transport: undefined,
  programs: undefined,
})!;

function renderArete() {
  let current: UseAreteResult<typeof stack, undefined> | undefined;
  let renderer: ReactTestRenderer | undefined;
  function Harness() {
    current = useArete(stack);
    return null;
  }
  act(() => {
    renderer = create(React.createElement(Harness));
  });
  return {
    result: () => current!,
    unmount: () => act(() => renderer?.unmount()),
  };
}

describe('useArete lifecycle surface', () => {
  beforeEach(() => {
    mockAreteContext.getClient.mockReset();
    mockAreteContext.getOrCreateClient.mockReset();
    mockAreteContext.retryClient.mockReset();
    mockAreteContext.subscribeToClientChanges.mockReset();
    mockAreteContext.config = {};
    clientChangeListeners.clear();
    mockAreteContext.subscribeToClientChanges.mockImplementation((listener) => {
      clientChangeListeners.add(listener);
      return () => clientChangeListeners.delete(listener);
    });
  });

  it('keeps connected fields nullable and exposes static stack extensions', () => {
    mockAreteContext.getClient.mockReturnValue(null);
    mockAreteContext.getOrCreateClient.mockReturnValue(new Promise(() => undefined));
    const rendered = renderArete();

    expect(rendered.result().client).toBeNull();
    expect(rendered.result().chain).toBeNull();
    expect(rendered.result().zustandStore).toBeNull();
    expect(rendered.result().read).toBeDefined();
    expect(rendered.result().reads).toBeDefined();
    expect(rendered.result().addresses.board()).toBe('board-address');
    expect(rendered.result().constants.tileCount).toBe(25);
    expect(rendered.result().math.double(3)).toBe(6);
    rendered.unmount();
  });

  it('reports an initial connection failure through status', async () => {
    const failure = new Error('initial connection failed');
    mockAreteContext.getClient.mockReturnValue(null);
    mockAreteContext.getOrCreateClient.mockRejectedValue(failure);
    const rendered = renderArete();

    await act(async () => {
      await Promise.resolve();
    });

    expect(rendered.result()).toMatchObject({
      client: null,
      status: 'error',
      isLoading: false,
      canRetry: true,
      error: failure,
    });
    rendered.unmount();
  });

  it('does not advertise provider retry when autoConnect is disabled', () => {
    const client = {
      connectionState: 'disconnected' as const,
      onConnectionStateChange: jest.fn(() => jest.fn()),
      onSocketIssue: jest.fn(() => jest.fn()),
      programs: undefined,
      queries: {},
      chain: {},
      store: {},
    };
    mockAreteContext.config = { autoConnect: false };
    mockAreteContext.getClient.mockReturnValue(client);
    mockAreteContext.getOrCreateClient.mockResolvedValue(client);
    const rendered = renderArete();

    expect(rendered.result()).toMatchObject({
      status: 'disconnected',
      canRetry: false,
    });
    rendered.unmount();
  });

  it('surfaces post-connect errors and socket issues and deduplicates retry', async () => {
    let stateHandler: ((state: 'connected' | 'error', error?: string) => void) | undefined;
    let issueHandler: ((issue: SocketIssue) => void) | undefined;
    let resolveRetry: (() => void) | undefined;
    const retryAttempt = new Promise<void>((resolve) => {
      resolveRetry = resolve;
    });
    const client = {
      connectionState: 'connected' as const,
      onConnectionStateChange: jest.fn((handler) => {
        stateHandler = handler;
        return jest.fn();
      }),
      onSocketIssue: jest.fn((handler) => {
        issueHandler = handler;
        return jest.fn();
      }),
      programs: undefined,
      queries: {},
      chain: {},
      store: {},
    };
    mockAreteContext.getClient.mockReturnValue(client);
    mockAreteContext.getOrCreateClient.mockResolvedValue(client);
    mockAreteContext.retryClient.mockReturnValue(retryAttempt.then(() => client));
    const rendered = renderArete();

    act(() => stateHandler?.('error', 'socket closed'));
    expect(rendered.result().error?.message).toBe('socket closed');

    const issue: SocketIssue = {
      error: 'subscription limit',
      message: 'too many subscriptions',
      code: 'SUBSCRIPTION_LIMIT_EXCEEDED',
      retryable: true,
      fatal: false,
    };
    act(() => issueHandler?.(issue));
    expect(rendered.result().socketIssue).toBe(issue);

    let firstRetry!: Promise<void>;
    let secondRetry!: Promise<void>;
    act(() => {
      firstRetry = rendered.result().retry();
      secondRetry = rendered.result().retry();
    });
    expect(secondRetry).toBe(firstRetry);
    expect(mockAreteContext.retryClient).toHaveBeenCalledTimes(1);
    expect(rendered.result().error).toBeNull();
    expect(rendered.result().socketIssue).toBeNull();

    await act(async () => {
      resolveRetry?.();
      await firstRetry;
    });
    rendered.unmount();
  });

  it('adopts one provider replacement across sibling consumers', async () => {
    const makeClient = () => ({
      connectionState: 'connected' as const,
      onConnectionStateChange: jest.fn(() => jest.fn()),
      onSocketIssue: jest.fn(() => jest.fn()),
      programs: undefined,
      queries: {},
      chain: {},
      store: {},
    });
    const original = makeClient();
    const replacement = makeClient();
    let cachedClient: ReturnType<typeof makeClient> | null = original;
    let finishReplacement: (() => void) | undefined;
    const replacementReady = new Promise<void>((resolve) => {
      finishReplacement = resolve;
    });
    mockAreteContext.getClient.mockImplementation(() => cachedClient);
    mockAreteContext.getOrCreateClient.mockImplementation(async () => cachedClient ?? original);
    mockAreteContext.retryClient.mockImplementation(() => {
      cachedClient = null;
      clientChangeListeners.forEach((listener) => listener({
        cacheKey: stackCacheKey,
        status: 'connecting',
      }));
      return replacementReady.then(() => {
        cachedClient = replacement;
        clientChangeListeners.forEach((listener) => listener({
          cacheKey: stackCacheKey,
          status: 'connected',
        }));
        return replacement;
      });
    });

    let first: UseAreteResult<typeof stack, undefined> | undefined;
    let second: UseAreteResult<typeof stack, undefined> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      first = useArete(stack);
      second = useArete(stack);
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });
    expect(first?.client).toBe(original);
    expect(second?.client).toBe(original);

    let retry!: Promise<void>;
    act(() => { retry = first!.retry(); });
    expect(first).toMatchObject({ client: null, status: 'connecting' });
    expect(second).toMatchObject({ client: null, status: 'connecting' });

    await act(async () => {
      finishReplacement?.();
      await retry;
    });
    expect(first?.client).toBe(replacement);
    expect(second?.client).toBe(replacement);
    expect(mockAreteContext.retryClient).toHaveBeenCalledTimes(1);
    act(() => renderer?.unmount());
  });

  it('shares a rejected replacement outcome across sibling consumers', async () => {
    const client = {
      connectionState: 'connected' as const,
      onConnectionStateChange: jest.fn(() => jest.fn()),
      onSocketIssue: jest.fn(() => jest.fn()),
      programs: undefined,
      queries: {},
      chain: {},
      store: {},
    };
    let cachedClient: typeof client | null = client;
    let rejectReplacement: ((error: Error) => void) | undefined;
    const replacement = new Promise<typeof client>((_resolve, reject) => {
      rejectReplacement = reject;
    });
    mockAreteContext.getClient.mockImplementation(() => cachedClient);
    mockAreteContext.getOrCreateClient.mockImplementation(async () => cachedClient ?? client);
    mockAreteContext.retryClient.mockImplementation(() => {
      cachedClient = null;
      clientChangeListeners.forEach((listener) => listener({
        cacheKey: stackCacheKey,
        status: 'connecting',
      }));
      return replacement.catch((error: Error) => {
        clientChangeListeners.forEach((listener) => listener({
          cacheKey: stackCacheKey,
          status: 'error',
          error,
        }));
        throw error;
      });
    });

    let first: UseAreteResult<typeof stack, undefined> | undefined;
    let second: UseAreteResult<typeof stack, undefined> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      first = useArete(stack);
      second = useArete(stack);
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });

    let retry!: Promise<void>;
    act(() => { retry = first!.retry(); });
    expect(first).toMatchObject({ client: null, status: 'connecting', isLoading: true });
    expect(second).toMatchObject({ client: null, status: 'connecting', isLoading: true });

    const failure = new Error('replacement connection failed');
    await act(async () => {
      rejectReplacement?.(failure);
      await expect(retry).rejects.toBe(failure);
    });
    expect(first).toMatchObject({
      client: null,
      status: 'error',
      isLoading: false,
      canRetry: true,
      error: failure,
    });
    expect(second).toMatchObject({
      client: null,
      status: 'error',
      isLoading: false,
      canRetry: true,
      error: failure,
    });
    act(() => renderer?.unmount());
  });

  it('resolves the default stack and stack options from the provider config', () => {
    mockAreteContext.config = {
      stack,
      stackOptions: { url: 'wss://override.invalid' },
    };
    mockAreteContext.getClient.mockReturnValue(null);
    mockAreteContext.getOrCreateClient.mockReturnValue(new Promise(() => undefined));

    let current: UseAreteResult<typeof stack, undefined> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      current = useArete() as UseAreteResult<typeof stack, undefined>;
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });

    expect(mockAreteContext.getOrCreateClient).toHaveBeenCalledWith(
      stack,
      expect.objectContaining({ url: 'wss://override.invalid' })
    );
    expect(current?.addresses.board()).toBe('board-address');
    act(() => renderer?.unmount());
  });

  it('applies provider stack options when the provider stack is passed explicitly', () => {
    mockAreteContext.config = {
      stack,
      stackOptions: {
        url: 'ws://localhost:8878',
        httpUrl: 'http://localhost:8081',
      },
    };
    mockAreteContext.getClient.mockReturnValue(null);
    mockAreteContext.getOrCreateClient.mockReturnValue(new Promise(() => undefined));

    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      useArete(stack);
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });

    expect(mockAreteContext.getOrCreateClient).toHaveBeenCalledWith(
      stack,
      expect.objectContaining({
        url: 'ws://localhost:8878',
        httpUrl: 'http://localhost:8081',
      })
    );
    act(() => renderer?.unmount());
  });

  it('lets an explicit stack and options win over the provider defaults', () => {
    mockAreteContext.config = {
      stack,
      stackOptions: { url: 'wss://override.invalid' },
    };
    mockAreteContext.getClient.mockReturnValue(null);
    mockAreteContext.getOrCreateClient.mockReturnValue(new Promise(() => undefined));
    const otherStack = { ...stack, name: 'other-stack' };

    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      useArete(otherStack, { url: 'wss://explicit.invalid' });
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });

    expect(mockAreteContext.getOrCreateClient).toHaveBeenCalledWith(
      otherStack,
      expect.objectContaining({ url: 'wss://explicit.invalid' })
    );
    act(() => renderer?.unmount());
  });

  it('does not apply provider stack options to another explicit stack', () => {
    mockAreteContext.config = {
      stack,
      stackOptions: { url: 'wss://override.invalid' },
    };
    mockAreteContext.getClient.mockReturnValue(null);
    mockAreteContext.getOrCreateClient.mockReturnValue(new Promise(() => undefined));
    const otherStack = { ...stack, name: 'other-stack' };

    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      useArete(otherStack);
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });

    expect(mockAreteContext.getOrCreateClient).toHaveBeenCalledWith(
      otherStack,
      expect.objectContaining({ url: undefined, httpUrl: undefined })
    );
    act(() => renderer?.unmount());
  });

  it('throws a clear error when no stack is passed and no default is configured', () => {
    mockAreteContext.config = {};
    mockAreteContext.getClient.mockReturnValue(null);
    const consoleError = jest.spyOn(console, 'error').mockImplementation(() => undefined);

    function Harness() {
      useArete();
      return null;
    }
    expect(() => {
      act(() => { create(React.createElement(Harness)); });
    }).toThrow(/no default stack is configured/);
    consoleError.mockRestore();
  });
});
