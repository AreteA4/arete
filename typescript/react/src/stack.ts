import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { StoreApi, UseBoundStore } from 'zustand';
import { useAreteContext } from './provider';
import { buildDisconnectedProgramHooks, buildProgramHookInterfaces, type BuildProgramInterface } from './program-hooks';
import { buildReadInterfaces, type ReactReadInterface } from './read-hooks';
import { createStateViewHook, createListViewHook } from './view-hooks';
import { useInstructionMutation } from './hooks';
import { createClientCacheKey } from './client-key';
import type {
  ConnectionState,
  ClientLookupOptions,
  StackDefinition,
  ViewDef,
  ViewMode,
  StateViewHookOptions,
  ListViewHookOptions,
  ListOneViewHookOptions,
  ViewHookResult,
  ListParamsSingle,
  ListParamsMultiple,
  ListParamsBase,
  ViewGroup,
  UseAreteOptions
} from './types';
import { ZustandAdapter, type AreteStore } from './zustand-adapter';
import { getStackRuntimeExtensions } from '@usearete/sdk';
import type {
  ChainClient,
  ConnectedArete,
  ProgramSdkDefinition,
  ProgramsInterface,
  QueriesInterface,
  ReadArgumentCounts,
  SocketIssue,
  StackQueryDefinition,
  StackDefinition as BaseStackDefinition,
  StackConnectedExtensions,
  StackWithAttachedPrograms,
} from '@usearete/sdk';

type AnyClient = ConnectedArete<BaseStackDefinition>;
type ProgramMap = Record<string, ProgramSdkDefinition>;
type ResolvedStack<
  TStack extends StackDefinition,
  TPrograms extends ProgramMap | undefined,
> = StackWithAttachedPrograms<TStack, TPrograms>;
type StackQueries<TStack> = TStack extends {
  queries?: infer TQueries extends Record<string, StackQueryDefinition<unknown, unknown>> | undefined;
}
  ? TQueries
  : undefined;
type StackPrograms<TStack> = TStack extends {
  programs?: infer TPrograms extends Record<string, ProgramSdkDefinition> | undefined;
}
  ? TPrograms
  : undefined;
type ConnectedStack<
  TStack extends StackDefinition,
  TPrograms extends ProgramMap | undefined,
> = ConnectedArete<ResolvedStack<TStack, TPrograms>>;
type ConnectedField<TClient, TKey extends PropertyKey> =
  TKey extends keyof TClient ? TClient[TKey] : undefined;

type ViewHookForDef<TDef> = TDef extends ViewDef<infer T, 'state', infer TKey>
  ? {
      use: <TSchema = T>(
        key: TKey | null | undefined,
        options?: StateViewHookOptions<TSchema>
      ) => ViewHookResult<TSchema>;
      refresh: (key?: TKey | null) => Promise<void>;
    }
  : TDef extends ViewDef<infer T, 'list'>
  ? {
      use: {
        <TSchema = T>(params: ListParamsSingle<TSchema>, options?: ListOneViewHookOptions<TSchema>): ViewHookResult<TSchema>;
        <TSchema = T>(params?: ListParamsMultiple<TSchema>, options?: ListViewHookOptions<TSchema>): ViewHookResult<TSchema[]>;
      };
      useOne: <TSchema = T>(
        params?: Omit<ListParamsBase<TSchema>, 'take'>,
        options?: ListOneViewHookOptions<TSchema>
      ) => ViewHookResult<TSchema>;
      refresh: () => Promise<void>;
    }
  : TDef extends ViewDef<infer T, 'state' | 'list', infer TKey>
  ? {
      use: {
        <TSchema = T>(params: ListParamsSingle<TSchema>, options?: ListOneViewHookOptions<TSchema>): ViewHookResult<TSchema>;
        <TSchema = T>(params?: ListParamsMultiple<TSchema> | TKey, options?: ListViewHookOptions<TSchema> | StateViewHookOptions<TSchema>): ViewHookResult<TSchema | TSchema[]>;
      };
      useOne: <TSchema = T>(
        params?: Omit<ListParamsBase<TSchema>, 'take'>,
        options?: ListOneViewHookOptions<TSchema>
      ) => ViewHookResult<TSchema>;
      refresh: (key?: TKey | null) => Promise<void>;
    }
  : never;

type BuildViewInterface<TViews extends Record<string, ViewGroup>> = {
  [K in keyof TViews]: {
    [SubK in keyof TViews[K] as TViews[K][SubK] extends ViewDef<any, ViewMode, any> ? SubK : never]: ViewHookForDef<TViews[K][SubK]>;
  };
};

export type UseAreteResult<
  TStack extends StackDefinition,
  TPrograms extends ProgramMap | undefined = undefined,
> = {
  /**
   * Typed subscription hooks, one per stack view. Keyed hooks only ever
   * expose data for the key you passed — when the key changes, `data` is
   * `undefined` (and `isLoading` true) until the new key's snapshot arrives,
   * so callers never need to re-verify data against their inputs.
   */
  views: BuildViewInterface<TStack['views']>;
  queries: QueriesInterface<StackQueries<ResolvedStack<TStack, TPrograms>>>;
  programs: BuildProgramInterface<StackPrograms<ResolvedStack<TStack, TPrograms>>, ConnectedStack<TStack, TPrograms>>;
  chain: ChainClient | null;
  zustandStore: UseBoundStore<StoreApi<AreteStore>> | null;
  client: ConnectedStack<TStack, TPrograms> | null;
  read: ReactReadInterface<
    NonNullable<ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'read'>>
  >;
  /** @deprecated Hooks and imperative reads now share the `read` namespace. */
  reads: ReactReadInterface<
    NonNullable<ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'read'>>
  >;
  addresses: ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'addresses'>;
  constants: ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'constants'>;
  defaults: ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'defaults'>;
  math: ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'math'>;
  /**
   * Raw connection state reported by the client. Prefer {@link status},
   * which also covers the initial client-creation window.
   */
  connectionState: ConnectionState;
  /**
   * The single connection field UIs should read: 'connecting' while the
   * client is being created or connecting, then the client's own state.
   */
  status: ConnectionState;
  isConnected: boolean;
  isLoading: boolean;
  /** True when retry() can start a new shared connection attempt. */
  canRetry: boolean;
  error: Error | null;
  socketIssue: SocketIssue | null;
  retry: () => Promise<void>;
};

function normalizeConnectionError(value: unknown, fallback: string): Error {
  if (value instanceof Error) return value;
  return new Error(typeof value === 'string' && value.length > 0 ? value : fallback);
}

/**
 * Module-augmentation point for the default stack's type. When the provider
 * declares a default stack (`<AreteProvider stack={...}>`), register its type
 * once so argument-less `useArete()` calls are fully typed:
 *
 * ```ts
 * declare module '@usearete/react' {
 *   interface AreteDefaultStackRegistry { defaultStack: OreStreamStack }
 * }
 * ```
 */
export interface AreteDefaultStackRegistry {}

type RegisteredDefaultStack = AreteDefaultStackRegistry extends {
  defaultStack: infer TStack extends StackDefinition;
}
  ? TStack
  : StackDefinition;

export function useArete(
  stack?: undefined,
  options?: UseAreteOptions
): UseAreteResult<RegisteredDefaultStack, undefined>;
export function useArete<
  TStack extends StackDefinition,
  TPrograms extends ProgramMap | undefined = undefined,
>(
  stack: TStack,
  options?: UseAreteOptions<TPrograms>
): UseAreteResult<TStack, TPrograms>;
export function useArete(
  stack?: StackDefinition,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  options?: any
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
): any {
  const {
    getOrCreateClient,
    getClient,
    retryClient,
    subscribeToClientChanges,
    config,
  } = useAreteContext();
  const resolvedStack = stack ?? config.stack;
  if (!resolvedStack) {
    throw new Error(
      'useArete() was called without a stack and no default stack is configured. ' +
      'Pass a stack explicitly or set <AreteProvider stack={...}>.'
    );
  }
  const usesProviderStack = stack === undefined || stack === config.stack;
  const resolvedOptions = (usesProviderStack
    ? { ...(config.stackOptions ?? {}), ...(options ?? {}) }
    : options) as
    UseAreteOptions<ProgramMap> | undefined;
  const url = resolvedOptions?.url;
  const httpUrl = resolvedOptions?.httpUrl;
  const transport = resolvedOptions?.transport;
  const transactions = resolvedOptions?.transactions;
  const attachedPrograms = resolvedOptions?.programs;
  const lookupOptions = useMemo(
    () => ({ url, httpUrl, transport, transactions, programs: attachedPrograms }) as ClientLookupOptions<ProgramMap>,
    [url, httpUrl, transport, transactions, attachedPrograms]
  );
  const lookupKey = createClientCacheKey(resolvedStack, lookupOptions);
  const initialClient = getClient(resolvedStack, lookupOptions) as ConnectedStack<StackDefinition, ProgramMap> | null;
  const [clientState, setClientState] = useState<{
    lookupKey: string | null;
    client: ConnectedStack<StackDefinition, ProgramMap> | null;
  }>(() => ({ lookupKey, client: initialClient }));
  const clientStateRef = useRef(clientState);
  clientStateRef.current = clientState;
  const client = clientState.lookupKey === lookupKey ? clientState.client : null;
  const [isConnecting, setIsConnecting] = useState(!initialClient);
  const [error, setError] = useState<Error | null>(null);
  const [socketIssue, setSocketIssue] = useState<SocketIssue | null>(null);
  const [connectionState, setConnectionState] = useState<ConnectionState>(() =>
    client?.connectionState ?? 'disconnected'
  );
  const retryPromiseRef = useRef<{ lookupKey: string | null; promise: Promise<void> } | null>(null);
  const lookupKeyRef = useRef(lookupKey);
  lookupKeyRef.current = lookupKey;

  useEffect(() => {
    let active = true;
    const existingClient = getClient(resolvedStack, lookupOptions);
    if (existingClient) {
      setClientState({
        lookupKey,
        client: existingClient as ConnectedStack<StackDefinition, ProgramMap>,
      });
      setIsConnecting(false);
      setError(null);
      setSocketIssue(null);
      return () => {
        active = false;
      };
    }

    setClientState({ lookupKey, client: null });
    setIsConnecting(true);
    setError(null);
    setSocketIssue(null);

    getOrCreateClient(resolvedStack, lookupOptions)
      .then((newClient) => {
        if (active) {
          setClientState({
            lookupKey,
            client: newClient as ConnectedStack<StackDefinition, ProgramMap>,
          });
          setIsConnecting(false);
        }
      })
      .catch((err) => {
        if (active) {
          setError(err instanceof Error ? err : new Error(String(err)));
          setIsConnecting(false);
        }
      });
    return () => {
      active = false;
    };
  }, [resolvedStack, getOrCreateClient, getClient, lookupKey, lookupOptions]);

  useEffect(() => subscribeToClientChanges((change) => {
    if (change && change.cacheKey !== lookupKey) return;
    const sharedClient = getClient(resolvedStack, lookupOptions) as ConnectedStack<StackDefinition, ProgramMap> | null;
    const current = clientStateRef.current;

    if (change?.status === 'connecting') {
      const next = { lookupKey, client: null };
      clientStateRef.current = next;
      setClientState(next);
      setIsConnecting(true);
      setError(null);
      setSocketIssue(null);
      return;
    }
    if (change?.status === 'error') {
      const next = { lookupKey, client: null };
      clientStateRef.current = next;
      setClientState(next);
      setIsConnecting(false);
      setError(change.error ?? new Error('Arete connection attempt failed'));
      setSocketIssue(null);
      return;
    }
    if (current.lookupKey === lookupKey && current.client === sharedClient) return;

    const next = { lookupKey, client: sharedClient };
    clientStateRef.current = next;
    setClientState(next);
    setIsConnecting(sharedClient === null);
    setError(null);
    setSocketIssue(null);
  }), [getClient, lookupKey, lookupOptions, resolvedStack, subscribeToClientChanges]);

  useEffect(() => {
    if (!client) {
      setConnectionState('disconnected');
      return;
    }
    
    setConnectionState(client.connectionState);
    const unsubscribeState = client.onConnectionStateChange((state, stateError) => {
      setConnectionState(state);
      if (state === 'error') {
        setError(normalizeConnectionError(stateError, 'Arete connection entered an error state'));
      } else if (state === 'connected') {
        setError(null);
        setSocketIssue(null);
      }
    });
    const unsubscribeIssue = client.onSocketIssue((issue) => {
      setSocketIssue(issue);
    });

    return () => {
      unsubscribeIssue();
      unsubscribeState();
    };
  }, [client]);

  const retry = useCallback((): Promise<void> => {
    if (retryPromiseRef.current?.lookupKey === lookupKey) {
      return retryPromiseRef.current.promise;
    }

    const retryLookupKey = lookupKey;
    setError(null);
    setSocketIssue(null);
    setIsConnecting(true);
    setClientState({ lookupKey: retryLookupKey, client: null });

    const attempt = retryClient(resolvedStack, lookupOptions).then((newClient) => {
      if (lookupKeyRef.current === retryLookupKey) {
        setClientState({
          lookupKey: retryLookupKey,
          client: newClient as ConnectedStack<StackDefinition, ProgramMap>,
        });
        setIsConnecting(false);
      }
    });
    const tracked = attempt.catch((value) => {
      const retryError = normalizeConnectionError(value, 'Arete connection retry failed');
      if (lookupKeyRef.current === retryLookupKey) {
        setError(retryError);
        setIsConnecting(false);
      }
      throw retryError;
    }).finally(() => {
      if (retryPromiseRef.current?.promise === tracked) {
        retryPromiseRef.current = null;
      }
    });
    retryPromiseRef.current = { lookupKey, promise: tracked };
    return tracked;
  }, [lookupKey, lookupOptions, retryClient, resolvedStack]);

  const views = useMemo(() => {
    const result: Record<string, Record<string, unknown>> = {};

    for (const [viewName, viewGroup] of Object.entries(resolvedStack.views)) {
      result[viewName] = {};

      if (typeof viewGroup === 'object' && viewGroup !== null) {
        for (const [subViewName, viewDef] of Object.entries(viewGroup)) {
          if (!viewDef || typeof viewDef !== 'object' || !('mode' in viewDef)) continue;

          if (viewDef.mode === 'state') {
            result[viewName]![subViewName] = createStateViewHook(viewDef as ViewDef<unknown, 'state', unknown>, client as AnyClient | null);
          } else if (viewDef.mode === 'list') {
            result[viewName]![subViewName] = createListViewHook(viewDef as ViewDef<unknown, 'list'>, client as AnyClient | null);
          }
        }
      }
    }

    return result;
  }, [resolvedStack, client]);

  const programs = useMemo(() => {
    if (!client) {
      // Placeholder hooks so components can render (and keep hook order)
      // before the client connects; submitting throws "not connected".
      return buildDisconnectedProgramHooks();
    }
    return buildProgramHookInterfaces(
      client.programs as ProgramsInterface<Record<string, ProgramSdkDefinition>> | undefined,
      client as ConnectedArete<ResolvedStack<StackDefinition, ProgramMap>> | null,
      useInstructionMutation,
      { defaultReconciliation: transport !== 'http' }
    );
  }, [client, transport]);

  const connectedRead = ((client as (ConnectedStack<StackDefinition, ProgramMap> & { read?: unknown }) | null)?.read ?? null) as
    ConnectedField<StackConnectedExtensions<ResolvedStack<StackDefinition, ProgramMap>>, 'read'> | null;
  const readArgCounts = getStackRuntimeExtensions(resolvedStack)?.readArgCounts;
  const read = useMemo(
    () => buildReadInterfaces(
      connectedRead as Record<string, (...args: never[]) => unknown> | null,
      readArgCounts as ReadArgumentCounts<
        Record<string, (...args: never[]) => unknown>
      > | undefined,
    ),
    [connectedRead, readArgCounts]
  );

  const isClientPending = clientState.lookupKey !== lookupKey || isConnecting;
  const visibleError = clientState.lookupKey === lookupKey ? error : null;
  const status: ConnectionState = isClientPending
    ? 'connecting'
    : visibleError
      ? 'error'
      : client
        ? connectionState
        : 'disconnected';

  return {
    views: views as BuildViewInterface<StackDefinition['views']>,
    queries: (client?.queries ?? {}) as QueriesInterface<StackQueries<ResolvedStack<StackDefinition, ProgramMap>>>,
    programs: programs as BuildProgramInterface<StackPrograms<ResolvedStack<StackDefinition, ProgramMap>>, ConnectedStack<StackDefinition, ProgramMap>>,
    chain: client?.chain ?? null,
    zustandStore: (client?.store as ZustandAdapter | undefined)?.store ?? null,
    client,
    read,
    reads: read as UseAreteResult<StackDefinition, ProgramMap>['reads'],
    addresses: ((client as (ConnectedStack<StackDefinition, ProgramMap> & { addresses?: unknown }) | null)?.addresses
      ?? (resolvedStack as StackDefinition & { addresses?: unknown }).addresses) as ConnectedField<StackConnectedExtensions<ResolvedStack<StackDefinition, ProgramMap>>, 'addresses'>,
    constants: ((client as (ConnectedStack<StackDefinition, ProgramMap> & { constants?: unknown }) | null)?.constants
      ?? (resolvedStack as StackDefinition & { constants?: unknown }).constants) as ConnectedField<StackConnectedExtensions<ResolvedStack<StackDefinition, ProgramMap>>, 'constants'>,
    defaults: ((client as (ConnectedStack<StackDefinition, ProgramMap> & { defaults?: unknown }) | null)?.defaults
      ?? (resolvedStack as StackDefinition & { defaults?: unknown }).defaults) as ConnectedField<StackConnectedExtensions<ResolvedStack<StackDefinition, ProgramMap>>, 'defaults'>,
    math: ((client as (ConnectedStack<StackDefinition, ProgramMap> & { math?: unknown }) | null)?.math
      ?? (resolvedStack as StackDefinition & { math?: unknown }).math) as ConnectedField<StackConnectedExtensions<ResolvedStack<StackDefinition, ProgramMap>>, 'math'>,
    connectionState: client ? connectionState : 'disconnected',
    status,
    isConnected: Boolean(client) && connectionState === 'connected',
    isLoading: isClientPending,
    canRetry: config.autoConnect !== false
      && (status === 'error' || status === 'disconnected'),
    error: visibleError,
    socketIssue: clientState.lookupKey === lookupKey ? socketIssue : null,
    retry,
  } as unknown as UseAreteResult<StackDefinition, ProgramMap>;
}
