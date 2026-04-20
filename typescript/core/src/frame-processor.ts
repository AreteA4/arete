import type { Frame, SnapshotFrame, EntityFrame, SubscribedFrame } from './frame';
import { isSnapshotFrame, isSubscribedFrame } from './frame';
import type { StorageAdapter } from './storage/adapter';
import type { RichUpdate, Schema } from './types';
import { DEFAULT_MAX_ENTRIES_PER_VIEW } from './types';

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

export class FrameProcessor {
  private storage: StorageAdapter;
  private maxEntriesPerView: number | null;
  private flushIntervalMs: number;
  private schemas?: Record<string, Schema<unknown>>;
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
  }

  private getSchema(viewPath: string): Schema<unknown> | null {
    const schemas = this.schemas;
    if (!schemas) return null;
    const entityName = viewPath.split('/')[0];
    if (typeof entityName !== 'string' || entityName.length === 0) return null;
    const entityKey: string = entityName;
    return schemas[entityKey] ?? null;
  }

  private validateEntity(viewPath: string, data: unknown): boolean {
    const schema = this.getSchema(viewPath);
    if (!schema) return true;
    const result = schema.safeParse(data);
    if (!result.success) {
      console.warn('[Arete] Frame validation failed:', {
        view: viewPath,
        error: result.error,
      });
      return false;
    }
    return true;
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
      this.storage.setViewConfig(frame.view, { sort: frame.sort });
    }
  }

  private handleSnapshotFrame<T>(frame: SnapshotFrame<T>): void {
    this.handleSnapshotFrameWithoutEnforce(frame);
    this.enforceMaxEntries(frame.entity);
  }

  private handleSnapshotFrameWithoutEnforce<T>(frame: SnapshotFrame<T>): void {
    const viewPath = frame.entity;

    for (const entity of frame.data) {
      if (!this.validateEntity(viewPath, entity.data)) {
        continue;
      }
      const previousValue = this.storage.get<T>(viewPath, entity.key);
      this.storage.set(viewPath, entity.key, entity.data);

      this.storage.notifyUpdate(viewPath, entity.key, {
        type: 'upsert',
        key: entity.key,
        data: entity.data,
      });

      this.emitRichUpdate(viewPath, entity.key, previousValue, entity.data, 'upsert');
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
        if (!this.validateEntity(viewPath, frame.data)) {
          break;
        }
        this.storage.set(viewPath, frame.key, frame.data);
        this.storage.notifyUpdate(viewPath, frame.key, {
          type: 'upsert',
          key: frame.key,
          data: frame.data,
        });
        this.emitRichUpdate(viewPath, frame.key, previousValue, frame.data, frame.op);
        break;

      case 'patch': {
        const existing = this.storage.get<T>(viewPath, frame.key);
        const appendPaths = frame.append ?? [];
        const merged = existing
          ? deepMergeWithAppend(existing, frame.data as Partial<T>, appendPaths)
          : frame.data;
        if (!this.validateEntity(viewPath, merged)) {
          break;
        }
        this.storage.set(viewPath, frame.key, merged);
        this.storage.notifyUpdate(viewPath, frame.key, {
          type: 'patch',
          key: frame.key,
          data: frame.data as Partial<T>,
        });
        this.emitRichUpdate(viewPath, frame.key, previousValue, merged as T, 'patch', frame.data);
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
