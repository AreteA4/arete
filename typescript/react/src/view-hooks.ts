import { useEffect, useState, useCallback, useSyncExternalStore, useRef } from 'react';
import { ViewDef, ViewHookOptions, ViewHookResult, ListParams, ListParamsBase, Schema } from './types';
import type { HyperStack } from 'hyperstack-typescript';

function shallowArrayEqual<T>(a: T[] | undefined, b: T[] | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

export function useStateView<T>(
  viewDef: ViewDef<T, 'state'>,
  client: HyperStack<any> | null,
  key?: Record<string, string>,
  options?: ViewHookOptions
): ViewHookResult<T> {
  const [isLoading, setIsLoading] = useState(!options?.initialData);
  const [error, setError] = useState<Error | undefined>();
  const clientRef = useRef(client);
  clientRef.current = client;
  const cachedSnapshotRef = useRef<T | undefined>(undefined);

  const keyString = key ? Object.values(key)[0] : undefined;
  const enabled = options?.enabled !== false;
  const schema = options?.schema as Schema<T> | undefined;
  const withSnapshot = options?.withSnapshot;
  const after = options?.after;
  const snapshotLimit = options?.snapshotLimit;

  useEffect(() => {
    if (!enabled || !clientRef.current) return undefined;

    try {
      const registry = clientRef.current.getSubscriptionRegistry();
      const unsubscribe = registry.subscribe({ 
        view: viewDef.view, 
        key: keyString,
        withSnapshot,
        after,
        snapshotLimit
      });
      setIsLoading(true);

      return () => {
        try {
          unsubscribe();
        } catch (err) {
          console.error('[Hyperstack] Error unsubscribing from view:', err);
        }
      };
    } catch (err) {
      setError(err instanceof Error ? err : new Error('Subscription failed'));
      setIsLoading(false);
      return undefined;
    }
  }, [viewDef.view, keyString, enabled, withSnapshot, after, snapshotLimit, client]);

  const refresh = useCallback(() => {
    if (!enabled || !clientRef.current) return;

    try {
      const registry = clientRef.current.getSubscriptionRegistry();
      const unsubscribe = registry.subscribe({ 
        view: viewDef.view, 
        key: keyString,
        withSnapshot,
        after,
        snapshotLimit
      });
      setIsLoading(true);

      setTimeout(() => {
        try {
          unsubscribe();
        } catch (err) {
          console.error('[Hyperstack] Error during refresh unsubscribe:', err);
        }
      }, 0);
    } catch (err) {
      setError(err instanceof Error ? err : new Error('Refresh failed'));
      setIsLoading(false);
    }
  }, [viewDef.view, keyString, enabled, withSnapshot, after, snapshotLimit]);

  const subscribe = useCallback((callback: () => void) => {
    if (!clientRef.current) return () => {};
    return clientRef.current.store.onUpdate(callback);
  }, [client]);

  const getSnapshot = useCallback(() => {
    if (!clientRef.current) return cachedSnapshotRef.current;
    const entity = keyString 
      ? clientRef.current.store.get(viewDef.view, keyString)
      : clientRef.current.store.getAll(viewDef.view)[0];
    
    const validated = entity && schema 
      ? (schema.safeParse(entity).success ? entity : undefined)
      : entity;
    
    if (validated !== cachedSnapshotRef.current) {
      cachedSnapshotRef.current = validated as T | undefined;
    }
    return cachedSnapshotRef.current;
  }, [viewDef.view, keyString, schema, client]);

  const data = useSyncExternalStore(subscribe, getSnapshot);

  useEffect(() => {
    if (data !== undefined && isLoading) {
      setIsLoading(false);
    }
  }, [data, isLoading]);

  return {
    data: (options?.initialData ?? data) as T | undefined,
    isLoading: client === null || isLoading,
    error,
    refresh
  };
}

export function useListView<T>(
  viewDef: ViewDef<T, 'list'>,
  client: HyperStack<any> | null,
  params?: ListParams,
  options?: ViewHookOptions
): ViewHookResult<T[]> {
  const [isLoading, setIsLoading] = useState(!options?.initialData);
  const [error, setError] = useState<Error | undefined>();
  const clientRef = useRef(client);
  clientRef.current = client;
  const cachedSnapshotRef = useRef<T[] | undefined>(undefined);

  const enabled = options?.enabled !== false;
  const key = params?.key;
  const take = params?.take;
  const skip = params?.skip;
  const whereJson = params?.where ? JSON.stringify(params.where) : undefined;
  const filtersJson = params?.filters ? JSON.stringify(params.filters) : undefined;
  const limit = params?.limit;
  const schema = params?.schema as Schema<T> | undefined;
  const withSnapshot = params?.withSnapshot;
  const after = params?.after;
  const snapshotLimit = params?.snapshotLimit;

  useEffect(() => {
    if (!enabled || !clientRef.current) return undefined;

    try {
      const registry = clientRef.current.getSubscriptionRegistry();
      const unsubscribe = registry.subscribe({ 
        view: viewDef.view, 
        key, 
        filters: params?.filters,
        take,
        skip,
        withSnapshot,
        after,
        snapshotLimit
      });
      setIsLoading(true);

      return () => {
        try {
          unsubscribe();
        } catch (err) {
          console.error('[Hyperstack] Error unsubscribing from list view:', err);
        }
      };
    } catch (err) {
      setError(err instanceof Error ? err : new Error('Subscription failed'));
      setIsLoading(false);
      return undefined;
    }
  }, [viewDef.view, enabled, key, filtersJson, take, skip, withSnapshot, after, snapshotLimit, client]);

  const refresh = useCallback(() => {
    if (!enabled || !clientRef.current) return;

    try {
      const registry = clientRef.current.getSubscriptionRegistry();
      const unsubscribe = registry.subscribe({ 
        view: viewDef.view, 
        key, 
        filters: params?.filters,
        take,
        skip,
        withSnapshot,
        after,
        snapshotLimit
      });
      setIsLoading(true);

      setTimeout(() => {
        try {
          unsubscribe();
        } catch (err) {
          console.error('[Hyperstack] Error during list refresh unsubscribe:', err);
        }
      }, 0);
    } catch (err) {
      setError(err instanceof Error ? err : new Error('Refresh failed'));
      setIsLoading(false);
    }
  }, [viewDef.view, enabled, key, filtersJson, take, skip, withSnapshot, after, snapshotLimit]);

  const subscribe = useCallback((callback: () => void) => {
    if (!clientRef.current) return () => {};
    return clientRef.current.store.onUpdate(callback);
  }, [client]);

  const getSnapshot = useCallback(() => {
    if (!clientRef.current) return cachedSnapshotRef.current;
    const viewData = clientRef.current.store.getAll(viewDef.view);
    
    if (!viewData || viewData.length === 0) {
      if (cachedSnapshotRef.current !== undefined) {
        cachedSnapshotRef.current = undefined;
      }
      return cachedSnapshotRef.current;
    }

    let items = viewData;
    
    if (params?.where) {
      items = items.filter((item) => {
        return Object.entries(params.where!).every(([fieldKey, condition]) => {
          const value = (item as Record<string, unknown>)[fieldKey];

          if (typeof condition === 'object' && condition !== null) {
            const cond = condition as Record<string, unknown>;
            if ('gte' in cond) return (value as number) >= (cond.gte as number);
            if ('lte' in cond) return (value as number) <= (cond.lte as number);
            if ('gt' in cond) return (value as number) > (cond.gt as number);
            if ('lt' in cond) return (value as number) < (cond.lt as number);
          }

          return value === condition;
        });
      });
    }

    if (schema) {
      items = items.filter((item) => schema.safeParse(item).success);
    }

    if (limit) {
      items = items.slice(0, limit);
    }

    const result = items as T[];
    
    if (!shallowArrayEqual(cachedSnapshotRef.current, result)) {
      cachedSnapshotRef.current = result;
    }
    return cachedSnapshotRef.current;
  }, [viewDef.view, whereJson, limit, schema, client]);

  const data = useSyncExternalStore(subscribe, getSnapshot);

  useEffect(() => {
    if (data !== undefined && isLoading) {
      setIsLoading(false);
    }
  }, [data, isLoading]);

  return {
    data: (options?.initialData ?? data) as T[] | undefined,
    isLoading: client === null || isLoading,
    error,
    refresh
  };
}

export function createStateViewHook<T>(
  viewDef: ViewDef<T, 'state'>,
  client: HyperStack<any> | null
) {
  return {
    use: (key?: Record<string, string>, options?: ViewHookOptions): ViewHookResult<T> => {
      return useStateView(viewDef, client, key, options);
    }
  };
}

export function createListViewHook<T>(
  viewDef: ViewDef<T, 'list'>,
  client: HyperStack<any> | null
) {
  function use(params?: ListParams, options?: ViewHookOptions): ViewHookResult<T[]> | ViewHookResult<T | undefined> {
    const result = useListView(viewDef, client, params, options);
    
    if (params?.take === 1) {
      return {
        data: result.data?.[0],
        isLoading: result.isLoading,
        error: result.error,
        refresh: result.refresh
      } as ViewHookResult<T | undefined>;
    }
    
    return result;
  }

  function useOne<TSchema = T>(params?: Omit<ListParamsBase<TSchema>, 'take'>, options?: ViewHookOptions<TSchema>): ViewHookResult<TSchema | undefined> {
    const paramsWithTake = params ? { ...params, take: 1 as const } : { take: 1 as const };
    const result = useListView(viewDef as unknown as ViewDef<TSchema, 'list'>, client, paramsWithTake as ListParams, options as ViewHookOptions);
    
    return {
      data: result.data?.[0] as TSchema | undefined,
      isLoading: result.isLoading,
      error: result.error,
      refresh: result.refresh
    };
  }

  return { use, useOne };
}
