import { useEffect, useMemo, useState } from 'react';
import type { StoreApi, UseBoundStore } from 'zustand';
import { useAreteContext } from './provider';
import { buildProgramHookInterfaces, type BuildProgramInterface } from './program-hooks';
import { createStateViewHook, createListViewHook } from './view-hooks';
import { useInstructionMutation } from './hooks';
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

type StackClient<
  TStack extends StackDefinition,
  TPrograms extends ProgramMap | undefined,
> = {
  views: BuildViewInterface<TStack['views']>;
  queries: QueriesInterface<StackQueries<ResolvedStack<TStack, TPrograms>>>;
  programs: BuildProgramInterface<StackPrograms<ResolvedStack<TStack, TPrograms>>, ConnectedArete<ResolvedStack<TStack, TPrograms>>>;
  chain: ChainClient;
  zustandStore: UseBoundStore<StoreApi<AreteStore>>;
  client: ConnectedArete<ResolvedStack<TStack, TPrograms>>;
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
): StackClient<TStack, TPrograms> {
  const { getOrCreateClient, getClient } = useAreteContext();
  const url = options?.url;
  const httpUrl = options?.httpUrl;
  const transport = options?.transport;
  const attachedPrograms = options?.programs;
  const lookupOptions = useMemo(
    () => ({ url, httpUrl, transport, programs: attachedPrograms }) as ClientLookupOptions<TPrograms>,
    [url, httpUrl, transport, attachedPrograms]
  );
  const [client, setClient] = useState<ConnectedArete<ResolvedStack<TStack, TPrograms>> | null>(
    getClient(stack, lookupOptions) as ConnectedArete<ResolvedStack<TStack, TPrograms>> | null
  );
  const [isLoading, setIsLoading] = useState(!client);
  const [error, setError] = useState<Error | null>(null);
  const [connectionState, setConnectionState] = useState<ConnectionState>(() =>
    client?.connectionState ?? 'disconnected'
  );

  useEffect(() => {
    const existingClient = getClient(stack, lookupOptions);
    if (existingClient) {
      setClient(existingClient as ConnectedArete<ResolvedStack<TStack, TPrograms>>);
      setIsLoading(false);
      return;
    }

    setIsLoading(true);
    setError(null);

    getOrCreateClient(stack, lookupOptions)
      .then((newClient) => {
        setClient(newClient as ConnectedArete<ResolvedStack<TStack, TPrograms>>);
        setIsLoading(false);
      })
      .catch((err) => {
        setError(err instanceof Error ? err : new Error(String(err)));
        setIsLoading(false);
      });
  }, [stack, getOrCreateClient, getClient, lookupOptions]);

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
    programs: programs as BuildProgramInterface<StackPrograms<ResolvedStack<TStack, TPrograms>>, ConnectedArete<ResolvedStack<TStack, TPrograms>>>,
    chain: client?.chain as ChainClient,
    zustandStore: (client?.store as ZustandAdapter | undefined)?.store as UseBoundStore<StoreApi<AreteStore>>,
    client: client!,
    connectionState,
    isConnected: connectionState === 'connected',
    isLoading,
    error
  };
}
