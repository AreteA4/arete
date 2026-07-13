import type { SortConfig } from '../frame';
import type { StorageAdapter, UpdateCallback, RichUpdateCallback, ViewSortConfig } from './adapter';
import type { Update, RichUpdate } from '../types';

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

  if (typeof a === 'bigint' && typeof b === 'bigint') {
    return a < b ? -1 : 1;
  }
  if (typeof a === 'number' && typeof b === 'number') {
    return a - b;
  }
  if (typeof a === 'bigint' && typeof b === 'number' && Number.isInteger(b)) {
    return a < BigInt(b) ? -1 : 1;
  }
  if (typeof a === 'number' && typeof b === 'bigint' && Number.isInteger(a)) {
    return BigInt(a) < b ? -1 : 1;
  }
  if (typeof a === 'string' && typeof b === 'string') {
    return a.localeCompare(b);
  }
  if (typeof a === 'boolean' && typeof b === 'boolean') {
    return (a ? 1 : 0) - (b ? 1 : 0);
  }

  return String(a).localeCompare(String(b));
}

export class SortedStorageDecorator implements StorageAdapter {
  private inner: StorageAdapter;
  private sortConfigs: Map<string, SortConfig> = new Map();
  private sortedKeysMap: Map<string, string[]> = new Map();

  constructor(inner: StorageAdapter) {
    this.inner = inner;
  }

  get<T>(viewPath: string, key: string): T | null {
    return this.inner.get(viewPath, key);
  }

  getAll<T>(viewPath: string): T[] {
    const sortedKeys = this.sortedKeysMap.get(viewPath);
    if (sortedKeys && sortedKeys.length > 0) {
      return sortedKeys
        .map(k => this.inner.get<T>(viewPath, k))
        .filter((v): v is T => v !== null);
    }
    return this.inner.getAll(viewPath);
  }

  getAllSync<T>(viewPath: string): T[] | undefined {
    const sortedKeys = this.sortedKeysMap.get(viewPath);
    if (sortedKeys && sortedKeys.length > 0) {
      return sortedKeys
        .map(k => this.inner.getSync<T>(viewPath, k))
        .filter((v): v is T => v !== null && v !== undefined);
    }
    return this.inner.getAllSync(viewPath);
  }

  getSync<T>(viewPath: string, key: string): T | null | undefined {
    return this.inner.getSync(viewPath, key);
  }

  has(viewPath: string, key: string): boolean {
    return this.inner.has(viewPath, key);
  }

  keys(viewPath: string): string[] {
    const sortedKeys = this.sortedKeysMap.get(viewPath);
    if (sortedKeys) return [...sortedKeys];
    return this.inner.keys(viewPath);
  }

  size(viewPath: string): number {
    return this.inner.size(viewPath);
  }

  set<T>(viewPath: string, key: string, data: T): void {
    this.inner.set(viewPath, key, data);

    const sortConfig = this.sortConfigs.get(viewPath);
    if (sortConfig) {
      this.updateSortedPosition(viewPath, key, data, sortConfig);
    }
  }

  delete(viewPath: string, key: string): void {
    const sortedKeys = this.sortedKeysMap.get(viewPath);
    if (sortedKeys) {
      const idx = sortedKeys.indexOf(key);
      if (idx !== -1) {
        sortedKeys.splice(idx, 1);
      }
    }
    this.inner.delete(viewPath, key);
  }

  clear(viewPath?: string): void {
    if (viewPath) {
      this.sortedKeysMap.delete(viewPath);
      this.sortConfigs.delete(viewPath);
    } else {
      this.sortedKeysMap.clear();
      this.sortConfigs.clear();
    }
    this.inner.clear(viewPath);
  }

  evictOldest(viewPath: string): string | undefined {
    const sortedKeys = this.sortedKeysMap.get(viewPath);
    if (sortedKeys && sortedKeys.length > 0) {
      const oldest = sortedKeys.pop()!;
      this.inner.delete(viewPath, oldest);
      return oldest;
    }
    return this.inner.evictOldest?.(viewPath);
  }

  setViewConfig(viewPath: string, config: ViewSortConfig): void {
    if (config.sort && !this.sortConfigs.has(viewPath)) {
      this.sortConfigs.set(viewPath, config.sort);
      this.rebuildSortedKeys(viewPath, config.sort);
    }
    this.inner.setViewConfig?.(viewPath, config);
  }

  getViewConfig(viewPath: string): ViewSortConfig | undefined {
    const sortConfig = this.sortConfigs.get(viewPath);
    if (sortConfig) return { sort: sortConfig };
    return this.inner.getViewConfig?.(viewPath);
  }

  onUpdate(callback: UpdateCallback): () => void {
    return this.inner.onUpdate(callback);
  }

  onRichUpdate(callback: RichUpdateCallback): () => void {
    return this.inner.onRichUpdate(callback);
  }

  notifyUpdate<T>(viewPath: string, key: string, update: Update<T>): void {
    this.inner.notifyUpdate(viewPath, key, update);
  }

  notifyRichUpdate<T>(viewPath: string, key: string, update: RichUpdate<T>): void {
    this.inner.notifyRichUpdate(viewPath, key, update);
  }

  private updateSortedPosition(viewPath: string, key: string, data: unknown, sortConfig: SortConfig): void {
    let sortedKeys = this.sortedKeysMap.get(viewPath);
    if (!sortedKeys) {
      sortedKeys = [];
      this.sortedKeysMap.set(viewPath, sortedKeys);
    }

    const existingIdx = sortedKeys.indexOf(key);
    if (existingIdx !== -1) {
      sortedKeys.splice(existingIdx, 1);
    }

    const insertIdx = this.binarySearchInsertPosition(viewPath, sortedKeys, sortConfig, key, data);
    sortedKeys.splice(insertIdx, 0, key);
  }

  private binarySearchInsertPosition(
    viewPath: string,
    sortedKeys: string[],
    sortConfig: SortConfig,
    newKey: string,
    newValue: unknown
  ): number {
    const newSortValue = getNestedValue(newValue, sortConfig.field);
    const isDesc = sortConfig.order === 'desc';
    let low = 0;
    let high = sortedKeys.length;

    while (low < high) {
      const mid = Math.floor((low + high) / 2);
      const midKey = sortedKeys[mid]!;
      const midEntity = this.inner.get(viewPath, midKey);
      const midValue = getNestedValue(midEntity, sortConfig.field);

      let cmp = compareSortValues(newSortValue, midValue);
      if (isDesc) cmp = -cmp;

      if (cmp === 0) {
        cmp = newKey.localeCompare(midKey);
      }

      if (cmp < 0) {
        high = mid;
      } else {
        low = mid + 1;
      }
    }

    return low;
  }

  private rebuildSortedKeys(viewPath: string, sortConfig: SortConfig): void {
    const allKeys = this.inner.keys(viewPath);
    if (allKeys.length === 0) return;

    const isDesc = sortConfig.order === 'desc';
    const entries = allKeys.map(k => [k, this.inner.get(viewPath, k)] as [string, unknown]);

    entries.sort((a, b) => {
      const aValue = getNestedValue(a[1], sortConfig.field);
      const bValue = getNestedValue(b[1], sortConfig.field);
      let cmp = compareSortValues(aValue, bValue);
      if (isDesc) cmp = -cmp;
      if (cmp === 0) {
        cmp = a[0].localeCompare(b[0]);
      }
      return cmp;
    });

    this.sortedKeysMap.set(viewPath, entries.map(([k]) => k));
  }
}
