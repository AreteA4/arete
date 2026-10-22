/**
 * What a prepared step's transactions do, read from the transactions
 * themselves rather than from the labels a prepare returns:
 *
 * - the programs each transaction invokes;
 * - the accounts it writes (static keys, plus address lookup table entries,
 *   resolved when a simulator is available);
 * - the signatures it carries and the ones still required;
 * - with a simulator, the signer's SOL and token balance changes from a
 *   simulation of the exact bytes, the simulation error and the tail of its
 *   logs;
 * - the instructions that would hand control of the signer's funds to
 *   someone else (see `authority.ts`): read from the top-level instructions
 *   always, and from the inner instructions when the simulation returns
 *   them.
 *
 * Without a simulator the balance changes are reported as unavailable,
 * never as zero.
 */

import { lamportsToSol } from './amounts';
import {
  type AuthorityChecked,
  type AuthorityFinding,
  checkInner,
  checkTopLevel,
  isNativeMint,
  isTokenProgram,
  mergeChecked,
} from './authority';
import {
  type DecodedTransaction,
  decodeTransaction,
  transactionPrograms,
  transactionSigners,
  writableStaticKeys,
} from './decode';
import {
  asString,
  asU64,
  decodeBase64,
  encodeBase58,
  encodeBase64,
  field,
} from './encoding';
import type { Simulator } from './simulator';

const LOOKUP_TABLE_HEADER = 56;
const LOG_TAIL = 20;
/** The most accounts one transaction's simulation reports on. */
const MAX_SIMULATED_ACCOUNTS = 64;

/** A prepare result, as far as the summary reads it. */
export interface PreparedStepLike {
  readonly intent?: unknown;
  readonly transactions: readonly { readonly index?: number; readonly transactionBase64: string }[];
  /** Role-labelled instructions, when the prepare returns them. */
  readonly anatomy?: unknown;
  /** The server's simulation metrics, when the prepare returns them. */
  readonly simulation?: unknown;
}

export interface TokenDelta {
  readonly mint: string;
  /** Base units, signed: negative when tokens leave the signer. */
  readonly amount: string;
}

export interface SignerEffectSummary {
  /** Whether every balance change of the signer is known. */
  readonly known: boolean;
  readonly solLamports?: string | null;
  readonly sol?: string | null;
  readonly tokens?: TokenDelta[];
  readonly closedAccountLamportsOut?: string | null;
  readonly unresolvedLookupTables?: string[];
  readonly omittedAccounts?: number;
  readonly innerInstructionsChecked?: boolean;
  readonly preStateSlot?: number | null;
  readonly simulatedSlot?: number | null;
  /** Why the simulation could not run. */
  readonly error?: string;
}

export interface TransactionSummary {
  readonly index: number;
  readonly version: 'legacy' | 'v0' | 'v1';
  readonly feePayer: string | null;
  readonly programs: Array<{ readonly programId: string; readonly roles: string[] }>;
  readonly accountsWritten: string[];
  readonly accountsWrittenComplete: boolean;
  readonly unresolvedWritableAccounts: Array<{ readonly table: string; readonly addressIndex: number }>;
  readonly signatures: {
    readonly required: string[];
    readonly present: string[];
    readonly missing: string[];
  };
  readonly serverSimulation: unknown;
  readonly lookupTables?: Array<{
    readonly table: string;
    readonly writableEntries: number;
    readonly readonlyEntries: number;
  }>;
  readonly signerEffect: SignerEffectSummary;
  readonly simulation?: { readonly err: unknown; readonly logsTail: string[] };
  readonly unsafeInstructions: AuthorityFinding[];
}

export interface PreparedSummary {
  readonly signer: string;
  readonly transactions: TransactionSummary[];
  readonly programsInvoked: string[];
  readonly signerBalanceChanges:
    | { readonly available: false; readonly reason: string }
    | {
        readonly available: true;
        readonly complete: boolean;
        readonly solLamports: string | null;
        readonly sol: string | null;
        readonly tokens: TokenDelta[];
        readonly note: string;
      };
  readonly simulationErrors: Array<{
    readonly transaction: number;
    readonly err: unknown;
    readonly dependsOnEarlierTransactions: boolean;
  }>;
  readonly signaturesStillRequired: string[];
}

// ---------------------------------------------------------------------------
// Simulation
// ---------------------------------------------------------------------------

interface TokenFacts {
  readonly mint: string;
  readonly owner: string;
  readonly amount: bigint;
}

/** One account's lamports, and its SPL token balance when it is a token account. */
interface AccountFacts {
  readonly lamports: bigint;
  readonly token: TokenFacts | null;
}

function readU64(bytes: Uint8Array, offset: number): bigint {
  let value = 0n;
  for (let index = 7; index >= 0; index--) {
    value = (value << 8n) | BigInt(bytes[offset + index]!);
  }
  return value;
}

// RPC lamports arrive as JSON numbers, so balances above 2^53 lamports lose
// precision when parsed. No single account the policy measures holds that.
function accountFacts(account: unknown): AccountFacts | null {
  if (account === null || account === undefined) return null;
  const lamports = BigInt(asU64(field(account, 'lamports')) ?? 0);
  const owner = asString(field(account, 'owner')) ?? '';
  let token: TokenFacts | null = null;
  if (isTokenProgram(owner)) {
    const data = field(account, 'data');
    const encoded = Array.isArray(data) ? asString(data[0]) : undefined;
    const bytes = encoded === undefined ? null : decodeBase64(encoded);
    if (bytes !== null && bytes.length >= 72) {
      token = {
        mint: encodeBase58(bytes.subarray(0, 32)),
        owner: encodeBase58(bytes.subarray(32, 64)),
        amount: readU64(bytes, 64),
      };
    }
  }
  return { lamports, token };
}

/** The accounts at `addresses` and the highest slot the RPC read them at. */
async function accountsAt(
  simulator: Simulator,
  addresses: readonly string[],
): Promise<{ accounts: unknown[]; slot: number | null }> {
  const accounts: unknown[] = [];
  let slot: number | null = null;
  for (let start = 0; start < addresses.length; start += 100) {
    const chunk = addresses.slice(start, start + 100);
    const result = await simulator.call('getMultipleAccounts', [
      chunk,
      { encoding: 'base64', commitment: 'confirmed' },
    ]);
    const values = field(result, 'value');
    if (!Array.isArray(values)) throw new Error('getMultipleAccounts returned no accounts');
    const read = asU64(field(field(result, 'context'), 'slot'));
    if (read !== undefined) slot = slot === null ? read : Math.max(slot, read);
    accounts.push(...values);
  }
  return { accounts, slot };
}

/** The lookup-table addresses of a v0 message. */
interface ResolvedLookups {
  /** Resolved writable entries. */
  readonly writable: string[];
  /** Every entry in message order (writable entries of each table, then read-only entries of each table); `null` where unresolved. */
  readonly keys: Array<string | null>;
  /** Tables whose writable entries could not be resolved. */
  readonly unresolved: string[];
}

function noLookups(): ResolvedLookups {
  return { writable: [], keys: [], unresolved: [] };
}

async function resolveLookups(
  simulator: Simulator,
  decoded: DecodedTransaction,
): Promise<ResolvedLookups> {
  const out = noLookups();
  if (decoded.lookups.length === 0) return out;
  let accounts: unknown[] = [];
  try {
    accounts = (
      await accountsAt(
        simulator,
        decoded.lookups.map((lookup) => lookup.table),
      )
    ).accounts;
  } catch {
    accounts = [];
  }
  const tableAddresses = decoded.lookups.map((_, index) => {
    const data = field(accounts[index], 'data');
    const encoded = Array.isArray(data) ? asString(data[0]) : undefined;
    const bytes = encoded === undefined ? null : decodeBase64(encoded);
    return bytes !== null && bytes.length >= LOOKUP_TABLE_HEADER
      ? bytes.subarray(LOOKUP_TABLE_HEADER)
      : null;
  });
  const entry = (table: number, position: number): string | null => {
    const addresses = tableAddresses[table];
    if (!addresses) return null;
    const start = position * 32;
    return start + 32 <= addresses.length
      ? encodeBase58(addresses.subarray(start, start + 32))
      : null;
  };
  decoded.lookups.forEach((lookup, index) => {
    for (const position of lookup.writable) {
      const key = entry(index, position);
      if (key !== null) out.writable.push(key);
      else if (!out.unresolved.includes(lookup.table)) out.unresolved.push(lookup.table);
      out.keys.push(key);
    }
  });
  decoded.lookups.forEach((lookup, index) => {
    for (const position of lookup.readonly) out.keys.push(entry(index, position));
  });
  return out;
}

/** What simulating one transaction showed about the signer. */
interface SignerEffect {
  /** Lamports: post minus pre. `null` when unknown. */
  readonly solLamports: bigint | null;
  /** Per mint: the signer's balance after minus before, over the token accounts the transaction writes. */
  readonly tokens: Map<string, bigint>;
  /** Lamports of token accounts the signer closes to another account; `null` when one could not be measured. */
  readonly closedLamportsOut: bigint | null;
  readonly err: unknown;
  readonly logsTail: string[];
  readonly unresolved: string[];
  readonly omittedAccounts: number;
  readonly findings: AuthorityFinding[];
  readonly innerChecked: boolean;
  readonly slots: [number | null, number | null];
}

function effectKnown(effect: SignerEffect): boolean {
  return (
    effect.solLamports !== null &&
    effect.unresolved.length === 0 &&
    effect.omittedAccounts === 0 &&
    effect.closedLamportsOut !== null
  );
}

/**
 * Simulate `transaction` (wire bytes) and measure its effect on `signer`.
 *
 * The pre-state is read with `getMultipleAccounts` and the simulation is
 * asked not to run on an older slot (`minContextSlot`), but the two are
 * separate reads: an account that changes in between skews the deltas.
 */
async function simulateSignerEffect(
  simulator: Simulator,
  transaction: Uint8Array,
  decoded: DecodedTransaction,
  signer: string,
  lookups: ResolvedLookups,
): Promise<SignerEffect> {
  const addresses: string[] = [];
  for (const key of [signer, ...writableStaticKeys(decoded), ...lookups.writable]) {
    if (!addresses.includes(key)) addresses.push(key);
  }
  const omittedAccounts = Math.max(0, addresses.length - MAX_SIMULATED_ACCOUNTS);
  addresses.length = Math.min(addresses.length, MAX_SIMULATED_ACCOUNTS);
  const accountKeys: Array<string | null> = [...decoded.staticKeys, ...lookups.keys];
  const checked: AuthorityChecked = checkTopLevel(decoded, signer, lookups.keys);
  const { accounts: before, slot: preSlot } = await accountsAt(simulator, addresses);
  const config: Record<string, unknown> = {
    encoding: 'base64',
    sigVerify: false,
    replaceRecentBlockhash: true,
    commitment: 'confirmed',
    innerInstructions: true,
    accounts: { encoding: 'base64', addresses },
  };
  if (preSlot !== null) config.minContextSlot = preSlot;
  const result = await simulator.call('simulateTransaction', [encodeBase64(transaction), config]);
  const value = field(result, 'value');
  const rawLogs = field(value, 'logs');
  const logs = Array.isArray(rawLogs)
    ? rawLogs.filter((line): line is string => typeof line === 'string')
    : [];
  const logsTail = logs.slice(Math.max(0, logs.length - LOG_TAIL));
  const err = field(value, 'err') ?? null;
  const inner = field(value, 'innerInstructions');
  const innerChecked = Array.isArray(inner);
  mergeChecked(checked, checkInner(inner, signer, accountKeys));
  const slots: [number | null, number | null] = [
    preSlot,
    asU64(field(field(result, 'context'), 'slot')) ?? null,
  ];
  const rawAfter = field(value, 'accounts');
  const after = Array.isArray(rawAfter) ? rawAfter : [];
  if (err !== null || after.length !== addresses.length) {
    return {
      solLamports: null,
      tokens: new Map(),
      closedLamportsOut: null,
      err,
      logsTail,
      unresolved: lookups.unresolved,
      omittedAccounts,
      findings: checked.findings,
      innerChecked,
      slots,
    };
  }
  let solLamports: bigint | null = null;
  const tokens = new Map<string, bigint>();
  const signersSide = (facts: AccountFacts | null): TokenFacts | null =>
    facts?.token && facts.token.owner === signer ? facts.token : null;
  for (const [index, address] of addresses.entries()) {
    const pre = accountFacts(before[index]);
    const post = accountFacts(after[index]);
    if (address === signer) {
      solLamports = (post?.lamports ?? 0n) - (pre?.lamports ?? 0n);
    }
    const preToken = signersSide(pre);
    if (preToken) tokens.set(preToken.mint, (tokens.get(preToken.mint) ?? 0n) - preToken.amount);
    const postToken = signersSide(post);
    if (postToken) {
      tokens.set(postToken.mint, (tokens.get(postToken.mint) ?? 0n) + postToken.amount);
    }
  }
  for (const [mint, delta] of tokens) if (delta === 0n) tokens.delete(mint);
  let closedLamportsOut: bigint | null = 0n;
  for (const closed of checked.closedToOthers) {
    let lamports: bigint | null = null;
    const index = closed === null ? -1 : addresses.indexOf(closed);
    const facts = index < 0 ? null : accountFacts(before[index]);
    if (facts) {
      const wrapped = facts.token && isNativeMint(facts.token.mint) ? facts.token.amount : 0n;
      lamports = facts.lamports > wrapped ? facts.lamports - wrapped : 0n;
    }
    closedLamportsOut =
      closedLamportsOut !== null && lamports !== null ? closedLamportsOut + lamports : null;
  }
  return {
    solLamports,
    tokens,
    closedLamportsOut,
    err,
    logsTail,
    unresolved: lookups.unresolved,
    omittedAccounts,
    findings: checked.findings,
    innerChecked,
    slots,
  };
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

/** The wire transactions of a prepare result, with their indexes. */
export function preparedTransactions(prepared: unknown): Array<[number, Uint8Array]> {
  const transactions = field(prepared, 'transactions');
  if (!Array.isArray(transactions)) throw new Error('the prepare returned no transactions');
  return transactions.map((transaction, position) => {
    const encoded = asString(field(transaction, 'transactionBase64'));
    if (encoded === undefined) throw new Error('a prepared transaction has no transactionBase64');
    const bytes = decodeBase64(encoded);
    if (bytes === null) throw new Error('a prepared transaction is not base64');
    return [asU64(field(transaction, 'index')) ?? position, bytes];
  });
}

function entryFor(list: unknown, index: number, position: number): unknown {
  if (!Array.isArray(list)) return null;
  const found =
    list.find((entry) => asU64(field(entry, 'index')) === index) ?? list[position];
  return found === undefined ? null : found;
}

function sortedTokens(tokens: Map<string, bigint>): TokenDelta[] {
  return [...tokens.keys()]
    .sort()
    .map((mint) => ({ mint, amount: tokens.get(mint)!.toString() }));
}

/**
 * The summary of a prepared step for `signer` (the wallet that signs).
 * `simulator` adds the simulated balance changes.
 */
export async function summarizePrepared(
  prepared: PreparedStepLike,
  signer: string,
  simulator?: Simulator | null,
): Promise<PreparedSummary> {
  const transactions = preparedTransactions(prepared);
  const entries: TransactionSummary[] = [];
  const programsAll: string[] = [];
  let solTotal: bigint | null = 0n;
  const tokenTotals = new Map<string, bigint>();
  const simulationErrors: PreparedSummary['simulationErrors'] = [];
  const stillRequired: string[] = [];
  for (const [position, [index, bytes]] of transactions.entries()) {
    let decoded: DecodedTransaction;
    try {
      decoded = decodeTransaction(bytes);
    } catch (error) {
      throw new Error(
        `prepared transaction ${index} does not decode: ${(error as Error).message}`,
      );
    }
    const anatomy = entryFor(field(prepared, 'anatomy'), index, position);
    const serverSimulation = entryFor(field(prepared, 'simulation'), index, position);
    const rawLabels = field(anatomy, 'instructions');
    const labels = Array.isArray(rawLabels) ? rawLabels : [];
    const programs = transactionPrograms(decoded).map((program) => {
      if (!programsAll.includes(program)) programsAll.push(program);
      const roles = new Set<string>();
      for (const label of labels) {
        if (field(label, 'programId') !== program) continue;
        const role = asString(field(label, 'role'));
        if (role === undefined) continue;
        const kind = asString(field(label, 'kind'));
        roles.add(kind !== undefined && kind !== role ? `${role}:${kind}` : role);
      }
      return { programId: program, roles: [...roles].sort() };
    });
    const signers = transactionSigners(decoded);
    const missing = signers.filter(([, signed]) => !signed).map(([key]) => key);
    for (const key of missing) if (!stillRequired.includes(key)) stillRequired.push(key);
    // Resolve once for both the displayed write set and simulation. Even if
    // simulation fails, the write set must not silently omit lookup keys.
    const lookups = simulator ? await resolveLookups(simulator, decoded) : noLookups();
    const accountsWritten = writableStaticKeys(decoded);
    const unresolvedWrites: Array<{ table: string; addressIndex: number }> = [];
    let offset = 0;
    for (const lookup of decoded.lookups) {
      for (const addressIndex of lookup.writable) {
        const key = lookups.keys[offset] ?? null;
        if (key !== null) accountsWritten.push(key);
        else unresolvedWrites.push({ table: lookup.table, addressIndex });
        offset += 1;
      }
    }
    const lookupWritable = decoded.lookups.reduce((sum, lookup) => sum + lookup.writable.length, 0);
    const dependent = field(serverSimulation, 'simulated') === false;
    let unsafeInstructions = checkTopLevel(decoded, signer, []).findings;
    let signerEffect: SignerEffectSummary;
    let simulation: TransactionSummary['simulation'];
    if (simulator) {
      try {
        const effect = await simulateSignerEffect(simulator, bytes, decoded, signer, lookups);
        unsafeInstructions = effect.findings;
        if (effect.err !== null) {
          simulationErrors.push({
            transaction: index,
            err: effect.err,
            dependsOnEarlierTransactions: dependent,
          });
        }
        solTotal =
          effect.solLamports !== null && solTotal !== null ? solTotal + effect.solLamports : null;
        for (const [mint, delta] of effect.tokens) {
          tokenTotals.set(mint, (tokenTotals.get(mint) ?? 0n) + delta);
        }
        signerEffect = {
          solLamports: effect.solLamports?.toString() ?? null,
          sol: effect.solLamports === null ? null : lamportsToSol(effect.solLamports),
          tokens: sortedTokens(effect.tokens),
          closedAccountLamportsOut: effect.closedLamportsOut?.toString() ?? null,
          known: effectKnown(effect),
          unresolvedLookupTables: effect.unresolved,
          omittedAccounts: effect.omittedAccounts,
          innerInstructionsChecked: effect.innerChecked,
          preStateSlot: effect.slots[0],
          simulatedSlot: effect.slots[1],
        };
        if (effect.omittedAccounts > 0) solTotal = null;
        simulation = { err: effect.err, logsTail: effect.logsTail };
      } catch (error) {
        solTotal = null;
        signerEffect = { known: false, error: (error as Error).message };
      }
    } else {
      solTotal = null;
      signerEffect = { known: false };
    }
    entries.push({
      index,
      version: decoded.version === null ? 'legacy' : `v${decoded.version}`,
      feePayer: decoded.staticKeys[0] ?? null,
      programs,
      accountsWritten,
      accountsWrittenComplete: unresolvedWrites.length === 0,
      unresolvedWritableAccounts: unresolvedWrites,
      signatures: {
        required: signers.map(([key]) => key),
        present: signers.filter(([, signed]) => signed).map(([key]) => key),
        missing,
      },
      serverSimulation,
      ...(lookupWritable > 0
        ? {
            lookupTables: decoded.lookups.map((lookup) => ({
              table: lookup.table,
              writableEntries: lookup.writable.length,
              readonlyEntries: lookup.readonly.length,
            })),
          }
        : {}),
      signerEffect,
      ...(simulation ? { simulation } : {}),
      unsafeInstructions,
    });
  }
  for (const [mint, delta] of tokenTotals) if (delta === 0n) tokenTotals.delete(mint);
  const signerBalanceChanges: PreparedSummary['signerBalanceChanges'] = simulator
    ? {
        available: true,
        complete: solTotal !== null && entries.every((entry) => entry.signerEffect.known),
        solLamports: solTotal?.toString() ?? null,
        sol: solTotal === null ? null : lamportsToSol(solTotal),
        tokens: sortedTokens(tokenTotals),
        note: 'Simulated one transaction at a time against current chain state; a transaction that depends on an earlier one may not simulate on its own.',
      }
    : {
        available: false,
        reason:
          "no RPC URL is configured to simulate the transactions, so the signer's balance changes are unknown",
      };
  return {
    signer,
    transactions: entries,
    programsInvoked: programsAll,
    signerBalanceChanges,
    simulationErrors,
    signaturesStillRequired: stillRequired,
  };
}
