import { useCallback, useEffect, useRef, useState } from 'react';

export type AsyncReadKey =
  | string
  | number
  | bigint
  | boolean
  | null
  | undefined
  | readonly AsyncReadKey[]
  | { readonly [key: string]: AsyncReadKey };

export interface AsyncReadContext {
  readonly signal: AbortSignal;
  readonly key: AsyncReadKey;
}

export interface UseAsyncReadOptions<T> {
  enabled?: boolean;
  initialData?: T;
  debounceMs?: number;
  /** Status to expose when the read is disabled by an unavailable dependency. */
  disabledStatus?: Extract<AsyncReadStatus, 'disabled' | 'connecting'>;
}

export type AsyncReadStatus =
  | 'disabled'
  | 'connecting'
  | 'loading'
  | 'ready'
  | 'refreshing'
  | 'error';

export interface UseAsyncReadResult<T> {
  data: T | undefined;
  error: Error | null;
  status: AsyncReadStatus;
  /** True while the read is connecting or loading its first result. */
  isPending: boolean;
  /** True after the read has completed once, including during a refresh. */
  isReady: boolean;
  /** True when a ready read resolved to `null` or `undefined`. */
  isEmpty: boolean;
  isLoading: boolean;
  isRefreshing: boolean;
  refresh: () => Promise<T | undefined>;
}

interface AsyncReadState<T> {
  /** Stable serialization of the key that produced this state. */
  key: string;
  enabled: boolean;
  data: T | undefined;
  hasResolved: boolean;
  error: Error | null;
  isLoading: boolean;
  isRefreshing: boolean;
}

function stableKeyPart(value: AsyncReadKey): string {
  if (value === undefined) return 'undefined';
  if (value === null) return 'null';
  if (typeof value === 'bigint') return `bigint:${value}`;
  if (typeof value === 'string') return `string:${JSON.stringify(value)}`;
  if (typeof value === 'number') return `number:${String(value)}`;
  if (typeof value === 'boolean') return `boolean:${String(value)}`;
  if (Array.isArray(value)) {
    return `[${value.map(stableKeyPart).join(',')}]`;
  }
  const record = value as { readonly [key: string]: AsyncReadKey };
  return `{${Object.keys(record).sort().map((key) =>
    `${JSON.stringify(key)}:${stableKeyPart(record[key])}`
  ).join(',')}}`;
}

function normalizeReadError(value: unknown): Error {
  if (value instanceof Error) {
    return value;
  }
  const error = new Error(typeof value === 'string' ? value : String(value));
  (error as Error & { cause?: unknown }).cause = value;
  return error;
}

/**
 * Minimal keyed async state with no cache, retry, or request deduplication.
 * A newer request aborts and supersedes the previous request for this hook.
 *
 * The returned state is tagged with the key that produced it: when the key
 * changes, the hook immediately reports the fresh key's initial state instead
 * of briefly exposing data fetched for the previous key. Callers never need
 * to re-verify that `data` matches the arguments they passed.
 */
export function useAsyncRead<T>(
  key: AsyncReadKey,
  read: (context: AsyncReadContext) => Promise<T>,
  options: UseAsyncReadOptions<T> = {}
): UseAsyncReadResult<T> {
  const stableKey = stableKeyPart(key);
  const enabled = options.enabled ?? true;
  const [state, setState] = useState<AsyncReadState<T>>(() => ({
    key: stableKey,
    enabled,
    data: options.initialData,
    hasResolved: options.initialData !== undefined,
    error: null,
    isLoading: enabled && options.initialData === undefined,
    isRefreshing: false,
  }));
  const readRef = useRef(read);
  const keyRef = useRef(key);
  const enabledRef = useRef(enabled);
  const initialDataRef = useRef(options.initialData);
  const dataRef = useRef<T | undefined>(options.initialData);
  const hasResolvedRef = useRef(options.initialData !== undefined);
  const requestRef = useRef<{ id: number; controller: AbortController | null }>({
    id: 0,
    controller: null,
  });
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mountedRef = useRef(true);

  readRef.current = read;
  keyRef.current = key;
  enabledRef.current = enabled;
  initialDataRef.current = options.initialData;
  dataRef.current = state.data;

  const cancelCurrent = useCallback(() => {
    if (debounceTimerRef.current !== null) {
      clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
    requestRef.current.id += 1;
    requestRef.current.controller?.abort();
    requestRef.current.controller = null;
  }, []);

  const runRead = useCallback(async (): Promise<T | undefined> => {
    if (debounceTimerRef.current !== null) {
      clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
    if (!enabledRef.current) {
      return dataRef.current;
    }

    const id = requestRef.current.id + 1;
    requestRef.current.id = id;
    requestRef.current.controller?.abort();
    const controller = new AbortController();
    requestRef.current.controller = controller;
    const hasData = hasResolvedRef.current;
    if (mountedRef.current) {
      setState((current) => ({
        ...current,
        error: null,
        isLoading: !hasData,
        isRefreshing: hasData,
      }));
    }

    try {
      const data = await readRef.current({
        signal: controller.signal,
        key: keyRef.current,
      });
      if (mountedRef.current && requestRef.current.id === id) {
        dataRef.current = data;
        hasResolvedRef.current = true;
        setState({
          key: stableKeyPart(keyRef.current),
          enabled: true,
          data,
          hasResolved: true,
          error: null,
          isLoading: false,
          isRefreshing: false,
        });
      }
      return data;
    } catch (value) {
      const error = normalizeReadError(value);
      if (
        mountedRef.current
        && requestRef.current.id === id
        && !controller.signal.aborted
      ) {
        setState((current) => ({
          ...current,
          error,
          isLoading: false,
          isRefreshing: false,
        }));
      }
      throw error;
    } finally {
      if (requestRef.current.id === id) {
        requestRef.current.controller = null;
      }
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, [cancelCurrent]);

  useEffect(() => {
    cancelCurrent();
    dataRef.current = initialDataRef.current;
    hasResolvedRef.current = initialDataRef.current !== undefined;
    setState({
      key: stableKey,
      enabled: enabledRef.current,
      data: initialDataRef.current,
      hasResolved: initialDataRef.current !== undefined,
      error: null,
      isLoading: enabledRef.current && initialDataRef.current === undefined,
      isRefreshing: false,
    });
    if (enabledRef.current) {
      const debounceMs = options.debounceMs ?? 0;
      if (debounceMs > 0) {
        debounceTimerRef.current = setTimeout(() => {
          debounceTimerRef.current = null;
          void runRead().catch(() => undefined);
        }, debounceMs);
      } else {
        void runRead().catch(() => undefined);
      }
    }
    return cancelCurrent;
  }, [cancelCurrent, runRead, stableKey, enabled, options.debounceMs]);

  // State committed under a different key must not leak into this render:
  // report the fresh key's initial state until the effect above commits.
  const visible: AsyncReadState<T> = state.key === stableKey && state.enabled === enabled
    ? state
    : {
        key: stableKey,
        enabled,
        data: options.initialData,
        hasResolved: options.initialData !== undefined,
        error: null,
        isLoading: enabled && options.initialData === undefined,
        isRefreshing: false,
      };

  const status: AsyncReadStatus = !enabled
    ? options.disabledStatus ?? 'disabled'
    : visible.error
      ? 'error'
      : visible.isLoading
        ? 'loading'
        : visible.isRefreshing
          ? 'refreshing'
          : 'ready';
  return {
    data: visible.data,
    error: visible.error,
    status,
    isPending: status === 'connecting' || status === 'loading',
    isReady: status === 'ready' || status === 'refreshing',
    isEmpty: visible.hasResolved
      && (status === 'ready' || status === 'refreshing')
      && visible.data == null,
    isLoading: visible.isLoading,
    isRefreshing: visible.isRefreshing,
    refresh: runRead,
  };
}
