import { describe, expect, it } from 'vitest';

import manifest from '../../../tests/fixtures/websocket-v2/manifest.json';
import keyedState from '../../../tests/fixtures/websocket-v2/keyed-state.json';
import listWindows from '../../../tests/fixtures/websocket-v2/list-windows.json';
import filters from '../../../tests/fixtures/websocket-v2/filters.json';
import multiBatch from '../../../tests/fixtures/websocket-v2/multi-batch-authoritative.json';
import emptySnapshot from '../../../tests/fixtures/websocket-v2/empty-snapshot.json';
import removeFixture from '../../../tests/fixtures/websocket-v2/remove.json';
import deleteFixture from '../../../tests/fixtures/websocket-v2/delete.json';
import incremental from '../../../tests/fixtures/websocket-v2/incremental-snapshot.json';
import reconnect from '../../../tests/fixtures/websocket-v2/reconnect-replacement.json';
import errors from '../../../tests/fixtures/websocket-v2/errors.json';
import { parseFrame } from './frame';
import { FrameProcessor } from './frame-processor';
import { QueryStore } from './query-store';
import { canonicalQueryKey } from './subscription';
import { MemoryAdapter } from './storage/memory-adapter';
import type { Subscription, SubscriptionQuery } from './types';

function subscription(subscriptionId: string, query: SubscriptionQuery): Subscription {
  return {
    type: 'subscribe',
    protocolVersion: 2,
    subscriptionId,
    query,
    snapshot: { enabled: true },
  };
}

function harness() {
  const storage = new MemoryAdapter();
  const queries = new QueryStore(storage);
  const processor = new FrameProcessor(storage, { queryStore: queries });
  const register = (subscriptionId: string, query: SubscriptionQuery) => {
    const active = subscription(subscriptionId, query);
    queries.register(active, canonicalQueryKey(active));
    return active;
  };
  const process = (frame: unknown) => processor.handleFrame(parseFrame(JSON.stringify(frame)));
  return { storage, queries, processor, register, process };
}

describe('shared WebSocket protocol v2 fixtures', () => {
  it('loads the authoritative fixture manifest', () => {
    expect(manifest.protocolVersion).toBe(2);
    expect(manifest.fixtures).toEqual([
      'keyed-state.json',
      'list-windows.json',
      'filters.json',
      'multi-batch-authoritative.json',
      'empty-snapshot.json',
      'remove.json',
      'delete.json',
      'incremental-snapshot.json',
      'reconnect-replacement.json',
      'errors.json',
    ]);
    expect(parseFrame(JSON.stringify({
      protocolVersion: 2,
      subscriptionId: 'things:all',
      op: 'unsubscribed',
    }))).toMatchObject({ op: 'unsubscribed', subscriptionId: 'things:all' });
    expect(() => parseFrame(JSON.stringify({
      protocolVersion: 2,
      subscriptionId: ' things:all',
      op: 'unsubscribed',
    }))).toThrow(/Invalid WebSocket protocol v2 frame/);
  });

  it('applies keyed state snapshots and patches to only their query', () => {
    const { queries, register, process } = harness();
    const request = keyedState.client[0]!;
    register(request.subscriptionId, request.query);
    keyedState.server.forEach(process);

    expect(queries.getSnapshot<{ score: number }>(request.subscriptionId)).toMatchObject({
      keys: ['wallet-a'],
      data: [{ authority: 'wallet-a', score: 2 }],
      isLoading: false,
    });
  });

  it('keeps independent list windows and exact dot-path filters distinct', () => {
    const identities = listWindows.client.map((request) =>
      canonicalQueryKey({ query: request.query })
    );
    expect(new Set(identities).size).toBe(2);
    expect(listWindows.expectedKeys['rounds:page-1']).toEqual(['6', '5']);
    expect(listWindows.expectedKeys['rounds:page-2']).toEqual(['4', '3']);
    expect(filters.client[0]!.query.filters).toEqual({
      'state.status': 'open',
      'market.symbol': 'SOL',
    });
    expect(filters.notMatching).toHaveLength(2);
  });

  it('stages all authoritative batches and commits membership atomically', () => {
    const { queries, register, process } = harness();
    register('things:all', { view: 'Thing/list' });

    process(multiBatch.server[0]);
    expect(queries.getSnapshot('things:all')).toMatchObject({ keys: [], isLoading: true });

    process(multiBatch.server[1]);
    expect(queries.getSnapshot('things:all')).toMatchObject({
      keys: ['three', 'two', 'one'],
      isLoading: false,
    });

    process({
      ...emptySnapshot.server[0],
      subscriptionId: 'things:all',
      entity: 'Thing/list',
      mode: 'list',
    });
    expect(queries.getSnapshot('things:all')).toMatchObject({ keys: [], data: [] });
  });

  it('merges incremental snapshots without replacing prior membership', () => {
    const { queries, register, process } = harness();
    const request = incremental.client[0]!;
    register(request.subscriptionId, request.query);
    process({
      ...incremental.server[0],
      authoritative: true,
      snapshotId: 'initial',
      data: [{ key: 'order-10', data: { _seq: '40:000000000010' } }],
    });
    process(incremental.server[0]);

    expect(queries.getSnapshot(request.subscriptionId)?.keys).toEqual([
      'order-10',
      'order-11',
      'order-12',
    ]);
  });

  it('treats remove as query-local and delete as source-wide', () => {
    const { queries, storage, register, process } = harness();
    register('orders:open', { view: 'Order/list', filters: { 'state.status': 'open' } });
    register('orders:all', { view: 'Order/list' });
    for (const subscriptionId of ['orders:open', 'orders:all']) {
      process({
        protocolVersion: 2,
        subscriptionId,
        snapshotId: `snapshot:${subscriptionId}`,
        authoritative: true,
        mode: 'list',
        entity: 'Order/list',
        op: 'snapshot',
        data: [{ key: 'order-7', data: { id: 7 } }],
        complete: true,
      });
    }

    process(removeFixture.server[0]);
    expect(queries.getSnapshot('orders:open')?.keys).toEqual([]);
    expect(queries.getSnapshot('orders:all')?.keys).toEqual(['order-7']);
    expect(storage.get('Order/list', 'order-7')).toEqual({ id: 7 });

    process(deleteFixture.server[0]);
    expect(queries.getSnapshot('orders:all')?.keys).toEqual([]);
    expect(storage.get('Order/list', 'order-7')).toBeNull();
  });

  it('normalizes one sequenced patch once while routing it to multiple queries', () => {
    const { queries, storage, register, process } = harness();
    register('events:all', { view: 'Event/list' });
    register('events:open', {
      view: 'Event/list',
      filters: { 'state.status': 'open' },
    });
    for (const subscriptionId of ['events:all', 'events:open']) {
      process({
        protocolVersion: 2,
        subscriptionId,
        op: 'subscribed',
        query: queries.getSubscription(subscriptionId)!.query,
        mode: 'list',
      });
      process({
        protocolVersion: 2,
        subscriptionId,
        mode: 'list',
        entity: 'Event/list',
        op: 'upsert',
        key: 'event-1',
        data: { values: ['a'] },
        seq: '50:000000000001',
      });
    }
    for (const subscriptionId of ['events:all', 'events:open']) {
      process({
        protocolVersion: 2,
        subscriptionId,
        mode: 'list',
        entity: 'Event/list',
        op: 'patch',
        key: 'event-1',
        data: { values: ['b'] },
        append: ['values'],
        seq: '51:000000000001',
      });
    }

    expect(storage.get('Event/list', 'event-1')).toEqual({ values: ['a', 'b'] });
    expect(queries.getSnapshot('events:all')?.keys).toEqual(['event-1']);
    expect(queries.getSnapshot('events:open')?.keys).toEqual(['event-1']);
  });

  it('retains old data during reconnect and replaces it on the complete snapshot', () => {
    const { queries, register, process } = harness();
    const [before, after] = reconnect.sessions;
    register(before!.subscriptionId, { view: 'Round/list' });
    const snapshot = (session: typeof before) => ({
      protocolVersion: 2,
      subscriptionId: session!.subscriptionId,
      snapshotId: session!.snapshotId,
      authoritative: true,
      mode: 'list',
      entity: 'Round/list',
      op: 'snapshot',
      data: session!.authoritativeKeys.map((key) => ({ key, data: { id: key } })),
      complete: true,
    });
    process(snapshot(before));
    queries.beginReconnect();
    expect(queries.getSnapshot(before!.subscriptionId)).toMatchObject({
      keys: ['10', '9'],
      isRefreshing: true,
      isLoading: false,
    });

    process(snapshot(after));
    expect(queries.getSnapshot(before!.subscriptionId)).toMatchObject({
      keys: ['11', '10'],
      isRefreshing: false,
    });
  });

  it('routes protocol errors to the identified query', () => {
    const { queries, register, process } = harness();
    for (const fixture of errors.cases) {
      const response = fixture.response;
      if (response.subscriptionId === 'duplicate') {
        register('duplicate', { view: 'Thing/list' });
        process(response);
        expect(queries.getSnapshot('duplicate')?.error?.code).toBe(
          'duplicate-subscription-id'
        );
      } else {
        expect(parseFrame(JSON.stringify(response)).subscriptionId).toBe(response.subscriptionId);
      }
    }
  });
});
