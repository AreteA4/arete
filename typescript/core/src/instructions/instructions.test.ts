import { describe, it, expect } from 'vitest';

import {
  findProgramAddress,
  findProgramAddressSync,
  decodeBase58,
  encodeBase58,
  createSeed,
} from './pda';
import { serializeInstructionData, type ArgSchema } from './serializer';
import {
  resolveAccounts,
  validateAccountResolution,
  type AccountMeta,
} from './account-resolver';
import {
  InstructionError,
  parseInstructionError,
  formatProgramError,
  normalizeTransactionError,
  TransactionExecutionError,
} from './error-parser';
import {
  createInstructionHandler,
  buildInstruction,
  executeInstruction,
} from './executor';
import type { WalletAdapter, BuiltInstruction, SendResult } from '../wallet/types';

// Well-known, valid 32-byte base58 program/account addresses.
const SYSTEM_PROGRAM = '11111111111111111111111111111111';
const TOKEN_PROGRAM = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA';
const WSOL_MINT = 'So11111111111111111111111111111111111111112';

describe('base58', () => {
  it('decodes the system program to 32 zero bytes', () => {
    const bytes = decodeBase58(SYSTEM_PROGRAM);
    expect(bytes.length).toBe(32);
    expect([...bytes].every((b) => b === 0)).toBe(true);
  });

  it('round-trips known program addresses', () => {
    for (const addr of [TOKEN_PROGRAM, WSOL_MINT, SYSTEM_PROGRAM]) {
      const decoded = decodeBase58(addr);
      expect(decoded.length).toBe(32);
      expect(encodeBase58(decoded)).toBe(addr);
    }
  });

  it('rejects invalid base58 characters', () => {
    expect(() => decodeBase58('0OIl')).toThrow(/Invalid base58/);
  });
});

describe('createSeed', () => {
  it('encodes a u64 number/bigint as 8 little-endian bytes', () => {
    expect([...createSeed(1n)]).toEqual([1, 0, 0, 0, 0, 0, 0, 0]);
    expect([...createSeed(256)]).toEqual([0, 1, 0, 0, 0, 0, 0, 0]);
  });

  it('encodes strings as utf-8 bytes', () => {
    expect([...createSeed('abc')]).toEqual([0x61, 0x62, 0x63]);
  });
});

describe('findProgramAddress', () => {
  it('is deterministic and off-curve (sync)', () => {
    const seeds = [createSeed('treasury')];
    const [addr1, bump1] = findProgramAddressSync(seeds, TOKEN_PROGRAM);
    const [addr2, bump2] = findProgramAddressSync(seeds, TOKEN_PROGRAM);
    expect(addr1).toBe(addr2);
    expect(bump1).toBe(bump2);
    // Canonical bump is the highest valid one, typically near 255.
    expect(bump1).toBeGreaterThanOrEqual(0);
    expect(bump1).toBeLessThanOrEqual(255);
    expect(decodeBase58(addr1).length).toBe(32);
  });

  it('matches between sync and async implementations', async () => {
    const seeds = [createSeed('miner'), decodeBase58(WSOL_MINT)];
    const [syncAddr, syncBump] = findProgramAddressSync(seeds, TOKEN_PROGRAM);
    const [asyncAddr, asyncBump] = await findProgramAddress(seeds, TOKEN_PROGRAM);
    expect(asyncAddr).toBe(syncAddr);
    expect(asyncBump).toBe(syncBump);
  });

  it('produces different addresses for different seeds', () => {
    const [a] = findProgramAddressSync([createSeed('a')], TOKEN_PROGRAM);
    const [b] = findProgramAddressSync([createSeed('b')], TOKEN_PROGRAM);
    expect(a).not.toBe(b);
  });

  it('rejects more than 16 seeds and oversized seeds', () => {
    const tooMany = Array.from({ length: 17 }, () => createSeed('x'));
    expect(() => findProgramAddressSync(tooMany, TOKEN_PROGRAM)).toThrow(/16 seeds/);
    expect(() => findProgramAddressSync([new Uint8Array(33)], TOKEN_PROGRAM)).toThrow(
      /maximum length/
    );
  });
});

describe('serializeInstructionData', () => {
  it('does not require an ambient Buffer global', () => {
    const ambientBuffer = globalThis.Buffer;
    try {
      Object.defineProperty(globalThis, 'Buffer', {
        configurable: true,
        value: undefined,
      });
      const data = serializeInstructionData(
        Uint8Array.from([0xaa]),
        { amount: 1n },
        [{ name: 'amount', type: 'u64' }]
      );
      expect([...data]).toEqual([0xaa, 1, 0, 0, 0, 0, 0, 0, 0]);
    } finally {
      Object.defineProperty(globalThis, 'Buffer', {
        configurable: true,
        value: ambientBuffer,
      });
    }
  });

  it('prefixes the discriminator and serializes primitives', () => {
    const schema: ArgSchema[] = [
      { name: 'amount', type: 'u64' },
      { name: 'flag', type: 'bool' },
      { name: 'count', type: 'u8' },
    ];
    const data = serializeInstructionData(
      Uint8Array.from([0xaa, 0xbb]),
      { amount: 1n, flag: true, count: 7 },
      schema
    );
    expect([...data]).toEqual([
      0xaa, 0xbb, // discriminator
      1, 0, 0, 0, 0, 0, 0, 0, // u64 = 1
      1, // bool = true
      7, // u8 = 7
    ]);
  });

  it('serializes a pubkey from a base58 string into 32 bytes', () => {
    const schema: ArgSchema[] = [{ name: 'mint', type: 'pubkey' }];
    const data = serializeInstructionData(new Uint8Array(0), { mint: SYSTEM_PROGRAM }, schema);
    expect(data.length).toBe(32);
    expect([...data].every((b) => b === 0)).toBe(true);
  });

  it('rejects pubkeys that do not decode to 32 bytes', () => {
    const schema: ArgSchema[] = [{ name: 'mint', type: 'pubkey' }];
    expect(() =>
      serializeInstructionData(new Uint8Array(0), { mint: 'abc' }, schema)
    ).toThrow(/Invalid pubkey/);
  });

  it('serializes strings with a length prefix', () => {
    const schema: ArgSchema[] = [{ name: 's', type: 'string' }];
    const data = serializeInstructionData(new Uint8Array(0), { s: 'hi' }, schema);
    expect([...data]).toEqual([2, 0, 0, 0, 0x68, 0x69]);
  });

  it('serializes option None and Some', () => {
    const schema: ArgSchema[] = [{ name: 'maybe', type: { option: 'u8' } }];
    expect([...serializeInstructionData(new Uint8Array(0), { maybe: null }, schema)]).toEqual([0]);
    expect([...serializeInstructionData(new Uint8Array(0), { maybe: 9 }, schema)]).toEqual([1, 9]);
  });

  it('serializes negative i64/i128 values in two\'s complement', () => {
    const schema: ArgSchema[] = [
      { name: 'small', type: 'i64' },
      { name: 'big', type: 'i128' },
    ];
    const data = serializeInstructionData(
      new Uint8Array(0),
      { small: -1n, big: -1n },
      schema
    );
    // -1 is all 0xff in two's complement at any width.
    expect(data.length).toBe(24);
    expect([...data].every((b) => b === 0xff)).toBe(true);

    const min = serializeInstructionData(
      new Uint8Array(0),
      { small: -(2n ** 63n), big: -(2n ** 127n) },
      schema
    );
    expect([...min.subarray(0, 8)]).toEqual([0, 0, 0, 0, 0, 0, 0, 0x80]);
    expect([...min.subarray(8, 24)]).toEqual([
      0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80,
    ]);
  });

  it('serializes f32/f64 little-endian and bytes with a length prefix', () => {
    const schema: ArgSchema[] = [
      { name: 'ratio', type: 'f32' },
      { name: 'price', type: 'f64' },
      { name: 'blob', type: 'bytes' },
    ];
    const data = serializeInstructionData(
      new Uint8Array(0),
      { ratio: 1.5, price: 2.5, blob: Uint8Array.from([9, 8]) },
      schema
    );
    const expectedF32 = Buffer.alloc(4);
    expectedF32.writeFloatLE(1.5);
    const expectedF64 = Buffer.alloc(8);
    expectedF64.writeDoubleLE(2.5);
    expect([...data.subarray(0, 4)]).toEqual([...expectedF32]);
    expect([...data.subarray(4, 12)]).toEqual([...expectedF64]);
    expect([...data.subarray(12)]).toEqual([2, 0, 0, 0, 9, 8]); // u32 len + raw

    // bytes accepts plain number arrays too.
    const fromArray = serializeInstructionData(
      new Uint8Array(0),
      { ratio: 0, price: 0, blob: [1] },
      schema
    );
    expect([...fromArray.subarray(12)]).toEqual([1, 0, 0, 0, 1]);
  });

  it('serializes structs in field order, including nesting', () => {
    const schema: ArgSchema[] = [
      {
        name: 'data',
        type: {
          struct: [
            { name: 'amount', type: 'u64' },
            { name: 'inner', type: { struct: [{ name: 'flag', type: 'bool' }] } },
          ],
        },
      },
    ];
    const data = serializeInstructionData(
      new Uint8Array(0),
      { data: { inner: { flag: true }, amount: 3n } }, // intentionally reordered keys
      schema
    );
    expect([...data]).toEqual([3, 0, 0, 0, 0, 0, 0, 0, 1]);
  });

  it('rejects structs with missing required fields', () => {
    const schema: ArgSchema[] = [
      { name: 'data', type: { struct: [{ name: 'amount', type: 'u64' }] } },
    ];
    expect(() =>
      serializeInstructionData(new Uint8Array(0), { data: {} }, schema)
    ).toThrow(/Missing required struct field "amount"/);
  });

  it('serializes fieldless enums by name or index', () => {
    const schema: ArgSchema[] = [{ name: 'status', type: { enum: ['active', 'sunset'] } }];
    expect([
      ...serializeInstructionData(new Uint8Array(0), { status: 'sunset' }, schema),
    ]).toEqual([1]);
    expect([
      ...serializeInstructionData(new Uint8Array(0), { status: 0 }, schema),
    ]).toEqual([0]);
    expect(() =>
      serializeInstructionData(new Uint8Array(0), { status: 'paused' }, schema)
    ).toThrow(/Unknown enum variant "paused"/);
  });

  it('serializes data-carrying enum variants (struct and tuple)', () => {
    const schema: ArgSchema[] = [
      {
        name: 'op',
        type: {
          enum: [
            'noop',
            { name: 'transfer', fields: [{ name: 'amount', type: 'u64' }] },
            { name: 'pair', tuple: ['u8', 'u16'] },
          ],
        },
      },
    ];
    expect([
      ...serializeInstructionData(
        new Uint8Array(0),
        { op: { transfer: { amount: 7n } } },
        schema
      ),
    ]).toEqual([1, 7, 0, 0, 0, 0, 0, 0, 0]);
    expect([
      ...serializeInstructionData(new Uint8Array(0), { op: { pair: [5, 0x0102] } }, schema),
    ]).toEqual([2, 5, 2, 1]);
    // Data-carrying variants cannot be passed as bare names.
    expect(() =>
      serializeInstructionData(new Uint8Array(0), { op: 'transfer' }, schema)
    ).toThrow(/carries data/);
  });

  it('serializes vec with a length prefix and fixed arrays without one', () => {
    const vecSchema: ArgSchema[] = [{ name: 'v', type: { vec: 'u8' } }];
    expect([...serializeInstructionData(new Uint8Array(0), { v: [1, 2] }, vecSchema)]).toEqual([
      2, 0, 0, 0, 1, 2,
    ]);

    const arrSchema: ArgSchema[] = [{ name: 'a', type: { array: ['u8', 3] } }];
    expect([...serializeInstructionData(new Uint8Array(0), { a: [4, 5, 6] }, arrSchema)]).toEqual([
      4, 5, 6,
    ]);
    expect(() =>
      serializeInstructionData(new Uint8Array(0), { a: [1] }, arrSchema)
    ).toThrow(/length mismatch/);
  });

  it('serializes vecU64Len with a u64 little-endian length prefix', () => {
    // Pinned to the Rust test `serializes_vec_u64_len_with_an_eight_byte_prefix`.
    const schema: ArgSchema[] = [{ name: 'v', type: { vecU64Len: 'u16' } }];
    expect([...serializeInstructionData(new Uint8Array(0), { v: [1, 258] }, schema)]).toEqual([
      2, 0, 0, 0, 0, 0, 0, 0, 1, 0, 2, 1,
    ]);

    // Empty vectors still carry the full 8-byte count.
    expect([...serializeInstructionData(new Uint8Array(0), { v: [] }, schema)]).toEqual([
      0, 0, 0, 0, 0, 0, 0, 0,
    ]);
  });

  it('serializes inline tuples recursively without a length prefix', () => {
    const schema: ArgSchema[] = [
      {
        name: 'checks',
        type: {
          vec: {
            tuple: [
              { enum: ['Create', 'Transfer'] },
              { struct: [{ name: 'flags', type: 'u32' }] },
            ],
          },
        },
      },
    ];
    const data = serializeInstructionData(
      new Uint8Array(0),
      {
        checks: [
          ['Transfer', { flags: 0x01020304 }],
          ['Create', { flags: 5 }],
        ],
      },
      schema
    );

    expect([...data]).toEqual([
      2, 0, 0, 0,
      1, 4, 3, 2, 1,
      0, 5, 0, 0, 0,
    ]);
  });

  it('rejects non-array and wrong-length inline tuple values', () => {
    const schema: ArgSchema[] = [{ name: 'pair', type: { tuple: ['u8', 'u16'] } }];

    expect(() =>
      serializeInstructionData(new Uint8Array(0), { pair: { left: 1, right: 2 } }, schema)
    ).toThrow(/Tuple value must be an array/);
    expect(() =>
      serializeInstructionData(new Uint8Array(0), { pair: [1] }, schema)
    ).toThrow(/Tuple length mismatch: expected 2, got 1/);
  });

  it('serializes string-key maps in deterministic key order', () => {
    const schema: ArgSchema[] = [{ name: 'labels', type: { hashMap: ['string', 'u8'] } }];
    const left = serializeInstructionData(new Uint8Array(0), { labels: { z: 1, a: 2 } }, schema);
    const right = serializeInstructionData(new Uint8Array(0), { labels: { a: 2, z: 1 } }, schema);

    expect([...left]).toEqual([...right]);
    expect([...left]).toEqual([
      2, 0, 0, 0,
      1, 0, 0, 0, 0x61, 2,
      1, 0, 0, 0, 0x7a, 1,
    ]);
  });

  it('serializes string-key maps with string values', () => {
    const schema: ArgSchema[] = [{ name: 'metadata', type: { hashMap: ['string', 'string'] } }];
    const data = serializeInstructionData(
      new Uint8Array(0),
      { metadata: { b: 'two', a: 'one' } },
      schema
    );

    expect([...data]).toEqual([
      2, 0, 0, 0,
      1, 0, 0, 0, 0x61,
      3, 0, 0, 0, 0x6f, 0x6e, 0x65,
      1, 0, 0, 0, 0x62,
      3, 0, 0, 0, 0x74, 0x77, 0x6f,
    ]);
  });

  it('serializes nested Metaplex-style authorization payload maps', () => {
    const schema: ArgSchema[] = [
      {
        name: 'authorizationData',
        type: {
          struct: [
            {
              name: 'payload',
              type: {
                struct: [
                  {
                    name: 'map',
                    type: {
                      hashMap: [
                        'string',
                        {
                          enum: [
                            { name: 'Pubkey', tuple: ['pubkey'] },
                            { name: 'Number', tuple: ['u64'] },
                          ],
                        },
                      ],
                    },
                  },
                ],
              },
            },
          ],
        },
      },
    ];
    const data = serializeInstructionData(
      new Uint8Array(0),
      {
        authorizationData: {
          payload: {
            map: {
              b: { Number: [7n] },
              a: { Pubkey: [SYSTEM_PROGRAM] },
            },
          },
        },
      },
      schema
    );

    expect([...data]).toEqual([
      2, 0, 0, 0,
      1, 0, 0, 0, 0x61,
      0, ...new Array(32).fill(0),
      1, 0, 0, 0, 0x62,
      1, 7, 0, 0, 0, 0, 0, 0, 0,
    ]);
  });

  it('rejects invalid map inputs and unsupported map key schemas', () => {
    const schema: ArgSchema[] = [{ name: 'labels', type: { hashMap: ['string', 'u8'] } }];
    expect(() =>
      serializeInstructionData(new Uint8Array(0), { labels: ['a'] }, schema)
    ).toThrow(/HashMap value must be a plain object/);

    const badKeySchema: ArgSchema[] = [{ name: 'labels', type: { hashMap: ['u64', 'u8'] } }];
    expect(() =>
      serializeInstructionData(new Uint8Array(0), { labels: { a: 1 } }, badKeySchema)
    ).toThrow(/must use the 'string' schema/);
  });
});

describe('resolveAccounts', () => {
  const wallet = { publicKey: WSOL_MINT } as WalletAdapter;

  it('resolves signer, known, and userProvided categories', () => {
    const metas: AccountMeta[] = [
      { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' },
      {
        name: 'systemProgram',
        isSigner: false,
        isWritable: false,
        category: 'known',
        knownAddress: SYSTEM_PROGRAM,
      },
      { name: 'mint', isSigner: false, isWritable: false, category: 'userProvided' },
    ];
    const result = resolveAccounts(metas, {}, {
      wallet,
      accounts: { mint: TOKEN_PROGRAM },
    });
    validateAccountResolution(result);
    expect(result.accounts.map((a) => a.name)).toEqual(['authority', 'systemProgram', 'mint']);
    expect(result.accounts[0]!.address).toBe(WSOL_MINT);
    expect(result.accounts[1]!.address).toBe(SYSTEM_PROGRAM);
    expect(result.accounts[2]!.address).toBe(TOKEN_PROGRAM);
  });

  it('prefers explicit signer overrides over the wallet public key', () => {
    const alternateSigner = TOKEN_PROGRAM;
    const metas: AccountMeta[] = [
      { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' },
      { name: 'mint', isSigner: false, isWritable: false, category: 'userProvided' },
    ];
    const result = resolveAccounts(metas, {}, {
      wallet,
      accounts: {
        authority: alternateSigner,
        mint: WSOL_MINT,
      },
    });

    validateAccountResolution(result);
    expect(result.accounts.map((account) => account.address)).toEqual([
      alternateSigner,
      WSOL_MINT,
    ]);
  });

  it('derives a PDA referencing a signer account and keeps original order', () => {
    const metas: AccountMeta[] = [
      { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' },
      {
        name: 'state',
        isSigner: false,
        isWritable: true,
        category: 'pda',
        pdaConfig: {
          programId: TOKEN_PROGRAM,
          seeds: [
            { type: 'literal', value: 'state' },
            { type: 'accountRef', accountName: 'authority' },
          ],
        },
      },
    ];
    const result = resolveAccounts(metas, {}, { wallet });
    validateAccountResolution(result);
    // Original (instruction) order is preserved even though PDAs resolve later.
    expect(result.accounts.map((a) => a.name)).toEqual(['authority', 'state']);

    const expected = findProgramAddressSync(
      [createSeed('state'), decodeBase58(WSOL_MINT)],
      TOKEN_PROGRAM
    )[0];
    expect(result.accounts[1]!.address).toBe(expected);
  });

  it('derives a PDA from raw byte seeds', () => {
    const raw = [1, 2, 255];
    const metas: AccountMeta[] = [
      {
        name: 'config',
        isSigner: false,
        isWritable: false,
        category: 'pda',
        pdaConfig: {
          programId: TOKEN_PROGRAM,
          seeds: [{ type: 'bytes', value: raw }],
        },
      },
    ];
    const result = resolveAccounts(metas, {}, {});
    validateAccountResolution(result);

    const expected = findProgramAddressSync([Uint8Array.from(raw)], TOKEN_PROGRAM)[0];
    expect(result.accounts[0]!.address).toBe(expected);
  });

  it('derives a PDA from a nested arg path', () => {
    const metas: AccountMeta[] = [
      {
        name: 'proposal',
        isSigner: false,
        isWritable: true,
        category: 'pda',
        pdaConfig: {
          programId: TOKEN_PROGRAM,
          seeds: [
            { type: 'literal', value: 'proposal' },
            { type: 'argRef', argName: 'args.transactionIndex', argType: 'u64' },
          ],
        },
      },
    ];
    const result = resolveAccounts(metas, { args: { transactionIndex: 7n } }, {});
    validateAccountResolution(result);

    const expected = findProgramAddressSync([createSeed('proposal'), createSeed(7n)], TOKEN_PROGRAM)[0];
    expect(result.accounts[0]!.address).toBe(expected);
  });

  it('derives a PDA from helper-only resolve inputs when the arg is not on-chain', () => {
    const metas: AccountMeta[] = [
      {
        name: 'proposal',
        isSigner: false,
        isWritable: true,
        category: 'pda',
        pdaConfig: {
          programId: TOKEN_PROGRAM,
          seeds: [
            { type: 'literal', value: 'proposal' },
            { type: 'argRef', argName: 'transactionIndex', argType: 'u64' },
          ],
        },
      },
    ];
    const result = resolveAccounts(metas, {}, { resolve: { transactionIndex: 9n } });
    validateAccountResolution(result);

    const expected = findProgramAddressSync([createSeed('proposal'), createSeed(9n)], TOKEN_PROGRAM)[0];
    expect(result.accounts[0]!.address).toBe(expected);
  });

  it('reports missing required user-provided accounts', () => {
    const metas: AccountMeta[] = [
      { name: 'mint', isSigner: false, isWritable: false, category: 'userProvided' },
    ];
    const result = resolveAccounts(metas, {}, {});
    expect(result.missingUserAccounts).toEqual(['mint']);
    expect(() => validateAccountResolution(result)).toThrow(/Missing required accounts/);
  });

  it('substitutes the program id for omitted non-trailing optional accounts', () => {
    const metas: AccountMeta[] = [
      { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' },
      {
        name: 'referrer',
        isSigner: false,
        isWritable: false,
        category: 'userProvided',
        isOptional: true,
      },
      { name: 'mint', isSigner: false, isWritable: false, category: 'userProvided' },
    ];
    const result = resolveAccounts(metas, {}, {
      wallet,
      accounts: { mint: TOKEN_PROGRAM },
      programId: SYSTEM_PROGRAM,
    });
    validateAccountResolution(result);
    expect(result.accounts.map((a) => a.name)).toEqual(['authority', 'referrer', 'mint']);
    // Anchor convention: omitted optional in a non-trailing slot = program id.
    expect(result.accounts[1]!.address).toBe(SYSTEM_PROGRAM);
    expect(result.accounts[1]!.isSigner).toBe(false);
    expect(result.accounts[1]!.isWritable).toBe(false);
  });

  it('drops omitted trailing optional accounts', () => {
    const metas: AccountMeta[] = [
      { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' },
      {
        name: 'referrer',
        isSigner: false,
        isWritable: false,
        category: 'userProvided',
        isOptional: true,
      },
    ];
    const result = resolveAccounts(metas, {}, { wallet, programId: SYSTEM_PROGRAM });
    validateAccountResolution(result);
    expect(result.accounts.map((a) => a.name)).toEqual(['authority']);
  });

  it('resolves provided optional accounts normally', () => {
    const metas: AccountMeta[] = [
      { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' },
      {
        name: 'referrer',
        isSigner: false,
        isWritable: false,
        category: 'userProvided',
        isOptional: true,
      },
      { name: 'mint', isSigner: false, isWritable: false, category: 'userProvided' },
    ];
    const result = resolveAccounts(metas, {}, {
      wallet,
      accounts: { referrer: WSOL_MINT, mint: TOKEN_PROGRAM },
      programId: SYSTEM_PROGRAM,
    });
    expect(result.accounts.map((a) => a.address)).toEqual([
      WSOL_MINT,
      WSOL_MINT,
      TOKEN_PROGRAM,
    ]);
  });
});

describe('parseInstructionError', () => {
  const errors = [{ code: 6000, name: 'SlippageExceeded', msg: 'Slippage tolerance exceeded' }];

  it('maps an InstructionError Custom code to IDL metadata', () => {
    const parsed = parseInstructionError(
      { InstructionError: [0, { Custom: 6000 }] },
      errors
    );
    expect(parsed).toEqual({
      code: 6000,
      name: 'SlippageExceeded',
      message: 'Slippage tolerance exceeded',
    });
    expect(formatProgramError(parsed!)).toBe(
      'SlippageExceeded (6000): Slippage tolerance exceeded'
    );
  });

  it('falls back to a synthetic error for unknown codes', () => {
    const parsed = parseInstructionError({ code: 12345 }, errors);
    expect(parsed).toEqual({
      code: 12345,
      name: 'CustomError12345',
      message: 'Unknown error with code 12345',
    });
  });

  it('returns null for non-program errors', () => {
    expect(parseInstructionError(null, errors)).toBeNull();
    expect(parseInstructionError(new Error('network down'), errors)).toBeNull();
  });

  it('finds an InstructionError nested in RPC response and cause wrappers', () => {
    const parsed = parseInstructionError({
      cause: {
        value: { err: { InstructionError: [2, { Custom: 6000 }] } },
      },
    }, errors);
    expect(parsed).toMatchObject({ code: 6000, name: 'SlippageExceeded' });
  });
});

describe('normalizeTransactionError', () => {
  const errors = [{ code: 6000, name: 'OreProgramError', msg: 'ORE failed' }];

  it('lets deterministic program errors override submitted-unknown outcomes', () => {
    const chainCause = { InstructionError: [0, { Custom: 6000 }] };
    const submittedUnknown = new TransactionExecutionError({
      status: 'submitted-unknown',
      phase: 'confirmation',
      signature: 'known-signature',
      slot: 44,
      cause: chainCause,
    });

    const normalized = normalizeTransactionError(submittedUnknown, errors);

    expect(normalized).toBeInstanceOf(InstructionError);
    expect(normalized).toMatchObject({
      cause: chainCause,
      signature: 'known-signature',
      slot: 44,
      programError: { code: 6000, name: 'OreProgramError' },
      outcome: {
        status: 'chain-failed',
        phase: 'chain',
        signature: 'known-signature',
        slot: 44,
        cause: chainCause,
      },
    });
  });

  it('prefers nested InstructionError evidence over a conflicting wallet code', () => {
    const chainCause = { InstructionError: [1, { Custom: 6000 }] };
    const normalized = normalizeTransactionError({ code: 4001, cause: chainCause }, errors);

    expect(normalized).toBeInstanceOf(InstructionError);
    expect(normalized).toMatchObject({
      outcome: { status: 'chain-failed' },
      programError: { code: 6000, name: 'OreProgramError' },
    });
  });

  it('treats a direct code matching IDL metadata as deterministic', () => {
    const chainCause = { code: 6000, message: 'transaction simulation failed' };
    const submittedUnknown = new TransactionExecutionError({
      status: 'submitted-unknown',
      phase: 'confirmation',
      signature: 'known-signature',
      cause: chainCause,
    });

    expect(normalizeTransactionError(submittedUnknown, errors)).toMatchObject({
      name: 'InstructionError',
      cause: chainCause,
      outcome: { status: 'chain-failed', signature: 'known-signature' },
      programError: { code: 6000, name: 'OreProgramError' },
    });
  });

  it('does not replace submitted-unknown with an unrecognized generic adapter code', () => {
    const submittedUnknown = new TransactionExecutionError({
      status: 'submitted-unknown',
      phase: 'confirmation',
      signature: 'known-signature',
      cause: { code: -32002, message: 'RPC request failed' },
    });

    expect(normalizeTransactionError(submittedUnknown, errors)).toBe(submittedUnknown);
  });

  it('matches nested user rejection narrowly without treating generic wallet failures as rejection', () => {
    const nestedRejection = Object.assign(new Error('Signing failed'), {
      cause: new Error('User rejected the transaction.'),
    });
    expect(normalizeTransactionError(nestedRejection, errors)).toMatchObject({
      outcome: { status: 'not-submitted', phase: 'wallet', cause: nestedRejection },
    });

    const genericWalletFailure = new Error(
      'Wallet rejected transaction because simulation failed'
    );
    expect(normalizeTransactionError(genericWalletFailure, errors)).toMatchObject({
      outcome: { status: 'not-submitted', phase: 'send', cause: genericWalletFailure },
    });
  });

  it('keeps InstructionError cause and outcome cause aligned without traversing explicit outcomes', () => {
    const originalCause = new Error('original chain failure');
    const opaqueCause = {};
    Object.defineProperty(opaqueCause, 'outcome', {
      get() {
        throw new Error('outcome should not be inspected');
      },
    });
    const outcome = {
      status: 'chain-failed' as const,
      phase: 'chain' as const,
      signature: 'chain-signature',
      cause: originalCause,
    };

    const error = new InstructionError('ORE failed', null, opaqueCause, outcome);

    expect(error.cause).toBe(originalCause);
    expect(error.outcome.cause).toBe(originalCause);
    expect(error.signature).toBe('chain-signature');
  });

  it('unwraps structured causes once and retains their transaction context', () => {
    const originalCause = new Error('chain failure');
    const transactionError = new TransactionExecutionError({
      status: 'chain-failed',
      phase: 'confirmation',
      signature: 'chain-signature',
      slot: 45,
      cause: originalCause,
    });

    const error = new InstructionError('ORE failed', null, transactionError);

    expect(error.cause).toBe(originalCause);
    expect(error.outcome.cause).toBe(originalCause);
    expect(error).toMatchObject({ signature: 'chain-signature', slot: 45 });
  });
});

describe('createInstructionHandler + buildInstruction', () => {
  function makeHandler() {
    return createInstructionHandler({
      programId: TOKEN_PROGRAM,
      discriminator: [1],
      accounts: [
        {
          name: 'authority',
          isSigner: true,
          isWritable: true,
          category: 'signer',
          signerKind: 'wallet',
        },
        { name: 'mint', isSigner: false, isWritable: false, category: 'userProvided' },
        {
          name: 'state',
          isSigner: false,
          isWritable: true,
          category: 'pda',
          pdaConfig: {
            seeds: [
              { type: 'literal', value: 'state' },
              { type: 'accountRef', accountName: 'authority' },
            ],
          },
        },
      ],
      args: [{ name: 'amount', type: 'u64' }],
      errors: [{ code: 6000, name: 'Boom', msg: 'boom' }],
    });
  }

  const wallet = { publicKey: WSOL_MINT } as WalletAdapter;

  it('splits merged params into args and account overrides', () => {
    const handler = makeHandler();
    const built = buildInstruction(
      handler,
      { amount: 100n, mint: SYSTEM_PROGRAM },
      { wallet }
    );

    expect(built.programId).toBe(TOKEN_PROGRAM);
    expect(built.keys.map((k) => k.pubkey)).toEqual([
      WSOL_MINT, // authority (signer)
      SYSTEM_PROGRAM, // mint (user-provided)
      findProgramAddressSync(
        [createSeed('state'), decodeBase58(WSOL_MINT)],
        TOKEN_PROGRAM
      )[0], // state (PDA derived from authority)
    ]);
    // discriminator [1] + u64 100 little-endian.
    expect([...built.data]).toEqual([1, 100, 0, 0, 0, 0, 0, 0, 0]);
    expect(built.keys[0]!.isSigner).toBe(true);
  });

  it('lets merged params override a signer slot explicitly', () => {
    const handler = makeHandler();
    const alternateSigner = TOKEN_PROGRAM;
    const built = buildInstruction(
      handler,
      { amount: 7n, authority: alternateSigner, mint: SYSTEM_PROGRAM },
      { wallet }
    );

    expect(built.keys.map((key) => key.pubkey)).toEqual([
      alternateSigner,
      SYSTEM_PROGRAM,
      findProgramAddressSync(
        [createSeed('state'), decodeBase58(alternateSigner)],
        TOKEN_PROGRAM
      )[0],
    ]);
  });

  it('lets an explicit PDA override win over automatic derivation', () => {
    const built = buildInstruction(
      makeHandler(),
      { amount: 7n, mint: SYSTEM_PROGRAM },
      { wallet, accounts: { state: TOKEN_PROGRAM } }
    );
    expect(built.keys[2]!.pubkey).toBe(TOKEN_PROGRAM);
  });

  it('rejects an invalid explicit account override while building', () => {
    for (const state of ['', 'not-a-public-key']) {
      expect(() => buildInstruction(
        makeHandler(),
        { amount: 7n, mint: SYSTEM_PROGRAM },
        { wallet, accounts: { state } }
      )).toThrow(/Invalid account override for "state"/);
    }
  });

  it('does not assign an unannotated signer slot to the wallet', () => {
    const handler = createInstructionHandler({
      programId: TOKEN_PROGRAM,
      discriminator: [3],
      accounts: [
        { name: 'new_mint', isSigner: true, isWritable: true, category: 'signer' },
      ],
      args: [],
    });
    expect(() => buildInstruction(handler, {}, { wallet })).toThrow(/new_mint/);
  });

  it('derives a PDA under a program selected from another account', () => {
    const handler = createInstructionHandler({
      programId: SYSTEM_PROGRAM,
      discriminator: [4],
      accounts: [
        {
          name: 'metadata',
          isSigner: false,
          isWritable: true,
          category: 'pda',
          pdaConfig: {
            program: { type: 'accountRef', accountName: 'metadata_program' },
            seeds: [
              { type: 'literal', value: 'metadata' },
              { type: 'accountRef', accountName: 'metadata_program' },
              { type: 'accountRef', accountName: 'mint' },
            ],
          },
        },
        {
          name: 'metadata_program',
          isSigner: false,
          isWritable: false,
          category: 'known',
          knownAddress: TOKEN_PROGRAM,
        },
        {
          name: 'mint',
          isSigner: false,
          isWritable: false,
          category: 'userProvided',
        },
      ],
      args: [],
    });
    const built = buildInstruction(handler, { mint: WSOL_MINT });
    const expected = findProgramAddressSync(
      [createSeed('metadata'), decodeBase58(TOKEN_PROGRAM), decodeBase58(WSOL_MINT)],
      TOKEN_PROGRAM
    )[0];
    expect(built.keys[0]!.pubkey).toBe(expected);
    expect(built.keys[0]!.pubkey).not.toBe(
      findProgramAddressSync(
        [createSeed('metadata'), decodeBase58(TOKEN_PROGRAM), decodeBase58(WSOL_MINT)],
        SYSTEM_PROGRAM
      )[0]
    );
  });

  it('throws when a non-arg param is not a string address', () => {
    const handler = makeHandler();
    expect(() =>
      buildInstruction(handler, { amount: 1n, mint: 42 as unknown as string }, { wallet })
    ).toThrow(/not a known argument/);
  });

  it('accepts helper-only resolve inputs for PDA derivation', () => {
    const handler = createInstructionHandler({
      programId: TOKEN_PROGRAM,
      discriminator: [2],
      accounts: [
        { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' },
        {
          name: 'proposal',
          isSigner: false,
          isWritable: true,
          category: 'pda',
          pdaConfig: {
            seeds: [
              { type: 'literal', value: 'proposal' },
              { type: 'argRef', argName: 'transactionIndex', argType: 'u64' },
            ],
          },
        },
      ],
      args: [{ name: 'amount', type: 'u64' }],
    });

    const built = buildInstruction(
      handler,
      { amount: 5n, resolve: { transactionIndex: 11n } },
      { wallet }
    );

    expect(built.keys.map((k) => k.pubkey)).toEqual([
      WSOL_MINT,
      findProgramAddressSync([createSeed('proposal'), createSeed(11n)], TOKEN_PROGRAM)[0],
    ]);
    expect([...built.data]).toEqual([2, 5, 0, 0, 0, 0, 0, 0, 0]);
  });

  it('rejects non-object resolve inputs', () => {
    const handler = makeHandler();
    expect(() =>
      buildInstruction(handler, { amount: 1n, mint: SYSTEM_PROGRAM, resolve: 1 as unknown as Record<string, unknown> }, { wallet })
    ).toThrow(/resolve/);
  });

  it('rejects unknown parameter names instead of silently dropping them', () => {
    const handler = makeHandler();
    expect(() =>
      buildInstruction(
        handler,
        { amount: 1n, mint: SYSTEM_PROGRAM, mnit: TOKEN_PROGRAM },
        { wallet }
      )
    ).toThrow(/Unknown parameter "mnit"/);
  });

  it('rejects missing required args instead of encoding zeros', () => {
    const handler = makeHandler();
    expect(() =>
      buildInstruction(handler, { mint: SYSTEM_PROGRAM }, { wallet })
    ).toThrow(/Missing required argument "amount"/);
  });

  it('still treats an omitted option arg as None', () => {
    const schema: ArgSchema[] = [{ name: 'maybe', type: { option: 'u8' } }];
    expect([...serializeInstructionData(new Uint8Array(0), {}, schema)]).toEqual([0]);
  });

  it('appends remainingAccounts after the declared accounts', () => {
    const handler = makeHandler();
    const extra = { pubkey: TOKEN_PROGRAM, isSigner: false, isWritable: true };
    const built = buildInstruction(
      handler,
      { amount: 1n, mint: SYSTEM_PROGRAM },
      { wallet, remainingAccounts: [extra] }
    );
    expect(built.keys[built.keys.length - 1]).toEqual(extra);
    expect(built.keys).toHaveLength(4); // 3 declared + 1 remaining
  });

  it('executes via the wallet and parses program errors', async () => {
    const handler = makeHandler();
    let sent: BuiltInstruction[] | null = null;
    const okWallet: WalletAdapter = {
      publicKey: WSOL_MINT,
      async signAndSend(ixs): Promise<SendResult> {
        sent = ixs;
        return { signature: 'sig123', slot: 99 };
      },
    };
    const result = await executeInstruction(
      handler,
      { amount: 1n, mint: SYSTEM_PROGRAM },
      { wallet: okWallet }
    );
    expect(result).toEqual({ signature: 'sig123', slot: 99 });
    expect(sent!).toHaveLength(1);

    const failWallet: WalletAdapter = {
      publicKey: WSOL_MINT,
      async signAndSend(): Promise<SendResult> {
        throw { InstructionError: [0, { Custom: 6000 }] };
      },
    };
    await expect(
      executeInstruction(handler, { amount: 1n, mint: SYSTEM_PROGRAM }, { wallet: failWallet })
    ).rejects.toMatchObject({ name: 'InstructionError', programError: { code: 6000, name: 'Boom' } });
  });

  it('classifies instruction build failures before wallet submission', async () => {
    const handler = makeHandler();
    const signAndSend = async (): Promise<SendResult> => ({ signature: 'not-called' });

    const error = await executeInstruction(
      handler,
      { mint: SYSTEM_PROGRAM },
      { wallet: { publicKey: WSOL_MINT, signAndSend } }
    ).catch((cause: unknown) => cause);

    expect(error).toBeInstanceOf(TransactionExecutionError);
    expect(error).toMatchObject({
      outcome: { status: 'not-submitted', phase: 'build' },
      cause: expect.objectContaining({ message: expect.stringContaining('amount') }),
    });
  });
});
