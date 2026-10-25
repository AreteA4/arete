import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { createApiBearerSource, shareApiCredential } from './api-credential';
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

async function gatewayCallsFor(sessionEndpoint: string, auth: Parameters<typeof createHostedSolanaGatewayTransports>[1]['auth']) {
  const { fetch, calls } = gatewayNetwork();
  const transports = createHostedSolanaGatewayTransports(
    {
      chain: gatewayBinding(sessionEndpoint, ['read']),
      transactions: gatewayBinding(sessionEndpoint, ['transaction:inspect', 'transaction:send']),
    },
    { auth, fetch }
  );
  await transports.chain.exists('account');
  return calls;
}

describe('API credential destinations', () => {
  it('sends a saved a4 login only to the Arete API, never to a foreign session endpoint', async () => {
    useSavedLogin();
    const foreign = await gatewayCallsFor(FOREIGN_SESSIONS, { tokenKind: 'api' });
    expect(foreign.map((call) => call.authorization)).toEqual([null, 'Bearer gateway-session']);

    const arete = await gatewayCallsFor(ARETE_SESSIONS, { tokenKind: 'api' });
    expect(arete.map((call) => call.authorization)).toEqual([`Bearer ${SAVED}`, 'Bearer gateway-session']);
  });

  it('keeps the existing rule for explicit and ARETE_API_KEY keys: they go where configured', async () => {
    const explicit = await gatewayCallsFor(FOREIGN_SESSIONS, { tokenKind: 'api', secretKey: 'a4_sk_explicit' });
    expect(explicit[0]!.authorization).toBe('Bearer a4_sk_explicit');

    vi.stubEnv('ARETE_API_KEY', ENV_KEY);
    const environment = await gatewayCallsFor(FOREIGN_SESSIONS, { tokenKind: 'api' });
    expect(environment[0]!.authorization).toBe(`Bearer ${ENV_KEY}`);
  });

  it('hands out a saved login key per destination, even from a shared source', async () => {
    useSavedLogin();
    const fetchMock = vi.fn() as unknown as typeof fetch;
    const auth = shareApiCredential(resolveAuthConfig({ tokenKind: 'api' }), fetchMock);
    const source = createApiBearerSource(auth, fetchMock);
    await expect(source.get(ARETE_SESSIONS)).resolves.toBe(SAVED);
    await expect(source.get('https://api.arete.run.example.test/ws/sessions')).resolves.toBeUndefined();
    await expect(source.get(FOREIGN_SESSIONS)).resolves.toBeUndefined();

    // The platform client built from the session's shared auth refuses a
    // foreign API rather than sending the saved key there.
    expect(() => createPlatformHttpClient({ apiUrl: 'https://api.example.test', auth, fetch: fetchMock }))
      .toThrow('No Arete credential');
    const arete = createPlatformHttpClient({ auth, fetch: fetchMock });
    expect(arete.apiUrl).toBe('https://api.arete.run/api');
    expect(fetchMock).not.toHaveBeenCalled();
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

    await expect(source.get(ARETE_SESSIONS)).resolves.toBe('login-1');
    const refreshing = source.get(ARETE_SESSIONS, 'login-1');
    // Another transport asks while the refresh is running.
    const waiting = source.get(ARETE_SESSIONS);
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

    await source.get(ARETE_SESSIONS);
    // Two refusals of login-1 at once share one refresh.
    await expect(Promise.all([
      source.get(ARETE_SESSIONS, 'login-1'),
      source.get(ARETE_SESSIONS, 'login-1'),
    ])).resolves.toEqual(['login-2', 'login-2']);
    // A refusal of login-1 arriving after the refresh finished.
    await expect(source.get(ARETE_SESSIONS, 'login-1')).resolves.toBe('login-2');
    expect(getToken).toHaveBeenCalledTimes(2);
    expect(getToken.mock.calls.map(([request]) => request)).toEqual([
      { scopes: [], forceRefresh: false },
      { scopes: [], forceRefresh: true },
    ]);
    // Refusing the current credential refreshes it.
    await expect(source.get(ARETE_SESSIONS, 'login-2')).resolves.toBe('login-3');
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
