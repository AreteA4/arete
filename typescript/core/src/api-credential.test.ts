import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { createApiBearerSource, shareApiCredential } from './api-credential';
import { AreteError } from './types';
import { resolveAuthConfig } from './auth-keys';
import { createPlatformHttpClient } from './platform-http';
import { createHostedSolanaGatewayTransports } from './solana-gateway';
import type { HostedSolanaGatewayCapabilityBinding, SolanaGatewayAuthScope } from './types';

const SAVED = 'a4_sk_savedlogin';
const ENV_KEY = 'a4_ak_fromenv';
const ARETE_SESSIONS = 'https://api.arete.run/ws/sessions';
const FOREIGN_SESSIONS = 'https://sessions.example.test/ws/sessions';
const GATEWAY_ID = 'sgb_00000000000000000000000000000007';

let credentialsDir: string;
let savedProfile: string | undefined;

/** An `a4` login on disk, found the way a server finds it: implicitly. */
function useSavedLogin(): void {
  const path = join(credentialsDir, 'credentials.toml');
  writeFileSync(path, `api_key = "${SAVED}"\n`);
  vi.stubEnv('ARETE_CREDENTIALS_PATH', path);
}

beforeEach(() => {
  credentialsDir = mkdtempSync(join(tmpdir(), 'arete-api-credential-'));
  vi.stubEnv('ARETE_API_KEY', '');
  vi.stubEnv('ARETE_CREDENTIALS_PATH', join(credentialsDir, 'missing.toml'));
  savedProfile = process.env['ARETE_PROFILE'];
  delete process.env['ARETE_PROFILE'];
});

afterEach(() => {
  vi.unstubAllEnvs();
  if (savedProfile !== undefined) process.env['ARETE_PROFILE'] = savedProfile;
  rmSync(credentialsDir, { recursive: true, force: true });
});

function gatewayBinding(
  sessionEndpoint: string,
  scopes: readonly SolanaGatewayAuthScope[]
): HostedSolanaGatewayCapabilityBinding {
  return {
    endpoint: 'https://gateway.example.test/',
    authPolicy: 'signed_session',
    solanaGatewayBindingId: GATEWAY_ID,
    cluster: 'mainnet-beta',
    region: 'test',
    auth: {
      required: true,
      mode: 'signed_session',
      sessionEndpoint,
      jwksUrl: 'https://api.arete.run/.well-known/jwks.json',
      tokenTransport: 'bearer',
      audience: 'arete:solana-gateway',
      targetKind: 'solana-gateway-binding',
      targetId: GATEWAY_ID,
      scopes,
      acceptedKeyClasses: ['secret'],
      transactionEntitlementRequired: scopes.some((scope) => scope.startsWith('transaction:')),
    },
  };
}

function gatewayNetwork() {
  const calls: { url: string; authorization: string | null }[] = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    calls.push({ url, authorization: new Headers(init?.headers).get('authorization') });
    if (url.endsWith('/ws/sessions')) {
      return new Response(JSON.stringify({ token: 'gateway-session', scopes: ['read'] }), { status: 200 });
    }
    return new Response(JSON.stringify({ exists: true }), { status: 200 });
  });
  return { fetch: fetchMock as typeof fetch, calls };
}

function gatewayFor(sessionEndpoint: string, auth: Parameters<typeof createHostedSolanaGatewayTransports>[1]['auth']) {
  const { fetch, calls } = gatewayNetwork();
  const transports = createHostedSolanaGatewayTransports(
    {
      chain: gatewayBinding(sessionEndpoint, ['read']),
      transactions: gatewayBinding(sessionEndpoint, ['transaction:inspect', 'transaction:send']),
    },
    { auth, fetch }
  );
  return { transports, calls };
}

async function gatewayCallsFor(sessionEndpoint: string, auth: Parameters<typeof createHostedSolanaGatewayTransports>[1]['auth']) {
  const { transports, calls } = gatewayFor(sessionEndpoint, auth);
  await transports.chain.exists('account');
  return calls;
}

const UNTRUSTED = { code: 'UNTRUSTED_CREDENTIAL_DESTINATION' };

type GatewayAuth = Parameters<typeof createHostedSolanaGatewayTransports>[1]['auth'];

/** Every way to supply an account credential, set up as the app would. */
const ACCOUNT_CREDENTIALS: [string, () => GatewayAuth, string][] = [
  ['an explicit secret key', () => ({ secretKey: 'a4_sk_explicit' }), 'a4_sk_explicit'],
  ['an explicit key with tokenKind api', () => ({ tokenKind: 'api', secretKey: 'a4_ak_explicit' }), 'a4_ak_explicit'],
  ['an api token', () => ({ tokenKind: 'api', token: 'dashboard-login' }), 'dashboard-login'],
  ['an api getToken', () => ({ tokenKind: 'api', getToken: async () => 'provided-login' }), 'provided-login'],
  ['ARETE_API_KEY', () => {
    vi.stubEnv('ARETE_API_KEY', ENV_KEY);
    return undefined;
  }, ENV_KEY],
  ['a saved a4 login', () => {
    useSavedLogin();
    return undefined;
  }, SAVED],
];

describe('account credential destinations', () => {
  it.each(ACCOUNT_CREDENTIALS)('refuses %s for a foreign session endpoint, sending nothing', async (_name, setup) => {
    const auth = setup();
    const { transports, calls } = gatewayFor(FOREIGN_SESSIONS, auth);
    await expect(transports.chain.exists('account')).rejects.toMatchObject(UNTRUSTED);
    expect(calls).toEqual([]);
  });

  it.each(ACCOUNT_CREDENTIALS)('sends %s to the Arete API session endpoint', async (_name, setup, bearer) => {
    const calls = await gatewayCallsFor(ARETE_SESSIONS, setup());
    expect(calls.map((call) => call.authorization)).toEqual([`Bearer ${bearer}`, 'Bearer gateway-session']);
  });

  it('names the destination, the credential and how to allow it', async () => {
    const { transports, calls } = gatewayFor(FOREIGN_SESSIONS, { secretKey: 'a4_sk_explicit' });
    const error = await transports.chain.exists('account').catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(AreteError);
    expect(error).toMatchObject({
      ...UNTRUSTED,
      details: {
        destination: 'https://sessions.example.test',
        credential: 'auth.secretKey',
        trustedOrigins: ['https://api.arete.run'],
      },
    });
    const message = (error as AreteError).message;
    expect(message).toContain('https://sessions.example.test');
    expect(message).toContain('set apiUrl');
    expect(message).not.toContain('a4_sk_explicit');
  });

  it('trusts ARETE_API_URL for explicit and ARETE_API_KEY keys, never for the a4 login', async () => {
    vi.stubEnv('ARETE_API_URL', 'https://sessions.example.test');
    const explicit = await gatewayCallsFor(FOREIGN_SESSIONS, { secretKey: 'a4_sk_explicit' });
    expect(explicit[0]!.authorization).toBe('Bearer a4_sk_explicit');

    vi.stubEnv('ARETE_API_KEY', ENV_KEY);
    const environment = await gatewayCallsFor(FOREIGN_SESSIONS, undefined);
    expect(environment[0]!.authorization).toBe(`Bearer ${ENV_KEY}`);

    vi.stubEnv('ARETE_API_KEY', '');
    useSavedLogin();
    await expect(gatewayCallsFor(FOREIGN_SESSIONS, { tokenKind: 'api' })).rejects.toMatchObject({
      ...UNTRUSTED,
      details: { credential: 'a4 login' },
    });
  });

  it('trusts an explicit apiUrl for an explicit credential only', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ ok: true }), { status: 200 })) as unknown as typeof fetch;
    const explicit = createPlatformHttpClient({ apiUrl: 'http://localhost:3000', auth: { secretKey: 'a4_sk_local' }, fetch: fetchMock });
    await expect(explicit.request({ path: '/account' })).resolves.toEqual({ status: 200, body: { ok: true } });

    vi.stubEnv('ARETE_API_KEY', ENV_KEY);
    expect(() => createPlatformHttpClient({ apiUrl: 'http://localhost:3000', fetch: fetchMock }))
      .toThrow(expect.objectContaining({ ...UNTRUSTED, details: expect.objectContaining({ credential: 'ARETE_API_KEY' }) }));
    vi.stubEnv('ARETE_API_KEY', '');
    useSavedLogin();
    const shared = shareApiCredential(resolveAuthConfig({ tokenKind: 'api' }), fetchMock);
    expect(() => createPlatformHttpClient({ apiUrl: 'https://api.example.test', auth: shared, fetch: fetchMock }))
      .toThrow(expect.objectContaining(UNTRUSTED));
    expect(createPlatformHttpClient({ auth: shared, fetch: fetchMock }).apiUrl).toBe('https://api.arete.run/api');
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('leaves publishable keys and session tokens alone', async () => {
    const publishable = await gatewayCallsFor(FOREIGN_SESSIONS, { publishableKey: 'a4_pk_browser' });
    expect(publishable.map((call) => call.authorization)).toEqual(['Bearer a4_pk_browser', 'Bearer gateway-session']);
    // A secret-class key put in publishableKey is still an account credential.
    await expect(gatewayCallsFor(FOREIGN_SESSIONS, { publishableKey: 'a4_sk_misplaced' })).rejects.toMatchObject({
      ...UNTRUSTED,
      details: { credential: 'auth.publishableKey' },
    });

    const { transports, calls } = gatewayFor(FOREIGN_SESSIONS, { getToken: async () => ({ token: 'minted-session', scopes: ['read'] }) });
    await expect(transports.chain.exists('account')).resolves.toBe(true);
    expect(calls.map((call) => call.authorization)).toEqual(['Bearer minted-session']);
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe('API credential refresh', () => {
  it('drops a refused credential: callers wait for the refresh instead of sending it', async () => {
    const issued: string[] = [];
    const refresh = deferred<string>();
    const getToken = vi.fn(async ({ forceRefresh }: { forceRefresh?: boolean }) => {
      if (!forceRefresh) {
        issued.push('login-1');
        return 'login-1';
      }
      const token = await refresh.promise;
      issued.push(token);
      return token;
    });
    const source = createApiBearerSource({ tokenKind: 'api', getToken }, vi.fn() as unknown as typeof fetch);

    await expect(source.get()).resolves.toBe('login-1');
    const refreshing = source.get('login-1');
    // Another transport asks while the refresh is running.
    const waiting = source.get();
    refresh.resolve('login-2');
    await expect(refreshing).resolves.toBe('login-2');
    await expect(waiting).resolves.toBe('login-2');
    expect(getToken).toHaveBeenCalledTimes(2);
    expect(issued).toEqual(['login-1', 'login-2']);
  });

  it('does not refresh again for a late 401 of a credential already replaced', async () => {
    let issued = 0;
    const getToken = vi.fn(async () => `login-${++issued}`);
    const source = createApiBearerSource({ tokenKind: 'api', getToken }, vi.fn() as unknown as typeof fetch);

    await source.get();
    // Two refusals of login-1 at once share one refresh.
    await expect(Promise.all([
      source.get('login-1'),
      source.get('login-1'),
    ])).resolves.toEqual(['login-2', 'login-2']);
    // A refusal of login-1 arriving after the refresh finished.
    await expect(source.get('login-1')).resolves.toBe('login-2');
    expect(getToken).toHaveBeenCalledTimes(2);
    expect(getToken.mock.calls.map(([request]) => request)).toEqual([
      { scopes: [], forceRefresh: false },
      { scopes: [], forceRefresh: true },
    ]);
    // Refusing the current credential refreshes it.
    await expect(source.get('login-2')).resolves.toBe('login-3');
  });

  it('refreshes once when concurrent platform calls are refused with the same credential', async () => {
    let issued = 0;
    const getToken = vi.fn(async () => `login-${++issued}`);
    const refusedResponses = [deferred<Response>(), deferred<Response>()];
    const bothSent = deferred<void>();
    let refusals = 0;
    const authorizations: (string | null)[] = [];
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      const authorization = new Headers(init?.headers).get('authorization');
      authorizations.push(authorization);
      if (authorization === 'Bearer login-1') {
        const response = refusedResponses[refusals]!.promise;
        refusals += 1;
        if (refusals === 2) bothSent.resolve();
        return response;
      }
      return new Response(JSON.stringify({ ok: true }), { status: 200 });
    }) as unknown as typeof fetch;
    const client = createPlatformHttpClient({ auth: { tokenKind: 'api', getToken }, fetch: fetchMock });
    const refused = () => new Response(JSON.stringify({ error: 'expired' }), { status: 401 });

    const first = client.request({ path: '/account/a' });
    const second = client.request({ path: '/account/b' });
    await bothSent.promise;
    // The first refusal is handled (and the credential refreshed) before the
    // second one arrives.
    refusedResponses[0]!.resolve(refused());
    await expect(first).resolves.toEqual({ status: 200, body: { ok: true } });
    refusedResponses[1]!.resolve(refused());
    await expect(second).resolves.toEqual({ status: 200, body: { ok: true } });

    expect(getToken).toHaveBeenCalledTimes(2);
    expect(authorizations).toEqual(['Bearer login-1', 'Bearer login-1', 'Bearer login-2', 'Bearer login-2']);
  });
});
