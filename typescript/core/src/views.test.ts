import { describe, expect, it } from 'vitest';

import { MemoryAdapter } from './storage/memory-adapter';
import { SubscriptionRegistry } from './subscription';
import type { ConnectionManager } from './connection';
import { QueryStore } from './query-store';
import type { TypedViewGroup, ViewDef } from './types';
import { createTypedStateView, serializeViewKey } from './views';

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
