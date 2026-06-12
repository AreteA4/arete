import { describe, it, expect } from 'vitest';

import { normalizeSeedType, serializeSeedValue } from './seed-serializer';
import { decodeBase58 } from './pda';

const TOKEN_PROGRAM = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA';

describe('normalizeSeedType', () => {
  it('canonicalizes pubkey and string spellings', () => {
    for (const t of ['pubkey', 'Pubkey', 'publicKey', 'PublicKey', 'solana_pubkey::Pubkey']) {
      expect(normalizeSeedType(t)).toBe('pubkey');
    }
    for (const t of ['string', 'String', 'str']) {
      expect(normalizeSeedType(t)).toBe('string');
    }
  });

  it('passes integer widths through and rejects everything else', () => {
    expect(normalizeSeedType('u32')).toBe('u32');
    expect(normalizeSeedType('i64')).toBe('i64');
    expect(normalizeSeedType('u24')).toBeUndefined();
    expect(normalizeSeedType('Vec<u8>')).toBeUndefined();
    expect(normalizeSeedType(undefined)).toBeUndefined();
  });
});

describe('serializeSeedValue (typed)', () => {
  it('encodes integers little-endian at the declared width', () => {
    expect([...serializeSeedValue(1, 'u8')]).toEqual([1]);
    expect([...serializeSeedValue(0x0102, 'u16')]).toEqual([2, 1]);
    expect([...serializeSeedValue(7, 'u32')]).toEqual([7, 0, 0, 0]);
    expect([...serializeSeedValue(42n, 'u64')]).toEqual([42, 0, 0, 0, 0, 0, 0, 0]);
  });

  it('encodes negative signed integers in two\'s complement', () => {
    expect([...serializeSeedValue(-1n, 'i64')]).toEqual([
      0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    ]);
  });

  it('rejects values that overflow the declared width', () => {
    expect(() => serializeSeedValue(256, 'u8')).toThrow(/does not fit/);
    expect(() => serializeSeedValue(-1, 'u32')).toThrow(/does not fit/);
  });

  it('decodes pubkey seeds from base58 to 32 bytes', () => {
    const bytes = serializeSeedValue(TOKEN_PROGRAM, 'pubkey');
    expect(bytes.length).toBe(32);
    expect([...bytes]).toEqual([...decodeBase58(TOKEN_PROGRAM)]);
    // Path-qualified Rust spelling works too.
    expect([...serializeSeedValue(TOKEN_PROGRAM, 'solana_pubkey::Pubkey')]).toEqual([...bytes]);
  });

  it('rejects non-pubkey strings for pubkey seeds', () => {
    expect(() => serializeSeedValue('abc', 'pubkey')).toThrow(/expected 32/);
    expect(() => serializeSeedValue(42, 'pubkey')).toThrow(/base58 string/);
  });

  it('utf8-encodes typed string seeds without base58 guessing', () => {
    // 44 chars: heuristic path would base58-decode this; typed must not.
    const fortyFour = 'a'.repeat(44);
    expect([...serializeSeedValue(fortyFour, 'string')]).toEqual(
      [...new TextEncoder().encode(fortyFour)]
    );
  });
});

describe('serializeSeedValue (untyped heuristics)', () => {
  it('passes Uint8Array through', () => {
    const raw = Uint8Array.from([1, 2, 3]);
    expect(serializeSeedValue(raw)).toBe(raw);
  });

  it('tries base58 for 43/44-char strings and utf8 otherwise', () => {
    expect(serializeSeedValue(TOKEN_PROGRAM).length).toBe(32);
    expect([...serializeSeedValue('treasury')]).toEqual([
      ...new TextEncoder().encode('treasury'),
    ]);
  });

  it('encodes numbers as 8-byte little-endian', () => {
    expect([...serializeSeedValue(256)]).toEqual([0, 1, 0, 0, 0, 0, 0, 0]);
  });
});
