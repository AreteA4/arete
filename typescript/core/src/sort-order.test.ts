import { describe, expect, it } from 'vitest';

import type { EntityFrame, SortOrder } from './frame';
import { FrameProcessor } from './frame-processor';
import { QueryStore } from './query-store';
import { compareSortOrder } from './sort-order';
import { MemoryAdapter } from './storage/memory-adapter';
import { SortedStorageDecorator } from './storage/sorted-decorator';
import { EntityStore } from './store';
import type { Subscription } from './types';

const VIEW = 'Pool/list';

// Two ranked entities, one with the sort field missing, one with it null.
const ENTITIES: Record<string, { score?: number | null }> = {
  high: { score: 9 },
  low: { score: 1 },
  missing: {},
  nulled: { score: null },
};

// Unranked entities trail in both directions, tied among themselves and
// ordered by the ascending key tie-break.
const EXPECTED: Record<SortOrder, string[]> = {
  asc: ['low', 'high', 'missing', 'nulled'],
  desc: ['high', 'low', 'missing', 'nulled'],
};

describe('compareSortOrder', () => {
  it.each(['asc', 'desc'] as const)('ranks missing and null values last under %s', (order) => {
    expect(compareSortOrder(undefined, 1, order)).toBeGreaterThan(0);
    expect(compareSortOrder(1, undefined, order)).toBeLessThan(0);
    expect(compareSortOrder(null, 1, order)).toBeGreaterThan(0);
    expect(compareSortOrder(1, null, order)).toBeLessThan(0);
    expect(compareSortOrder(null, undefined, order)).toBe(0);
  });

  it('reverses only the comparison between two ranked values', () => {
    expect(compareSortOrder(1, 2, 'asc')).toBeLessThan(0);
    expect(compareSortOrder(1, 2, 'desc')).toBeGreaterThan(0);
    expect(compareSortOrder(2n, 1, 'desc')).toBeLessThan(0);
  });
});

describe('QueryStore ordering of unranked entities', () => {
  function setup(query: Subscription['query'], sort?: { field: string[]; order: SortOrder }) {
    const storage = new MemoryAdapter();
    const queries = new QueryStore(storage);
    const processor = new FrameProcessor(storage, { queryStore: queries });
    queries.register({
      type: 'subscribe',
      protocolVersion: 2,
      subscriptionId: 'pools',
      query,
      snapshot: { enabled: false },
    }, 'pools');
    queries.acknowledge('pools', query, 'list', sort);
    const upsert = (key: string, data: unknown, seq?: string) => processor.handleFrame({
      subscriptionId: 'pools',
      mode: 'list',
      entity: VIEW,
      op: 'upsert',
      key,
      data,
      ...(seq ? { seq } : {}),
    } satisfies EntityFrame);
    return { upsert, keys: () => queries.getSnapshot('pools')!.keys };
  }

  it.each(['asc', 'desc'] as const)('sorts a missing or null sort field last under %s', (order) => {
    const { upsert, keys } = setup({ view: VIEW }, { field: ['score'], order });
    for (const key of ['missing', 'high', 'nulled', 'low']) upsert(key, ENTITIES[key]);

    expect(keys()).toEqual(EXPECTED[order]);
  });

  it.each([
    ['desc', { view: VIEW }, ['newer', 'older', 'unsequenced']],
    ['asc', { view: VIEW, after: '1:5' }, ['older', 'newer', 'unsequenced']],
  ] as const)('sorts an entity without a sequence last in %s sequence order', (_order, query, expected) => {
    const { upsert, keys } = setup(query);
    upsert('unsequenced', { id: 'unsequenced' });
    upsert('newer', { id: 'newer' }, '20:000000000001');
    upsert('older', { id: 'older' }, '10:000000000001');

    expect(keys()).toEqual(expected);
  });
});

describe('SortedStorageDecorator ordering of unranked entities', () => {
  it.each(['asc', 'desc'] as const)('keeps unranked entities last under %s', (order) => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    storage.setViewConfig(VIEW, { sort: { field: ['score'], order } });
    for (const key of ['missing', 'high', 'nulled', 'low']) storage.set(VIEW, key, ENTITIES[key]);

    expect(storage.keys(VIEW)).toEqual(EXPECTED[order]);
  });

  it.each(['asc', 'desc'] as const)('rebuilds with unranked entities last under %s', (order) => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    for (const key of ['missing', 'high', 'nulled', 'low']) storage.set(VIEW, key, ENTITIES[key]);
    storage.setViewConfig(VIEW, { sort: { field: ['score'], order } });

    expect(storage.keys(VIEW)).toEqual(EXPECTED[order]);
  });

  it('evicts an unranked entity before any ranked one', () => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    storage.setViewConfig(VIEW, { sort: { field: ['score'], order: 'desc' } });
    for (const key of ['missing', 'high', 'low']) storage.set(VIEW, key, ENTITIES[key]);

    expect(storage.evictOldest(VIEW)).toBe('missing');
    expect(storage.keys(VIEW)).toEqual(['high', 'low']);
  });
});

describe('EntityStore ordering of unranked entities', () => {
  it.each(['asc', 'desc'] as const)('keeps unranked entities last under %s', (order) => {
    const inserted = new EntityStore();
    inserted.setViewConfig(VIEW, { sort: { field: ['score'], order } });
    const rebuilt = new EntityStore();
    for (const key of ['missing', 'high', 'nulled', 'low']) {
      const frame = { mode: 'list', entity: VIEW, op: 'upsert', key, data: ENTITIES[key] } as const;
      inserted.handleFrame(frame);
      rebuilt.handleFrame(frame);
    }
    rebuilt.setViewConfig(VIEW, { sort: { field: ['score'], order } });

    const keys = (store: EntityStore) =>
      store.getAll<{ score?: number | null }>(VIEW).map((entity) =>
        Object.keys(ENTITIES).find((key) => ENTITIES[key] === entity)
      );
    expect(keys(inserted)).toEqual(EXPECTED[order]);
    expect(keys(rebuilt)).toEqual(EXPECTED[order]);
  });
});
