import { describe, expect, it, vi } from 'vitest';
import { createHostedSolanaGatewayTransports } from './solana-gateway';
import { Arete } from './client';
import { createSession } from './session';
import type {
  AuthTokenRequest,
  HostedSolanaGatewayCapabilityBinding,
  SolanaGatewayAuthScope,
} from './types';

const GATEWAY_ID = 'sgb_00000000000000000000000000000001';
const ENDPOINT = 'https://solana.example.test/gateway/';

function binding(
  scopes: readonly SolanaGatewayAuthScope[],
  transactionEntitlementRequired: boolean
): HostedSolanaGatewayCapabilityBinding {
  return {
    endpoint: ENDPOINT,
    authPolicy: 'signed_session',
    solanaGatewayBindingId: GATEWAY_ID,
    cluster: 'mainnet-beta',
    region: 'us-west-1',
    auth: {
      required: true,
      mode: 'signed_session',
      sessionEndpoint: 'https://api.arete.run/ws/sessions',
      jwksUrl: 'https://api.example.test/.well-known/jwks.json',
      tokenTransport: 'bearer',
      audience: 'arete:solana-gateway',
      targetKind: 'solana-gateway-binding',
      targetId: GATEWAY_ID,
      scopes,
      acceptedKeyClasses: transactionEntitlementRequired
        ? ['publishable', 'secret']
        : ['anonymous', 'publishable', 'secret'],
      transactionEntitlementRequired,
    },
  };
}

const BINDINGS = {
  chain: binding(['read'], false),
  transactions: binding(['transaction:inspect', 'transaction:send'], true),
} as const;

describe('hosted Solana gateway transports', () => {
  it('is selected automatically by a generated hosted stack', async () => {
    const urls: string[] = [];
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      urls.push(url);
      if (url.endsWith('/chain/exists/account')) {
        return new Response(JSON.stringify({ exists: true }), { status: 200 });
      }
      if (url.endsWith('/transactions/v1/latest-blockhash')) {
        return new Response(JSON.stringify({
          blockhash: 'blockhash',
          contextSlot: '42',
          lastValidBlockHeight: '99',
        }), { status: 200 });
      }
      throw new Error(`Unexpected request: ${url}`);
    });
    const stack = {
      name: 'hosted',
      endpoints: { ws: '', http: 'https://tenant.stack.arete.run' },
      views: {},
      gateway: BINDINGS,
    } as const;
    const client = await Arete.connect(stack, {
      transport: 'http',
      auth: {
        getToken: async (request) => ({ token: 'gateway-token', scopes: request.scopes }),
      },
      fetch: fetchMock as typeof fetch,
    });

    await expect(client.chain.exists('account')).resolves.toBe(true);
    await expect(client.transactions.getLatestBlockhash()).resolves.toMatchObject({
      blockhash: 'blockhash',
    });
    expect(urls).toEqual([
      `${ENDPOINT}chain/exists/account`,
      `${ENDPOINT}transactions/v1/latest-blockhash`,
    ]);
    expect(urls.every((url) => !url.includes('tenant.stack.arete.run'))).toBe(true);
  });

  it('is selected automatically by a generated hosted composition', async () => {
    const urls: string[] = [];
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      urls.push(url);
      if (url.endsWith('/chain/exists/account')) {
        return new Response(JSON.stringify({ exists: true }), { status: 200 });
      }
      if (url.endsWith('/transactions/v1/latest-blockhash')) {
        return new Response(JSON.stringify({
          blockhash: 'blockhash',
          contextSlot: '42',
          lastValidBlockHeight: '99',
        }), { status: 200 });
      }
      throw new Error(`Unexpected request: ${url}`);
    });
    const definition = {
      mode: 'composition',
      gateway: BINDINGS,
      stacks: {
        hosted: {
          name: 'hosted',
          endpoints: { ws: '', http: 'https://tenant.stack.arete.run' },
          views: {},
          gateway: BINDINGS,
        },
      },
    } as const;
    const session = await createSession(definition, {
      auth: {
        getToken: async (request) => ({ token: 'gateway-token', scopes: request.scopes }),
      },
      fetch: fetchMock as typeof fetch,
      stacks: { hosted: { transport: 'http' } },
    });

    await expect(session.chain.exists('account')).resolves.toBe(true);
    await expect(session.transactions.getLatestBlockhash()).resolves.toMatchObject({
      blockhash: 'blockhash',
    });
    expect(urls.every((url) => !url.includes('tenant.stack.arete.run'))).toBe(true);
    session.close();
  });

  it('shares one endpoint and binding while isolating exact target tokens per scope', async () => {
    const tokenRequests: AuthTokenRequest[] = [];
    const gatewayRequests: Array<{ url: string; authorization: string | null }> = [];
    const getToken = vi.fn(async (request?: AuthTokenRequest) => {
      tokenRequests.push(request!);
      return { token: `token-${request!.scopes.join('+')}`, scopes: request!.scopes };
    });
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      gatewayRequests.push({
        url,
        authorization: new Headers(init?.headers).get('authorization'),
      });
      if (url.endsWith('/chain/exists/account')) {
        return new Response(JSON.stringify({ exists: true }), { status: 200 });
      }
      if (url.endsWith('/transactions/v1/latest-blockhash')) {
        return new Response(JSON.stringify({
          blockhash: 'blockhash',
          contextSlot: '42',
          lastValidBlockHeight: '99',
        }), { status: 200 });
      }
      if (url.endsWith('/transactions/v1/send')) {
        return new Response(JSON.stringify({ signature: 'signature' }), { status: 200 });
      }
      throw new Error(`Unexpected gateway request: ${url}`);
    });
    const transports = createHostedSolanaGatewayTransports(BINDINGS, {
      auth: { getToken },
      fetch: fetchMock as typeof fetch,
    });

    await expect(transports.chain.exists('account')).resolves.toBe(true);
    await expect(transports.chain.exists('account')).resolves.toBe(true);
    await expect(transports.transactions.getLatestBlockhash()).resolves.toMatchObject({
      blockhash: 'blockhash',
      contextSlot: 42n,
    });
    await transports.transactions.getLatestBlockhash();
    await expect(transports.transactions.sendTransaction('signed'))
      .resolves.toEqual({ signature: 'signature' });

    expect(tokenRequests).toEqual([
      { targetKind: 'solana-gateway-binding', targetId: GATEWAY_ID, scopes: ['read'], forceRefresh: false },
      {
        targetKind: 'solana-gateway-binding',
        targetId: GATEWAY_ID,
        scopes: ['transaction:inspect'],
        forceRefresh: false,
      },
      {
        targetKind: 'solana-gateway-binding',
        targetId: GATEWAY_ID,
        scopes: ['transaction:send'],
        forceRefresh: false,
      },
    ]);
    expect(gatewayRequests.map(({ url }) => url)).toEqual([
      `${ENDPOINT}chain/exists/account`,
      `${ENDPOINT}chain/exists/account`,
      `${ENDPOINT}transactions/v1/latest-blockhash`,
      `${ENDPOINT}transactions/v1/latest-blockhash`,
      `${ENDPOINT}transactions/v1/send`,
    ]);
    expect(gatewayRequests.map(({ authorization }) => authorization)).toEqual([
      'Bearer token-read',
      'Bearer token-read',
      'Bearer token-transaction:inspect',
      'Bearer token-transaction:inspect',
      'Bearer token-transaction:send',
    ]);
    expect(getToken).toHaveBeenCalledTimes(3);
  });

  it('refreshes once only when transaction dispatch is explicitly safe to replay', async () => {
    const getToken = vi
      .fn(async (request?: AuthTokenRequest) => ({
        token: getToken.mock.calls.length === 1 ? 'stale' : 'fresh',
        scopes: request?.scopes,
      }));
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ code: 'token-expired' }), {
        status: 401,
        headers: {
          'X-Error-Code': 'token-expired',
          'X-Arete-Upstream-Attempted': 'false',
        },
      }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ signature: 'signature' }), {
        status: 200,
      }));
    const { transactions } = createHostedSolanaGatewayTransports(BINDINGS, {
      auth: { getToken },
      fetch: fetchMock as typeof fetch,
    });

    await expect(transactions.sendTransaction('signed'))
      .resolves.toEqual({ signature: 'signature' });
    expect(getToken).toHaveBeenCalledTimes(2);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(new Headers(fetchMock.mock.calls[0]?.[1]?.headers).get('authorization'))
      .toBe('Bearer stale');
    expect(new Headers(fetchMock.mock.calls[1]?.[1]?.headers).get('authorization'))
      .toBe('Bearer fresh');
  });

  it('never refreshes a transaction after upstream dispatch may have started', async () => {
    const getToken = vi.fn(async (request?: AuthTokenRequest) => ({
      token: 'stale',
      scopes: request?.scopes,
    }));
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ code: 'token-expired' }), {
      status: 401,
      headers: {
        'X-Error-Code': 'token-expired',
        'X-Arete-Upstream-Attempted': 'true',
      },
    }));
    const { transactions } = createHostedSolanaGatewayTransports(BINDINGS, {
      auth: { getToken },
      fetch: fetchMock as typeof fetch,
    });

    await expect(transactions.sendTransaction('signed')).rejects.toMatchObject({ status: 401 });
    expect(getToken).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});

describe('hosted Solana gateway transports with an API credential (tokenKind: api)', () => {
  const SESSION_ENDPOINT = 'https://api.arete.run/ws/sessions';

  function gatewayFetch(options: { rejectFirstMint?: boolean } = {}) {
    const mints: Array<{ authorization: string | null; body: Record<string, unknown> }> = [];
    const gateway: Array<{ url: string; authorization: string | null }> = [];
    let rejectMint = options.rejectFirstMint ?? false;
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      const authorization = new Headers(init?.headers).get('authorization');
      if (url === SESSION_ENDPOINT) {
        const body = JSON.parse(String(init?.body)) as Record<string, unknown>;
        mints.push({ authorization, body });
        if (rejectMint) {
          rejectMint = false;
          return new Response(JSON.stringify({ error: 'expired', code: 'token-expired' }), { status: 401 });
        }
        const scopes = body.scopes as string[];
        return new Response(JSON.stringify({
          token: `gateway-session:${scopes.join('+')}`,
          expires_at: Math.floor(Date.now() / 1000) + 300,
          scopes,
        }), { status: 200 });
      }
      gateway.push({ url, authorization });
      if (url.endsWith('/chain/exists/account')) {
        return new Response(JSON.stringify({ exists: true }), { status: 200 });
      }
      if (url.endsWith('/transactions/v1/send')) {
        return new Response(JSON.stringify({ signature: 'signature' }), { status: 200 });
      }
      throw new Error(`Unexpected request: ${url}`);
    });
    return { fetch: fetchMock as typeof fetch, mints, gateway };
  }

  it('mints binding sessions with a static API bearer and never sends it to the gateway', async () => {
    const { fetch, mints, gateway } = gatewayFetch();
    const transports = createHostedSolanaGatewayTransports(BINDINGS, {
      auth: { token: 'dashboard-login', tokenKind: 'api' },
      fetch,
    });

    await transports.chain.exists('account');
    await transports.chain.exists('account');
    await expect(transports.transactions.sendTransaction('signed'))
      .resolves.toEqual({ signature: 'signature' });

    expect(mints).toEqual([
      {
        authorization: 'Bearer dashboard-login',
        body: { targetKind: 'solana-gateway-binding', targetId: GATEWAY_ID, scopes: ['read'] },
      },
      {
        authorization: 'Bearer dashboard-login',
        body: { targetKind: 'solana-gateway-binding', targetId: GATEWAY_ID, scopes: ['transaction:send'] },
      },
    ]);
    expect(gateway.map(({ authorization }) => authorization)).toEqual([
      'Bearer gateway-session:read',
      'Bearer gateway-session:read',
      'Bearer gateway-session:transaction:send',
    ]);
  });

  it('asks getToken for the API credential with no target, and again when a mint is refused', async () => {
    const { fetch, mints, gateway } = gatewayFetch({ rejectFirstMint: true });
    const requests: Array<AuthTokenRequest | undefined> = [];
    let issued = 0;
    const transports = createHostedSolanaGatewayTransports(BINDINGS, {
      auth: {
        tokenKind: 'api',
        getToken: async (request) => {
          requests.push(request);
          issued += 1;
          return `login-${issued}`;
        },
      },
      fetch,
    });

    await transports.chain.exists('account');

    expect(requests).toEqual([{ scopes: [], forceRefresh: false }, { scopes: [], forceRefresh: true }]);
    expect(mints.map(({ authorization }) => authorization)).toEqual(['Bearer login-1', 'Bearer login-2']);
    expect(gateway.map(({ authorization }) => authorization)).toEqual(['Bearer gateway-session:read']);
  });

  it('keeps tokenKind session (the default) as a gateway-session provider', async () => {
    const { fetch, mints, gateway } = gatewayFetch();
    const transports = createHostedSolanaGatewayTransports(BINDINGS, {
      auth: { token: 'gateway-session-from-app' },
      fetch,
    });
    await transports.chain.exists('account');
    expect(mints).toEqual([]);
    expect(gateway.map(({ authorization }) => authorization)).toEqual(['Bearer gateway-session-from-app']);
  });
});
