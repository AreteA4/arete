export type ConnectionState =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'error';

export type Update<T> =
  | { type: 'upsert'; key: string; data: T }
  | { type: 'patch'; key: string; data: Partial<T> }
  | { type: 'delete'; key: string };

export type RichUpdate<T> =
  | { type: 'created'; key: string; data: T }
  | { type: 'updated'; key: string; before: T; after: T; patch?: unknown }
  | { type: 'deleted'; key: string; lastKnown?: T };

export interface ViewDef<T, TMode extends 'state' | 'list'> {
  readonly mode: TMode;
  readonly view: string;
  readonly _entity?: T;
}

export interface StackDefinition {
  readonly name: string;
  readonly url: string;
  readonly views: Record<string, ViewGroup>;
  readonly schemas?: Record<string, Schema<unknown>>;
  instructions?: Record<string, import('./instructions').InstructionHandler>;
}

export interface ViewGroup {
  state?: ViewDef<unknown, 'state'>;
  list?: ViewDef<unknown, 'list'>;
}

export interface Subscription {
  view: string;
  key?: string;
  partition?: string;
  filters?: Record<string, string>;
  take?: number;
  skip?: number;
  /** Whether to include initial snapshot (defaults to true for backward compatibility) */
  withSnapshot?: boolean;
  /** Cursor for resuming from a specific point (_seq value) */
  after?: string;
  /** Maximum number of entities to include in snapshot (pagination hint) */
  snapshotLimit?: number;
}

export type SchemaResult<T> =
  | { success: true; data: T }
  | { success: false; error: unknown };

export interface Schema<T> {
  safeParse: (input: unknown) => SchemaResult<T>;
}

export interface WatchOptions<TSchema = unknown> {
  take?: number;
  skip?: number;
  filters?: Record<string, string>;
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
}

export interface WebSocketFactoryInit {
  headers?: Record<string, string>;
}

/**
 * Authentication configuration for Arete connections
 */
export interface AuthConfig {
  /** Custom token provider function - called before each connection and during refresh */
  getToken?: () => Promise<string | AuthTokenResult>;
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
  websocketUrl?: string;
  reconnectIntervals?: number[];
  maxReconnectAttempts?: number;
  initialSubscriptions?: Subscription[];
  maxEntriesPerView?: number | null;
  /** Authentication configuration */
  auth?: AuthConfig;
}

export interface SocketIssue {
  error: string;
  message: string;
  code: AuthErrorCode;
  retryable: boolean;
  retryAfter?: number;
  suggestedAction?: string;
  docsUrl?: string;
  fatal: boolean;
}

export const DEFAULT_CONFIG: Required<
  Pick<AreteConfig, 'reconnectIntervals' | 'maxReconnectAttempts' | 'maxEntriesPerView'>
> = {
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
  [K in keyof TGroup]: TGroup[K] extends ViewDef<infer T, 'state'>
    ? TypedStateView<T>
    : TGroup[K] extends ViewDef<infer T, 'list'>
      ? TypedListView<T>
      : never;
};

export interface TypedStateView<T> {
  use<TSchema = T>(key: string, options?: WatchOptions<TSchema>): AsyncIterable<TSchema>;
  watch(key: string, options?: WatchOptions): AsyncIterable<Update<T>>;
  watchRich(key: string, options?: WatchOptions): AsyncIterable<RichUpdate<T>>;
  get(key: string): Promise<T | null>;
  getSync(key: string): T | null | undefined;
}

export interface TypedListView<T> {
  use<TSchema = T>(options?: WatchOptions<TSchema>): AsyncIterable<TSchema>;
  watch(options?: WatchOptions): AsyncIterable<Update<T>>;
  watchRich(options?: WatchOptions): AsyncIterable<RichUpdate<T>>;
  get(): Promise<T[]>;
  getSync(): T[] | undefined;
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
