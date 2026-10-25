import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { StoreApi, UseBoundStore } from 'zustand';
import { useAreteContext } from './provider';
import { useOptionalAreteSession } from './session-provider';
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

/**
 * A connected stack client: a session member (`session.stacks.<key>` from
 * `createSession`) or a client from `Arete.connect`.
 */
export type AnyStackClient = Pick<
  AnyClient,
  'getSubscriptionRegistry' | 'onConnectionStateChange' | 'onSocketIssue' | 'connectionState'
>;

/** The stack a connected client was connected for (with its attached programs). */
export type StackOfClient<TClient> = NonNullable<TClient> extends ConnectedArete<infer TStack extends StackDefinition, infer _TExtensions>
  ? TStack
  : never;

/**
 * The definition of a member's stack, for `useArete(member, { stack })`: any
 * stack definition with the member's views (the generated stack, before or
 * after `programs` are attached).
 */
export type MemberStackDefinition<TStack extends StackDefinition> = StackDefinition
  & Pick<TStack, 'views'>;

/** Options for `useArete(member)`, where `member` is a client someone else connected. */
export interface UseAreteMemberOptions<TStack extends StackDefinition = StackDefinition> {
  /**
   * The member's stack definition. Required while the member can be `null` or
   * `undefined` (the session is still opening), so views, programs,
   * addresses and constants exist before it opens; once it has opened, the
   * member's own `stack` is used.
   */
  stack?: MemberStackDefinition<TStack>;
}

function isStackClient(value: unknown): value is AnyClient {
  return typeof value === 'object'
    && value !== null
    && typeof (value as Partial<AnyClient>).getSubscriptionRegistry === 'function'
    && typeof (value as Partial<AnyClient>).onConnectionStateChange === 'function';
}

function isMemberOptions(value: unknown): value is UseAreteMemberOptions & { stack: StackDefinition } {
  const stack = (value as { stack?: unknown } | null | undefined)?.stack;
  return typeof stack === 'object' && stack !== null && 'views' in stack;
}

function zustandStoreOf(client: AnyClient | null): UseBoundStore<StoreApi<AreteStore>> | null {
  return (client?.store as ZustandAdapter | undefined)?.store ?? null;
}

interface MemberConnection {
  readonly client: AnyClient | null;
  readonly state: ConnectionState;
  readonly error: Error | null;
  readonly socketIssue: SocketIssue | null;
}

/**
 * `useArete` for a client the caller (usually a session) connected and
 * owns. It reads the member's own connection and store: nothing connects,
 * and nothing is disconnected when the component unmounts or the member is
 * replaced. A `null` member reads as the connecting state `useArete(stack)`
 * reports before its client exists.
 */
function useMemberArete(
  member: AnyClient | null,
  options: UseAreteMemberOptions | undefined
): UseAreteResult<StackDefinition, ProgramMap> {
  const client = member;
  const stack: StackDefinition | undefined = client?.stack ?? options?.stack;
  if (!stack) {
    throw new Error(
      'useArete(member) could not read the stack definition of the member it was given. '
      + 'Pass it as useArete(member, { stack }).'
    );
  }
  // Mutations wait for the streamed slot they landed in only when the member
  // was connected over the socket.
  const transport = client?.transport;
  const sessionContext = useOptionalAreteSession();

  const [connection, setConnection] = useState<MemberConnection>(() => ({
    client,
    state: client?.connectionState ?? 'disconnected',
    error: null,
    socketIssue: null,
  }));
  // A new member (a reopened session) never shows the previous one's state.
  const current: MemberConnection = connection.client === client
    ? connection
    : { client, state: client?.connectionState ?? 'disconnected', error: null, socketIssue: null };

  useEffect(() => {
    if (!client) {
      setConnection((previous) => previous.client === null
        ? previous
        : { client: null, state: 'disconnected', error: null, socketIssue: null });
      return undefined;
    }
    const update = (next: Partial<Omit<MemberConnection, 'client'>>) => {
      setConnection((previous) => {
        const base = previous.client === client
          ? previous
          : { client, state: client.connectionState, error: null, socketIssue: null };
        return { ...base, ...next };
      });
    };
    update({ state: client.connectionState });
    const unsubscribeState = client.onConnectionStateChange((state, stateError) => {
      if (state === 'error') {
        update({ state, error: normalizeConnectionError(stateError, 'Arete connection entered an error state') });
      } else if (state === 'connected') {
        update({ state, error: null, socketIssue: null });
      } else {
        update({ state });
      }
    });
    const unsubscribeIssue = client.onSocketIssue((socketIssue) => {
      update({ socketIssue });
    });
    // Only listeners are removed: the member's connection is its owner's.
    return () => {
      unsubscribeIssue();
      unsubscribeState();
    };
  }, [client]);

  const views = useMemo(
    () => buildViewHooks(stack, client as AnyClient | null),
    [stack, client]
  );

  const programs = useMemo(() => {
    if (!client) {
      return buildDisconnectedProgramHooks(
        stack.programs as Record<string, ProgramSdkDefinition> | undefined,
      );
    }
    return buildProgramHookInterfaces(
      client.programs as ProgramsInterface<Record<string, ProgramSdkDefinition>> | undefined,
      client as ConnectedArete<ResolvedStack<StackDefinition, ProgramMap>> | null,
      useInstructionMutation,
      { defaultReconciliation: transport !== 'http' }
    );
  }, [client, stack, transport]);

  const connectedRead = ((client as (ConnectedStack<StackDefinition, ProgramMap> & { read?: unknown }) | null)?.read ?? null) as
    Record<string, (...args: never[]) => unknown> | null;
  const readArgCounts = getStackRuntimeExtensions(stack)?.readArgCounts;
  const read = useMemo(
    () => buildReadInterfaces(
      connectedRead,
      readArgCounts as ReadArgumentCounts<Record<string, (...args: never[]) => unknown>> | undefined,
    ),
    [connectedRead, readArgCounts]
  );

  const sessionStatus = sessionContext?.status;
  const sessionError = sessionContext?.error ?? null;
  const sessionRetry = sessionContext?.retry;
  const sessionFailed = !client && sessionStatus === 'error';
  const retry = useCallback(async (): Promise<void> => {
    // The member's connection belongs to its session: retrying reopens a
    // session that failed to open, and does nothing otherwise.
    if (sessionFailed) sessionRetry?.();
  }, [sessionFailed, sessionRetry]);

  const status: ConnectionState = client
    ? current.error && current.state !== 'connected' ? 'error' : current.state
    : sessionFailed ? 'error' : 'connecting';
  const error = client ? current.error : sessionFailed ? sessionError : null;
  const extension = (name: 'addresses' | 'constants' | 'defaults' | 'math') =>
    (client as Record<string, unknown> | null)?.[name]
      ?? (stack as StackDefinition & Record<string, unknown>)[name];

  return {
    views,
    queries: client?.queries ?? {},
    programs,
    chain: client?.chain ?? null,
    zustandStore: zustandStoreOf(client),
    client,
    read,
    reads: read,
    addresses: extension('addresses'),
    constants: extension('constants'),
    defaults: extension('defaults'),
    math: extension('math'),
    connectionState: client ? current.state : 'disconnected',
    status,
    isConnected: Boolean(client) && current.state === 'connected',
    isLoading: !client && !sessionFailed,
    canRetry: sessionFailed,
    error,
    socketIssue: client ? current.socketIssue : null,
    retry,
  } as unknown as UseAreteResult<StackDefinition, ProgramMap>;
}

function buildViewHooks(
  stack: StackDefinition,
  client: AnyClient | null
): Record<string, Record<string, unknown>> {
  const result: Record<string, Record<string, unknown>> = {};

  for (const [viewName, viewGroup] of Object.entries(stack.views)) {
    result[viewName] = {};

    if (typeof viewGroup === 'object' && viewGroup !== null) {
      for (const [subViewName, viewDef] of Object.entries(viewGroup)) {
        if (!viewDef || typeof viewDef !== 'object' || !('mode' in viewDef)) continue;

        if (viewDef.mode === 'state') {
          result[viewName]![subViewName] = createStateViewHook(viewDef as ViewDef<unknown, 'state', unknown>, client);
        } else if (viewDef.mode === 'list') {
          result[viewName]![subViewName] = createListViewHook(viewDef as ViewDef<unknown, 'list'>, client);
        }
      }
    }
  }

  return result;
}

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
/**
 * The hooks for a stack client that is already connected, typically a
 * session member: `useArete(session.stacks.ore)`. The hooks read the
 * member's own connection and store, so nothing connects again, and the
 * member's owner (the session) disconnects it; unmounting never does.
 * Mutations sign with the member's wallet, which `session.setWallet` (or
 * `AreteSessionProvider`'s `wallet`) sets.
 *
 * While the session opens, pass the member as it may be (`null` or
 * `undefined`) together with its stack definition, and the result is the
 * connecting state `useArete(stack)` reports before its client exists:
 *
 * ```tsx
 * const { session } = useAreteSession<typeof APP>();
 * const ore = useArete(session?.stacks.ore, { stack: APP.stacks.ore });
 * ```
 */
export function useArete<TClient extends AnyStackClient>(
  member: TClient,
  options?: UseAreteMemberOptions<StackOfClient<TClient>>
): UseAreteResult<StackOfClient<TClient>>;
export function useArete<TClient extends AnyStackClient>(
  member: TClient | null | undefined,
  options: UseAreteMemberOptions<StackOfClient<TClient>> & {
    stack: MemberStackDefinition<StackOfClient<TClient>>;
  }
): UseAreteResult<StackOfClient<TClient>>;
export function useArete(
  stackOrMember?: StackDefinition | AnyClient | null,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  options?: any
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
): any {
  // A call site keeps one form for its lifetime (a stack definition, or a
  // session member that may be null while the session opens), so the hooks
  // either branch calls stay in the same order.
  if (isStackClient(stackOrMember)) {
    // eslint-disable-next-line react-hooks/rules-of-hooks -- one form per call site
    return useMemberArete(stackOrMember, options as UseAreteMemberOptions | undefined);
  }
  if (stackOrMember == null && isMemberOptions(options)) {
    // eslint-disable-next-line react-hooks/rules-of-hooks -- one form per call site
    return useMemberArete(null, options);
  }
  // eslint-disable-next-line react-hooks/rules-of-hooks -- one form per call site
  return useStackArete((stackOrMember ?? undefined) as StackDefinition | undefined, options);
}

function useStackArete(
  stack: StackDefinition | undefined,
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
  const chain = resolvedOptions?.chain;
  const transactions = resolvedOptions?.transactions;
  const attachedPrograms = resolvedOptions?.programs;
  const lookupOptions = useMemo(
    () => ({ url, httpUrl, transport, chain, transactions, programs: attachedPrograms }) as ClientLookupOptions<ProgramMap>,
    [url, httpUrl, transport, chain, transactions, attachedPrograms]
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

  const views = useMemo(
    () => buildViewHooks(resolvedStack, client as AnyClient | null),
    [resolvedStack, client]
  );

  const programs = useMemo(() => {
    if (!client) {
      // Placeholder hooks so components can render (and keep hook order)
      // before the client connects: addresses, PDAs and constants come from
      // the definitions, while submitting throws "not connected".
      return buildDisconnectedProgramHooks(
        resolvedStack.programs as Record<string, ProgramSdkDefinition> | undefined,
        attachedPrograms as Record<string, ProgramSdkDefinition> | undefined,
      );
    }
    return buildProgramHookInterfaces(
      client.programs as ProgramsInterface<Record<string, ProgramSdkDefinition>> | undefined,
      client as ConnectedArete<ResolvedStack<StackDefinition, ProgramMap>> | null,
      useInstructionMutation,
      { defaultReconciliation: transport !== 'http' }
    );
  }, [client, transport, resolvedStack, attachedPrograms]);

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
    zustandStore: zustandStoreOf(client),
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
