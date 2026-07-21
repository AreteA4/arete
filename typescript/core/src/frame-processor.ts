import type { Frame, SnapshotFrame, EntityFrame, SubscribedFrame } from './frame';
import {
  isEntityFrame,
  isErrorFrame,
  isSnapshotFrame,
  isSubscribedFrame,
  isUnsubscribedFrame,
} from './frame';
import type { StorageAdapter } from './storage/adapter';
import type { RichUpdate, Schema, Update } from './types';
import { DEFAULT_MAX_ENTRIES_PER_VIEW } from './types';
import type { QueryStore } from './query-store';

const INTERNAL_SEQ_FIELD = '__seq';

export interface WaitForProcessedSlotOptions {
  /** Reject if the requested slot has not been processed within this duration. */
  timeoutMs?: number;
  /** Abort this wait without affecting frame processing or the connection. */
  signal?: AbortSignal;
}

export class ProcessedSlotTimeoutError extends Error {
  readonly targetSlot: bigint;
  readonly processedSlot: bigint | null;

  constructor(targetSlot: bigint, processedSlot: bigint | null) {
    super(`Timed out waiting for Arete to process slot ${targetSlot}`);
    this.name = 'ProcessedSlotTimeoutError';
    this.targetSlot = targetSlot;
    this.processedSlot = processedSlot;
  }
}

interface ProcessedSlotWaiter {
  targetSlot: bigint;
  resolve: (slot: bigint) => void;
  reject: (error: Error) => void;
  timeout: ReturnType<typeof setTimeout> | null;
  signal?: AbortSignal;
  onAbort?: () => void;
}

export interface FrameProcessorConfig {
  maxEntriesPerView?: number | null;
  /**
   * Interval in milliseconds to buffer frames before flushing to storage.
   * Set to 0 for immediate processing (no buffering).
   * Default: 0 (immediate)
   *
   * For React applications, 16ms (one frame at 60fps) is recommended to
   * reduce unnecessary re-renders during high-frequency updates.
   */
  flushIntervalMs?: number;
  schemas?: Record<string, Schema<unknown>>;
  patchSchemas?: Record<string, Schema<unknown>>;
  queryStore?: QueryStore;
  /** Whether rejected frames should also be written to `console.warn`. */
  warnOnValidationError?: boolean;
  /** Receives structured details whenever a generated schema rejects a frame. */
  onValidationError?: (diagnostic: FrameValidationDiagnostic) => void;
}

export interface FrameValidationDiagnostic {
  readonly view: string;
  readonly key?: string;
  readonly seq?: string;
  readonly operation: 'snapshot' | 'upsert' | 'patch';
  readonly error: unknown;
}

interface PendingUpdate<T = unknown> {
  frame: Frame<T>;
}

function isObject(item: unknown): item is Record<string, unknown> {
  return item !== null && typeof item === 'object' && !Array.isArray(item);
}

function deepMergeWithAppend<T>(
  target: T,
  source: Partial<T>,
  appendPaths: string[],
  currentPath = ''
): T {
  if (!isObject(target) || !isObject(source)) {
    return source as T;
  }

  const result = { ...target } as Record<string, unknown>;

  for (const key in source) {
    const sourceValue = source[key];
    if (sourceValue === undefined) {
      continue;
    }

    const targetValue = result[key];
    const fieldPath = currentPath ? `${currentPath}.${key}` : key;

    if (Array.isArray(sourceValue) && Array.isArray(targetValue)) {
      if (appendPaths.includes(fieldPath)) {
        result[key] = [...targetValue, ...sourceValue];
      } else {
        result[key] = sourceValue;
      }
    } else if (isObject(sourceValue) && isObject(targetValue)) {
      result[key] = deepMergeWithAppend(
        targetValue,
        sourceValue as Record<string, unknown>,
        appendPaths,
        fieldPath
      );
    } else {
      result[key] = sourceValue;
    }
  }

  return result as T;
}

function stripUndefinedProperties<T>(value: T): T {
  if (Array.isArray(value)) {
    return value.map(item => stripUndefinedProperties(item)) as T;
  }

  if (!isObject(value)) {
    return value;
  }

  const result: Record<string, unknown> = {};
  for (const [key, nestedValue] of Object.entries(value)) {
    if (nestedValue === undefined) {
      continue;
    }

    result[key] = stripUndefinedProperties(nestedValue);
  }

  return result as T;
}

function toCamelCaseSegment(value: string): string {
  if (value === '_seq') {
    return INTERNAL_SEQ_FIELD;
  }

  const pascal = value
    .split(/[_.-]/)
    .filter(segment => segment.length > 0)
    .map(segment => segment[0]!.toUpperCase() + segment.slice(1))
    .join('');

  if (pascal.length === 0) {
    return value;
  }

  return pascal[0]!.toLowerCase() + pascal.slice(1);
}

export class FrameProcessor {
  private storage: StorageAdapter;
  private maxEntriesPerView: number | null;
  private flushIntervalMs: number;
  private schemas?: Record<string, Schema<unknown>>;
  private patchSchemas?: Record<string, Schema<unknown>>;
  private queryStore?: QueryStore;
  private warnOnValidationError: boolean;
  private onValidationError?: (diagnostic: FrameValidationDiagnostic) => void;
  private pendingUpdates: PendingUpdate[] = [];
  private flushTimer: ReturnType<typeof setTimeout> | null = null;
  private isProcessing = false;
  private latestProcessedSlot: bigint | null = null;
  private processedSlotWaiters = new Set<ProcessedSlotWaiter>();

  constructor(storage: StorageAdapter, config: FrameProcessorConfig = {}) {
    this.storage = storage;
    this.maxEntriesPerView = config.maxEntriesPerView === undefined
      ? DEFAULT_MAX_ENTRIES_PER_VIEW
      : config.maxEntriesPerView;
    this.flushIntervalMs = config.flushIntervalMs ?? 0;
    this.schemas = config.schemas;
    this.patchSchemas = config.patchSchemas;
    this.queryStore = config.queryStore;
    this.warnOnValidationError = config.warnOnValidationError ?? true;
    this.onValidationError = config.onValidationError;
  }

  private getSchema(viewPath: string, patch = false): Schema<unknown> | null {
    const schemas = patch ? this.patchSchemas : this.schemas;
    if (!schemas) return null;
    const entityName = viewPath.split('/')[0];
    if (typeof entityName !== 'string' || entityName.length === 0) return null;
    const entityKey: string = entityName;
    return schemas[entityKey] ?? null;
  }

  private normalizeEntity<T>(
    viewPath: string,
    data: unknown,
    context: Omit<FrameValidationDiagnostic, 'view' | 'error'>,
    patch = false
  ): T | null {
    const fullSchema = this.getSchema(viewPath);
    const patchSchema = this.getSchema(viewPath, true);
    const schemas = patch
      ? [patchSchema, fullSchema]
      : [fullSchema, patchSchema];
    let validationError: unknown;

    for (const schema of schemas) {
      if (!schema) continue;

      try {
        const result = schema.safeParse(data);
        if (result.success) {
          return stripUndefinedProperties(result.data as T);
        }
        validationError = result.error;
      } catch (error) {
        validationError = error;
      }
    }

    if (!fullSchema && !patchSchema) return data as T;

    const diagnostic: FrameValidationDiagnostic = {
      view: viewPath,
      ...context,
      error: validationError,
    };
    try {
      this.onValidationError?.(diagnostic);
    } catch (callbackError) {
      console.error('[Arete] Frame validation callback failed:', callbackError);
    }
    if (this.warnOnValidationError) {
      console.warn('[Arete] Frame validation failed:', diagnostic);
    }
    return null;
  }

  private hasSchema(viewPath: string): boolean {
    return this.getSchema(viewPath) !== null;
  }

  private normalizePathSegment(viewPath: string, segment: string): string {
    if (!this.hasSchema(viewPath)) {
      return segment;
    }

    return toCamelCaseSegment(segment);
  }

  private normalizePath(viewPath: string, path: string): string {
    if (!this.hasSchema(viewPath)) {
      return path;
    }

    return path
      .split('.')
      .filter(segment => segment.length > 0)
      .map(segment => this.normalizePathSegment(viewPath, segment))
      .join('.');
  }

  private normalizeSortConfig(viewPath: string, sort: NonNullable<SubscribedFrame['sort']>) {
    if (!this.hasSchema(viewPath)) {
      return sort;
    }

    return {
      ...sort,
      field: sort.field.map(segment => this.normalizePathSegment(viewPath, segment)),
    };
  }

  private normalizeAppendPaths(viewPath: string, append: string[] | undefined): string[] {
    if (!append || append.length === 0 || !this.hasSchema(viewPath)) {
      return append ?? [];
    }

    return append.map(path => this.normalizePath(viewPath, path));
  }

  private extractSeq(data: unknown): string | undefined {
    if (!isObject(data)) {
      return undefined;
    }

    const seq = data._seq;
    if (typeof seq === 'string') {
      return seq;
    }

    if (typeof seq === 'number' && Number.isFinite(seq)) {
      return String(seq);
    }

    return undefined;
  }

  private getInternalSeq(data: unknown): string | undefined {
    if (!isObject(data)) {
      return undefined;
    }

    const seq = (data as Record<string, unknown>)[INTERNAL_SEQ_FIELD];
    return typeof seq === 'string' ? seq : undefined;
  }

  private compareSeq(left: string, right: string): number {
    const [leftSlot, leftIndex = ''] = left.split(':', 2);
    const [rightSlot, rightIndex = ''] = right.split(':', 2);
    if (/^\d+$/.test(leftSlot ?? '') && /^\d+$/.test(rightSlot ?? '')) {
      const leftValue = BigInt(leftSlot!);
      const rightValue = BigInt(rightSlot!);
      if (leftValue !== rightValue) return leftValue < rightValue ? -1 : 1;
    }
    return leftIndex.localeCompare(rightIndex);
  }

  private attachInternalSeq<T>(_viewPath: string, data: T, seq?: string): T {
    if (!seq || !isObject(data)) {
      return data;
    }

    Object.defineProperty(data, INTERNAL_SEQ_FIELD, {
      value: seq,
      enumerable: false,
      configurable: true,
      writable: true,
    });

    return data;
  }

  handleFrame<T>(frame: Frame<T>): void {
    if (this.flushIntervalMs === 0) {
      this.processFrame(frame);
      return;
    }

    this.pendingUpdates.push({ frame });
    this.scheduleFlush();
  }

  /**
   * Immediately flush all pending updates.
   * Useful for ensuring all updates are processed before reading state.
   */
  flush(): void {
    if (this.flushTimer !== null) {
      clearTimeout(this.flushTimer);
      this.flushTimer = null;
    }
    this.flushPendingUpdates();
  }

  getProcessedSlot(): bigint | null {
    return this.latestProcessedSlot;
  }

  waitForProcessedSlot(
    slot: number | bigint,
    options: WaitForProcessedSlotOptions = {}
  ): Promise<bigint> {
    const targetSlot = this.normalizeSlot(slot);
    if (this.latestProcessedSlot !== null && this.latestProcessedSlot >= targetSlot) {
      return Promise.resolve(this.latestProcessedSlot);
    }
    if (options.signal?.aborted) {
      return Promise.reject(this.createAbortError());
    }
    if (
      options.timeoutMs !== undefined
      && (!Number.isFinite(options.timeoutMs) || options.timeoutMs < 0)
    ) {
      return Promise.reject(new RangeError('timeoutMs must be a non-negative finite number'));
    }

    return new Promise<bigint>((resolve, reject) => {
      const waiter: ProcessedSlotWaiter = {
        targetSlot,
        resolve,
        reject,
        timeout: null,
        signal: options.signal,
      };
      const rejectWaiter = (error: Error) => {
        this.removeProcessedSlotWaiter(waiter);
        reject(error);
      };

      if (options.timeoutMs !== undefined) {
        waiter.timeout = setTimeout(() => {
          rejectWaiter(new ProcessedSlotTimeoutError(targetSlot, this.latestProcessedSlot));
        }, options.timeoutMs);
      }
      if (options.signal) {
        waiter.onAbort = () => rejectWaiter(this.createAbortError());
        options.signal.addEventListener('abort', waiter.onAbort, { once: true });
      }
      this.processedSlotWaiters.add(waiter);
    });
  }

  /**
   * Clean up any pending timers. Call when disposing the processor.
   */
  dispose(): void {
    if (this.flushTimer !== null) {
      clearTimeout(this.flushTimer);
      this.flushTimer = null;
    }
    this.pendingUpdates = [];
  }

  private scheduleFlush(): void {
    if (this.flushTimer !== null) {
      return;
    }

    this.flushTimer = setTimeout(() => {
      this.flushTimer = null;
      this.flushPendingUpdates();
    }, this.flushIntervalMs);
  }

  private flushPendingUpdates(): void {
    if (this.isProcessing || this.pendingUpdates.length === 0) {
      return;
    }

    this.isProcessing = true;

    const batch = this.pendingUpdates;
    this.pendingUpdates = [];

    try {
      const viewsToEnforce = new Set<string>();

      for (const { frame } of batch) {
        const viewPath = this.processFrameWithoutEnforce(frame);
        if (viewPath) {
          viewsToEnforce.add(viewPath);
        }
      }

      viewsToEnforce.forEach((viewPath) => {
        this.enforceMaxEntries(viewPath);
      });

      for (const { frame } of batch) {
        this.markFrameProcessed(frame);
      }
    } finally {
      this.isProcessing = false;
    }
  }

  private processFrame<T>(frame: Frame<T>): void {
    if (isErrorFrame(frame)) {
      this.queryStore?.failFrame(frame);
    } else if (isUnsubscribedFrame(frame)) {
      // Local lease state owns cancellation; the acknowledgement is informational.
    } else if (isSubscribedFrame(frame)) {
      this.handleSubscribedFrame(frame);
    } else if (isSnapshotFrame(frame)) {
      this.handleSnapshotFrame(frame);
    } else if (isEntityFrame(frame)) {
      this.handleEntityFrame(frame);
    }
    this.markFrameProcessed(frame);
  }

  private normalizeSlot(slot: number | bigint): bigint {
    if (typeof slot === 'number') {
      if (!Number.isSafeInteger(slot) || slot < 0) {
        throw new RangeError('slot must be a non-negative safe integer or bigint');
      }
      return BigInt(slot);
    }
    if (slot < 0n) {
      throw new RangeError('slot must be non-negative');
    }
    return slot;
  }

  private slotFromSeq(seq: unknown): bigint | null {
    if (typeof seq !== 'string') {
      return null;
    }
    const slot = seq.split(':', 1)[0];
    return slot && /^\d+$/.test(slot) ? BigInt(slot) : null;
  }

  private frameSlot(frame: Frame): bigint | null {
    if (isSubscribedFrame(frame) || isUnsubscribedFrame(frame) || isErrorFrame(frame)) {
      return null;
    }
    if (isSnapshotFrame(frame)) {
      let latest: bigint | null = null;
      for (const entity of frame.data) {
        const slot = isObject(entity.data)
          ? this.slotFromSeq(entity.data._seq)
          : null;
        if (slot !== null && (latest === null || slot > latest)) {
          latest = slot;
        }
      }
      return latest;
    }
    return this.slotFromSeq(frame.seq)
      ?? (isObject(frame.data) ? this.slotFromSeq(frame.data._seq) : null);
  }

  private markFrameProcessed(frame: Frame): void {
    const slot = this.frameSlot(frame);
    if (slot === null || (this.latestProcessedSlot !== null && slot <= this.latestProcessedSlot)) {
      return;
    }

    this.latestProcessedSlot = slot;
    for (const waiter of [...this.processedSlotWaiters]) {
      if (slot >= waiter.targetSlot) {
        this.removeProcessedSlotWaiter(waiter);
        waiter.resolve(slot);
      }
    }
  }

  private removeProcessedSlotWaiter(waiter: ProcessedSlotWaiter): void {
    this.processedSlotWaiters.delete(waiter);
    if (waiter.timeout !== null) {
      clearTimeout(waiter.timeout);
    }
    if (waiter.signal && waiter.onAbort) {
      waiter.signal.removeEventListener('abort', waiter.onAbort);
    }
  }

  private createAbortError(): Error {
    const error = new Error('Waiting for processed slot was aborted');
    error.name = 'AbortError';
    return error;
  }

  private processFrameWithoutEnforce<T>(frame: Frame<T>): string | null {
    if (isErrorFrame(frame)) {
      this.queryStore?.failFrame(frame);
      return null;
    } else if (isUnsubscribedFrame(frame)) {
      return null;
    } else if (isSubscribedFrame(frame)) {
      this.handleSubscribedFrame(frame);
      return null;
    } else if (isSnapshotFrame(frame)) {
      this.handleSnapshotFrameWithoutEnforce(frame);
      return frame.entity;
    } else if (isEntityFrame(frame)) {
      this.handleEntityFrameWithoutEnforce(frame);
      return frame.entity;
    }
    return null;
  }

  private handleSubscribedFrame(frame: SubscribedFrame): void {
    const viewPath = frame.query.view;
    if (this.storage.setViewConfig && frame.sort) {
      this.storage.setViewConfig(viewPath, {
        sort: this.normalizeSortConfig(viewPath, frame.sort),
      });
    }
    this.queryStore?.acknowledge(
      frame.subscriptionId,
      frame.query,
      frame.mode,
      frame.sort ? this.normalizeSortConfig(viewPath, frame.sort) : undefined
    );
  }

  private handleSnapshotFrame<T>(frame: SnapshotFrame<T>): void {
    this.handleSnapshotFrameWithoutEnforce(frame);
    this.enforceMaxEntries(frame.entity);
  }

  private handleSnapshotFrameWithoutEnforce<T>(frame: SnapshotFrame<T>): void {
    const viewPath = frame.entity;
    const acceptedKeys: string[] = [];

    for (const entity of frame.data) {
      const normalized = this.normalizeEntity<T>(viewPath, entity.data, {
        key: entity.key,
        operation: 'snapshot',
        seq: this.extractSeq(entity.data),
      });
      if (normalized === null) {
        continue;
      }

      const nextValue = this.attachInternalSeq(viewPath, normalized, this.extractSeq(entity.data));
      acceptedKeys.push(entity.key);
      const previousValue = this.storage.get<T>(viewPath, entity.key);
      this.storage.set(viewPath, entity.key, nextValue);

      this.storage.notifyUpdate(viewPath, entity.key, {
        type: 'upsert',
        key: entity.key,
        data: nextValue,
      });

      this.emitRichUpdate(viewPath, entity.key, previousValue, nextValue, 'upsert');
    }
    this.queryStore?.stageSnapshot(frame, acceptedKeys);
  }

  private handleEntityFrame<T>(frame: EntityFrame<T>): void {
    this.handleEntityFrameWithoutEnforce(frame);
    this.enforceMaxEntries(frame.entity);
  }

  private handleEntityFrameWithoutEnforce<T>(frame: EntityFrame<T>): void {
    const viewPath = frame.entity;
    const previousValue = this.storage.get<T>(viewPath, frame.key);
    const previousSequence = this.getInternalSeq(previousValue);
    const duplicateOrStaleSequence = frame.seq !== undefined
      && previousSequence !== undefined
      && this.compareSeq(frame.seq, previousSequence) <= 0;

    switch (frame.op) {
      case 'upsert':
        {
          if (frame.data === null) break;
          if (duplicateOrStaleSequence && previousValue !== null) {
            const update: Update<T> = {
              type: 'upsert',
              key: frame.key,
              data: previousValue,
            };
            this.queryStore?.applyLive(
              frame.subscriptionId,
              frame.key,
              update,
              this.createRichUpdate(frame.key, null, previousValue),
              frame.seq
            );
            break;
          }
          const normalized = this.normalizeEntity<T>(viewPath, frame.data, {
            key: frame.key,
            operation: 'upsert',
            seq: frame.seq ?? this.extractSeq(frame.data),
          });
          if (normalized === null) {
            break;
          }

          const nextValue = this.attachInternalSeq(
            viewPath,
            normalized,
            frame.seq ?? this.extractSeq(frame.data)
          );
          this.storage.set(viewPath, frame.key, nextValue);
          const update: Update<T> = {
            type: 'upsert',
            key: frame.key,
            data: nextValue,
          };
          this.storage.notifyUpdate(viewPath, frame.key, update);
          const richUpdate = this.createRichUpdate(
            frame.key,
            previousValue,
            nextValue
          );
          this.storage.notifyRichUpdate(viewPath, frame.key, richUpdate);
          this.queryStore?.applyLive(
            frame.subscriptionId,
            frame.key,
            update,
            richUpdate,
            frame.seq ?? this.extractSeq(frame.data)
          );
          break;
        }

      case 'patch': {
        if (frame.data === null) break;
        const existing = this.storage.get<T>(viewPath, frame.key);
        const normalizedPatch = this.normalizeEntity<Partial<T>>(viewPath, frame.data, {
          key: frame.key,
          operation: 'patch',
          seq: frame.seq ?? this.extractSeq(frame.data),
        }, true);
        if (normalizedPatch === null) {
          break;
        }
        if (duplicateOrStaleSequence && existing !== null) {
          const update: Update<T> = {
            type: 'patch',
            key: frame.key,
            data: normalizedPatch,
          };
          this.queryStore?.applyLive(
            frame.subscriptionId,
            frame.key,
            update,
            this.createRichUpdate(frame.key, existing, existing, normalizedPatch),
            frame.seq
          );
          break;
        }

        const appendPaths = this.normalizeAppendPaths(viewPath, frame.append);
        const merged = existing
          ? deepMergeWithAppend(existing, normalizedPatch, appendPaths)
          : normalizedPatch;
        const nextValue = this.attachInternalSeq(
          viewPath,
          merged as T,
          frame.seq ?? this.extractSeq(frame.data) ?? this.getInternalSeq(existing)
        );
        this.storage.set(viewPath, frame.key, nextValue);
        const update: Update<T> = {
          type: 'patch',
          key: frame.key,
          data: normalizedPatch,
        };
        this.storage.notifyUpdate(viewPath, frame.key, update);
        const richUpdate = this.createRichUpdate(
          frame.key,
          previousValue,
          nextValue,
          normalizedPatch
        );
        this.storage.notifyRichUpdate(viewPath, frame.key, richUpdate);
        this.queryStore?.applyLive(
          frame.subscriptionId,
          frame.key,
          update,
          richUpdate,
          frame.seq ?? this.extractSeq(frame.data)
        );
        break;
      }

      case 'remove':
        this.queryStore?.applyLive(
          frame.subscriptionId,
          frame.key,
          { type: 'remove', key: frame.key }
        );
        break;

      case 'delete':
        this.storage.delete(viewPath, frame.key);
        this.storage.notifyUpdate(viewPath, frame.key, {
          type: 'delete',
          key: frame.key,
        });
        if (previousValue !== null) {
          const richUpdate: RichUpdate<T> = { type: 'deleted', key: frame.key, lastKnown: previousValue };
          this.storage.notifyRichUpdate(viewPath, frame.key, richUpdate);
        }
        this.queryStore?.deleteGlobal(viewPath, frame.key, previousValue ?? undefined);
        break;
    }
  }

  private emitRichUpdate<T>(
    viewPath: string,
    key: string,
    before: T | null,
    after: T,
    _op: 'upsert' | 'patch',
    patch?: unknown
  ): void {
    const richUpdate: RichUpdate<T> = before === null
      ? { type: 'created', key, data: after }
      : { type: 'updated', key, before, after, patch };

    this.storage.notifyRichUpdate(viewPath, key, richUpdate);
  }

  private createRichUpdate<T>(
    key: string,
    before: T | null,
    after: T,
    patch?: unknown
  ): RichUpdate<T> {
    return before === null
      ? { type: 'created', key, data: after }
      : { type: 'updated', key, before, after, patch };
  }

  private enforceMaxEntries(viewPath: string): void {
    if (this.maxEntriesPerView === null) return;
    if (!this.storage.evictOldest) return;

    while (this.storage.size(viewPath) > this.maxEntriesPerView) {
      const evicted = this.storage.evictOldest(viewPath);
      if (evicted !== undefined) this.queryStore?.evict(viewPath, evicted);
    }
  }
}
