import type { Frame } from './frame';
import { parseFrame, parseFrameFromBlob } from './frame';
import type {
  AuthConfig,
  AuthTokenTarget,
  AuthTokenRequest,
  AuthTokenResult,
  ConnectionState,
  ConnectionStateCallback,
  AreteConfig,
  ProgramReadBindingAuthTarget,
  SocketIssue,
  SocketIssueCallback,
  Subscription,
  WebSocketFactoryInit,
} from './types';
import { DEFAULT_CONFIG, AreteError, parseErrorCode, shouldRefreshToken } from './types';
import {
  normalizeSubscription,
} from './subscription';

export type FrameHandler = <T>(frame: Frame<T>) => void;

const TOKEN_REFRESH_BUFFER_SECONDS = 60;
const MIN_REFRESH_DELAY_MS = 1_000;
const DEFAULT_QUERY_PARAMETER = 'hs_token';
const DEFAULT_HOSTED_TOKEN_ENDPOINT = 'https://api.arete.run/ws/sessions';
const HOSTED_WEBSOCKET_SUFFIX = '.stack.arete.run';
const MAX_HTTP_AUTH_TOKEN_STATES = 32;

interface TokenEndpointResponse {
  token: string;
  expires_at?: number;
  expiresAt?: number;
  scopes?: string[];
}

interface TokenEndpointErrorResponse {
  error?: string;
  code?: string;
}

interface RefreshAuthResponseMessage {
  success: boolean;
  error?: string;
  expires_at?: number;
  expiresAt?: number;
}

interface SocketIssueWireMessage {
  type: 'error';
  protocolVersion?: 2;
  subscriptionId?: string | null;
  error?: string;
  message?: string;
  code: string;
  retryable?: boolean;
  retry_after?: number;
  suggested_action?: string;
  docs_url?: string;
  fatal: boolean;
}

type AuthStrategy =
  | { kind: 'none' }
  | { kind: 'static-token'; token: string }
  | { kind: 'token-provider'; getToken: NonNullable<AuthConfig['getToken']> }
  | { kind: 'token-endpoint'; endpoint: string };

interface HttpAuthTokenState {
  token?: string;
  expiresAt?: number;
  inFlight?: Promise<string | undefined>;
}

function normalizeTokenResult(result: string | AuthTokenResult): AuthTokenResult {
  if (typeof result === 'string') {
    return { token: result };
  }

  return result;
}

function decodeBase64Url(value: string): string | undefined {
  const normalized = value.replace(/-/g, '+').replace(/_/g, '/');
  const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, '=');

  if (typeof atob === 'function') {
    return atob(padded);
  }

  const bufferCtor = (globalThis as { Buffer?: typeof Buffer }).Buffer;
  if (bufferCtor) {
    return bufferCtor.from(padded, 'base64').toString('utf-8');
  }

  return undefined;
}

function parseJwtExpiry(token: string): number | undefined {
  const parts = token.split('.');
  if (parts.length !== 3) {
    return undefined;
  }

  const payload = decodeBase64Url(parts[1] ?? '');
  if (!payload) {
    return undefined;
  }

  try {
    const decoded = JSON.parse(payload) as { exp?: unknown };
    return typeof decoded.exp === 'number' ? decoded.exp : undefined;
  } catch {
    return undefined;
  }
}

function normalizeExpiryTimestamp(expiresAt?: number, expires_at?: number): number | undefined {
  return expiresAt ?? expires_at;
}

function normalizeScopes(scopes: readonly string[]): string[] {
  return [...new Set(scopes)].sort();
}

function isProgramReadBindingAuthTarget(value: unknown): value is ProgramReadBindingAuthTarget {
  if (typeof value !== 'object' || value === null) {
    return false;
  }

  const candidate = value as Record<string, unknown>;
  return candidate['targetKind'] === 'program-read-binding'
    && typeof candidate['targetId'] === 'string'
    && typeof candidate['programReleaseHash'] === 'string';
}

function isAuthTokenTarget(value: unknown): value is AuthTokenTarget {
  if (isProgramReadBindingAuthTarget(value)) {
    return true;
  }
  if (typeof value !== 'object' || value === null) {
    return false;
  }

  const candidate = value as Record<string, unknown>;
  return candidate['targetKind'] === 'solana-gateway-binding'
    && typeof candidate['targetId'] === 'string'
    && candidate['programReleaseHash'] === undefined;
}

function isAuthTokenRequest(value: unknown): value is AuthTokenRequest {
  return typeof value === 'object'
    && value !== null
    && Array.isArray((value as { scopes?: unknown }).scopes);
}

function hasAuthTokenTarget(
  request: AuthTokenRequest
): request is AuthTokenRequest & AuthTokenTarget {
  return isAuthTokenTarget(request);
}

function normalizeAuthTokenRequest(request: AuthTokenRequest): AuthTokenRequest {
  const scopes = normalizeScopes(request.scopes);
  const hasTargetField = request.targetKind !== undefined
    || request.targetId !== undefined
    || request.programReleaseHash !== undefined;

  if (!hasTargetField) {
    return { scopes };
  }
  if (!hasAuthTokenTarget(request)) {
    throw new AreteError(
      'Targeted authentication requires a complete supported target identity',
      'INVALID_CONFIG'
    );
  }

  if (isProgramReadBindingAuthTarget(request)) {
    return {
      scopes,
      targetKind: request.targetKind,
      targetId: request.targetId,
      programReleaseHash: request.programReleaseHash,
    };
  }
  return {
    scopes,
    targetKind: request.targetKind,
    targetId: request.targetId,
  };
}

function createTargetedHttpTokenIdentity(
  request: AuthTokenRequest & AuthTokenTarget
): string {
  return JSON.stringify([
    request.targetKind,
    request.targetId,
    request.programReleaseHash ?? null,
    normalizeScopes(request.scopes),
  ]);
}

function isRefreshAuthResponseMessage(value: unknown): value is RefreshAuthResponseMessage {
  if (typeof value !== 'object' || value === null) {
    return false;
  }

  const candidate = value as Record<string, unknown>;
  return typeof candidate['success'] === 'boolean'
    && !('op' in candidate)
    && !('entity' in candidate)
    && !('mode' in candidate);
}

function isSocketIssueMessage(value: unknown): value is SocketIssueWireMessage {
  if (typeof value !== 'object' || value === null) {
    return false;
  }

  const candidate = value as Record<string, unknown>;
  return candidate['type'] === 'error'
    && typeof candidate['code'] === 'string'
    && typeof candidate['fatal'] === 'boolean'
    && (candidate['message'] === undefined || typeof candidate['message'] === 'string')
    && (candidate['error'] === undefined || typeof candidate['error'] === 'string')
    && (candidate['retryable'] === undefined || typeof candidate['retryable'] === 'boolean');
}

export function isHostedAreteEndpoint(url: string): boolean {
  try {
    return new URL(url).hostname.toLowerCase().endsWith(HOSTED_WEBSOCKET_SUFFIX);
  } catch {
    return false;
  }
}

/**
 * Historical sentinel accepted for back-compat: connecting with this URL is
 * treated the same as passing no WebSocket URL at all (HTTP-only mode).
 */
export const DISABLED_WEBSOCKET_URL = 'ws://127.0.0.1/__arete_disabled__';

export class ConnectionManager {
  private ws: WebSocket | null = null;
  private websocketUrl: string | null;
  private readonly autoReconnect: boolean;
  private reconnectIntervals: number[];
  private maxReconnectAttempts: number;
  private reconnectAttempts = 0;
  private reconnectTimeout: ReturnType<typeof setTimeout> | null = null;
  private pingInterval: ReturnType<typeof setInterval> | null = null;
  private tokenRefreshTimeout: ReturnType<typeof setTimeout> | null = null;
  private tokenRefreshInFlight: Promise<void> | null = null;
  private tokenRequestInFlight: Promise<string | undefined> | null = null;
  private readonly httpAuthTokenStates = new Map<string, HttpAuthTokenState>();
  private currentState: ConnectionState = 'disconnected';
  private subscriptionQueue: Map<string, Subscription> = new Map();
  private activeSubscriptions: Map<string, Subscription> = new Map();
  private socketGeneration = 0;
  private pendingConnect: {
    generation: number;
    reject: (error: AreteError) => void;
  } | null = null;

  private frameHandlers: Set<FrameHandler> = new Set();
  private stateHandlers: Set<ConnectionStateCallback> = new Set();
  private socketIssueHandlers: Set<SocketIssueCallback> = new Set();

  private authConfig?: AuthConfig;
  private readonly authFetch: typeof fetch;
  private currentToken?: string;
  private tokenExpiry?: number;
  private tokenScopes = new Set<string>();
  private requestedScopes = new Set<string>();
  private readonly hostedAreteUrl: boolean;
  private reconnectForTokenRefresh = false;

  constructor(config: AreteConfig) {
    const websocketUrl =
      config.websocketUrl && config.websocketUrl !== DISABLED_WEBSOCKET_URL
        ? config.websocketUrl
        : null;
    this.websocketUrl = websocketUrl;
    this.hostedAreteUrl = websocketUrl !== null && isHostedAreteEndpoint(websocketUrl);
    this.autoReconnect = config.autoReconnect ?? DEFAULT_CONFIG.autoReconnect;
    this.reconnectIntervals = config.reconnectIntervals ?? DEFAULT_CONFIG.reconnectIntervals;
    this.maxReconnectAttempts =
      config.maxReconnectAttempts ?? DEFAULT_CONFIG.maxReconnectAttempts;
    this.authConfig = config.auth;
    this.authFetch = config.fetch ?? ((input, init) => {
      if (typeof globalThis.fetch !== 'function') {
        throw new AreteError(
          'A fetch implementation is required for authentication token requests',
          'INVALID_CONFIG'
        );
      }
      return globalThis.fetch(input, init);
    });

    if (config.initialSubscriptions) {
      for (const subscription of config.initialSubscriptions) {
        this.addQueuedSubscription(subscription);
      }
    }
  }

  private getTokenEndpoint(): string | undefined {
    if (this.authConfig?.tokenEndpoint) {
      return this.authConfig.tokenEndpoint;
    }

    // Hosted stacks always mint through the Arete Cloud token endpoint.
    // Without a publishable key the session is minted anonymously — the API
    // rate limits by IP/origin and issues low-trust public limits instead.
    if (this.hostedAreteUrl) {
      return DEFAULT_HOSTED_TOKEN_ENDPOINT;
    }

    return undefined;
  }

  private getAuthStrategy(): AuthStrategy {
    if (this.authConfig?.token) {
      return { kind: 'static-token', token: this.authConfig.token };
    }

    if (this.authConfig?.getToken) {
      return { kind: 'token-provider', getToken: this.authConfig.getToken };
    }

    const tokenEndpoint = this.getTokenEndpoint();
    if (tokenEndpoint) {
      return { kind: 'token-endpoint', endpoint: tokenEndpoint };
    }

    return { kind: 'none' };
  }

  private hasRefreshableAuth(): boolean {
    const strategy = this.getAuthStrategy();
    return strategy.kind === 'token-provider' || strategy.kind === 'token-endpoint';
  }

  private updateTokenState(result: string | AuthTokenResult, requestedScopes: readonly string[]): string {
    const normalized = normalizeTokenResult(result);
    if (!normalized.token) {
      throw new AreteError(
        'Authentication provider returned an empty token',
        'TOKEN_INVALID'
      );
    }

    this.currentToken = normalized.token;
    this.tokenExpiry = normalizeExpiryTimestamp(normalized.expiresAt, normalized.expires_at)
      ?? parseJwtExpiry(normalized.token);
    this.tokenScopes = new Set(normalized.scopes ?? requestedScopes);

    if (this.isTokenExpired()) {
      throw new AreteError('Authentication token is expired', 'TOKEN_EXPIRED');
    }

    return normalized.token;
  }

  private clearTokenState(): void {
    this.currentToken = undefined;
    this.tokenExpiry = undefined;
    this.tokenScopes.clear();
  }

  private tokenCovers(scopes: readonly string[]): boolean {
    return scopes.every((scope) => this.tokenScopes.has(scope));
  }

  private async getOrRefreshToken(
    forceRefresh = false,
    requiredScopes: readonly string[] = ['read']
  ): Promise<string | undefined> {
    const normalizedRequiredScopes = normalizeScopes(requiredScopes);
    for (const scope of normalizedRequiredScopes) this.requestedScopes.add(scope);
    if (
      !forceRefresh
      && this.currentToken
      && !this.isTokenExpired()
      && this.tokenCovers(normalizedRequiredScopes)
    ) {
      return this.currentToken;
    }

    if (this.tokenRequestInFlight) {
      await this.tokenRequestInFlight;
      if (!this.currentToken) {
        return undefined;
      }
      if (
        this.currentToken
        && !this.isTokenExpired()
        && this.tokenCovers(normalizedRequiredScopes)
      ) {
        return this.currentToken;
      }
      throw new AreteError(
        `Authentication token was not granted required scopes: ${normalizedRequiredScopes.join(', ')}`,
        'AUTH_REQUIRED'
      );
    }

    let request!: Promise<string | undefined>;
    request = Promise.resolve().then(() =>
      this.fetchAuthToken(
        normalizeScopes([...this.tokenScopes, ...this.requestedScopes]),
        () => this.tokenRequestInFlight === request
      )
    );
    this.tokenRequestInFlight = request;
    try {
      const token = await request;
      if (token && !this.tokenCovers(normalizedRequiredScopes)) {
        throw new AreteError(
          `Authentication token was not granted required scopes: ${normalizedRequiredScopes.join(', ')}`,
          'AUTH_REQUIRED'
        );
      }
      return token;
    } finally {
      if (this.tokenRequestInFlight === request) {
        this.tokenRequestInFlight = null;
      }
    }
  }

  private async fetchAuthToken(
    scopes: readonly string[],
    isCurrent: () => boolean
  ): Promise<string | undefined> {
    const request: AuthTokenRequest = { scopes: normalizeScopes(scopes) };
    const result = await this.requestAuthToken(request, isCurrent);
    return result === undefined ? undefined : this.updateTokenState(result, request.scopes);
  }

  private async requestAuthToken(
    request: AuthTokenRequest,
    isCurrent: () => boolean
  ): Promise<string | AuthTokenResult | undefined> {
    const strategy = this.getAuthStrategy();
    const requireCurrent = () => {
      if (!isCurrent()) {
        throw new AreteError('Authentication request was superseded', 'CONNECTION_CANCELLED');
      }
    };

    switch (strategy.kind) {
      case 'static-token': {
        requireCurrent();
        return strategy.token;
      }
      case 'token-provider':
        try {
          const result = await strategy.getToken(request);
          requireCurrent();
          return result;
        } catch (error) {
          if (error instanceof AreteError) {
            throw error;
          }
          throw new AreteError(
            'Failed to get authentication token',
            'AUTH_REQUIRED',
            error
          );
        }
      case 'token-endpoint':
        try {
          const result = await this.fetchTokenFromEndpoint(strategy.endpoint, request);
          requireCurrent();
          return result;
        } catch (error) {
          if (error instanceof AreteError) {
            throw error;
          }
          throw new AreteError(
            'Failed to fetch authentication token from endpoint',
            'AUTH_REQUIRED',
            error
          );
        }
      case 'none':
        requireCurrent();
        return undefined;
    }
  }

  private updateHttpTokenState(
    state: HttpAuthTokenState,
    result: string | AuthTokenResult,
    request: AuthTokenRequest
  ): string {
    const normalized = normalizeTokenResult(result);
    if (!normalized.token) {
      throw new AreteError(
        'Authentication provider returned an empty token',
        'TOKEN_INVALID'
      );
    }

    const grantedScopes = new Set(normalized.scopes ?? request.scopes);
    if (!request.scopes.every((scope) => grantedScopes.has(scope))) {
      throw new AreteError(
        `Authentication token was not granted required scopes: ${request.scopes.join(', ')}`,
        'AUTH_REQUIRED'
      );
    }

    const expiresAt = normalizeExpiryTimestamp(normalized.expiresAt, normalized.expires_at)
      ?? parseJwtExpiry(normalized.token);
    if (this.isExpiryExpired(expiresAt)) {
      throw new AreteError('Authentication token is expired', 'TOKEN_EXPIRED');
    }

    state.token = normalized.token;
    state.expiresAt = expiresAt;
    return normalized.token;
  }

  private async getTargetedHttpAuthToken(
    request: AuthTokenRequest & AuthTokenTarget,
    forceRefresh: boolean
  ): Promise<string | undefined> {
    const identity = createTargetedHttpTokenIdentity(request);
    let state = this.httpAuthTokenStates.get(identity);

    if (!forceRefresh && state?.token && !this.isExpiryExpired(state.expiresAt)) {
      this.touchHttpAuthTokenState(identity, state);
      return state.token;
    }
    if (state?.inFlight) {
      this.touchHttpAuthTokenState(identity, state);
      return state.inFlight;
    }
    if (!state) {
      state = {};
      this.httpAuthTokenStates.set(identity, state);
    }

    const tokenState = state;
    let inFlight!: Promise<string | undefined>;
    inFlight = Promise.resolve().then(async () => {
      const result = await this.requestAuthToken(
        request,
        () => this.httpAuthTokenStates.get(identity) === tokenState
          && tokenState.inFlight === inFlight
      );
      return result === undefined
        ? undefined
        : this.updateHttpTokenState(tokenState, result, request);
    });
    tokenState.inFlight = inFlight;
    this.touchHttpAuthTokenState(identity, tokenState);
    this.trimHttpAuthTokenStates();

    try {
      return await inFlight;
    } finally {
      if (
        this.httpAuthTokenStates.get(identity) === tokenState
        && tokenState.inFlight === inFlight
      ) {
        tokenState.inFlight = undefined;
      }
      this.trimHttpAuthTokenStates();
    }
  }

  private touchHttpAuthTokenState(identity: string, state: HttpAuthTokenState): void {
    this.httpAuthTokenStates.delete(identity);
    this.httpAuthTokenStates.set(identity, state);
  }

  private trimHttpAuthTokenStates(): void {
    if (this.httpAuthTokenStates.size <= MAX_HTTP_AUTH_TOKEN_STATES) {
      return;
    }

    for (const [identity, state] of this.httpAuthTokenStates) {
      if (!state.inFlight) {
        this.httpAuthTokenStates.delete(identity);
      }
      if (this.httpAuthTokenStates.size <= MAX_HTTP_AUTH_TOKEN_STATES) {
        break;
      }
    }
  }

  private createTokenEndpointRequestBody(
    request: AuthTokenRequest
  ): Record<string, string | readonly string[]> {
    if (hasAuthTokenTarget(request)) {
      const target = {
        targetKind: request.targetKind,
        targetId: request.targetId,
        scopes: request.scopes,
      };
      return isProgramReadBindingAuthTarget(request)
        ? { ...target, programReleaseHash: request.programReleaseHash }
        : target;
    }

    return {
      websocket_url: this.websocketUrl ?? '',
      scopes: request.scopes,
    };
  }

  private async fetchTokenFromEndpoint(
    tokenEndpoint: string,
    request: AuthTokenRequest
  ): Promise<TokenEndpointResponse> {
    const response = await this.authFetch(tokenEndpoint, {
      method: 'POST',
      headers: {
        ...(this.authConfig?.publishableKey
          ? { Authorization: `Bearer ${this.authConfig.publishableKey}` }
          : {}),
        ...(this.authConfig?.tokenEndpointHeaders ?? {}),
        'Content-Type': 'application/json',
      },
      credentials: this.authConfig?.tokenEndpointCredentials,
      body: JSON.stringify(this.createTokenEndpointRequestBody(request)),
    });

    if (!response.ok) {
      const rawError = await response.text();
      let parsedError: TokenEndpointErrorResponse | undefined;

      if (rawError) {
        try {
          parsedError = JSON.parse(rawError) as TokenEndpointErrorResponse;
        } catch {
          parsedError = undefined;
        }
      }

      const wireErrorCode = response.headers.get('X-Error-Code')
        ?? (typeof parsedError?.code === 'string' ? parsedError.code : null);
      const errorCode = wireErrorCode
        ? parseErrorCode(wireErrorCode)
        : response.status === 429
          ? 'QUOTA_EXCEEDED'
          : 'AUTH_REQUIRED';
      const errorMessage = typeof parsedError?.error === 'string' && parsedError.error.length > 0
        ? parsedError.error
        : rawError || response.statusText || 'Authentication request failed';

      throw new AreteError(
        `Token endpoint returned ${response.status}: ${errorMessage}`,
        errorCode,
        {
          status: response.status,
          wireErrorCode,
          responseBody: rawError || null,
        }
      );
    }

    const data = (await response.json()) as TokenEndpointResponse;
    if (!data.token) {
      throw new AreteError(
        'Token endpoint did not return a token',
        'TOKEN_INVALID'
      );
    }

    return data;
  }

  private isExpiryExpired(expiresAt?: number): boolean {
    if (!expiresAt) {
      return false;
    }

    return Date.now() >= (expiresAt - TOKEN_REFRESH_BUFFER_SECONDS) * 1000;
  }

  private isTokenExpired(): boolean {
    return this.isExpiryExpired(this.tokenExpiry);
  }

  private scheduleTokenRefresh(): void {
    this.clearTokenRefreshTimeout();

    if (!this.hasRefreshableAuth() || !this.tokenExpiry) {
      return;
    }

    const refreshAtMs = Math.max(
      Date.now() + MIN_REFRESH_DELAY_MS,
      (this.tokenExpiry - TOKEN_REFRESH_BUFFER_SECONDS) * 1000
    );
    const delayMs = Math.max(MIN_REFRESH_DELAY_MS, refreshAtMs - Date.now());

    this.tokenRefreshTimeout = setTimeout(() => {
      void this.refreshTokenInBackground();
    }, delayMs);
  }

  private clearTokenRefreshTimeout(): void {
    if (this.tokenRefreshTimeout) {
      clearTimeout(this.tokenRefreshTimeout);
      this.tokenRefreshTimeout = null;
    }
  }

  private async refreshTokenInBackground(): Promise<void> {
    if (!this.hasRefreshableAuth()) {
      return;
    }

    if (this.tokenRefreshInFlight) {
      return this.tokenRefreshInFlight;
    }

    let refresh!: Promise<void>;
    refresh = (async () => {
      const previousToken = this.currentToken;
      try {
        await this.getOrRefreshToken(true, [...this.tokenScopes]);
        if (
          previousToken &&
          this.currentToken &&
          this.currentToken !== previousToken &&
          this.ws?.readyState === WebSocket.OPEN
        ) {
          // Try in-band auth refresh first
          const refreshed = await this.sendInBandAuthRefresh(this.currentToken);
          if (!refreshed) {
            // Fall back to reconnecting if in-band refresh failed
            this.rotateConnectionForTokenRefresh();
          }
        }
        if (this.tokenRefreshInFlight === refresh) {
          this.scheduleTokenRefresh();
        }
      } catch {
        if (this.tokenRefreshInFlight === refresh) {
          this.scheduleTokenRefresh();
        }
      } finally {
        if (this.tokenRefreshInFlight === refresh) {
          this.tokenRefreshInFlight = null;
        }
      }
    })();
    this.tokenRefreshInFlight = refresh;

    return refresh;
  }

  private async sendInBandAuthRefresh(token: string): Promise<boolean> {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      return false;
    }

    try {
      const message = JSON.stringify({
        type: 'refresh_auth',
        token: token,
      });
      this.ws.send(message);
      return true;
    } catch (error) {
      console.warn('Failed to send in-band auth refresh:', error);
      return false;
    }
  }

  private handleRefreshAuthResponse(message: RefreshAuthResponseMessage): boolean {
    if (message.success) {
      const expiresAt = normalizeExpiryTimestamp(message.expiresAt, message.expires_at);
      if (typeof expiresAt === 'number') {
        this.tokenExpiry = expiresAt;
      }
      this.scheduleTokenRefresh();
      return true;
    }

    const errorCode = message.error ? parseErrorCode(message.error) : 'INTERNAL_ERROR';
    if (shouldRefreshToken(errorCode)) {
      this.clearTokenState();
    }

    this.rotateConnectionForTokenRefresh();
    return true;
  }

  private handleSocketIssueMessage(message: SocketIssueWireMessage): boolean {
    this.notifySocketIssue(message);

    if (message.fatal) {
      this.updateState('error', message.message);
    }

    return true;
  }

  private rotateConnectionForTokenRefresh(): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN || this.reconnectForTokenRefresh) {
      return;
    }

    if (!this.autoReconnect) {
      this.updateState(
        'error',
        'Token refresh requires a new connection, but automatic reconnection is disabled'
      );
      this.ws.close(1000, 'token refresh');
      return;
    }

    this.reconnectForTokenRefresh = true;
    this.updateState('reconnecting');
    this.ws.close(1000, 'token refresh');
  }

  private buildAuthUrl(token: string | undefined): string {
    const websocketUrl = this.requireWebsocketUrl();
    if (this.authConfig?.tokenTransport === 'bearer') {
      return websocketUrl;
    }

    if (!token) {
      return websocketUrl;
    }

    const separator = websocketUrl.includes('?') ? '&' : '?';
    return `${websocketUrl}${separator}${DEFAULT_QUERY_PARAMETER}=${encodeURIComponent(token)}`;
  }

  private requireWebsocketUrl(): string {
    if (this.websocketUrl === null) {
      throw new AreteError(
        'WebSocket transport is disabled (client was connected with transport: "http"); views and subscriptions are unavailable',
        'WEBSOCKET_DISABLED'
      );
    }
    return this.websocketUrl;
  }

  private createWebSocket(url: string, token: string | undefined): WebSocket {
    if (this.authConfig?.tokenTransport === 'bearer') {
      const init: WebSocketFactoryInit | undefined = token
        ? { headers: { Authorization: `Bearer ${token}` } }
        : undefined;

      if (this.authConfig.websocketFactory) {
        return this.authConfig.websocketFactory(url, init);
      }

      throw new AreteError(
        'auth.tokenTransport="bearer" requires auth.websocketFactory in this environment',
        'INVALID_CONFIG'
      );
    }

    if (this.authConfig?.websocketFactory) {
      return this.authConfig.websocketFactory(url);
    }

    return new WebSocket(url);
  }

  getState(): ConnectionState {
    return this.currentState;
  }

  async getHttpAuthToken(forceRefresh?: boolean): Promise<string | undefined>;
  async getHttpAuthToken(requiredScopes?: readonly string[], forceRefresh?: boolean): Promise<string | undefined>;
  async getHttpAuthToken(request: AuthTokenRequest, forceRefresh?: boolean): Promise<string | undefined>;
  async getHttpAuthToken(
    target: AuthTokenTarget,
    forceRefresh?: boolean
  ): Promise<string | undefined>;
  async getHttpAuthToken(
    target: AuthTokenTarget,
    requiredScopes: readonly string[],
    forceRefresh?: boolean
  ): Promise<string | undefined>;
  async getHttpAuthToken(
    requiredScopes: readonly string[],
    target: AuthTokenTarget,
    forceRefresh?: boolean
  ): Promise<string | undefined>;
  async getHttpAuthToken(
    requiredScopes: readonly string[],
    forceRefresh: boolean,
    target: AuthTokenTarget
  ): Promise<string | undefined>;
  async getHttpAuthToken(
    requestOrScopesOrTargetOrForce:
      | AuthTokenRequest
      | AuthTokenTarget
      | readonly string[]
      | boolean = ['read'],
    scopesTargetOrForce: readonly string[] | AuthTokenTarget | boolean = false,
    targetOrForce: AuthTokenTarget | boolean = false
  ): Promise<string | undefined> {
    let request: AuthTokenRequest;
    let force = false;

    if (typeof requestOrScopesOrTargetOrForce === 'boolean') {
      request = { scopes: ['read'] };
      force = requestOrScopesOrTargetOrForce;
    } else if (Array.isArray(requestOrScopesOrTargetOrForce)) {
      if (isAuthTokenTarget(scopesTargetOrForce)) {
        request = {
          scopes: requestOrScopesOrTargetOrForce,
          ...scopesTargetOrForce,
        };
        force = typeof targetOrForce === 'boolean' ? targetOrForce : false;
      } else if (isAuthTokenTarget(targetOrForce)) {
        request = {
          scopes: requestOrScopesOrTargetOrForce,
          ...targetOrForce,
        };
        force = typeof scopesTargetOrForce === 'boolean' ? scopesTargetOrForce : false;
      } else {
        request = { scopes: requestOrScopesOrTargetOrForce };
        force = typeof scopesTargetOrForce === 'boolean' ? scopesTargetOrForce : false;
      }
    } else if (isAuthTokenRequest(requestOrScopesOrTargetOrForce)) {
      request = requestOrScopesOrTargetOrForce;
      force = typeof scopesTargetOrForce === 'boolean' ? scopesTargetOrForce : false;
    } else if (isAuthTokenTarget(requestOrScopesOrTargetOrForce)) {
      const requiredScopes = Array.isArray(scopesTargetOrForce)
        ? scopesTargetOrForce
        : ['read'];
      request = {
        scopes: requiredScopes,
        ...requestOrScopesOrTargetOrForce,
      };
      force = typeof scopesTargetOrForce === 'boolean'
        ? scopesTargetOrForce
        : typeof targetOrForce === 'boolean' ? targetOrForce : false;
    } else {
      throw new AreteError('Invalid HTTP authentication token request', 'INVALID_CONFIG');
    }

    const normalizedRequest = normalizeAuthTokenRequest(request);
    return hasAuthTokenTarget(normalizedRequest)
      ? this.getTargetedHttpAuthToken(normalizedRequest, force)
      : this.getOrRefreshToken(force, normalizedRequest.scopes);
  }

  clearHttpAuthToken(): void;
  clearHttpAuthToken(request: AuthTokenRequest): void;
  clearHttpAuthToken(
    target: AuthTokenTarget,
    requiredScopes?: readonly string[]
  ): void;
  clearHttpAuthToken(
    identity?: AuthTokenRequest | AuthTokenTarget,
    requiredScopes: readonly string[] = ['read']
  ): void {
    if (identity === undefined) {
      this.clearTokenState();
      this.httpAuthTokenStates.clear();
      return;
    }

    const request = normalizeAuthTokenRequest(
      isAuthTokenRequest(identity)
        ? identity
        : {
            scopes: requiredScopes,
            ...identity,
          }
    );
    if (hasAuthTokenTarget(request)) {
      this.httpAuthTokenStates.delete(createTargetedHttpTokenIdentity(request));
    } else {
      this.clearTokenState();
    }
  }

  onFrame(handler: FrameHandler): () => void {
    this.frameHandlers.add(handler);
    return () => {
      this.frameHandlers.delete(handler);
    };
  }

  onStateChange(handler: ConnectionStateCallback): () => void {
    this.stateHandlers.add(handler);
    return () => {
      this.stateHandlers.delete(handler);
    };
  }

  onSocketIssue(handler: SocketIssueCallback): () => void {
    this.socketIssueHandlers.add(handler);
    return () => {
      this.socketIssueHandlers.delete(handler);
    };
  }

  private notifySocketIssue(message: SocketIssueWireMessage): SocketIssue {
    const issue: SocketIssue = {
      error: message.error ?? message.code,
      message: message.message ?? message.error ?? message.code,
      code: parseErrorCode(message.code),
      retryable: message.retryable ?? false,
      retryAfter: message.retry_after,
      suggestedAction: message.suggested_action,
      docsUrl: message.docs_url,
      fatal: message.fatal,
      subscriptionId: message.subscriptionId,
    };

    for (const handler of this.socketIssueHandlers) {
      handler(issue);
    }

    return issue;
  }

  async connect(recovering = false): Promise<void> {
    this.requireWebsocketUrl();

    if (
      this.ws?.readyState === WebSocket.OPEN ||
      this.ws?.readyState === WebSocket.CONNECTING ||
      this.currentState === 'connecting'
    ) {
      return;
    }

    const generation = ++this.socketGeneration;
    const cancellation = new Promise<never>((_resolve, reject) => {
      this.pendingConnect = { generation, reject };
    });
    this.updateState(recovering ? 'reconnecting' : 'connecting');
    if (generation !== this.socketGeneration) {
      return cancellation;
    }

    let token: string | undefined;
    try {
      token = await Promise.race([
        this.getOrRefreshToken(false, ['read']),
        cancellation,
      ]);
    } catch (error) {
      if (this.pendingConnect?.generation === generation) {
        this.pendingConnect = null;
      }
      if (generation === this.socketGeneration) {
        this.updateState(
          recovering ? 'reconnecting' : 'error',
          error instanceof Error ? error.message : 'Failed to get token'
        );
      }
      throw error;
    }
    if (generation !== this.socketGeneration) {
      throw new AreteError('WebSocket connection attempt was superseded', 'CONNECTION_CANCELLED');
    }

    const wsUrl = this.buildAuthUrl(token);

    const socketConnection = new Promise<void>((resolve, reject) => {
      let settled = false;
      const finish = (fn: () => void) => {
        if (settled) {
          return;
        }
        settled = true;
        if (this.pendingConnect?.generation === generation) {
          this.pendingConnect = null;
        }
        fn();
      };
      try {
        const socket = this.createWebSocket(wsUrl, token);
        this.ws = socket;
        const isCurrentSocket = () =>
          this.ws === socket && this.socketGeneration === generation;

        socket.onopen = () => {
          if (!isCurrentSocket()) return;
          this.reconnectAttempts = 0;
          this.updateState('connected');
          this.startPingInterval();
          this.scheduleTokenRefresh();
          this.resubscribeActive();
          this.flushSubscriptionQueue();
          finish(() => resolve());
        };

        socket.onmessage = async (event) => {
          if (!isCurrentSocket()) return;
          try {
            let frame: Frame;

            if (event.data instanceof ArrayBuffer) {
              frame = parseFrame(event.data);
            } else if (event.data instanceof Blob) {
              frame = await parseFrameFromBlob(event.data);
            } else if (typeof event.data === 'string') {
              const parsed = JSON.parse(event.data) as unknown;
              if (isRefreshAuthResponseMessage(parsed)) {
                this.handleRefreshAuthResponse(parsed);
                return;
              }
              if (isSocketIssueMessage(parsed)) {
                if (parsed.protocolVersion === 2) {
                  const issueFrame = parseFrame(JSON.stringify(parsed));
                  if (isCurrentSocket()) this.notifyFrameHandlers(issueFrame);
                  if (parsed.subscriptionId === null || parsed.fatal) {
                    this.handleSocketIssueMessage(parsed);
                  }
                } else {
                  this.handleSocketIssueMessage(parsed);
                }
                return;
              }
              frame = parseFrame(JSON.stringify(parsed));
            } else {
              throw new AreteError(
                `Unsupported message type: ${typeof event.data}`,
                'PARSE_ERROR'
              );
            }

            if (isCurrentSocket()) this.notifyFrameHandlers(frame);
          } catch {
            if (!isCurrentSocket()) return;
            this.updateState('error', 'Failed to parse frame from server');
          }
        };

        socket.onerror = () => {
          if (!isCurrentSocket()) return;
          const error = new AreteError('WebSocket connection error', 'CONNECTION_ERROR');
          const wasConnecting = this.currentState === 'connecting';
          this.updateState(
            wasConnecting || !this.autoReconnect ? 'error' : 'reconnecting',
            error.message
          );
          if (wasConnecting) {
            finish(() => reject(error));
          }
        };

        socket.onclose = (event) => {
          if (!isCurrentSocket()) return;
          this.stopPingInterval();
          this.clearTokenRefreshTimeout();
          this.ws = null;

          if (!settled) {
            const detail = event.reason
              ? `${event.code}: ${event.reason}`
              : `code ${event.code}`;
            const errorMessage = `WebSocket closed before open (${detail})`;
            this.updateState(recovering ? 'reconnecting' : 'error', errorMessage);
            finish(() =>
              reject(new AreteError(errorMessage, 'CONNECTION_ERROR'))
            );
            return;
          }

          if (this.reconnectForTokenRefresh) {
            this.reconnectForTokenRefresh = false;
            if (!this.autoReconnect) {
              this.updateState(
                'error',
                'WebSocket closed for token refresh and automatic reconnection is disabled'
              );
              return;
            }
            void this.connect(true).catch(() => {
              this.handleReconnect();
            });
            return;
          }

          // Parse close reason for error codes (e.g., "token-expired: Token has expired")
          const closeReason = event.reason || '';
          const errorCodeMatch = closeReason.match(/^([\w-]+):/);
          const errorCode = errorCodeMatch ? parseErrorCode(errorCodeMatch[1]!) : null;

          // Check for auth errors that require token refresh
          if (event.code === 1008 || errorCode) {
            const isAuthError = errorCode
              ? shouldRefreshToken(errorCode)
              : /expired|invalid|token/i.test(closeReason);

            if (isAuthError) {
              this.clearTokenState();
              if (!this.autoReconnect) {
                this.updateState(
                  'error',
                  `Authentication refresh requires reconnection, but automatic reconnection is disabled: ${closeReason || event.code}`
                );
                return;
              }
              // Try to reconnect immediately with a fresh token
              void this.connect(true).catch(() => {
                this.handleReconnect();
              });
              return;
            }

            // Check for rate limit errors
            const isRateLimit = errorCode === 'RATE_LIMIT_EXCEEDED' ||
              errorCode === 'CONNECTION_LIMIT_EXCEEDED' ||
              /rate.?limit|quota|limit.?exceeded/i.test(closeReason);

            if (isRateLimit) {
              this.updateState('error', `Rate limit exceeded: ${closeReason}`);
              // Don't auto-reconnect on rate limits, let user handle it
              return;
            }
          }

          if (this.currentState !== 'disconnected') {
            if (this.autoReconnect) {
              this.handleReconnect();
            } else {
              const detail = event.reason
                ? `${event.code}: ${event.reason}`
                : `code ${event.code}`;
              this.updateState(
                'error',
                `WebSocket closed (${detail}) and automatic reconnection is disabled`
              );
            }
          }
        };
      } catch (error) {
        const hsError = new AreteError(
          'Failed to create WebSocket connection',
          'CONNECTION_ERROR',
          error
        );
        this.updateState(recovering ? 'reconnecting' : 'error', hsError.message);
        finish(() => reject(hsError));
      }
    });
    return Promise.race([socketConnection, cancellation]);
  }

  disconnect(): void {
    this.clearReconnectTimeout();
    this.stopPingInterval();
    this.clearTokenRefreshTimeout();
    this.reconnectForTokenRefresh = false;
    const pendingConnect = this.pendingConnect;
    this.pendingConnect = null;
    this.tokenRequestInFlight = null;
    this.tokenRefreshInFlight = null;
    this.socketGeneration++;
    pendingConnect?.reject(
      new AreteError('WebSocket connection attempt was cancelled', 'CONNECTION_CANCELLED')
    );

    if (this.ws) {
      const socket = this.ws;
      this.ws = null;
      socket.close();
    }
    this.updateState('disconnected');
  }

  subscribe(subscription: Subscription): void {
    this.requireWebsocketUrl();

    const normalized = normalizeSubscription(subscription);
    const subscriptionId = normalized.subscriptionId;
    const existing = this.activeSubscriptions.get(subscriptionId)
      ?? this.subscriptionQueue.get(subscriptionId);

    if (existing) {
      if (JSON.stringify(existing) !== JSON.stringify(normalized)) {
        throw new AreteError(
          `subscriptionId '${subscriptionId}' is already registered locally`,
          'DUPLICATE_SUBSCRIPTION_ID'
        );
      }
      return;
    }

    if (this.currentState === 'connected' && this.ws?.readyState === WebSocket.OPEN) {
      try {
        this.ws.send(JSON.stringify(normalized));
      } catch (error) {
        try {
          const unsubMsg = {
            type: 'unsubscribe',
            protocolVersion: 2,
            subscriptionId,
          };
          this.ws.send(JSON.stringify(unsubMsg));
        } catch {
          // The original send error is more useful; cleanup is best-effort.
        }
        throw error;
      }
      this.activeSubscriptions.set(subscriptionId, normalized);
    } else {
      this.subscriptionQueue.set(subscriptionId, normalized);
    }
  }

  unsubscribe(subscriptionId: string): void {
    this.subscriptionQueue.delete(subscriptionId);

    if (this.activeSubscriptions.has(subscriptionId)) {
      this.activeSubscriptions.delete(subscriptionId);

      if (this.ws?.readyState === WebSocket.OPEN) {
        const unsubMsg = { type: 'unsubscribe', protocolVersion: 2, subscriptionId };
        this.ws.send(JSON.stringify(unsubMsg));
      }
    }
  }

  refresh(subscription: Subscription): void {
    this.requireWebsocketUrl();

    const normalized = normalizeSubscription(subscription);
    const subscriptionId = normalized.subscriptionId;
    const existing = this.activeSubscriptions.get(subscriptionId)
      ?? this.subscriptionQueue.get(subscriptionId);

    if (!existing) {
      throw new AreteError(
        `Cannot refresh inactive subscription '${subscriptionId}'`,
        'SUBSCRIPTION_NOT_FOUND'
      );
    }
    if (JSON.stringify(existing) !== JSON.stringify(normalized)) {
      throw new AreteError('Cannot change a subscription while refreshing it', 'INVALID_SUBSCRIPTION');
    }

    try {
      this.unsubscribe(subscriptionId);
      this.subscribe(existing);
    } catch (error) {
      if (
        !this.activeSubscriptions.has(subscriptionId)
        && !this.subscriptionQueue.has(subscriptionId)
      ) {
        this.addQueuedSubscription(existing);
      }
      throw error;
    }
  }

  isConnected(): boolean {
    return this.currentState === 'connected' && this.ws?.readyState === WebSocket.OPEN;
  }

  private flushSubscriptionQueue(): void {
    const queuedSubscriptions = Array.from(this.subscriptionQueue.values());
    this.subscriptionQueue.clear();
    for (const subscription of queuedSubscriptions) {
      this.subscribe(subscription);
    }
  }

  private resubscribeActive(): void {
    for (const subscription of this.activeSubscriptions.values()) {
      if (this.ws?.readyState === WebSocket.OPEN) {
        this.ws.send(JSON.stringify(subscription));
      }
    }
  }

  private addQueuedSubscription(subscription: Subscription): void {
    const normalized = normalizeSubscription(subscription);
    const subscriptionId = normalized.subscriptionId;
    const existing = this.subscriptionQueue.get(subscriptionId);

    if (existing && JSON.stringify(existing) !== JSON.stringify(normalized)) {
      throw new AreteError(
        `subscriptionId '${subscriptionId}' is already queued`,
        'DUPLICATE_SUBSCRIPTION_ID'
      );
    }
    if (!existing) {
      this.subscriptionQueue.set(subscriptionId, normalized);
    }
  }

  private updateState(state: ConnectionState, error?: string): void {
    this.currentState = state;
    for (const handler of this.stateHandlers) {
      handler(state, error);
    }
  }

  private notifyFrameHandlers(frame: Frame): void {
    for (const handler of this.frameHandlers) {
      handler(frame);
    }
  }

  private handleReconnect(): void {
    if (!this.autoReconnect) {
      this.updateState('error', 'Automatic reconnection is disabled');
      return;
    }
    if (this.reconnectAttempts >= this.maxReconnectAttempts) {
      this.updateState(
        'error',
        `Max reconnection attempts (${this.reconnectAttempts}) reached`
      );
      return;
    }

    this.updateState('reconnecting');

    const attemptIndex = Math.min(
      this.reconnectAttempts,
      this.reconnectIntervals.length - 1
    );
    const delay = this.reconnectIntervals[attemptIndex] ?? 1000;

    this.reconnectAttempts++;

    this.reconnectTimeout = setTimeout(() => {
      this.connect(true).catch(() => {
        // Once a socket exists, its close event owns the next retry. Token
        // acquisition and socket construction can fail before that point.
        if (this.ws === null && this.currentState !== 'disconnected') {
          this.handleReconnect();
        }
      });
    }, delay);
  }

  private clearReconnectTimeout(): void {
    if (this.reconnectTimeout) {
      clearTimeout(this.reconnectTimeout);
      this.reconnectTimeout = null;
    }
  }

  private startPingInterval(): void {
    this.stopPingInterval();
    this.pingInterval = setInterval(() => {
      if (this.ws?.readyState === WebSocket.OPEN) {
        this.ws.send('{"type":"ping"}');
      }
    }, 15000);
  }

  private stopPingInterval(): void {
    if (this.pingInterval) {
      clearInterval(this.pingInterval);
      this.pingInterval = null;
    }
  }
}
