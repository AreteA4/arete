import React, { createContext, useContext, useEffect, useRef, ReactNode, useCallback } from 'react';
import {
  Arete,
  type ConnectedArete,
  type ConnectionState,
  type ProgramSdkDefinition,
  type StackDefinition,
  type StackWithAttachedPrograms,
} from '@usearete/sdk';
import type { AreteConfig, ClientLookupOptions } from './types';
import { DEFAULT_FLUSH_INTERVAL_MS } from './types';
import { ZustandAdapter } from './zustand-adapter';
import { initializeConnectedClient, syncClientWallets } from './wallet-sync';
import { createClientCacheKey } from './client-key';
import { trackConnectingPromise } from './provider-cache';

type AnyClient = ConnectedArete<StackDefinition>;
type ProgramMap = Record<string, ProgramSdkDefinition>;
type ResolvedStack<
  TStack extends StackDefinition,
  TPrograms extends ProgramMap | undefined,
> = StackWithAttachedPrograms<TStack, TPrograms>;

interface ClientEntry {
  client: AnyClient;
  disconnect: () => void;
}

interface ClientChange {
  cacheKey: string;
  status: 'connecting' | 'connected' | 'error';
  error?: Error;
}

interface AreteContextValue {
  getOrCreateClient: <
    TStack extends StackDefinition,
    TPrograms extends ProgramMap | undefined = undefined,
  >(
    stack: TStack,
    options?: ClientLookupOptions<TPrograms>
  ) => Promise<ConnectedArete<ResolvedStack<TStack, TPrograms>>>;
  getClient: <
    TStack extends StackDefinition,
    TPrograms extends ProgramMap | undefined = undefined,
  >(
    stack: TStack | undefined,
    options?: ClientLookupOptions<TPrograms>
  ) => ConnectedArete<ResolvedStack<TStack, TPrograms>> | null;
  retryClient: <
    TStack extends StackDefinition,
    TPrograms extends ProgramMap | undefined = undefined,
  >(
    stack: TStack,
    options?: ClientLookupOptions<TPrograms>
  ) => Promise<ConnectedArete<ResolvedStack<TStack, TPrograms>>>;
  subscribeToClientChanges: (callback: (change?: ClientChange) => void) => () => void;
  config: AreteConfig;
}

const AreteContext = createContext<AreteContextValue | null>(null);

export function AreteProvider({
  children,
  ...config
}: AreteConfig & {
  children: ReactNode;
}) {
  const clientsRef = useRef<Map<string, ClientEntry>>(new Map());
  const connectingRef = useRef<Map<string, Promise<AnyClient>>>(new Map());
  const clientChangeListenersRef = useRef<Set<(change?: ClientChange) => void>>(new Set());
  const latestWalletRef = useRef(config.wallet);
  const lifecycleRef = useRef({ active: true, generation: 0 });

  latestWalletRef.current = config.wallet;
  
  const notifyClientChange = useCallback((change?: ClientChange) => {
    clientChangeListenersRef.current.forEach(cb => { cb(change); });
  }, []);
  
  const subscribeToClientChanges = useCallback((callback: (change?: ClientChange) => void) => {
    clientChangeListenersRef.current.add(callback);
    return () => {
      clientChangeListenersRef.current.delete(callback);
    };
  }, []);

  const getOrCreateClient = useCallback(async <
    TStack extends StackDefinition,
    TPrograms extends ProgramMap | undefined = undefined,
  >(
    stack: TStack,
    options?: ClientLookupOptions<TPrograms>
  ): Promise<ConnectedArete<ResolvedStack<TStack, TPrograms>>> => {
    const cacheKey = createClientCacheKey(stack, options);
    if (!cacheKey) {
      throw new Error('Stack is required to create an Arete client');
    }

    const existing = clientsRef.current.get(cacheKey);
    if (existing) {
      return existing.client as unknown as ConnectedArete<ResolvedStack<TStack, TPrograms>>;
    }

    const connecting = connectingRef.current.get(cacheKey);
    if (connecting) {
      return connecting as unknown as Promise<ConnectedArete<ResolvedStack<TStack, TPrograms>>>;
    }

    const adapter = new ZustandAdapter();
    const lifecycleGeneration = lifecycleRef.current.generation;
    let connectionPromise!: Promise<AnyClient>;
    connectionPromise = Arete.connect(stack, {
      url: options?.url,
      httpUrl: options?.httpUrl,
      transport: options?.transport,
      programs: options?.programs,
      storage: adapter,
      autoConnect: config.autoConnect,
      autoReconnect: config.autoReconnect,
      reconnectIntervals: config.reconnectIntervals,
      maxReconnectAttempts: config.maxReconnectAttempts,
      maxEntriesPerView: config.maxEntriesPerView,
      flushIntervalMs: config.flushIntervalMs ?? DEFAULT_FLUSH_INTERVAL_MS,
      fetch: config.fetch,
      validateFrames: config.validateFrames,
      onFrameValidationError: config.onFrameValidationError,
      auth: config.auth,
      wallet: latestWalletRef.current,
    }).then((client) => {
      const ownsAttempt = lifecycleRef.current.active
        && lifecycleRef.current.generation === lifecycleGeneration
        && connectingRef.current.get(cacheKey) === connectionPromise;
      if (!ownsAttempt) {
        client.disconnect();
        throw new Error('Arete connection attempt was superseded');
      }

      initializeConnectedClient(client, adapter, latestWalletRef.current);

      clientsRef.current.set(cacheKey, {
        client: client as unknown as AnyClient,
        disconnect: () => client.disconnect()
      });
      connectingRef.current.delete(cacheKey);
      notifyClientChange({ cacheKey, status: 'connected' });
      return client as unknown as AnyClient;
    }).catch((value) => {
      const error = value instanceof Error ? value : new Error(String(value));
      if (connectingRef.current.get(cacheKey) === connectionPromise) {
        connectingRef.current.delete(cacheKey);
        if (
          lifecycleRef.current.active
          && lifecycleRef.current.generation === lifecycleGeneration
        ) {
          notifyClientChange({ cacheKey, status: 'error', error });
        }
      }
      throw error;
    });
    
    trackConnectingPromise(
      connectingRef.current,
      cacheKey,
      connectionPromise as unknown as Promise<AnyClient>
    );
    notifyClientChange({ cacheKey, status: 'connecting' });
    return connectionPromise as unknown as Promise<ConnectedArete<ResolvedStack<TStack, TPrograms>>>;
  }, [config.autoConnect, config.autoReconnect, config.reconnectIntervals, config.maxReconnectAttempts, config.maxEntriesPerView, config.flushIntervalMs, config.fetch, config.validateFrames, config.onFrameValidationError, config.auth, notifyClientChange]);

  const getClient = useCallback(<
    TStack extends StackDefinition,
    TPrograms extends ProgramMap | undefined = undefined,
  >(
    stack: TStack | undefined,
    options?: ClientLookupOptions<TPrograms>
  ): ConnectedArete<ResolvedStack<TStack, TPrograms>> | null => {
    if (!stack) {
      if (clientsRef.current.size === 1) {
          const firstEntry = clientsRef.current.values().next().value;
          return firstEntry ? (firstEntry.client as unknown as ConnectedArete<ResolvedStack<TStack, TPrograms>>) : null;
      }
      return null;
    }
    const cacheKey = createClientCacheKey(stack, options);
    if (!cacheKey) {
      return null;
    }
    const entry = clientsRef.current.get(cacheKey);
    return entry ? (entry.client as unknown as ConnectedArete<ResolvedStack<TStack, TPrograms>>) : null;
  }, []);

  const retryClient = useCallback(<
    TStack extends StackDefinition,
    TPrograms extends ProgramMap | undefined = undefined,
  >(
    stack: TStack,
    options?: ClientLookupOptions<TPrograms>
  ): Promise<ConnectedArete<ResolvedStack<TStack, TPrograms>>> => {
    const cacheKey = createClientCacheKey(stack, options);
    if (!cacheKey) {
      return Promise.reject(new Error('Stack is required to retry an Arete client'));
    }
    const existing = clientsRef.current.get(cacheKey);
    if (existing) {
      existing.disconnect();
      clientsRef.current.delete(cacheKey);
    }
    return getOrCreateClient(stack, options);
  }, [getOrCreateClient]);

  useEffect(() => {
    syncClientWallets(clientsRef.current.values(), config.wallet);
    notifyClientChange();
  }, [config.wallet, notifyClientChange]);

  useEffect(() => {
    const lifecycle = lifecycleRef.current;
    const clients = clientsRef.current;
    const connecting = connectingRef.current;
    lifecycle.active = true;
    return () => {
      lifecycle.active = false;
      lifecycle.generation++;
      clients.forEach((entry) => {
        entry.disconnect();
      });
      clients.clear();
      connecting.clear();
    };
  }, []);

  const value: AreteContextValue = {
    getOrCreateClient,
    getClient,
    retryClient,
    subscribeToClientChanges,
    config,
  };

  return (
    <AreteContext.Provider value={value}>
      {children}
    </AreteContext.Provider>
  );
}

export function useAreteContext() {
  const context = useContext(AreteContext);
  if (!context) {
    throw new Error('useAreteContext must be used within AreteProvider');
  }
  return context;
}

export function useConnectionState(
  stack?: StackDefinition,
  options?: ClientLookupOptions
): ConnectionState {
  const { getClient, subscribeToClientChanges } = useAreteContext();
  const [state, setState] = React.useState<ConnectionState>(() => {
    const client = getClient(stack, options);
    return client?.connectionState ?? 'disconnected';
  });
  const unsubscribeRef = React.useRef<(() => void) | undefined>(undefined);
  
  React.useEffect(() => {
    let mounted = true;
    
    const setupClientSubscription = () => {
      unsubscribeRef.current?.();
      unsubscribeRef.current = undefined;
      
        const client = getClient(stack, options);
        if (client && mounted) {
          setState(client.connectionState);
          unsubscribeRef.current = client.onConnectionStateChange((newState: ConnectionState) => {
          if (mounted) setState(newState);
        });
      } else if (mounted) {
        setState('disconnected');
      }
    };
    
    const unsubscribeFromClientChanges = subscribeToClientChanges(setupClientSubscription);
    setupClientSubscription();
    
    return () => {
      mounted = false;
      unsubscribeFromClientChanges();
      unsubscribeRef.current?.();
    };
  }, [getClient, subscribeToClientChanges, stack, options]);
  
  return state;
}
