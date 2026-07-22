import type {
  QuerySnapshot,
  RichUpdate,
  Subscription,
  SubscriptionQuery,
  UnsubscribeFn,
  Update,
} from './types';
import { AreteError } from './types';
import type { ErrorFrame, FrameMode, SortConfig, SnapshotFrame } from './frame';
import type { StorageAdapter } from './storage/adapter';

interface StagedSnapshot {
  snapshotId: string;
  authoritative: boolean;
  keys: string[];
}

interface RefreshWaiter {
  promise: Promise<void>;
  resolve: () => void;
  reject: (error: AreteError) => void;
}

interface QueryRecord {
  subscription: Subscription;
  queryKey: string;
  keys: string[];
  sequences: Map<string, string>;
  isLoading: boolean;
  isRefreshing: boolean;
  resolved: boolean;
  error?: AreteError;
  mode?: FrameMode;
  sort?: SortConfig;
  staged?: StagedSnapshot;
  refreshWaiter?: RefreshWaiter;
  listeners: Set<() => void>;
  updateListeners: Set<(update: Update<unknown>) => void>;
  richUpdateListeners: Set<(update: RichUpdate<unknown>) => void>;
  version: number;
  cachedVersion: number;
  cachedSnapshot?: QuerySnapshot<unknown>;
}

function getNestedValue(value: unknown, path: readonly string[]): unknown {
  let current = value;
  for (const segment of path) {
    if (current === null || typeof current !== 'object') return undefined;
    current = (current as Record<string, unknown>)[segment];
  }
  return current;
}

function compareValues(left: unknown, right: unknown): number {
  if (left === right) return 0;
  if (left === undefined || left === null) return -1;
  if (right === undefined || right === null) return 1;
  if (typeof left === 'number' && typeof right === 'number') return left - right;
  if (typeof left === 'bigint' && typeof right === 'bigint') return left < right ? -1 : 1;
  if (typeof left === 'boolean' && typeof right === 'boolean') {
    return Number(left) - Number(right);
  }
  return String(left).localeCompare(String(right));
}

function compareSequences(left: unknown, right: unknown): number {
  if (typeof left !== 'string' || typeof right !== 'string') return compareValues(left, right);
  const [leftSlot, leftIndex = ''] = left.split(':', 2);
  const [rightSlot, rightIndex = ''] = right.split(':', 2);
  if (/^\d+$/.test(leftSlot ?? '') && /^\d+$/.test(rightSlot ?? '')) {
    const leftValue = BigInt(leftSlot!);
    const rightValue = BigInt(rightSlot!);
    if (leftValue !== rightValue) return leftValue < rightValue ? -1 : 1;
  }
  return leftIndex.localeCompare(rightIndex);
}

function appendUnique(target: string[], keys: readonly string[]): void {
  for (const key of keys) {
    if (!target.includes(key)) target.push(key);
  }
}

export class QueryStore {
  private readonly records = new Map<string, QueryRecord>();

  constructor(private readonly storage: StorageAdapter) {}

  register(subscription: Subscription, queryKey: string): void {
    if (this.records.has(subscription.subscriptionId)) return;
    this.records.set(subscription.subscriptionId, {
      subscription,
      queryKey,
      keys: [],
      sequences: new Map(),
      isLoading: subscription.snapshot.enabled,
      isRefreshing: false,
      resolved: !subscription.snapshot.enabled,
      listeners: new Set(),
      updateListeners: new Set(),
      richUpdateListeners: new Set(),
      version: 0,
      cachedVersion: -1,
    });
  }

  unregister(subscriptionId: string): void {
    const record = this.records.get(subscriptionId);
    if (!record) return;
    this.rejectRefresh(
      record,
      new AreteError('Subscription was released while refreshing', 'SUBSCRIPTION_NOT_FOUND')
    );
    this.records.delete(subscriptionId);
  }

  getSubscription(subscriptionId: string): Subscription | undefined {
    return this.records.get(subscriptionId)?.subscription;
  }

  getQueryKey(subscriptionId: string): string | undefined {
    return this.records.get(subscriptionId)?.queryKey;
  }

  getSnapshot<T>(subscriptionId: string): QuerySnapshot<T> | undefined {
    const record = this.records.get(subscriptionId);
    if (!record) return undefined;
    if (record.cachedSnapshot && record.cachedVersion === record.version) {
      return record.cachedSnapshot as QuerySnapshot<T>;
    }

    const data = record.keys.flatMap((key) => {
      const entity = this.storage.get<T>(record.subscription.query.view, key);
      return entity === null ? [] : [entity];
    });
    const snapshot: QuerySnapshot<T> = {
      subscriptionId,
      query: record.subscription.query,
      keys: record.keys,
      data,
      isLoading: record.isLoading,
      isRefreshing: record.isRefreshing,
      ...(record.error ? { error: record.error } : {}),
    };
    record.cachedSnapshot = snapshot as QuerySnapshot<unknown>;
    record.cachedVersion = record.version;
    return snapshot;
  }

  onChange(subscriptionId: string, callback: () => void): UnsubscribeFn {
    const listeners = this.requireRecord(subscriptionId).listeners;
    listeners.add(callback);
    return () => listeners.delete(callback);
  }

  onUpdate<T>(subscriptionId: string, callback: (update: Update<T>) => void): UnsubscribeFn {
    const listeners = this.requireRecord(subscriptionId).updateListeners;
    listeners.add(callback as (update: Update<unknown>) => void);
    return () => listeners.delete(callback as (update: Update<unknown>) => void);
  }

  onRichUpdate<T>(
    subscriptionId: string,
    callback: (update: RichUpdate<T>) => void
  ): UnsubscribeFn {
    const listeners = this.requireRecord(subscriptionId).richUpdateListeners;
    listeners.add(callback as (update: RichUpdate<unknown>) => void);
    return () => listeners.delete(callback as (update: RichUpdate<unknown>) => void);
  }

  acknowledge(
    subscriptionId: string,
    effectiveQuery: SubscriptionQuery,
    mode: FrameMode,
    sort?: SortConfig
  ): void {
    const record = this.records.get(subscriptionId);
    if (!record) return;
    record.mode = mode;
    record.sort = sort;
    record.error = undefined;
    if (!record.subscription.snapshot.enabled) {
      record.isLoading = false;
      record.isRefreshing = false;
      record.resolved = true;
    }
    if (effectiveQuery.view !== record.subscription.query.view) {
      this.fail(
        subscriptionId,
        new AreteError('Server acknowledged a different view for the subscription', 'INVALID_FRAME')
      );
      return;
    }
    this.touch(record);
  }

  stageSnapshot(frame: SnapshotFrame, keys: readonly string[]): void {
    const record = this.records.get(frame.subscriptionId);
    if (!record) return;
    if (
      !record.staged
      || record.staged.snapshotId !== frame.snapshotId
    ) {
      record.staged = {
        snapshotId: frame.snapshotId,
        authoritative: frame.authoritative,
        keys: [],
      };
    }
    const staged = record.staged;
    if (staged.authoritative !== frame.authoritative) {
      this.fail(
        frame.subscriptionId,
        new AreteError('Snapshot batches disagree on authoritative mode', 'INVALID_FRAME')
      );
      record.staged = undefined;
      return;
    }
    appendUnique(staged.keys, keys);
    if (!frame.complete) return;

    const stagedKeys = staged.keys;
    if (frame.authoritative) {
      const retained = new Set(stagedKeys);
      for (const key of record.sequences.keys()) {
        if (!retained.has(key)) record.sequences.delete(key);
      }
    }
    for (const key of stagedKeys) {
      const sequence = this.entitySequence(record.subscription.query.view, key);
      if (sequence !== undefined) record.sequences.set(key, sequence);
    }
    record.keys = frame.authoritative
      ? [...stagedKeys]
      : this.mergeIncremental(record.keys, stagedKeys);
    record.staged = undefined;
    record.isLoading = false;
    record.isRefreshing = false;
    record.resolved = true;
    record.error = undefined;
    this.touch(record);
    this.resolveRefresh(record);

    for (const key of stagedKeys) {
      const data = this.storage.get<unknown>(record.subscription.query.view, key);
      if (data !== null) {
        this.emitUpdate(record, { type: 'upsert', key, data });
        this.emitRichUpdate(record, { type: 'created', key, data });
      }
    }
  }

  applyLive(
    subscriptionId: string,
    key: string,
    update: Update<unknown>,
    richUpdate?: RichUpdate<unknown>,
    sequence?: string
  ): void {
    const record = this.records.get(subscriptionId);
    if (!record) return;

    if (update.type === 'remove') {
      const lastKnown = this.storage.get<unknown>(record.subscription.query.view, key) ?? undefined;
      record.keys = record.keys.filter((entry) => entry !== key);
      record.sequences.delete(key);
      this.touch(record);
      this.emitUpdate(record, update);
      this.emitRichUpdate(record, { type: 'removed', key, lastKnown });
      return;
    }

    if (!record.keys.includes(key)) record.keys = [...record.keys, key];
    const nextSequence = sequence ?? this.entitySequence(record.subscription.query.view, key);
    if (nextSequence !== undefined) record.sequences.set(key, nextSequence);
    this.sortKeys(record);
    record.isLoading = false;
    if (!record.refreshWaiter) record.isRefreshing = false;
    record.resolved = true;
    record.error = undefined;
    this.touch(record);
    this.emitUpdate(record, update);
    if (richUpdate) this.emitRichUpdate(record, richUpdate);
  }

  deleteGlobal(view: string, key: string, lastKnown?: unknown): void {
    for (const record of this.records.values()) {
      if (record.subscription.query.view !== view || !record.keys.includes(key)) continue;
      record.keys = record.keys.filter((entry) => entry !== key);
      record.sequences.delete(key);
      this.touch(record);
      this.emitUpdate(record, { type: 'delete', key });
      this.emitRichUpdate(record, { type: 'deleted', key, lastKnown });
    }
  }

  evict(view: string, key: string): void {
    for (const record of this.records.values()) {
      if (record.subscription.query.view !== view || !record.keys.includes(key)) continue;
      record.keys = record.keys.filter((entry) => entry !== key);
      record.sequences.delete(key);
      this.touch(record);
    }
  }

  failFrame(frame: ErrorFrame): void {
    if (frame.subscriptionId === null) return;
    this.fail(
      frame.subscriptionId,
      new AreteError(frame.message ?? frame.error ?? frame.code, frame.code, frame)
    );
  }

  beginRefresh(subscriptionId: string, waitForSnapshot = false): Promise<void> {
    const record = this.requireRecord(subscriptionId);
    record.error = undefined;
    record.isLoading = !record.resolved && record.subscription.snapshot.enabled;
    record.isRefreshing = record.resolved && record.subscription.snapshot.enabled;
    record.staged = undefined;
    this.touch(record);

    if (!waitForSnapshot || !record.subscription.snapshot.enabled) {
      return Promise.resolve();
    }
    if (record.refreshWaiter) return record.refreshWaiter.promise;

    let resolve!: () => void;
    let reject!: (error: AreteError) => void;
    const promise = new Promise<void>((resolvePromise, rejectPromise) => {
      resolve = resolvePromise;
      reject = rejectPromise;
    });
    record.refreshWaiter = { promise, resolve, reject };
    return promise;
  }

  failRefresh(subscriptionId: string, error: AreteError): void {
    this.fail(subscriptionId, error);
  }

  beginReconnect(): void {
    for (const record of this.records.values()) {
      this.beginRefresh(record.subscription.subscriptionId);
    }
  }

  failRefreshing(error: AreteError): void {
    for (const [subscriptionId, record] of this.records) {
      if (record.isRefreshing || record.refreshWaiter) {
        this.fail(subscriptionId, error);
      }
    }
  }

  clear(): void {
    for (const record of this.records.values()) {
      this.rejectRefresh(
        record,
        new AreteError('Subscription store was cleared while refreshing', 'CONNECTION_CANCELLED')
      );
    }
    this.records.clear();
  }

  private mergeIncremental(existing: readonly string[], incoming: readonly string[]): string[] {
    const result = [...existing];
    appendUnique(result, incoming);
    return result;
  }

  private sortKeys(record: QueryRecord): void {
    if (record.keys.length < 2) return;
    if (!record.sort && record.mode !== 'list' && record.mode !== 'append') return;
    const field = record.sort?.field;
    const order = record.sort?.order ?? (record.subscription.query.after ? 'asc' : 'desc');
    const view = record.subscription.query.view;
    record.keys = [...record.keys].sort((leftKey, rightKey) => {
      const left = field
        ? getNestedValue(this.storage.get(view, leftKey), field)
        : record.sequences.get(leftKey);
      const right = field
        ? getNestedValue(this.storage.get(view, rightKey), field)
        : record.sequences.get(rightKey);
      let compared = field ? compareValues(left, right) : compareSequences(left, right);
      if (order === 'desc') compared = -compared;
      return compared === 0 ? leftKey.localeCompare(rightKey) : compared;
    });
  }

  private entitySequence(view: string, key: string): string | undefined {
    const entity = this.storage.get<unknown>(view, key);
    if (entity === null || typeof entity !== 'object') return undefined;
    const value = entity as Record<string, unknown>;
    const sequence = value['__seq'] ?? value['_seq'];
    return typeof sequence === 'string' ? sequence : undefined;
  }

  private fail(subscriptionId: string, error: AreteError): void {
    const record = this.records.get(subscriptionId);
    if (!record) return;
    record.error = error;
    record.isLoading = false;
    record.isRefreshing = false;
    record.staged = undefined;
    this.touch(record);
    this.rejectRefresh(record, error);
  }

  private resolveRefresh(record: QueryRecord): void {
    const waiter = record.refreshWaiter;
    if (!waiter) return;
    record.refreshWaiter = undefined;
    waiter.resolve();
  }

  private rejectRefresh(record: QueryRecord, error: AreteError): void {
    const waiter = record.refreshWaiter;
    if (!waiter) return;
    record.refreshWaiter = undefined;
    waiter.reject(error);
  }

  private requireRecord(subscriptionId: string): QueryRecord {
    const record = this.records.get(subscriptionId);
    if (!record) {
      throw new AreteError(`Unknown local subscription '${subscriptionId}'`, 'SUBSCRIPTION_NOT_FOUND');
    }
    return record;
  }

  private touch(record: QueryRecord): void {
    record.version++;
    for (const listener of record.listeners) listener();
  }

  private emitUpdate(record: QueryRecord, update: Update<unknown>): void {
    for (const listener of record.updateListeners) listener(update);
  }

  private emitRichUpdate(record: QueryRecord, update: RichUpdate<unknown>): void {
    for (const listener of record.richUpdateListeners) listener(update);
  }
}
