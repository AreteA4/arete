import { useCallback, useEffect, useRef, useSyncExternalStore } from 'react';
import type {
  ConnectedArete,
  QueryLease,
  QuerySnapshot,
  StackDefinition,
  SubscriptionQuery,
} from '@usearete/sdk';
import { AreteError, canonicalQueryKey, serializeViewKey } from '@usearete/sdk';
import type {
  ListParams,
  ListParamsBase,
  ListParamsSingle,
  ListOneViewHookOptions,
  ListViewHookOptions,
  Schema,
  StateViewHookOptions,
  ViewDef,
  ViewHookResult,
  ViewSchemaFilterWarning,
  ViewSchemaValidationDiagnostic,
  ViewSchemaValidationErrorCallback,
  ViewStatus,
} from './types';

type AnyClient = ConnectedArete<StackDefinition>;
const noop = () => {};
const developmentEnvironment =
  typeof process === 'undefined' || process.env?.NODE_ENV !== 'production';

interface QueryHookState<T> {
  client: AnyClient | null;
  queryKey: string;
  lease: QueryLease | null;
  fallback: QuerySnapshot<T>;
  hasInitialData: boolean;
  seededSource?: QuerySnapshot<T>;
  seededSnapshot?: QuerySnapshot<T>;
}

function queryError(value: unknown, fallback: string): AreteError {
  if (value instanceof AreteError) return value;
  return new AreteError(
    value instanceof Error ? value.message : fallback,
    'SUBSCRIPTION_ERROR',
    value
  );
}

function useQuery<T>(
  client: AnyClient | null,
  query: SubscriptionQuery,
  snapshotEnabled: boolean,
  enabled: boolean,
  initialData: readonly T[] | undefined
): { snapshot: QuerySnapshot<T>; refresh: () => Promise<void> } {
  const queryKey = canonicalQueryKey({ query, snapshot: { enabled: snapshotEnabled } });
  const stateRef = useRef<QueryHookState<T>>({
    client,
    queryKey,
    lease: null,
    hasInitialData: initialData !== undefined,
    fallback: {
      subscriptionId: `pending:${queryKey}`,
      query,
      keys: [],
      data: initialData ?? [],
      isLoading: enabled && snapshotEnabled && initialData === undefined,
      isRefreshing: false,
    },
  });
  if (stateRef.current.client !== client || stateRef.current.queryKey !== queryKey) {
    stateRef.current = {
      client,
      queryKey,
      lease: null,
      hasInitialData: initialData !== undefined,
      fallback: {
        subscriptionId: `pending:${queryKey}`,
        query,
        keys: [],
        data: initialData ?? [],
        isLoading: enabled && snapshotEnabled && initialData === undefined,
        isRefreshing: false,
      },
    };
  }
  const state = stateRef.current;

  const subscribe = useCallback((callback: () => void) => {
    if (!enabled || !client) return noop;
    try {
      const lease = client.getSubscriptionRegistry().subscribe(query, snapshotEnabled);
      state.lease = lease;
      const unsubscribe = lease.onChange(callback);
      return () => {
        unsubscribe();
        lease.release();
        if (state.lease === lease) state.lease = null;
      };
    } catch (value) {
      state.fallback = {
        ...state.fallback,
        isLoading: false,
        isRefreshing: false,
        error: queryError(value, 'Subscription failed'),
      };
      return noop;
    }
  // queryKey represents every query field; state is replaced with that key.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, enabled, queryKey, snapshotEnabled]);

  const getSnapshot = useCallback(() => {
    if (!state.lease) return state.fallback;
    try {
      const current = state.lease.getSnapshot<T>();
      if (state.hasInitialData && current.isLoading && current.data.length === 0) {
        if (state.seededSource !== current) {
          state.seededSource = current;
          state.seededSnapshot = {
            ...current,
            data: state.fallback.data,
            isLoading: false,
          };
        }
        return state.seededSnapshot!;
      }
      return current;
    } catch (value) {
      state.fallback = {
        ...state.fallback,
        isLoading: false,
        isRefreshing: false,
        error: queryError(value, 'Subscription snapshot failed'),
      };
      return state.fallback;
    }
  // state is replaced whenever either listed identity changes.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [queryKey, client]);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const getServerSnapshot = useCallback(() => state.fallback, [queryKey, client]);
  const snapshot = useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
  const refresh = useCallback(async () => {
    if (!state.lease) return;
    await state.lease.refresh();
  // state is replaced whenever either listed identity changes.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [queryKey, client]);
  return { snapshot, refresh };
}

function computeStatus(
  enabled: boolean,
  client: AnyClient | null,
  snapshot: QuerySnapshot<unknown>
): ViewStatus {
  if (!enabled) return 'disabled';
  if (!client) return 'connecting';
  if (snapshot.error) return 'error';
  return snapshot.isLoading ? 'subscribing' : 'ready';
}

function useProjectedData<T>(
  snapshot: QuerySnapshot<unknown>,
  schema: Schema<T> | undefined,
  onSchemaValidationError: ViewSchemaValidationErrorCallback | undefined
): readonly T[] {
  const callbackRef = useRef(onSchemaValidationError);
  callbackRef.current = onSchemaValidationError;
  const cache = useRef<{
    snapshot: QuerySnapshot<unknown> | null;
    schema: Schema<T> | undefined;
    value: readonly T[];
    diagnostics: readonly ViewSchemaValidationDiagnostic[];
  }>({ snapshot: null, schema, value: [], diagnostics: [] });
  if (cache.current.snapshot !== snapshot || cache.current.schema !== schema) {
    const value: T[] = [];
    const diagnostics: ViewSchemaValidationDiagnostic[] = [];
    snapshot.data.forEach((entity, index) => {
      if (!schema) {
        value.push(entity as T);
        return;
      }
      try {
        const parsed = schema.safeParse(entity);
        if (parsed.success) {
          value.push(parsed.data);
        } else {
          diagnostics.push({
            view: snapshot.query.view,
            ...(snapshot.keys[index] === undefined ? {} : { key: snapshot.keys[index] }),
            entity,
            error: parsed.error,
          });
        }
      } catch (error) {
        diagnostics.push({
          view: snapshot.query.view,
          ...(snapshot.keys[index] === undefined ? {} : { key: snapshot.keys[index] }),
          entity,
          error,
        });
      }
    });
    cache.current = { snapshot, schema, value, diagnostics };
  }
  const projection = cache.current;
  useEffect(() => {
    if (projection.diagnostics.length === 0) return;
    const callback = callbackRef.current;
    if (callback) {
      for (const diagnostic of projection.diagnostics) {
        try {
          callback(diagnostic);
        } catch (error) {
          console.error('[Arete] View schema validation callback failed:', error);
        }
      }
      return;
    }
    if (!developmentEnvironment) return;
    const view = snapshot.query.view;
    const singleKey = snapshot.keys.length === 1 ? snapshot.keys[0] : undefined;
    const warning: ViewSchemaFilterWarning = {
      view,
      ...(singleKey === undefined ? {} : { key: singleKey }),
      rejectedCount: projection.diagnostics.length,
      diagnostics: projection.diagnostics,
    };
    console.warn(
      `[Arete] View schema filtered ${projection.diagnostics.length} entit${projection.diagnostics.length === 1 ? 'y' : 'ies'} from ${view}. Pass onSchemaValidationError to inspect or suppress this warning.`,
      warning,
    );
  // The projection object changes whenever snapshot/schema changes and carries
  // the diagnostic context used here.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projection]);
  return projection.value;
}

export function useStateView<T, TKey>(
  viewDef: ViewDef<T, 'state', TKey>,
  client: AnyClient | null,
  key: TKey | null | undefined,
  options?: StateViewHookOptions<T>
): ViewHookResult<T> {
  const enabled = options?.enabled !== false && key !== null && key !== undefined;
  const keyString = key === null || key === undefined
    ? undefined
    : serializeViewKey(viewDef, key);
  const query: SubscriptionQuery = {
    view: viewDef.view,
    ...(keyString === undefined ? {} : { key: keyString }),
    ...(options?.partition === undefined ? {} : { partition: options.partition }),
    ...(options?.filters === undefined ? {} : { filters: options.filters }),
    ...(options?.after === undefined ? {} : { after: options.after }),
    ...(options?.snapshotLimit === undefined ? {} : { snapshotLimit: options.snapshotLimit }),
  };
  const snapshotEnabled = options?.withSnapshot ?? true;
  const { snapshot, refresh } = useQuery<T>(
    client,
    query,
    snapshotEnabled,
    enabled,
    options?.initialData === undefined ? undefined : [options.initialData as T]
  );
  const data = useProjectedData<T>(
    snapshot,
    options?.schema,
    options?.onSchemaValidationError
  )[0];
  const status = computeStatus(enabled, client, snapshot);
  return {
    data: enabled ? data : undefined,
    status,
    isPending: status === 'connecting' || status === 'subscribing',
    isReady: status === 'ready',
    isEmpty: status === 'ready' && data === undefined,
    isLoading: enabled && snapshot.isLoading,
    isRefreshing: enabled && snapshot.isRefreshing,
    error: status === 'error' ? snapshot.error : undefined,
    refresh,
  } as ViewHookResult<T>;
}

export function useListView<T>(
  viewDef: ViewDef<T, 'list'>,
  client: AnyClient | null,
  params?: ListParams,
  options?: ListViewHookOptions<T>
): ViewHookResult<T[]> {
  const enabled = options?.enabled !== false;
  const partition = params?.partition ?? options?.partition;
  const filters = params?.filters ?? options?.filters;
  const after = params?.after ?? options?.after;
  const snapshotLimit = params?.snapshotLimit ?? options?.snapshotLimit;
  const query: SubscriptionQuery = {
    view: viewDef.view,
    ...(params?.key === undefined ? {} : { key: params.key }),
    ...(partition === undefined ? {} : { partition }),
    ...(filters === undefined ? {} : { filters }),
    ...(params?.take === undefined ? {} : { take: params.take }),
    ...(params?.skip === undefined ? {} : { skip: params.skip }),
    ...(after === undefined ? {} : { after }),
    ...(snapshotLimit === undefined ? {} : { snapshotLimit }),
  };
  const snapshotEnabled = params?.withSnapshot ?? options?.withSnapshot ?? true;
  const { snapshot, refresh } = useQuery<T>(
    client,
    query,
    snapshotEnabled,
    enabled,
    options?.initialData
  );
  const data = useProjectedData<T>(
    snapshot,
    (params?.schema ?? options?.schema) as Schema<T> | undefined,
    params?.onSchemaValidationError ?? options?.onSchemaValidationError
  );
  const status = computeStatus(enabled, client, snapshot);
  return {
    data: enabled ? data as T[] : options?.initialData as T[] | undefined,
    status,
    isPending: status === 'connecting' || status === 'subscribing',
    isReady: status === 'ready',
    isEmpty: status === 'ready' && data.length === 0,
    isLoading: enabled && snapshot.isLoading,
    isRefreshing: enabled && snapshot.isRefreshing,
    error: status === 'error' ? snapshot.error : undefined,
    refresh,
  } as ViewHookResult<T[]>;
}

export function createStateViewHook<T, TKey>(
  viewDef: ViewDef<T, 'state', TKey>,
  client: AnyClient | null
) {
  function use<TSchema = T>(
    key: TKey | null | undefined,
    options?: StateViewHookOptions<TSchema>
  ): ViewHookResult<TSchema> {
    return useStateView(
      viewDef as unknown as ViewDef<TSchema, 'state', TKey>,
      client,
      key,
      options
    );
  }
  function refresh(key?: TKey | null): Promise<void> {
    if (!client) return Promise.resolve();
    const keyString = key === null || key === undefined
      ? undefined
      : serializeViewKey(viewDef, key);
    return client.getSubscriptionRegistry().refreshView(viewDef.view, keyString);
  }
  return { use, refresh };
}

export function createListViewHook<T>(
  viewDef: ViewDef<T, 'list'>,
  client: AnyClient | null
) {
  function use<TSchema = T>(
    params: ListParamsSingle<TSchema>,
    options?: ListOneViewHookOptions<TSchema>,
  ): ViewHookResult<TSchema>;
  function use<TSchema = T>(
    params?: ListParams,
    options?: ListViewHookOptions<TSchema>,
  ): ViewHookResult<TSchema[]>;
  function use<TSchema = T>(
    params?: ListParams,
    options?: ListViewHookOptions<TSchema> | ListOneViewHookOptions<TSchema>
  ): ViewHookResult<TSchema[]> | ViewHookResult<TSchema> {
    const listOptions: ListViewHookOptions<TSchema> | undefined = params?.take === 1
      ? options
        ? {
            ...options,
            initialData: options.initialData === undefined
              ? undefined
              : [options.initialData as TSchema],
          }
        : undefined
      : options as ListViewHookOptions<TSchema> | undefined;
    const result = useListView(
      viewDef as unknown as ViewDef<TSchema, 'list'>,
      client,
      params,
      listOptions,
    );
    if (params?.take !== 1) return result;
    return { ...result, data: result.data?.[0] } as ViewHookResult<TSchema>;
  }

  function useOne<TSchema = T>(
    params?: Omit<ListParamsBase<TSchema>, 'take'>,
    options?: ListOneViewHookOptions<TSchema>
  ): ViewHookResult<TSchema> {
    const listOptions: ListViewHookOptions<TSchema> | undefined = options
      ? {
          ...options,
          initialData: options.initialData === undefined
            ? undefined
            : [options.initialData],
        }
      : undefined;
    const result = useListView(
      viewDef as unknown as ViewDef<TSchema, 'list'>,
      client,
      { ...(params ?? {}), take: 1 },
      listOptions
    );
    return { ...result, data: result.data?.[0] } as ViewHookResult<TSchema>;
  }
  function refresh(): Promise<void> {
    if (!client) return Promise.resolve();
    return client.getSubscriptionRegistry().refreshView(viewDef.view);
  }
  return { use, useOne, refresh };
}
