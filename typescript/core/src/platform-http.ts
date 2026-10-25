/**
 * Authenticated HTTP to the Arete API's account routes (`/api/...`).
 *
 * Account routes accept the caller's own credential as the bearer: a secret
 * or agent key (servers and scripts) or a user session token (an app that
 * signs its users in to Arete). They do not take the short-lived sessions
 * minted for stacks, gateways and protocol deployments, so this client never
 * mints one: `token`, `getToken` and `tokenEndpoint` hand out the API
 * credential, as with `auth.tokenKind: 'api'` (set it, and the same auth also
 * drives the hosted gateway transports, which mint binding sessions with it),
 * and a secret key is sent as it is.
 *
 * SDK extensions receive a client as a {@link PlatformHttpClient} value
 * (the type is exported from `@usearete/sdk/program`), so extension code
 * makes platform calls without importing the client runtime or touching a
 * host global.
 */

import { resolveAuthConfig } from './auth-keys';
import { createApiBearerSource, isApiCredentialAuth, sharedApiCredential } from './api-credential';
import { DEFAULT_A4_API_URL } from './a4-profile';
import {
  accountCredentialSource,
  assertCredentialDestination,
  UNTRUSTED_CREDENTIAL_DESTINATION,
  withTrustedApiUrl,
} from './credential-destination';
import { AreteError, parseApiProblem, parseErrorCode, type AuthConfig } from './types';

export type PlatformHttpMethod = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

export interface PlatformHttpRequest {
  /** Path under the API base, e.g. `/account/squads`. */
  readonly path: string;
  readonly method?: PlatformHttpMethod;
  /** JSON body; serialized with `JSON.stringify`. */
  readonly body?: unknown;
  readonly signal?: AbortSignal;
}

export interface PlatformHttpResponse<T = unknown> {
  readonly status: number;
  /** The parsed JSON body, `undefined` for an empty one. */
  readonly body: T;
}

/**
 * Makes authenticated calls to the Arete API. A non-2xx answer rejects with
 * an `AreteError` whose `details` carry `status`, `wireErrorCode`, the parsed
 * `body` and `apiProblem`.
 */
export interface PlatformHttpClient {
  /** The API base, ending in `/api`. */
  readonly apiUrl: string;
  request<T = unknown>(request: PlatformHttpRequest): Promise<PlatformHttpResponse<T>>;
}

export interface PlatformHttpClientOptions {
  /**
   * API origin, with or without `/api`. Defaults to `https://api.arete.run`.
   * Setting it trusts that origin for a credential passed in `auth`.
   */
  readonly apiUrl?: string;
  /**
   * `secretKey`, `token`, `getToken` or `tokenEndpoint`. Outside browsers an
   * unset auth falls back to `ARETE_API_KEY`, then the `a4` login. Account
   * credentials go only to trusted Arete API origins: an `apiUrl` they may
   * not reach throws `UNTRUSTED_CREDENTIAL_DESTINATION`. Publishable keys are
   * refused.
   */
  readonly auth?: AuthConfig;
  readonly fetch?: typeof fetch;
}

function resolveFetch(fetchImpl: typeof fetch | undefined): typeof fetch {
  if (fetchImpl) return fetchImpl;
  if (typeof globalThis.fetch === 'function') return globalThis.fetch.bind(globalThis);
  throw new AreteError('A fetch implementation is required for Arete API calls', 'INVALID_CONFIG');
}

function apiBase(apiUrl: string | undefined): string {
  const origin = (apiUrl ?? DEFAULT_A4_API_URL).trim().replace(/\/+$/, '');
  if (!/^https?:\/\//.test(origin)) {
    throw new AreteError('apiUrl must be an http(s) URL', 'INVALID_CONFIG');
  }
  return origin.endsWith('/api') ? origin : `${origin}/api`;
}

function parseBody(text: string): unknown {
  if (!text) return undefined;
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return text;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * The platform client a client or session builds for itself: one when `auth`
 * carries an API credential (a secret key, or `tokenKind: 'api'`), otherwise
 * `undefined`. When that credential may not be sent to `apiUrl` (an `a4`
 * login or `ARETE_API_KEY` key with an `apiUrl` on another origin), the
 * client still exists and every request rejects with
 * `UNTRUSTED_CREDENTIAL_DESTINATION` before anything is sent, so the error
 * names the cause where the call is made.
 */
export function defaultPlatformHttpClient(
  options: PlatformHttpClientOptions
): PlatformHttpClient | undefined {
  const { auth } = options;
  if (auth?.secretKey === undefined && !isApiCredentialAuth(auth)) return undefined;
  if (accountCredentialSource(auth, true) === undefined) return undefined;
  try {
    return createPlatformHttpClient(options);
  } catch (error) {
    if (!(error instanceof AreteError) || error.code !== UNTRUSTED_CREDENTIAL_DESTINATION) throw error;
    const apiUrl = apiBase(options.apiUrl);
    return {
      apiUrl,
      request: async () => {
        throw error;
      },
    };
  }
}

/** Create a {@link PlatformHttpClient}. */
export function createPlatformHttpClient(options: PlatformHttpClientOptions = {}): PlatformHttpClient {
  const fetchImpl = resolveFetch(options.fetch);
  const base = apiBase(options.apiUrl);
  const auth = withTrustedApiUrl(resolveAuthConfig(options.auth), options.apiUrl);
  if (auth?.publishableKey !== undefined && auth.secretKey === undefined
    && auth.token === undefined && !auth.getToken && !auth.tokenEndpoint) {
    throw new AreteError(
      'Arete account routes refuse publishable keys; use a secret key on a server or a user session token',
      'INVALID_CONFIG'
    );
  }
  if (auth?.tokenKind === 'session' && (auth.token !== undefined || auth.getToken || auth.tokenEndpoint)) {
    throw new AreteError(
      "Arete API routes take the caller's own credential, not a stack or gateway session; "
        + "set auth.tokenKind to 'api' for a token, getToken or tokenEndpoint that hands one out",
      'INVALID_CONFIG'
    );
  }
  const credential = accountCredentialSource(auth, true);
  if (credential === undefined) {
    throw new AreteError(
      'No Arete credential: pass auth.secretKey, auth.token, auth.getToken or auth.tokenEndpoint',
      'AUTH_REQUIRED'
    );
  }
  // Account credentials go only to trusted Arete API origins.
  assertCredentialDestination(auth, base, credential);
  const source = sharedApiCredential(auth) ?? createApiBearerSource(auth, fetchImpl);
  const refreshable = source.refreshable;

  return {
    apiUrl: base,
    async request<T>(request: PlatformHttpRequest): Promise<PlatformHttpResponse<T>> {
      const path = request.path.startsWith('/') ? request.path : `/${request.path}`;
      const body = request.body === undefined ? undefined : JSON.stringify(request.body);
      let sent: string | undefined;
      const attempt = async (refused?: string) => {
        const token = await source.get(refused);
        sent = token;
        const headers = new Headers({ Accept: 'application/json' });
        if (token) headers.set('Authorization', `Bearer ${token}`);
        if (body !== undefined) headers.set('Content-Type', 'application/json');
        return fetchImpl(`${base}${path}`, {
          method: request.method ?? 'GET',
          headers,
          body,
          signal: request.signal,
        });
      };
      let response = await attempt();
      if (response.status === 401 && refreshable && sent) response = await attempt(sent);
      const parsed = parseBody(await response.text());
      if (!response.ok) {
        const wireErrorCode = response.headers.get('X-Error-Code')
          ?? (isRecord(parsed) && typeof parsed['code'] === 'string' ? parsed['code'] : null);
        const message = isRecord(parsed) && typeof parsed['error'] === 'string'
          ? parsed['error']
          : typeof parsed === 'string' && parsed ? parsed : response.statusText || 'request failed';
        throw new AreteError(
          `${request.method ?? 'GET'} ${path} returned ${response.status}: ${message}`,
          wireErrorCode ? parseErrorCode(wireErrorCode) : 'REQUEST_FAILED',
          {
            status: response.status,
            wireErrorCode,
            body: parsed,
            apiProblem: parseApiProblem(parsed),
          }
        );
      }
      return { status: response.status, body: parsed as T };
    },
  };
}
