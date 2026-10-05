import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { createChainClient } from './chain';
import { createTransactionTransport } from './transactions';
import { createProgramReadTransport } from './program-read-transport';
import { managedU64, accountTombstone } from './managed-solana';

const fixture = (name: string) => JSON.parse(readFileSync(new URL(`../../../tests/fixtures/managed-solana-v1/${name}.json`, import.meta.url), 'utf8'));
const response = (body: unknown) => new Response(JSON.stringify(body), { status: 200 });

describe('managed Solana v1 shared fixtures', () => {
  it('round trips owner pages, exact amounts and unknown watermark', async () => {
    const cases = fixture('owner-token-accounts').cases.slice(0, 3);
    const mock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => response(cases[mock.mock.calls.length - 1].response));
    const chain = createChainClient('https://gateway.example', mock as typeof fetch);
    const page = await chain.ownerTokenAccounts(cases[0].request);
    expect(page.items[0].amount).toBe(18_446_744_073_709_551_615n);
    expect(page.items[0].delegatedAmount).toBe(9_007_199_254_740_993n);
    expect(page.discovery.watermark).toBeUndefined();
    const next = await chain.ownerTokenAccounts({ ...cases[0].request, cursor: page.nextCursor! });
    expect(next.items).toEqual([]);
    expect(next.discovery.watermark).toBe(9_007_199_254_740_993n);
    expect(await chain.ownerTokenAccounts(cases[2].request)).toMatchObject({ items: [], nextCursor: null });
    expect(JSON.parse(String(mock.mock.calls[1][1]?.body))).toEqual(cases[1].request);
  });

  it('keeps read options, actual context, aligned missing accounts and empty batches', async () => {
    const cases = fixture('contextual-accounts').cases;
    const mock = vi.fn(async (_input: unknown, init?: RequestInit) => {
      const input = JSON.parse(String(init?.body));
      return response(input.address ? cases[0].response : cases[2].response);
    });
    const chain = createChainClient('https://gateway.example', mock as typeof fetch);
    const read = await chain.accountWithContext(cases[0].request.address, { commitment: 'finalized', minContextSlot: 9_007_199_254_740_993n });
    expect(read.context?.slot).toBe(9_007_199_254_740_994n);
    expect(read.value?.lamports).toBe(9_007_199_254_740_993n);
    expect(JSON.parse(String(mock.mock.calls[0][1]?.body))).toEqual(cases[0].request);
    const batch = await chain.accountsWithContext(cases[2].request.addresses);
    expect(batch.value[1]).toBeNull();
    expect(batch.context?.slot).toBe(42n);
    expect(await chain.accountsWithContext([])).toEqual({ context: null, value: [] });
    expect(mock).toHaveBeenCalledTimes(2);
    await expect(chain.accountWithContext(cases[0].request.address, { minContextSlot: 18_446_744_073_709_551_615n })).rejects.toThrow('below minContextSlot');
  });

  it('preserves full legacy/versioned successful and failed transactions', async () => {
    for (const test of fixture('transactions').cases) {
      const transport = createTransactionTransport('https://gateway.example', async () => response(test.response));
      const actual = await transport.get('fixture-signature', { maxSupportedTransactionVersion: 1 });
      const expected = test.response.transaction;
      if (expected === null) { expect(actual).toBeNull(); continue; }
      expect(actual?.transaction).toEqual(expected.transaction);
      expect(actual?.meta).toEqual(expected.meta);
      expect(actual?.version).toEqual(expected.version);
      expect(actual?.metadataAvailable).toEqual(expected.metadataAvailable);
      if (actual?.meta) {
        expect(actual.meta.fee).toBe('5000');
        expect(actual.meta.postTokenBalances).toEqual(expected.meta.postTokenBalances);
      }
    }
    const malformed = createTransactionTransport('https://gateway.example', async () => response({}));
    await expect(malformed.get('signature')).rejects.toThrow('Invalid transaction response');
  });

  it('addresses contextual reads and native queries by exact release/type', async () => {
    const cases = fixture('native-position-query').cases;
    const mock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => response(cases[0].response));
    const transport = createProgramReadTransport({ kind: 'local-http', endpoint: 'https://read.example', release: { programReleaseHash: `arete:h1:program-release:sha256:${'a'.repeat(64)}`, programSpecHash: 'arete:h1:spec' }, fetch: mock as typeof fetch });
    await transport.read({ operation: 'nativeQuery', account: 'Position', query: cases[0].request });
    expect(mock.mock.calls[0][0]).toBe(`https://read.example${cases[0].path}`);
    expect(JSON.parse(String(mock.mock.calls[0][1]?.body))).toEqual(cases[0].request);
    await transport.read({ operation: 'fetchManyWithContext', account: 'Position', addresses: ['a'], options: { commitment: 'finalized', minContextSlot: 42n } });
    expect(JSON.parse(String(mock.mock.calls[1][1]?.body))).toEqual({ addresses: ['a'], options: { commitment: 'finalized', minContextSlot: '42' } });
    for (const bad of [42, '18446744073709551616', '-1', '+42', '1.5']) expect(() => managedU64(bad, 'slot')).toThrow();
  });
});


it('uses the canonical route manifest and exact deletion contract', () => {
  const manifest = fixture('manifest');
  expect(manifest.contract).toBe('managed-solana/v1');
  expect(manifest.bundleVersion).toBe('1.0.0');
  const definitions = fixture('schema').$defs;
  for (const route of manifest.routes) {
    expect(route.method).toBe('POST');
    expect(definitions[route.requestSchema]).toBeDefined();
    expect(definitions[route.responseSchema]).toBeDefined();
  }
  for (const test of fixture('wire-cases').cases) {
    expect(test.method).toBe(manifest.routes.find((route: { id: string }) => route.id === test.route).method);
  }
  const value = accountTombstone(fixture('account-deletion').tombstone);
  expect(value.slot).toBe(9_007_199_254_740_993n);
  expect(value.writeVersion).toBe(18_446_744_073_709_551_615n);
  expect(() => accountTombstone({ ...fixture('account-deletion').tombstone, writeVersion: 42 })).toThrow();
});
