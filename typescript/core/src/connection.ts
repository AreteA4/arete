import type { Frame } from './frame';
import { parseFrame, parseFrameFromBlob } from './frame';
import type {
  AuthConfig,
  AuthTokenResult,
  ConnectionState,
  ConnectionStateCallback,
  HyperStackConfig,
  SocketIssue,
  SocketIssueCallback,
  Subscription,
  WebSocketFactoryInit,
} from './types';
import { DEFAULT_CONFIG, HyperStackError, parseErrorCode, shouldRefreshToken } from './types';

export type FrameHandler = <T>(frame: Frame<T>) => void;

const TOKEN_REFRESH_BUFFER_SECONDS = 60;
const MIN_REFRESH_DELAY_MS = 1_000;
const DEFAULT_QUERY_PARAMETER = 'hs_token';
const DEFAULT_HOSTED_TOKEN_ENDPOINT = 'https://api.usehyperstack.com/ws/sessions';
const HOSTED_WEBSOCKET_SUFFIX = '.stack.usehyperstack.com';

interface TokenEndpointResponse {
  token: string;
  expires_at?: number;
  expiresAt?: number;
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
  error: string;
  message: string;
  code: string;
  retryable: boolean;
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
    && typeof candidate['message'] === 'string'
    && typeof candidate['code'] === 'string'
    && typeof candidate['retryable'] === 'boolean'
    && typeof candidate['fatal'] === 'boolean';
}

function isHostedHyperstackWebsocketUrl(websocketUrl: string): boolean {
  try {
    return new URL(websocketUrl).hostname.toLowerCase().endsWith(HOSTED_WEBSOCKET_SUFFIX);
  } catch {
    return false;
  }
}

export class ConnectionManager {
  private ws: WebSocket | null = null;
  private websocketUrl: string;
  private reconnectIntervals: number[];
  private maxReconnectAttempts: number;
  private reconnectAttempts = 0;
  private reconnectTimeout: ReturnType<typeof setTimeout> | null = null;
  private pingInterval: ReturnType<typeof setInterval> | null = null;
  private tokenRefreshTimeout: ReturnType<typeof setTimeout> | null = null;
  private tokenRefreshInFlight: Promise<void> | null = null;
  private currentState: ConnectionState = 'disconnected';
  private subscriptionQueue: Subscription[] = [];
  private activeSubscriptions: Set<string> = new Set();

  private frameHandlers: Set<FrameHandler> = new Set();
  private stateHandlers: Set<ConnectionStateCallback> = new Set();
  private socketIssueHandlers: Set<SocketIssueCallback> = new Set();

  private authConfig?: AuthConfig;
  private currentToken?: string;
  private tokenExpiry?: number;
  private readonly hostedHyperstackUrl: boolean;
  private reconnectForTokenRefresh = false;

  constructor(config: HyperStackConfig) {
    if (!config.websocketUrl) {
      throw new HyperStackError('websocketUrl is required', 'INVALID_CONFIG');
    }
    this.websocketUrl = config.websocketUrl;
    this.hostedHyperstackUrl = isHostedHyperstackWebsocketUrl(config.websocketUrl);
    this.reconnectIntervals = config.reconnectIntervals ?? DEFAULT_CONFIG.reconnectIntervals;
    this.maxReconnectAttempts =
      config.maxReconnectAttempts ?? DEFAULT_CONFIG.maxReconnectAttempts;
    this.authConfig = config.auth;

    if (config.initialSubscriptions) {
      this.subscriptionQueue.push(...config.initialSubscriptions);
    }
  }

  private getTokenEndpoint(): string | undefined {
    if (this.authConfig?.tokenEndpoint) {
      return this.authConfig.tokenEndpoint;
    }

    if (this.hostedHyperstackUrl && this.authConfig?.publishableKey) {
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

  private updateTokenState(result: string | AuthTokenResult): string {
    const normalized = normalizeTokenResult(result);
    if (!normalized.token) {
      throw new HyperStackError(
        'Authentication provider returned an empty token',
        'TOKEN_INVALID'
      );
    }

    this.currentToken = normalized.token;
    this.tokenExpiry = normalizeExpiryTimestamp(normalized.expiresAt, normalized.expires_at)
      ?? parseJwtExpiry(normalized.token);

    if (this.isTokenExpired()) {
      throw new HyperStackError('Authentication token is expired', 'TOKEN_EXPIRED');
    }

    return normalized.token;
  }

  private clearTokenState(): void {
    this.currentToken = undefined;
    this.tokenExpiry = undefined;
  }

  private async getOrRefreshToken(forceRefresh = false): Promise<string | undefined> {
    if (!forceRefresh && this.currentToken && !this.isTokenExpired()) {
      return this.currentToken;
    }

    const strategy = this.getAuthStrategy();

    if (strategy.kind === 'none' && this.hostedHyperstackUrl) {
      throw new HyperStackError(
        'Hosted Hyperstack websocket connections require auth.publishableKey, auth.getToken, auth.tokenEndpoint, or auth.token',
        'AUTH_REQUIRED'
      );
    }

    switch (strategy.kind) {
      case 'static-token':
        return this.updateTokenState(strategy.token);
      case 'token-provider':
        try {
          return this.updateTokenState(await strategy.getToken());
        } catch (error) {
          if (error instanceof HyperStackError) {
            throw error;
          }
          throw new HyperStackError(
            'Failed to get authentication token',
            'AUTH_REQUIRED',
            error
          );
        }
      case 'token-endpoint':
        try {
          return this.updateTokenState(
            await this.fetchTokenFromEndpoint(strategy.endpoint)
          );
        } catch (error) {
          if (error instanceof HyperStackError) {
            throw error;
          }
          throw new HyperStackError(
            'Failed to fetch authentication token from endpoint',
            'AUTH_REQUIRED',
            error
          );
        }
      case 'none':
        return undefined;
    }
  }

  private createTokenEndpointRequestBody(): Record<string, string> {
    return {
      websocket_url: this.websocketUrl,
    };
  }

  private async fetchTokenFromEndpoint(
    tokenEndpoint: string
  ): Promise<TokenEndpointResponse> {
    const response = await fetch(tokenEndpoint, {
      method: 'POST',
      headers: {
        ...(this.authConfig?.publishableKey
          ? { Authorization: `Bearer ${this.authConfig.publishableKey}` }
          : {}),
        ...(this.authConfig?.tokenEndpointHeaders ?? {}),
        'Content-Type': 'application/json',
      },
      credentials: this.authConfig?.tokenEndpointCredentials,
      body: JSON.stringify(this.createTokenEndpointRequestBody()),
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

      throw new HyperStackError(
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
      throw new HyperStackError(
        'Token endpoint did not return a token',
        'TOKEN_INVALID'
      );
    }

    return data;
  }

  private isTokenExpired(): boolean {
    if (!this.tokenExpiry) {
      return false;
    }

    return Date.now() >= (this.tokenExpiry - TOKEN_REFRESH_BUFFER_SECONDS) * 1000;
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

    this.tokenRefreshInFlight = (async () => {
      const previousToken = this.currentToken;
      try {
        await this.getOrRefreshToken(true);
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
        this.scheduleTokenRefresh();
      } catch {
        this.scheduleTokenRefresh();
      } finally {
        this.tokenRefreshInFlight = null;
      }
    })();

    return this.tokenRefreshInFlight;
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

    this.reconnectForTokenRefresh = true;
    this.updateState('reconnecting');
    this.ws.close(1000, 'token refresh');
  }

  private buildAuthUrl(token: string | undefined): string {
    if (this.authConfig?.tokenTransport === 'bearer') {
      return this.websocketUrl;
    }

    if (!token) {
      return this.websocketUrl;
    }

    const separator = this.websocketUrl.includes('?') ? '&' : '?';
    return `${this.websocketUrl}${separator}${DEFAULT_QUERY_PARAMETER}=${encodeURIComponent(token)}`;
  }

  private createWebSocket(url: string, token: string | undefined): WebSocket {
    if (this.authConfig?.tokenTransport === 'bearer') {
      const init: WebSocketFactoryInit | undefined = token
        ? { headers: { Authorization: `Bearer ${token}` } }
        : undefined;

      if (this.authConfig.websocketFactory) {
        return this.authConfig.websocketFactory(url, init);
      }

      throw new HyperStackError(
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
      error: message.error,
      message: message.message,
      code: parseErrorCode(message.code),
      retryable: message.retryable,
      retryAfter: message.retry_after,
      suggestedAction: message.suggested_action,
      docsUrl: message.docs_url,
      fatal: message.fatal,
    };

    for (const handler of this.socketIssueHandlers) {
      handler(issue);
    }

    return issue;
  }

  async connect(): Promise<void> {
    if (
      this.ws?.readyState === WebSocket.OPEN ||
      this.ws?.readyState === WebSocket.CONNECTING ||
      this.currentState === 'connecting'
    ) {
      return;
    }

    this.updateState('connecting');

    let token: string | undefined;
    try {
      token = await this.getOrRefreshToken();
    } catch (error) {
      this.updateState(
        'error',
        error instanceof Error ? error.message : 'Failed to get token'
      );
      throw error;
    }

    const wsUrl = this.buildAuthUrl(token);

    return new Promise((resolve, reject) => {
      try {
        this.ws = this.createWebSocket(wsUrl, token);

        this.ws.onopen = () => {
          this.reconnectAttempts = 0;
          this.updateState('connected');
          this.startPingInterval();
          this.scheduleTokenRefresh();
          this.resubscribeActive();
          this.flushSubscriptionQueue();
          resolve();
        };

        this.ws.onmessage = async (event) => {
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
                this.handleSocketIssueMessage(parsed);
                return;
              }
              frame = parseFrame(JSON.stringify(parsed));
            } else {
              throw new HyperStackError(
                `Unsupported message type: ${typeof event.data}`,
                'PARSE_ERROR'
              );
            }

            this.notifyFrameHandlers(frame);
          } catch {
            this.updateState('error', 'Failed to parse frame from server');
          }
        };

        this.ws.onerror = () => {
          const error = new HyperStackError('WebSocket connection error', 'CONNECTION_ERROR');
          this.updateState('error', error.message);
          if (this.currentState === 'connecting') {
            reject(error);
          }
        };

        this.ws.onclose = (event) => {
          this.stopPingInterval();
          this.clearTokenRefreshTimeout();
          this.ws = null;

          if (this.reconnectForTokenRefresh) {
            this.reconnectForTokenRefresh = false;
            void this.connect().catch(() => {
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
              // Try to reconnect immediately with a fresh token
              void this.connect().catch(() => {
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
            this.handleReconnect();
          }
        };
      } catch (error) {
        const hsError = new HyperStackError(
          'Failed to create WebSocket connection',
          'CONNECTION_ERROR',
          error
        );
        this.updateState('error', hsError.message);
        reject(hsError);
      }
    });
  }

  disconnect(): void {
    this.clearReconnectTimeout();
    this.stopPingInterval();
    this.clearTokenRefreshTimeout();
    this.reconnectForTokenRefresh = false;
    this.updateState('disconnected');

    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
  }

  subscribe(subscription: Subscription): void {
    const subKey = this.makeSubKey(subscription);

    if (this.currentState === 'connected' && this.ws?.readyState === WebSocket.OPEN) {
      if (this.activeSubscriptions.has(subKey)) {
        return;
      }
      const subMsg = { type: 'subscribe', ...subscription };
      this.ws.send(JSON.stringify(subMsg));
      this.activeSubscriptions.add(subKey);
    } else {
      const alreadyQueued = this.subscriptionQueue.some(
        (queuedSubscription) => this.makeSubKey(queuedSubscription) === subKey
      );
      if (!alreadyQueued) {
        this.subscriptionQueue.push(subscription);
      }
    }
  }

  unsubscribe(view: string, key?: string): void {
    const subscription: Subscription = { view, key };
    const subKey = this.makeSubKey(subscription);

    if (this.activeSubscriptions.has(subKey)) {
      this.activeSubscriptions.delete(subKey);

      if (this.ws?.readyState === WebSocket.OPEN) {
        const unsubMsg = { type: 'unsubscribe', view, key };
        this.ws.send(JSON.stringify(unsubMsg));
      }
    }
  }

  isConnected(): boolean {
    return this.currentState === 'connected' && this.ws?.readyState === WebSocket.OPEN;
  }

  private makeSubKey(subscription: Subscription): string {
    return `${subscription.view}:${subscription.key ?? '*'}:${subscription.partition ?? ''}`;
  }

  private flushSubscriptionQueue(): void {
    while (this.subscriptionQueue.length > 0) {
      const subscription = this.subscriptionQueue.shift();
      if (subscription) {
        this.subscribe(subscription);
      }
    }
  }

  private resubscribeActive(): void {
    for (const subKey of this.activeSubscriptions) {
      const [view, key, partition] = subKey.split(':');
      const subscription: Subscription = {
        view: view ?? '',
        key: key === '*' ? undefined : key,
        partition: partition || undefined,
      };

      if (this.ws?.readyState === WebSocket.OPEN) {
        const subMsg = { type: 'subscribe', ...subscription };
        this.ws.send(JSON.stringify(subMsg));
      }
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
      this.connect().catch(() => {
        /* retry handled by onclose */
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
