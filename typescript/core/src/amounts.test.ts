import { describe, it, expect, vi } from 'vitest';
import {
  parseUiAmountToRaw,
  formatRawToUi,
  toRawAmount,
  safeToRawAmount,
  getMintDecimals,
  resolveAmount,
  resolveAmountToRaw,
  resolveAmountsToRaw,
} from './amounts';
import type { ChainClient, MintAccountInfo } from './chain';

function fakeChain(decimals: number | null) {
  const mint = vi.fn(async (address: string): Promise<MintAccountInfo | null> => ({
    address,
    ownerProgram: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
    decimals,
    supply: null,
    mintAuthority: null,
    freezeAuthority: null,
  }));
  const chain = { mint } as unknown as ChainClient;
  return { chain, mint };
}

describe('parseUiAmountToRaw', () => {
  it('converts decimal strings without float math', () => {
    expect(parseUiAmountToRaw('1.5', 6)).toBe(1_500_000n);
    expect(parseUiAmountToRaw('0.000001', 6)).toBe(1n);
    expect(parseUiAmountToRaw('100', 6)).toBe(100_000_000n);
    expect(parseUiAmountToRaw(0, 6)).toBe(0n);
    expect(parseUiAmountToRaw('12345678901234567890', 0)).toBe(12345678901234567890n);
  });

  it('accepts trailing zero fraction digits beyond the mint decimals', () => {
    expect(parseUiAmountToRaw('1.120000000', 6)).toBe(1_120_000n);
  });

  it('rejects malformed and negative inputs', () => {
    expect(() => parseUiAmountToRaw('1.2.3', 6)).toThrow('Invalid UI amount');
    expect(() => parseUiAmountToRaw('abc', 6)).toThrow('Invalid UI amount');
    expect(() => parseUiAmountToRaw('-1', 6)).toThrow('Invalid UI amount');
    expect(() => parseUiAmountToRaw('', 6)).toThrow('Invalid UI amount');
  });

  it('rejects non-zero fractional digits below the mint precision', () => {
    expect(() => parseUiAmountToRaw('1.1234567', 6)).toThrow('more fractional digits');
  });
});

describe('formatRawToUi', () => {
  it('is the inverse of parseUiAmountToRaw', () => {
    expect(formatRawToUi(1_500_000n, 6)).toBe('1.5');
    expect(formatRawToUi(1n, 6)).toBe('0.000001');
    expect(formatRawToUi(0n, 6)).toBe('0');
    expect(formatRawToUi(100_000_000n, 6)).toBe('100');
    expect(formatRawToUi('2500000', 6)).toBe('2.5');
  });

  it('handles zero decimals and negatives', () => {
    expect(formatRawToUi(5n, 0)).toBe('5');
    expect(formatRawToUi(-1_500_000n, 6)).toBe('-1.5');
  });
});

describe('toRawAmount', () => {
  it('passes raw inputs through', () => {
    expect(toRawAmount(42n, 6)).toBe(42n);
    expect(toRawAmount({ raw: '25' }, 6)).toBe(25n);
    expect(toRawAmount({ raw: 7 }, 6)).toBe(7n);
  });

  it('scales ui inputs', () => {
    expect(toRawAmount({ ui: 2 }, 6)).toBe(2_000_000n);
    expect(toRawAmount({ ui: '0.25' }, 8)).toBe(25_000_000n);
  });
});

describe('safeToRawAmount', () => {
  it('returns the same raw value without throwing for valid input', () => {
    expect(safeToRawAmount({ ui: '1.25' }, 9)).toEqual({
      success: true,
      data: 1_250_000_000n,
    });
  });

  it('returns invalid UI and raw inputs as failures', () => {
    const invalidUi = safeToRawAmount({ ui: '1.2.3' }, 6);
    const excessivePrecision = safeToRawAmount({ ui: '1.0000001' }, 6);
    const invalidRaw = safeToRawAmount({ raw: 'not-an-integer' }, 6);

    expect(invalidUi).toMatchObject({ success: false });
    expect(excessivePrecision).toMatchObject({ success: false });
    expect(invalidRaw).toMatchObject({ success: false });
    if (!invalidUi.success) {
      expect(invalidUi.error).toBeInstanceOf(Error);
    }
  });
});

describe('getMintDecimals', () => {
  it('returns decimals from the chain read', async () => {
    const { chain } = fakeChain(9);
    await expect(getMintDecimals(chain, 'MintA')).resolves.toBe(9);
  });

  it('throws when the mint has no decimals on the endpoint', async () => {
    const { chain } = fakeChain(null);
    await expect(getMintDecimals(chain, 'MintA')).rejects.toThrow('missing decimals');
  });
});

describe('resolveAmount', () => {
  it('never fetches when decimals are provided', async () => {
    const { chain, mint } = fakeChain(6);
    await expect(
      resolveAmount(chain, { mint: 'MintA', amount: { ui: '1.5' }, decimals: 6 })
    ).resolves.toEqual({ raw: 1_500_000n, decimals: 6 });
    expect(mint).not.toHaveBeenCalled();
  });

  it('fetches decimals once for ui inputs when unknown', async () => {
    const { chain, mint } = fakeChain(6);
    await expect(
      resolveAmount(chain, { mint: 'MintA', amount: { ui: '1.5' } })
    ).resolves.toEqual({ raw: 1_500_000n, decimals: 6 });
    expect(mint).toHaveBeenCalledTimes(1);
  });

  it('resolves raw inputs without needing decimals for conversion', async () => {
    const { chain, mint } = fakeChain(6);
    await expect(
      resolveAmount(chain, { mint: 'MintA', amount: 123n, decimals: 6 })
    ).resolves.toEqual({ raw: 123n, decimals: 6 });
    await expect(
      resolveAmount(chain, { mint: 'MintA', amount: { raw: '456' }, decimals: 6 })
    ).resolves.toEqual({ raw: 456n, decimals: 6 });
    expect(mint).not.toHaveBeenCalled();
  });
});

describe('resolveAmountToRaw', () => {
  it('never fetches when the input is already raw', async () => {
    const { chain, mint } = fakeChain(6);
    await expect(resolveAmountToRaw(chain, { mint: 'MintA', amount: 123n })).resolves.toBe(123n);
    await expect(resolveAmountToRaw(chain, { mint: 'MintA', amount: { raw: '456' } })).resolves.toBe(456n);
    expect(mint).not.toHaveBeenCalled();
  });

  it('fetches decimals for ui inputs when they are unknown', async () => {
    const { chain, mint } = fakeChain(6);
    await expect(resolveAmountToRaw(chain, { mint: 'MintA', amount: { ui: '1.5' } })).resolves.toBe(1_500_000n);
    expect(mint).toHaveBeenCalledTimes(1);
  });
});

describe('resolveAmountsToRaw', () => {
  it('resolves multiple named amounts and preserves their keys', async () => {
    const { chain, mint } = fakeChain(6);

    await expect(
      resolveAmountsToRaw(chain, {
        amountIn: { mint: 'MintA', amount: { ui: '1.5' } },
        minimumAmountOut: { mint: 'MintB', amount: 42n },
      })
    ).resolves.toEqual({
      amountIn: 1_500_000n,
      minimumAmountOut: 42n,
    });

    expect(mint).toHaveBeenCalledTimes(1);
    expect(mint).toHaveBeenCalledWith('MintA');
  });

  it('returns an empty object for an empty input map', async () => {
    const { chain, mint } = fakeChain(6);

    await expect(resolveAmountsToRaw(chain, {})).resolves.toEqual({});
    expect(mint).not.toHaveBeenCalled();
  });
});
