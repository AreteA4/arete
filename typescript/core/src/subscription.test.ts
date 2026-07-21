import { describe, expect, it, vi } from 'vitest';

import type { ConnectionManager } from './connection';
import { QueryStore } from './query-store';
import { MemoryAdapter } from './storage/memory-adapter';
import { canonicalQueryKey, SubscriptionRegistry } from './subscription';

function createRegistry() {
  const connection = {
    subscribe: vi.fn(),
    unsubscribe: vi.fn(),
    refresh: vi.fn(),
  };
  const storage = new MemoryAdapter();
  const queryStore = new QueryStore(storage);
  const registry = new SubscriptionRegistry(
    connection as unknown as ConnectionManager,
    queryStore
  );
  return { connection, queryStore, registry, storage };
}

describe('protocol v2 SubscriptionRegistry', () => {
  it('refcounts equivalent canonical queries and keeps one opaque ID', () => {
    const { connection, registry } = createRegistry();
    const first = registry.subscribe({
      view: 'Position/list',
      filters: { owner: 'wallet', 'state.status': 'open' },
      take: 10,
      skip: 0,
    });
    const equivalent = registry.subscribe({
      view: 'Position/list',
      filters: { 'state.status': 'open', owner: 'wallet' },
      skip: 0,
      take: 10,
    });

    expect(connection.subscribe).toHaveBeenCalledTimes(1);
    expect(first.subscription).toEqual({
      type: 'subscribe',
      protocolVersion: 2,
      subscriptionId: expect.stringMatching(/^a4-/),
      query: {
        view: 'Position/list',
        filters: { owner: 'wallet', 'state.status': 'open' },
        take: 10,
        skip: 0,
      },
      snapshot: { enabled: true },
    });
    expect(equivalent.subscription.subscriptionId).toBe(first.subscription.subscriptionId);
    expect(registry.getRefCount(first.subscription.query)).toBe(2);

    first.release();
    expect(connection.unsubscribe).not.toHaveBeenCalled();
    equivalent.release();
    expect(connection.unsubscribe).toHaveBeenCalledWith(first.subscription.subscriptionId);
  });

  it('allows different queries on the same view to coexist', () => {
    const { connection, registry } = createRegistry();
    const firstPage = registry.subscribe({ view: 'Round/list', take: 2, skip: 0 });
    const secondPage = registry.subscribe({ view: 'Round/list', take: 2, skip: 2 });
    const filtered = registry.subscribe({
      view: 'Round/list',
      filters: { 'state.status': 'open' },
      take: 2,
    });

    expect(connection.subscribe).toHaveBeenCalledTimes(3);
    expect(new Set([
      firstPage.subscription.subscriptionId,
      secondPage.subscription.subscriptionId,
      filtered.subscription.subscriptionId,
    ]).size).toBe(3);
  });

  it('includes every query field and snapshot enabled in canonical identity', () => {
    const base = {
      view: 'Thing/list',
      key: 'key',
      partition: 'partition',
      filters: { 'state.value': { nested: true } },
      take: 1,
      skip: 0,
      after: '1:0001',
      snapshotLimit: 10,
    };
    const identity = canonicalQueryKey({ query: base, snapshot: { enabled: true } });

    expect(canonicalQueryKey({
      query: { ...base, filters: { 'state.value': { nested: true } } },
    })).toBe(identity);
    expect(canonicalQueryKey({ query: base, snapshot: { enabled: false } })).not.toBe(identity);
    expect(canonicalQueryKey({ query: { ...base, skip: 1 } })).not.toBe(identity);
  });

  it('refreshes with the stable subscription ID and retains membership', async () => {
    const { connection, queryStore, registry, storage } = createRegistry();
    const lease = registry.subscribe({ view: 'Round/list', take: 2 });
    storage.set('Round/list', '10', { id: 10 });
    queryStore.stageSnapshot({
      protocolVersion: 2,
      subscriptionId: lease.subscription.subscriptionId,
      snapshotId: 'initial',
      authoritative: true,
      mode: 'list',
      entity: 'Round/list',
      op: 'snapshot',
      data: [{ key: '10', data: { id: 10 } }],
      complete: true,
    }, ['10']);

    const refresh = lease.refresh();
    let settled = false;
    void refresh.then(() => { settled = true; });
    await Promise.resolve();

    expect(connection.refresh).toHaveBeenCalledWith(lease.subscription);
    expect(settled).toBe(false);
    expect(lease.getSnapshot()).toMatchObject({
      keys: ['10'],
      isLoading: false,
      isRefreshing: true,
    });

    queryStore.stageSnapshot({
      protocolVersion: 2,
      subscriptionId: lease.subscription.subscriptionId,
      snapshotId: 'refreshed',
      authoritative: true,
      mode: 'list',
      entity: 'Round/list',
      op: 'snapshot',
      data: [{ key: '10', data: { id: 10 } }],
      complete: true,
    }, ['10']);
    await refresh;
    expect(settled).toBe(true);
    expect(lease.getSnapshot()).toMatchObject({ isRefreshing: false });
  });

  it('rejects a pending refresh when its final lease is released', async () => {
    const { registry } = createRegistry();
    const lease = registry.subscribe({ view: 'Round/state', key: '10' });
    const refresh = lease.refresh();

    lease.release();

    await expect(refresh).rejects.toThrow('released while refreshing');
  });

  it('keeps pending refreshes alive while reconnecting', async () => {
    const { queryStore, registry } = createRegistry();
    const lease = registry.subscribe({ view: 'Round/state', key: '10' });
    const refresh = lease.refresh();

    registry.handleConnectionState('reconnecting');
    expect(lease.getSnapshot().isRefreshing).toBe(false);
    expect(lease.getSnapshot().error).toBeUndefined();

    queryStore.stageSnapshot({
      protocolVersion: 2,
      subscriptionId: lease.subscription.subscriptionId,
      snapshotId: 'reconnected',
      authoritative: true,
      mode: 'state',
      entity: 'Round/state',
      op: 'snapshot',
      data: [],
      complete: true,
    }, []);
    await expect(refresh).resolves.toBeUndefined();
  });

  it('rejects pending refreshes when reconnection fails terminally', async () => {
    const { registry } = createRegistry();
    const lease = registry.subscribe({ view: 'Round/state', key: '10' });
    const refresh = lease.refresh();

    registry.handleConnectionState('reconnecting');
    registry.handleConnectionState('error');

    await expect(refresh).rejects.toThrow('Connection failed while refreshing subscriptions');
    expect(lease.getSnapshot()).toMatchObject({
      isRefreshing: false,
      error: expect.objectContaining({ code: 'CONNECTION_ERROR' }),
    });
  });

  it('rejects pending refreshes even when unsubscribe throws during release', async () => {
    const { connection, registry } = createRegistry();
    const lease = registry.subscribe({ view: 'Round/state', key: '10' });
    const refresh = lease.refresh();
    connection.unsubscribe.mockImplementationOnce(() => {
      throw new Error('unsubscribe failed');
    });

    expect(() => lease.release()).not.toThrow();
    await expect(refresh).rejects.toThrow('released while refreshing');
  });

  it('publishes refresh send failures on the query and rejects the request', async () => {
    const { connection, registry } = createRegistry();
    const lease = registry.subscribe({ view: 'Round/list', take: 2 });
    connection.refresh.mockImplementationOnce(() => {
      throw new Error('refresh send failed');
    });

    await expect(lease.refresh()).rejects.toThrow('refresh send failed');
    expect(lease.getSnapshot()).toMatchObject({
      isLoading: false,
      isRefreshing: false,
      error: expect.objectContaining({ message: 'refresh send failed' }),
    });
  });

  it('does not retain a query when the connection rejects registration', () => {
    const { connection, registry } = createRegistry();
    connection.subscribe.mockImplementationOnce(() => {
      throw new Error('connection rejected subscription');
    });

    expect(() => registry.subscribe({ view: 'Miner/state', key: 'wallet' }))
      .toThrowError(/connection rejected/);
    expect(registry.getRefCount({ view: 'Miner/state', key: 'wallet' })).toBe(0);
    expect(registry.getActiveSubscriptions()).toEqual([]);
  });

  it('refreshView refreshes every active subscription for a view', async () => {
    const { connection, registry } = createRegistry();
    const first = registry.subscribe({ view: 'Round/state', key: '1' }, false);
    const second = registry.subscribe({ view: 'Round/state', key: '2' }, false);
    registry.subscribe({ view: 'Miner/state', key: 'wallet' }, false);

    await registry.refreshView('Round/state');

    expect(connection.refresh).toHaveBeenCalledTimes(2);
    expect(connection.refresh).toHaveBeenCalledWith(first.subscription);
    expect(connection.refresh).toHaveBeenCalledWith(second.subscription);
  });

  it('refreshView narrows to a single key when provided', async () => {
    const { connection, registry } = createRegistry();
    registry.subscribe({ view: 'Round/state', key: '1' }, false);
    const second = registry.subscribe({ view: 'Round/state', key: '2' }, false);

    await registry.refreshView('Round/state', '2');

    expect(connection.refresh).toHaveBeenCalledTimes(1);
    expect(connection.refresh).toHaveBeenCalledWith(second.subscription);
  });

  it('refreshView is a no-op when no subscription matches', async () => {
    const { connection, registry } = createRegistry();
    registry.subscribe({ view: 'Round/state', key: '1' });

    await expect(registry.refreshView('Round/state', 'unknown')).resolves.toBeUndefined();
    await expect(registry.refreshView('Treasury/state')).resolves.toBeUndefined();
    expect(connection.refresh).not.toHaveBeenCalled();
  });
});
