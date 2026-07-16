import type {
  AuthConfig as CoreAuthConfig,
  ConnectOptions as CoreConnectOptions,
  ProgramSdkDefinition as CoreProgramSdkDefinition,
  Schema,
  WalletAdapter,
} from '@usearete/sdk';

export type {
  ConnectionState,
  ConnectOptions,
  ConnectedArete,
  Subscription,
  Frame,
  EntityFrame,
  SnapshotFrame,
  SnapshotEntity,
  Update,
  RichUpdate,
  StackDefinition,
  ProgramSdkDefinition,
  ProgramAccountReadDefinition,
  ProgramQueryDefinition,
  StackQueryDefinition,
  StackEndpoints,
  ViewDef,
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
 * Note: WebSocket URL is no longer configured here. The URL is:
 * 1. Embedded in the stack definition (`stack.endpoints.ws` / `stack.endpoints.http`)
 * 2. Optionally overridden per-hook via `useArete(stack, { url, httpUrl })`
 */
export interface AreteConfig {
  autoConnect?: boolean;
  wallet?: WalletAdapter;
  reconnectIntervals?: number[];
  maxReconnectAttempts?: number;
  maxEntriesPerView?: number | null;
  flushIntervalMs?: number;
  fetch?: CoreConnectOptions['fetch'];
  validateFrames?: boolean;
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
  /** Attach additional program SDKs to the connected client. */
  programs?: TPrograms;
}

export type UseAreteOptions<TPrograms extends ProgramMap | undefined = undefined> =
  ClientLookupOptions<TPrograms>;

export interface ViewHookOptions<TSchema = unknown> {
  enabled?: boolean;
  initialData?: unknown;
  refreshOnReconnect?: boolean;
  /** Schema to validate entities. Returns undefined if validation fails. */
  schema?: Schema<TSchema>;
  /** Whether to include initial snapshot (defaults to true) */
  withSnapshot?: boolean;
  /** Cursor for resuming from a specific point (_seq value) */
  after?: string;
  /** Maximum number of entities to include in snapshot */
  snapshotLimit?: number;
}

export interface ViewHookResult<T> {
  data: T | undefined;
  isLoading: boolean;
  error?: Error;
  refresh: () => void;
}

export interface ListParamsBase<TSchema = unknown> {
  key?: string;
  where?: Record<string, unknown>;
  limit?: number;
  filters?: Record<string, string>;
  skip?: number;
  /** Schema to validate/filter entities. Only entities passing safeParse will be returned. */
  schema?: Schema<TSchema>;
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

export interface StateViewHook<T> {
  use: (key: { [keyField: string]: string }, options?: ViewHookOptions) => ViewHookResult<T>;
}

export interface ListViewHook<T> {
  use<TSchema = T>(params: ListParamsSingle<TSchema>, options?: ViewHookOptions): ViewHookResult<TSchema | undefined>;
  use<TSchema = T>(params?: ListParamsMultiple<TSchema>, options?: ViewHookOptions): ViewHookResult<TSchema[]>;
  useOne: <TSchema = T>(params?: Omit<ListParamsBase<TSchema>, 'take'>, options?: ViewHookOptions) => ViewHookResult<TSchema | undefined>;
}
