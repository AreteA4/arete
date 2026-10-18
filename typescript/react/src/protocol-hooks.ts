import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import type {
  ProtocolClient,
  ProtocolEventsOptions,
  ProtocolInstanceDetail,
  ProtocolInstanceHandle,
  ProtocolInstanceSnapshot,
  ProtocolKeyClass,
  ProtocolParticipantDetail,
  ProtocolStepAvailabilityFor,
  ProtocolStreamEvent,
  ProtocolTypeMap,
  ProtocolWalletActions,
} from '@usearete/sdk';
import {
  useAsyncRead,
  type UseAsyncReadOptions,
  type UseAsyncReadResult,
} from './hooks/use-async-read';

/** Any protocol client, whatever its key class and members. */
export type AnyProtocolClient<TTypes extends ProtocolTypeMap = ProtocolTypeMap> = ProtocolClient<
  TTypes,
  ProtocolKeyClass,
  unknown,
  unknown
>;

type TypesOfClient<TClient> = TClient extends ProtocolClient<infer TTypes, any, any, any>
  ? TTypes
  : ProtocolTypeMap;

const clientIds = new WeakMap<object, number>();
let nextClientId = 1;

function clientKey(client: object | null | undefined): number | null {
  if (!client) return null;
  const existing = clientIds.get(client);
  if (existing !== undefined) return existing;
  const id = nextClientId++;
  clientIds.set(client, id);
  return id;
}

function abortError(): Error {
  const error = new Error('Protocol read was aborted');
  error.name = 'AbortError';
  return error;
}

export interface UseProtocolInstanceOptions {
  /** Evaluate `instance.allowedSteps` for this wallet. */
  wallet?: string | null;
  enabled?: boolean;
}

export interface UseProtocolInstanceResult<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  /** The canonical instance from the hosted API, refreshed on every stream event. */
  instance: ProtocolInstanceDetail<TTypes> | undefined;
  /** The protocol stack's live row, when the client is a session member with a live stack. */
  live: unknown | undefined;
  error: Error | undefined;
  /** True until the first read resolves or fails. */
  isLoading: boolean;
  refresh: () => Promise<void>;
}

interface InstanceView {
  readonly snapshot: ProtocolInstanceSnapshot<any> | undefined;
  readonly error: Error | undefined;
}

const EMPTY_VIEW: InstanceView = { snapshot: undefined, error: undefined };

/**
 * Follow one protocol instance: the canonical instance from the hosted API,
 * refreshed whenever its event stream reports a change, plus the protocol
 * stack's live row when available. The stream opens while the component is
 * mounted and closes when it unmounts.
 */
export function useProtocolInstance<TClient extends AnyProtocolClient<any>>(
  client: TClient | null | undefined,
  instanceId: string | null | undefined,
  options: UseProtocolInstanceOptions = {}
): UseProtocolInstanceResult<TypesOfClient<TClient>> {
  const enabled = (options.enabled ?? true) && Boolean(client && instanceId);
  const wallet = options.wallet ?? undefined;
  const handle = useMemo<ProtocolInstanceHandle<any> | undefined>(
    () => (enabled && client && instanceId
      ? client.instances.use(instanceId, wallet ? { wallet } : undefined)
      : undefined),
    [client, instanceId, wallet, enabled]
  );
  useEffect(() => () => handle?.close(), [handle]);

  const cache = useRef<{
    handle: ProtocolInstanceHandle<any> | undefined;
    view: InstanceView;
  }>({ handle: undefined, view: EMPTY_VIEW });

  const subscribe = useCallback(
    (listener: () => void) => (handle ? handle.subscribe(listener) : () => undefined),
    [handle]
  );
  const getSnapshot = useCallback((): InstanceView => {
    if (!handle) return EMPTY_VIEW;
    const snapshot = handle.get();
    const error = handle.getError();
    const cached = cache.current;
    if (cached.handle === handle && cached.view.snapshot === snapshot && cached.view.error === error) {
      return cached.view;
    }
    const view = { snapshot, error };
    cache.current = { handle, view };
    return view;
  }, [handle]);
  const view = useSyncExternalStore(subscribe, getSnapshot, () => EMPTY_VIEW);

  const refresh = useCallback(async () => {
    await handle?.refresh();
  }, [handle]);

  return {
    instance: view.snapshot?.instance as ProtocolInstanceDetail<TypesOfClient<TClient>> | undefined,
    live: view.snapshot?.live,
    error: view.error,
    isLoading: Boolean(handle) && view.snapshot === undefined && view.error === undefined,
    refresh,
  };
}

export interface UseProtocolStepsOptions<T> extends UseAsyncReadOptions<T> {
  /** Re-read whenever the instance's event stream reports a change. */
  refreshOnEvents?: boolean;
}

function useRefreshOnEvents(
  client: AnyProtocolClient<any> | null | undefined,
  instanceId: string | null | undefined,
  enabled: boolean,
  refresh: () => Promise<unknown>
): void {
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  useEffect(() => {
    if (!enabled || !client || !instanceId) return undefined;
    const subscription = client.events(instanceId, {
      onEvent: () => {
        void refreshRef.current().catch(() => undefined);
      },
    });
    return () => subscription.close();
  }, [client, instanceId, enabled]);
}

/**
 * Which steps `wallet` can call on `instance` now, and why not (a guard code
 * or an access code).
 */
export function useAllowedSteps<TClient extends AnyProtocolClient<any>>(
  client: TClient | null | undefined,
  input: { instance: string | null | undefined; wallet: string | null | undefined },
  options: UseProtocolStepsOptions<ProtocolStepAvailabilityFor<TypesOfClient<TClient>>[]> = {}
): UseAsyncReadResult<ProtocolStepAvailabilityFor<TypesOfClient<TClient>>[]> {
  const { instance, wallet } = input;
  const enabled = (options.enabled ?? true) && Boolean(client && instance && wallet);
  const result = useAsyncRead(
    ['protocol-allowed-steps', clientKey(client), instance, wallet],
    async ({ signal }) => {
      if (!client || !instance || !wallet) {
        throw new Error('A protocol client, instance and wallet are required for allowed steps');
      }
      if (signal.aborted) throw abortError();
      const steps = await client.allowedSteps({ instance, wallet, signal });
      if (signal.aborted) throw abortError();
      return steps as ProtocolStepAvailabilityFor<TypesOfClient<TClient>>[];
    },
    { enabled, initialData: options.initialData, debounceMs: options.debounceMs }
  );
  useRefreshOnEvents(client, instance, enabled && (options.refreshOnEvents ?? false), result.refresh);
  return result;
}

export interface UseWalletActionsInput {
  /** The wallet to evaluate steps for; the hook stays disabled until it is set. */
  wallet: string | null | undefined;
  /** Only this instance, instead of every instance the wallet relates to. */
  instance?: string | null;
  /** `nextCursor` from the previous page. */
  cursor?: string | null;
  /** Instances per page, at most 50. */
  limit?: number;
}

/**
 * Which steps `wallet` can call now across the deployment: create steps and
 * one page of the instances it relates to, each with its availability,
 * reason and typed arguments. With `instance` set, `refreshOnEvents: true`
 * re-reads on that instance's stream events.
 */
export function useWalletActions<TClient extends AnyProtocolClient<any>>(
  client: TClient | null | undefined,
  input: UseWalletActionsInput,
  options: UseProtocolStepsOptions<ProtocolWalletActions<TypesOfClient<TClient>>> = {}
): UseAsyncReadResult<ProtocolWalletActions<TypesOfClient<TClient>>> {
  const { wallet } = input;
  const instance = input.instance ?? undefined;
  const cursor = input.cursor ?? undefined;
  const limit = input.limit;
  const enabled = (options.enabled ?? true) && Boolean(client && wallet);
  const result = useAsyncRead(
    ['protocol-wallet-actions', clientKey(client), wallet, instance, cursor, limit],
    async ({ signal }) => {
      if (!client || !wallet) {
        throw new Error('A protocol client and wallet are required for wallet actions');
      }
      if (signal.aborted) throw abortError();
      const actions = await client.actions({ wallet, instance, cursor, limit, signal });
      if (signal.aborted) throw abortError();
      return actions as ProtocolWalletActions<TypesOfClient<TClient>>;
    },
    { enabled, initialData: options.initialData, debounceMs: options.debounceMs }
  );
  useRefreshOnEvents(client, instance, enabled && (options.refreshOnEvents ?? false), result.refresh);
  return result;
}

/** One wallet's participant record on an instance. */
export function useProtocolParticipant<TClient extends AnyProtocolClient<any>>(
  client: TClient | null | undefined,
  input: { instance: string | null | undefined; wallet: string | null | undefined },
  options: UseProtocolStepsOptions<ProtocolParticipantDetail<TypesOfClient<TClient>>> = {}
): UseAsyncReadResult<ProtocolParticipantDetail<TypesOfClient<TClient>>> {
  const { instance, wallet } = input;
  const enabled = (options.enabled ?? true) && Boolean(client && instance && wallet);
  const result = useAsyncRead(
    ['protocol-participant', clientKey(client), instance, wallet],
    async ({ signal }) => {
      if (!client || !instance || !wallet) {
        throw new Error('A protocol client, instance and wallet are required for a participant read');
      }
      if (signal.aborted) throw abortError();
      const participant = await client.participants.get({ instance, wallet, signal });
      if (signal.aborted) throw abortError();
      return participant as ProtocolParticipantDetail<TypesOfClient<TClient>>;
    },
    { enabled, initialData: options.initialData, debounceMs: options.debounceMs }
  );
  useRefreshOnEvents(client, instance, enabled && (options.refreshOnEvents ?? false), result.refresh);
  return result;
}

export interface UseProtocolEventsOptions<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  onEvent: (event: ProtocolStreamEvent<TTypes>) => void;
  onError?: (error: Error) => void;
  /** Resume after this event id when the stream first opens. */
  lastEventId?: string;
  enabled?: boolean;
}

/**
 * Follow an instance's event stream while mounted. The latest callbacks are
 * always used; the stream reopens only when the client or instance changes.
 */
export function useProtocolEvents<TClient extends AnyProtocolClient<any>>(
  client: TClient | null | undefined,
  instanceId: string | null | undefined,
  options: UseProtocolEventsOptions<TypesOfClient<TClient>>
): void {
  const optionsRef = useRef(options);
  optionsRef.current = options;
  const enabled = (options.enabled ?? true) && Boolean(client && instanceId);
  useEffect(() => {
    if (!enabled || !client || !instanceId) return undefined;
    const eventOptions: ProtocolEventsOptions<any> = {
      lastEventId: optionsRef.current.lastEventId,
      onEvent: (event) => optionsRef.current.onEvent(event as ProtocolStreamEvent<TypesOfClient<TClient>>),
      onError: (error) => optionsRef.current.onError?.(error),
    };
    const subscription = client.events(instanceId, eventOptions);
    return () => subscription.close();
  }, [client, instanceId, enabled]);
}
