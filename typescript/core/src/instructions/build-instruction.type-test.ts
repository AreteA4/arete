// Type-level tests: `npm run typecheck` compiles this file; nothing runs it.
//
// `buildInstruction` checks its params against the handler's params type, the
// way generated program modules declare it: `createInstructionHandler<Params,
// Error>({...})`. Wrong, unknown or missing keys are compile errors, while
// untyped `Record<string, unknown>` params (as older generated modules pass
// them) keep compiling.

import type { WalletAdapter } from '../wallet/types';
import {
  buildInstruction,
  createInstructionExecutor,
  createInstructionHandler,
  executeInstruction,
  type InstructionParams,
} from './executor';

type Equal<TLeft, TRight> =
  (<T>() => T extends TLeft ? 1 : 2) extends
  (<T>() => T extends TRight ? 1 : 2)
    ? true
    : false;
type Assert<T extends true> = T;

const MINT = 'So11111111111111111111111111111111111111112';

// Generated from a snake_case IDL (SPL Token): args keep the IDL spelling.
interface InitializeMint2Params {
  decimals: number;
  mint_authority: string;
  freeze_authority: string | null;
  mint: string;
}
const InitializeMint2Instruction = createInstructionHandler<InitializeMint2Params, never>({
  programId: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
  discriminator: [20],
  args: [
    { name: 'decimals', type: 'u8' },
    { name: 'mint_authority', type: 'pubkey' },
    { name: 'freeze_authority', type: { option: 'pubkey' } },
  ],
  accounts: [{ name: 'mint', isSigner: false, isWritable: true, category: 'userProvided' }],
});

interface MintToParams {
  amount: bigint;
  mint: string;
  destination: string;
  authority: string;
}
const MintToInstruction = createInstructionHandler<MintToParams, never>({
  programId: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
  discriminator: [7],
  args: [{ name: 'amount', type: 'u64' }],
  accounts: [
    { name: 'mint', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'destination', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'authority', isSigner: true, isWritable: false, category: 'signer', signerKind: 'provided' },
  ],
});

// Generated from a camelCase IDL: PDA and optional accounts are optional
// params, PDA seed inputs go under `resolve`.
interface DepositParams {
  maxAmount: bigint;
  tokenAccount: string;
  vault?: string;
  referrer?: string;
  resolve?: {
    poolIndex?: bigint;
  };
}
const DepositInstruction = createInstructionHandler<DepositParams, never>({
  programId: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
  discriminator: [1, 2, 3, 4, 5, 6, 7, 8],
  args: [{ name: 'maxAmount', type: 'u64' }],
  accounts: [
    { name: 'tokenAccount', isSigner: false, isWritable: true, category: 'userProvided' },
    {
      name: 'vault',
      isSigner: false,
      isWritable: true,
      category: 'pda',
      pdaConfig: { seeds: [{ type: 'argRef', argName: 'poolIndex', argType: 'u64' }] },
    },
    { name: 'referrer', isSigner: false, isWritable: false, category: 'userProvided', isOptional: true },
  ],
});

export type HandlerParamsAssertions = [
  Assert<Equal<InstructionParams<typeof InitializeMint2Instruction>, InitializeMint2Params>>,
  Assert<Equal<InstructionParams<typeof DepositInstruction>, DepositParams>>,
  Assert<Equal<InstructionParams<typeof looseHandler>, Record<string, unknown>>>,
];

// --- Correct calls compile. ---
buildInstruction(InitializeMint2Instruction, {
  decimals: 6,
  mint_authority: MINT,
  freeze_authority: null,
  mint: MINT,
});
buildInstruction(MintToInstruction, {
  amount: 1n,
  mint: MINT,
  destination: MINT,
  authority: MINT,
});
buildInstruction(DepositInstruction, {
  maxAmount: 1n,
  tokenAccount: MINT,
  resolve: { poolIndex: 0n },
});
buildInstruction(
  DepositInstruction,
  { maxAmount: 1n, tokenAccount: MINT, vault: MINT, referrer: MINT },
  { accounts: { vault: MINT } }
);

// --- Wrong names, missing keys and wrong value types are errors. ---
buildInstruction(InitializeMint2Instruction, {
  decimals: 6,
  // @ts-expect-error camelCase spelling of `mint_authority` is not a param
  mintAuthority: MINT,
  freezeAuthority: null,
  mint: MINT,
});
// @ts-expect-error `mint_authority` and `freeze_authority` are missing
buildInstruction(InitializeMint2Instruction, { decimals: 6, mint: MINT });
buildInstruction(MintToInstruction, {
  amount: 1n,
  mint: MINT,
  // @ts-expect-error `account` is not an account of MintTo (it is `destination`)
  account: MINT,
  owner: MINT,
});
// @ts-expect-error `destination` and `authority` are missing
buildInstruction(MintToInstruction, { amount: 1n, mint: MINT });
buildInstruction(MintToInstruction, {
  // @ts-expect-error amounts are bigint
  amount: 'one',
  mint: MINT,
  destination: MINT,
  authority: MINT,
});
buildInstruction(DepositInstruction, {
  // @ts-expect-error snake_case spelling of a camelCase IDL arg
  max_amount: 1n,
  tokenAccount: MINT,
});

// A params object built ahead of time is checked too (no excess-property
// check, but required keys still are).
const prebuilt = { mintAuthority: MINT, decimals: 6, mint: MINT };
// @ts-expect-error missing `mint_authority` and `freeze_authority`
buildInstruction(InitializeMint2Instruction, prebuilt);

// --- Untyped params keep compiling. ---
declare const untyped: Record<string, unknown>;
declare const typedParams: MintToParams;
buildInstruction(MintToInstruction, untyped);
// What generated operation wrappers emit.
buildInstruction(MintToInstruction, typedParams as unknown as Record<string, unknown>);

// A handler created without a params type accepts any object.
const looseHandler = createInstructionHandler({
  programId: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
  discriminator: [9],
  args: [],
  accounts: [],
});
buildInstruction(looseHandler, { anything: 1 });
// @ts-expect-error params must be an object
buildInstruction(looseHandler, 1);

// --- executeInstruction and the bound executor use the same checks. ---
declare const wallet: WalletAdapter;
void executeInstruction(
  MintToInstruction,
  { amount: 1n, mint: MINT, destination: MINT, authority: MINT },
  { wallet }
);
// @ts-expect-error `owner` is not an account of MintTo
void executeInstruction(MintToInstruction, { amount: 1n, mint: MINT, destination: MINT, owner: MINT });
const executor = createInstructionExecutor(wallet);
executor.build(MintToInstruction, { amount: 1n, mint: MINT, destination: MINT, authority: MINT });
// @ts-expect-error `destination` is missing
executor.build(MintToInstruction, { amount: 1n, mint: MINT, authority: MINT });

// --- Option args may be omitted; they encode as `None`. ---
buildInstruction(InitializeMint2Instruction, { decimals: 6, mint_authority: MINT, mint: MINT });
// @ts-expect-error `mint_authority` is required even when the option arg is omitted
buildInstruction(InitializeMint2Instruction, { decimals: 6, mint: MINT });
buildInstruction(InitializeMint2Instruction, {
  decimals: 6,
  mint_authority: MINT,
  // @ts-expect-error option args take the inner type or null
  freeze_authority: 1,
  mint: MINT,
});
// An `unknown` typed arg is not mistaken for an option.
interface OpaqueArgParams {
  data: unknown;
  owner: string;
}
const OpaqueArgInstruction = createInstructionHandler<OpaqueArgParams, never>({
  programId: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
  discriminator: [10],
  args: [{ name: 'data', type: 'bytes' }],
  accounts: [{ name: 'owner', isSigner: false, isWritable: false, category: 'userProvided' }],
});
// @ts-expect-error `data` is missing
buildInstruction(OpaqueArgInstruction, { owner: MINT });

// --- Accounts passed through `options.accounts` need not be in params. ---
// Shaped like a generated Ore `close`: accounts only, no args.
interface CloseParams {
  signer: string;
  board?: string;
  rentPayer: string;
  round: string;
}
const CloseInstruction = createInstructionHandler<CloseParams, never>({
  programId: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
  discriminator: [5],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    {
      name: 'board',
      isSigner: false,
      isWritable: true,
      category: 'pda',
      pdaConfig: { seeds: [{ type: 'literal', value: 'board' }] },
    },
    { name: 'rentPayer', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'round', isSigner: false, isWritable: true, category: 'userProvided' },
  ],
});
buildInstruction(CloseInstruction, {}, { accounts: { signer: MINT, rentPayer: MINT, round: MINT } });
buildInstruction(CloseInstruction, { signer: MINT }, { accounts: { rentPayer: MINT, round: MINT } });
// @ts-expect-error `round` is neither in params nor in options.accounts
buildInstruction(CloseInstruction, {}, { accounts: { signer: MINT, rentPayer: MINT } });
buildInstruction(
  CloseInstruction,
  // @ts-expect-error unknown keys are still rejected when accounts come from options
  { rent_payer: MINT },
  { accounts: { signer: MINT, rentPayer: MINT, round: MINT } }
);
buildInstruction(MintToInstruction, { amount: 1n }, {
  accounts: { mint: MINT, destination: MINT, authority: MINT },
});
// @ts-expect-error `amount` is an arg and still required
buildInstruction(MintToInstruction, {}, { accounts: { mint: MINT, destination: MINT, authority: MINT } });
// A wide accounts map names no account in particular, so params stay checked.
declare const accountMap: Record<string, string>;
// @ts-expect-error `destination` and `authority` are missing from params
buildInstruction(MintToInstruction, { amount: 1n, mint: MINT }, { accounts: accountMap });
void executeInstruction(CloseInstruction, {}, {
  wallet,
  accounts: { signer: MINT, rentPayer: MINT, round: MINT },
});
// @ts-expect-error `round` is missing
void executeInstruction(CloseInstruction, {}, { wallet, accounts: { signer: MINT, rentPayer: MINT } });
executor.build(CloseInstruction, {}, { accounts: { signer: MINT, rentPayer: MINT, round: MINT } });
void executor.execute(InitializeMint2Instruction, { decimals: 6, mint_authority: MINT, mint: MINT });
// @ts-expect-error `round` is missing
executor.build(CloseInstruction, {}, { accounts: { signer: MINT, rentPayer: MINT } });
