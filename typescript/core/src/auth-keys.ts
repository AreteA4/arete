import { isA4LoginKeyDestination, readA4ProfileKey } from './a4-profile';
import type { AuthConfig } from './types';
import { AreteError } from './types';

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

/**
 * True in a browser page: both `window` and `document` exist. Web workers,
 * service workers, edge runtimes, SSR (Node/Bun) and Deno have no `document`
 * and count as server-side.
 */
export function isBrowserEnvironment(): boolean {
  const scope = globalThis as { window?: unknown; document?: unknown };
  return typeof scope.window === 'object'
    && scope.window !== null
    && typeof scope.document === 'object'
    && scope.document !== null;
}

interface DenoEnvironment {
  env?: { get?: (name: string) => string | undefined };
  permissions?: {
    querySync?: (descriptor: { name: 'env'; variable: string }) => { state?: string };
  };
}

/**
 * Read an environment variable in Node, Bun or Deno without prompting for or
 * failing on missing permissions. Browsers have no environment.
 */
function readEnvironmentVariable(name: string): string | undefined {
  try {
    const deno = (globalThis as { Deno?: DenoEnvironment }).Deno;
    if (deno) {
      // Reading without --allow-env throws or prompts; ask first.
      const state = deno.permissions?.querySync?.({ name: 'env', variable: name })?.state;
      if (state !== 'granted') return undefined;
      const value = deno.env?.get?.(name);
      return typeof value === 'string' ? value : undefined;
    }
    const value = (globalThis as { process?: { env?: Record<string, string | undefined> } })
      .process?.env?.[name];
    return typeof value === 'string' ? value : undefined;
  } catch {
    return undefined;
  }
}

const warned = new Set<string>();

/**
 * Marks a resolved config whose `secretKey` came from the `a4` login. It holds
 * that key, so the mark is carried through object spreads (binding paths copy
 * the resolved config) and only applies while `secretKey` is still that key:
 * a copy given another `secretKey` is no longer restricted. Per config, never
 * process-wide, so the same key passed explicitly elsewhere is unaffected.
 */
const A4_LOGIN_SECRET_KEY: unique symbol = Symbol('arete.a4LoginSecretKey');

type MarkedAuthConfig = AuthConfig & { [A4_LOGIN_SECRET_KEY]?: string };

/** True when `auth.secretKey` was supplied by the `a4` login fallback. */
export function secretKeyFromA4Login(auth: AuthConfig | undefined): boolean {
  const marked = auth as MarkedAuthConfig | undefined;
  return marked?.secretKey !== undefined && marked[A4_LOGIN_SECRET_KEY] === marked.secretKey;
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
      return { ...auth, secretKey: environmentKey };
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
    const resolved: MarkedAuthConfig = {
      ...auth,
      secretKey: profile.key,
      [A4_LOGIN_SECRET_KEY]: profile.key,
    };
    return resolved;
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
 * The key sent as the bearer credential to `endpoint`, if any. A key taken
 * from the `a4` login is only sent to the Arete API it was stored for.
 */
export function tokenEndpointApiKey(
  auth: AuthConfig | undefined,
  endpoint: string
): string | undefined {
  if (secretKeyFromA4Login(auth) && !isA4LoginKeyDestination(endpoint)) return undefined;
  return auth?.secretKey ?? auth?.publishableKey;
}

/** True when `headers` carries its own `Authorization` header. */
export function hasAuthorizationHeader(headers: Record<string, string> | undefined): boolean {
  return Object.keys(headers ?? {}).some((name) => name.toLowerCase() === 'authorization');
}
