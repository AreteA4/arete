import type {
  AuthConfig as CoreAuthConfig,
  ConnectOptions as CoreConnectOptions,
  ProgramSdkDefinition as CoreProgramSdkDefinition,
  Schema,
  StackDefinition,
  WalletAdapter,
} from '@usearete/sdk';

export type {
  ConnectionState,
  ConnectOptions,
  ConnectedArete,
  Subscription,
  SubscriptionQuery,
  SubscriptionRequest,
  SubscriptionSnapshotOptions,
  QueryLease,
  QuerySnapshot,
  Frame,
  EntityFrame,
  SnapshotFrame,
  SnapshotEntity,
  SubscribedFrame,
  UnsubscribedFrame,
  ErrorFrame,
  Update,
  RichUpdate,
  StackDefinition,
  ProgramSdkDefinition,
  ProgramAccountReadDefinition,
  ProgramQueryDefinition,
  StackQueryDefinition,
  StackEndpoints,
  ViewDef,
  ViewKeyValue,
  DefaultViewKey,
  ViewGroup,
  WalletAdapter,
  Schema,
  ChainClient,
  ProgramsInterface,
  QueriesInterface,
  ProgramInterface,
  MergeProgramMaps,
  StackWithAttachedPrograms,
  PreparedOperation,
  PreparedInstruction,
  PreparedTransaction,
  PreparedFlow,
  OperationReceiptFor,
  OperationExecutionOptions,
  TypedAccountReader,
  TypedQueryExecutor,
  TransactionOptions,
  TransactionFailureOutcome,
  TransactionOutcome,
  NativeBalanceInfo,
  TokenBalanceInfo,
  TokenBalanceInput,
  ContextSlotOptions,
  WaitForProcessedSlotOptions,
  AuthConfig,
  SocketIssue,
} from '@usearete/sdk';

export { DEFAULT_MAX_ENTRIES_PER_VIEW } from '@usearete/sdk';

export type ViewMode = 'state' | 'list';

export interface TransactionDefinition<TParams = unknown> {
  build: (params: TParams) => {
    instruction: string;
    params: TParams;
  };
  refresh?: Array<{ view: string; key?: string | ((params: TParams) => string) }>;
}

export const DEFAULT_FLUSH_INTERVAL_MS = 16;

type ProgramMap = Record<string, CoreProgramSdkDefinition>;

/**
 * Global configuration for AreteProvider.
 * 
 * Note: WebSocket URL is usually embedded in the stack definition
 * (`stack.endpoints.ws` / `stack.endpoints.http`). Override it for the whole
 * app via `stackOptions`, or per-hook via `useArete(stack, { url, httpUrl })`.
 */
export interface AreteConfig {
  /**
   * Default stack for the app. When set, components can call `useArete()`
   * with no arguments; passing a stack explicitly always wins. Register the
   * stack's type once for full inference on argument-less calls:
   *
   * ```ts
   * declare module '@usearete/react' {
   *   interface AreteDefaultStackRegistry { defaultStack: OreStreamStack }
   * }
   * ```
   */
  stack?: StackDefinition;
  /**
   * Default lookup options for the provider stack. They also apply when that
   * same stack is passed explicitly to `useArete(stack)` without options.
   */
  stackOptions?: ClientLookupOptions;
  /**
   * Connect immediately when the client is created (defaults to true).
   * Provider connection settings are applied when a shared client is created;
   * changing them does not mutate an existing client. Call retry() to replace
   * that client with the latest settings. Wallet is the reactive exception.
   */
  autoConnect?: boolean;
  /** Reconnect automatically after an established connection is lost (defaults to true). */
  autoReconnect?: boolean;
  wallet?: WalletAdapter;
  reconnectIntervals?: number[];
  maxReconnectAttempts?: number;
  maxEntriesPerView?: number | null;
  flushIntervalMs?: number;
  fetch?: CoreConnectOptions['fetch'];
  validateFrames?: boolean;
  /** Receives structured details when a generated schema rejects a frame. */
  onFrameValidationError?: CoreConnectOptions['onFrameValidationError'];
  /** Authentication configuration */
  auth?: CoreAuthConfig;
}

/**
 * Client lookup/connect options for React hooks.
 */
export interface ClientLookupOptions<TPrograms extends ProgramMap | undefined = undefined> {
  /** Override the stack's embedded WebSocket URL (useful for local development) */
  url?: string;
  /** Override the stack's embedded HTTP read URL (useful for local development) */
  httpUrl?: string;
  /** Override the stack transport. HTTP mode disables streaming view subscriptions. */
  transport?: CoreConnectOptions<TPrograms>['transport'];
  /** Override the transaction transport independently from the stack HTTP endpoint. */
  transactions?: CoreConnectOptions<TPrograms>['transactions'];
  /** Override the chain transport independently from the stack HTTP endpoint. */
  chain?: CoreConnectOptions<TPrograms>['chain'];
  /** Attach additional program SDKs to the connected client. */
  programs?: TPrograms;
}

export type UseAreteOptions<TPrograms extends ProgramMap | undefined = undefined> =
  ClientLookupOptions<TPrograms>;

export interface ViewSchemaValidationDiagnostic {
  readonly view: string;
  readonly key?: string;
  readonly entity: unknown;
  readonly error: unknown;
}

export interface ViewSchemaFilterWarning {
  readonly view: string;
  readonly key?: string;
  readonly rejectedCount: number;
  readonly diagnostics: readonly ViewSchemaValidationDiagnostic[];
}

export type ViewSchemaValidationErrorCallback = (
  diagnostic: ViewSchemaValidationDiagnostic
) => void;

interface ViewHookSharedOptions<TSchema = unknown> {
  enabled?: boolean;
  /** Schema used to validate and project cached entities. Rejected entities are excluded. */
  schema?: Schema<TSchema>;
  /** Observe entities rejected by the caller-supplied schema and suppress the development warning. */
  onSchemaValidationError?: ViewSchemaValidationErrorCallback;
  /** Whether to include initial snapshot (defaults to true) */
  withSnapshot?: boolean;
  /** Cursor for resuming from a specific point (_seq value) */
  after?: string;
  /** Maximum number of entities to include in snapshot */
  snapshotLimit?: number;
  partition?: string;
  filters?: Record<string, unknown>;
}

export interface StateViewHookOptions<TSchema = unknown>
  extends ViewHookSharedOptions<TSchema> {
  initialData?: TSchema;
}

export interface ListViewHookOptions<TSchema = unknown>
  extends ViewHookSharedOptions<TSchema> {
  initialData?: readonly TSchema[];
}

export interface ListOneViewHookOptions<TSchema = unknown>
  extends ViewHookSharedOptions<TSchema> {
  initialData?: TSchema;
}

/** @deprecated Use StateViewHookOptions, ListViewHookOptions, or ListOneViewHookOptions. */
export type ViewHookOptions<TSchema = unknown> =
  | StateViewHookOptions<TSchema>
  | ListViewHookOptions<TSchema>
  | ListOneViewHookOptions<TSchema>;

/**
 * Lifecycle status of a view hook:
 * - `'disabled'` — no key/params or `enabled: false`; nothing is subscribed.
 * - `'connecting'` — enabled, but the Arete client has not connected yet.
 * - `'subscribing'` — subscribed, waiting for the first snapshot.
 * - `'ready'` — data is usable, from a live snapshot or trusted `initialData`.
 * - `'error'` — the subscription failed; see `error`.
 */
export type ViewStatus = 'disabled' | 'connecting' | 'subscribing' | 'ready' | 'error';

interface ViewHookResultBase<T> {
  data: T | undefined;
  isRefreshing: boolean;
  refresh: () => Promise<void>;
}

type EmptyViewData<T> = T extends readonly unknown[] ? T : undefined;

export type ViewHookResult<T> =
  | (ViewHookResultBase<T> & {
      status: 'disabled';
      isPending: false;
      isReady: false;
      isEmpty: false;
      isLoading: false;
      error: undefined;
    })
  | (ViewHookResultBase<T> & {
      status: 'connecting' | 'subscribing';
      isPending: true;
      isReady: false;
      isEmpty: false;
      isLoading: boolean;
      error: undefined;
    })
  | (ViewHookResultBase<T> & {
      status: 'error';
      isPending: false;
      isReady: false;
      isEmpty: false;
      isLoading: false;
      error: Error;
    })
  | (Omit<ViewHookResultBase<T>, 'data'> & {
      data: T;
      status: 'ready';
      isPending: false;
      isReady: true;
      isEmpty: false;
      isLoading: false;
      error: undefined;
    })
  | (Omit<ViewHookResultBase<T>, 'data'> & {
      data: EmptyViewData<T>;
      status: 'ready';
      isPending: false;
      isReady: true;
      isEmpty: true;
      isLoading: false;
      error: undefined;
    });

export interface ListParamsBase<TSchema = unknown> {
  key?: string;
  partition?: string;
  filters?: Record<string, unknown>;
  skip?: number;
  /** Schema to validate/filter entities. Only entities passing safeParse will be returned. */
  schema?: Schema<TSchema>;
  /** Observe entities rejected by the caller-supplied schema and suppress the development warning. */
  onSchemaValidationError?: ViewSchemaValidationErrorCallback;
  /** Whether to include initial snapshot (defaults to true) */
  withSnapshot?: boolean;
  /** Cursor for resuming from a specific point (_seq value) */
  after?: string;
  /** Maximum number of entities to include in snapshot */
  snapshotLimit?: number;
}

export interface ListParamsSingle<TSchema = unknown> extends ListParamsBase<TSchema> {
  take: 1;
}

export interface ListParamsMultiple<TSchema = unknown> extends ListParamsBase<TSchema> {
  take?: number;
}

export type ListParams<TSchema = unknown> = ListParamsSingle<TSchema> | ListParamsMultiple<TSchema>;

export interface UseMutationReturn {
  submit: (instructionOrTx: unknown | unknown[]) => Promise<string>;
  status: 'idle' | 'pending' | 'success' | 'error';
  error?: string;
  signature?: string;
  reset: () => void;
}

export interface StateViewHook<T, TKey = string> {
  use: (key: TKey | null | undefined, options?: StateViewHookOptions<T>) => ViewHookResult<T>;
  /**
   * Refresh active subscriptions for this view without holding a hook result.
   * Pass a key to refresh one keyed subscription; omit it to refresh every
   * active subscription of the view. No-op when nothing is subscribed, which
   * makes it safe to use in `reconcile: { refresh: [...] }` targets.
   */
  refresh: (key?: TKey | null) => Promise<void>;
}

export interface ListViewHook<T> {
  use<TSchema = T>(params: ListParamsSingle<TSchema>, options?: ListOneViewHookOptions<TSchema>): ViewHookResult<TSchema>;
  use<TSchema = T>(params?: ListParamsMultiple<TSchema>, options?: ListViewHookOptions<TSchema>): ViewHookResult<TSchema[]>;
  useOne: <TSchema = T>(params?: Omit<ListParamsBase<TSchema>, 'take'>, options?: ListOneViewHookOptions<TSchema>) => ViewHookResult<TSchema>;
  /** Refresh every active subscription of this list view. No-op when none. */
  refresh: () => Promise<void>;
}
