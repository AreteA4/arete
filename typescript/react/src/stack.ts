import { useEffect, useState, useMemo } from 'react';
import type { ConnectionState } from '@usearete/sdk';
import type { StoreApi, UseBoundStore } from 'zustand';
import { useAreteContext } from './provider';
import { createStateViewHook, createListViewHook } from './view-hooks';
import { useInstructionMutation, UseMutationResult } from './hooks';
import type {
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
import type { InstructionHandler, InstructionExecutor } from '@usearete/sdk';
import type { Arete } from '@usearete/sdk';

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

type InstructionHook = {
  useMutation: () => UseMutationResult;
  execute: InstructionExecutor;
};

type BuildInstructionInterface<TInstructions extends Record<string, InstructionHandler> | undefined> = 
  TInstructions extends Record<string, InstructionHandler>
    ? { [K in keyof TInstructions]: InstructionHook }
    : {};

type StackClient<TStack extends StackDefinition> = {
  views: BuildViewInterface<TStack['views']>;
  instructions: BuildInstructionInterface<TStack['instructions']>;
  zustandStore: UseBoundStore<StoreApi<AreteStore>>;
  client: Arete<TStack>;
  connectionState: ConnectionState;
  isConnected: boolean;
  isLoading: boolean;
  error: Error | null;
};

export function useArete<TStack extends StackDefinition>(
  stack: TStack,
  options?: UseAreteOptions
): StackClient<TStack> {
  const { getOrCreateClient, getClient } = useAreteContext();
  const urlOverride = options?.url;
  const [client, setClient] = useState<Arete<TStack> | null>(getClient(stack) as Arete<TStack> | null);
  const [isLoading, setIsLoading] = useState(!client);
  const [error, setError] = useState<Error | null>(null);
  const [connectionState, setConnectionState] = useState<ConnectionState>(() => 
    client?.connectionState ?? 'disconnected'
  );

  useEffect(() => {
    const existingClient = getClient(stack);
    if (existingClient && !urlOverride) {
      setClient(existingClient as Arete<TStack>);
      setIsLoading(false);
      return;
    }

    setIsLoading(true);
    setError(null);

    getOrCreateClient(stack, urlOverride)
      .then((newClient) => {
        setClient(newClient as Arete<TStack>);
        setIsLoading(false);
      })
      .catch((err) => {
        setError(err instanceof Error ? err : new Error(String(err)));
        setIsLoading(false);
      });
  }, [stack, getOrCreateClient, getClient, urlOverride]);

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
            result[viewName]![subViewName] = createStateViewHook(viewDef as ViewDef<unknown, 'state'>, client);
          } else if (viewDef.mode === 'list') {
            result[viewName]![subViewName] = createListViewHook(viewDef as ViewDef<unknown, 'list'>, client);
          }
        }
      }
    }

    return result;
  }, [stack, client]);

  const instructions = useMemo(() => {
    const result: Record<string, InstructionHook> = {};

    if (client?.instructions) {
      for (const [instructionName, executeFn] of Object.entries(client.instructions)) {
        result[instructionName] = {
          execute: executeFn as InstructionExecutor,
          useMutation: () => useInstructionMutation(executeFn as InstructionExecutor)
        };
      }
    }

    return result;
  }, [client]);

  return {
    views: views as BuildViewInterface<TStack['views']>,
    instructions: instructions as BuildInstructionInterface<TStack['instructions']>,
    zustandStore: (client?.store as ZustandAdapter | undefined)?.store as UseBoundStore<StoreApi<AreteStore>>,
    client: client!,
    connectionState,
    isConnected: connectionState === 'connected',
    isLoading,
    error
  };
}
