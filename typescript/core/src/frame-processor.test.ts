import { describe, expect, it, vi } from 'vitest';
import { z } from 'zod';

import type { EntityFrame, SnapshotFrame, SubscribedFrame } from './frame';
import {
  FrameProcessor,
  ProcessedSlotTimeoutError,
  type FrameValidationDiagnostic,
} from './frame-processor';
import { QueryStore } from './query-store';
import { SortedStorageDecorator } from './storage/sorted-decorator';
import { MemoryAdapter } from './storage/memory-adapter';
import type { Subscription, Update } from './types';

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

  it('keeps the tracked version when a frame arrives without one', () => {
    const storage = new MemoryAdapter();
    const processor = new FrameProcessor(storage);
    const frame = (op: 'upsert' | 'patch', data: Record<string, unknown>, seq: string): EntityFrame => ({
      mode: 'state',
      entity: 'Thing/state',
      op,
      key: 'k',
      data,
      seq,
    } as EntityFrame);

    processor.handleFrame(frame('upsert', { v: 'first', _version: '3f9a2c1d:5' }, '50:000000000009'));
    // No version: the seq rule decides, and this seq is newer.
    processor.handleFrame(frame('patch', { w: 'second' }, '51:000000000001'));
    expect(storage.get('Thing/state', 'k')).toMatchObject({ v: 'first', w: 'second' });

    // The version recorded before the unversioned write still orders frames.
    processor.handleFrame(frame('patch', { v: 'stale', _version: '3f9a2c1d:4' }, '52:000000000001'));
    expect(storage.get('Thing/state', 'k')).toMatchObject({ v: 'first', w: 'second' });
  });

  it('never treats a version it cannot parse as stale', () => {
    const storage = new MemoryAdapter();
    const processor = new FrameProcessor(storage);
    const frame = (data: Record<string, unknown>): EntityFrame => ({
      mode: 'state',
      entity: 'Thing/state',
      op: 'patch',
      key: 'k',
      data,
      seq: '50:000000000001',
    } as EntityFrame);

    processor.handleFrame({ ...frame({ v: 1, _version: '3f9a2c1d:5' }), op: 'upsert' } as EntityFrame);
    processor.handleFrame(frame({ v: 2, _version: 'not-a-version' }));
    expect(storage.get('Thing/state', 'k')).toMatchObject({ v: 2 });
  });
});

describe('FrameProcessor patches for keys the client does not hold', () => {
  const VIEW = 'Round/list';

  function setup(maxEntriesPerView?: number, { wholeEntities = true }: { wholeEntities?: boolean } = {}) {
    const storage = new MemoryAdapter();
    const queries = new QueryStore(storage);
    const diagnostics: FrameValidationDiagnostic[] = [];
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const processor = new FrameProcessor(storage, {
      queryStore: queries,
      onValidationError: (diagnostic) => diagnostics.push(diagnostic),
      ...(maxEntriesPerView !== undefined ? { maxEntriesPerView } : {}),
    });
    const subscription: Subscription = {
      type: 'subscribe',
      protocolVersion: 2,
      subscriptionId: 'rounds',
      query: { view: VIEW },
      snapshot: { enabled: false },
    };
    queries.register(subscription, 'rounds');
    const updates: Update<unknown>[] = [];
    queries.onUpdate('rounds', (update) => updates.push(update));
    const frame = (op: 'upsert' | 'patch', key: string, data: unknown, seq?: string): EntityFrame => ({
      subscriptionId: 'rounds',
      mode: 'list',
      entity: VIEW,
      op,
      key,
      data,
      ...(seq ? { seq } : {}),
    } as EntityFrame);
    const acknowledge = (guarantee: boolean) => processor.handleFrame({
      protocolVersion: 2,
      subscriptionId: 'rounds',
      op: 'subscribed',
      query: { view: VIEW },
      mode: 'list',
      ...(guarantee ? { wholeEntities: true } : {}),
    } satisfies SubscribedFrame);
    acknowledge(wholeEntities);
    return { storage, queries, processor, diagnostics, updates, frame, warn, acknowledge };
  }

  it('ignores a patch for a key that was never received', () => {
    const { storage, queries, processor, diagnostics, updates, frame, warn } = setup();

    processor.handleFrame(frame('patch', '7', { motherlode: 3 }, '100:000000000001'));

    expect(storage.get(VIEW, '7')).toBeNull();
    expect(queries.getSnapshot('rounds')?.keys).toEqual([]);
    expect(updates).toEqual([]);
    expect(diagnostics).toEqual([expect.objectContaining({
      view: VIEW,
      key: '7',
      seq: '100:000000000001',
      operation: 'patch',
      reason: 'unknown-key',
      error: expect.objectContaining({ code: 'PATCH_FOR_UNKNOWN_KEY' }),
    })]);
    expect(warn).not.toHaveBeenCalled();
    warn.mockRestore();
  });

  it('merges patches once a full upsert has arrived', () => {
    const { storage, queries, processor, frame, warn } = setup();

    processor.handleFrame(frame('patch', '7', { motherlode: 3 }, '100:000000000001'));
    processor.handleFrame(frame('upsert', '7', { id: '7', motherlode: 1, deployed: 5 }, '101:000000000001'));
    processor.handleFrame(frame('patch', '7', { motherlode: 4 }, '102:000000000001'));

    expect(storage.get(VIEW, '7')).toEqual({ id: '7', motherlode: 4, deployed: 5 });
    expect(queries.getSnapshot('rounds')?.keys).toEqual(['7']);
    warn.mockRestore();
  });

  it('ignores a patch for a key evicted by maxEntriesPerView, and says so', () => {
    const { storage, queries, processor, diagnostics, frame, warn } = setup(1);

    processor.handleFrame(frame('upsert', 'old', { id: 'old', value: 1 }, '1:000000000001'));
    processor.handleFrame(frame('upsert', 'new', { id: 'new', value: 2 }, '2:000000000001'));
    expect(storage.get(VIEW, 'old')).toBeNull();

    processor.handleFrame(frame('patch', 'old', { value: 3 }, '3:000000000001'));
    processor.handleFrame(frame('patch', 'old', { value: 3.5 }, '3:000000000002'));

    expect(storage.get(VIEW, 'old')).toBeNull();
    expect(queries.getSnapshot('rounds')?.keys).toEqual(['new']);
    expect(diagnostics.map(({ key, reason, error }) => ({
      key,
      reason,
      code: (error as { code?: string }).code,
    }))).toEqual([
      { key: 'old', reason: 'evicted-key', code: 'PATCH_FOR_EVICTED_KEY' },
      { key: 'old', reason: 'evicted-key', code: 'PATCH_FOR_EVICTED_KEY' },
    ]);
    // The server still thinks the client holds it, so this is worth one
    // warning per view.
    expect(warn).toHaveBeenCalledTimes(1);

    // The entity comes back with a full upsert, and later patches merge again.
    processor.handleFrame(frame('upsert', 'old', { id: 'old', value: 4 }, '4:000000000001'));
    processor.handleFrame(frame('patch', 'old', { value: 5 }, '5:000000000001'));
    expect(storage.get(VIEW, 'old')).toEqual({ id: 'old', value: 5 });
    warn.mockRestore();
  });

  it('keeps a patch for an unknown key from a server without the guarantee', () => {
    // An older server may send a key's first change as a patch (after a
    // truncated or disabled snapshot); that patch is all the client will get.
    const { storage, queries, processor, diagnostics, frame, warn } = setup(undefined, {
      wholeEntities: false,
    });

    processor.handleFrame(frame('patch', '7', { motherlode: 3 }, '100:000000000001'));

    expect(storage.get(VIEW, '7')).toEqual({ motherlode: 3 });
    expect(queries.getSnapshot('rounds')?.keys).toEqual(['7']);
    expect(diagnostics).toEqual([]);
    warn.mockRestore();
  });

  it('follows the latest acknowledgement, as after a reconnect to another server', () => {
    const { storage, processor, frame, acknowledge, warn } = setup();

    processor.handleFrame(frame('patch', 'a', { value: 1 }, '1:000000000001'));
    expect(storage.get(VIEW, 'a')).toBeNull();

    acknowledge(false);
    processor.handleFrame(frame('patch', 'b', { value: 2 }, '2:000000000001'));
    expect(storage.get(VIEW, 'b')).toEqual({ value: 2 });

    acknowledge(true);
    processor.handleFrame(frame('patch', 'c', { value: 3 }, '3:000000000001'));
    expect(storage.get(VIEW, 'c')).toBeNull();
    warn.mockRestore();
  });

  it('still applies replayable append-view records, which are events', () => {
    const { storage, processor, diagnostics, warn } = setup();

    processor.handleFrame({
      subscriptionId: 'rounds',
      mode: 'append',
      entity: VIEW,
      op: 'patch',
      key: 'trade',
      data: { amount: 100 },
      offset: 4209,
    } satisfies EntityFrame);

    expect(storage.get(VIEW, 'trade')).toEqual({ amount: 100 });
    expect(diagnostics).toEqual([]);
    warn.mockRestore();
  });
});
