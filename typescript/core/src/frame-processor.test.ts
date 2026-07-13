import { describe, expect, it, vi } from 'vitest';
import { z } from 'zod';

import type { EntityFrame, SnapshotFrame, SubscribedFrame } from './frame';
import { FrameProcessor } from './frame-processor';
import { SortedStorageDecorator } from './storage/sorted-decorator';
import { MemoryAdapter } from './storage/memory-adapter';

const bigintSchema = z
  .union([z.bigint(), z.string(), z.number().int()])
  .transform((value) => BigInt(value));

const tokenPositionSchema = z
  .object({
    total_deposit: bigintSchema.optional(),
    recent_ids: z.array(z.string()).nullable().optional(),
    metrics: z
      .object({
        last_updated_at: bigintSchema.optional(),
      })
      .transform((value) => ({
        lastUpdatedAt: value.last_updated_at,
      }))
      .optional(),
  })
  .transform((value) => ({
    totalDeposit: value.total_deposit,
    recentIds: value.recent_ids,
    metrics: value.metrics,
  }));

describe('FrameProcessor', () => {
  it('stores canonical snapshot data and keeps seq as internal metadata', () => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    const processor = new FrameProcessor(storage, {
      schemas: { TokenPosition: tokenPositionSchema },
    });
    const updateSpy = vi.fn();
    storage.onUpdate(updateSpy);

    const frame: SnapshotFrame = {
      mode: 'list',
      entity: 'TokenPosition/list',
      op: 'snapshot',
      data: [{
        key: 'position-1',
        data: {
          total_deposit: '7',
          metrics: { last_updated_at: '9' },
          _seq: '10:000000000001',
        },
      }],
      complete: true,
    };

    processor.handleFrame(frame);

    const stored = storage.get<Record<string, unknown>>('TokenPosition/list', 'position-1');
    expect(stored).toEqual({
      totalDeposit: 7n,
      metrics: { lastUpdatedAt: 9n },
    });
    expect((stored as Record<string, unknown>).__seq).toBe('10:000000000001');
    expect(Object.keys(stored ?? {})).not.toContain('__seq');
    expect(updateSpy).toHaveBeenCalledWith('TokenPosition/list', 'position-1', {
      type: 'upsert',
      key: 'position-1',
      data: stored,
    });
  });

  it('normalizes patch payloads before merge and append-path handling', () => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    const processor = new FrameProcessor(storage, {
      schemas: { TokenPosition: tokenPositionSchema },
    });
    const richUpdateSpy = vi.fn();
    storage.onRichUpdate(richUpdateSpy);

    const firstFrame: EntityFrame = {
      mode: 'list',
      entity: 'TokenPosition/list',
      op: 'upsert',
      key: 'position-1',
      data: {
        total_deposit: '1',
        recent_ids: ['a'],
      },
      seq: '1:000000000001',
    };
    processor.handleFrame(firstFrame);

    const patchFrame: EntityFrame = {
      mode: 'list',
      entity: 'TokenPosition/list',
      op: 'patch',
      key: 'position-1',
      data: {
        total_deposit: '2',
        recent_ids: ['b'],
        metrics: { last_updated_at: '12' },
      },
      append: ['recent_ids'],
      seq: '2:000000000002',
    };
    processor.handleFrame(patchFrame);

    const stored = storage.get<Record<string, unknown>>('TokenPosition/list', 'position-1');
    expect(stored).toEqual({
      totalDeposit: 2n,
      recentIds: ['a', 'b'],
      metrics: { lastUpdatedAt: 12n },
    });

    const patchUpdate = richUpdateSpy.mock.calls.at(-1)?.[2] as {
      type: string;
      patch?: Record<string, unknown>;
      after?: Record<string, unknown>;
    };
    expect(patchUpdate.type).toBe('updated');
    expect(patchUpdate.patch).toEqual({
      totalDeposit: 2n,
      recentIds: ['b'],
      metrics: { lastUpdatedAt: 12n },
    });
    expect(patchUpdate.after).toEqual(stored);
    expect((patchUpdate.after as Record<string, unknown>).__seq).toBe('2:000000000002');
  });

  it('preserves omitted patch fields while allowing explicit null clears', () => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    const processor = new FrameProcessor(storage, {
      schemas: { TokenPosition: tokenPositionSchema },
    });

    processor.handleFrame({
      mode: 'list',
      entity: 'TokenPosition/list',
      op: 'upsert',
      key: 'position-1',
      data: {
        total_deposit: '1',
        recent_ids: ['a'],
        metrics: { last_updated_at: '9' },
      },
      seq: '1:000000000001',
    } satisfies EntityFrame);

    processor.handleFrame({
      mode: 'list',
      entity: 'TokenPosition/list',
      op: 'patch',
      key: 'position-1',
      data: {
        total_deposit: '2',
        metrics: {},
      },
      seq: '2:000000000002',
    } satisfies EntityFrame);

    expect(storage.get<Record<string, unknown>>('TokenPosition/list', 'position-1')).toEqual({
      totalDeposit: 2n,
      recentIds: ['a'],
      metrics: { lastUpdatedAt: 9n },
    });

    processor.handleFrame({
      mode: 'list',
      entity: 'TokenPosition/list',
      op: 'patch',
      key: 'position-1',
      data: {
        recent_ids: null,
      },
      seq: '3:000000000003',
    } satisfies EntityFrame);

    expect(storage.get<Record<string, unknown>>('TokenPosition/list', 'position-1')).toEqual({
      totalDeposit: 2n,
      recentIds: null,
      metrics: { lastUpdatedAt: 9n },
    });
  });

  it('preserves raw payloads for schema-less stacks', () => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    const processor = new FrameProcessor(storage);

    processor.handleFrame({
      mode: 'list',
      entity: 'TokenPosition/list',
      op: 'upsert',
      key: 'position-1',
      data: {
        total_deposit: '1',
        recent_ids: ['a'],
      },
    } satisfies EntityFrame);
    processor.handleFrame({
      mode: 'list',
      entity: 'TokenPosition/list',
      op: 'patch',
      key: 'position-1',
      data: {
        recent_ids: ['b'],
      },
      append: ['recent_ids'],
    } satisfies EntityFrame);

    expect(storage.get<Record<string, unknown>>('TokenPosition/list', 'position-1')).toEqual({
      total_deposit: '1',
      recent_ids: ['a', 'b'],
    });
  });

  it('normalizes subscribed sort paths so canonical caches stay ordered', () => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    const processor = new FrameProcessor(storage, {
      schemas: { TokenPosition: tokenPositionSchema },
    });

    const subscribed: SubscribedFrame = {
      op: 'subscribed',
      view: 'TokenPosition/list',
      mode: 'list',
      sort: {
        field: ['_seq'],
        order: 'desc',
      },
    };
    processor.handleFrame(subscribed);

    processor.handleFrame({
      mode: 'list',
      entity: 'TokenPosition/list',
      op: 'upsert',
      key: 'older',
      data: { total_deposit: '1' },
      seq: '1:000000000001',
    } satisfies EntityFrame);
    processor.handleFrame({
      mode: 'list',
      entity: 'TokenPosition/list',
      op: 'upsert',
      key: 'newer',
      data: { total_deposit: '2' },
      seq: '2:000000000002',
    } satisfies EntityFrame);

    const ordered = storage.getAll<Record<string, unknown>>('TokenPosition/list');
    expect(ordered.map((item) => item.totalDeposit)).toEqual([2n, 1n]);
  });

  it('normalizes sort fields before max-entry eviction', () => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    const processor = new FrameProcessor(storage, {
      maxEntriesPerView: 1,
      schemas: { TokenPosition: tokenPositionSchema },
    });

    processor.handleFrame({
      op: 'subscribed',
      view: 'TokenPosition/list',
      mode: 'list',
      sort: {
        field: ['total_deposit'],
        order: 'desc',
      },
    } satisfies SubscribedFrame);

    processor.handleFrame({
      mode: 'list',
      entity: 'TokenPosition/list',
      op: 'upsert',
      key: 'smaller',
      data: { total_deposit: '1' },
    } satisfies EntityFrame);
    processor.handleFrame({
      mode: 'list',
      entity: 'TokenPosition/list',
      op: 'upsert',
      key: 'larger',
      data: { total_deposit: '2' },
    } satisfies EntityFrame);

    expect(storage.getAll<Record<string, unknown>>('TokenPosition/list')).toEqual([
      { totalDeposit: 2n },
    ]);
    expect(storage.get('TokenPosition/list', 'smaller')).toBeNull();
  });
});
