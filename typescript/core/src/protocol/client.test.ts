import { readFileSync } from 'node:fs';

import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  canonicalizeJson,
  createProtocolClient,
  isProtocolAccessRejectionCode,
  isProtocolStateRejectionCode,
  PROTOCOL_STATE_REJECTION_CODES,
  protocolMetadataProof,
  protocolMetadataProofMessage,
  type ProtocolDefinition,
  type ProtocolTypeMap,
  verifyReceipt,
  verifyReceiptOnChain,
} from '../index';
import { createProtocolClientWithMembers, resolveProtocolEndpoints } from './client';
import { ServerSentEventParser } from './sse';

const DEPLOYMENT_ID = '00000000-0000-4000-8000-000000000001';
const API = 'https://api.example.test/api';
const SESSIONS = 'https://api.example.test/ws/sessions';
const WALLET = 'So11111111111111111111111111111111111111112';

const DEFINITION: ProtocolDefinition<ProtocolTypeMap> = {
  kind: 'protocol',
  name: 'tokenRaise',
  version: '1.0.0',
  package: 'token-raise',
  packageReleaseHash: 'arete:registry-package-release:v2:sha256:protocol',
  protocolSpecHash: 'arete:h1:protocol-spec:sha256:spec',
  protocolReleaseHash: 'arete:h1:protocol-release:sha256:release',
  deployment: { deploymentId: DEPLOYMENT_ID, access: 'public', apiUrl: `${API}/`, sessionEndpoint: SESSIONS },
  instanceVisibility: 'account',
  steps: {
    create: { scope: 'create', access: 'secret', guards: ['soft_cap_above_hard_cap'], args: ['hardCap'] },
    contribute: { scope: 'participant', access: 'public', guards: ['raise_not_open'], args: ['amount'] },
    claim: { scope: 'participant', access: 'public', guards: ['raise_not_finalized'], args: [] },
  },
  events: { 'raise.succeeded': { machine: 'instance' } },
  metadata: {
    instance: {
      name: { writer: 'creator', filterable: true },
      tagline: { writer: 'creator', filterable: false },
    },
  },
  stacks: {},
  programs: {},
};

interface Call {
  url: string;
  method: string;
  headers: Headers;
  body: unknown;
}

type Route = (call: Call) => Response | Promise<Response> | undefined;

function json(value: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json', ...headers },
  });
}

function mockFetch(...routes: Route[]) {
  const calls: Call[] = [];
  let tokens = 0;
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const text = typeof init?.body === 'string' ? init.body : undefined;
    const call: Call = {
      url: String(input),
      method: init?.method ?? 'GET',
      headers: new Headers(init?.headers),
      body: text === undefined ? undefined : JSON.parse(text),
    };
    calls.push(call);
    for (const route of routes) {
      const response = await route(call);
      if (response) return response;
    }
    if (call.url === SESSIONS) {
      tokens += 1;
      return json({ token: `tok-${tokens}`, expires_at: Math.floor(Date.now() / 1000) + 3600 });
    }
    return json({ error: `unrouted ${call.method} ${call.url}` }, 404);
  });
  return {
    fetch: fetchMock as unknown as typeof fetch,
    calls,
    mints: () => calls.filter((call) => call.url === SESSIONS),
    requests: () => calls.filter((call) => call.url !== SESSIONS),
  };
}

const DETAIL = {
  id: 'inst-1',
  deploymentId: DEPLOYMENT_ID,
  revision: 1,
  state: 'open',
  terminal: false,
  visibility: 'shared',
  accounts: { presale: 'PresaleAccount1111111111111111111111111111' },
  roles: {},
  createdByWallet: WALLET,
  createdAt: '2026-10-13T00:00:00Z',
  updatedAt: '2026-10-13T00:00:00Z',
  closedAt: null,
  releaseHash: 'arete:h1:protocol-release:sha256:release',
  params: { hardCap: '1000' },
  lastErrorCode: null,
  derived: null,
  metadata: { name: 'Raise' },
  metadataVersion: 3,
  pendingTimers: [],
  allowedSteps: [
    { step: 'contribute', allowed: true },
    { step: 'claim', allowed: false, reason: 'raise_not_finalized' },
  ],
};

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
});

describe('protocol client environment', () => {
  const deployment = { deploymentId: DEPLOYMENT_ID, apiUrl: API, sessionEndpoint: SESSIONS };
  const none = () => undefined;

  it('uses the generated descriptor by default', () => {
    expect(resolveProtocolEndpoints(deployment, undefined, none)).toEqual(deployment);
  });

  it('switches environment with ARETE_API_URL, the API origin', () => {
    const env = (name: string) => (name === 'ARETE_API_URL' ? ' http://127.0.0.1:48705/ ' : undefined);
    expect(resolveProtocolEndpoints(deployment, undefined, env)).toEqual({
      deploymentId: DEPLOYMENT_ID,
      apiUrl: 'http://127.0.0.1:48705/api',
      sessionEndpoint: 'http://127.0.0.1:48705/ws/sessions',
    });
  });

  it('prefers the options, and an apiUrl carries its own session endpoint', () => {
    const env = () => 'http://ignored.test';
    expect(resolveProtocolEndpoints(deployment, { apiUrl: 'http://127.0.0.1:4100/api/' }, env)).toEqual({
      deploymentId: DEPLOYMENT_ID,
      apiUrl: 'http://127.0.0.1:4100/api',
      sessionEndpoint: 'http://127.0.0.1:4100/ws/sessions',
    });
    expect(
      resolveProtocolEndpoints(
        deployment,
        { apiUrl: 'http://a.test/api', sessionEndpoint: 'http://b.test/ws/sessions', deploymentId: 'other' },
        env
      )
    ).toEqual({ deploymentId: 'other', apiUrl: 'http://a.test/api', sessionEndpoint: 'http://b.test/ws/sessions' });
  });

  it('reads ARETE_API_URL outside browsers and calls the deploymentId override', async () => {
    vi.stubEnv('ARETE_API_URL', 'http://127.0.0.1:48705');
    const local = 'http://127.0.0.1:48705';
    const calls: string[] = [];
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push(String(input));
      if (String(input) === `${local}/ws/sessions`) {
        expect(JSON.parse(String(init?.body)).targetId).toBe('other-deployment');
        return json({ token: 'tok', expires_at: Math.floor(Date.now() / 1000) + 3600 });
      }
      return json({ instances: [], nextCursor: null });
    });
    const client = createProtocolClient(DEFINITION, {
      auth: { secretKey: 'a4_sk_local' },
      deploymentId: 'other-deployment',
      fetch: fetchMock as unknown as typeof fetch,
    });
    expect(client.deploymentId).toBe('other-deployment');
    await client.instances.list();
    expect(calls).toEqual([
      `${local}/ws/sessions`,
      `${local}/api/protocol-deployments/other-deployment/instances`,
    ]);
  });
});

describe('protocol client sessions', () => {
  it('mints a read-only protocol-deployment session for reads', async () => {
    const mock = mockFetch((call) =>
      call.url.startsWith(`${API}/protocol-deployments/`) ? json({ instances: [], nextCursor: null }) : undefined
    );
    const client = createProtocolClient(DEFINITION, {
      auth: { publishableKey: 'a4_pk_test' },
      fetch: mock.fetch,
    });
    expect(client.keyClass).toBe('publishable');

    await client.instances.list();
    await client.instances.list({ state: 'open' });

    const [mint] = mock.mints();
    expect(mock.mints()).toHaveLength(1);
    expect(mint!.method).toBe('POST');
    expect(mint!.headers.get('authorization')).toBe('Bearer a4_pk_test');
    expect(mint!.body).toEqual({
      targetKind: 'protocol-deployment',
      targetId: DEPLOYMENT_ID,
      scopes: ['protocol:read'],
    });
    expect(mock.requests()[0]!.headers.get('authorization')).toBe('Bearer tok-1');
  });

  it('mints one prepare-capable session for the public steps on the first prepare', async () => {
    const mock = mockFetch((call) =>
      call.url.endsWith('/prepare')
        ? json({ intent: { executionId: 'exec-1' }, transactions: [{ index: 0, transactionBase64: 'AAA=' }] })
        : undefined
    );
    const client = createProtocolClient(DEFINITION, {
      auth: { publishableKey: 'a4_pk_test' },
      fetch: mock.fetch,
    });

    const result = await client.steps.contribute!.prepare({
      instance: 'inst-1',
      wallet: WALLET,
      amount: 12345678901234567890n,
    });
    await client.steps.claim!.prepare({ instance: 'inst-1', wallet: WALLET });

    expect(result).toEqual({
      ok: true,
      intent: { executionId: 'exec-1' },
      transactions: [{ index: 0, transactionBase64: 'AAA=' }],
    });
    expect(mock.mints()).toHaveLength(1);
    expect(mock.mints()[0]!.body).toEqual({
      targetKind: 'protocol-deployment',
      targetId: DEPLOYMENT_ID,
      scopes: ['protocol:prepare:claim', 'protocol:prepare:contribute', 'protocol:read', 'protocol:submit'],
    });
    const prepare = mock.requests()[0]!;
    expect(prepare.method).toBe('POST');
    expect(prepare.url).toBe(`${API}/protocol-instances/inst-1/steps/contribute/prepare`);
    expect(prepare.headers.get('authorization')).toBe('Bearer tok-1');
    expect(prepare.body).toEqual({
      idempotencyKey: expect.stringMatching(/^[0-9a-f-]{36}$/),
      wallet: WALLET,
      args: { amount: '12345678901234567890' },
      transactionFormat: 'v1',
    });
  });

  it('refuses a secret key for the descriptor session endpoint unless apiUrl trusts its origin', async () => {
    const mock = mockFetch(() => undefined);
    // The generated descriptor names api.example.test; naming it trusts nothing.
    const client = createProtocolClient(DEFINITION, { auth: { secretKey: 'a4_sk_test' }, fetch: mock.fetch });
    await expect(client.instances.list()).rejects.toMatchObject({
      code: 'UNTRUSTED_CREDENTIAL_DESTINATION',
      details: { destination: 'https://api.example.test', credential: 'auth.secretKey' },
    });
    // Nor does a sessionEndpoint override.
    const overridden = createProtocolClient(DEFINITION, {
      auth: { secretKey: 'a4_sk_test' },
      sessionEndpoint: 'https://sessions.example.test/ws/sessions',
      apiUrl: API,
      fetch: mock.fetch,
    });
    await expect(overridden.instances.list()).rejects.toMatchObject({ code: 'UNTRUSTED_CREDENTIAL_DESTINATION' });
    expect(mock.calls).toEqual([]);
  });

  it('mints protocol:prepare:* for a secret key and routes create steps to the deployment', async () => {
    const mock = mockFetch((call) =>
      call.url.endsWith('/prepare') ? json({ intent: { executionId: 'exec-2' }, transactions: [] }) : undefined
    );
    const client = createProtocolClient(DEFINITION, {
      auth: { secretKey: 'a4_sk_test' },
      // The test API is not the default one: trust it for the key.
      apiUrl: API,
      fetch: mock.fetch,
    });
    expect(client.keyClass).toBe('secret');

    await client.steps.create!.prepare(
      { wallet: WALLET, hardCap: '10' },
      { transactionFormat: 'v0', idempotencyKey: 'idem-1', clientContext: { requestId: 'req-1' } }
    );

    expect(mock.mints()[0]!.body).toEqual({
      targetKind: 'protocol-deployment',
      targetId: DEPLOYMENT_ID,
      scopes: ['protocol:prepare:*', 'protocol:read', 'protocol:submit'],
    });
    expect(mock.mints()[0]!.headers.get('authorization')).toBe('Bearer a4_sk_test');
    expect(mock.requests()[0]!.url).toBe(
      `${API}/protocol-deployments/${DEPLOYMENT_ID}/steps/create/prepare`
    );
    expect(mock.requests()[0]!.body).toEqual({
      idempotencyKey: 'idem-1',
      wallet: WALLET,
      args: { hardCap: '10' },
      transactionFormat: 'v0',
      clientContext: { requestId: 'req-1' },
    });
  });

  it('returns guard rejections (422) and access rejections (403) as results', async () => {
    const mock = mockFetch((call) => {
      if (call.url.includes('/steps/contribute/')) {
        return json({
          rejected: 'raise_not_open',
          step: 'contribute',
          guard: "instance.state == 'open'",
          detail: { 'instance.state': 'closed' },
        }, 422);
      }
      if (call.url.includes('/steps/claim/')) {
        return json({ error: 'not in scope' }, 403, { 'X-Error-Code': 'step_not_in_session_scope' });
      }
      return undefined;
    });
    const client = createProtocolClient(DEFINITION, {
      auth: { publishableKey: 'a4_pk_test' },
      fetch: mock.fetch,
    });

    await expect(client.steps.contribute!.prepare({ instance: 'inst-1', wallet: WALLET, amount: '1' }))
      .resolves.toEqual({
        ok: false,
        rejection: {
          rejected: 'raise_not_open',
          step: 'contribute',
          guard: "instance.state == 'open'",
          detail: { 'instance.state': 'closed' },
        },
      });
    await expect(client.steps.claim!.prepare({ instance: 'inst-1', wallet: WALLET }))
      .resolves.toEqual({
        ok: false,
        rejection: { rejected: 'step_not_in_session_scope', step: 'claim' },
      });
  });

  it('returns state rejections (instance_terminal, participant_terminal) with their detail', async () => {
    const mock = mockFetch((call) => {
      if (call.url.includes('/steps/contribute/')) {
        return json({ rejected: 'participant_terminal', step: 'contribute', detail: { state: 'left' } }, 422);
      }
      if (call.url.includes('/steps/claim/')) {
        return json({ rejected: 'instance_terminal', step: 'claim', detail: { state: 'closed' } }, 422);
      }
      return undefined;
    });
    const client = createProtocolClient(DEFINITION, {
      auth: { publishableKey: 'a4_pk_test' },
      fetch: mock.fetch,
    });

    const left = await client.steps.contribute!.prepare({ instance: 'inst-1', wallet: WALLET, amount: '1' });
    expect(left).toEqual({
      ok: false,
      rejection: { rejected: 'participant_terminal', step: 'contribute', detail: { state: 'left' } },
    });
    if (!left.ok) {
      expect(isProtocolStateRejectionCode(left.rejection.rejected)).toBe(true);
      expect(isProtocolAccessRejectionCode(left.rejection.rejected)).toBe(false);
    }
    await expect(client.steps.claim!.prepare({ instance: 'inst-1', wallet: WALLET }))
      .resolves.toEqual({
        ok: false,
        rejection: { rejected: 'instance_terminal', step: 'claim', detail: { state: 'closed' } },
      });
    expect(PROTOCOL_STATE_REJECTION_CODES).toEqual(['instance_terminal', 'participant_terminal']);
  });

  it('refuses a secret step on a publishable client without a network call', async () => {
    const mock = mockFetch();
    const client = createProtocolClient(DEFINITION, {
      auth: { publishableKey: 'a4_pk_test' },
      fetch: mock.fetch,
    });
    const steps = client.steps as Record<string, { prepare(input: unknown): Promise<unknown> }>;

    await expect(steps['create']!.prepare({ wallet: WALLET, hardCap: '1' })).resolves.toEqual({
      ok: false,
      rejection: { rejected: 'step_requires_secret_key', step: 'create' },
    });
    await expect(steps['finalize']?.prepare({})).toBeUndefined();
    expect(mock.calls).toHaveLength(0);
  });

  it('maps a mint refusal of a secret step to a rejection', async () => {
    const mock = mockFetch((call) =>
      call.url === SESSIONS
        ? json({ error: 'secret step', code: 'step_requires_secret_key' }, 403)
        : undefined
    );
    const client = createProtocolClient(DEFINITION, {
      auth: { publishableKey: 'a4_pk_test' },
      fetch: mock.fetch,
      steps: ['contribute', 'create'],
    });

    await expect(client.steps.contribute!.prepare({ instance: 'inst-1', wallet: WALLET, amount: '1' }))
      .resolves.toEqual({
        ok: false,
        rejection: { rejected: 'step_requires_secret_key', step: 'contribute' },
      });
    expect(mock.mints()[0]!.body).toMatchObject({
      scopes: ['protocol:prepare:contribute', 'protocol:prepare:create', 'protocol:read', 'protocol:submit'],
    });
  });

  it('refreshes the session once on a refreshable error code', async () => {
    let attempts = 0;
    const mock = mockFetch((call) => {
      if (!call.url.startsWith(`${API}/protocol-instances/`)) return undefined;
      attempts += 1;
      return attempts === 1
        ? json({ error: 'expired' }, 401, { 'X-Error-Code': 'token-expired' })
        : json(DETAIL);
    });
    const client = createProtocolClient(DEFINITION, {
      auth: { publishableKey: 'a4_pk_test' },
      fetch: mock.fetch,
    });

    await expect(client.instances.get('inst-1')).resolves.toMatchObject({ id: 'inst-1' });
    expect(mock.mints()).toHaveLength(2);
    expect(mock.requests().map((call) => call.headers.get('authorization'))).toEqual([
      'Bearer tok-1',
      'Bearer tok-2',
    ]);
  });

  it('sends a static token as is and never mints', async () => {
    const mock = mockFetch((call) => (call.url.includes('/protocol-instances/') ? json(DETAIL) : undefined));
    const client = createProtocolClient(DEFINITION, { auth: { token: 'static-jwt' }, fetch: mock.fetch });
    await client.instances.get('inst-1');
    expect(mock.mints()).toHaveLength(0);
    expect(mock.requests()[0]!.headers.get('authorization')).toBe('Bearer static-jwt');
  });

  it('uses a getToken provider with the protocol target', async () => {
    const getToken = vi.fn(async () => 'provided');
    const mock = mockFetch((call) => (call.url.includes('/protocol-instances/') ? json(DETAIL) : undefined));
    const client = createProtocolClient(DEFINITION, { auth: { getToken }, fetch: mock.fetch });
    await client.instances.get('inst-1');
    expect(getToken).toHaveBeenCalledWith({
      targetKind: 'protocol-deployment',
      targetId: DEPLOYMENT_ID,
      scopes: ['protocol:read'],
      forceRefresh: false,
    });
    expect(mock.requests()[0]!.headers.get('authorization')).toBe('Bearer provided');
  });
});

describe('protocol client reads', () => {
  it('builds list, get, participant and allowed-step URLs', async () => {
    const mock = mockFetch((call) => {
      if (call.url.includes('/instances')) return json({ instances: [DETAIL], nextCursor: 'c2' });
      if (call.url.includes('/participants/')) {
        return json({
          instanceId: 'inst-1',
          wallet: WALLET,
          participantId: 'p-1',
          state: 'contributed',
          metadata: {},
          history: [],
          allowedSteps: [],
        });
      }
      if (call.url.includes('/protocol-instances/')) return json(DETAIL);
      return undefined;
    });
    const client = createProtocolClient(DEFINITION, {
      auth: { publishableKey: 'a4_pk_test' },
      fetch: mock.fetch,
    });

    await expect(client.instances.list({
      state: 'open',
      cursor: 'c1',
      limit: 20,
      metadata: { name: 'My raise' },
    })).resolves.toEqual({ instances: [DETAIL], nextCursor: 'c2' });
    await client.instances.get('inst/1', { wallet: WALLET });
    await client.participants.get({ instance: 'inst-1', wallet: WALLET });
    await expect(client.allowedSteps({ instance: 'inst-1', wallet: WALLET }))
      .resolves.toEqual(DETAIL.allowedSteps);

    expect(mock.requests().map((call) => call.url)).toEqual([
      `${API}/protocol-deployments/${DEPLOYMENT_ID}/instances?state=open&cursor=c1&limit=20&metadata.name=My+raise`,
      `${API}/protocol-instances/inst%2F1?wallet=${WALLET}`,
      `${API}/protocol-instances/inst-1/participants/${WALLET}`,
      `${API}/protocol-instances/inst-1?wallet=${WALLET}`,
    ]);
    // Reads share one read-only session.
    expect(mock.mints()).toHaveLength(1);

    await expect(client.instances.list({ metadata: { tagline: 'x' } })).rejects.toMatchObject({
      code: 'INVALID_CONFIG',
    });
  });

  it('reads a wallet\'s actions across the deployment, page by page', async () => {
    const CONTRIBUTE_ARGS = [{ name: 'amount', type: 'string', format: 'u64', optional: false, description: 'Quote amount' }];
    const firstPage = {
      deploymentId: DEPLOYMENT_ID,
      wallet: WALLET,
      createSteps: [{
        step: 'create',
        allowed: true,
        scope: 'create',
        argsDependent: ['soft_cap_above_hard_cap'],
        args: [{ name: 'hardCap', type: 'string', format: 'u64', optional: false }],
      }],
      instances: [{
        instanceId: 'inst-1',
        state: 'open',
        relation: ['participant', 'role:admin', 'creator'],
        participantState: 'contributed',
        steps: [
          { step: 'contribute', allowed: true, scope: 'participant', args: CONTRIBUTE_ARGS },
          { step: 'claim', allowed: false, reason: 'raise_not_finalized', scope: 'participant', args: [] },
        ],
      }],
      nextCursor: 'c2',
    };
    const secondPage = {
      deploymentId: DEPLOYMENT_ID,
      wallet: WALLET,
      createSteps: [],
      instances: [{ instanceId: 'inst-2', state: 'closed', relation: ['creator'], participantState: null, steps: [] }],
      nextCursor: null,
    };
    const mock = mockFetch((call) => {
      if (!call.url.includes('/wallets/')) return undefined;
      return json(new URL(call.url).searchParams.get('cursor') === 'c2' ? secondPage : firstPage);
    });
    const client = createProtocolClient(DEFINITION, {
      auth: { publishableKey: 'a4_pk_test' },
      fetch: mock.fetch,
    });

    const page = await client.actions({ wallet: WALLET, limit: 1 });
    expect(page).toEqual(firstPage);
    expect(page.instances[0]!.steps[0]!.args).toEqual(CONTRIBUTE_ARGS);
    await expect(client.actions({ wallet: WALLET, cursor: page.nextCursor!, limit: 1 }))
      .resolves.toEqual(secondPage);
    await client.actions({ wallet: WALLET, instance: 'inst/1' });

    const base = `${API}/protocol-deployments/${DEPLOYMENT_ID}/wallets/${WALLET}/actions`;
    expect(mock.requests().map((call) => call.url)).toEqual([
      `${base}?limit=1`,
      `${base}?cursor=c2&limit=1`,
      `${base}?instance=inst%2F1`,
    ]);
    expect(mock.requests().every((call) => call.method === 'GET')).toBe(true);
    // A read: one read-only session.
    expect(mock.mints()).toHaveLength(1);
    expect(mock.mints()[0]!.body).toMatchObject({ scopes: ['protocol:read'] });

    await expect(client.actions({ wallet: '' })).rejects.toMatchObject({ code: 'INVALID_CONFIG' });
    expect(mock.requests()).toHaveLength(3);
  });

  it('finds the instances a wallet created across every page of its actions', async () => {
    const entry = (instanceId: string, relation: string[]) => ({
      instanceId,
      state: 'open',
      relation,
      participantState: null,
      steps: [],
    });
    const pages: Record<string, { instances: ReturnType<typeof entry>[]; nextCursor: string | null }> = {
      '': {
        instances: [entry('inst-1', ['creator']), entry('inst-2', ['participant']), entry('inst-3', ['participant', 'creator'])],
        nextCursor: 'c2',
      },
      c2: { instances: [entry('inst-4', ['role:admin']), entry('inst-5', ['creator', 'role:admin'])], nextCursor: 'c3' },
      c3: { instances: [entry('inst-6', ['participant'])], nextCursor: null },
    };
    const mock = mockFetch((call) => {
      if (!call.url.includes('/wallets/')) return undefined;
      const page = pages[new URL(call.url).searchParams.get('cursor') ?? '']!;
      return json({ deploymentId: DEPLOYMENT_ID, wallet: WALLET, createSteps: [], ...page });
    });
    const client = createProtocolClient(DEFINITION, { auth: { publishableKey: 'a4_pk_test' }, fetch: mock.fetch });

    const created = await client.instances.forWallet(WALLET, { relation: 'creator' });
    expect(created.map((instance) => instance.instanceId)).toEqual(['inst-1', 'inst-3', 'inst-5']);
    expect(created[0]).toEqual(entry('inst-1', ['creator']));
    const base = `${API}/protocol-deployments/${DEPLOYMENT_ID}/wallets/${WALLET}/actions`;
    expect(mock.requests().map((call) => call.url)).toEqual([
      `${base}?limit=50`,
      `${base}?cursor=c2&limit=50`,
      `${base}?cursor=c3&limit=50`,
    ]);

    const admin = await client.instances.forWallet(WALLET, { relation: 'role:admin' });
    expect(admin.map((instance) => instance.instanceId)).toEqual(['inst-4', 'inst-5']);
    const all = await client.instances.forWallet(WALLET);
    expect(all.map((instance) => instance.instanceId)).toEqual(['inst-1', 'inst-2', 'inst-3', 'inst-4', 'inst-5', 'inst-6']);
    await expect(client.instances.forWallet('')).rejects.toMatchObject({ code: 'INVALID_CONFIG' });
  });

  it('stops when wallet actions return a cursor twice', async () => {
    const mock = mockFetch((call) =>
      call.url.includes('/wallets/')
        ? json({ deploymentId: DEPLOYMENT_ID, wallet: WALLET, instances: [], nextCursor: 'same' })
        : undefined
    );
    const client = createProtocolClient(DEFINITION, { auth: { token: 't' }, fetch: mock.fetch });
    await expect(client.instances.forWallet(WALLET)).rejects.toMatchObject({ code: 'PROTOCOL_REQUEST_FAILED' });
    expect(mock.requests()).toHaveLength(2);
  });

  it('encodes the wallet and defaults missing lists on wallet actions', async () => {
    const mock = mockFetch((call) =>
      call.url.includes('/wallets/') ? json({ deploymentId: DEPLOYMENT_ID, wallet: 'a/b' }) : undefined
    );
    const client = createProtocolClient(DEFINITION, { auth: { token: 't' }, fetch: mock.fetch });
    await expect(client.actions({ wallet: 'a/b' })).resolves.toEqual({
      deploymentId: DEPLOYMENT_ID,
      wallet: 'a/b',
      createSteps: [],
      instances: [],
      nextCursor: null,
    });
    expect(mock.requests()[0]!.url).toBe(`${API}/protocol-deployments/${DEPLOYMENT_ID}/wallets/a%2Fb/actions`);
    expect(mock.requests()[0]!.headers.get('authorization')).toBe('Bearer t');
  });

  it('maps wallet-action failures to AreteError with the wire code', async () => {
    const mock = mockFetch((call) =>
      call.url.includes('/wallets/')
        ? json({ error: 'limit must be at most 50', code: 'invalid_request' }, 400)
        : undefined
    );
    const client = createProtocolClient(DEFINITION, { auth: { token: 't' }, fetch: mock.fetch });
    await expect(client.actions({ wallet: WALLET, limit: 51 })).rejects.toMatchObject({
      name: 'AreteError',
      code: 'invalid_request',
      details: { status: 400, wireErrorCode: 'invalid_request' },
    });
  });

  it('throws AreteError with the wire code for other failures', async () => {
    const mock = mockFetch((call) =>
      call.url.includes('/protocol-instances/')
        ? json({ error: 'no such instance', code: 'instance_not_found' }, 404)
        : undefined
    );
    const client = createProtocolClient(DEFINITION, { auth: { token: 't' }, fetch: mock.fetch });
    await expect(client.instances.get('missing')).rejects.toMatchObject({
      name: 'AreteError',
      code: 'instance_not_found',
      details: { status: 404 },
    });
  });
});

describe('protocol client submit', () => {
  it('base64-encodes signed transactions and reads the intent state', async () => {
    const state = {
      executionId: 'exec-1',
      state: 'submitting',
      transactions: [{ index: 0, signature: 'sig', state: 'sent', error: null }],
    };
    const mock = mockFetch((call) => (call.url.includes('/execution-intents/') ? json(state) : undefined));
    const client = createProtocolClient(DEFINITION, {
      auth: { publishableKey: 'a4_pk_test' },
      fetch: mock.fetch,
    });

    await expect(client.submit(
      { executionId: 'exec-1' } as never,
      [new Uint8Array([0, 1, 2, 3, 255]), 'already-base64']
    )).resolves.toEqual(state);
    await expect(client.intents.get('exec-1')).resolves.toEqual(state);

    const [submit, read] = mock.requests();
    expect(submit!.url).toBe(`${API}/execution-intents/exec-1/submit`);
    expect(submit!.body).toEqual({ signedTransactions: ['AAECA/8=', 'already-base64'] });
    expect(read!.url).toBe(`${API}/execution-intents/exec-1`);
    expect(read!.method).toBe('GET');
    expect(mock.mints().map((call) => (call.body as { scopes: string[] }).scopes)).toEqual([
      ['protocol:prepare:claim', 'protocol:prepare:contribute', 'protocol:read', 'protocol:submit'],
      ['protocol:read'],
    ]);
  });
});

describe('protocol metadata', () => {
  it('serializes RFC 8785 canonical JSON', () => {
    // RFC 8785 section 3.2.2 sample.
    const value = JSON.parse(
      '{"numbers":[333333333.33333329,1E30,4.50,2e-3,0.000000000000000000000000001],'
        + '"string":"\\u20ac$\\u000F\\u000aA\'\\u0042\\u0022\\u005c\\\\\\"\\/",'
        + '"literals":[null,true,false]}'
    ) as unknown;
    expect(canonicalizeJson(value)).toBe(
      '{"literals":[null,true,false],"numbers":[333333333.3333333,1e+30,4.5,0.002,1e-27],'
        + '"string":"€$\\u000f\\nA\'B\\"\\\\\\\\\\"/"}'
    );
    // Keys sort by UTF-16 code units.
    expect(canonicalizeJson({ '€': 1, '😀': 2, a: { z: null, b: [1, 'x'] }, B: true }))
      .toBe('{"B":true,"a":{"b":[1,"x"],"z":null},"€":1,"😀":2}');
    expect(canonicalizeJson({ skipped: undefined, kept: -0 })).toBe('{"kept":0}');
    expect(() => canonicalizeJson({ n: Number.NaN })).toThrow(TypeError);
    expect(() => canonicalizeJson({ n: 1n })).toThrow(/bigint/);
    expect(() => canonicalizeJson('\ud800')).toThrow(/surrogate/);
  });

  it('rejects objects whose wire representation differs from their signed value', () => {
    class Custom { value = 1; }
    const hooked = Object.defineProperty({}, 'toJSON', { value: () => 'changed' });
    const array = Object.assign([1], { toJSON: () => [2] });
    for (const value of [new Date(0), new Map(), new Custom(), hooked, array]) {
      expect(() => canonicalizeJson({ nested: value })).toThrow(TypeError);
    }
    expect(canonicalizeJson(Object.assign(Object.create(null), { ok: 1 }))).toBe('{"ok":1}');
  });

  it('builds the wallet proof message and proof', () => {
    const message = protocolMetadataProofMessage({
      deploymentId: DEPLOYMENT_ID,
      instanceId: 'inst-1',
      wallet: WALLET,
      patch: { tagline: 'Hello', name: null },
      expiresAt: 1_790_000_100,
      now: 1_790_000_000,
    });
    // sha256('{"name":null,"tagline":"Hello"}')
    expect(message.patchSha256).toBe(
      '674f43c7ebbdbd1c6b643ba842cb4145dedfc86118e50c8c4e8748b91afe6be6'
    );
    expect(message.message).toBe([
      'arete.protocol-metadata/v1',
      DEPLOYMENT_ID,
      'inst-1',
      WALLET,
      message.patchSha256,
      '1790000100',
    ].join('\n'));
    expect(new TextDecoder().decode(message.bytes)).toBe(message.message);

    expect(() => protocolMetadataProofMessage({
      deploymentId: DEPLOYMENT_ID,
      instanceId: 'inst-1',
      wallet: WALLET,
      patch: {},
      expiresAt: 1_790_000_301,
      now: 1_790_000_000,
    })).toThrow(RangeError);
    expect(() => protocolMetadataProofMessage({
      deploymentId: DEPLOYMENT_ID,
      instanceId: 'inst-1',
      wallet: WALLET,
      patch: {},
      expiresAt: 1_790_000_000,
      now: 1_790_000_000,
    })).toThrow(RangeError);

    const signature = new Uint8Array(64).fill(1);
    expect(protocolMetadataProof({ wallet: WALLET, signature, expiresAt: 1_790_000_100 })).toEqual({
      wallet: WALLET,
      signature: '2AXDGYSE4f2sz7tvMMzyHvUfcoJmxudvdhBcmiUSo6ijwfYmfZYsKRxboQMPh3R4kUhXRVdtSXFXMheka4Rc4P2',
      expiresAt: 1_790_000_100,
    });
    expect(() => protocolMetadataProof({ wallet: WALLET, signature: new Uint8Array(63), expiresAt: 1 }))
      .toThrow(TypeError);
  });

  it('reads and updates instance and participant metadata', async () => {
    const mock = mockFetch((call) => {
      if (call.url.endsWith('/metadata')) {
        return json({ instanceId: 'inst-1', participantId: null, version: 4, metadata: { name: 'New' } });
      }
      if (call.url.includes('/participants/')) {
        return json({ instanceId: 'inst-1', wallet: WALLET, participantId: 'p', state: null, metadata: { note: 'n' }, history: [], allowedSteps: [] });
      }
      if (call.url.includes('/protocol-instances/')) return json(DETAIL);
      return undefined;
    });
    const client = createProtocolClient(DEFINITION, { auth: { secretKey: 'a4_sk_test' }, apiUrl: API, fetch: mock.fetch });

    await expect(client.metadata.instance('inst-1').get()).resolves.toEqual({
      metadata: { name: 'Raise' },
      version: 3,
    });
    await expect(client.metadata.participant({ instance: 'inst-1', wallet: WALLET }).get())
      .resolves.toEqual({ metadata: { note: 'n' }, version: null });

    const handle = client.metadata.participant({ instance: 'inst-1', wallet: WALLET });
    const proofMessage = handle.proofMessage({ note: 'x' }, { wallet: WALLET });
    const proof = protocolMetadataProof({
      wallet: WALLET,
      signature: new Uint8Array(64),
      expiresAt: proofMessage.expiresAt,
    });
    await expect(handle.update({ note: 'x' }, { walletProof: proof })).resolves.toMatchObject({ version: 4 });
    await client.metadata.instance('inst-1').update({ name: 'New', tagline: null });

    const patches = mock.requests().filter((call) => call.method === 'PATCH');
    expect(patches.map((call) => call.url)).toEqual([
      `${API}/protocol-instances/inst-1/metadata`,
      `${API}/protocol-instances/inst-1/metadata`,
    ]);
    expect(patches[0]!.body).toEqual({ patch: { note: 'x' }, participant: WALLET, proof });
    expect(patches[1]!.body).toEqual({ patch: { name: 'New', tagline: null } });
    // Metadata needs only a read session.
    expect(mock.mints().map((call) => (call.body as { scopes: string[] }).scopes)).toEqual([
      ['protocol:read'],
    ]);
  });
});

function sseResponse(chunks: string[]): Response {
  const encoder = new TextEncoder();
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk));
      controller.close();
    },
  });
  return new Response(stream, { status: 200, headers: { 'Content-Type': 'text/event-stream' } });
}

async function waitFor(condition: () => boolean, timeoutMs = 2000): Promise<void> {
  const start = Date.now();
  while (!condition()) {
    if (Date.now() - start > timeoutMs) throw new Error('timed out');
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

describe('protocol events', () => {
  it('parses server-sent events across chunk boundaries', () => {
    const parser = new ServerSentEventParser();
    expect(parser.push('event: transition\r\nid: 1\r\ndata: {"a":')).toEqual([]);
    expect(parser.push('1}\r\n\r\n: comment\n\nretry: 250\nid: 2\nevent: emit\ndata: x\ndata: y\n\r')).toEqual([
      { event: 'transition', data: '{"a":1}', id: '1' },
    ]);
    expect(parser.push('\n')).toEqual([{ event: 'emit', data: 'x\ny', id: '2' }]);
    expect(parser.retryMs).toBe(250);
    expect(parser.lastEventId).toBe('2');
  });

  it('delivers typed events and resumes with Last-Event-ID after the server ends the stream', async () => {
    let connections = 0;
    const mock = mockFetch((call) => {
      if (!call.url.endsWith('/events')) return undefined;
      connections += 1;
      if (connections === 1) {
        return sseResponse([
          'event: transition\nid: 10\ndata: {"to":"open"}\n\n',
          'event: emit\nid: 11\ndata: {"id":5,"event":"raise.succeeded","reaction":"r","instanceId":"inst-1","participantId":null,"transitionId":4,"at":"2026"}\n\n',
          'retry: 0\n\n',
        ]);
      }
      if (connections === 2) {
        return sseResponse([
          'event: error\ndata: {"code":"session_expired"}\n\n',
        ]);
      }
      if (connections === 3) {
        return sseResponse(['event: participant\nid: 12\ndata: {"to":"contributed"}\n\n']);
      }
      // Like the server, nothing after Last-Event-ID 12: a reconnect that
      // races the close below gets an empty stream.
      return sseResponse([]);
    });
    const client = createProtocolClient(DEFINITION, {
      auth: { publishableKey: 'a4_pk_test' },
      fetch: mock.fetch,
    });
    const events: unknown[] = [];
    const errors: Error[] = [];
    const subscription = client.events('inst-1', {
      lastEventId: '9',
      onEvent: (event) => events.push(event),
      onError: (error) => errors.push(error),
    });

    await waitFor(() => events.length >= 3);
    subscription.close();

    expect(events).toEqual([
      { type: 'transition', id: '10', data: { to: 'open' } },
      {
        type: 'emit',
        id: '11',
        event: 'raise.succeeded',
        data: {
          id: 5,
          event: 'raise.succeeded',
          reaction: 'r',
          instanceId: 'inst-1',
          participantId: null,
          transitionId: 4,
          at: '2026',
        },
      },
      { type: 'participant', id: '12', data: { to: 'contributed' } },
    ]);
    expect(errors).toEqual([]);
    expect(subscription.lastEventId).toBe('12');
    const streams = mock
      .requests()
      .filter((call) => call.url.endsWith('/events'))
      .slice(0, 3);
    expect(streams[0]!.url).toBe(`${API}/protocol-instances/inst-1/events`);
    expect(streams.map((call) => call.headers.get('last-event-id'))).toEqual(['9', '11', '11']);
    expect(streams[0]!.headers.get('accept')).toBe('text/event-stream');
    // The expired session was re-minted before the third connection.
    expect(streams.map((call) => call.headers.get('authorization'))).toEqual([
      'Bearer tok-1',
      'Bearer tok-1',
      'Bearer tok-2',
    ]);
    expect(subscription.closed).toBe(true);
  });

  it('stops on a refused stream and reports the error', async () => {
    const mock = mockFetch((call) =>
      call.url.endsWith('/events') ? json({ error: 'gone', code: 'instance_not_found' }, 404) : undefined
    );
    const client = createProtocolClient(DEFINITION, { auth: { token: 't' }, fetch: mock.fetch });
    const errors: Error[] = [];
    const subscription = client.events('inst-1', { onEvent: () => undefined, onError: (e) => errors.push(e) });
    await waitFor(() => subscription.closed);
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatchObject({ code: 'instance_not_found' });
  });

  it('refreshes an instance handle on stream events', async () => {
    let reads = 0;
    let release: (() => void) | undefined;
    const mock = mockFetch((call) => {
      if (call.url.endsWith('/events')) {
        const encoder = new TextEncoder();
        return new Response(new ReadableStream<Uint8Array>({
          start(controller) {
            release = () => controller.enqueue(encoder.encode('event: transition\nid: 1\ndata: {}\n\n'));
          },
        }), { status: 200 });
      }
      if (call.url.includes('/protocol-instances/inst-1')) {
        reads += 1;
        return json({ ...DETAIL, revision: reads });
      }
      return undefined;
    });
    const client = createProtocolClient(DEFINITION, { auth: { token: 't' }, fetch: mock.fetch });
    const handle = client.instances.use('inst-1', { wallet: WALLET });
    const seen: number[] = [];
    const unsubscribe = handle.subscribe(() => {
      const revision = handle.get()?.instance.revision;
      if (revision !== undefined && seen[seen.length - 1] !== revision) seen.push(revision);
    });

    await waitFor(() => seen.length === 1 && release !== undefined);
    release!();
    await waitFor(() => seen.length === 2);
    expect(seen).toEqual([1, 2]);
    expect(handle.get()?.live).toBeUndefined();
    expect(mock.requests()[0]!.url).toBe(`${API}/protocol-instances/inst-1?wallet=${WALLET}`);

    unsubscribe();
    client.close();
  });

  it('adds the live stack row keyed by the instance account when the stack is connected', async () => {
    let detail = { ...DETAIL, accounts: { presale: "P1" } };
    const seen: Array<ReturnType<typeof handle.get>> = [];
    const closed: string[] = [];
    const mock = mockFetch((call) => {
      if (call.url.endsWith('/events')) return new Response(new ReadableStream(), { status: 200 });
      if (call.url.includes('/protocol-instances/inst-1')) return json(detail);
      return undefined;
    });
    const keys: string[] = [];
    const pushes = new Map<string, (row: unknown) => void>();
    const connectedStack = {
      views: {
        TokenRaiseInstance: {
          state: {
            use(key: string): AsyncIterable<unknown> {
              keys.push(key);
              return {
                [Symbol.asyncIterator]() {
                  return {
                    next: () => new Promise<IteratorResult<unknown>>((resolve) => {
                      pushes.set(key, (row) => resolve({ done: false, value: row }));
                    }),
                    return: async () => { closed.push(key); return { done: true, value: undefined }; },
                  };
                },
              };
            },
          },
        },
      },
    };
    const client = createProtocolClientWithMembers(
      { ...DEFINITION, liveStack: { stack: 'raiseStack', view: 'TokenRaiseInstance/state', keyAccount: 'presale' } },
      { auth: { token: 't' }, fetch: mock.fetch },
      { stacks: { raiseStack: connectedStack }, programs: {} }
    );
    const handle = client.instances.use('inst-1');
    const unsubscribe = handle.subscribe(() => seen.push(handle.get()));
    await waitFor(() => pushes.has("P1"));
    expect(keys).toEqual(["P1"]);
    pushes.get('P1')!({ raised: '42' });
    await waitFor(() => handle.get()?.live !== undefined);
    expect(handle.get()).toMatchObject({ instance: { id: 'inst-1' }, live: { raised: '42' } });
    await handle.refresh();
    expect(handle.get()?.live).toEqual({ raised: '42' });
    detail = { ...detail, accounts: { presale: 'P2' } };
    await handle.refresh();
    expect(handle.get()?.live).toBeUndefined();
    await waitFor(() => pushes.has('P2'));
    pushes.get('P1')!({ raised: 'stale' });
    await Promise.resolve();
    expect(handle.get()?.live).toBeUndefined();
    pushes.get('P2')!({ raised: '84' });
    await waitFor(() => handle.get()?.live !== undefined);
    detail = { ...detail, accounts: {} as { presale: string } };
    await handle.refresh();
    expect(handle.get()?.live).toBeUndefined();
    pushes.get('P2')!({ raised: 'stale' });
    await Promise.resolve();
    expect(handle.get()?.live).toBeUndefined();
    expect(closed).toEqual(['P1', 'P2']);
    expect(seen.every((s) => !s?.live || (s.live as { raised: string }).raised === (s.instance.accounts.presale === 'P1' ? '42' : '84'))).toBe(true);
    unsubscribe();
    client.close();
  });
});

describe('squads onboarding', () => {
  const MULTISIG = 'Mu1tisig11111111111111111111111111111111111';
  const PROPOSER = 'Proposer11111111111111111111111111111111111';
  const SQUADS = `${API}/protocol-deployments/${DEPLOYMENT_ID}/squads`;

  it('reads the proposer and checks status', async () => {
    const mock = mockFetch((call) => {
      if (call.url === `${SQUADS}/proposer` && call.method === 'GET') {
        return json({ pubkey: PROPOSER, permissions: 'initiate', rootKeyId: 'proposer-root-1' });
      }
      if (call.url.startsWith(`${SQUADS}/status?`)) {
        return json({
          ready: false,
          member: false,
          threshold: 2,
          multisig: MULTISIG,
          proposer: PROPOSER,
          problem: 'not_member',
        });
      }
      return undefined;
    });
    const client = createProtocolClient(DEFINITION, {
      auth: { publishableKey: 'a4_pk_test' },
      fetch: mock.fetch,
    });

    expect(await client.squads.proposer()).toEqual({
      pubkey: PROPOSER,
      permissions: 'initiate',
      rootKeyId: 'proposer-root-1',
    });
    // Arete builds no Squads transactions.
    expect('addProposer' in client.squads).toBe(false);
    const status = await client.squads.status(MULTISIG);
    expect(status).toMatchObject({ ready: false, member: false, mask: null, problem: 'not_member' });

    const requests = mock.requests();
    expect(requests.map((call) => `${call.method} ${call.url}`)).toEqual([
      `GET ${SQUADS}/proposer`,
      `GET ${SQUADS}/status?multisig=${MULTISIG}`,
    ]);
    expect(requests.every((call) => call.headers.get('authorization') === 'Bearer tok-1')).toBe(true);
    client.close();
  });

  it('surfaces coded refusals', async () => {
    const mock = mockFetch((call) =>
      call.url.startsWith(`${SQUADS}/status?`)
        ? json({ error: 'not used', code: 'protocol-squads-not-used' }, 409)
        : undefined
    );
    const client = createProtocolClient(DEFINITION, { auth: { token: 't' }, fetch: mock.fetch });
    await expect(client.squads.status(MULTISIG)).rejects.toMatchObject({
      details: expect.objectContaining({ wireErrorCode: 'protocol-squads-not-used' }),
    });
    await expect(client.squads.status('')).rejects.toMatchObject({ code: 'INVALID_CONFIG' });
    client.close();
  });

  it('delivers arete.squads.proposerMissing as a typed stream event', async () => {
    const data = {
      id: 7,
      deploymentId: DEPLOYMENT_ID,
      instanceId: 'inst-1',
      outboxId: 3,
      step: 'finalize',
      multisig: MULTISIG,
      proposer: PROPOSER,
      problem: 'not_member',
      at: '2026-10-14T00:00:00Z',
    };
    const mock = mockFetch((call) =>
      call.url.endsWith('/events')
        ? sseResponse([`event: arete.squads.proposerMissing\nid: 1-2-3-7\ndata: ${JSON.stringify(data)}\n\n`])
        : undefined
    );
    const client = createProtocolClient(DEFINITION, { auth: { token: 't' }, fetch: mock.fetch });
    const events: unknown[] = [];
    const subscription = client.events('inst-1', { onEvent: (event) => events.push(event) });
    await waitFor(() => events.length >= 1);
    subscription.close();
    expect(events[0]).toEqual({ type: 'arete.squads.proposerMissing', id: '1-2-3-7', data });
    client.close();
  });
});

describe('protocol receipts', () => {
  const vectors = JSON.parse(
    readFileSync(
      new URL('../../../../test-vectors/protocol-client/execution-receipt-v1.json', import.meta.url),
      'utf8'
    )
  );

  it('fetches a receipt and the keys, and both verifiers accept them', async () => {
    const receipt = vectors.verifyCases[0].receipt;
    const mock = mockFetch((call) => {
      if (call.url === `${API}/protocol-receipts/keys`) return json(vectors.keySet);
      if (call.url === `${API}/protocol-receipts/${receipt.signature}`) return json(receipt);
      if (call.url === `${API}/protocol-receipts/missing`) {
        return json({ error: 'no receipt', code: 'protocol-receipt-not-found' }, 404);
      }
      return undefined;
    });
    const client = createProtocolClient(DEFINITION, { auth: { token: 't' }, fetch: mock.fetch });
    const fetched = await client.getReceipt(receipt.signature);
    const keys = await client.getReceiptKeys();
    expect(await verifyReceipt(fetched, keys)).toEqual({ valid: true, keyId: 'test-receipt-1' });
    expect(await verifyReceiptOnChain(fetched, vectors.onChainCases[0].transaction)).toEqual({ valid: true });
    await expect(client.getReceipt('missing')).rejects.toMatchObject({
      code: 'protocol-receipt-not-found',
      details: { status: 404 },
    });
    expect(mock.requests().every((call) => call.method === 'GET')).toBe(true);
  });
});
