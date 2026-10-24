import { describe, expect, it, vi } from 'vitest';

import { createPlatformHttpClient } from './platform-http';
import { AreteError } from './types';

function fakeFetch(answer: (url: string, init: RequestInit) => Response) {
  const calls: { url: string; init: RequestInit }[] = [];
  const impl = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    calls.push({ url, init: init ?? {} });
    return answer(url, init ?? {});
  });
  return { calls, fetch: impl as unknown as typeof fetch };
}

const json = (status: number, body: unknown) =>
  new Response(body === undefined ? '' : JSON.stringify(body), { status });

describe('createPlatformHttpClient', () => {
  it('sends a secret key as the bearer and parses JSON', async () => {
    const { calls, fetch } = fakeFetch(() => json(200, { setup: 'ready' }));
    const client = createPlatformHttpClient({
      apiUrl: 'https://api.example.test',
      auth: { secretKey: 'a4_sk_test' },
      fetch,
    });
    expect(client.apiUrl).toBe('https://api.example.test/api');
    const answer = await client.request({ path: '/account/squads' });
    expect(answer).toEqual({ status: 200, body: { setup: 'ready' } });
    expect(calls[0]!.url).toBe('https://api.example.test/api/account/squads');
    expect(new Headers(calls[0]!.init.headers).get('authorization')).toBe('Bearer a4_sk_test');
  });

  it('asks getToken for the bearer and again after a 401', async () => {
    let issued = 0;
    const getToken = vi.fn(async () => `user-${++issued}`);
    let first = true;
    const { calls, fetch } = fakeFetch(() => {
      if (first) {
        first = false;
        return json(401, { error: 'expired', code: 'token-expired' });
      }
      return json(201, { address: 'x' });
    });
    const client = createPlatformHttpClient({ auth: { getToken }, fetch });
    const answer = await client.request({ path: 'account/squads/multisigs', method: 'POST', body: { address: 'x' } });
    expect(answer.status).toBe(201);
    expect(calls.map((call) => new Headers(call.init.headers).get('authorization'))).toEqual([
      'Bearer user-1',
      'Bearer user-2',
    ]);
    expect(calls[1]!.init.body).toBe('{"address":"x"}');
  });

  it('rejects with the API problem', async () => {
    const { fetch } = fakeFetch(() =>
      json(409, { error: 'Remove the proposer first', code: 'protocol-squads-multisig-active', details: { a: 1 } })
    );
    const client = createPlatformHttpClient({ auth: { token: 'jwt' }, fetch });
    const error = await client.request({ path: '/account/squads/multisigs/x', method: 'DELETE' }).catch((e) => e);
    expect(error).toBeInstanceOf(AreteError);
    expect(error.details).toMatchObject({
      status: 409,
      wireErrorCode: 'protocol-squads-multisig-active',
      body: { details: { a: 1 } },
    });
  });

  it('refuses publishable keys and missing credentials', () => {
    const { fetch } = fakeFetch(() => json(200, {}));
    expect(() => createPlatformHttpClient({ auth: { publishableKey: 'a4_pk_x' }, fetch })).toThrow(/publishable/);
  });
});
