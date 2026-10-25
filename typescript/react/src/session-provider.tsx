import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import {
  createSession,
  type CompositionSessionOptions,
  type Session,
  type SessionDefinition,
  type SessionOptions,
  type WalletAdapter,
} from '@usearete/sdk';

import { useOptionalAreteContext } from './provider';

/** Any session from `createSession`. */
export type AnySession = Session<any, any, any>;

/**
 * The session type for `T`: a session type as it is, or the session
 * `createSession(definition)` opens for a definition type.
 */
export type SessionOf<T> = T extends AnySession
  ? T
  : T extends SessionDefinition
    ? Session<T>
    : never;

/**
 * Options for the session the provider opens: `createSession`'s options
 * without `wallet` (the provider's `wallet` prop sets it, without reopening).
 */
export type AreteSessionOptions<TDef extends SessionDefinition = SessionDefinition> = Omit<
  TDef extends { readonly mode: 'composition' }
    ? CompositionSessionOptions<TDef>
    : SessionOptions<TDef>,
  'wallet'
>;

/**
 * - `opening`: the session is being opened (or, for a provider given a
 *   `session`, none has been given yet).
 * - `open`: `session` is ready.
 * - `error`: opening failed; `error` says why and `retry()` tries again.
 */
export type AreteSessionStatus = 'opening' | 'open' | 'error';

export interface UseAreteSessionResult<TSession extends AnySession = AnySession> {
  /** The open session; `null` while it opens, after a failure, and while it is reopened. */
  readonly session: TSession | null;
  readonly status: AreteSessionStatus;
  /** Why the session could not be opened. */
  readonly error: Error | null;
  /**
   * The session once it is open. Waits through a reopen (new auth or
   * options) and resolves with the new session; rejects when opening fails.
   * Stable, so it can be a `useOperationMutation` executor:
   * `{ executor: ready }`.
   */
  ready(): Promise<TSession>;
  /** Open the session again after a failure. Only for sessions the provider opens. */
  retry(): void;
}

interface SessionProviderBaseProps {
  children: ReactNode;
  /**
   * The wallet the session signs with (`session.wallet`). Changing it calls
   * `session.setWallet`; the session is not reopened.
   */
  wallet?: WalletAdapter;
}

/** The provider opens (and closes) the session. */
export interface OwnedAreteSessionProviderProps<TDef extends SessionDefinition>
  extends SessionProviderBaseProps {
  /** What to open, as for `createSession(definition, options)`. */
  definition: TDef;
  /**
   * `createSession` options. `auth` and `fetch` default to the enclosing
   * `AreteProvider`'s. A change reopens the session (see the README).
   */
  options?: AreteSessionOptions<TDef>;
  session?: never;
}

/** The app opened the session; the provider shares it and never closes it. */
export interface ExternalAreteSessionProviderProps extends SessionProviderBaseProps {
  /** The session to share; `null`/`undefined` while the app is still opening it. */
  session: AnySession | null | undefined;
  definition?: never;
  options?: never;
}

export type AreteSessionProviderProps<TDef extends SessionDefinition = SessionDefinition> =
  | OwnedAreteSessionProviderProps<TDef>
  | ExternalAreteSessionProviderProps;

const AreteSessionContext = createContext<UseAreteSessionResult | null>(null);

interface Deferred {
  readonly promise: Promise<AnySession>;
  settled: boolean;
  resolve(value: AnySession | Promise<AnySession>): void;
  reject(error: unknown): void;
}

function createDeferred(): Deferred {
  let resolvePromise!: (value: AnySession | Promise<AnySession>) => void;
  let rejectPromise!: (error: unknown) => void;
  const promise = new Promise<AnySession>((resolve, reject) => {
    resolvePromise = resolve;
    rejectPromise = reject;
  });
  // A failure nobody awaited is reported through `status`, not as an
  // unhandled rejection.
  promise.catch(() => undefined);
  const deferred: Deferred = {
    promise,
    settled: false,
    resolve(value) {
      if (deferred.settled) return;
      deferred.settled = true;
      resolvePromise(value);
    },
    reject(error) {
      if (deferred.settled) return;
      deferred.settled = true;
      rejectPromise(error);
    },
  };
  return deferred;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  if (typeof value !== 'object' || value === null) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

/**
 * Equal by identity, or plain objects with the same keys whose values are
 * equal to `depth` levels. Arrays, functions and class instances compare by
 * identity.
 */
function sameShape(left: unknown, right: unknown, depth: number): boolean {
  if (Object.is(left, right)) return true;
  if (depth === 0 || !isPlainObject(left) || !isPlainObject(right)) return false;
  const leftKeys = Object.keys(left).filter((key) => left[key] !== undefined);
  const rightKeys = Object.keys(right).filter((key) => right[key] !== undefined);
  return leftKeys.length === rightKeys.length
    && leftKeys.every((key) =>
      Object.prototype.hasOwnProperty.call(right, key) && sameShape(left[key], right[key], depth - 1)
    );
}

/**
 * `value`, or the previous one while it has the same shape: two levels of
 * plain objects, so `options={{ apiUrl, auth: { tokenKind: 'api', … } }}`
 * written inline does not reopen the session on every render.
 */
function useSameShape<T>(value: T): T {
  const ref = useRef(value);
  if (!sameShape(ref.current, value, 2)) ref.current = value;
  return ref.current;
}

function toError(value: unknown): Error {
  return value instanceof Error ? value : new Error(String(value));
}

const openSession = createSession as unknown as (
  definition: SessionDefinition,
  options?: Record<string, unknown>
) => Promise<AnySession>;

interface SessionKey {
  readonly definition: SessionDefinition | undefined;
  readonly options: Record<string, unknown>;
  readonly external: AnySession | null;
  readonly attempt: number;
}

interface OpenedSession {
  readonly key: SessionKey;
  readonly session: AnySession | null;
  readonly error: Error | null;
}

/**
 * One Arete session (`createSession`) for everything below it, read with
 * {@link useAreteSession}.
 *
 * ```tsx
 * const SQUADS = { programs: { squads: SQUADS_V4_PROGRAM } };
 * const auth = { tokenKind: 'api', tokenEndpoint: '/api/arete-token' } as const;
 *
 * <AreteSessionProvider definition={SQUADS} options={{ auth }} wallet={wallet}>
 *   <App />
 * </AreteSessionProvider>;
 *
 * // below it
 * const { session } = useAreteSession<typeof SQUADS>();
 * session?.programs.squads; // typed
 * ```
 *
 * Given `definition`, the provider opens the session when it mounts and
 * closes it when it unmounts. `options.auth` and `options.fetch` default to
 * the enclosing `AreteProvider`'s. When the definition or the options
 * change, the old session is closed at once and a new one is opened:
 * `session` is `null` until it is ready, so nothing runs against a session
 * opened with credentials that no longer apply. Options are compared by
 * shape two levels deep (an inline `auth` object with the same fields is the
 * same auth); functions such as `getToken` or `fetch` compare by identity,
 * so keep them stable. The wallet never reopens the session: changing
 * `wallet` calls `session.setWallet`.
 *
 * Given `session`, the provider only shares it: the app opened it and closes
 * it. `wallet`, when passed, is set on it the same way.
 */
export function AreteSessionProvider<TDef extends SessionDefinition>(
  props: AreteSessionProviderProps<TDef>
): React.ReactElement {
  const arete = useOptionalAreteContext();
  const isExternal = props.definition === undefined;
  const externalSession = isExternal ? props.session ?? null : null;
  const definition = useSameShape(props.definition as SessionDefinition | undefined);

  const given = (props.options ?? {}) as Record<string, unknown>;
  const inheritedAuth = arete?.config.auth;
  const inheritedFetch = arete?.config.fetch;
  const options = useSameShape<Record<string, unknown>>({
    ...given,
    ...(given.auth === undefined && inheritedAuth ? { auth: inheritedAuth } : {}),
    ...(given.fetch === undefined && inheritedFetch ? { fetch: inheritedFetch } : {}),
  });
  const [attempt, setAttempt] = useState(0);

  const key = useMemo<SessionKey>(
    () => ({ definition, options, external: externalSession, attempt }),
    [definition, options, externalSession, attempt]
  );
  const keyRef = useRef(key);
  keyRef.current = key;

  const walletRef = useRef(props.wallet);
  walletRef.current = props.wallet;
  const syncsWallet = !isExternal || 'wallet' in props;

  const deferredsRef = useRef(new WeakMap<SessionKey, Deferred>());
  const deferredFor = useCallback((forKey: SessionKey): Deferred => {
    let deferred = deferredsRef.current.get(forKey);
    if (!deferred) {
      deferred = createDeferred();
      deferredsRef.current.set(forKey, deferred);
    }
    return deferred;
  }, []);

  const [opened, setOpened] = useState<OpenedSession | null>(null);

  // A caller waiting on the previous key's session gets the next one.
  const previousKeyRef = useRef<SessionKey | null>(null);
  useEffect(() => {
    const previous = previousKeyRef.current;
    previousKeyRef.current = key;
    if (previous && previous !== key) {
      const waiting = deferredsRef.current.get(previous);
      if (waiting && !waiting.settled) waiting.resolve(deferredFor(key).promise);
    }
  }, [key, deferredFor]);

  useEffect(() => {
    if (key.external) deferredFor(key).resolve(key.external);
  }, [key, deferredFor]);

  useEffect(() => {
    if (!key.definition) return undefined;
    let active = true;
    let session: AnySession | null = null;
    const deferred = deferredFor(key);
    const wallet = walletRef.current;
    openSession(key.definition, { ...key.options, ...(wallet ? { wallet } : {}) }).then(
      (openedSession) => {
        if (!active) {
          openedSession.close();
          return;
        }
        session = openedSession;
        // The wallet may have changed while the session was opening.
        if (openedSession.wallet !== walletRef.current) openedSession.setWallet(walletRef.current);
        deferred.resolve(openedSession);
        setOpened({ key, session: openedSession, error: null });
      },
      (cause: unknown) => {
        if (!active) return;
        const error = toError(cause);
        deferred.reject(error);
        setOpened({ key, session: null, error });
      }
    );
    return () => {
      active = false;
      session?.close();
    };
  }, [key, deferredFor]);

  let session: AnySession | null = null;
  let error: Error | null = null;
  let status: AreteSessionStatus = 'opening';
  if (isExternal) {
    session = externalSession;
    status = session ? 'open' : 'opening';
  } else if (opened?.key === key) {
    session = opened.session;
    error = opened.error;
    status = error ? 'error' : 'open';
  }

  const { wallet } = props;
  useEffect(() => {
    if (session && syncsWallet && session.wallet !== wallet) session.setWallet(wallet);
  }, [session, syncsWallet, wallet]);

  const ready = useCallback(() => deferredFor(keyRef.current).promise, [deferredFor]);
  const retry = useCallback(() => {
    setAttempt((current) => current + 1);
  }, []);

  const value = useMemo<UseAreteSessionResult>(
    () => ({ session, status, error, ready, retry }),
    [session, status, error, ready, retry]
  );

  return (
    <AreteSessionContext.Provider value={value}>
      {props.children}
    </AreteSessionContext.Provider>
  );
}

/**
 * The nearest {@link AreteSessionProvider}'s session. Pass the definition
 * (or the session type) for typed members:
 *
 * ```tsx
 * const { session, status, ready } = useAreteSession<typeof SQUADS>();
 * const squads = session?.programs.squads;
 * ```
 *
 * Throws outside an `AreteSessionProvider`.
 */
export function useAreteSession<
  T extends SessionDefinition | AnySession = AnySession,
>(): UseAreteSessionResult<SessionOf<T>> {
  const value = useContext(AreteSessionContext);
  if (!value) {
    throw new Error('useAreteSession must be used within AreteSessionProvider');
  }
  return value as unknown as UseAreteSessionResult<SessionOf<T>>;
}

/** The nearest `AreteSessionProvider`'s value, or `null` outside one. */
export function useOptionalAreteSession(): UseAreteSessionResult | null {
  return useContext(AreteSessionContext);
}
