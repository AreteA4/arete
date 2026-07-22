import type { EntityFrame, SnapshotFrame, Frame, SortConfig, SubscribedFrame } from './frame';
import { isSnapshotFrame, isSubscribedFrame } from './frame';
import type { Update, RichUpdate, SubscribeCallback, UnsubscribeFn } from './types';
import { DEFAULT_MAX_ENTRIES_PER_VIEW } from './types';

export interface EntityStoreConfig {
  maxEntriesPerView?: number | null;
}

export interface ViewConfig {
  sort?: SortConfig;
}

function getNestedValue(obj: unknown, path: string[]): unknown {
  let current: unknown = obj;
  for (const segment of path) {
    if (current === null || current === undefined) return undefined;
    if (typeof current !== 'object') return undefined;
    current = (current as Record<string, unknown>)[segment];
  }
  return current;
}

function compareSortValues(a: unknown, b: unknown): number {
  if (a === b) return 0;
  if (a === undefined || a === null) return -1;
  if (b === undefined || b === null) return 1;

  if (typeof a === 'number' && typeof b === 'number') {
    return a - b;
  }
  if (typeof a === 'string' && typeof b === 'string') {
    return a.localeCompare(b);
  }
  if (typeof a === 'boolean' && typeof b === 'boolean') {
    return (a ? 1 : 0) - (b ? 1 : 0);
  }

  return String(a).localeCompare(String(b));
}

class ViewData<T = unknown> {
  private entities: Map<string, T> = new Map();
  private accessOrder: string[] = [];
  private sortConfig: SortConfig | undefined;
  private sortedKeys: string[] = [];

  constructor(sortConfig?: SortConfig) {
    this.sortConfig = sortConfig;
  }

  get(key: string): T | undefined {
    return this.entities.get(key);
  }

  set(key: string, value: T): void {
    const isNew = !this.entities.has(key);
    this.entities.set(key, value);

    if (this.sortConfig) {
      this.updateSortedPosition(key, value, isNew);
    } else {
      if (isNew) {
        this.accessOrder.push(key);
      } else {
        this.touch(key);
      }
    }
  }

  private updateSortedPosition(key: string, value: T, isNew: boolean): void {
    if (!isNew) {
      const existingIdx = this.sortedKeys.indexOf(key);
      if (existingIdx !== -1) {
        this.sortedKeys.splice(existingIdx, 1);
      }
    }

    const sortValue = getNestedValue(value, this.sortConfig!.field);
    const isDesc = this.sortConfig!.order === 'desc';

    let insertIdx = this.binarySearchInsertPosition(sortValue, key, isDesc);
    this.sortedKeys.splice(insertIdx, 0, key);
  }

  private binarySearchInsertPosition(sortValue: unknown, key: string, isDesc: boolean): number {
    let low = 0;
    let high = this.sortedKeys.length;

    while (low < high) {
      const mid = Math.floor((low + high) / 2);
      const midKey = this.sortedKeys[mid];
      if (midKey === undefined) break;
      const midEntity = this.entities.get(midKey);
      const midValue = getNestedValue(midEntity, this.sortConfig!.field);

      let cmp = compareSortValues(sortValue, midValue);
      if (isDesc) cmp = -cmp;

      if (cmp === 0) {
        cmp = key.localeCompare(midKey);
      }

      if (cmp < 0) {
        high = mid;
      } else {
        low = mid + 1;
      }
    }

    return low;
  }

  delete(key: string): boolean {
    if (this.sortConfig) {
      const idx = this.sortedKeys.indexOf(key);
      if (idx !== -1) {
        this.sortedKeys.splice(idx, 1);
      }
    } else {
      const idx = this.accessOrder.indexOf(key);
      if (idx !== -1) {
        this.accessOrder.splice(idx, 1);
      }
    }
    return this.entities.delete(key);
  }

  has(key: string): boolean {
    return this.entities.has(key);
  }

  values(): T[] {
    if (this.sortConfig) {
      return this.sortedKeys.map(k => this.entities.get(k)!);
    }
    return Array.from(this.entities.values());
  }

  keys(): string[] {
    if (this.sortConfig) {
      return [...this.sortedKeys];
    }
    return Array.from(this.entities.keys());
  }

  get size(): number {
    return this.entities.size;
  }

  touch(key: string): void {
    if (this.sortConfig) return;
    const idx = this.accessOrder.indexOf(key);
    if (idx !== -1) {
      this.accessOrder.splice(idx, 1);
      this.accessOrder.push(key);
    }
  }

  evictOldest(): string | undefined {
    if (this.sortConfig) {
      const oldest = this.sortedKeys.pop();
      if (oldest !== undefined) {
        this.entities.delete(oldest);
      }
      return oldest;
    }
    const oldest = this.accessOrder.shift();
    if (oldest !== undefined) {
      this.entities.delete(oldest);
    }
    return oldest;
  }

  setSortConfig(config: SortConfig): void {
    if (this.sortConfig) return;
    this.sortConfig = config;
    this.rebuildSortedKeys();
  }

  private rebuildSortedKeys(): void {
    if (!this.sortConfig) return;

    const entries = Array.from(this.entities.entries());
    const isDesc = this.sortConfig.order === 'desc';

    entries.sort((a, b) => {
      const aValue = getNestedValue(a[1], this.sortConfig!.field);
      const bValue = getNestedValue(b[1], this.sortConfig!.field);
      let cmp = compareSortValues(aValue, bValue);
      if (isDesc) cmp = -cmp;
      if (cmp === 0) {
        cmp = a[0].localeCompare(b[0]);
      }
      return cmp;
    });

    this.sortedKeys = entries.map(([k]) => k);
    this.accessOrder = [];
  }

  getSortConfig(): SortConfig | undefined {
    return this.sortConfig;
  }
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

type EntityUpdateCallback = (viewPath: string, key: string, update: Update<unknown>) => void;
type RichUpdateCallback = (viewPath: string, key: string, update: RichUpdate<unknown>) => void;

export class EntityStore {
  private views: Map<string, ViewData<unknown>> = new Map();
  private viewConfigs: Map<string, ViewConfig> = new Map();
  private updateCallbacks: Set<EntityUpdateCallback> = new Set();
  private richUpdateCallbacks: Set<RichUpdateCallback> = new Set();
  private maxEntriesPerView: number | null;

  constructor(config: EntityStoreConfig = {}) {
    this.maxEntriesPerView = config.maxEntriesPerView === undefined
      ? DEFAULT_MAX_ENTRIES_PER_VIEW
      : config.maxEntriesPerView;
  }

  private enforceMaxEntries(viewData: ViewData<unknown>): void {
    if (this.maxEntriesPerView === null) return;
    while (viewData.size > this.maxEntriesPerView) {
      viewData.evictOldest();
    }
  }

  handleFrame<T>(frame: Frame<T>): void {
    if (isSubscribedFrame(frame)) {
      this.handleSubscribedFrame(frame);
      return;
    }
    if (isSnapshotFrame(frame)) {
      this.handleSnapshotFrame(frame);
      return;
    }
    this.handleEntityFrame(frame as EntityFrame<T>);
  }

  private handleSubscribedFrame(frame: SubscribedFrame): void {
    const viewPath = frame.query.view;
    const config: ViewConfig = {};

    if (frame.sort) {
      config.sort = frame.sort;
    }

    this.viewConfigs.set(viewPath, config);

    const existingView = this.views.get(viewPath);
    if (existingView && frame.sort) {
      existingView.setSortConfig(frame.sort);
    }
  }

  private handleSnapshotFrame<T>(frame: SnapshotFrame<T>): void {
    const viewPath = frame.entity;
    let viewData = this.views.get(viewPath);
    const viewConfig = this.viewConfigs.get(viewPath);

    if (!viewData) {
      viewData = new ViewData(viewConfig?.sort);
      this.views.set(viewPath, viewData);
    }

    for (const entity of frame.data) {
      const previousValue = viewData.get(entity.key) as T | undefined;
      viewData.set(entity.key, entity.data);
      this.notifyUpdate(viewPath, entity.key, {
        type: 'upsert',
        key: entity.key,
        data: entity.data,
      });
      this.notifyRichUpdate(viewPath, entity.key, previousValue, entity.data, 'upsert');
    }
    this.enforceMaxEntries(viewData);
  }

  private handleEntityFrame<T>(frame: EntityFrame<T>): void {
    const viewPath = frame.entity;
    let viewData = this.views.get(viewPath);
    const viewConfig = this.viewConfigs.get(viewPath);

    if (!viewData) {
      viewData = new ViewData(viewConfig?.sort);
      this.views.set(viewPath, viewData);
    }

    const previousValue = viewData.get(frame.key) as T | undefined;

    switch (frame.op) {
      case 'upsert':
        if (frame.data === null) break;
        viewData.set(frame.key, frame.data);
        this.enforceMaxEntries(viewData);
        this.notifyUpdate(viewPath, frame.key, {
          type: 'upsert',
          key: frame.key,
          data: frame.data,
        });
        this.notifyRichUpdate(viewPath, frame.key, previousValue, frame.data, frame.op);
        break;

      case 'patch': {
        if (frame.data === null) break;
        const existing = viewData.get(frame.key);
        const appendPaths = frame.append ?? [];
        
        const merged = existing
          ? deepMergeWithAppend(existing, frame.data as Partial<unknown>, appendPaths)
          : frame.data;
        
        viewData.set(frame.key, merged);
        this.enforceMaxEntries(viewData);
        this.notifyUpdate(viewPath, frame.key, {
          type: 'patch',
          key: frame.key,
          data: frame.data as Partial<unknown>,
        });
        this.notifyRichUpdate(viewPath, frame.key, previousValue, merged as T, 'patch', frame.data);
        break;
      }

      case 'remove':
        break;

      case 'delete':
        viewData.delete(frame.key);
        this.notifyUpdate(viewPath, frame.key, {
          type: 'delete',
          key: frame.key,
        });
        if (previousValue !== undefined) {
          this.notifyRichDelete(viewPath, frame.key, previousValue);
        }
        break;
    }
  }

  getAll<T>(viewPath: string): T[] {
    const viewData = this.views.get(viewPath);
    if (!viewData) return [];
    return viewData.values() as T[];
  }

  get<T>(viewPath: string, key: string): T | null {
    const viewData = this.views.get(viewPath);
    if (!viewData) return null;
    const value = viewData.get(key);
    return value !== undefined ? (value as T) : null;
  }

  getAllSync<T>(viewPath: string): T[] | undefined {
    const viewData = this.views.get(viewPath);
    if (!viewData) return undefined;
    return viewData.values() as T[];
  }

  getSync<T>(viewPath: string, key: string): T | null | undefined {
    const viewData = this.views.get(viewPath);
    if (!viewData) return undefined;
    const value = viewData.get(key);
    return value !== undefined ? (value as T) : null;
  }

  keys(viewPath: string): string[] {
    const viewData = this.views.get(viewPath);
    if (!viewData) return [];
    return viewData.keys();
  }

  size(viewPath: string): number {
    const viewData = this.views.get(viewPath);
    return viewData?.size ?? 0;
  }

  clear(): void {
    this.views.clear();
  }

  clearView(viewPath: string): void {
    this.views.delete(viewPath);
    this.viewConfigs.delete(viewPath);
  }

  getViewConfig(viewPath: string): ViewConfig | undefined {
    return this.viewConfigs.get(viewPath);
  }

  setViewConfig(viewPath: string, config: ViewConfig): void {
    this.viewConfigs.set(viewPath, config);
    const existingView = this.views.get(viewPath);
    if (existingView && config.sort) {
      existingView.setSortConfig(config.sort);
    }
  }

  onUpdate(callback: EntityUpdateCallback): UnsubscribeFn {
    this.updateCallbacks.add(callback);
    return () => {
      this.updateCallbacks.delete(callback);
    };
  }

  onRichUpdate(callback: RichUpdateCallback): UnsubscribeFn {
    this.richUpdateCallbacks.add(callback);
    return () => {
      this.richUpdateCallbacks.delete(callback);
    };
  }

  subscribe<T>(viewPath: string, callback: SubscribeCallback<T>): UnsubscribeFn {
    const handler: EntityUpdateCallback = (path, _key, update) => {
      if (path === viewPath) {
        callback(update as Update<T>);
      }
    };
    this.updateCallbacks.add(handler);
    return () => {
      this.updateCallbacks.delete(handler);
    };
  }

  subscribeToKey<T>(
    viewPath: string,
    key: string,
    callback: SubscribeCallback<T>
  ): UnsubscribeFn {
    const handler: EntityUpdateCallback = (path, updateKey, update) => {
      if (path === viewPath && updateKey === key) {
        callback(update as Update<T>);
      }
    };
    this.updateCallbacks.add(handler);
    return () => {
      this.updateCallbacks.delete(handler);
    };
  }

  private notifyUpdate(viewPath: string, key: string, update: Update<unknown>): void {
    for (const callback of this.updateCallbacks) {
      callback(viewPath, key, update);
    }
  }

  private notifyRichUpdate<T>(
    viewPath: string,
    key: string,
    before: T | undefined,
    after: T,
    _op: 'create' | 'upsert' | 'patch',
    patch?: unknown
  ): void {
    const richUpdate: RichUpdate<T> =
      before === undefined
        ? { type: 'created', key, data: after }
        : { type: 'updated', key, before, after, patch };

    for (const callback of this.richUpdateCallbacks) {
      callback(viewPath, key, richUpdate as RichUpdate<unknown>);
    }
  }

  private notifyRichDelete<T>(viewPath: string, key: string, lastKnown: T): void {
    const richUpdate: RichUpdate<T> = { type: 'deleted', key, lastKnown };

    for (const callback of this.richUpdateCallbacks) {
      callback(viewPath, key, richUpdate as RichUpdate<unknown>);
    }
  }
}
