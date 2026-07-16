import { describe, expect, it, vi } from 'vitest';

import type { ConnectionManager } from './connection';
import { SubscriptionRegistry } from './subscription';
import { AreteError } from './types';

function createConnection() {
  return {
    subscribe: vi.fn(),
    unsubscribe: vi.fn(),
    refresh: vi.fn(),
  };
}

describe('SubscriptionRegistry', () => {
  it('refcounts equivalent options with canonical filter ordering', () => {
    const connection = createConnection();
    const registry = new SubscriptionRegistry(connection as unknown as ConnectionManager);
    const first = {
      view: 'Position/list',
      filters: { owner: 'wallet', status: 'open' },
      take: 10,
    };
    const equivalent = {
      view: 'Position/list',
      filters: { status: 'open', owner: 'wallet' },
      take: 10,
    };

    const unsubscribeFirst = registry.subscribe(first);
    const unsubscribeEquivalent = registry.subscribe(equivalent);

    expect(connection.subscribe).toHaveBeenCalledTimes(1);
    expect(connection.subscribe).toHaveBeenCalledWith({
      view: 'Position/list',
      filters: { owner: 'wallet', status: 'open' },
      take: 10,
    });
    expect(registry.getRefCount(equivalent)).toBe(2);

    unsubscribeFirst();
    unsubscribeFirst();
    expect(registry.getRefCount(first)).toBe(1);
    expect(connection.unsubscribe).not.toHaveBeenCalled();

    unsubscribeEquivalent();
    expect(connection.unsubscribe).toHaveBeenCalledWith('Position/list', undefined);
  });

  it('rejects incompatible options for the same wire identity', () => {
    const connection = createConnection();
    const registry = new SubscriptionRegistry(connection as unknown as ConnectionManager);

    registry.subscribe({ view: 'Position/list', key: 'position-1', take: 1 });

    expect(() =>
      registry.subscribe({ view: 'Position/list', key: 'position-1', take: 2 })
    ).toThrowError(/incompatible options/);
    expect(registry.getRefCount({ view: 'Position/list', key: 'position-1', take: 1 })).toBe(1);
    expect(registry.getRefCount({ view: 'Position/list', key: 'position-1', take: 2 })).toBe(0);
    expect(connection.subscribe).toHaveBeenCalledTimes(1);
  });

  it('refreshes the exact active options without changing the refcount', () => {
    const connection = createConnection();
    const registry = new SubscriptionRegistry(connection as unknown as ConnectionManager);
    const subscription = {
      view: 'OreMiner/state',
      key: 'wallet',
      withSnapshot: true,
      after: '10:1',
      snapshotLimit: 1,
    };

    registry.subscribe(subscription);
    registry.subscribe(subscription);
    registry.refresh(subscription);

    expect(connection.refresh).toHaveBeenCalledWith(subscription);
    expect(registry.getRefCount(subscription)).toBe(2);
    expect(connection.subscribe).toHaveBeenCalledTimes(1);
    expect(connection.unsubscribe).not.toHaveBeenCalled();
  });

  it('rejects refresh for an inactive identity', () => {
    const connection = createConnection();
    const registry = new SubscriptionRegistry(connection as unknown as ConnectionManager);

    expect(() =>
      registry.refresh({ view: 'OreMiner/state', key: 'wallet' })
    ).toThrowError(/Cannot refresh inactive subscription/);
    expect(connection.refresh).not.toHaveBeenCalled();
  });

  it('does not retain a subscription when the connection rejects it', () => {
    const connection = createConnection();
    connection.subscribe.mockImplementationOnce(() => {
      throw new Error('connection rejected subscription');
    });
    const registry = new SubscriptionRegistry(connection as unknown as ConnectionManager);
    const subscription = { view: 'OreMiner/state', key: 'wallet' };

    expect(() => registry.subscribe(subscription)).toThrowError(/connection rejected/);
    expect(connection.unsubscribe).toHaveBeenCalledWith('OreMiner/state', 'wallet');
    expect(registry.getRefCount(subscription)).toBe(0);
    expect(registry.getActiveSubscriptions()).toEqual([]);
  });

  it('preserves the registration error when best-effort rollback also fails', () => {
    const connection = createConnection();
    const registrationError = new Error('registration failed');
    connection.subscribe.mockImplementationOnce(() => {
      throw registrationError;
    });
    connection.unsubscribe.mockImplementationOnce(() => {
      throw new Error('rollback failed');
    });
    const registry = new SubscriptionRegistry(connection as unknown as ConnectionManager);
    const subscription = { view: 'OreMiner/state', key: 'wallet' };

    expect(() => registry.subscribe(subscription)).toThrow(registrationError);
    expect(registry.getRefCount(subscription)).toBe(0);
    expect(registry.getActiveSubscriptions()).toEqual([]);
  });

  it('does not roll back a pre-existing incompatible connection subscription', () => {
    const connection = createConnection();
    connection.subscribe.mockImplementationOnce(() => {
      throw new AreteError('incompatible', 'INCOMPATIBLE_SUBSCRIPTION');
    });
    const registry = new SubscriptionRegistry(connection as unknown as ConnectionManager);

    expect(() =>
      registry.subscribe({ view: 'Position/list', key: 'position', take: 2 })
    ).toThrowError(/incompatible/);
    expect(connection.unsubscribe).not.toHaveBeenCalled();
    expect(registry.getActiveSubscriptions()).toEqual([]);
  });

  it('ignores unsubscribe for an inactive identity', () => {
    const connection = createConnection();
    const registry = new SubscriptionRegistry(connection as unknown as ConnectionManager);

    registry.unsubscribe({ view: 'OreMiner/state', key: 'wallet' });

    expect(connection.unsubscribe).not.toHaveBeenCalled();
  });

  it('tracks different keys independently', () => {
    const connection = createConnection();
    const registry = new SubscriptionRegistry(connection as unknown as ConnectionManager);

    const unsubscribeFirst = registry.subscribe({ view: 'OreMiner/state', key: 'wallet-1' });
    registry.subscribe({ view: 'OreMiner/state', key: 'wallet-2' });
    unsubscribeFirst();

    expect(connection.unsubscribe).toHaveBeenCalledTimes(1);
    expect(connection.unsubscribe).toHaveBeenCalledWith('OreMiner/state', 'wallet-1');
    expect(registry.getRefCount({ view: 'OreMiner/state', key: 'wallet-2' })).toBe(1);
  });
});
