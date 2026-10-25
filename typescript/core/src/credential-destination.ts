/**
 * Where an Arete account credential may be sent.
 *
 * An account credential is the caller's own Arete API credential: a secret
 * or agent key (`auth.secretKey`, `ARETE_API_KEY` or the `a4` login) or a
 * token handed out with `auth.tokenKind: 'api'`. It is sent only to trusted
 * Arete API origins:
 *
 * - `https://api.arete.run`, always;
 * - the origin of `ARETE_API_URL` (outside browsers), except for the `a4`
 *   login key, which is stored for `https://api.arete.run` alone;
 * - for a credential the app passed in code, the origin of an `apiUrl` the
 *   app configured in code.
 *
 * Nothing a stack, binding or generated descriptor names becomes trusted by
 * being named, so a session endpoint on another origin is refused with
 * `UNTRUSTED_CREDENTIAL_DESTINATION` before anything is sent. Publishable keys
 * and Arete session tokens are not account credentials and are unaffected.
 */

import { DEFAULT_A4_API_URL } from './a4-profile';
import { isBrowserEnvironment, readEnvironmentVariable } from './environment';
import { AreteError, type AuthConfig } from './types';

/** `AreteError.code` when an account credential would go to an untrusted origin. */
export const UNTRUSTED_CREDENTIAL_DESTINATION = 'UNTRUSTED_CREDENTIAL_DESTINATION';

/** Environment variable read (outside browsers) for the Arete API origin. */
export const ARETE_API_URL_ENV = 'ARETE_API_URL';

/** Where an account credential came from, as named in errors. */
export type AccountCredentialSource =
  | 'auth.secretKey'
  | 'auth.publishableKey'
  | 'ARETE_API_KEY'
  | 'a4 login'
  | 'auth.token'
  | 'auth.getToken'
  | 'auth.tokenEndpoint';

/** Shape of {@link AreteError.details} for `UNTRUSTED_CREDENTIAL_DESTINATION`. */
export interface UntrustedCredentialDestinationDetails {
  /** The refused destination's origin (`undefined` when it is not a usable URL). */
  readonly destination: string | undefined;
  readonly credential: AccountCredentialSource;
  /** The origins this credential may be sent to. */
  readonly trustedOrigins: readonly string[];
}

/**
 * `url`'s origin as `scheme://host[:port]`: lowercase host without trailing
 * dots, default ports dropped. `undefined` for anything but an http(s) URL
 * without user info.
 */
export function credentialOrigin(url: string): string | undefined {
  try {
    const parsed = new URL(url.trim());
    if (parsed.protocol !== 'https:' && parsed.protocol !== 'http:') return undefined;
    if (parsed.username !== '' || parsed.password !== '') return undefined;
    const host = parsed.hostname.toLowerCase().replace(/\.+$/, '');
    if (!host) return undefined;
    return `${parsed.protocol}//${host}${parsed.port ? `:${parsed.port}` : ''}`;
  } catch {
    return undefined;
  }
}

const DEFAULT_API_ORIGIN = credentialOrigin(DEFAULT_A4_API_URL)!;

/**
 * Marks a resolved config whose `secretKey` the SDK found itself, and where.
 * Carried through object spreads (binding paths copy the resolved config);
 * applies only while `secretKey` is still that key, so a copy given another
 * key counts as explicit. Per config, never process-wide.
 */
const IMPLICIT_SECRET_KEY: unique symbol = Symbol('arete.implicitSecretKey');

/** The explicit `apiUrl` origins the app configured alongside this auth. */
const TRUSTED_API_ORIGINS: unique symbol = Symbol('arete.trustedApiOrigins');

type MarkedAuthConfig = AuthConfig & {
  [IMPLICIT_SECRET_KEY]?: { key: string; source: 'ARETE_API_KEY' | 'a4 login' };
  [TRUSTED_API_ORIGINS]?: readonly string[];
};

/** `auth` with `secretKey` set to a key found in `ARETE_API_KEY` or the `a4` login. */
export function withImplicitSecretKey(
  auth: AuthConfig | undefined,
  key: string,
  source: 'ARETE_API_KEY' | 'a4 login'
): AuthConfig {
  const marked: MarkedAuthConfig = { ...auth, secretKey: key, [IMPLICIT_SECRET_KEY]: { key, source } };
  return marked;
}

/** Where `auth.secretKey` was found, when the SDK found it rather than the app passing it. */
export function implicitSecretKeySource(
  auth: AuthConfig | undefined
): 'ARETE_API_KEY' | 'a4 login' | undefined {
  const mark = (auth as MarkedAuthConfig | undefined)?.[IMPLICIT_SECRET_KEY];
  return mark !== undefined && mark.key === auth?.secretKey ? mark.source : undefined;
}

/**
 * `auth` trusting the origin of `apiUrl`, an Arete API URL the app configured
 * in code (for a local, development or self-hosted API). Only credentials the
 * app passed itself are sent there. Never call this with a URL taken from a
 * stack, binding or generated descriptor. Unchanged for no auth or no URL.
 */
export function withTrustedApiUrl(auth: AuthConfig | undefined, apiUrl: string | undefined): AuthConfig | undefined {
  if (!auth || !apiUrl?.trim()) return auth;
  const origin = credentialOrigin(apiUrl);
  const existing = (auth as MarkedAuthConfig)[TRUSTED_API_ORIGINS] ?? [];
  if (!origin || existing.includes(origin)) return auth;
  const marked: MarkedAuthConfig = { ...auth, [TRUSTED_API_ORIGINS]: [...existing, origin] };
  return marked;
}

function environmentApiOrigin(): string | undefined {
  if (isBrowserEnvironment()) return undefined;
  const value = readEnvironmentVariable(ARETE_API_URL_ENV)?.trim();
  return value ? credentialOrigin(value) : undefined;
}

/**
 * The account credential `auth` carries. With `includeTokens`, `token`,
 * `getToken` and `tokenEndpoint` count (they hand out an API credential);
 * otherwise only a secret key does.
 */
export function accountCredentialSource(
  auth: AuthConfig | undefined,
  includeTokens: boolean
): AccountCredentialSource | undefined {
  if (includeTokens) {
    if (auth?.token !== undefined) return 'auth.token';
    if (auth?.getToken) return 'auth.getToken';
    if (auth?.tokenEndpoint) return 'auth.tokenEndpoint';
  }
  if (auth?.secretKey === undefined) return undefined;
  return implicitSecretKeySource(auth) ?? 'auth.secretKey';
}

/** The origins the credential `source` from `auth` may be sent to. */
export function trustedCredentialOrigins(
  auth: AuthConfig | undefined,
  source: AccountCredentialSource
): string[] {
  const origins = [DEFAULT_API_ORIGIN];
  const add = (origin: string | undefined) => {
    if (origin && !origins.includes(origin)) origins.push(origin);
  };
  // The a4 login key is stored for the default API and goes nowhere else.
  if (source === 'a4 login') return origins;
  add(environmentApiOrigin());
  if (source === 'ARETE_API_KEY') return origins;
  for (const origin of (auth as MarkedAuthConfig | undefined)?.[TRUSTED_API_ORIGINS] ?? []) add(origin);
  return origins;
}

function allowHint(source: AccountCredentialSource, destination: string | undefined): string {
  if (source === 'a4 login') {
    return 'The a4 login key is stored for https://api.arete.run and is sent nowhere else. '
      + 'To use another Arete API, pass its key as auth.secretKey and set apiUrl to its origin.';
  }
  const origin = destination ?? 'that origin';
  if (source === 'ARETE_API_KEY') {
    return `If ${origin} is your Arete API, set ARETE_API_URL to it, or pass the key as `
      + 'auth.secretKey and set apiUrl to that origin.';
  }
  return `If ${origin} is your Arete API, set apiUrl to it in code (or ARETE_API_URL). `
    + 'A session endpoint named by a stack or binding is not trusted for being named.';
}

/**
 * Throw `UNTRUSTED_CREDENTIAL_DESTINATION` unless the account credential
 * `source` from `auth` may be sent to `destination`. Messages never carry key
 * material.
 */
export function assertCredentialDestination(
  auth: AuthConfig | undefined,
  destination: string,
  source: AccountCredentialSource
): void {
  const origin = credentialOrigin(destination);
  const trustedOrigins = trustedCredentialOrigins(auth, source);
  if (origin !== undefined && trustedOrigins.includes(origin)) return;
  const details: UntrustedCredentialDestinationDetails = { destination: origin, credential: source, trustedOrigins };
  throw new AreteError(
    `Refusing to send the Arete account credential from ${source} to ${origin ?? 'a URL that is not a plain http(s) origin'}: `
      + `account credentials go only to trusted Arete API origins (${trustedOrigins.join(', ')}). `
      + allowHint(source, origin),
    UNTRUSTED_CREDENTIAL_DESTINATION,
    details
  );
}
