import { readA4ProfileKey } from './a4-profile';
import { assertCredentialDestination, implicitSecretKeySource, withImplicitSecretKey } from './credential-destination';
import { isBrowserEnvironment, readEnvironmentVariable } from './environment';
import type { AuthConfig } from './types';
import { AreteError } from './types';

export { isBrowserEnvironment, readEnvironmentVariable };

/** Environment variable read for a secret-class key outside browsers. */
export const ARETE_API_KEY_ENV = 'ARETE_API_KEY';

const CREATE_PUBLISHABLE_HINT =
  'create one with `a4 auth keys create-publishable --origin <scheme://host[:port]>`';

/**
 * Key class inferred from an API key's prefix. `unknown` covers keys this SDK
 * does not recognise (for example self-hosted credentials); they are passed
 * through unvalidated.
 */
export type ApiKeyClass = 'publishable' | 'secret' | 'unknown';

const PUBLISHABLE_PREFIXES = ['a4_pk_', 'hspk_'];
const SECRET_PREFIXES = ['a4_sk_', 'a4_ak_', 'hsk_'];

export function classifyApiKey(key: string): ApiKeyClass {
  const trimmed = key.trim();
  if (PUBLISHABLE_PREFIXES.some((prefix) => trimmed.startsWith(prefix))) return 'publishable';
  if (SECRET_PREFIXES.some((prefix) => trimmed.startsWith(prefix))) return 'secret';
  return 'unknown';
}

const warned = new Set<string>();

/** True when `auth.secretKey` was supplied by the `a4` login fallback. */
export function secretKeyFromA4Login(auth: AuthConfig | undefined): boolean {
  return implicitSecretKeySource(auth) === 'a4 login';
}

function warnOnce(id: string, message: string): void {
  if (warned.has(id)) return;
  warned.add(id);
  console.warn(`[arete] ${message}`);
}

/** @internal Reset warn-once state between tests. */
export function resetAuthKeyWarningsForTesting(): void {
  warned.clear();
}

function hasExplicitAuth(auth: AuthConfig | undefined): boolean {
  return auth !== undefined && (
    auth.token !== undefined
    || auth.getToken !== undefined
    || auth.tokenEndpoint !== undefined
    || auth.publishableKey !== undefined
    || auth.secretKey !== undefined
  );
}

/**
 * Validate the configured API keys and apply the server-side credential chain.
 *
 * - `secretKey` is refused in browsers and refuses publishable keys.
 * - A secret-class key in `publishableKey` is refused in browsers and warned
 *   about elsewhere (it still works server-side, as it always has).
 * - Outside browsers, when no auth option is set at all, `ARETE_API_KEY`
 *   supplies `secretKey`; without it, the agent or secret key from the active
 *   `a4` CLI login does. Browsers never read either.
 *
 * Error and warning text never includes key material.
 */
export function resolveAuthConfig(
  auth: AuthConfig | undefined,
  readProfileKey: typeof readA4ProfileKey = readA4ProfileKey
): AuthConfig | undefined {
  const browser = isBrowserEnvironment();

  if (auth?.secretKey !== undefined) {
    if (browser) {
      throw new AreteError(
        'auth.secretKey cannot be used in a browser: anything shipped to a browser is public. '
          + 'Use auth.publishableKey with a publishable key (a4_pk_...) instead; '
          + `${CREATE_PUBLISHABLE_HINT}. Keep secret and agent keys on servers and in local scripts.`,
        'INVALID_CONFIG'
      );
    }
    if (auth.secretKey.trim() === '') {
      throw new AreteError('auth.secretKey is empty', 'INVALID_CONFIG');
    }
    if (classifyApiKey(auth.secretKey) === 'publishable') {
      throw new AreteError(
        'auth.secretKey was given a publishable key (a4_pk_...). Pass it as auth.publishableKey '
          + 'instead; auth.secretKey takes an agent key (a4_ak_...) or secret key (a4_sk_...).',
        'INVALID_CONFIG'
      );
    }
    if (auth.publishableKey !== undefined) {
      throw new AreteError(
        'Set either auth.secretKey (servers and scripts) or auth.publishableKey (browsers), not both.',
        'INVALID_CONFIG'
      );
    }
  }

  if (auth?.publishableKey !== undefined && classifyApiKey(auth.publishableKey) === 'secret') {
    if (browser) {
      throw new AreteError(
        'auth.publishableKey was given a secret-class key (a4_sk_... or a4_ak_...). Never ship a '
          + 'secret or agent key to a browser. Use a publishable key (a4_pk_...) instead; '
          + `${CREATE_PUBLISHABLE_HINT}.`,
        'INVALID_CONFIG'
      );
    }
    warnOnce(
      'secret-in-publishable',
      'auth.publishableKey was given a secret-class key (a4_sk_... or a4_ak_...). '
        + 'Pass it as auth.secretKey instead (servers and scripts only), or set ARETE_API_KEY.'
    );
  }

  if (browser || hasExplicitAuth(auth)) return auth;

  const environmentKey = readEnvironmentVariable(ARETE_API_KEY_ENV)?.trim();
  if (environmentKey) {
    if (classifyApiKey(environmentKey) !== 'publishable') {
      return withImplicitSecretKey(auth, environmentKey, 'ARETE_API_KEY');
    }
    warnOnce(
      'publishable-in-env',
      `${ARETE_API_KEY_ENV} holds a publishable key (a4_pk_...) and was ignored. Set it to an agent `
        + 'key (a4_ak_...) or secret key (a4_sk_...), or pass the publishable key as auth.publishableKey.'
    );
  }

  const profile = readProfileKey();
  if (profile.ambiguous) {
    warnOnce(
      'ambiguous-a4-profile',
      'More than one a4 login profile holds a key; not choosing one. Set ARETE_PROFILE '
        + `(for example \`agent\`) or ${ARETE_API_KEY_ENV}.`
    );
  }
  if (profile.key && classifyApiKey(profile.key) === 'secret') {
    return withImplicitSecretKey(auth, profile.key, 'a4 login');
  }
  return auth;
}

/**
 * Appended to a 401 for a request that carried no API key. It names the
 * commands and options that supply one, never where credentials are stored.
 */
export const NO_API_KEY_HINT =
  'No Arete API key found. Run `a4 auth login` (or `a4 auth signup` for an agent), '
  + 'or set ARETE_API_KEY, or pass auth.secretKey.';

/**
 * The key sent as the bearer credential to `endpoint`, if any. A secret key
 * (or a secret-class key given as `publishableKey`) is an account
 * credential: for an endpoint off the trusted Arete API origins this throws `UNTRUSTED_CREDENTIAL_DESTINATION` rather than send it
 * (see `credential-destination.ts`). A publishable key goes as it is.
 */
export function tokenEndpointApiKey(
  auth: AuthConfig | undefined,
  endpoint: string
): string | undefined {
  if (auth?.secretKey !== undefined) {
    assertCredentialDestination(auth, endpoint, implicitSecretKeySource(auth) ?? 'auth.secretKey');
    return auth.secretKey;
  }
  // A secret-class key given as publishableKey (allowed outside browsers) is
  // still an account credential.
  if (auth?.publishableKey !== undefined && classifyApiKey(auth.publishableKey) === 'secret') {
    assertCredentialDestination(auth, endpoint, 'auth.publishableKey');
  }
  return auth?.publishableKey;
}

/** True when `headers` carries its own `Authorization` header. */
export function hasAuthorizationHeader(headers: Record<string, string> | undefined): boolean {
  return Object.keys(headers ?? {}).some((name) => name.toLowerCase() === 'authorization');
}
