import { describe, expect, it } from 'vitest';

import { lamportsToSol, plainDecimal } from './amounts';
import { definePolicy, PolicyError } from './policy';
import { rpcSimulator } from './simulator';

const MINT = 'LbUiWL3xVV8hTFYBVdbTNrpDo41NKS6o3LHHuDzjfcY';

describe('definePolicy', () => {
  it('accepts bigint token caps', () => {
    const policy = definePolicy({
      deployments: ['d'],
      maxSolPerTransaction: '0.5',
      maxTokenAmounts: { [MINT]: 340282366920938463463374607431768211455n },
    });
    expect(policy.maxTokenAmounts?.get(MINT)).toBe(340282366920938463463374607431768211455n);
  });

  it('refuses token caps a JavaScript number cannot hold exactly, or above u128', () => {
    expect(() =>
      definePolicy({ deployments: ['d'], maxSolPerTransaction: 1, maxTokenAmounts: { [MINT]: 2 ** 60 } }),
    ).toThrow(PolicyError);
    expect(() =>
      definePolicy({
        deployments: ['d'],
        maxSolPerTransaction: 1,
        maxTokenAmounts: { [MINT]: 340282366920938463463374607431768211456n },
      }),
    ).toThrow(PolicyError);
  });

  it('names the field at fault', () => {
    expect(() => definePolicy({ deployments: ['d'], maxSolPerTransaction: 1, programs: ['nope'] })).toThrow(
      "policy.programs: 'nope' is not a program id",
    );
  });
});

describe('amounts', () => {
  it('formats lamports as SOL', () => {
    expect(lamportsToSol(1_500_000_000n)).toBe('1.5');
    expect(lamportsToSol(-5000n)).toBe('-0.000005');
    expect(lamportsToSol(2_000_000_000n)).toBe('2');
  });

  it('writes numbers without exponents', () => {
    expect(plainDecimal(1e-7)).toBe('0.0000001');
    expect(plainDecimal(1.5e-9)).toBe('0.0000000015');
    expect(plainDecimal(1e21)).toBe('1000000000000000000000');
    expect(plainDecimal(1.25e22)).toBe('12500000000000000000000');
    expect(plainDecimal(0.05)).toBe('0.05');
  });
});

describe('rpcSimulator', () => {
  function fakeFetch(body: unknown, seen: unknown[] = []): typeof fetch {
    return (async (_url: string, init: RequestInit) => {
      seen.push(JSON.parse(String(init.body)));
      return new Response(typeof body === 'string' ? body : JSON.stringify(body));
    }) as unknown as typeof fetch;
  }

  it('posts a JSON-RPC request and returns the result', async () => {
    const seen: unknown[] = [];
    const simulator = rpcSimulator('https://rpc.example', {
      fetch: fakeFetch({ jsonrpc: '2.0', id: 1, result: { value: [] } }, seen),
    });
    await expect(simulator.call('getMultipleAccounts', [['a']])).resolves.toEqual({ value: [] });
    expect(seen).toEqual([{ jsonrpc: '2.0', id: 1, method: 'getMultipleAccounts', params: [['a']] }]);
  });

  it('rejects with the JSON-RPC error message', async () => {
    const simulator = rpcSimulator('https://rpc.example', {
      fetch: fakeFetch({ jsonrpc: '2.0', id: 1, error: { code: -32005, message: 'node is behind' } }),
    });
    await expect(simulator.call('simulateTransaction', [])).rejects.toThrow(
      'simulateTransaction failed: node is behind',
    );
  });

  it('gives up on an RPC that does not answer in time', async () => {
    const hanging = ((_url: string, init: RequestInit) =>
      new Promise((_resolve, reject) => {
        init.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')));
      })) as unknown as typeof fetch;
    await expect(rpcSimulator('https://rpc.example', { fetch: hanging, timeoutMs: 20 }).call('getSlot', [])).rejects.toThrow(
      'did not answer getSlot within 20ms',
    );
  });

  it('without a URL: the edge in dev mode, an error in production and direct mode', async () => {
    const previous = { ...process.env };
    try {
      delete process.env.ARETE_ENVIRONMENT;
      delete process.env.ARETE_DEV_URL;
      delete process.env.ARETE_DEV_ENV;
      expect(() => rpcSimulator()).toThrow(/needs a JSON-RPC URL in production/);
      expect(() => rpcSimulator(undefined, { environment: 'production' })).toThrow(/production/);

      const urls: string[] = [];
      const recording = (async (url: string) => {
        urls.push(url);
        return new Response(JSON.stringify({ jsonrpc: '2.0', id: 1, result: 1 }));
      }) as unknown as typeof fetch;
      await rpcSimulator(undefined, { environment: { kind: 'dev', url: 'http://127.0.0.1:8911' }, fetch: recording }).call('getSlot', []);
      process.env.ARETE_ENVIRONMENT = 'dev';
      process.env.ARETE_DEV_URL = 'http://localhost:8912/';
      await rpcSimulator(undefined, { fetch: recording }).call('getSlot', []);
      expect(urls).toEqual(['http://127.0.0.1:8911', 'http://localhost:8912']);

      delete process.env.ARETE_DEV_URL;
      process.env.ARETE_DEV_ENV = 'my-env';
      process.env.ARETE_API_KEY = 'a4_sk_simulatortest0000000000000000000000';
      expect(() => rpcSimulator()).toThrow(expect.objectContaining({ code: 'devenv-surface-unavailable' }));
    } finally {
      for (const name of ['ARETE_ENVIRONMENT', 'ARETE_DEV_URL', 'ARETE_DEV_ENV', 'ARETE_API_KEY']) {
        if (previous[name] === undefined) delete process.env[name];
        else process.env[name] = previous[name];
      }
    }
  });

  it('rejects invalid JSON and unreachable endpoints', async () => {
    await expect(
      rpcSimulator('https://rpc.example', { fetch: fakeFetch('<html>') }).call('getSlot', []),
    ).rejects.toThrow('invalid JSON');
    const failing = (async () => {
      throw new TypeError('fetch failed');
    }) as unknown as typeof fetch;
    await expect(rpcSimulator('https://rpc.example', { fetch: failing }).call('getSlot', [])).rejects.toThrow(
      'unreachable',
    );
  });
});
