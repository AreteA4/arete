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
import { parseInstructionError, formatProgramError } from './error-parser';
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
});

describe('resolveAccounts', () => {
  const wallet = { publicKey: WSOL_MINT } as WalletAdapter;

  it('resolves signer, known, and userProvided categories', () => {
    const metas: AccountMeta[] = [
      { name: 'authority', isSigner: true, isWritable: true, category: 'signer' },
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

  it('derives a PDA referencing a signer account and keeps original order', () => {
    const metas: AccountMeta[] = [
      { name: 'authority', isSigner: true, isWritable: true, category: 'signer' },
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
      { name: 'authority', isSigner: true, isWritable: true, category: 'signer' },
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
      { name: 'authority', isSigner: true, isWritable: true, category: 'signer' },
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
      { name: 'authority', isSigner: true, isWritable: true, category: 'signer' },
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
});

describe('createInstructionHandler + buildInstruction', () => {
  function makeHandler() {
    return createInstructionHandler({
      programId: TOKEN_PROGRAM,
      discriminator: [1],
      accounts: [
        { name: 'authority', isSigner: true, isWritable: true, category: 'signer' },
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

  it('throws when a non-arg param is not a string address', () => {
    const handler = makeHandler();
    expect(() =>
      buildInstruction(handler, { amount: 1n, mint: 42 as unknown as string }, { wallet })
    ).toThrow(/not a known argument/);
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
});
