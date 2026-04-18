import type { SortConfig } from '../frame';
import type { Update, RichUpdate } from '../types';

export type UpdateCallback<T = unknown> = (
  viewPath: string,
  key: string,
  update: Update<T>
) => void;

export type RichUpdateCallback<T = unknown> = (
  viewPath: string,
  key: string,
  update: RichUpdate<T>
) => void;

export interface StorageAdapterConfig {
  maxEntriesPerView?: number | null;
}

export interface ViewSortConfig {
  sort?: SortConfig;
}

/**
 * Storage adapter interface for Arete entity storage.
 * Implement this to integrate with Zustand, Pinia, Svelte stores, Redux, IndexedDB, etc.
 */
export interface StorageAdapter {
  get<T>(viewPath: string, key: string): T | null;
  getAll<T>(viewPath: string): T[];
  getAllSync<T>(viewPath: string): T[] | undefined;
  getSync<T>(viewPath: string, key: string): T | null | undefined;
  has(viewPath: string, key: string): boolean;
  keys(viewPath: string): string[];
  size(viewPath: string): number;

  set<T>(viewPath: string, key: string, data: T): void;
  delete(viewPath: string, key: string): void;
  clear(viewPath?: string): void;

  evictOldest?(viewPath: string): string | undefined;

  setViewConfig?(viewPath: string, config: ViewSortConfig): void;
  getViewConfig?(viewPath: string): ViewSortConfig | undefined;

  onUpdate(callback: UpdateCallback): () => void;
  onRichUpdate(callback: RichUpdateCallback): () => void;

  notifyUpdate<T>(viewPath: string, key: string, update: Update<T>): void;
  notifyRichUpdate<T>(viewPath: string, key: string, update: RichUpdate<T>): void;
}
