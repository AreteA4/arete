import { describe, expect, it } from 'vitest';

import { stringifyBigints } from './display';

describe('stringifyBigints', () => {
  it('converts bigints to strings', () => {
    expect(stringifyBigints(10n)).toBe('10');
  });

  it('maps arrays of bigints', () => {
    expect(stringifyBigints([1n, 2n, 3n])).toEqual(['1', '2', '3']);
  });

  it('recurses through plain objects', () => {
    const input = {
      roundId: 42n,
      state: { deployedPerSquare: [0n, 5n], totalMiners: 7n },
      label: 'round',
      nothing: null,
    };
    expect(stringifyBigints(input)).toEqual({
      roundId: '42',
      state: { deployedPerSquare: ['0', '5'], totalMiners: '7' },
      label: 'round',
      nothing: null,
    });
  });

  it('leaves non-bigint primitives untouched', () => {
    expect(stringifyBigints('abc')).toBe('abc');
    expect(stringifyBigints(12)).toBe(12);
    expect(stringifyBigints(null)).toBe(null);
    expect(stringifyBigints(undefined)).toBe(undefined);
  });

  it('does not descend into class instances', () => {
    class PublicKeyLike {
      constructor(public readonly value: bigint) {}
    }
    const key = new PublicKeyLike(9n);
    const result = stringifyBigints({ key });
    expect(result.key).toBe(key);
  });

  it('preserves the mapped type at the type level', () => {
    const input = { amounts: [1n, 2n], name: 'x' } as const;
    const result = stringifyBigints(input);
    const amounts: readonly string[] = result.amounts;
    const name: 'x' = result.name;
    expect(amounts).toEqual(['1', '2']);
    expect(name).toBe('x');
  });
});
