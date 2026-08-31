import { describe, expect, it, vi } from 'vitest';
import { createChainClient } from './chain';

describe('ChainClient contextual balances', () => {
  it('keeps the legacy numeric lamports API unchanged', async () => {
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify({ lamports: 1_461_600 }), { status: 200 })
    );
    const chain = createChainClient('https://example.invalid', fetchMock as typeof fetch);

    await expect(chain.lamports('owner')).resolves.toBe(1_461_600);
    expect(fetchMock).toHaveBeenCalledWith(
      'https://example.invalid/chain/lamports/owner'
    );
  });

  it('returns exact native balances and serializes minContextSlot as a string', async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      expect(JSON.parse(String(init?.body))).toEqual({
        address: 'owner',
        minContextSlot: '9007199254740997',
      });
      return new Response(
        JSON.stringify({
          lamports: '9007199254740993',
          contextSlot: '9007199254740995',
        }),
        { status: 200 }
      );
    });
    const chain = createChainClient('https://example.invalid', fetchMock as typeof fetch);

    await expect(
      chain.nativeBalance('owner', { minContextSlot: 9_007_199_254_740_997n })
    ).resolves.toEqual({
      lamports: 9_007_199_254_740_993n,
      contextSlot: 9_007_199_254_740_995n,
    });
    expect(fetchMock).toHaveBeenCalledWith(
      'https://example.invalid/chain/native-balance',
      expect.objectContaining({
        method: 'POST',
        headers: { 'content-type': 'application/json' },
      })
    );
  });

  it('preserves raw token amounts and returns a bigint context slot', async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      expect(JSON.parse(String(init?.body))).toEqual({
        owner: 'owner',
        mint: 'mint',
        tokenProgram: 'token-program',
        minContextSlot: '9007199254740997',
      });
      return new Response(
        JSON.stringify({
          exists: true,
          address: 'token-account',
          owner: 'owner',
          mint: 'mint',
          tokenProgram: 'token-program',
          amount: '18446744073709551615',
          decimals: 9,
          uiAmountString: '18446744073.709551615',
          contextSlot: '9007199254740999',
        }),
        { status: 200 }
      );
    });
    const chain = createChainClient('https://example.invalid', fetchMock as typeof fetch);

    await expect(
      chain.balance(
        { owner: 'owner', mint: 'mint', tokenProgram: 'token-program' },
        { minContextSlot: 9_007_199_254_740_997n }
      )
    ).resolves.toEqual({
      exists: true,
      address: 'token-account',
      owner: 'owner',
      mint: 'mint',
      tokenProgram: 'token-program',
      amount: '18446744073709551615',
      decimals: 9,
      uiAmountString: '18446744073.709551615',
      contextSlot: 9_007_199_254_740_999n,
    });
  });

  it('rejects unsafe numeric minContextSlot values before fetching', async () => {
    const fetchMock = vi.fn();
    const chain = createChainClient('https://example.invalid', fetchMock as typeof fetch);

    await expect(
      chain.nativeBalance('owner', { minContextSlot: Number.MAX_SAFE_INTEGER + 1 })
    ).rejects.toThrow('minContextSlot must be a non-negative safe integer or bigint');
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

describe('ChainClient batch accounts', () => {
  it('posts every address and decodes each account payload', async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      expect(JSON.parse(String(init?.body))).toEqual({ addresses: ['addr-1', 'addr-2'] });
      return new Response(
        JSON.stringify({
          items: [
            {
              address: 'addr-1',
              ownerProgram: 'owner-program',
              lamports: '1461600',
              executable: false,
              data: 'AQID',
            },
            {
              address: 'addr-2',
              ownerProgram: 'owner-program',
              lamports: '0',
              executable: true,
              data: '',
            },
          ],
        }),
        { status: 200 }
      );
    });
    const chain = createChainClient('https://example.invalid', fetchMock as typeof fetch);

    const items = await chain.accounts(['addr-1', 'addr-2']);

    expect(items).toHaveLength(2);
    expect(items[0]).toMatchObject({
      address: 'addr-1',
      ownerProgram: 'owner-program',
      lamports: 1_461_600n,
      executable: false,
    });
    expect(Array.from(items[0]!.data)).toEqual([1, 2, 3]);
    expect(Array.from(items[1]!.data)).toEqual([]);
    expect(fetchMock).toHaveBeenCalledWith(
      'https://example.invalid/chain/accounts',
      expect.objectContaining({
        method: 'POST',
        headers: { 'content-type': 'application/json' },
      })
    );
  });

  it('keeps absent accounts as positional nulls', async () => {
    const fetchMock = vi.fn(async () =>
      new Response(
        JSON.stringify({
          items: [
            null,
            {
              address: 'addr-2',
              ownerProgram: 'owner-program',
              lamports: '7',
              executable: false,
              data: 'BAU=',
            },
          ],
        }),
        { status: 200 }
      )
    );
    const chain = createChainClient('https://example.invalid', fetchMock as typeof fetch);

    const items = await chain.accounts(['missing', 'addr-2']);

    expect(items[0]).toBeNull();
    expect(items[1]).toMatchObject({ address: 'addr-2', lamports: 7n });
    expect(Array.from(items[1]!.data)).toEqual([4, 5]);
  });

  it('resolves an empty batch without fetching', async () => {
    const fetchMock = vi.fn();
    const chain = createChainClient('https://example.invalid', fetchMock as typeof fetch);

    await expect(chain.accounts([])).resolves.toEqual([]);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('rejects batches over the address limit before fetching', async () => {
    const fetchMock = vi.fn();
    const chain = createChainClient('https://example.invalid', fetchMock as typeof fetch);
    const addresses = Array.from({ length: 101 }, (_value, index) => `addr-${index}`);

    await expect(chain.accounts(addresses)).rejects.toThrow(
      'addresses exceeds the 100-address limit for one batch'
    );
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('rejects a response whose item count differs from the request', async () => {
    const fetchMock = vi.fn(
      async () => new Response(JSON.stringify({ items: [null] }), { status: 200 })
    );
    const chain = createChainClient('https://example.invalid', fetchMock as typeof fetch);

    await expect(chain.accounts(['addr-1', 'addr-2'])).rejects.toThrow(
      "Invalid chain response for '/chain/accounts': expected 2 items, got 1"
    );
  });

  // 9007199254740993 is the first integer a JSON number cannot hold: as a number the wire value
  // would arrive already rounded to ...92, before any decoding here could intervene. The batch is
  // the custody-sweep path, so a silently rounded balance would be acted on.
  it('keeps lamports above 2^53 exact', async () => {
    const fetchMock = vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            items: [
              {
                address: 'whale',
                ownerProgram: 'owner-program',
                lamports: '9007199254740993',
                executable: false,
                data: '',
              },
            ],
          }),
          { status: 200 }
        )
    );
    const chain = createChainClient('https://example.invalid', fetchMock as typeof fetch);

    const items = await chain.accounts(['whale']);

    expect(items[0]!.lamports).toBe(9_007_199_254_740_993n);
  });

  // `readonly string[]` accepts a mutable array, so a caller can splice it mid-flight. The
  // response must be validated against what was actually requested, not against the array as it
  // looks when the promise resolves.
  it('validates against the addresses as requested, not as later mutated', async () => {
    const addresses = ['addr-1', 'addr-2'];
    const fetchMock = vi.fn(async () => {
      addresses.pop();
      return new Response(
        JSON.stringify({
          items: [
            { address: 'addr-1', ownerProgram: 'p', lamports: '1', executable: false, data: '' },
            { address: 'addr-2', ownerProgram: 'p', lamports: '2', executable: false, data: '' },
          ],
        }),
        { status: 200 }
      );
    });
    const chain = createChainClient('https://example.invalid', fetchMock as typeof fetch);

    const items = await chain.accounts(addresses);

    expect(items).toHaveLength(2);
    expect(JSON.parse(String(fetchMock.mock.calls[0]![1]!.body))).toEqual({
      addresses: ['addr-1', 'addr-2'],
    });
  });
});
