import type { WalletAdapter, Schema } from 'hyperstack-typescript';

export type {
  ConnectionState,
  Subscription,
  Frame,
  EntityFrame,
  SnapshotFrame,
  SnapshotEntity,
  Update,
  RichUpdate,
  StackDefinition,
  ViewDef,
  ViewGroup,
  WalletAdapter,
  Schema,
} from 'hyperstack-typescript';

export { DEFAULT_MAX_ENTRIES_PER_VIEW } from 'hyperstack-typescript';

export type ViewMode = 'state' | 'list';

export interface TransactionDefinition<TParams = unknown> {
  build: (params: TParams) => {
    instruction: string;
    params: TParams;
  };
  refresh?: Array<{ view: string; key?: string | ((params: TParams) => string) }>;
}

export const DEFAULT_FLUSH_INTERVAL_MS = 16;

/**
 * Global configuration for HyperstackProvider.
 * 
 * Note: WebSocket URL is no longer configured here. The URL is:
 * 1. Embedded in the stack definition (stack.url)
 * 2. Optionally overridden per-hook via useHyperstack(stack, { url: '...' })
 */
export interface HyperstackConfig {
  autoConnect?: boolean;
  wallet?: WalletAdapter;
  reconnectIntervals?: number[];
  maxReconnectAttempts?: number;
  maxEntriesPerView?: number | null;
  flushIntervalMs?: number;
  /** Authentication configuration */
  auth?: import('hyperstack-typescript').AuthConfig;
}

/**
 * Options for useHyperstack hook
 */
export interface UseHyperstackOptions {
  /** Override the stack's embedded URL (useful for local development) */
  url?: string;
}

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
