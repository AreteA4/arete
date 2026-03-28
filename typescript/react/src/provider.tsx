import React, { createContext, useContext, useEffect, useRef, ReactNode, useSyncExternalStore, useCallback } from 'react';
import { HyperStack, type ConnectionState, type StackDefinition } from 'hyperstack-typescript';
import type { HyperstackConfig } from './types';
import { DEFAULT_FLUSH_INTERVAL_MS } from './types';
import { ZustandAdapter } from './zustand-adapter';

interface ClientEntry {
  client: HyperStack<any>;
  disconnect: () => void;
}

interface HyperstackContextValue {
  getOrCreateClient: <TStack extends StackDefinition>(stack: TStack, urlOverride?: string) => Promise<HyperStack<TStack>>;
  getClient: <TStack extends StackDefinition>(stack: TStack | undefined) => HyperStack<TStack> | null;
  subscribeToClientChanges: (callback: () => void) => () => void;
  config: HyperstackConfig;
}

const HyperstackContext = createContext<HyperstackContextValue | null>(null);

export function HyperstackProvider({
  children,
  fallback = null,
  ...config
}: HyperstackConfig & {
  children: ReactNode;
  fallback?: ReactNode;
}) {
  const clientsRef = useRef<Map<string, ClientEntry>>(new Map());
  const connectingRef = useRef<Map<string, Promise<HyperStack<any>>>>(new Map());
  const clientChangeListenersRef = useRef<Set<() => void>>(new Set());
  
  const notifyClientChange = useCallback(() => {
    clientChangeListenersRef.current.forEach(cb => { cb(); });
  }, []);
  
  const subscribeToClientChanges = useCallback((callback: () => void) => {
    clientChangeListenersRef.current.add(callback);
    return () => {
      clientChangeListenersRef.current.delete(callback);
    };
  }, []);

  const getOrCreateClient = useCallback(async <TStack extends StackDefinition>(stack: TStack, urlOverride?: string): Promise<HyperStack<TStack>> => {
    const cacheKey = urlOverride ? `${stack.name}:${urlOverride}` : stack.name;
    
    const existing = clientsRef.current.get(cacheKey);
    if (existing) {
      return existing.client as HyperStack<TStack>;
    }

    const connecting = connectingRef.current.get(cacheKey);
    if (connecting) {
      return connecting as Promise<HyperStack<TStack>>;
    }

    const adapter = new ZustandAdapter();
    const connectionPromise = HyperStack.connect(stack, {
      url: urlOverride,
      storage: adapter,
      autoReconnect: config.autoConnect,
      reconnectIntervals: config.reconnectIntervals,
      maxReconnectAttempts: config.maxReconnectAttempts,
      maxEntriesPerView: config.maxEntriesPerView,
      flushIntervalMs: config.flushIntervalMs ?? DEFAULT_FLUSH_INTERVAL_MS,
      auth: config.auth,
    }).then((client) => {
      client.onConnectionStateChange((state, error) => {
        adapter.setConnectionState(state, error);
      });
      adapter.setConnectionState(client.connectionState);

      clientsRef.current.set(cacheKey, {
        client,
        disconnect: () => client.disconnect()
      });
      connectingRef.current.delete(cacheKey);
      notifyClientChange();
      return client;
    });

    connectingRef.current.set(cacheKey, connectionPromise);
    return connectionPromise as Promise<HyperStack<TStack>>;
  }, [config.autoConnect, config.reconnectIntervals, config.maxReconnectAttempts, config.maxEntriesPerView, config.flushIntervalMs, config.auth, notifyClientChange]);

  const getClient = useCallback(<TStack extends StackDefinition>(stack: TStack | undefined): HyperStack<TStack> | null => {
    if (!stack) {
      if (clientsRef.current.size === 1) {
        const firstEntry = clientsRef.current.values().next().value;
        return firstEntry ? (firstEntry.client as HyperStack<TStack>) : null;
      }
      return null;
    }
    const entry = clientsRef.current.get(stack.name);
    return entry ? (entry.client as HyperStack<TStack>) : null;
  }, []);

  useEffect(() => {
    return () => {
      clientsRef.current.forEach((entry) => {
        entry.disconnect();
      });
      clientsRef.current.clear();
      connectingRef.current.clear();
    };
  }, []);

  const value: HyperstackContextValue = {
    getOrCreateClient,
    getClient,
    subscribeToClientChanges,
    config,
  };

  return (
    <HyperstackContext.Provider value={value}>
      {children}
    </HyperstackContext.Provider>
  );
}

export function useHyperstackContext() {
  const context = useContext(HyperstackContext);
  if (!context) {
    throw new Error('useHyperstackContext must be used within HyperstackProvider');
  }
  return context;
}

export function useConnectionState(stack?: StackDefinition): ConnectionState {
  const { getClient, subscribeToClientChanges } = useHyperstackContext();
  const [state, setState] = React.useState<ConnectionState>(() => {
    const client = getClient(stack);
    return client?.connectionState ?? 'disconnected';
  });
  const unsubscribeRef = React.useRef<(() => void) | undefined>(undefined);
  
  React.useEffect(() => {
    let mounted = true;
    
    const setupClientSubscription = () => {
      unsubscribeRef.current?.();
      unsubscribeRef.current = undefined;
      
      const client = getClient(stack);
      if (client && mounted) {
        setState(client.connectionState);
        unsubscribeRef.current = client.onConnectionStateChange((newState) => {
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
  }, [getClient, subscribeToClientChanges, stack]);
  
  return state;
}

export function useView<T>(stack: StackDefinition, viewPath: string): T[] {
  const { getClient } = useHyperstackContext();
  const client = getClient(stack);
  
  return useSyncExternalStore(
    (callback) => {
      if (!client) return () => {};
      return client.store.onUpdate(callback);
    },
    () => {
      if (!client) return [];
      const data = client.store.getAll(viewPath);
      return data as T[];
    }
  );
}

export function useEntity<T>(stack: StackDefinition, viewPath: string, key: string): T | null {
  const { getClient } = useHyperstackContext();
  const client = getClient(stack);
  
  return useSyncExternalStore(
    (callback) => {
      if (!client) return () => {};
      return client.store.onUpdate(callback);
    },
    () => {
      if (!client) return null;
      const data = client.store.get(viewPath, key);
      return data as T | null;
    }
  );
}