import { useEffect, useMemo, useState } from 'react';
import type { StoreApi, UseBoundStore } from 'zustand';
import { useAreteContext } from './provider';
import { buildProgramHookInterfaces, type BuildProgramInterface } from './program-hooks';
import { createStateViewHook, createListViewHook } from './view-hooks';
import { useInstructionMutation } from './hooks';
import { createClientCacheKey } from './client-key';
import type {
  ConnectionState,
  ClientLookupOptions,
  StackDefinition,
  ViewDef,
  ViewMode,
  ViewHookOptions,
  ViewHookResult,
  ListParamsSingle,
  ListParamsMultiple,
  ListParamsBase,
  ViewGroup,
  UseAreteOptions
} from './types';
import { ZustandAdapter, type AreteStore } from './zustand-adapter';
import type {
  ChainClient,
  ConnectedArete,
  ProgramSdkDefinition,
  ProgramsInterface,
  QueriesInterface,
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

type ViewHookForDef<TDef> = TDef extends ViewDef<infer T, 'state'>
  ? {
      use: <TSchema = T>(
        key?: Record<string, string>,
        options?: ViewHookOptions<TSchema>
      ) => ViewHookResult<TSchema>;
    }
  : TDef extends ViewDef<infer T, 'list'>
  ? {
      use: {
        <TSchema = T>(params: ListParamsSingle<TSchema>, options?: ViewHookOptions<TSchema>): ViewHookResult<TSchema | undefined>;
        <TSchema = T>(params?: ListParamsMultiple<TSchema>, options?: ViewHookOptions<TSchema>): ViewHookResult<TSchema[]>;
      };
      useOne: <TSchema = T>(
        params?: Omit<ListParamsBase<TSchema>, 'take'>,
        options?: ViewHookOptions<TSchema>
      ) => ViewHookResult<TSchema | undefined>;
    }
  : TDef extends ViewDef<infer T, 'state' | 'list'>
  ? {
      use: {
        <TSchema = T>(params: ListParamsSingle<TSchema>, options?: ViewHookOptions<TSchema>): ViewHookResult<TSchema | undefined>;
        <TSchema = T>(params?: ListParamsMultiple<TSchema> | Record<string, string>, options?: ViewHookOptions<TSchema>): ViewHookResult<TSchema | TSchema[]>;
      };
      useOne: <TSchema = T>(
        params?: Omit<ListParamsBase<TSchema>, 'take'>,
        options?: ViewHookOptions<TSchema>
      ) => ViewHookResult<TSchema | undefined>;
    }
  : never;

type BuildViewInterface<TViews extends Record<string, ViewGroup>> = {
  [K in keyof TViews]: {
    [SubK in keyof TViews[K] as TViews[K][SubK] extends ViewDef<unknown, ViewMode> ? SubK : never]: ViewHookForDef<TViews[K][SubK]>;
  };
};

export type UseAreteResult<
  TStack extends StackDefinition,
  TPrograms extends ProgramMap | undefined,
> = {
  views: BuildViewInterface<TStack['views']>;
  queries: QueriesInterface<StackQueries<ResolvedStack<TStack, TPrograms>>>;
  programs: BuildProgramInterface<StackPrograms<ResolvedStack<TStack, TPrograms>>, ConnectedStack<TStack, TPrograms>>;
  chain: ChainClient | null;
  zustandStore: UseBoundStore<StoreApi<AreteStore>> | null;
  client: ConnectedStack<TStack, TPrograms> | null;
  read: ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'read'> | null;
  addresses: ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'addresses'>;
  constants: ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'constants'>;
  defaults: ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'defaults'>;
  math: ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'math'>;
  connectionState: ConnectionState;
  isConnected: boolean;
  isLoading: boolean;
  error: Error | null;
};

export function useArete<
  TStack extends StackDefinition,
  TPrograms extends ProgramMap | undefined = undefined,
>(
  stack: TStack,
  options?: UseAreteOptions<TPrograms>
): UseAreteResult<TStack, TPrograms> {
  const { getOrCreateClient, getClient } = useAreteContext();
  const url = options?.url;
  const httpUrl = options?.httpUrl;
  const transport = options?.transport;
  const attachedPrograms = options?.programs;
  const lookupOptions = useMemo(
    () => ({ url, httpUrl, transport, programs: attachedPrograms }) as ClientLookupOptions<TPrograms>,
    [url, httpUrl, transport, attachedPrograms]
  );
  const lookupKey = createClientCacheKey(stack, lookupOptions);
  const initialClient = getClient(stack, lookupOptions) as ConnectedStack<TStack, TPrograms> | null;
  const [clientState, setClientState] = useState<{
    lookupKey: string | null;
    client: ConnectedStack<TStack, TPrograms> | null;
  }>(() => ({ lookupKey, client: initialClient }));
  const client = clientState.lookupKey === lookupKey ? clientState.client : null;
  const [isConnecting, setIsConnecting] = useState(!initialClient);
  const [error, setError] = useState<Error | null>(null);
  const [connectionState, setConnectionState] = useState<ConnectionState>(() =>
    client?.connectionState ?? 'disconnected'
  );

  useEffect(() => {
    let active = true;
    const existingClient = getClient(stack, lookupOptions);
    if (existingClient) {
      setClientState({
        lookupKey,
        client: existingClient as ConnectedStack<TStack, TPrograms>,
      });
      setIsConnecting(false);
      return () => {
        active = false;
      };
    }

    setClientState({ lookupKey, client: null });
    setIsConnecting(true);
    setError(null);

    getOrCreateClient(stack, lookupOptions)
      .then((newClient) => {
        if (active) {
          setClientState({
            lookupKey,
            client: newClient as ConnectedStack<TStack, TPrograms>,
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
  }, [stack, getOrCreateClient, getClient, lookupKey, lookupOptions]);

  useEffect(() => {
    if (!client) {
      setConnectionState('disconnected');
      return;
    }
    
    setConnectionState(client.connectionState);
    const unsubscribe = client.onConnectionStateChange((state) => {
      setConnectionState(state);
    });
    
    return unsubscribe;
  }, [client]);

  const views = useMemo(() => {
    const result: Record<string, Record<string, unknown>> = {};

    for (const [viewName, viewGroup] of Object.entries(stack.views)) {
      result[viewName] = {};

      if (typeof viewGroup === 'object' && viewGroup !== null) {
        for (const [subViewName, viewDef] of Object.entries(viewGroup)) {
          if (!viewDef || typeof viewDef !== 'object' || !('mode' in viewDef)) continue;

          if (viewDef.mode === 'state') {
            result[viewName]![subViewName] = createStateViewHook(viewDef as ViewDef<unknown, 'state'>, client as AnyClient | null);
          } else if (viewDef.mode === 'list') {
            result[viewName]![subViewName] = createListViewHook(viewDef as ViewDef<unknown, 'list'>, client as AnyClient | null);
          }
        }
      }
    }

    return result;
  }, [stack, client]);

  const programs = useMemo(() => {
    return buildProgramHookInterfaces(
      client?.programs as ProgramsInterface<Record<string, ProgramSdkDefinition>> | undefined,
      client as ConnectedArete<ResolvedStack<TStack, TPrograms>> | null,
      useInstructionMutation
    );
  }, [client]);

  return {
    views: views as BuildViewInterface<TStack['views']>,
    queries: (client?.queries ?? {}) as QueriesInterface<StackQueries<ResolvedStack<TStack, TPrograms>>>,
    programs: programs as BuildProgramInterface<StackPrograms<ResolvedStack<TStack, TPrograms>>, ConnectedStack<TStack, TPrograms>>,
    chain: client?.chain ?? null,
    zustandStore: (client?.store as ZustandAdapter | undefined)?.store ?? null,
    client,
    read: (client as (ConnectedStack<TStack, TPrograms> & { read?: unknown }) | null)?.read as ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'read'> ?? null,
    addresses: ((client as (ConnectedStack<TStack, TPrograms> & { addresses?: unknown }) | null)?.addresses
      ?? (stack as TStack & { addresses?: unknown }).addresses) as ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'addresses'>,
    constants: ((client as (ConnectedStack<TStack, TPrograms> & { constants?: unknown }) | null)?.constants
      ?? (stack as TStack & { constants?: unknown }).constants) as ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'constants'>,
    defaults: ((client as (ConnectedStack<TStack, TPrograms> & { defaults?: unknown }) | null)?.defaults
      ?? (stack as TStack & { defaults?: unknown }).defaults) as ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'defaults'>,
    math: ((client as (ConnectedStack<TStack, TPrograms> & { math?: unknown }) | null)?.math
      ?? (stack as TStack & { math?: unknown }).math) as ConnectedField<StackConnectedExtensions<ResolvedStack<TStack, TPrograms>>, 'math'>,
    connectionState: client ? connectionState : 'disconnected',
    isConnected: Boolean(client) && connectionState === 'connected',
    isLoading: clientState.lookupKey !== lookupKey || isConnecting,
    error: clientState.lookupKey === lookupKey ? error : null
  };
}
