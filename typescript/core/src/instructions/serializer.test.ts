import { describe, expect, it } from 'vitest';

import { serializeInstructionData } from './buffer-serializer';
import { encodeBase58 } from './pda';
import { referenceSerializeInstructionData } from './serializer-reference.fixture';
import { encodeInstructionData, type ArgSchema, type ArgType } from './serializer';

/**
 * The `Uint8Array` encoder must write exactly the bytes the `Buffer`
 * encoder wrote, and reject exactly the inputs it rejected.
 */

type Outcome = { ok: true; bytes: number[] } | { ok: false; range: boolean };

function run(encode: () => Uint8Array): Outcome {
  try {
    return { ok: true, bytes: [...encode()] };
  } catch (error) {
    return { ok: false, range: error instanceof RangeError };
  }
}

function expectSameAsReference(
  discriminator: Uint8Array,
  args: Record<string, unknown>,
  schema: ArgSchema[],
): Outcome {
  const expected = run(() => referenceSerializeInstructionData(discriminator, args, schema));
  const actual = run(() => encodeInstructionData(discriminator, args, schema));
  expect(actual, JSON.stringify({ schema, args }, (_, v) => (typeof v === 'bigint' ? `${v}n` : v))).toEqual(
    expected,
  );
  return actual;
}

/** Deterministic xorshift PRNG so failures reproduce. */
function rng(seed: number) {
  let state = seed >>> 0 || 1;
  const next = () => {
    state ^= state << 13;
    state >>>= 0;
    state ^= state >>> 17;
    state ^= state << 5;
    state >>>= 0;
    return state;
  };
  return {
    int: (max: number) => next() % max,
    float: () => (next() / 0xffffffff - 0.5) * 2 ** (next() % 80),
    big: (bits: number) => {
      let value = BigInt(0);
      for (let i = 0; i < Math.ceil(bits / 32); i += 1) value = (value << BigInt(32)) | BigInt(next());
      return value & ((BigInt(1) << BigInt(bits)) - BigInt(1));
    },
    bytes: (length: number) => Uint8Array.from({ length }, () => next() & 0xff),
  };
}

const DISCRIMINATOR = Uint8Array.from([1, 2, 3, 4, 5, 6, 7, 8]);
const PRIMITIVES = [
  'u8', 'u16', 'u32', 'u64', 'u128',
  'i8', 'i16', 'i32', 'i64', 'i128',
  'f32', 'f64', 'bool', 'string', 'pubkey', 'bytes',
] as const;

const BOUNDARIES: Record<(typeof PRIMITIVES)[number], unknown[]> = {
  u8: [0, 1, 127, 128, 255, 256, -1, 1.7, '7', NaN],
  i8: [0, -1, -128, 127, 128, -129],
  u16: [0, 1, 0xffff, 0x10000, -1, 1.9, '513', NaN, Infinity],
  i16: [0, -1, -32768, 32767, 32768, -32769, -1.5],
  u32: [0, 0xffffffff, 0x100000000, -1, 3.99],
  i32: [0, -1, -(2 ** 31), 2 ** 31 - 1, 2 ** 31, -(2 ** 31) - 1],
  u64: [0, 1, '18446744073709551615', '18446744073709551616', -1, BigInt(2) ** BigInt(63), BigInt(42)],
  i64: [0, -1, '-9223372036854775808', '9223372036854775807', '9223372036854775808', '-9223372036854775809'],
  u128: [0, 1, (BigInt(1) << BigInt(128)) - BigInt(1), BigInt(1) << BigInt(128), -1, BigInt(1) << BigInt(64)],
  i128: [0, -1, -(BigInt(1) << BigInt(127)), (BigInt(1) << BigInt(127)) - BigInt(1), BigInt(1) << BigInt(127), -(BigInt(1) << BigInt(64))],
  f32: [0, -0, 1.5, -2.25, 3.4e38, 1e39, Number.MIN_VALUE, Infinity, -Infinity, NaN, 0.1],
  f64: [0, -0, 0.1, -1e308, Number.MAX_VALUE, Number.MIN_VALUE, Infinity, NaN],
  bool: [true, false, 0, 1, 'x', ''],
  string: ['', 'hi', 'héllo wörld', '日本語', '\u{1F600}', '\uD800', 'a\u0000b', [104, 105], Uint8Array.of(1, 2), 5, null, {}],
  pubkey: ['11111111111111111111111111111111', 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA', 'abc', new Uint8Array(32).fill(7), new Uint8Array(31), 7],
  bytes: [new Uint8Array(0), Uint8Array.of(1, 2, 3), [1, 2, 300, -1], 'nope'],
};

describe('encodeInstructionData matches the Buffer encoder', () => {
  it('writes the same bytes for every primitive at its boundaries', () => {
    let compared = 0;
    for (const type of PRIMITIVES) {
      for (const value of BOUNDARIES[type]) {
        expectSameAsReference(DISCRIMINATOR, { v: value }, [{ name: 'v', type }]);
        compared += 1;
      }
    }
    expect(compared).toBeGreaterThan(100);
  });

  it('rejects out-of-range integers with a RangeError, as Buffer did', () => {
    for (const [type, value] of [
      ['u16', 0x10000],
      ['i32', 2 ** 31],
      ['u64', -1],
      ['u128', BigInt(1) << BigInt(128)],
      ['i128', BigInt(1) << BigInt(127)],
    ] as const) {
      const outcome = expectSameAsReference(DISCRIMINATOR, { v: value }, [{ name: 'v', type }]);
      expect(outcome).toEqual({ ok: false, range: true });
    }
  });

  it('writes the same bytes for composite schemas', () => {
    const inner: ArgType = {
      struct: [
        { name: 'id', type: 'u64' },
        { name: 'label', type: 'string' },
        { name: 'flags', type: { array: ['bool', 3] } },
      ],
    };
    const schema: ArgSchema[] = [
      { name: 'amount', type: 'u64' },
      { name: 'items', type: { vec: inner } },
      { name: 'wide', type: { vecU64Len: 'u16' } },
      { name: 'seed', type: 'stringU64Len' },
      { name: 'maybe', type: { option: 'i128' } },
      { name: 'none', type: { option: 'pubkey' } },
      { name: 'pair', type: { tuple: ['u8', 'string'] } },
      { name: 'labels', type: { hashMap: ['string', 'u32'] } },
      {
        name: 'mode',
        type: {
          enum: [
            'idle',
            { name: 'transfer', fields: [{ name: 'to', type: 'pubkey' }, { name: 'amount', type: 'u64' }] },
            { name: 'pair', tuple: ['u8', 'u16'] },
          ],
        },
      },
      { name: 'raw', type: 'bytes' },
    ];
    const args = {
      amount: '1000000000000',
      items: [
        { id: 1, label: 'one', flags: [true, false, true] },
        { id: BigInt(2) ** BigInt(64) - BigInt(1), label: 'ünïcødé', flags: [false, false, false] },
      ],
      wide: [1, 65535],
      seed: 'vault-seed',
      maybe: -(BigInt(1) << BigInt(100)),
      none: null,
      pair: [9, 'x'],
      labels: { z: 1, a: 2, 'é': 3, B: 4, '\u{1F600}': 5, '￿': 6 },
      mode: { transfer: { to: '11111111111111111111111111111111', amount: 5 } },
      raw: Uint8Array.of(9, 8, 7),
    };
    const outcome = expectSameAsReference(DISCRIMINATOR, args, schema);
    expect(outcome.ok).toBe(true);
    for (const mode of ['idle', 0, { pair: [1, 2] }, 'transfer', { nope: 1 }, 3]) {
      expectSameAsReference(DISCRIMINATOR, { ...args, mode }, schema);
    }
  });

  it('writes the same bytes for randomized values', () => {
    const random = rng(0x5eed);
    const valueFor = (type: (typeof PRIMITIVES)[number]): unknown => {
      switch (type) {
        case 'u8': return random.int(256);
        case 'i8': return random.int(256) - 128;
        case 'u16': return random.int(0x10000);
        case 'i16': return random.int(0x10000) - 0x8000;
        case 'u32': return random.int(0xffffffff);
        case 'i32': return random.int(0xffffffff) - 0x80000000;
        case 'u64': return random.big(64);
        case 'i64': return random.big(64) - (BigInt(1) << BigInt(63));
        case 'u128': return random.big(128);
        case 'i128': return random.big(128) - (BigInt(1) << BigInt(127));
        case 'f32':
        case 'f64': return random.float();
        case 'bool': return random.int(2) === 1;
        case 'string': return String.fromCodePoint(...Array.from({ length: random.int(12) }, () => random.int(0x2ffff)));
        case 'pubkey': return random.int(2) === 0 ? random.bytes(32) : encodeBase58(random.bytes(32));
        case 'bytes': return random.bytes(random.int(40));
      }
    };
    for (let i = 0; i < 2000; i += 1) {
      const type = PRIMITIVES[random.int(PRIMITIVES.length)]!;
      const wrapped = random.int(3);
      if (wrapped === 0) {
        expectSameAsReference(DISCRIMINATOR, { v: valueFor(type) }, [{ name: 'v', type }]);
      } else if (wrapped === 1) {
        const values = Array.from({ length: random.int(5) }, () => valueFor(type));
        expectSameAsReference(DISCRIMINATOR, { v: values }, [{ name: 'v', type: { vec: type } }]);
      } else {
        const map = Object.fromEntries(
          Array.from({ length: random.int(5) }, () => [valueFor('string') as string, valueFor(type)]),
        );
        expectSameAsReference(DISCRIMINATOR, { v: map }, [{ name: 'v', type: { hashMap: ['string', type] } }]);
      }
    }
  });

  it('keeps the root serializeInstructionData a Buffer with the same bytes', () => {
    const schema: ArgSchema[] = [{ name: 'amount', type: 'u64' }, { name: 'memo', type: 'string' }];
    const args = { amount: 42, memo: 'gm' };
    const data = serializeInstructionData(DISCRIMINATOR, args, schema);
    expect(typeof (data as { readUInt8?: unknown }).readUInt8).toBe('function');
    expect([...data]).toEqual([...referenceSerializeInstructionData(DISCRIMINATOR, args, schema)]);
  });
});
