import { describe, expect, it, vi } from 'vitest';
import { z } from 'zod';

import type { EntityFrame, SnapshotFrame, SubscribedFrame } from './frame';
import { FrameProcessor, ProcessedSlotTimeoutError } from './frame-processor';
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

const completeEntitySchema = z
  .object({
    entity_id: z.string(),
    total_deposit: bigintSchema,
  })
  .transform((value) => ({
    entityId: value.entity_id,
    totalDeposit: value.total_deposit,
    validatedBy: 'full' as const,
  }));

const partialEntitySchema = z
  .object({
    entity_id: z.string().optional(),
    total_deposit: bigintSchema.optional(),
  })
  .transform((value) => ({
    ...(value.entity_id !== undefined ? { entityId: value.entity_id } : {}),
    ...(value.total_deposit !== undefined ? { totalDeposit: value.total_deposit } : {}),
    validatedBy: 'patch' as const,
  }));

describe('FrameProcessor', () => {
  it('resolves processed-slot waits only after buffered storage updates are flushed', async () => {
    vi.useFakeTimers();
    try {
      const storage = new SortedStorageDecorator(new MemoryAdapter());
      const processor = new FrameProcessor(storage, { flushIntervalMs: 16 });
      const wait = processor.waitForProcessedSlot(42, { timeoutMs: 100 });

      processor.handleFrame({
        mode: 'state',
        entity: 'Board/state',
        op: 'upsert',
        key: 'board',
        data: { round: 7 },
        seq: '42:000000000001',
      } satisfies EntityFrame);

      expect(processor.getProcessedSlot()).toBeNull();
      expect(storage.get('Board/state', 'board')).toBeNull();

      await vi.advanceTimersByTimeAsync(16);

      await expect(wait).resolves.toBe(42n);
      expect(processor.getProcessedSlot()).toBe(42n);
      expect(storage.get('Board/state', 'board')).toEqual({ round: 7 });
    } finally {
      vi.useRealTimers();
    }
  });

  it('supports processed-slot timeout and abort without changing processed state', async () => {
    vi.useFakeTimers();
    try {
      const processor = new FrameProcessor(new MemoryAdapter());
      const timedOut = processor.waitForProcessedSlot(10n, { timeoutMs: 5 });
      const timedOutAssertion = expect(timedOut).rejects.toBeInstanceOf(
        ProcessedSlotTimeoutError
      );
      const controller = new AbortController();
      const aborted = processor.waitForProcessedSlot(11n, { signal: controller.signal });

      controller.abort();
      await expect(aborted).rejects.toMatchObject({ name: 'AbortError' });
      await vi.advanceTimersByTimeAsync(5);
      await timedOutAssertion;
      expect(processor.getProcessedSlot()).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

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

  it('prefers the full schema for complete snapshots', () => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    const processor = new FrameProcessor(storage, {
      schemas: { SparseEntity: completeEntitySchema },
      patchSchemas: { SparseEntity: partialEntitySchema },
    });

    processor.handleFrame({
      mode: 'list',
      entity: 'SparseEntity/list',
      op: 'snapshot',
      data: [{
        key: 'complete',
        data: { entity_id: 'complete', total_deposit: '7' },
      }],
      complete: true,
    } satisfies SnapshotFrame);

    expect(storage.get('SparseEntity/list', 'complete')).toEqual({
      entityId: 'complete',
      totalDeposit: 7n,
      validatedBy: 'full',
    });
  });

  it('falls back to the patch schema for sparse snapshots', () => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    const processor = new FrameProcessor(storage, {
      schemas: { SparseEntity: completeEntitySchema },
      patchSchemas: { SparseEntity: partialEntitySchema },
    });

    processor.handleFrame({
      mode: 'list',
      entity: 'SparseEntity/list',
      op: 'snapshot',
      data: [{
        key: 'sparse',
        data: { entity_id: 'sparse' },
      }],
      complete: true,
    } satisfies SnapshotFrame);

    expect(storage.get('SparseEntity/list', 'sparse')).toEqual({
      entityId: 'sparse',
      validatedBy: 'patch',
    });
  });

  it('falls back to the patch schema for sparse upserts', () => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    const processor = new FrameProcessor(storage, {
      schemas: { SparseEntity: completeEntitySchema },
      patchSchemas: { SparseEntity: partialEntitySchema },
    });

    processor.handleFrame({
      mode: 'list',
      entity: 'SparseEntity/list',
      op: 'upsert',
      key: 'sparse',
      data: { total_deposit: '9' },
    } satisfies EntityFrame);

    expect(storage.get('SparseEntity/list', 'sparse')).toEqual({
      totalDeposit: 9n,
      validatedBy: 'patch',
    });
  });

  it('prefers the patch schema for patch frames', () => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    const processor = new FrameProcessor(storage, {
      schemas: { SparseEntity: completeEntitySchema },
      patchSchemas: { SparseEntity: partialEntitySchema },
    });

    processor.handleFrame({
      mode: 'list',
      entity: 'SparseEntity/list',
      op: 'upsert',
      key: 'entity',
      data: { entity_id: 'entity', total_deposit: '1' },
    } satisfies EntityFrame);
    processor.handleFrame({
      mode: 'list',
      entity: 'SparseEntity/list',
      op: 'patch',
      key: 'entity',
      data: { entity_id: 'entity', total_deposit: '2' },
    } satisfies EntityFrame);

    expect(storage.get('SparseEntity/list', 'entity')).toEqual({
      entityId: 'entity',
      totalDeposit: 2n,
      validatedBy: 'patch',
    });
  });

  it('rejects frames that fail both full and patch schemas', () => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    const onValidationError = vi.fn();
    const processor = new FrameProcessor(storage, {
      schemas: { SparseEntity: completeEntitySchema },
      patchSchemas: { SparseEntity: partialEntitySchema },
      onValidationError,
    });
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => undefined);

    processor.handleFrame({
      mode: 'list',
      entity: 'SparseEntity/list',
      op: 'upsert',
      key: 'invalid',
      data: { entity_id: 7 },
    } satisfies EntityFrame);

    expect(storage.get('SparseEntity/list', 'invalid')).toBeNull();
    expect(warnSpy).toHaveBeenCalledWith(
      '[Arete] Frame validation failed:',
      expect.objectContaining({ view: 'SparseEntity/list' }),
    );
    expect(onValidationError).toHaveBeenCalledWith(expect.objectContaining({
      view: 'SparseEntity/list',
      key: 'invalid',
      operation: 'upsert',
    }));
    warnSpy.mockRestore();
  });

  it('reports rejected frames without logging when warnings are disabled', () => {
    const onValidationError = vi.fn();
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const processor = new FrameProcessor(new MemoryAdapter(), {
      schemas: { SparseEntity: completeEntitySchema },
      patchSchemas: { SparseEntity: partialEntitySchema },
      warnOnValidationError: false,
      onValidationError,
    });

    processor.handleFrame({
      mode: 'list',
      entity: 'SparseEntity/list',
      op: 'upsert',
      key: 'invalid',
      seq: '9:000000000001',
      data: { entity_id: 7 },
    } satisfies EntityFrame);

    expect(warnSpy).not.toHaveBeenCalled();
    expect(onValidationError).toHaveBeenCalledWith(expect.objectContaining({
      key: 'invalid',
      seq: '9:000000000001',
    }));
    warnSpy.mockRestore();
  });

  it('reports thrown schema transforms and continues a buffered batch', async () => {
    vi.useFakeTimers();
    try {
      const onValidationError = vi.fn();
      const storage = new MemoryAdapter();
      const schema = z.object({ value: z.string() }).transform((value) => {
        if (value.value === 'invalid') throw new Error('transform failed');
        return value;
      });
      const processor = new FrameProcessor(storage, {
        flushIntervalMs: 1,
        schemas: { Item: schema },
        warnOnValidationError: false,
        onValidationError,
      });

      processor.handleFrame({
        mode: 'list',
        entity: 'Item/list',
        op: 'upsert',
        key: 'invalid',
        data: { value: 'invalid' },
      } satisfies EntityFrame);
      processor.handleFrame({
        mode: 'list',
        entity: 'Item/list',
        op: 'upsert',
        key: 'valid',
        data: { value: 'valid' },
      } satisfies EntityFrame);

      await vi.advanceTimersByTimeAsync(1);
      expect(onValidationError).toHaveBeenCalledWith(expect.objectContaining({
        key: 'invalid',
        error: expect.objectContaining({ message: 'transform failed' }),
      }));
      expect(storage.get('Item/list', 'valid')).toEqual({ value: 'valid' });
    } finally {
      vi.useRealTimers();
    }
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
      protocolVersion: 2,
      subscriptionId: 'positions:all',
      op: 'subscribed',
      query: { view: 'TokenPosition/list' },
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
      protocolVersion: 2,
      subscriptionId: 'positions:all',
      op: 'subscribed',
      query: { view: 'TokenPosition/list' },
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

  it('keeps the tracked sequence when an upsert arrives without one', () => {
    // An unsequenced upsert must not disarm the staleness guard: the patch
    // branch already falls back to the stored sequence, and Python/Rust
    // retain it too. Without the fallback the older frame below would win.
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    const processor = new FrameProcessor(storage);

    const upsert = (data: unknown, seq?: string): EntityFrame => ({
      mode: 'state',
      entity: 'Thing/state',
      op: 'upsert',
      key: 'k',
      data,
      ...(seq ? { seq } : {}),
    } as EntityFrame);

    processor.handleFrame(upsert({ v: 'first' }, '50:000000000009'));
    processor.handleFrame(upsert({ v: 'second' }));
    expect(storage.get('Thing/state', 'k')).toEqual({ v: 'second' });

    // Older than the sequence recorded before the unsequenced write.
    processor.handleFrame(upsert({ v: 'third' }, '50:000000000001'));
    expect(storage.get('Thing/state', 'k')).toEqual({ v: 'second' });
  });
});
