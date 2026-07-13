import type { Frame, SnapshotFrame, EntityFrame, SubscribedFrame } from './frame';
import { isSnapshotFrame, isSubscribedFrame } from './frame';
import type { StorageAdapter } from './storage/adapter';
import type { RichUpdate, Schema } from './types';
import { DEFAULT_MAX_ENTRIES_PER_VIEW } from './types';

const INTERNAL_SEQ_FIELD = '__seq';

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
  private pendingUpdates: PendingUpdate[] = [];
  private flushTimer: ReturnType<typeof setTimeout> | null = null;
  private isProcessing = false;

  constructor(storage: StorageAdapter, config: FrameProcessorConfig = {}) {
    this.storage = storage;
    this.maxEntriesPerView = config.maxEntriesPerView === undefined
      ? DEFAULT_MAX_ENTRIES_PER_VIEW
      : config.maxEntriesPerView;
    this.flushIntervalMs = config.flushIntervalMs ?? 0;
    this.schemas = config.schemas;
    this.patchSchemas = config.patchSchemas;
  }

  private getSchema(viewPath: string, patch = false): Schema<unknown> | null {
    const schemas = patch ? this.patchSchemas : this.schemas;
    if (!schemas) return null;
    const entityName = viewPath.split('/')[0];
    if (typeof entityName !== 'string' || entityName.length === 0) return null;
    const entityKey: string = entityName;
    return schemas[entityKey] ?? null;
  }

  private normalizeEntity<T>(viewPath: string, data: unknown, patch = false): T | null {
    const schema = this.getSchema(viewPath, patch)
      ?? (!patch ? null : this.getSchema(viewPath));
    if (!schema) return data as T;

    const result = schema.safeParse(data);
    if (!result.success) {
      console.warn('[Arete] Frame validation failed:', {
        view: viewPath,
        error: result.error,
      });
      return null;
    }

    return stripUndefinedProperties(result.data as T);
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

  private attachInternalSeq<T>(viewPath: string, data: T, seq?: string): T {
    if (!seq || !this.hasSchema(viewPath) || !isObject(data)) {
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

    this.isProcessing = false;
  }

  private processFrame<T>(frame: Frame<T>): void {
    if (isSubscribedFrame(frame)) {
      this.handleSubscribedFrame(frame);
    } else if (isSnapshotFrame(frame)) {
      this.handleSnapshotFrame(frame);
    } else {
      this.handleEntityFrame(frame);
    }
  }

  private processFrameWithoutEnforce<T>(frame: Frame<T>): string | null {
    if (isSubscribedFrame(frame)) {
      this.handleSubscribedFrame(frame);
      return null;
    } else if (isSnapshotFrame(frame)) {
      this.handleSnapshotFrameWithoutEnforce(frame);
      return frame.entity;
    } else {
      this.handleEntityFrameWithoutEnforce(frame);
      return frame.entity;
    }
  }

  private handleSubscribedFrame(frame: SubscribedFrame): void {
    if (this.storage.setViewConfig && frame.sort) {
      this.storage.setViewConfig(frame.view, {
        sort: this.normalizeSortConfig(frame.view, frame.sort),
      });
    }
  }

  private handleSnapshotFrame<T>(frame: SnapshotFrame<T>): void {
    this.handleSnapshotFrameWithoutEnforce(frame);
    this.enforceMaxEntries(frame.entity);
  }

  private handleSnapshotFrameWithoutEnforce<T>(frame: SnapshotFrame<T>): void {
    const viewPath = frame.entity;

    for (const entity of frame.data) {
      const normalized = this.normalizeEntity<T>(viewPath, entity.data);
      if (normalized === null) {
        continue;
      }

      const nextValue = this.attachInternalSeq(viewPath, normalized, this.extractSeq(entity.data));
      const previousValue = this.storage.get<T>(viewPath, entity.key);
      this.storage.set(viewPath, entity.key, nextValue);

      this.storage.notifyUpdate(viewPath, entity.key, {
        type: 'upsert',
        key: entity.key,
        data: nextValue,
      });

      this.emitRichUpdate(viewPath, entity.key, previousValue, nextValue, 'upsert');
    }
  }

  private handleEntityFrame<T>(frame: EntityFrame<T>): void {
    this.handleEntityFrameWithoutEnforce(frame);
    this.enforceMaxEntries(frame.entity);
  }

  private handleEntityFrameWithoutEnforce<T>(frame: EntityFrame<T>): void {
    const viewPath = frame.entity;
    const previousValue = this.storage.get<T>(viewPath, frame.key);

    switch (frame.op) {
      case 'create':
      case 'upsert':
        {
          const normalized = this.normalizeEntity<T>(viewPath, frame.data);
          if (normalized === null) {
            break;
          }

          const nextValue = this.attachInternalSeq(
            viewPath,
            normalized,
            frame.seq ?? this.extractSeq(frame.data)
          );
          this.storage.set(viewPath, frame.key, nextValue);
          this.storage.notifyUpdate(viewPath, frame.key, {
            type: 'upsert',
            key: frame.key,
            data: nextValue,
          });
          this.emitRichUpdate(viewPath, frame.key, previousValue, nextValue, frame.op);
          break;
        }

      case 'patch': {
        const existing = this.storage.get<T>(viewPath, frame.key);
        const normalizedPatch = this.normalizeEntity<Partial<T>>(viewPath, frame.data, true);
        if (normalizedPatch === null) {
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
        this.storage.notifyUpdate(viewPath, frame.key, {
          type: 'patch',
          key: frame.key,
          data: normalizedPatch,
        });
        this.emitRichUpdate(viewPath, frame.key, previousValue, nextValue, 'patch', normalizedPatch);
        break;
      }

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
        break;
    }
  }

  private emitRichUpdate<T>(
    viewPath: string,
    key: string,
    before: T | null,
    after: T,
    _op: 'create' | 'upsert' | 'patch',
    patch?: unknown
  ): void {
    const richUpdate: RichUpdate<T> = before === null
      ? { type: 'created', key, data: after }
      : { type: 'updated', key, before, after, patch };

    this.storage.notifyRichUpdate(viewPath, key, richUpdate);
  }

  private enforceMaxEntries(viewPath: string): void {
    if (this.maxEntriesPerView === null) return;
    if (!this.storage.evictOldest) return;

    while (this.storage.size(viewPath) > this.maxEntriesPerView) {
      this.storage.evictOldest(viewPath);
    }
  }
}
