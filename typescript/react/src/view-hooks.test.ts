import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import {
  FrameProcessor,
  MemoryAdapter,
  QueryStore,
  SubscriptionRegistry,
  parseFrame,
  type Subscription,
} from '@usearete/sdk';

import { createListViewHook, createStateViewHook, useListView, useStateView } from './view-hooks';
import type { ViewHookResult } from './types';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const originalConsoleError = console.error;
beforeAll(() => {
  jest.spyOn(console, 'error').mockImplementation((message: unknown, ...args: unknown[]) => {
    if (typeof message === 'string' && message.startsWith('react-test-renderer is deprecated')) return;
    originalConsoleError(message, ...args);
  });
});
afterAll(() => jest.restoreAllMocks());

function createClient() {
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
  const process = (frame: unknown) => {
    processor.handleFrame(parseFrame(JSON.stringify(frame)));
  };
  const active = (predicate: (subscription: Subscription) => boolean) => {
    const match = connection.subscribe.mock.calls
      .map(([value]) => value as Subscription)
      .find(predicate);
    if (!match) throw new Error('Expected active subscription');
    return match;
  };
  return { active, client, connection, process, queries, registry };
}

function snapshot(
  subscription: Subscription,
  keys: string[],
  snapshotId = 'snapshot',
  authoritative = true
) {
  return {
    protocolVersion: 2,
    subscriptionId: subscription.subscriptionId,
    snapshotId,
    authoritative,
    mode: subscription.query.key ? 'state' : 'list',
    entity: subscription.query.view,
    op: 'snapshot',
    ...(subscription.query.key ? { key: subscription.query.key } : {}),
    data: keys.map((key) => ({ key, data: { id: key } })),
    complete: true,
  };
}

describe('protocol v2 view hooks', () => {
  it('keeps simultaneous useOne, take, and filter queries isolated', () => {
    const { active, client, connection, process } = createClient();
    let page: ViewHookResult<Array<{ id: string }>> | undefined;
    let open: ViewHookResult<Array<{ id: string }>> | undefined;
    let one: ViewHookResult<{ id: string }> | undefined;
    let renderer: ReactTestRenderer | undefined;
    const hook = createListViewHook<{ id: string }>(
      { mode: 'list', view: 'Order/list' },
      client as never
    );
    function Harness() {
      page = useListView(
        { mode: 'list', view: 'Order/list' },
        client as never,
        { take: 2, skip: 2 }
      );
      open = useListView(
        { mode: 'list', view: 'Order/list' },
        client as never,
        { filters: { 'state.status': 'open' }, take: 10 }
      );
      one = hook.useOne(undefined, { initialData: { id: 'seed' } });
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });

    expect(connection.subscribe).toHaveBeenCalledTimes(3);
    expect(one).toMatchObject({
      data: { id: 'seed' },
      status: 'ready',
      isReady: true,
      error: undefined,
    });
    const pageSubscription = active((value) => value.query.skip === 2);
    const openSubscription = active((value) => value.query.filters?.['state.status'] === 'open');
    const oneSubscription = active((value) => value.query.take === 1);
    act(() => {
      process(snapshot(pageSubscription, ['4', '3'], 'page'));
      process(snapshot(openSubscription, ['7', '5'], 'open'));
      process(snapshot(oneSubscription, ['9'], 'one'));
    });

    expect(page?.data).toEqual([{ id: '4' }, { id: '3' }]);
    expect(open?.data).toEqual([{ id: '7' }, { id: '5' }]);
    expect(one?.data).toEqual({ id: '9' });
    act(() => renderer?.unmount());
  });

  it('keeps disabled result variants internally consistent', () => {
    let state: ViewHookResult<{ id: string }> | undefined;
    let list: ViewHookResult<Array<{ id: string }>> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      state = useStateView(
        { mode: 'state', view: 'Order/state' },
        null,
        undefined,
        { initialData: { id: 'unkeyed-seed' } },
      );
      list = useListView(
        { mode: 'list', view: 'Order/list' },
        null,
        undefined,
        { enabled: false, initialData: [{ id: 'list-seed' }] },
      );
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });

    expect(state).toMatchObject({
      data: undefined,
      status: 'disabled',
      isLoading: false,
      isRefreshing: false,
      error: undefined,
    });
    expect(list).toMatchObject({
      data: [{ id: 'list-seed' }],
      status: 'disabled',
      isLoading: false,
      isRefreshing: false,
      error: undefined,
    });
    act(() => renderer?.unmount());
  });

  it('honors list query fields in options while giving params precedence', () => {
    const { active, client, process } = createClient();
    let optionsOnly: ViewHookResult<Array<{ id: string }>> | undefined;
    let paramsWin: ViewHookResult<Array<{ id: string }>> | undefined;
    let renderer: ReactTestRenderer | undefined;
    const optionsSchema = {
      safeParse: (value: unknown) => ({
        success: true as const,
        data: { id: `options:${(value as { id: string }).id}` },
      }),
    };
    const paramsSchema = {
      safeParse: (value: unknown) => ({
        success: true as const,
        data: { id: `params:${(value as { id: string }).id}` },
      }),
    };
    function Harness() {
      optionsOnly = useListView(
        { mode: 'list', view: 'Order/list' },
        client as never,
        undefined,
        {
          partition: 'options',
          filters: { source: 'options' },
          after: 'options-after',
          snapshotLimit: 3,
          withSnapshot: false,
          schema: optionsSchema,
        },
      );
      paramsWin = useListView(
        { mode: 'list', view: 'Order/list' },
        client as never,
        {
          partition: 'params',
          filters: { source: 'params' },
          after: 'params-after',
          snapshotLimit: 2,
          withSnapshot: true,
          schema: paramsSchema,
        },
        {
          partition: 'ignored',
          filters: { source: 'ignored' },
          after: 'ignored-after',
          snapshotLimit: 99,
          withSnapshot: false,
          schema: optionsSchema,
        },
      );
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });

    const fromOptions = active((value) => value.query.partition === 'options');
    const fromParams = active((value) => value.query.partition === 'params');
    expect(fromOptions).toMatchObject({
      query: {
        filters: { source: 'options' },
        after: 'options-after',
        snapshotLimit: 3,
      },
      snapshot: { enabled: false },
    });
    expect(fromParams).toMatchObject({
      query: {
        filters: { source: 'params' },
        after: 'params-after',
        snapshotLimit: 2,
      },
      snapshot: { enabled: true },
    });
    act(() => {
      process(snapshot(fromOptions, ['1'], 'options'));
      process(snapshot(fromParams, ['2'], 'params'));
    });
    expect(optionsOnly?.data).toEqual([{ id: 'options:1' }]);
    expect(paramsWin?.data).toEqual([{ id: 'params:2' }]);
    act(() => renderer?.unmount());
  });

  it('reports caller schema rejections without failing accepted view data', () => {
    const { active, client, process } = createClient();
    const rejected = new Error('invalid entity');
    const onSchemaValidationError = jest.fn();
    let result: ViewHookResult<Array<{ id: string }>> | undefined;
    let renderer: ReactTestRenderer | undefined;
    const schema = {
      safeParse: (value: unknown) => {
        const entity = value as { id: string };
        return entity.id === '2'
          ? { success: false as const, error: rejected }
          : { success: true as const, data: { id: `parsed:${entity.id}` } };
      },
    };
    function Harness() {
      result = useListView(
        { mode: 'list', view: 'Order/list' },
        client as never,
        { schema, onSchemaValidationError },
      );
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });
    const subscription = active(() => true);

    act(() => {
      process(snapshot(subscription, ['1', '2'], 'schema-validation'));
    });

    expect(result?.data).toEqual([{ id: 'parsed:1' }]);
    expect(result?.error).toBeUndefined();
    expect(onSchemaValidationError).toHaveBeenCalledWith({
      view: 'Order/list',
      key: '2',
      entity: { id: '2' },
      error: rejected,
    });
    act(() => renderer?.unmount());
  });

  it('refcounts equivalent normalized hook queries', () => {
    const { client, connection } = createClient();
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      useListView(
        { mode: 'list', view: 'Order/list' },
        client as never,
        { filters: { owner: 'wallet', 'state.status': 'open' } }
      );
      useListView(
        { mode: 'list', view: 'Order/list' },
        client as never,
        { filters: { 'state.status': 'open', owner: 'wallet' } }
      );
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });
    expect(connection.subscribe).toHaveBeenCalledTimes(1);
    act(() => renderer?.unmount());
    expect(connection.unsubscribe).toHaveBeenCalledTimes(1);
  });

  it('distinguishes initial loading from reconnect refresh while retaining old data', () => {
    const { active, client, process, registry } = createClient();
    let result: ViewHookResult<Array<{ id: string }>> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      result = useListView({ mode: 'list', view: 'Round/list' }, client as never);
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });
    expect(result).toMatchObject({ data: [], isLoading: true, isRefreshing: false });
    const subscription = active(() => true);

    act(() => process(snapshot(subscription, ['10', '9'], 'before')));
    expect(result).toMatchObject({
      data: [{ id: '10' }, { id: '9' }],
      isLoading: false,
      isRefreshing: false,
    });

    act(() => registry.handleConnectionState('reconnecting'));
    expect(result).toMatchObject({
      data: [{ id: '10' }, { id: '9' }],
      isLoading: false,
      isRefreshing: true,
    });
    act(() => process(snapshot(subscription, ['11', '10'], 'after')));
    expect(result).toMatchObject({
      data: [{ id: '11' }, { id: '10' }],
      isRefreshing: false,
    });
    act(() => renderer?.unmount());
  });

  it('surfaces server errors only on the identified query', () => {
    const { active, client, process } = createClient();
    let first: ViewHookResult<Array<{ id: string }>> | undefined;
    let second: ViewHookResult<Array<{ id: string }>> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      first = useListView(
        { mode: 'list', view: 'Thing/list' },
        client as never,
        { take: 1 }
      );
      second = useListView(
        { mode: 'list', view: 'Thing/list' },
        client as never,
        { take: 2 }
      );
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });
    const rejected = active((value) => value.query.take === 1);
    act(() => process({
      type: 'error',
      protocolVersion: 2,
      subscriptionId: rejected.subscriptionId,
      error: 'subscription-rejected',
      message: 'query rejected',
      code: 'subscription-rejected',
      retryable: false,
      fatal: false,
    }));

    expect(first?.error?.message).toBe('query rejected');
    expect(first?.isLoading).toBe(false);
    expect(second?.error).toBeUndefined();
    expect(second?.isLoading).toBe(true);
    act(() => renderer?.unmount());
  });

  it('preserves typed state keys and refreshes the exact lease', async () => {
    const { active, client, connection, process } = createClient();
    let result: ViewHookResult<{ id: string }> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      result = useStateView(
        { mode: 'state', view: 'Miner/state', keyFields: ['authority'] },
        client as never,
        { authority: 'wallet' }
      );
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });
    const subscription = active(() => true);
    expect(subscription.query.key).toBe('wallet');
    act(() => process(snapshot(subscription, ['wallet'])));
    let refresh!: Promise<void>;
    act(() => { refresh = result!.refresh(); });
    expect(connection.refresh).toHaveBeenCalledWith(subscription);
    expect(result).toMatchObject({ data: { id: 'wallet' }, isRefreshing: true });
    act(() => process(snapshot(subscription, ['wallet'], 'refreshed')));
    await act(async () => { await refresh; });
    act(() => renderer?.unmount());
  });

  it('rejects refresh send failures and exposes them on the active query', async () => {
    const { active, client, connection, process } = createClient();
    let result: ViewHookResult<{ id: string }> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      result = useStateView(
        { mode: 'state', view: 'Miner/state', keyFields: ['authority'] },
        client as never,
        { authority: 'wallet' },
      );
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });
    const subscription = active(() => true);
    act(() => process(snapshot(subscription, ['wallet'])));
    connection.refresh.mockImplementationOnce(() => {
      throw new Error('refresh send failed');
    });

    let refresh!: Promise<void>;
    act(() => { refresh = result!.refresh(); });
    await expect(refresh).rejects.toThrow('refresh send failed');
    expect(result).toMatchObject({
      data: { id: 'wallet' },
      isRefreshing: false,
      error: expect.objectContaining({ message: 'refresh send failed' }),
    });
    act(() => renderer?.unmount());
  });

  it('does not lease disabled state queries', () => {
    const { client, connection } = createClient();
    let result: ViewHookResult<{ id: string }> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      result = useStateView(
        { mode: 'state', view: 'Miner/state', keyFields: ['authority'] },
        client as never,
        undefined,
        { enabled: false }
      );
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });
    expect(connection.subscribe).not.toHaveBeenCalled();
    expect(result).toMatchObject({ data: undefined, isLoading: false, isRefreshing: false });
    act(() => renderer?.unmount());
  });

  it('reports lifecycle status from disabled through subscribing to ready', () => {
    const { active, client, process } = createClient();
    let result: ViewHookResult<{ id: string }> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness(props: { authority?: string }) {
      result = useStateView(
        { mode: 'state', view: 'Miner/state', keyFields: ['authority'] },
        client as never,
        props.authority ? { authority: props.authority } : undefined
      );
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness, {})); });
    expect(result?.status).toBe('disabled');

    act(() => renderer?.update(React.createElement(Harness, { authority: 'wallet' })));
    expect(result?.status).toBe('subscribing');

    const subscription = active(() => true);
    act(() => process(snapshot(subscription, ['wallet'])));
    expect(result?.status).toBe('ready');
    act(() => renderer?.unmount());
  });

  it('reports connecting while the client is absent', () => {
    let result: ViewHookResult<{ id: string }> | undefined;
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      result = useStateView(
        { mode: 'state', view: 'Miner/state', keyFields: ['authority'] },
        null,
        { authority: 'wallet' }
      );
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });
    expect(result).toMatchObject({ status: 'connecting', isLoading: true });
    act(() => renderer?.unmount());
  });

  it('refreshes active view subscriptions through the hook factory', async () => {
    const { active, client, connection, process } = createClient();
    const hook = createStateViewHook<{ id: string }, { authority: string }>(
      { mode: 'state', view: 'Miner/state', keyFields: ['authority'] },
      client as never
    );
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      hook.use({ authority: 'wallet' });
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });
    const subscription = active(() => true);
    act(() => process(snapshot(subscription, ['wallet'])));

    let refresh!: Promise<void>;
    act(() => { refresh = hook.refresh({ authority: 'wallet' }); });
    expect(connection.refresh).toHaveBeenCalledWith(subscription);
    act(() => process(snapshot(subscription, ['wallet'], 'refreshed')));
    await act(async () => { await refresh; });

    await act(async () => { await hook.refresh({ authority: 'other' }); });
    expect(connection.refresh).toHaveBeenCalledTimes(1);
    act(() => renderer?.unmount());
  });

  it('refresh without a key refreshes every active subscription of the view', async () => {
    const { client, connection, process } = createClient();
    const hook = createStateViewHook<{ id: string }, { authority: string }>(
      { mode: 'state', view: 'Miner/state', keyFields: ['authority'] },
      client as never
    );
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      hook.use({ authority: 'first' });
      hook.use({ authority: 'second' });
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });

    let refresh!: Promise<void>;
    act(() => { refresh = hook.refresh(); });
    expect(connection.refresh).toHaveBeenCalledTimes(2);
    act(() => {
      for (const [subscription] of connection.refresh.mock.calls) {
        process(snapshot(subscription, [subscription.query.key], `refreshed-${subscription.query.key}`));
      }
    });
    await act(async () => { await refresh; });
    act(() => renderer?.unmount());
  });

  it('list hook factories refresh all active subscriptions of the view', async () => {
    const { client, connection, process } = createClient();
    const hook = createListViewHook<{ id: string }>(
      { mode: 'list', view: 'Round/latest' },
      client as never
    );
    let renderer: ReactTestRenderer | undefined;
    function Harness() {
      hook.use({ take: 8 });
      return null;
    }
    act(() => { renderer = create(React.createElement(Harness)); });

    let refresh!: Promise<void>;
    act(() => { refresh = hook.refresh(); });
    expect(connection.refresh).toHaveBeenCalledTimes(1);
    const subscription = connection.refresh.mock.calls[0]?.[0];
    act(() => process(snapshot(subscription, ['round'], 'refreshed')));
    await act(async () => { await refresh; });
    act(() => renderer?.unmount());
  });

  it('hook factory refresh resolves without a client', async () => {
    const stateHook = createStateViewHook<{ id: string }, { authority: string }>(
      { mode: 'state', view: 'Miner/state', keyFields: ['authority'] },
      null
    );
    const listHook = createListViewHook<{ id: string }>(
      { mode: 'list', view: 'Round/latest' },
      null
    );
    await expect(stateHook.refresh({ authority: 'wallet' })).resolves.toBeUndefined();
    await expect(stateHook.refresh()).resolves.toBeUndefined();
    await expect(listHook.refresh()).resolves.toBeUndefined();
  });
});
