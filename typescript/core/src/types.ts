export type ConnectionState =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'error';

export type Update<T> =
  | { type: 'upsert'; key: string; data: T }
  | { type: 'patch'; key: string; data: Partial<T> }
  | { type: 'remove'; key: string }
  | { type: 'delete'; key: string };

export type RichUpdate<T> =
  | { type: 'created'; key: string; data: T }
  | { type: 'updated'; key: string; before: T; after: T; patch?: unknown }
  | { type: 'removed'; key: string; lastKnown?: T }
  | { type: 'deleted'; key: string; lastKnown?: T };

export type ViewKeyValue = string | number | bigint;

export type ViewKeyFields<TKey> = unknown extends TKey
  ? readonly string[]
  : TKey extends object
    ? readonly Extract<keyof TKey, string>[]
    : readonly string[];

export interface ViewDef<T, TMode extends 'state' | 'list', TKey = unknown> {
  readonly mode: TMode;
  readonly view: string;
  readonly keyFields?: ViewKeyFields<TKey>;
  readonly _entity?: T;
  readonly _key?: TKey;
}

export interface StackEndpoints {
  readonly ws: string;
  readonly http?: string;
}

export type ReadTransportMethod = 'GET' | 'POST';

export interface ProgramAccountReadDefinition<T> {
  readonly account: string;
  readonly schema?: Schema<T>;
  readonly _result?: T;
}

export type ProgramAccountBatchItem<T> =
  | { readonly address: string; readonly status: 'ok'; readonly value: T }
  | { readonly address: string; readonly status: 'missing' }
  | {
      readonly address: string;
      readonly status: 'error';
      readonly error: { readonly code: string };
    };

export interface ProgramAccountBatchResult<T> {
  readonly items: readonly ProgramAccountBatchItem<T>[];
}

export interface ProgramQueryDefinition<TParams = unknown, TResult = unknown> {
  readonly name: string;
  readonly path: string;
  readonly method?: ReadTransportMethod;
  readonly schema?: Schema<TResult>;
  readonly _params?: TParams;
  readonly _result?: TResult;
}

export interface StackQueryDefinition<TParams = unknown, TResult = unknown> {
  readonly name: string;
  readonly path: string;
  readonly method?: ReadTransportMethod;
  readonly schema?: Schema<TResult>;
  readonly _params?: TParams;
  readonly _result?: TResult;
}

export interface ProgramSdkDefinition {
  readonly name: string;
  readonly programId?: string;
  /** Typed identity of generated program content. V2 excludes compiler provenance. */
  readonly sdkDefinitionHash?: string;
  readonly programSpecHash?: string;
  readonly idlContentHash?: string;
  readonly normalizedIdlHash?: string;
  readonly schemas?: Record<string, Schema<unknown>>;
  readonly pdas?: Record<string, unknown>;
  readonly accounts?: Record<string, ProgramAccountReadDefinition<unknown>>;
  readonly queries?: Record<string, ProgramQueryDefinition<unknown, unknown>>;
  readonly rawInstructions?: Record<string, import('./instructions').InstructionHandler<any, any>>;
  readonly addresses?: Record<string, unknown>;
  readonly constants?: unknown;
  readonly defaults?: unknown;
  readonly math?: unknown;
  /** Managed-hosting transports. Absent for local/self-hosted generation. */
  readonly gateway?: HostedSolanaGatewayBindings;
}

export interface ProgramReleaseReference {
  readonly programReleaseHash: string;
  readonly programSpecHash: string;
}

/** Public, non-secret metadata describing how an HTTP bearer token is acquired. */
export interface HttpAuthMetadata {
  readonly required?: boolean;
  readonly mode?: string;
  readonly sessionEndpoint: string;
  readonly jwksUrl?: string;
  readonly tokenTransport?: string;
  readonly audience?: string;
  readonly targetKind: 'program-read-binding';
  readonly targetId: string;
  readonly scopes?: readonly string[];
  readonly acceptedKeyClasses?: readonly string[];
}

export interface ProgramReadBinding {
  readonly endpoint: string;
  readonly programReadBindingId: string;
  readonly auth: HttpAuthMetadata;
}

export type SolanaGatewayAuthScope =
  | 'read'
  | 'transaction:inspect'
  | 'transaction:send';

/** Complete public auth metadata emitted for a hosted Solana gateway binding. */
export interface SolanaGatewayAuthMetadata {
  readonly required: boolean;
  readonly mode: string;
  readonly sessionEndpoint: string;
  readonly jwksUrl: string;
  readonly tokenTransport: string;
  readonly audience: 'arete:solana-gateway';
  readonly targetKind: 'solana-gateway-binding';
  readonly targetId: string;
  readonly scopes: readonly SolanaGatewayAuthScope[];
  readonly acceptedKeyClasses: readonly string[];
  readonly transactionEntitlementRequired: boolean;
}

/** One generated, non-inheriting hosted Solana gateway capability binding. */
export interface HostedSolanaGatewayCapabilityBinding {
  readonly endpoint: string;
  readonly authPolicy: string;
  readonly solanaGatewayBindingId: string;
  readonly cluster: string;
  readonly region: string;
  readonly auth: SolanaGatewayAuthMetadata;
}

export interface HostedSolanaGatewayBindings {
  readonly chain: HostedSolanaGatewayCapabilityBinding;
  readonly transactions: HostedSolanaGatewayCapabilityBinding;
}

/** Generated release identity with one explicit, non-inheriting read transport. */
export type ProgramReadDescriptor =
  | {
      readonly release: ProgramReleaseReference;
      readonly transport: {
        readonly kind: 'local-http';
        readonly endpointSource: 'connect-http-url';
      };
    }
  | {
      readonly release: ProgramReleaseReference;
      readonly transport: {
        readonly kind: 'hosted-binding';
        readonly binding: ProgramReadBinding;
      };
    };

/** Runtime overrides replace the complete generated descriptor. */
export type ProgramReadOverride = ProgramReadDescriptor;

export type ProgramReadDescriptors<
  TPrograms extends Record<string, ProgramSdkDefinition> | undefined,
> = TPrograms extends Record<string, ProgramSdkDefinition>
  ? { readonly [K in keyof TPrograms]: ProgramReadDescriptor }
  : Record<string, never>;

export type ProgramReadOverrides<
  TPrograms extends Record<string, ProgramSdkDefinition> | undefined,
> = TPrograms extends Record<string, ProgramSdkDefinition>
  ? { readonly [K in keyof TPrograms]?: ProgramReadOverride }
  : Record<string, never>;

export interface StackDefinition<
  TPrograms extends Record<string, ProgramSdkDefinition> = Record<string, ProgramSdkDefinition>,
> {
  readonly name: string;
  readonly endpoints: StackEndpoints;
  readonly views: Record<string, ViewGroup>;
  readonly schemas?: Record<string, Schema<unknown>>;
  readonly patchSchemas?: Record<string, Schema<unknown>>;
  readonly queries?: Record<string, StackQueryDefinition<unknown, unknown>>;
  readonly programs?: TPrograms;
  /** Release and transport metadata keyed in parallel with `programs`. */
  readonly programReads?: ProgramReadDescriptors<TPrograms>;
  /** Managed-hosting transports. Absent for local/self-hosted generation. */
  readonly gateway?: HostedSolanaGatewayBindings;
}

export interface ViewGroup {
  state?: ViewDef<any, 'state', any>;
  list?: ViewDef<any, 'list'>;
}

export interface SubscriptionQuery {
  view: string;
  key?: string;
  partition?: string;
  filters?: Record<string, unknown>;
  take?: number;
  skip?: number;
  after?: string;
  snapshotLimit?: number;
}

export interface SubscriptionSnapshotOptions {
  enabled: boolean;
}

/** Protocol v2 wire subscription. The client-selected ID remains stable across reconnects. */
export interface Subscription {
  type: 'subscribe';
  protocolVersion: 2;
  subscriptionId: string;
  query: SubscriptionQuery;
  snapshot: SubscriptionSnapshotOptions;
}

export interface SubscriptionRequest {
  query: SubscriptionQuery;
  snapshot?: Partial<SubscriptionSnapshotOptions>;
}

/** Canonical query identity includes every server query field and snapshot behavior. */
export type SubscriptionIdentity = SubscriptionRequest;

export type SubscriptionOptions = SubscriptionSnapshotOptions;

export interface QuerySnapshot<T = unknown> {
  readonly subscriptionId: string;
  readonly query: SubscriptionQuery;
  readonly keys: readonly string[];
  readonly data: readonly T[];
  readonly isLoading: boolean;
  readonly isRefreshing: boolean;
  readonly error?: AreteError;
}

export interface QueryLease {
  readonly subscription: Subscription;
  readonly queryKey: string;
  getSnapshot<T = unknown>(): QuerySnapshot<T>;
  onChange(callback: () => void): UnsubscribeFn;
  onUpdate<T = unknown>(callback: (update: Update<T>) => void): UnsubscribeFn;
  onRichUpdate<T = unknown>(callback: (update: RichUpdate<T>) => void): UnsubscribeFn;
  refresh(): Promise<void>;
  release(): void;
}

export type SchemaResult<T> =
  | { success: true; data: T }
  | { success: false; error: unknown };

export interface Schema<T> {
  safeParse: (input: unknown) => SchemaResult<T>;
}

export interface WatchOptions<TSchema = unknown> {
  partition?: string;
  take?: number;
  skip?: number;
  filters?: Record<string, unknown>;
  schema?: Schema<TSchema>;
  /** Whether to include initial snapshot (defaults to true) */
  withSnapshot?: boolean;
  /** Cursor for resuming from a specific point (_seq value) */
  after?: string;
  /** Maximum number of entities to include in snapshot */
  snapshotLimit?: number;
}

export interface AreteOptions<TStack extends StackDefinition> {
  stack: TStack;
  /** Connect immediately when the client is created (defaults to true). */
  autoConnect?: boolean;
  /** Reconnect automatically after an established connection is lost (defaults to true). */
  autoReconnect?: boolean;
  reconnectIntervals?: number[];
  maxReconnectAttempts?: number;
  validateFrames?: boolean;
}

export const DEFAULT_MAX_ENTRIES_PER_VIEW = 10_000;

export interface AuthTokenResult {
  token: string;
  expiresAt?: number;
  expires_at?: number;
  scopes?: readonly string[];
}

export interface ProgramReadBindingAuthTarget {
  readonly targetKind: 'program-read-binding';
  readonly targetId: string;
  readonly programReleaseHash: string;
}

export interface SolanaGatewayBindingAuthTarget {
  readonly targetKind: 'solana-gateway-binding';
  readonly targetId: string;
  readonly programReleaseHash?: never;
}

export type AuthTokenTarget =
  | ProgramReadBindingAuthTarget
  | SolanaGatewayBindingAuthTarget;

export type AuthTokenRequest =
  | {
      readonly scopes: readonly string[];
      readonly targetKind?: never;
      readonly targetId?: never;
      readonly programReleaseHash?: never;
    }
  | ({ readonly scopes: readonly string[] } & ProgramReadBindingAuthTarget)
  | ({ readonly scopes: readonly string[] } & SolanaGatewayBindingAuthTarget);

export interface WebSocketFactoryInit {
  headers?: Record<string, string>;
}

/**
 * Authentication configuration for Arete connections
 */
export interface AuthConfig {
  /** Custom token provider function - called before each connection and during refresh */
  getToken?: (request?: AuthTokenRequest) => Promise<string | AuthTokenResult>;
  /** Arete Cloud token endpoint URL */
  tokenEndpoint?: string;
  /** Publishable key for Arete Cloud */
  publishableKey?: string;
  /** Pre-minted static token (for server-side use) */
  token?: string;
  /** How the websocket token is sent to the server */
  tokenTransport?: 'query' | 'bearer';
  /** Custom websocket factory for non-browser environments */
  websocketFactory?: (url: string, init?: WebSocketFactoryInit) => WebSocket;
  /** Additional headers sent to the token endpoint */
  tokenEndpointHeaders?: Record<string, string>;
  /** Credentials mode for token endpoint fetches */
  tokenEndpointCredentials?: RequestCredentials;
}

export interface AreteConfig {
  /** WebSocket endpoint. `null`/omitted disables the WebSocket transport (HTTP-only mode). */
  websocketUrl?: string | null;
  /** Reconnect automatically after an established connection is lost (defaults to true). */
  autoReconnect?: boolean;
  reconnectIntervals?: number[];
  maxReconnectAttempts?: number;
  initialSubscriptions?: Subscription[];
  maxEntriesPerView?: number | null;
  /** Authentication configuration */
  auth?: AuthConfig;
  /** Fetch implementation used for authentication token requests. */
  fetch?: typeof fetch;
}

export interface SocketIssue {
  error: string;
  message: string;
  code: string | AuthErrorCode;
  retryable: boolean;
  retryAfter?: number;
  suggestedAction?: string;
  docsUrl?: string;
  fatal: boolean;
  subscriptionId?: string | null;
}

export const DEFAULT_CONFIG: Required<
  Pick<AreteConfig, 'autoReconnect' | 'reconnectIntervals' | 'maxReconnectAttempts' | 'maxEntriesPerView'>
> = {
  autoReconnect: true,
  reconnectIntervals: [1000, 2000, 4000, 8000, 16000],
  maxReconnectAttempts: 5,
  maxEntriesPerView: DEFAULT_MAX_ENTRIES_PER_VIEW,
};

/**
 * Machine-readable error codes for authentication and rate limiting failures
 *
 * These codes match the Rust AuthErrorCode enum for cross-platform consistency.
 */
export type AuthErrorCode =
  // Token validation errors
  | 'TOKEN_MISSING'
  | 'TOKEN_EXPIRED'
  | 'TOKEN_INVALID_SIGNATURE'
  | 'TOKEN_INVALID_FORMAT'
  | 'TOKEN_INVALID_ISSUER'
  | 'TOKEN_INVALID_AUDIENCE'
  | 'TOKEN_MISSING_CLAIM'
  | 'TOKEN_KEY_NOT_FOUND'
  // Origin and security errors
  | 'ORIGIN_MISMATCH'
  | 'ORIGIN_REQUIRED'
  | 'ORIGIN_NOT_ALLOWED'
  | 'AUTH_REQUIRED'
  | 'MISSING_AUTHORIZATION_HEADER'
  | 'INVALID_AUTHORIZATION_FORMAT'
  | 'INVALID_API_KEY'
  | 'EXPIRED_API_KEY'
  | 'USER_NOT_FOUND'
  | 'SECRET_KEY_REQUIRED'
  | 'DEPLOYMENT_ACCESS_DENIED'
  // Rate limiting and quota errors
  | 'RATE_LIMIT_EXCEEDED'
  | 'WEBSOCKET_SESSION_RATE_LIMIT_EXCEEDED'
  | 'CONNECTION_LIMIT_EXCEEDED'
  | 'SUBSCRIPTION_LIMIT_EXCEEDED'
  | 'SNAPSHOT_LIMIT_EXCEEDED'
  | 'EGRESS_LIMIT_EXCEEDED'
  | 'QUOTA_EXCEEDED'
  // Static token errors
  | 'INVALID_STATIC_TOKEN'
  // Server errors
  | 'INTERNAL_ERROR';

/**
 * Determines if the error indicates the client should retry the same request
 */
export function shouldRetryError(code: AuthErrorCode): boolean {
  return code === 'RATE_LIMIT_EXCEEDED'
    || code === 'WEBSOCKET_SESSION_RATE_LIMIT_EXCEEDED'
    || code === 'INTERNAL_ERROR';
}

/**
 * Determines if the error indicates the client should fetch a new token
 */
export function shouldRefreshToken(code: AuthErrorCode): boolean {
  return [
    'TOKEN_EXPIRED',
    'TOKEN_INVALID_SIGNATURE',
    'TOKEN_INVALID_FORMAT',
    'TOKEN_INVALID_ISSUER',
    'TOKEN_INVALID_AUDIENCE',
    'TOKEN_KEY_NOT_FOUND',
  ].includes(code);
}

export class AreteError extends Error {
  constructor(
    message: string,
    public code: string | AuthErrorCode,
    public details?: unknown
  ) {
    super(message);
    this.name = 'AreteError';
  }
}

export type TypedViews<TViews extends StackDefinition['views']> = {
  [K in keyof TViews]: TypedViewGroup<TViews[K]>;
};

export type TypedViewGroup<TGroup> = {
  [K in keyof TGroup]: TGroup[K] extends ViewDef<infer T, 'state', infer TKey>
    ? TypedStateView<T, DefaultViewKey<TKey>>
    : TGroup[K] extends ViewDef<infer T, 'list'>
      ? TypedListView<T>
      : never;
};

export type DefaultViewKey<TKey> = unknown extends TKey ? string : TKey;

export interface TypedStateView<T, TKey = string> {
  use<TSchema = T>(key: TKey, options?: WatchOptions<TSchema>): AsyncIterable<TSchema>;
  watch(key: TKey, options?: WatchOptions): AsyncIterable<Update<T>>;
  watchRich(key: TKey, options?: WatchOptions): AsyncIterable<RichUpdate<T>>;
  get(key: TKey, options?: WatchOptions): Promise<T | null>;
  getSync(key: TKey, options?: WatchOptions): T | null | undefined;
}

export interface TypedListView<T> {
  use<TSchema = T>(options?: WatchOptions<TSchema>): AsyncIterable<TSchema>;
  watch(options?: WatchOptions): AsyncIterable<Update<T>>;
  watchRich(options?: WatchOptions): AsyncIterable<RichUpdate<T>>;
  get(options?: WatchOptions): Promise<T[]>;
  getSync(options?: WatchOptions): T[] | undefined;
}

export type SubscribeCallback<T> = (update: Update<T>) => void;
export type UnsubscribeFn = () => void;

export type ConnectionStateCallback = (state: ConnectionState, error?: string) => void;
export type SocketIssueCallback = (issue: SocketIssue) => void;

/**
 * Parse a kebab-case error code string (from X-Error-Code header) to AuthErrorCode
 */
export function parseErrorCode(errorCode: string): AuthErrorCode {
  const codeMap: Record<string, AuthErrorCode> = {
    'token-missing': 'TOKEN_MISSING',
    'token-expired': 'TOKEN_EXPIRED',
    'token-invalid-signature': 'TOKEN_INVALID_SIGNATURE',
    'token-invalid-format': 'TOKEN_INVALID_FORMAT',
    'token-invalid-issuer': 'TOKEN_INVALID_ISSUER',
    'token-invalid-audience': 'TOKEN_INVALID_AUDIENCE',
    'token-missing-claim': 'TOKEN_MISSING_CLAIM',
    'token-key-not-found': 'TOKEN_KEY_NOT_FOUND',
    'origin-mismatch': 'ORIGIN_MISMATCH',
    'origin-required': 'ORIGIN_REQUIRED',
    'origin-not-allowed': 'ORIGIN_NOT_ALLOWED',
    'rate-limit-exceeded': 'RATE_LIMIT_EXCEEDED',
    'websocket-session-rate-limit-exceeded': 'WEBSOCKET_SESSION_RATE_LIMIT_EXCEEDED',
    'connection-limit-exceeded': 'CONNECTION_LIMIT_EXCEEDED',
    'subscription-limit-exceeded': 'SUBSCRIPTION_LIMIT_EXCEEDED',
    'snapshot-limit-exceeded': 'SNAPSHOT_LIMIT_EXCEEDED',
    'egress-limit-exceeded': 'EGRESS_LIMIT_EXCEEDED',
    'invalid-static-token': 'INVALID_STATIC_TOKEN',
    'internal-error': 'INTERNAL_ERROR',
    'auth-required': 'AUTH_REQUIRED',
    'missing-authorization-header': 'MISSING_AUTHORIZATION_HEADER',
    'invalid-authorization-format': 'INVALID_AUTHORIZATION_FORMAT',
    'invalid-api-key': 'INVALID_API_KEY',
    'expired-api-key': 'EXPIRED_API_KEY',
    'user-not-found': 'USER_NOT_FOUND',
    'secret-key-required': 'SECRET_KEY_REQUIRED',
    'deployment-access-denied': 'DEPLOYMENT_ACCESS_DENIED',
    'quota-exceeded': 'QUOTA_EXCEEDED',
  };

  return codeMap[errorCode.toLowerCase()] || 'INTERNAL_ERROR';
}

/**
 * Determines if a WebSocket close code indicates an authentication error
 */
export function isAuthErrorCloseCode(code: number): boolean {
  // 1008 = Policy Violation (used for auth failures)
  return code === 1008;
}

/**
 * Determines if a WebSocket close code indicates rate limiting
 */
export function isRateLimitCloseCode(code: number): boolean {
  // 1008 = Policy Violation can be used for rate limits
  // Browsers don't expose HTTP 429 during WebSocket handshake,
  // so servers should use close code 1008 with appropriate reason
  return code === 1008;
}
