import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  AreteError,
  createSquadsClient,
  squadsDashboardUrl,
  squadsSetupRequired,
  SQUADS_DASHBOARD_URL,
  SQUADS_SETUP_REQUIRED_CODE,
} from '../index';

const API = 'https://api.example.test/api';
const BASE = `${API}/account/squads`;
const MULTISIG = 'MsiG1111111111111111111111111111111111111111';
const MEMBER = 'So11111111111111111111111111111111111111112';
const SECRET = 'a4_sk_test_secret';

interface Call {
  url: string;
  method: string;
  headers: Headers;
  body: unknown;
}

function json(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), { status, headers: { 'Content-Type': 'application/json' } });
}

function mockFetch(answer: (call: Call) => Response = () => json({ ok: true })) {
  const calls: Call[] = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const text = typeof init?.body === 'string' ? init.body : undefined;
    const call: Call = {
      url: String(input),
      method: init?.method ?? 'GET',
      headers: new Headers(init?.headers),
      body: text === undefined ? undefined : JSON.parse(text),
    };
    calls.push(call);
    return answer(call);
  });
  return { calls, fetch: fetchMock as unknown as typeof fetch };
}

afterEach(() => {
  vi.unstubAllEnvs();
});

describe('createSquadsClient', () => {
  it('calls every account route with the secret key as bearer', async () => {
    const { calls, fetch } = mockFetch((call) => (call.method === 'DELETE' ? new Response(null, { status: 204 }) : json({ ok: true })));
    const squads = createSquadsClient({ auth: { secretKey: SECRET }, apiUrl: `${API}/`, fetch });

    await squads.status();
    await squads.notifications();
    await squads.get(MULTISIG);
    await squads.attach({ address: MULTISIG, label: 'Treasury' });
    await squads.create({
      members: [{ pubkey: MEMBER, permissions: ['initiate', 'vote', 'execute'] }],
      threshold: 1,
      creator: MEMBER,
      label: 'Ops',
    });
    await squads.addProposer(MULTISIG, { member: MEMBER });
    await squads.approve(MULTISIG, 3, { member: MEMBER, transactionFormat: 'v1' });
    await squads.execute(MULTISIG, 3, { member: MEMBER });
    await squads.removeProposer(MULTISIG, { member: MEMBER });
    await expect(squads.forget(MULTISIG)).resolves.toBeUndefined();
    await squads.submit({ transactionId: 'tx-1', signedTransaction: 'AQID' });

    expect(calls.map((call) => [call.method, call.url, call.body])).toEqual([
      ['GET', BASE, undefined],
      ['GET', `${BASE}/notifications`, undefined],
      ['GET', `${BASE}/multisigs/${MULTISIG}`, undefined],
      ['POST', `${BASE}/multisigs`, { address: MULTISIG, label: 'Treasury' }],
      ['POST', `${BASE}/multisigs/create`, {
        members: [{ pubkey: MEMBER, permissions: ['initiate', 'vote', 'execute'] }],
        threshold: 1,
        creator: MEMBER,
        label: 'Ops',
      }],
      ['POST', `${BASE}/multisigs/${MULTISIG}/add-proposer`, { member: MEMBER }],
      ['POST', `${BASE}/multisigs/${MULTISIG}/proposals/3/approve`, { member: MEMBER, transactionFormat: 'v1' }],
      ['POST', `${BASE}/multisigs/${MULTISIG}/proposals/3/execute`, { member: MEMBER }],
      ['POST', `${BASE}/multisigs/${MULTISIG}/remove-proposer`, { member: MEMBER }],
      ['DELETE', `${BASE}/multisigs/${MULTISIG}`, undefined],
      ['POST', `${BASE}/transactions`, { transactionId: 'tx-1', signedTransactionBase64: 'AQID' }],
    ]);
    for (const call of calls) expect(call.headers.get('Authorization')).toBe(`Bearer ${SECRET}`);
  });

  it('base64-encodes signed bytes and uses a dashboard token as is', async () => {
    const { calls, fetch } = mockFetch();
    const squads = createSquadsClient({ auth: { getToken: async () => ({ token: 'dash-token' }) }, apiUrl: API, fetch });
    await squads.submit({ transactionId: 'tx-2', signedTransaction: new Uint8Array([1, 2, 3, 4]) });
    expect(calls[0]!.body).toEqual({ transactionId: 'tx-2', signedTransactionBase64: 'AQIDBA==' });
    expect(calls[0]!.headers.get('Authorization')).toBe('Bearer dash-token');
  });

  it('defaults the API to ARETE_API_URL, then api.arete.run', async () => {
    const { calls, fetch } = mockFetch();
    vi.stubEnv('ARETE_API_URL', 'https://staging.example.test/');
    await createSquadsClient({ auth: { secretKey: SECRET }, fetch }).status();
    vi.stubEnv('ARETE_API_URL', '');
    await createSquadsClient({ auth: { secretKey: SECRET }, fetch }).status();
    expect(calls.map((call) => call.url)).toEqual([
      'https://staging.example.test/api/account/squads',
      'https://api.arete.run/api/account/squads',
    ]);
  });

  it('refuses publishable keys and missing input', async () => {
    const { fetch } = mockFetch();
    expect(() => createSquadsClient({ auth: { publishableKey: 'a4_pk_test' }, fetch })).toThrow(/publishable keys are refused/);
    const squads = createSquadsClient({ auth: { secretKey: SECRET }, apiUrl: API, fetch });
    await expect(squads.addProposer(MULTISIG, { member: '' })).rejects.toThrow(/requires a member/);
    await expect(squads.approve(MULTISIG, -1, { member: MEMBER })).rejects.toThrow(/non-negative integer/);
    await expect(squads.get('')).rejects.toThrow(/multisig must be a non-empty string/);
  });

  it('maps refusals to AreteError with the wire code', async () => {
    const { fetch } = mockFetch(() => json({ code: 'protocol-squads-multisig-active', error: 'remove the proposer first' }, 409));
    const squads = createSquadsClient({ auth: { secretKey: SECRET }, apiUrl: API, fetch });
    const error = await squads.forget(MULTISIG).catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(AreteError);
    expect((error as AreteError).message).toBe('Squads forget returned 409: remove the proposer first');
    expect((error as AreteError).details).toMatchObject({ status: 409, wireErrorCode: 'protocol-squads-multisig-active' });
  });
});

describe('squadsSetupRequired', () => {
  const body = {
    code: SQUADS_SETUP_REQUIRED_CODE,
    message: `Step 'finalize' proposes to Squads multisig ${MULTISIG}, which is not set up for your account (not_member).`,
    details: {
      step: 'finalize',
      multisig: MULTISIG,
      state: 'pending',
      problem: 'not_member',
      proposer: MEMBER,
      setupUrl: squadsDashboardUrl(MULTISIG),
      commands: { status: `a4 squads status ${MULTISIG}` },
    },
  };

  it('reads the refusal from an AreteError or a raw body', () => {
    const error = new AreteError('Deploy returned 409', SQUADS_SETUP_REQUIRED_CODE, {
      status: 409,
      wireErrorCode: SQUADS_SETUP_REQUIRED_CODE,
      body,
    });
    expect(squadsSetupRequired(error)).toEqual({ message: body.message, details: body.details });
    expect(squadsSetupRequired(body)?.details.setupUrl).toBe(`${SQUADS_DASHBOARD_URL}/${MULTISIG}`);
    expect(squadsSetupRequired({ ...body, code: 'other' })).toBeUndefined();
    expect(squadsSetupRequired(new Error('nope'))).toBeUndefined();
  });

  it('builds dashboard links', () => {
    expect(squadsDashboardUrl()).toBe('https://app.arete.run/dashboard/squads');
    expect(squadsDashboardUrl(MULTISIG)).toBe(`https://app.arete.run/dashboard/squads/${MULTISIG}`);
  });
});
