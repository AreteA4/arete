/**
 * The caller's own Arete API credential, for `auth.tokenKind: 'api'`.
 *
 * With `tokenKind: 'api'`, `token`, `getToken` and `tokenEndpoint` hand out a
 * credential for the Arete API itself (for example the bearer of a user
 * signed in to an Arete dashboard), not a session for a stack, gateway or
 * protocol endpoint. The SDK sends that credential only to Arete API routes
 * and to the session endpoints that mint the binding-scoped sessions it
 * needs; it never sends it to a stack, gateway or program read endpoint.
 * Like every account credential, it goes only to trusted Arete API origins:
 * callers check the destination with `assertCredentialDestination` (see
 * `credential-destination.ts`) before asking this source for it.
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
  /**
   * The bearer, or `undefined` when the auth carries no API credential.
   * Callers check where it is going first (`assertCredentialDestination`),
   * so an untrusted destination never even fetches it.
   *
   * After a 401, pass the credential that request actually sent as
   * `refused`: if it is still the current one it is dropped and a fresh one
   * fetched (once, however many requests were refused with it); if it was
   * already replaced, the replacement is returned without another fetch.
   * While a fetch is running every caller waits for it.
   */
  get(refused?: string): Promise<string | undefined>;
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
    // Set when the current credential was refused: the provider's own cached
    // credential is stale too, so the next fetch forces a refresh.
    let refusedCurrent = false;
    const mint = async (forceRefresh: boolean): Promise<string> => {
      let result: AuthTokenResult;
      if (auth.getToken) {
        result = normalize(await auth.getToken({ scopes: [], forceRefresh }), 'auth.getToken');
      } else {
        const response = await fetchImpl(auth.tokenEndpoint!, {
          method: 'POST',
          headers: { ...(auth.tokenEndpointHeaders ?? {}), 'Content-Type': 'application/json' },
          credentials: auth.tokenEndpointCredentials,
          body: JSON.stringify({ scopes: [], forceRefresh }),
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
      refusedCurrent = false;
      return result.token;
    };
    return {
      refreshable: true,
      async get(refused) {
        if (refused !== undefined && cached?.token === refused) {
          // Drop it now, so no other request sends it while the fresh one is
          // fetched.
          cached = undefined;
          refusedCurrent = true;
        }
        // A fetch is running (first use, expiry or a refusal): wait for it
        // rather than send what it replaces.
        if (inFlight) return inFlight;
        const fresh = cached
          && (cached.expiresAt === undefined || cached.expiresAt - REFRESH_BUFFER_MS > Date.now());
        if (fresh) return cached!.token;
        // A provider's cached credential is stale once ours was refused or
        // expired.
        const stale = refusedCurrent || cached !== undefined;
        inFlight = mint(stale).finally(() => {
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

/** The credential source a session shares with every client it builds. */
const SHARED_API_CREDENTIAL: unique symbol = Symbol('arete.sharedApiCredential');

type SharedAuthConfig = AuthConfig & { [SHARED_API_CREDENTIAL]?: ApiBearerSource };

/** The shared source {@link shareApiCredential} attached to `auth`, if any. */
export function sharedApiCredential(auth: AuthConfig | undefined): ApiBearerSource | undefined {
  return (auth as SharedAuthConfig | undefined)?.[SHARED_API_CREDENTIAL];
}

/**
 * `auth` carrying one {@link ApiBearerSource} for its API credential
 * (`tokenKind: 'api'`), so every connection, transport and API client built
 * from it fetches the credential once per expiry and refreshes it once
 * after a refusal. Other auth is returned unchanged. Idempotent; the source
 * survives object spreads.
 */
export function shareApiCredential(auth: AuthConfig | undefined, fetchImpl: typeof fetch): AuthConfig | undefined {
  if (!isApiCredentialAuth(auth) || sharedApiCredential(auth)) return auth;
  const shared: SharedAuthConfig = { ...auth, [SHARED_API_CREDENTIAL]: createApiBearerSource(auth, fetchImpl) };
  return shared;
}
