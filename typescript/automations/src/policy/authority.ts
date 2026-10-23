/**
 * Instructions that hand control of the signer's funds to someone else
 * without moving a balance.
 *
 * The policy's caps measure balance changes, and some instructions change no
 * balance yet give the funds away: a token delegation (`approve`), a new
 * owner or authority (`setAuthority`, System `assign`), or lamports leaving
 * an account the signer controls that is not the signer's own (a nonce or
 * stake withdrawal). This module names those instructions in the System, SPL
 * Token, Token-2022, upgradeable loader and stake programs when the signer
 * takes part in them (signs, or is one of their accounts).
 *
 * Top-level instructions are read from the wire bytes. Inner (CPI)
 * instructions are read from a simulation when the RPC returns them.
 *
 * A token account closed to an account other than the signer is not refused;
 * its lamports count as SOL leaving the signer.
 */

import type { DecodedTransaction } from './decode';
import { asString, asU64, decodeBase58, field, isObject } from './encoding';

/** Programs that own SPL token accounts. */
export const TOKEN_PROGRAMS = [
  'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
  'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb',
] as const;
export const SYSTEM_PROGRAM = '11111111111111111111111111111111';

/** Why a transaction with a durable nonce is refused. */
export const DURABLE_NONCE =
  'makes it a durable-nonce transaction, valid until the nonce advances instead of expiring';
export const UPGRADEABLE_LOADER = 'BPFLoaderUpgradeab1e11111111111111111111111';
export const STAKE_PROGRAM = 'Stake11111111111111111111111111111111111111';
/** Wrapped SOL mints (SPL Token and Token-2022). */
export const NATIVE_MINTS = [
  'So11111111111111111111111111111111111111112',
  '9pan9bMn5HatX4EJdBwg9VgCa7Uz5HL8N1m5D3NdXejP',
] as const;

/**
 * Token instructions the signer may take part in. Everything else that
 * involves the signer is refused, including Token-2022 extension
 * instructions this list does not name.
 */
const TOKEN_ALLOWED = new Set([
  'initializeMint',
  'initializeMint2',
  'initializeAccount',
  'initializeAccount2',
  'initializeAccount3',
  'initializeMultisig',
  'initializeMultisig2',
  'initializeImmutableOwner',
  'transfer',
  'transferChecked',
  'transferCheckedWithFee',
  'mintTo',
  'mintToChecked',
  'burn',
  'burnChecked',
  'revoke',
  'freezeAccount',
  'thawAccount',
  'syncNative',
  'getAccountDataSize',
  'amountToUiAmount',
  'uiAmountToAmount',
  'reallocate',
  'createNativeMint',
]);

const SYSTEM_ALLOWED = new Set([
  'createAccount',
  'createAccountWithSeed',
  'transfer',
  'advanceNonce',
  'initializeNonce',
  'allocateWithSeed',
  'upgradeNonce',
]);

const STAKE_ALLOWED = new Set([
  'initialize',
  'initializeChecked',
  'delegate',
  'split',
  'deactivate',
  'deactivateDelinquent',
  'redelegate',
  'getMinimumDelegation',
]);

/** An instruction the signer must not sign. */
export interface AuthorityFinding {
  readonly programId: string;
  readonly instruction: string;
  /** Whether it is an inner (CPI) instruction from the simulation. */
  readonly inner: boolean;
  readonly reason: string;
}

/** What the checks found in one transaction. */
export interface AuthorityChecked {
  readonly findings: AuthorityFinding[];
  /**
   * Token accounts the signer closes to another account: their lamports
   * leave the signer. `null` when the account's address is not known (a
   * lookup-table entry that was not resolved).
   */
  readonly closedToOthers: Array<string | null>;
}

export function isTokenProgram(program: string): boolean {
  return (TOKEN_PROGRAMS as readonly string[]).includes(program);
}

export function isNativeMint(mint: string): boolean {
  return (NATIVE_MINTS as readonly string[]).includes(mint);
}

const TOKEN_NAMES: Record<number, string> = {
  0: 'initializeMint',
  1: 'initializeAccount',
  2: 'initializeMultisig',
  3: 'transfer',
  4: 'approve',
  5: 'revoke',
  6: 'setAuthority',
  7: 'mintTo',
  8: 'burn',
  9: 'closeAccount',
  10: 'freezeAccount',
  11: 'thawAccount',
  12: 'transferChecked',
  13: 'approveChecked',
  14: 'mintToChecked',
  15: 'burnChecked',
  16: 'initializeAccount2',
  17: 'syncNative',
  18: 'initializeAccount3',
  19: 'initializeMultisig2',
  20: 'initializeMint2',
  21: 'getAccountDataSize',
  22: 'initializeImmutableOwner',
  23: 'amountToUiAmount',
  24: 'uiAmountToAmount',
  25: 'initializeMintCloseAuthority',
  29: 'reallocate',
  31: 'createNativeMint',
  38: 'withdrawExcessLamports',
};

const SYSTEM_NAMES = [
  'createAccount',
  'assign',
  'transfer',
  'createAccountWithSeed',
  'advanceNonce',
  'withdrawFromNonce',
  'initializeNonce',
  'authorizeNonce',
  'allocate',
  'allocateWithSeed',
  'assignWithSeed',
  'transferWithSeed',
  'upgradeNonce',
];

const STAKE_NAMES = [
  'initialize',
  'authorize',
  'delegate',
  'split',
  'withdraw',
  'deactivate',
  'setLockup',
  'merge',
  'authorizeWithSeed',
  'initializeChecked',
  'authorizeChecked',
  'authorizeCheckedWithSeed',
  'setLockupChecked',
  'getMinimumDelegation',
  'deactivateDelinquent',
  'redelegate',
];

function tokenName(data: Uint8Array): string {
  const tag = data[0];
  if (tag === undefined) return 'emptyInstruction';
  if (tag === 26 && data[1] === 1) return 'transferCheckedWithFee';
  return TOKEN_NAMES[tag] ?? `tokenInstruction${tag}`;
}

function u32Tag(data: Uint8Array): number | undefined {
  if (data.length < 4) return undefined;
  return (data[0]! | (data[1]! << 8) | (data[2]! << 16) | (data[3]! << 24)) >>> 0;
}

function taggedName(data: Uint8Array, names: readonly string[], fallback: string): string {
  const tag = u32Tag(data);
  if (tag === undefined) return 'emptyInstruction';
  return names[tag] ?? `${fallback}${tag}`;
}

/** The instruction's name, for the programs this module checks. */
export function instructionName(program: string, data: Uint8Array): string | null {
  if (isTokenProgram(program)) return tokenName(data);
  if (program === SYSTEM_PROGRAM) return taggedName(data, SYSTEM_NAMES, 'systemInstruction');
  if (program === STAKE_PROGRAM) return taggedName(data, STAKE_NAMES, 'stakeInstruction');
  if (program === UPGRADEABLE_LOADER) {
    const tag = u32Tag(data);
    return tag === undefined ? 'emptyInstruction' : `upgradeableLoaderInstruction${tag}`;
  }
  return null;
}

/** Why the signer may not take part in `name` of `program`, or `null`. */
function refusalReason(program: string, name: string): string | null {
  if (isTokenProgram(program)) {
    switch (name) {
      case 'approve':
      case 'approveChecked':
        return 'delegates tokens the signer controls';
      case 'setAuthority':
        return 'changes an authority the signer holds';
      case 'closeAccount':
        return null;
      case 'withdrawExcessLamports':
        return 'moves lamports out of an account the signer controls';
      default:
        return TOKEN_ALLOWED.has(name)
          ? null
          : "is a token instruction the signer's policy cannot check";
    }
  }
  if (program === SYSTEM_PROGRAM) {
    switch (name) {
      case 'assign':
      case 'assignWithSeed':
        return 'changes the owner of an account the signer controls';
      case 'allocate':
        return "allocates data on the signer's account";
      case 'authorizeNonce':
        return 'changes a nonce authority the signer holds';
      case 'withdrawFromNonce':
      case 'transferWithSeed':
        return 'moves lamports out of an account the signer controls';
      default:
        return SYSTEM_ALLOWED.has(name)
          ? null
          : "is a System instruction the signer's policy cannot check";
    }
  }
  if (program === STAKE_PROGRAM) {
    return STAKE_ALLOWED.has(name)
      ? null
      : 'changes the authority or withdraws from a stake account the signer controls';
  }
  if (program === UPGRADEABLE_LOADER) {
    return 'changes a program or buffer the signer controls';
  }
  return null;
}

/** Check one instruction in which the signer takes part. */
function checkOne(
  program: string,
  name: string,
  inner: boolean,
  closed: string | null,
  destinationIsSigner: boolean,
  out: AuthorityChecked,
): void {
  if (isTokenProgram(program) && name === 'closeAccount') {
    if (!destinationIsSigner) out.closedToOthers.push(closed);
    return;
  }
  const reason = refusalReason(program, name);
  if (reason !== null) {
    out.findings.push({ programId: program, instruction: name, inner, reason });
  }
}

function emptyChecked(): AuthorityChecked {
  return { findings: [], closedToOthers: [] };
}

export function mergeChecked(into: AuthorityChecked, other: AuthorityChecked): void {
  into.findings.push(...other.findings);
  into.closedToOthers.push(...other.closedToOthers);
}

/**
 * Check the top-level instructions of `decoded`. `lookupKeys` are the
 * addresses its lookup tables load (writable entries, then read-only), where
 * they were resolved.
 */
export function checkTopLevel(
  decoded: DecodedTransaction,
  signer: string,
  lookupKeys: readonly (string | null)[],
): AuthorityChecked {
  const out = emptyChecked();
  const signerIndex = decoded.staticKeys.slice(0, decoded.requiredSignatures).indexOf(signer);
  // Not a signer of this transaction: no authority of its can be used.
  if (signerIndex < 0) return out;
  const key = (index: number): string | null =>
    decoded.staticKeys[index] ?? lookupKeys[index - decoded.staticKeys.length] ?? null;
  // A transaction that starts by advancing a nonce uses a durable nonce
  // instead of a recent blockhash: whoever the nonce authority is, the signed
  // bytes stay valid until the nonce advances, so they could land long after
  // the step was checked and its consent revoked.
  const first = decoded.instructions[0];
  if (
    first !== undefined &&
    decoded.staticKeys[first.programIndex] === SYSTEM_PROGRAM &&
    instructionName(SYSTEM_PROGRAM, first.data) === 'advanceNonce'
  ) {
    out.findings.push({ programId: SYSTEM_PROGRAM, instruction: 'advanceNonce', inner: false, reason: DURABLE_NONCE });
  }
  for (const instruction of decoded.instructions) {
    const program = decoded.staticKeys[instruction.programIndex];
    if (program === undefined) continue;
    const name = instructionName(program, instruction.data);
    if (name === null) continue;
    if (!instruction.accounts.includes(signerIndex)) continue;
    if (program === SYSTEM_PROGRAM && name === 'advanceNonce') continue;
    const first = instruction.accounts[0];
    const closed = first === undefined ? null : key(first);
    checkOne(program, name, false, closed, instruction.accounts[1] === signerIndex, out);
  }
  return out;
}

function mentions(value: unknown, signer: string): boolean {
  if (typeof value === 'string') return value === signer;
  if (Array.isArray(value)) return value.some((item) => mentions(item, signer));
  if (isObject(value)) return Object.values(value).some((item) => mentions(item, signer));
  return false;
}

/**
 * Check the inner instructions a simulation returned (`innerInstructions` of
 * `simulateTransaction`, in the RPC's parsed or partially decoded form).
 * `accountKeys` resolves compiled instructions' indexes.
 */
export function checkInner(
  inner: unknown,
  signer: string,
  accountKeys: readonly (string | null)[],
): AuthorityChecked {
  const out = emptyChecked();
  if (!Array.isArray(inner)) return out;
  for (const group of inner) {
    const instructions = field(group, 'instructions');
    if (!Array.isArray(instructions)) continue;
    for (const instruction of instructions) {
      mergeChecked(out, checkInnerOne(instruction, signer, accountKeys));
    }
  }
  return out;
}

function checkInnerOne(
  instruction: unknown,
  signer: string,
  accountKeys: readonly (string | null)[],
): AuthorityChecked {
  const out = emptyChecked();
  let program = asString(field(instruction, 'programId'));
  if (program === undefined) {
    const index = asU64(field(instruction, 'programIdIndex'));
    program = index === undefined ? undefined : (accountKeys[index] ?? undefined);
    if (program === undefined) return out;
  }
  const parsed = field(instruction, 'parsed');
  if (isObject(parsed)) {
    const name = asString(parsed.type);
    if (name === undefined) return out;
    const info = parsed.info;
    if (instructionName(program, new Uint8Array()) === null || !mentions(info, signer)) return out;
    const closed = asString(field(info, 'account')) ?? null;
    checkOne(program, name, true, closed, field(info, 'destination') === signer, out);
    return out;
  }
  const rawAccounts = field(instruction, 'accounts');
  if (!Array.isArray(rawAccounts)) return out;
  const accounts = rawAccounts.map((account): string | null => {
    if (typeof account === 'string') return account;
    if (typeof account === 'number') {
      const index = asU64(account);
      return index === undefined ? null : (accountKeys[index] ?? null);
    }
    return null;
  });
  const encoded = asString(field(instruction, 'data'));
  const data = (encoded === undefined ? null : decodeBase58(encoded)) ?? new Uint8Array();
  const name = instructionName(program, data);
  if (name === null) return out;
  if (!accounts.includes(signer)) return out;
  checkOne(program, name, true, accounts[0] ?? null, accounts[1] === signer, out);
  return out;
}
