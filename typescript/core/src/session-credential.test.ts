import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';

import { createSession } from './session';
import { withProgramRead } from './program-sdk';
import { extendProgram } from './stack-extensions';
import { programAccountRead } from './read';
import type { HostedSolanaGatewayCapabilityBinding, SolanaGatewayAuthScope } from './types';

const PROGRAM = 'SQDS111111111111111111111111111111111111111';
const READ_BINDING = 'prb_00000000000000000000000000000009';
const GATEWAY_ID = 'sgb_00000000000000000000000000000009';
const SESSIONS = 'https://api.example.test/ws/sessions';
const APP_TOKEN = 'https://app.example.test/api/arete-token';

function gatewayBinding(scopes: readonly SolanaGatewayAuthScope[]): HostedSolanaGatewayCapabilityBinding {
  return {
    endpoint: 'https://gateway.example.test/',
    authPolicy: 'signed_session',
    solanaGatewayBindingId: GATEWAY_ID,
    cluster: 'mainnet-beta',
    region: 'test',
    auth: {
      required: true,
      mode: 'signed_session',
      sessionEndpoint: SESSIONS,
      jwksUrl: 'https://api.example.test/.well-known/jwks.json',
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

const program = withProgramRead(
  extendProgram(
    {
      name: 'squads',
      programId: PROGRAM,
      programSpecHash: 'spec-squads',
      accounts: { Multisig: programAccountRead<{ threshold: number }>({ account: 'Multisig' }) },
      rawInstructions: {},
      gateway: {
        chain: gatewayBinding(['read']),
        transactions: gatewayBinding(['transaction:inspect', 'transaction:send']),
      },
    },
    {
      createRead(context) {
        return {
          // What an SDK extension does: account API calls through the
          // client's platform client, chain and program reads as usual.
          overview: () => {
            if (!context.platform) throw new Error('no platform client');
            return context.platform.request({ path: '/account/squads' });
          },
          multisig: (address: string) => context.program.accounts.Multisig.fetch(address),
          exists: (address: string) => context.chain.exists(address),
        };
      },
    }
  ),
  {
    release: { programReleaseHash: 'release-squads', programSpecHash: 'spec-squads' },
    transport: {
      kind: 'hosted-binding',
      binding: {
        endpoint: 'https://reads.example.test',
        programReadBindingId: READ_BINDING,
        auth: {
          required: true,
          sessionEndpoint: SESSIONS,
          targetKind: 'program-read-binding',
          targetId: READ_BINDING,
        },
      },
    },
  }
);

function fakeNetwork() {
  const calls: { url: string; authorization: string | null; body?: Record<string, unknown> }[] = [];
  let logins = 0;
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const authorization = new Headers(init?.headers).get('authorization');
    const body = init?.body ? (JSON.parse(String(init.body)) as Record<string, unknown>) : undefined;
    calls.push({ url, authorization, ...(body ? { body } : {}) });
    const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status });
    if (url === APP_TOKEN) {
      logins += 1;
      return json({ token: `login-${logins}`, expiresAt: Math.floor(Date.now() / 1000) + 600 });
    }
    if (url === SESSIONS) {
      const scopes = body!['scopes'] as string[];
      return json({ token: `${body!['targetKind']}:${scopes.join('+')}`, scopes });
    }
    if (url === 'https://api.example.test/api/account/squads') return json({ setup: 'ready' });
    if (url.startsWith('https://reads.example.test')) return json({ threshold: 2 });
    if (url.startsWith('https://gateway.example.test/chain/exists/')) return json({ exists: true });
    return json({}, 404);
  });
  return { fetch: fetchMock as typeof fetch, calls };
}

describe('session API credential', () => {
  it('fetches the app credential once for the platform client, program reads and the gateway', async () => {
    const { fetch, calls } = fakeNetwork();
    const session = await createSession(
      { programs: { squads: program } },
      {
        fetch,
        apiUrl: 'https://api.example.test',
        auth: { tokenKind: 'api', tokenEndpoint: APP_TOKEN, tokenEndpointCredentials: 'include' },
      }
    );

    await expect(session.programs.squads.read.overview()).resolves.toEqual({ status: 200, body: { setup: 'ready' } });
    await expect(session.programs.squads.read.multisig('address')).resolves.toEqual({ threshold: 2 });
    await expect(session.programs.squads.read.exists('address')).resolves.toBe(true);
    expect(session.platform).toBeDefined();

    expect(calls.filter((call) => call.url === APP_TOKEN)).toHaveLength(1);
    const byUrl = (prefix: string) => calls.filter((call) => call.url.startsWith(prefix));
    expect(byUrl('https://api.example.test/api/')[0]!.authorization).toBe('Bearer login-1');
    expect(byUrl(SESSIONS).map((call) => [call.authorization, call.body!['targetKind']])).toEqual([
      ['Bearer login-1', 'program-read-binding'],
      ['Bearer login-1', 'solana-gateway-binding'],
    ]);
    expect(byUrl('https://reads.example.test')[0]!.authorization).toBe('Bearer program-read-binding:read');
    expect(byUrl('https://gateway.example.test')[0]!.authorization).toBe('Bearer solana-gateway-binding:read');
    session.close();
  });

  it('gives extensions no platform client on a publishable key', async () => {
    const { fetch } = fakeNetwork();
    const session = await createSession(
      { programs: { squads: program } },
      { fetch, apiUrl: 'https://api.example.test', auth: { publishableKey: 'a4_pk_visitor' } }
    );
    expect(session.platform).toBeUndefined();
    expect(() => session.programs.squads.read.overview()).toThrow('no platform client');
    session.close();
  });

  it('gives extensions no platform client without an API credential', async () => {
    const { fetch } = fakeNetwork();
    const session = await createSession({ programs: { squads: program } }, { fetch });
    expect(session.platform).toBeUndefined();
    expect(() => session.programs.squads.read.overview()).toThrow('no platform client');
    session.close();
  });
});

describe('session credential fallbacks', () => {
  let credentialsDir: string | undefined;
  let savedProfile: string | undefined;

  /** An `a4` login on disk, found implicitly as on a developer's machine. */
  function useSavedLogin(key: string): void {
    credentialsDir = mkdtempSync(join(tmpdir(), 'arete-session-credential-'));
    const path = join(credentialsDir, 'credentials.toml');
    writeFileSync(path, `api_key = "${key}"\n`);
    vi.stubEnv('ARETE_API_KEY', '');
    vi.stubEnv('ARETE_CREDENTIALS_PATH', path);
    savedProfile = process.env['ARETE_PROFILE'];
    delete process.env['ARETE_PROFILE'];
  }

  afterEach(() => {
    vi.unstubAllEnvs();
    if (savedProfile !== undefined) process.env['ARETE_PROFILE'] = savedProfile;
    savedProfile = undefined;
    if (credentialsDir) rmSync(credentialsDir, { recursive: true, force: true });
    credentialsDir = undefined;
  });

  it('builds the session platform client with options omitted, as with {}', async () => {
    vi.stubEnv('ARETE_API_KEY', 'a4_sk_fromenv');
    const omitted = await createSession({ programs: { squads: program } });
    const empty = await createSession({ programs: { squads: program } }, {});
    expect(omitted.platform?.apiUrl).toBe('https://api.arete.run/api');
    expect(empty.platform?.apiUrl).toBe(omitted.platform?.apiUrl);
    omitted.close();
    empty.close();
  });

  it('refuses a saved a4 login for session endpoints and an API on another host', async () => {
    const saved = 'a4_sk_savedlogin';
    useSavedLogin(saved);
    const { fetch, calls } = fakeNetwork();
    const session = await createSession(
      { programs: { squads: program } },
      { fetch, apiUrl: 'https://api.example.test', auth: { tokenKind: 'api' } }
    );

    // The saved key is for the Arete API, not api.example.test: an explicit
    // apiUrl trusts only credentials the app passed itself.
    const untrusted = { code: 'UNTRUSTED_CREDENTIAL_DESTINATION', details: { credential: 'a4 login' } };
    expect(session.platform?.apiUrl).toBe('https://api.example.test/api');
    await expect(session.programs.squads.read.overview()).rejects.toMatchObject({
      ...untrusted,
      details: { ...untrusted.details, destination: 'https://api.example.test' },
    });
    await expect(session.programs.squads.read.multisig('address')).rejects.toMatchObject(untrusted);
    await expect(session.programs.squads.read.exists('address')).rejects.toMatchObject(untrusted);
    expect(calls).toEqual([]);
    session.close();
  });

  it('does not trust a binding session endpoint because apiUrl is explicit', async () => {
    const { fetch, calls } = fakeNetwork();
    const session = await createSession(
      { programs: { squads: program } },
      { fetch, apiUrl: 'https://api.other.test', auth: { secretKey: 'a4_sk_explicit' } }
    );
    // The bindings name api.example.test; only api.other.test was configured.
    await expect(session.programs.squads.read.multisig('address')).rejects.toMatchObject({
      code: 'UNTRUSTED_CREDENTIAL_DESTINATION',
      details: { destination: 'https://api.example.test', credential: 'auth.secretKey' },
    });
    expect(calls).toEqual([]);
    session.close();
  });

  it('sends an explicit key to binding session endpoints on the explicit apiUrl origin', async () => {
    const { fetch, calls } = fakeNetwork();
    const session = await createSession(
      { programs: { squads: program } },
      { fetch, apiUrl: 'https://api.example.test', auth: { secretKey: 'a4_sk_explicit' } }
    );
    await expect(session.programs.squads.read.multisig('address')).resolves.toEqual({ threshold: 2 });
    expect(calls.filter((call) => call.url === SESSIONS).map((call) => call.authorization))
      .toEqual(['Bearer a4_sk_explicit']);
    session.close();
  });
});
