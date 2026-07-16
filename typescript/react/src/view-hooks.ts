import { useEffect, useState, useCallback, useSyncExternalStore, useRef } from 'react';
import { ViewDef, ViewHookOptions, ViewHookResult, ListParams, ListParamsBase, Schema } from './types';
import type {
  ConnectedArete,
  Frame,
  SnapshotFrame,
  StackDefinition,
  Subscription,
} from '@usearete/sdk';

type AnyClient = ConnectedArete<StackDefinition>;

function isSnapshotFrame(frame: Frame): frame is SnapshotFrame {
  return frame.op === 'snapshot' && Array.isArray(frame.data);
}

function canonicalRecordJson(record: Record<string, string> | undefined): string | undefined {
  if (!record) return undefined;
  return JSON.stringify(
    Object.fromEntries(Object.entries(record).sort(([left], [right]) => left.localeCompare(right)))
  );
}

function isEmptySnapshotForSubscription(
  frame: Frame,
  subscription: Subscription
): boolean {
  if (
    !isSnapshotFrame(frame)
    || frame.entity !== subscription.view
    || frame.complete === false
  ) {
    return false;
  }

  return frame.data.length === 0 && frame.key === subscription.key;
}

function onEmptySnapshot(
  client: AnyClient,
  subscription: Subscription,
  callback: () => void
): () => void {
  return client.onFrame((frame) => {
    if (isEmptySnapshotForSubscription(frame, subscription)) {
      callback();
    }
  });
}

function shallowArrayEqual<T>(a: T[] | undefined, b: T[] | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

function compareNumericValues(value: unknown, condition: unknown, op: 'gte' | 'lte' | 'gt' | 'lt'): boolean {
  if (typeof value === 'bigint' || typeof condition === 'bigint') {
    const left = typeof value === 'bigint' ? value : BigInt(value as number);
    const right = typeof condition === 'bigint' ? condition : BigInt(condition as number);
    switch (op) {
      case 'gte':
        return left >= right;
      case 'lte':
        return left <= right;
      case 'gt':
        return left > right;
      case 'lt':
        return left < right;
    }
  }

  const left = value as number;
  const right = condition as number;
  switch (op) {
    case 'gte':
      return left >= right;
    case 'lte':
      return left <= right;
    case 'gt':
      return left > right;
    case 'lt':
      return left < right;
  }
}

export function useStateView<T>(
  viewDef: ViewDef<T, 'state'>,
  client: AnyClient | null,
  key?: Record<string, string>,
  options?: ViewHookOptions
): ViewHookResult<T> {
  const [isLoading, setIsLoading] = useState(!options?.initialData && options?.withSnapshot !== false);
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

    const activeClient = clientRef.current;
    const subscription: Subscription = {
      view: viewDef.view,
      key: keyString,
      withSnapshot,
      after,
      snapshotLimit
    };
    let unsubscribeFrame = () => {};
    try {
      const registry = activeClient.getSubscriptionRegistry();
      if (withSnapshot !== false) {
        unsubscribeFrame = onEmptySnapshot(activeClient, subscription, () => setIsLoading(false));
      }
      const unsubscribe = registry.subscribe(subscription);
      setError(undefined);
      if (withSnapshot !== false) {
        setIsLoading(true);
      }

      return () => {
        try {
          unsubscribeFrame();
        } catch (err) {
          console.error('[Arete] Error removing view frame listener:', err);
        }
        try {
          unsubscribe();
        } catch (err) {
          console.error('[Arete] Error unsubscribing from view:', err);
        }
      };
    } catch (err) {
      unsubscribeFrame();
      setError(err instanceof Error ? err : new Error('Subscription failed'));
      setIsLoading(false);
      return undefined;
    }
  }, [viewDef.view, keyString, enabled, withSnapshot, after, snapshotLimit, client]);

  const refresh = useCallback(() => {
    if (!enabled || !clientRef.current) return;

    try {
      const registry = clientRef.current.getSubscriptionRegistry();
      registry.refresh({
        view: viewDef.view, 
        key: keyString,
        withSnapshot,
        after,
        snapshotLimit
      });
      setError(undefined);
      const shouldLoad = withSnapshot ?? true;
      if (shouldLoad) {
        setIsLoading(true);
      }
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
      ? clientRef.current.store.getSync(viewDef.view, keyString)
      : clientRef.current.store.getAllSync(viewDef.view)?.[0];
    const availableEntity = entity ?? undefined;
    
    const validated = availableEntity && schema
      ? (() => {
          const parsed = schema.safeParse(availableEntity);
          return parsed.success ? parsed.data : undefined;
        })()
      : availableEntity;
    
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
  client: AnyClient | null,
  params?: ListParams,
  options?: ViewHookOptions
): ViewHookResult<T[]> {
  const [isLoading, setIsLoading] = useState(!options?.initialData && params?.withSnapshot !== false);
  const [error, setError] = useState<Error | undefined>();
  const clientRef = useRef(client);
  clientRef.current = client;
  const cachedSnapshotRef = useRef<T[] | undefined>(undefined);

  const enabled = options?.enabled !== false;
  const key = params?.key;
  const take = params?.take;
  const skip = params?.skip;
  const whereJson = params?.where ? JSON.stringify(params.where) : undefined;
  const filtersJson = canonicalRecordJson(params?.filters);
  const limit = params?.limit;
  const schema = params?.schema as Schema<T> | undefined;
  const withSnapshot = params?.withSnapshot;
  const after = params?.after;
  const snapshotLimit = params?.snapshotLimit;

  useEffect(() => {
    if (!enabled || !clientRef.current) return undefined;

    const activeClient = clientRef.current;
    const subscription: Subscription = {
      view: viewDef.view,
      key,
      filters: params?.filters,
      take,
      skip,
      withSnapshot,
      after,
      snapshotLimit
    };
    let unsubscribeFrame = () => {};
    try {
      const registry = activeClient.getSubscriptionRegistry();
      if (withSnapshot !== false) {
        unsubscribeFrame = onEmptySnapshot(activeClient, subscription, () => setIsLoading(false));
      }
      const unsubscribe = registry.subscribe(subscription);
      setError(undefined);
      if (withSnapshot !== false) {
        setIsLoading(true);
      }

      return () => {
        try {
          unsubscribeFrame();
        } catch (err) {
          console.error('[Arete] Error removing list view frame listener:', err);
        }
        try {
          unsubscribe();
        } catch (err) {
          console.error('[Arete] Error unsubscribing from list view:', err);
        }
      };
    } catch (err) {
      unsubscribeFrame();
      setError(err instanceof Error ? err : new Error('Subscription failed'));
      setIsLoading(false);
      return undefined;
    }
  }, [viewDef.view, enabled, key, filtersJson, take, skip, withSnapshot, after, snapshotLimit, client]);

  const refresh = useCallback(() => {
    if (!enabled || !clientRef.current) return;

    try {
      const registry = clientRef.current.getSubscriptionRegistry();
      registry.refresh({
        view: viewDef.view, 
        key, 
        filters: params?.filters,
        take,
        skip,
        withSnapshot,
        after,
        snapshotLimit
      });
      setError(undefined);
      const shouldLoad = withSnapshot ?? true;
      if (shouldLoad) {
        setIsLoading(true);
      }
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
      items = items.filter((item: unknown) => {
        return Object.entries(params.where!).every(([fieldKey, condition]) => {
          const value = (item as Record<string, unknown>)[fieldKey];

          if (typeof condition === 'object' && condition !== null) {
            const cond = condition as Record<string, unknown>;
            if ('gte' in cond) return compareNumericValues(value, cond.gte, 'gte');
            if ('lte' in cond) return compareNumericValues(value, cond.lte, 'lte');
            if ('gt' in cond) return compareNumericValues(value, cond.gt, 'gt');
            if ('lt' in cond) return compareNumericValues(value, cond.lt, 'lt');
          }

          return value === condition;
        });
      });
    }

    if (schema) {
      items = items.flatMap((item: unknown) => {
        const parsed = schema.safeParse(item);
        return parsed.success ? [parsed.data as T] : [];
      });
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
  client: AnyClient | null
) {
  return {
    use: (key?: Record<string, string>, options?: ViewHookOptions): ViewHookResult<T> => {
      return useStateView(viewDef, client, key, options);
    }
  };
}

export function createListViewHook<T>(
  viewDef: ViewDef<T, 'list'>,
  client: AnyClient | null
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
