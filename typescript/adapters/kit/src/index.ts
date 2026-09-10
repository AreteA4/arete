/**
 * @usearete/adapter-kit
 *
 * A reference {@link WalletAdapter} implementation backed by @solana/kit
 * (the functional successor to @solana/web3.js).
 *
 * The Arete core SDK is RPC-free: it only builds `BuiltInstruction` objects.
 * This adapter owns blockhash fetching, message construction, signing,
 * sending, and confirmation.
 */

import {
  address,
  createNoopSigner,
  addSignersToTransactionMessage,
  createTransactionMessage,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  setTransactionMessageComputeUnitLimit,
  setTransactionMessageComputeUnitPrice,
  setTransactionMessageHeapSize,
  setTransactionMessageLoadedAccountsDataSizeLimit,
  setTransactionMessageConfig,
  appendTransactionMessageInstructions,
  compileTransaction,
  getBase64Decoder,
  getBase64EncodedWireTransaction,
  getTransactionSize,
  signTransactionMessageWithSigners,
  getSignatureFromTransaction,
  assertIsTransactionWithBlockhashLifetime,
  isSolanaError,
  sendAndConfirmTransactionFactory,
  SOLANA_ERROR__JSON_RPC__SERVER_ERROR_SEND_TRANSACTION_PREFLIGHT_FAILURE,
  AccountRole,
  type Rpc,
  type RpcSubscriptions,
  type SolanaRpcApi,
  type SolanaRpcSubscriptionsApi,
  type TransactionSigner,
  type Instruction as KitInstruction,
  type AccountMeta as KitAccountMeta,
  type Commitment,
  type Signature,
  type Slot,
  type TransactionMessageBytesBase64,
} from '@solana/kit';
import type {
  WalletAdapter,
  BuiltInstruction,
  BuiltAccountMeta,
  SendOptions,
  SendResult,
  ConfirmationLevel,
  TransactionFailureOutcome,
  TransactionInspectionOptions,
  TransactionInspectionResult,
  TransactionTransport,
  TransactionVersion,
  ResolvedTransactionResourceOptions,
  WalletExecutionContext,
  TransactionBuildCapability,
} from '@usearete/sdk';
import {
  TransactionTransportError,
  resolveTransactionBuildOptions,
  toWireResourceOptions,
} from '@usearete/sdk';

/**
 * Kit 8 is the first release that constructs transaction V1 messages and
 * carries the resource budget as a typed message config, so this adapter
 * builds every version the contract defines and honours every typed
 * resource option.
 */
const CAPABILITY: TransactionBuildCapability = {
  supportedTransactionVersions: ['legacy', 0, 1],
  supportedResourceOptions: [
    'computeUnitLimit',
    'loadedAccountsDataSizeLimit',
    'heapSize',
    'priorityFeeLamports',
    'computeUnitPriceMicroLamports',
  ],
};

/** Wire ceiling for legacy and v0 transactions, in bytes (one UDP packet). */
export const LEGACY_TRANSACTION_SIZE_LIMIT = 1232;

/** Wire ceiling for V1 transactions, in bytes (SIMD-0385). */
export const V1_TRANSACTION_SIZE_LIMIT = 4096;

/** Structural V1 caps enforced before a signer is reached (SIMD-0385). */
export const V1_MAX_SIGNATURES = 12;
export const V1_MAX_ACCOUNTS = 64;
export const V1_MAX_INSTRUCTIONS = 64;

/**
 * Per-transaction compute-unit maximum. Also the limit a *provisional* V1
 * message declares for a budget the caller omitted: SIMD-0385 makes an unset
 * V1 config field request the **minimum**, not a default, so a provisional
 * message carrying nothing could never produce an ordinary successful
 * execution estimate.
 */
export const MAX_COMPUTE_UNIT_LIMIT = 1_400_000;

/** Loaded-account-data maximum (64 MiB), used the same way. */
export const MAX_LOADED_ACCOUNTS_DATA_SIZE = 64 * 1024 * 1024;

/**
 * Headroom added to measured compute consumption, in percent: a simulation
 * is one slot's view of the chain and the transaction that lands may take a
 * slightly different branch.
 */
export const ESTIMATED_COMPUTE_UNIT_HEADROOM_PERCENT = 20n;

/**
 * The runtime accounts for loaded data in pages of this size, so an
 * estimated data budget is rounded up to a whole page plus one page of
 * headroom for account growth between simulation and execution.
 */
export const LOADED_ACCOUNTS_DATA_PAGE_BYTES = 32 * 1024;

const COMPUTE_BUDGET_PROGRAM_ADDRESS = 'ComputeBudget111111111111111111111111111111';

/** Option keys that would carry address lookup tables. */
const LOOKUP_TABLE_OPTION_KEYS = [
  'addressLookupTables',
  'addressLookupTableAccounts',
  'lookupTables',
] as const;

export type AdapterTransportSelection = 'auto' | 'direct' | TransactionTransport;

export interface KitAdapterConfig {
  /** A Solana RPC client (from `createSolanaRpc`). */
  rpc?: Rpc<SolanaRpcApi>;
  /** A Solana RPC subscriptions client (from `createSolanaRpcSubscriptions`). */
  rpcSubscriptions?: RpcSubscriptions<SolanaRpcSubscriptionsApi>;
  transport?: AdapterTransportSelection;
  /** The fee-payer / signer for transactions. */
  signer: TransactionSigner;
  /** Optional local signers that can satisfy additional required signatures. */
  additionalSigners?: readonly TransactionSigner[];
  /** Default commitment used when the caller does not specify one. */
  defaultCommitment?: Commitment;
}

export interface KitSendOptions extends SendOptions {
  /** Extra local signers for this send only. */
  additionalSigners?: readonly TransactionSigner[];
  /** Override the fee payer for this send. */
  feePayer?: TransactionSigner;
  confirmationTimeoutMs?: number;
  statusPollIntervalMs?: number;
}

export interface KitTransactionInspectionOptions extends TransactionInspectionOptions {
  /** Commitment used to fetch the blockhash, fee, and simulation context. */
  commitment?: Commitment;
  /** Reject RPC responses evaluated before this slot. */
  minContextSlot?: Slot;
  /** Fee payer address used to compile the unsigned transaction. */
  feePayer?: string;
}

export interface KitTransactionInspectionResult extends TransactionInspectionResult {
  /** RPC context used for the fee estimate. */
  feeContextSlot?: number;
  /** The version the inspected message was compiled for. */
  transactionVersion: TransactionVersion;
  /**
   * The resource budget the inspected message actually declares, as decimal
   * strings. For a V1 message this includes the protocol maxima standing in
   * for the ceilings the caller omitted, which is what the reported metrics
   * let you replace with pinned values.
   */
  resources: Record<string, string>;
}

export interface KitWalletAdapter extends WalletAdapter {
  readonly signerAddresses: readonly string[];
  readonly supportedTransactionVersions: readonly ['legacy', 0, 1];
  signAndSend(
    instructions: readonly BuiltInstruction[],
    options?: KitSendOptions,
    context?: WalletExecutionContext
  ): Promise<SendResult>;
  inspectTransaction(
    instructions: readonly BuiltInstruction[],
    options?: KitTransactionInspectionOptions,
    context?: WalletExecutionContext
  ): Promise<KitTransactionInspectionResult>;
}

/** Structured adapter error consumed by the Arete transaction outcome APIs. */
export class KitTransactionExecutionError extends Error {
  readonly outcome: TransactionFailureOutcome;
  readonly cause: unknown;
  readonly signature?: string;
  readonly slot?: number;

  constructor(outcome: TransactionFailureOutcome) {
    super(outcomeMessage(outcome));
    this.name = 'KitTransactionExecutionError';
    this.outcome = outcome;
    this.cause = outcome.cause;
    this.signature = 'signature' in outcome ? outcome.signature : undefined;
    this.slot = 'slot' in outcome ? outcome.slot : undefined;
  }
}

interface SignatureStatus {
  readonly confirmationStatus: Commitment | null;
  readonly err: unknown | null;
  readonly slot: bigint;
}

function outcomeMessage(outcome: TransactionFailureOutcome): string {
  if (outcome.cause instanceof Error && outcome.cause.message) {
    return outcome.cause.message;
  }
  switch (outcome.status) {
    case 'not-submitted':
      return `Transaction was not submitted during ${outcome.phase}`;
    case 'submitted-unknown':
      return `Transaction ${outcome.signature} was submitted but its status is unknown`;
    case 'chain-failed':
      return outcome.signature
        ? `Transaction ${outcome.signature} failed on chain`
        : 'Transaction failed on chain';
  }
}

function chainFailureCause(confirmationError: unknown, transactionError: unknown): Error {
  const cause = new Error('Transaction failed on chain');
  Object.assign(cause, { cause: confirmationError, transactionError });
  return cause;
}

function toNumber(value: bigint): number {
  return Number(value);
}

function slotFromError(error: unknown): number | undefined {
  const seen = new Set<object>();
  let current = error;
  while (typeof current === 'object' && current !== null && !seen.has(current)) {
    seen.add(current);
    const candidate = current as {
      slot?: unknown;
      context?: { slot?: unknown };
      cause?: unknown;
    };
    const slot = candidate.slot ?? candidate.context?.slot;
    if (typeof slot === 'number' || typeof slot === 'bigint') {
      return Number(slot);
    }
    current = candidate.cause;
  }
  return undefined;
}

function hasReachedCommitment(
  actual: Commitment | null,
  required: Commitment
): boolean {
  if (actual === null) return false;
  const rank: Record<Commitment, number> = {
    processed: 0,
    confirmed: 1,
    finalized: 2,
  };
  return rank[actual] >= rank[required];
}

/** The backend one operation runs on, selected once before it starts. */
export type ResolvedTransport =
  | { readonly kind: 'arete'; readonly transport: TransactionTransport }
  | {
    readonly kind: 'direct';
    readonly rpc: Rpc<SolanaRpcApi>;
    readonly rpcSubscriptions: RpcSubscriptions<SolanaRpcSubscriptionsApi>;
  };

function resolveTransport(
  selection: AdapterTransportSelection | undefined,
  rpc: Rpc<SolanaRpcApi> | undefined,
  rpcSubscriptions: RpcSubscriptions<SolanaRpcSubscriptionsApi> | undefined,
  context: WalletExecutionContext | undefined
): ResolvedTransport {
  if (typeof selection === 'object') return { kind: 'arete', transport: selection };
  if (selection === 'direct') {
    if (!rpc || !rpcSubscriptions) throw new Error('Kit direct transport requires RPC and RPC subscriptions');
    return { kind: 'direct', rpc, rpcSubscriptions };
  }
  if (context?.transactionTransport) return { kind: 'arete', transport: context.transactionTransport };
  if (rpc && rpcSubscriptions) return { kind: 'direct', rpc, rpcSubscriptions };
  throw new Error('No transaction transport is available; connect through Arete or configure direct RPC');
}

/**
 * Report, never adopt, a relay signature that is not the one signed here.
 *
 * The adapter signs the final bytes and derives the signature from them, so
 * the local one is authoritative. Reconciling an echoed signature would
 * poll a transaction this adapter never submitted: it can report another
 * transaction's status as this one's, or never confirm the one that went
 * out.
 */
function reportSignatureMismatch(local: string, echoed: string | undefined): void {
  if (echoed && echoed !== local) {
    // eslint-disable-next-line no-console -- the adapter has no logger, and
    // this runs after the transaction may already be on the wire, so it
    // must not interrupt the signature-bearing outcome.
    console.warn(
      `Arete relay reported signature ${echoed} for a transaction signed as ${local}; `
      + 'the locally derived signature is authoritative and is the one being reconciled'
    );
  }
}

/** Thrown by {@link withDeadline} when the shared deadline passes first. */
class DeadlineExpiredError extends Error {
  constructor() {
    super('The configured confirmation deadline expired');
    this.name = 'DeadlineExpiredError';
  }
}

const DEADLINE_EXPIRED = Symbol('deadline-expired');

/**
 * Await `work`, or give up at `deadline`.
 *
 * A racing timer rather than an abort signal: `TransactionTransport` takes
 * none, and no HTTP client under it imposes a request timeout, so a backend
 * that stops answering would otherwise hold the caller forever. The
 * abandoned request is left with a no-op rejection handler — it may still be
 * in flight, which is exactly why the outcome is *unknown* rather than
 * failed.
 */
async function withDeadline<T>(deadline: number, work: Promise<T>): Promise<T> {
  work.catch(() => {});
  let timer: ReturnType<typeof setTimeout> | undefined;
  // Executor form, not `Promise.withResolvers`: that landed in Node 22 and
  // this package's floor is the Node 20.18 that Kit 8 requires.
  const expiry = new Promise<typeof DEADLINE_EXPIRED>((resolve) => {
    timer = setTimeout(() => resolve(DEADLINE_EXPIRED), Math.max(deadline - Date.now(), 0));
  });
  try {
    const result = await Promise.race([work, expiry]);
    if (result === DEADLINE_EXPIRED) throw new DeadlineExpiredError();
    return result;
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Poll until the signature reaches `commitment`, its lifetime expires, or
 * `deadline` passes — including while a request is in flight. Never
 * resubmits, and only ever polls the signature the caller signed.
 */
async function pollAreteStatus(
  transport: TransactionTransport,
  signature: string,
  commitment: Commitment,
  lastValidBlockHeight: bigint,
  deadline: number,
  options?: KitSendOptions
): Promise<SendResult> {
  const interval = options?.statusPollIntervalMs ?? 500;
  let emptyStatusPolls = 0;
  while (Date.now() <= deadline) {
    const status = await withDeadline(deadline, transport.getSignatureStatus(signature, {
      commitment, searchTransactionHistory: true,
    }));
    if (status?.err) {
      throw new KitTransactionExecutionError({
        status: 'chain-failed', phase: 'confirmation', signature,
        slot: status.slot === null ? undefined : Number(status.slot), cause: status.err,
      });
    }
    if (status && hasReachedCommitment(status.confirmationStatus, commitment)) {
      return { signature, slot: status.slot === null ? undefined : Number(status.slot) };
    }
    if (await withDeadline(deadline, transport.getBlockHeight({ commitment }))
      > lastValidBlockHeight) {
      throw new KitTransactionExecutionError({
        status: 'submitted-unknown', phase: 'confirmation', signature,
        slot: status?.slot === null ? undefined : Number(status?.slot),
        cause: new Error('Transaction blockhash expired before confirmation'),
      });
    }
    emptyStatusPolls = status ? 0 : emptyStatusPolls + 1;
    const backoffMs = Math.min(interval * (2 ** Math.min(emptyStatusPolls, 3)), 4_000);
    const delayMs = Math.min(backoffMs, Math.max(deadline - Date.now(), 0));
    await new Promise((resolve) => setTimeout(resolve, delayMs));
  }
  throw new KitTransactionExecutionError({
    status: 'submitted-unknown', phase: 'confirmation', signature,
    cause: new Error('Transaction confirmation timed out'),
  });
}

async function getSignatureStatus(
  rpc: Rpc<SolanaRpcApi>,
  signature: Signature,
  searchTransactionHistory: boolean
): Promise<SignatureStatus | null> {
  const { value } = await rpc
    .getSignatureStatuses([signature], { searchTransactionHistory })
    .send();
  return value[0] as SignatureStatus | null;
}

function toCommitment(
  level: ConfirmationLevel | undefined,
  fallback: Commitment
): Commitment {
  return (level as Commitment | undefined) ?? fallback;
}

function collectRequiredSignerAddresses(
  instructions: readonly BuiltInstruction[],
  feePayerAddress: string
): Set<string> {
  const required = new Set<string>([feePayerAddress]);

  for (const instruction of instructions) {
    for (const key of instruction.keys) {
      if (key.isSigner) {
        required.add(key.pubkey);
      }
    }
  }

  return required;
}

function indexLocalSigners(signers: readonly TransactionSigner[]): Map<string, TransactionSigner> {
  const indexed = new Map<string, TransactionSigner>();
  for (const signer of signers) {
    indexed.set(signer.address, signer);
  }
  return indexed;
}

/** Map an Arete account meta to a kit AccountRole. */
export function toAccountRole(meta: BuiltAccountMeta): AccountRole {
  if (meta.isSigner && meta.isWritable) return AccountRole.WRITABLE_SIGNER;
  if (meta.isSigner && !meta.isWritable) return AccountRole.READONLY_SIGNER;
  if (!meta.isSigner && meta.isWritable) return AccountRole.WRITABLE;
  return AccountRole.READONLY;
}

/** Map a kit AccountRole back to Arete signer/writable flags. */
export function fromAccountRole(role: AccountRole): { isSigner: boolean; isWritable: boolean } {
  switch (role) {
    case AccountRole.WRITABLE_SIGNER:
      return { isSigner: true, isWritable: true };
    case AccountRole.READONLY_SIGNER:
      return { isSigner: true, isWritable: false };
    case AccountRole.WRITABLE:
      return { isSigner: false, isWritable: true };
    default:
      return { isSigner: false, isWritable: false };
  }
}

/** Convert an Arete BuiltInstruction to a kit Instruction. */
export function toKitInstruction(ix: BuiltInstruction): KitInstruction {
  const accounts: KitAccountMeta[] = ix.keys.map((k) => ({
    address: address(k.pubkey),
    role: toAccountRole(k),
  }));
  return {
    programAddress: address(ix.programId),
    accounts,
    data: ix.data,
  };
}

/** Convert a kit Instruction to an Arete BuiltInstruction. */
export function fromKitInstruction(ix: KitInstruction): BuiltInstruction {
  return {
    programId: ix.programAddress,
    keys: (ix.accounts ?? []).map((account) => ({
      pubkey: account.address,
      ...fromAccountRole(account.role),
    })),
    data: ix.data ? new Uint8Array(ix.data) : new Uint8Array(0),
  };
}

// ---------------------------------------------------------------------------
// One version/configuration planner, shared by send and unsigned inspection
// ---------------------------------------------------------------------------

/**
 * Refuse the inputs the typed configuration owns.
 *
 * A hand-built `ComputeBudget` instruction is not an effective V1
 * configuration — the runtime reads V1 budgets from the message config — and
 * silently accepting one would apply a budget on legacy/v0 and none on V1
 * from the same caller code. Address lookup tables have no V1 encoding at
 * all, and this adapter compiles none for any version.
 */
function assertBuildableInputs(
  instructions: readonly BuiltInstruction[],
  options: TransactionInspectionOptions | SendOptions | undefined,
  version: TransactionVersion
): void {
  if (instructions.length === 0) {
    throw new Error('A transaction requires at least one instruction');
  }
  if (instructions.some((ix) => ix.programId === COMPUTE_BUDGET_PROGRAM_ADDRESS)) {
    throw new Error(
      'Caller-supplied ComputeBudget instructions are rejected: request compute units, heap '
      + 'size, loaded-accounts size and fees through the typed `resources` options so one '
      + 'contract covers every transaction version'
    );
  }
  const lookupTableKey = LOOKUP_TABLE_OPTION_KEYS.find(
    (key) => (options as Record<string, unknown> | undefined)?.[key] !== undefined
  );
  if (lookupTableKey) {
    throw new Error(
      `Address lookup tables are not supported by this adapter (rejected option `
      + `'${lookupTableKey}')`
      + (version === 1 ? '; transaction version 1 has no lookup tables at all (SIMD-0385)' : '')
    );
  }
}

function toUnits(value: bigint | undefined): number | undefined {
  return value === undefined ? undefined : Number(value);
}

interface PlanInput {
  readonly version: TransactionVersion;
  readonly resources: ResolvedTransactionResourceOptions;
  readonly instructions: readonly BuiltInstruction[];
  readonly latestBlockhash: { blockhash: string; lastValidBlockHeight: bigint };
  /**
   * The fee payer. Unsigned inspection passes a no-op signer for a bare
   * address, so both paths compile through one code path and the inspected
   * payload has the size and shape of the real one.
   */
  readonly feePayer: TransactionSigner;
  readonly attachedSigners?: readonly TransactionSigner[];
}

/**
 * A V1 message: the resource budget is the message's typed config, and the
 * runtime reads it from there — which is why a caller-supplied
 * `ComputeBudget` instruction is rejected rather than merged.
 */
function planV1Message(input: PlanInput) {
  const { resources } = input;
  const configured = setTransactionMessageConfig(
    {
      computeUnitLimit: toUnits(resources.computeUnitLimit),
      heapSize: toUnits(resources.heapSize),
      loadedAccountsDataSizeLimit: toUnits(resources.loadedAccountsDataSizeLimit),
      priorityFeeLamports: resources.priorityFeeLamports,
    },
    createTransactionMessage({ version: 1 })
  );
  const withFeePayer = setTransactionMessageFeePayerSigner(input.feePayer, configured);
  const withLifetime = setTransactionMessageLifetimeUsingBlockhash(
    input.latestBlockhash as Parameters<typeof setTransactionMessageLifetimeUsingBlockhash>[0],
    withFeePayer
  );
  return appendTransactionMessageInstructions(
    input.instructions.map(toKitInstruction),
    withLifetime
  );
}

/**
 * A legacy or v0 message: the same budget, rendered as the canonical
 * `ComputeBudget` instructions kit prepends.
 */
function planLegacyMessage(input: PlanInput) {
  const { resources } = input;
  const created = input.version === 0
    ? createTransactionMessage({ version: 0 })
    : createTransactionMessage({ version: 'legacy' });
  const withFeePayer = setTransactionMessageFeePayerSigner(input.feePayer, created);
  const withLifetime = setTransactionMessageLifetimeUsingBlockhash(
    input.latestBlockhash as Parameters<typeof setTransactionMessageLifetimeUsingBlockhash>[0],
    withFeePayer
  );
  let message = appendTransactionMessageInstructions(
    input.instructions.map(toKitInstruction),
    withLifetime
  );
  if (resources.computeUnitLimit !== undefined) {
    message = setTransactionMessageComputeUnitLimit(Number(resources.computeUnitLimit), message);
  }
  if (resources.computeUnitPriceMicroLamports !== undefined) {
    message = setTransactionMessageComputeUnitPrice(
      resources.computeUnitPriceMicroLamports,
      message
    );
  }
  if (resources.heapSize !== undefined) {
    message = setTransactionMessageHeapSize(Number(resources.heapSize), message);
  }
  if (resources.loadedAccountsDataSizeLimit !== undefined) {
    message = setTransactionMessageLoadedAccountsDataSizeLimit(
      Number(resources.loadedAccountsDataSizeLimit),
      message
    );
  }
  return message;
}

/**
 * One version/configuration planner for send and unsigned inspection: the
 * only difference between the two is whether the fee payer can sign.
 */
function planMessage(input: PlanInput) {
  const message = input.version === 1 ? planV1Message(input) : planLegacyMessage(input);
  return input.attachedSigners?.length
    ? addSignersToTransactionMessage([...input.attachedSigners], message)
    : message;
}

/**
 * The version's wire ceiling, checked on the compiled bytes before a signer
 * is ever reached.
 *
 * The V1 structural caps ({@link V1_MAX_SIGNATURES} signatures,
 * {@link V1_MAX_ACCOUNTS} accounts, {@link V1_MAX_INSTRUCTIONS} top-level
 * instructions) are enforced by `compileTransaction` itself, so they fail
 * here too — one compile earlier than this check, and equally before
 * signing. Re-implementing them would only risk disagreeing with the codec.
 */
function assertWithinSizeLimit(version: TransactionVersion, size: number): void {
  const limit = version === 1 ? V1_TRANSACTION_SIZE_LIMIT : LEGACY_TRANSACTION_SIZE_LIMIT;
  if (size > limit) {
    throw new Error(
      `Transaction is ${size} bytes, over the ${limit}-byte limit for version `
      + `${JSON.stringify(version)}`
    );
  }
}

/** Measured consumption plus headroom, positive and protocol-bounded. */
export function estimatedComputeUnitLimit(unitsConsumed: bigint): bigint {
  const padded = (unitsConsumed * (100n + ESTIMATED_COMPUTE_UNIT_HEADROOM_PERCENT) + 99n) / 100n;
  if (padded < 1n) return 1n;
  return padded > BigInt(MAX_COMPUTE_UNIT_LIMIT) ? BigInt(MAX_COMPUTE_UNIT_LIMIT) : padded;
}

/** Measured loaded data rounded up to a whole page plus one page of headroom. */
export function estimatedLoadedAccountsDataSize(loadedBytes: bigint): bigint {
  const page = BigInt(LOADED_ACCOUNTS_DATA_PAGE_BYTES);
  const padded = ((loadedBytes + page - 1n) / page + 1n) * page;
  const max = BigInt(MAX_LOADED_ACCOUNTS_DATA_SIZE);
  return padded > max ? max : padded;
}


interface SimulationMetrics {
  readonly contextSlot: bigint;
  readonly err: unknown;
  readonly logs?: readonly string[];
  /**
   * `undefined` means the backend reported no measurement; `0n` means it
   * measured zero. A V1 budget derived from a missing metric would be a
   * guess, so the distinction is preserved all the way to the refusal.
   */
  readonly unitsConsumed?: bigint;
  readonly loadedAccountsDataSize?: bigint;
}

async function fetchLatestBlockhash(
  resolved: ResolvedTransport,
  commitment: Commitment,
  minContextSlot?: Slot
): Promise<{ blockhash: string; lastValidBlockHeight: bigint }> {
  const value = resolved.kind === 'arete'
    ? await resolved.transport.getLatestBlockhash({ commitment, minContextSlot })
    : (await resolved.rpc.getLatestBlockhash({ commitment, minContextSlot }).send()).value;
  return {
    blockhash: value.blockhash,
    lastValidBlockHeight: BigInt(value.lastValidBlockHeight),
  };
}

/**
 * Simulate a transaction carrying placeholder signatures. Verification is
 * off on both backends: nothing on this path has touched a signer.
 */
async function simulateUnsigned(
  resolved: ResolvedTransport,
  wireTransaction: string,
  commitment: Commitment,
  minContextSlot?: Slot
): Promise<SimulationMetrics> {
  if (resolved.kind === 'arete') {
    const simulation = await resolved.transport.simulateTransaction(wireTransaction, {
      commitment,
      minContextSlot,
    });
    return {
      contextSlot: simulation.contextSlot,
      err: simulation.err ?? undefined,
      logs: simulation.logs ?? undefined,
      unitsConsumed: simulation.unitsConsumed,
      loadedAccountsDataSize: simulation.loadedAccountsDataSize,
    };
  }
  const simulation = await resolved.rpc.simulateTransaction(
    wireTransaction as Parameters<typeof resolved.rpc.simulateTransaction>[0],
    { commitment, encoding: 'base64', minContextSlot, sigVerify: false }
  ).send();
  // The loaded-accounts budget is optional upstream and comes back as null
  // when the node did not measure it; `Number(null)` would invent a measured
  // zero and destroy the distinction estimation depends on.
  const { loadedAccountsDataSize } = simulation.value as {
    loadedAccountsDataSize?: number | bigint | null;
  };
  return {
    contextSlot: simulation.context.slot,
    err: simulation.value.err ?? undefined,
    logs: simulation.value.logs ?? undefined,
    unitsConsumed: simulation.value.unitsConsumed == null
      ? undefined
      : BigInt(simulation.value.unitsConsumed),
    loadedAccountsDataSize: loadedAccountsDataSize == null
      ? undefined
      : BigInt(loadedAccountsDataSize),
  };
}

async function estimateFee(
  resolved: ResolvedTransport,
  encodedMessage: TransactionMessageBytesBase64,
  commitment: Commitment,
  minContextSlot?: Slot
): Promise<{ feeLamports?: bigint; contextSlot: bigint }> {
  if (resolved.kind === 'arete') {
    const fee = await resolved.transport.getFeeForMessage(encodedMessage, {
      commitment,
      minContextSlot,
    });
    return {
      feeLamports: fee.feeLamports === null ? undefined : fee.feeLamports,
      contextSlot: fee.contextSlot,
    };
  }
  const fee = await resolved.rpc
    .getFeeForMessage(encodedMessage, { commitment, minContextSlot })
    .send();
  return {
    feeLamports: fee.value === null ? undefined : fee.value,
    contextSlot: fee.context.slot,
  };
}

/**
 * The V1 message a simulation can actually execute: the caller's own budgets
 * where they gave them, the protocol maxima where they did not.
 */
function provisionalV1Resources(
  resources: ResolvedTransactionResourceOptions
): ResolvedTransactionResourceOptions {
  return {
    ...resources,
    computeUnitLimit: resources.computeUnitLimit ?? BigInt(MAX_COMPUTE_UNIT_LIMIT),
    loadedAccountsDataSizeLimit:
      resources.loadedAccountsDataSizeLimit ?? BigInt(MAX_LOADED_ACCOUNTS_DATA_SIZE),
  };
}

/**
 * Resolve the two budgets a signable V1 message must declare.
 *
 * An unset V1 budget requests the *minimum* rather than a default
 * (SIMD-0385), so a final message missing either could only fail on chain.
 * An explicit caller budget is used verbatim and never raised; a missing one
 * is measured by simulating the provisional message and derived with
 * headroom; and a metric the simulation never reported is refused by name.
 */
async function resolveV1Budgets(
  resolved: ResolvedTransport,
  compileProvisional: (resources: ResolvedTransactionResourceOptions) => string,
  resources: ResolvedTransactionResourceOptions,
  commitment: Commitment,
  minContextSlot?: Slot
): Promise<ResolvedTransactionResourceOptions> {
  if (
    resources.computeUnitLimit !== undefined
    && resources.loadedAccountsDataSizeLimit !== undefined
  ) {
    return resources;
  }

  const simulation = await simulateUnsigned(
    resolved,
    compileProvisional(provisionalV1Resources(resources)),
    commitment,
    minContextSlot
  );
  if (simulation.err) {
    throw new Error(
      `Budget estimation simulation reported ${JSON.stringify(simulation.err)}; `
      + 'nothing was submitted'
    );
  }

  const unestimable = (option: string, metric: string) => new Error(
    `${option} is required for transaction version 1 and could not be estimated: the `
    + `simulation reported no ${metric}. Pass an explicit ${option}, or use a backend whose `
    + `simulation reports ${metric}.`
  );
  let { computeUnitLimit, loadedAccountsDataSizeLimit } = resources;
  if (computeUnitLimit === undefined) {
    if (simulation.unitsConsumed === undefined) {
      throw unestimable('computeUnitLimit', 'unitsConsumed');
    }
    computeUnitLimit = estimatedComputeUnitLimit(simulation.unitsConsumed);
  }
  if (loadedAccountsDataSizeLimit === undefined) {
    if (simulation.loadedAccountsDataSize === undefined) {
      throw unestimable('loadedAccountsDataSizeLimit', 'loadedAccountsDataSize');
    }
    loadedAccountsDataSizeLimit = estimatedLoadedAccountsDataSize(
      simulation.loadedAccountsDataSize
    );
  }
  return { ...resources, computeUnitLimit, loadedAccountsDataSizeLimit };
}

/**
 * Create a {@link WalletAdapter} from a kit RPC pair and a signer.
 */
export function createWalletAdapter(config: KitAdapterConfig): KitWalletAdapter {
  const { rpc, rpcSubscriptions, signer } = config;
  const configuredLocalSigners = config.additionalSigners ?? [];
  const fallbackCommitment = config.defaultCommitment ?? 'confirmed';
  const signerAddresses = [signer.address, ...configuredLocalSigners.map(({ address }) => address)];

  return {
    publicKey: signer.address,
    signerAddresses: [...new Set(signerAddresses)],
    supportedTransactionVersions:
      CAPABILITY.supportedTransactionVersions as readonly ['legacy', 0, 1],

    async signAndSend(
      instructions: readonly BuiltInstruction[],
      options?: SendOptions,
      context?: WalletExecutionContext
    ): Promise<SendResult> {
      // Rejects an unsupported explicit version and any version-bound fee
      // used against the wrong version, before a wallet is ever prompted.
      let plan: { transactionVersion: TransactionVersion; resources: ResolvedTransactionResourceOptions };
      try {
        plan = resolveTransactionBuildOptions(options, CAPABILITY);
        assertBuildableInputs(instructions, options, plan.transactionVersion);
      } catch (cause) {
        throw new KitTransactionExecutionError({
          status: 'not-submitted',
          phase: 'build',
          cause,
        });
      }
      const version = plan.transactionVersion;

      const sendOptions = options as KitSendOptions | undefined;
      const feePayer = sendOptions?.feePayer ?? signer;
      const commitment = toCommitment(options?.confirmationLevel, fallbackCommitment);
      let resolved: ResolvedTransport;
      try {
        resolved = resolveTransport(config.transport, rpc, rpcSubscriptions, context);
      } catch (cause) {
        throw new KitTransactionExecutionError({ status: 'not-submitted', phase: 'build', cause });
      }
      let message;
      let lastValidBlockHeight: bigint;

      try {
        const requiredSignerAddresses = collectRequiredSignerAddresses(
          instructions,
          feePayer.address
        );
        const localSignerMap = indexLocalSigners([
          signer,
          ...configuredLocalSigners,
          ...((sendOptions?.signers ?? []) as readonly TransactionSigner[]),
          ...(sendOptions?.additionalSigners ?? []),
          ...(sendOptions?.feePayer ? [sendOptions.feePayer] : []),
        ]);
        const missingSignerAddresses = [...requiredSignerAddresses].filter(
          (requiredAddress) => !localSignerMap.has(requiredAddress)
        );
        if (missingSignerAddresses.length > 0) {
          throw new Error(
            `Missing signer(s) for transaction: ${missingSignerAddresses.join(', ')}`
          );
        }

        const attachedSigners = [...requiredSignerAddresses]
          .filter((requiredAddress) => requiredAddress !== feePayer.address)
          .map((requiredAddress) => localSignerMap.get(requiredAddress)!);
        const latestBlockhash = await fetchLatestBlockhash(resolved, commitment);
        lastValidBlockHeight = latestBlockhash.lastValidBlockHeight;
        const compile = (resources: ResolvedTransactionResourceOptions) => planMessage({
          version,
          resources,
          instructions,
          latestBlockhash,
          feePayer,
          attachedSigners,
        });

        // The budgets are resolved before the final message exists, so the
        // configuration that is compiled, sized, preflighted and signed is
        // one configuration. The provisional message is sized too: an
        // oversized payload is a build failure with an exact byte count,
        // not a backend-specific simulation error.
        const encodeSized = (compiled: ReturnType<typeof compileTransaction>) => {
          assertWithinSizeLimit(version, getTransactionSize(compiled));
          return getBase64EncodedWireTransaction(compiled);
        };
        const resources = version === 1
          ? await resolveV1Budgets(
            resolved,
            (provisional) => encodeSized(compileTransaction(compile(provisional))),
            plan.resources,
            commitment
          )
          : plan.resources;
        message = compile(resources);
        assertWithinSizeLimit(version, getTransactionSize(compileTransaction(message)));
      } catch (cause) {
        throw new KitTransactionExecutionError({
          status: 'not-submitted',
          phase: 'build',
          cause,
        });
      }

      let signedTransaction;
      let signature: Signature;
      try {
        signedTransaction = await signTransactionMessageWithSigners(message);
        assertIsTransactionWithBlockhashLifetime(signedTransaction);
        signature = getSignatureFromTransaction(signedTransaction);
      } catch (cause) {
        throw new KitTransactionExecutionError({
          status: 'not-submitted',
          phase: 'wallet',
          cause,
        });
      }

      if (resolved.kind === 'arete') {
        // One deadline over submission *and* confirmation: a relay that
        // takes the transaction and then stalls would otherwise hold the
        // caller forever, because no HTTP client here imposes a request
        // timeout. Expiry on either side is submitted-unknown under the
        // signature these bytes carry — never a rebuild, re-sign or resend.
        const deadline = Date.now() + (sendOptions?.confirmationTimeoutMs ?? 60_000);
        try {
          const sent = await withDeadline(deadline, resolved.transport.sendTransaction(
            getBase64EncodedWireTransaction(signedTransaction),
            { skipPreflight: options?.skipPreflight ?? false, preflightCommitment: commitment }
          ));
          reportSignatureMismatch(signature, sent.signature);
        } catch (cause) {
          if (cause instanceof DeadlineExpiredError) {
            throw new KitTransactionExecutionError({
              status: 'submitted-unknown', phase: 'send', signature,
              cause: new Error(
                'The relay did not acknowledge the submission before the confirmation '
                + 'deadline; the transaction was dispatched once and may still land'
              ),
            });
          }
          if (cause instanceof TransactionTransportError && cause.submissionState === 'not_submitted') {
            throw new KitTransactionExecutionError({ status: 'not-submitted', phase: 'send', cause });
          }
          if (cause instanceof TransactionTransportError) {
            reportSignatureMismatch(signature, cause.signature);
          }
          // The locally derived signature stays authoritative: it is the one
          // these bytes carry, so a relay reporting another identifies some
          // other transaction and cannot be what the caller reconciles.
          throw new KitTransactionExecutionError({
            status: 'submitted-unknown', phase: 'send', signature, cause,
          });
        }
        try {
          return await pollAreteStatus(
            resolved.transport, signature, commitment, lastValidBlockHeight, deadline, sendOptions
          );
        } catch (cause) {
          if (cause instanceof KitTransactionExecutionError) throw cause;
          throw new KitTransactionExecutionError({
            status: 'submitted-unknown', phase: 'confirmation', signature, cause,
          });
        }
      }

      const directRpc = resolved.rpc;
      try {
        const sendAndConfirm = sendAndConfirmTransactionFactory({
          rpc: resolved.rpc,
          rpcSubscriptions: resolved.rpcSubscriptions,
        });
        await sendAndConfirm(signedTransaction, {
          commitment,
          skipPreflight: options?.skipPreflight ?? false,
        });
      } catch (cause) {
        if (
          isSolanaError(
            cause,
            SOLANA_ERROR__JSON_RPC__SERVER_ERROR_SEND_TRANSACTION_PREFLIGHT_FAILURE
          )
        ) {
          throw new KitTransactionExecutionError({
            status: 'not-submitted',
            phase: 'send',
            cause,
          });
        }

        let status: SignatureStatus | null = null;
        try {
          status = await getSignatureStatus(directRpc, signature, true);
        } catch {
          // The confirmation error remains the authoritative cause.
        }

        const slot = status ? toNumber(status.slot) : slotFromError(cause);
        if (status?.err) {
          throw new KitTransactionExecutionError({
            status: 'chain-failed',
            phase: 'confirmation',
            signature,
            slot,
            cause: chainFailureCause(cause, status.err),
          });
        }
        if (status && hasReachedCommitment(status.confirmationStatus, commitment)) {
          return { signature, slot };
        }
        throw new KitTransactionExecutionError({
          status: 'submitted-unknown',
          phase: 'confirmation',
          signature,
          slot,
          cause,
        });
      }

      try {
        const status = await getSignatureStatus(directRpc, signature, false);
        if (status?.err) {
          throw new KitTransactionExecutionError({
            status: 'chain-failed',
            phase: 'confirmation',
            signature,
            slot: toNumber(status.slot),
            cause: status.err,
          });
        }
        return {
          signature,
          slot: status ? toNumber(status.slot) : undefined,
        };
      } catch (cause) {
        if (cause instanceof KitTransactionExecutionError) throw cause;
        return { signature };
      }
    },

    async inspectTransaction(
      instructions: readonly BuiltInstruction[],
      options?: TransactionInspectionOptions,
      context?: WalletExecutionContext
    ): Promise<KitTransactionInspectionResult> {
      const plan = resolveTransactionBuildOptions(options, CAPABILITY);
      const version = plan.transactionVersion;
      assertBuildableInputs(instructions, options, version);

      const inspectionOptions = options as KitTransactionInspectionOptions | undefined;
      const commitment = inspectionOptions?.commitment ?? fallbackCommitment;
      const minContextSlot = inspectionOptions?.minContextSlot;
      const resolved = resolveTransport(config.transport, rpc, rpcSubscriptions, context);
      const latestBlockhash = await fetchLatestBlockhash(resolved, commitment, minContextSlot);

      // Inspection is where you discover the budgets to pin, so an omitted
      // V1 ceiling becomes the protocol maximum rather than the SIMD-0385
      // minimum: that provisional message is what the reported metrics
      // describe. Nothing here touches a signer — the fee payer is a bare
      // address and the compiled transaction carries placeholder signatures.
      const resources = version === 1 ? provisionalV1Resources(plan.resources) : plan.resources;
      const message = planMessage({
        version,
        resources,
        instructions,
        latestBlockhash,
        feePayer: createNoopSigner(address(inspectionOptions?.feePayer ?? signer.address)),
      });
      const unsignedTransaction = compileTransaction(message);
      // The same ceiling the send path enforces: a payload that could only
      // be rejected on submission is refused here with its byte count,
      // rather than answered with inspection results or a backend-specific
      // simulation error.
      assertWithinSizeLimit(version, getTransactionSize(unsignedTransaction));
      const wireTransaction = getBase64EncodedWireTransaction(unsignedTransaction);
      const encodedMessage = getBase64Decoder().decode(
        unsignedTransaction.messageBytes
      ) as TransactionMessageBytesBase64;

      const [fee, simulation] = await Promise.all([
        estimateFee(resolved, encodedMessage, commitment, minContextSlot),
        simulateUnsigned(resolved, wireTransaction, commitment, minContextSlot),
      ]);
      return {
        feeLamports: fee.feeLamports === undefined ? undefined : toNumber(fee.feeLamports),
        logs: simulation.logs === undefined ? undefined : [...simulation.logs],
        computeUnitsConsumed: simulation.unitsConsumed === undefined
          ? undefined
          : toNumber(simulation.unitsConsumed),
        contextSlot: toNumber(simulation.contextSlot),
        error: simulation.err ?? undefined,
        loadedAccountsDataSize: simulation.loadedAccountsDataSize === undefined
          ? undefined
          : toNumber(simulation.loadedAccountsDataSize),
        feeContextSlot: toNumber(fee.contextSlot),
        transactionVersion: version,
        resources: toWireResourceOptions(resources),
      };
    },
  };
}

export {
  SOLANA_SIGN_AND_SEND_TRANSACTION,
  SOLANA_SIGN_TRANSACTION,
  WalletStandardSignerError,
  createWalletStandardSigner,
  type WalletStandardAccount,
  type WalletStandardSignTransactionFeature,
  type WalletStandardSignTransactionInput,
  type WalletStandardSignTransactionOutput,
  type WalletStandardSignerConfig,
  type WalletStandardSignerErrorCode,
  type WalletStandardTransactionVersion,
  type WalletStandardWallet,
} from './wallet-standard';
