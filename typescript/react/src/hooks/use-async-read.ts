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
}

export interface UseAsyncReadResult<T> {
  data: T | undefined;
  error: Error | null;
  isLoading: boolean;
  isRefreshing: boolean;
  refresh: () => Promise<T | undefined>;
}

interface AsyncReadState<T> {
  data: T | undefined;
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
 */
export function useAsyncRead<T>(
  key: AsyncReadKey,
  read: (context: AsyncReadContext) => Promise<T>,
  options: UseAsyncReadOptions<T> = {}
): UseAsyncReadResult<T> {
  const stableKey = stableKeyPart(key);
  const enabled = options.enabled ?? true;
  const [state, setState] = useState<AsyncReadState<T>>(() => ({
    data: options.initialData,
    error: null,
    isLoading: enabled && options.initialData === undefined,
    isRefreshing: false,
  }));
  const readRef = useRef(read);
  const keyRef = useRef(key);
  const enabledRef = useRef(enabled);
  const initialDataRef = useRef(options.initialData);
  const dataRef = useRef<T | undefined>(options.initialData);
  const requestRef = useRef<{ id: number; controller: AbortController | null }>({
    id: 0,
    controller: null,
  });
  const mountedRef = useRef(true);

  readRef.current = read;
  keyRef.current = key;
  enabledRef.current = enabled;
  initialDataRef.current = options.initialData;
  dataRef.current = state.data;

  const cancelCurrent = useCallback(() => {
    requestRef.current.id += 1;
    requestRef.current.controller?.abort();
    requestRef.current.controller = null;
  }, []);

  const runRead = useCallback(async (): Promise<T | undefined> => {
    if (!enabledRef.current) {
      return dataRef.current;
    }

    const id = requestRef.current.id + 1;
    requestRef.current.id = id;
    requestRef.current.controller?.abort();
    const controller = new AbortController();
    requestRef.current.controller = controller;
    const hasData = dataRef.current !== undefined;
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
        setState({
          data,
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
    setState({
      data: initialDataRef.current,
      error: null,
      isLoading: enabledRef.current && initialDataRef.current === undefined,
      isRefreshing: false,
    });
    if (enabledRef.current) {
      void runRead().catch(() => undefined);
    }
    return cancelCurrent;
  }, [cancelCurrent, runRead, stableKey, enabled]);

  return {
    ...state,
    refresh: runRead,
  };
}
