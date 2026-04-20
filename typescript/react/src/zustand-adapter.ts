import { create, type StoreApi, type UseBoundStore } from 'zustand';
import type {
  StorageAdapter,
  UpdateCallback,
  RichUpdateCallback,
  StorageAdapterConfig,
  ViewSortConfig,
  Update,
  RichUpdate,
  ConnectionState,
} from '@usearete/sdk';

interface ZustandState {
  entities: Map<string, Map<string, unknown>>;
  viewConfigs: Map<string, ViewSortConfig>;
  connectionState: ConnectionState;
  lastError?: string;
}

interface ZustandActions {
  _set: <T>(viewPath: string, key: string, data: T) => void;
  _delete: (viewPath: string, key: string) => void;
  _clear: (viewPath?: string) => void;
  _setConnectionState: (state: ConnectionState, error?: string) => void;
  _setViewConfig: (viewPath: string, config: ViewSortConfig) => void;
}

export type AreteStore = ZustandState & ZustandActions;

export class ZustandAdapter implements StorageAdapter {
  private updateCallbacks: Set<UpdateCallback> = new Set();
  private richUpdateCallbacks: Set<RichUpdateCallback> = new Set();
  private accessOrder: Map<string, string[]> = new Map();

  public readonly store: UseBoundStore<StoreApi<AreteStore>>;

  constructor(_config: StorageAdapterConfig = {}) {
    this.store = create<AreteStore>((set) => ({
      entities: new Map(),
      viewConfigs: new Map(),
      connectionState: 'disconnected',
      lastError: undefined,

      _set: <T>(viewPath: string, key: string, data: T) => {
        set((state) => {
          const newEntities = new Map(state.entities);
          const viewMap = new Map(newEntities.get(viewPath) ?? new Map());
          viewMap.set(key, data);
          newEntities.set(viewPath, viewMap);
          return { entities: newEntities };
        });
      },

      _delete: (viewPath: string, key: string) => {
        set((state) => {
          const newEntities = new Map(state.entities);
          const viewMap = newEntities.get(viewPath);
          if (viewMap) {
            const newViewMap = new Map(viewMap);
            newViewMap.delete(key);
            newEntities.set(viewPath, newViewMap);
          }
          return { entities: newEntities };
        });
      },

      _clear: (viewPath?: string) => {
        set((state) => {
          if (viewPath) {
            const newEntities = new Map(state.entities);
            newEntities.delete(viewPath);
            return { entities: newEntities };
          }
          return { entities: new Map() };
        });
      },

      _setConnectionState: (connectionState, lastError) => {
        set({ connectionState, lastError });
      },

      _setViewConfig: (viewPath: string, config: ViewSortConfig) => {
        set((state) => {
          const newConfigs = new Map(state.viewConfigs);
          newConfigs.set(viewPath, config);
          return { viewConfigs: newConfigs };
        });
      },
    }));
  }

  get<T>(viewPath: string, key: string): T | null {
    const viewMap = this.store.getState().entities.get(viewPath);
    if (!viewMap) return null;
    const value = viewMap.get(key);
    return value !== undefined ? (value as T) : null;
  }

  getAll<T>(viewPath: string): T[] {
    const viewMap = this.store.getState().entities.get(viewPath);
    if (!viewMap) return [];
    return Array.from(viewMap.values()) as T[];
  }

  getAllSync<T>(viewPath: string): T[] | undefined {
    const viewMap = this.store.getState().entities.get(viewPath);
    if (!viewMap) return undefined;
    return Array.from(viewMap.values()) as T[];
  }

  getSync<T>(viewPath: string, key: string): T | null | undefined {
    const viewMap = this.store.getState().entities.get(viewPath);
    if (!viewMap) return undefined;
    const value = viewMap.get(key);
    return value !== undefined ? (value as T) : null;
  }

  has(viewPath: string, key: string): boolean {
    return this.store.getState().entities.get(viewPath)?.has(key) ?? false;
  }

  keys(viewPath: string): string[] {
    const viewMap = this.store.getState().entities.get(viewPath);
    if (!viewMap) return [];
    return Array.from(viewMap.keys());
  }

  size(viewPath: string): number {
    return this.store.getState().entities.get(viewPath)?.size ?? 0;
  }

  set<T>(viewPath: string, key: string, data: T): void {
    let order = this.accessOrder.get(viewPath);
    if (!order) {
      order = [];
      this.accessOrder.set(viewPath, order);
    }

    const existingIdx = order.indexOf(key);
    if (existingIdx !== -1) {
      order.splice(existingIdx, 1);
    }
    order.push(key);

    this.store.getState()._set(viewPath, key, data);
  }

  delete(viewPath: string, key: string): void {
    const order = this.accessOrder.get(viewPath);
    if (order) {
      const idx = order.indexOf(key);
      if (idx !== -1) {
        order.splice(idx, 1);
      }
    }

    this.store.getState()._delete(viewPath, key);
  }

  clear(viewPath?: string): void {
    if (viewPath) {
      this.accessOrder.delete(viewPath);
    } else {
      this.accessOrder.clear();
    }

    this.store.getState()._clear(viewPath);
  }

  evictOldest(viewPath: string): string | undefined {
    const order = this.accessOrder.get(viewPath);
    if (!order || order.length === 0) return undefined;

    const oldest = order.shift();
    if (oldest !== undefined) {
      this.store.getState()._delete(viewPath, oldest);
    }
    return oldest;
  }

  onUpdate(callback: UpdateCallback): () => void {
    this.updateCallbacks.add(callback);
    return () => this.updateCallbacks.delete(callback);
  }

  onRichUpdate(callback: RichUpdateCallback): () => void {
    this.richUpdateCallbacks.add(callback);
    return () => this.richUpdateCallbacks.delete(callback);
  }

  notifyUpdate<T>(viewPath: string, key: string, update: Update<T>): void {
    for (const callback of this.updateCallbacks) {
      callback(viewPath, key, update);
    }
  }

  notifyRichUpdate<T>(viewPath: string, key: string, update: RichUpdate<T>): void {
    for (const callback of this.richUpdateCallbacks) {
      callback(viewPath, key, update);
    }
  }

  setConnectionState(state: ConnectionState, error?: string): void {
    this.store.getState()._setConnectionState(state, error);
  }

  setViewConfig(viewPath: string, config: ViewSortConfig): void {
    const state = this.store.getState();
    const existingConfig = state.viewConfigs.get(viewPath);
    if (existingConfig) return;

    state._setViewConfig(viewPath, config);
  }

  getViewConfig(viewPath: string): ViewSortConfig | undefined {
    return this.store.getState().viewConfigs.get(viewPath);
  }
}
