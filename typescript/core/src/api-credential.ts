/**
 * The caller's own Arete API credential, for `auth.tokenKind: 'api'`.
 *
 * With `tokenKind: 'api'`, `token`, `getToken` and `tokenEndpoint` hand out a
 * credential for the Arete API itself (for example the bearer of a user
 * signed in to an Arete dashboard), not a session for a stack, gateway or
 * protocol endpoint. The SDK sends that credential only to Arete API routes
 * and to the session endpoints that mint the binding-scoped sessions it
 * needs; it never sends it to a stack, gateway or program read endpoint.
 */

import { AreteError, parseApiProblem, parseErrorCode, type AuthConfig, type AuthTokenResult } from './types';

/** `auth.tokenKind` when it is `'api'`. */
export function isApiCredentialAuth(auth: AuthConfig | undefined): boolean {
  return auth?.tokenKind === 'api';
}

/** Where a session minted from an API credential is requested. */
const SESSION_MINT_ENDPOINT: unique symbol = Symbol('arete.sessionMintEndpoint');

type MintAuthConfig = AuthConfig & { [SESSION_MINT_ENDPOINT]?: string };

/**
 * `auth` with the session endpoint a binding's sessions are minted at. Kept
 * beside the app's own `tokenEndpoint`, which with `tokenKind: 'api'`
 * returns the API credential, not a session. Survives object spreads.
 */
export function withSessionMintEndpoint(auth: AuthConfig, endpoint: string): AuthConfig {
  const marked: MintAuthConfig = { ...auth, [SESSION_MINT_ENDPOINT]: endpoint };
  return marked;
}

/** The session endpoint {@link withSessionMintEndpoint} attached, if any. */
export function sessionMintEndpoint(auth: AuthConfig | undefined): string | undefined {
  return (auth as MintAuthConfig | undefined)?.[SESSION_MINT_ENDPOINT];
}

export interface ApiBearerSource {
  /** Whether a fresh credential can be fetched (after a 401). */
  readonly refreshable: boolean;
  /** The bearer, or `undefined` when the auth carries no API credential. */
  get(forceRefresh?: boolean): Promise<string | undefined>;
}

const REFRESH_BUFFER_MS = 30_000;

function decodeBase64Url(value: string): string | undefined {
  const normalized = value.replace(/-/g, '+').replace(/_/g, '/');
  const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, '=');
  if (typeof atob === 'function') return atob(padded);
  return undefined;
}

/** Expiry in milliseconds since the epoch: from the result, else the JWT. */
function expiryOf(result: AuthTokenResult): number | undefined {
  const seconds = result.expiresAt ?? result.expires_at;
  if (typeof seconds === 'number' && Number.isFinite(seconds)) {
    return seconds < 1e12 ? seconds * 1000 : seconds;
  }
  const parts = result.token.split('.');
  if (parts.length !== 3) return undefined;
  try {
    const payload = JSON.parse(decodeBase64Url(parts[1] ?? '') ?? '') as { exp?: unknown };
    return typeof payload.exp === 'number' ? payload.exp * 1000 : undefined;
  } catch {
    return undefined;
  }
}

function normalize(result: unknown, what: string): AuthTokenResult {
  if (typeof result === 'string' && result) return { token: result };
  if (typeof result === 'object' && result !== null) {
    const token = (result as { token?: unknown }).token;
    if (typeof token === 'string' && token) return result as AuthTokenResult;
  }
  throw new AreteError(`${what} returned no token`, 'TOKEN_INVALID');
}

/**
 * The caller's API credential from `auth`: `token` as it is, `getToken`
 * called with no target, `tokenEndpoint` POSTed `{"scopes": []}`, or
 * `secretKey`. Fetched credentials are cached until shortly before they
 * expire.
 */
export function createApiBearerSource(
  auth: AuthConfig | undefined,
  fetchImpl: typeof fetch
): ApiBearerSource {
  if (auth?.token !== undefined) {
    if (!auth.token) throw new AreteError('Authentication token is empty', 'TOKEN_INVALID');
    const token = auth.token;
    return { refreshable: false, get: async () => token };
  }
  if (auth?.getToken || auth?.tokenEndpoint) {
    let cached: { token: string; expiresAt?: number } | undefined;
    let inFlight: Promise<string> | undefined;
    const mint = async (): Promise<string> => {
      let result: AuthTokenResult;
      if (auth.getToken) {
        result = normalize(await auth.getToken({ scopes: [] }), 'auth.getToken');
      } else {
        const response = await fetchImpl(auth.tokenEndpoint!, {
          method: 'POST',
          headers: { ...(auth.tokenEndpointHeaders ?? {}), 'Content-Type': 'application/json' },
          credentials: auth.tokenEndpointCredentials,
          body: JSON.stringify({ scopes: [] }),
        });
        const text = await response.text();
        let body: unknown = text;
        try {
          body = text ? JSON.parse(text) : undefined;
        } catch {
          // Plain-text body.
        }
        if (!response.ok) {
          const problem = parseApiProblem(body);
          throw new AreteError(
            `Token endpoint returned ${response.status}: ${problem?.error ?? (text || response.statusText)}`,
            problem?.code ? parseErrorCode(problem.code) : 'AUTH_REQUIRED',
            { status: response.status, apiProblem: problem }
          );
        }
        result = normalize(body, 'Token endpoint');
      }
      cached = { token: result.token, expiresAt: expiryOf(result) };
      return result.token;
    };
    return {
      refreshable: true,
      async get(forceRefresh = false) {
        const fresh = cached
          && (cached.expiresAt === undefined || cached.expiresAt - REFRESH_BUFFER_MS > Date.now());
        if (!forceRefresh && fresh) return cached!.token;
        inFlight ??= mint().finally(() => {
          inFlight = undefined;
        });
        return inFlight;
      },
    };
  }
  if (auth?.secretKey !== undefined) {
    const key = auth.secretKey;
    return { refreshable: false, get: async () => key };
  }
  return { refreshable: false, get: async () => undefined };
}
