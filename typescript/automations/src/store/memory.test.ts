import { describe, expect, it } from 'vitest';

import { memoryStore } from './memory';

describe('memoryStore', () => {
  it('leaves a source unchanged when its delivery cannot be stored', async () => {
    const store = memoryStore();
    const advance = (payload: unknown, expectedVersion: number | null) =>
      store.sources.advance({
        source: 'rates',
        key: 'all',
        expectedVersion,
        next: { hash: 'b', value: { best: 'b' }, pending: null },
        now: 1,
        deliver: { name: 'rates', payload },
      });
    // A function cannot be cloned into the store.
    await expect(advance({ best: () => 'b' }, null)).rejects.toThrow();
    expect(await store.sources.get('rates', 'all')).toBeNull();
    expect(await store.jobs.list({ kind: 'delivery' })).toEqual([]);
    // The same change can still be recorded, with its trigger.
    expect((await advance({ best: 'b' }, null))?.generation).toBe(1);
    expect((await store.jobs.list({ kind: 'delivery' })).map((job) => job.id)).toEqual(['view:rates:all:1']);
  });
});
