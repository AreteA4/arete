import { describe, expect, it } from 'vitest';

import { MemoryAdapter } from './storage/memory-adapter';
import { SubscriptionRegistry } from './subscription';
import type { ConnectionManager } from './connection';
import { QueryStore } from './query-store';
import { AreteError } from './types';
import type { Subscription, TypedViewGroup, ViewDef } from './types';
import {
  createTypedListView,
  createTypedStateView,
  InitialDataTimeoutError,
  serializeViewKey,
} from './views';

type GeneratedRoundViews = TypedViewGroup<{
  state: ViewDef<{ name: string }, 'state', { roundId: bigint }>;
  list: ViewDef<{ name: string }, 'list'>;
}>;

if (false) {
  const views = null as unknown as GeneratedRoundViews;
  views.state.get({ roundId: 42n });
  // @ts-expect-error Generated state views reject raw wire keys.
  views.state.get('42');
  views.list.get();
}

describe('serializeViewKey', () => {
  it('serializes generated one-field object keys to the existing wire string', () => {
    const round = {
      mode: 'state',
      view: 'OreRound/state',
      keyFields: ['roundId'],
    } as const satisfies ViewDef<unknown, 'state', { roundId: bigint }>;
    const miner = {
      mode: 'state',
      view: 'OreMiner/state',
      keyFields: ['authority'],
    } as const satisfies ViewDef<unknown, 'state', { authority: string }>;
    const numbered = {
      mode: 'state',
      view: 'Position/state',
      keyFields: ['position'],
    } as const satisfies ViewDef<unknown, 'state', { position: number }>;

    expect(serializeViewKey(round, { roundId: 42n })).toBe('42');
    expect(serializeViewKey(miner, { authority: 'wallet' })).toBe('wallet');
    expect(serializeViewKey(numbered, { position: 7 })).toBe('7');
  });

  it('uses keyFields rather than object insertion order', () => {
    const view = {
      mode: 'state',
      view: 'OreRound/state',
      keyFields: ['roundId'],
    } as const satisfies ViewDef<unknown, 'state', { roundId: bigint }>;
    const key = { ignored: 'first', roundId: 9n };

    expect(serializeViewKey<{ roundId: bigint }>(view, key)).toBe('9');
  });

  it('preserves legacy scalar string keys when keyFields are absent', () => {
    const view = { mode: 'state', view: 'Legacy/state' } as const;
    expect(serializeViewKey(view, 'legacy-key')).toBe('legacy-key');
  });

  it('rejects lossy numbers and unsupported composite metadata', () => {
    const numbered = {
      mode: 'state',
      view: 'Position/state',
      keyFields: ['position'],
    } as const;
    const composite = {
      mode: 'state',
      view: 'Position/state',
      keyFields: ['owner', 'position'],
    } as const;

    expect(() => serializeViewKey(numbered, { position: Number.MAX_SAFE_INTEGER + 1 })).toThrow(
      /safe integer/
    );
    expect(() => serializeViewKey(composite, { owner: 'wallet', position: 1 })).toThrow(
      /unsupported composite key/
    );
  });
});

describe('createTypedStateView', () => {
  it('serializes typed keys for storage lookups', () => {
    const storage = new MemoryAdapter();
    storage.set('OreRound/state', '42', { name: 'round 42' });
    const queryStore = new QueryStore(storage);
    const registry = new SubscriptionRegistry({
      subscribe: () => undefined,
      unsubscribe: () => undefined,
      refresh: () => undefined,
    } as unknown as ConnectionManager, queryStore);
    const viewDef: ViewDef<{ name: string }, 'state', { roundId: bigint }> = {
      mode: 'state',
      view: 'OreRound/state',
      keyFields: ['roundId'],
    };
    const view = createTypedStateView(
      viewDef,
      storage,
      registry
    );
    const lease = registry.subscribe({ view: 'OreRound/state', key: '42' });
    queryStore.stageSnapshot({
      protocolVersion: 2,
      subscriptionId: lease.subscription.subscriptionId,
      snapshotId: 'round-42',
      authoritative: true,
      mode: 'state',
      entity: 'OreRound/state',
      op: 'snapshot',
      key: '42',
      data: [{ key: '42', data: { name: 'round 42' } }],
      complete: true,
    }, ['42']);

    expect(view.getSync({ roundId: 42n })).toEqual({ name: 'round 42' });
  });
});

type Round = { name: string };

function setup() {
  const storage = new MemoryAdapter();
  const queryStore = new QueryStore(storage);
  const subscribed: Subscription[] = [];
  const unsubscribed: string[] = [];
  const registry = new SubscriptionRegistry({
    subscribe: (subscription: Subscription) => { subscribed.push(subscription); },
    unsubscribe: (subscriptionId: string) => { unsubscribed.push(subscriptionId); },
    refresh: () => undefined,
  } as unknown as ConnectionManager, queryStore);
  const deliver = (subscription: Subscription, entities: Record<string, Round>) => {
    for (const [key, data] of Object.entries(entities)) {
      storage.set(subscription.query.view, key, data);
    }
    queryStore.stageSnapshot({
      protocolVersion: 2,
      subscriptionId: subscription.subscriptionId,
      snapshotId: `snapshot-${subscription.subscriptionId}`,
      authoritative: true,
      mode: subscription.query.key === undefined ? 'list' : 'state',
      entity: subscription.query.view,
      op: 'snapshot',
      data: Object.entries(entities).map(([key, data]) => ({ key, data })),
      complete: true,
    }, Object.keys(entities));
  };
  const list = createTypedListView<Round>(
    { mode: 'list', view: 'OreRound/list' },
    storage,
    registry
  );
  const state = createTypedStateView<Round, { roundId: bigint }>(
    { mode: 'state', view: 'OreRound/state', keyFields: ['roundId'] },
    storage,
    registry
  );
  return { queryStore, registry, subscribed, unsubscribed, deliver, list, state };
}

describe('get', () => {
  it('opens a subscription, waits for its snapshot, and releases it', async () => {
    const { subscribed, unsubscribed, deliver, list, registry } = setup();

    const pending = list.get();
    expect(subscribed).toHaveLength(1);
    deliver(subscribed[0]!, { a: { name: 'round a' }, b: { name: 'round b' } });

    await expect(pending).resolves.toEqual([{ name: 'round a' }, { name: 'round b' }]);
    expect(unsubscribed).toEqual([subscribed[0]!.subscriptionId]);
    expect(registry.getRefCount({ view: 'OreRound/list' })).toBe(0);
  });

  it('resolves a state view to its entity, or null when absent', async () => {
    const { subscribed, deliver, state } = setup();

    const present = state.get({ roundId: 42n });
    deliver(subscribed[0]!, { '42': { name: 'round 42' } });
    await expect(present).resolves.toEqual({ name: 'round 42' });

    const absent = state.get({ roundId: 7n });
    expect(subscribed[1]!.query.key).toBe('7');
    deliver(subscribed[1]!, {});
    await expect(absent).resolves.toBeNull();
  });

  it('reuses an active equivalent subscription without waiting or releasing it', async () => {
    const { subscribed, unsubscribed, deliver, list, registry } = setup();
    const live = registry.subscribe({ view: 'OreRound/list' });
    deliver(subscribed[0]!, { a: { name: 'round a' } });

    await expect(list.get()).resolves.toEqual([{ name: 'round a' }]);
    expect(subscribed).toHaveLength(1);
    expect(unsubscribed).toEqual([]);
    expect(registry.getRefCount({ view: 'OreRound/list' })).toBe(1);
    live.release();
  });

  it('rejects with InitialDataTimeoutError and releases when no snapshot arrives', async () => {
    const { subscribed, unsubscribed, list } = setup();

    const error = await list.get({ timeoutMs: 10 }).catch((value: unknown) => value);
    expect(error).toBeInstanceOf(InitialDataTimeoutError);
    expect(error).toMatchObject({ code: 'INITIAL_DATA_TIMEOUT', view: 'OreRound/list', timeoutMs: 10 });
    expect(unsubscribed).toEqual([subscribed[0]!.subscriptionId]);
  });

  it('rejects with the query error when the subscription fails', async () => {
    const { queryStore, subscribed, list } = setup();

    const pending = list.get();
    queryStore.failRefresh(
      subscribed[0]!.subscriptionId,
      new AreteError('view not found', 'VIEW_NOT_FOUND')
    );
    await expect(pending).rejects.toMatchObject({ code: 'VIEW_NOT_FOUND' });
  });

  it('resolves immediately when the snapshot is disabled', async () => {
    const { list } = setup();
    await expect(list.get({ withSnapshot: false })).resolves.toEqual([]);
  });

  it('keeps timeoutMs out of the wire query and rejects invalid values', async () => {
    const { subscribed, deliver, list } = setup();

    const pending = list.get({ take: 5, timeoutMs: null });
    expect(subscribed[0]!.query).toEqual({ view: 'OreRound/list', take: 5 });
    deliver(subscribed[0]!, {});
    await expect(pending).resolves.toEqual([]);

    await expect(list.get({ timeoutMs: 0 })).rejects.toBeInstanceOf(RangeError);
    await expect(list.get({ timeoutMs: Number.POSITIVE_INFINITY })).rejects.toBeInstanceOf(RangeError);
  });
});

describe('get when the connection fails', () => {
  it('rejects with the connection error instead of timing out', async () => {
    const { registry, subscribed, unsubscribed, list } = setup();

    const pending = list.get({ timeoutMs: null });
    registry.handleConnectionState('error', 'Authentication refused');

    await expect(pending).rejects.toMatchObject({
      code: 'CONNECTION_ERROR',
      message: 'Authentication refused',
    });
    expect(unsubscribed).toEqual([subscribed[0]!.subscriptionId]);
  });

  it('rejects at once while the connection is failed, and waits again once it restarts', async () => {
    const { registry, subscribed, deliver, list } = setup();
    registry.handleConnectionState('error', 'Authentication refused');

    await expect(list.get()).rejects.toMatchObject({ code: 'CONNECTION_ERROR' });

    registry.handleConnectionState('connecting');
    const pending = list.get();
    deliver(subscribed[1]!, { a: { name: 'round a' } });
    await expect(pending).resolves.toEqual([{ name: 'round a' }]);
  });

  it('still reads an active subscription that already has its snapshot', async () => {
    const { registry, subscribed, deliver, list } = setup();
    const live = registry.subscribe({ view: 'OreRound/list' });
    deliver(subscribed[0]!, { a: { name: 'round a' } });
    registry.handleConnectionState('error', 'Connection lost');

    await expect(list.get()).resolves.toEqual([{ name: 'round a' }]);
    live.release();
  });

  it('rejects an unbounded read when a disconnect clears the subscriptions', async () => {
    const { registry, list } = setup();

    const pending = list.get({ timeoutMs: null });
    registry.clear();

    await expect(pending).rejects.toMatchObject({ code: 'CONNECTION_CANCELLED' });
  });
});

describe('getOne', () => {
  it('reads the first item through a take: 1 subscription', async () => {
    const { subscribed, deliver, list } = setup();

    const first = list.getOne();
    expect(subscribed[0]!.query).toEqual({ view: 'OreRound/list', take: 1 });
    deliver(subscribed[0]!, { a: { name: 'round a' } });
    await expect(first).resolves.toEqual({ name: 'round a' });

    const empty = list.getOne();
    deliver(subscribed[1]!, {});
    await expect(empty).resolves.toBeNull();
  });
});
