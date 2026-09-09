/**
 * Wallet adapter boundary for the Arete SDK.
 *
 * The core SDK is intentionally RPC-free: it only constructs `BuiltInstruction`
 * objects. Everything network-related (recent blockhash, message compilation,
 * signing, sending, and confirmation) lives behind the `WalletAdapter`
 * boundary, implemented by adapters that wrap the Solana library of your choice
 * (@solana/web3.js, @solana/kit, a raw Keypair signer for scripts, etc.).
 */

import type { TransactionTransport } from '../transactions';

/**
 * A single account reference within a built instruction.
 */
export interface BuiltAccountMeta {
  /** Account address as a base58-encoded string */
  pubkey: string;
  /** Whether this account must sign the transaction */
  isSigner: boolean;
  /** Whether this account is writable */
  isWritable: boolean;
}

/**
 * A framework-agnostic representation of a Solana instruction.
 *
 * This is the boundary type between the core SDK (which builds instructions)
 * and wallet adapters (which broadcast them). It maps 1:1 onto a
 * @solana/web3.js `TransactionInstruction` or a @solana/kit `Instruction`.
 */
export interface BuiltInstruction {
  /** Program ID (base58) */
  programId: string;
  /** Account keys, in the exact order required by the program */
  keys: BuiltAccountMeta[];
  /** Serialized instruction data (discriminator + Borsh-encoded args) */
  data: Uint8Array;
}

/**
 * Confirmation level for transaction processing.
 * - `processed`: Transaction processed but not confirmed
 * - `confirmed`: Transaction confirmed by cluster
 * - `finalized`: Transaction finalized (recommended for production)
 */
export type ConfirmationLevel = 'processed' | 'confirmed' | 'finalized';

/**
 * Transaction version requested by the caller.
 *
 * Numeric versions stay JSON numbers; they are versions, not u64 quantities.
 */
export type TransactionVersion = 'legacy' | 0 | 1;

/**
 * u32 resource quantity: an exact integer (`number` or `bigint`) or a decimal
 * string. Only representations that may already have lost precision are
 * rejected, and below 2^32 a `number` cannot.
 */
export type ResourceUnits = number | bigint | string;

/**
 * u64 resource quantity: a `bigint` or a decimal string.
 *
 * A plain `number` is rejected: every JS number is a double, so a u64 that
 * arrived as one may already have lost precision.
 */
export type ResourceLamports = bigint | string;

/**
 * Resource budget options shared by sending and unsigned inspection.
 *
 * The two fee fields are mutually exclusive and version-bound:
 * `priorityFeeLamports` is V1 only, `computeUnitPriceMicroLamports` is
 * legacy/v0 only.
 */
export interface TransactionResourceOptions {
  /** Compute unit ceiling (u32). */
  computeUnitLimit?: ResourceUnits;
  /** Loaded account data ceiling in bytes (u32). */
  loadedAccountsDataSizeLimit?: ResourceUnits;
  /** Heap frame bytes (u32). */
  heapSize?: ResourceUnits;
  /** Total priority fee in lamports (u64). Transaction V1 only. */
  priorityFeeLamports?: ResourceLamports;
  /** Per-compute-unit price in micro-lamports (u64). Legacy/v0 only. */
  computeUnitPriceMicroLamports?: ResourceLamports;
}

/** Canonical resource option keys, in contract order. */
export const TRANSACTION_RESOURCE_OPTION_KEYS = [
  'computeUnitLimit',
  'loadedAccountsDataSizeLimit',
  'heapSize',
  'priorityFeeLamports',
  'computeUnitPriceMicroLamports',
] as const;

export type TransactionResourceOptionKey = typeof TRANSACTION_RESOURCE_OPTION_KEYS[number];

/**
 * Version + resource options understood by version-aware adapters.
 *
 * The resource budget is a nested, closed object: an unrecognized key inside
 * `resources` is rejected, which a flat shape could not enforce against the
 * open index signature on {@link SendOptions}.
 */
export interface TransactionBuildOptions {
  /** Explicit transaction version. Omitted means the adapter's default (v0). */
  transactionVersion?: TransactionVersion;
  /** Resource budget for this transaction. */
  resources?: TransactionResourceOptions;
}

/** Adapter capability declaration consulted before a build. */
export interface TransactionBuildCapability {
  /**
   * Versions the adapter can build. `undefined` means unknown, never empty:
   * ordinary operations keep working, but an explicit V1 request fails.
   */
  readonly supportedTransactionVersions?: readonly TransactionVersion[];
  /**
   * Resource options the adapter honours. `undefined` means unknown and is not
   * policed; an empty array means the adapter honours none and every supplied
   * resource option is rejected rather than silently dropped.
   */
  readonly supportedResourceOptions?: readonly TransactionResourceOptionKey[];
}

export type TransactionOptionsErrorCode =
  | 'unsupported_transaction_version'
  | 'unsupported_resource_option'
  | 'invalid_transaction_option';

/** Typed rejection for malformed or unsupported build options. */
export class TransactionOptionsError extends Error {
  readonly code: TransactionOptionsErrorCode;
  readonly option?: string;
  readonly requestedVersion?: TransactionVersion;
  readonly supportedVersions?: readonly TransactionVersion[];

  constructor(
    code: TransactionOptionsErrorCode,
    message: string,
    details: {
      option?: string;
      requestedVersion?: TransactionVersion;
      supportedVersions?: readonly TransactionVersion[];
    } = {}
  ) {
    super(message);
    this.name = 'TransactionOptionsError';
    this.code = code;
    this.option = details.option;
    this.requestedVersion = details.requestedVersion;
    this.supportedVersions = details.supportedVersions;
  }
}

/** Resource budget after validation: every quantity normalized to a bigint. */
export interface ResolvedTransactionResourceOptions {
  readonly computeUnitLimit?: bigint;
  readonly loadedAccountsDataSizeLimit?: bigint;
  readonly heapSize?: bigint;
  readonly priorityFeeLamports?: bigint;
  readonly computeUnitPriceMicroLamports?: bigint;
}

/** Build options after validation. */
export interface ResolvedTransactionBuildOptions {
  /** Effective version: the explicit request, or the v0 default. */
  readonly transactionVersion: TransactionVersion;
  readonly resources: ResolvedTransactionResourceOptions;
}

const U32_MAX = 0xffff_ffffn;
const U64_MAX = (1n << 64n) - 1n;
const DECIMAL = /^\d+$/;

function inRange(value: bigint, max: bigint, option: TransactionResourceOptionKey): bigint {
  if (value > max) {
    throw new TransactionOptionsError(
      'invalid_transaction_option',
      `Transaction option '${option}' exceeds its maximum of ${max}`,
      { option }
    );
  }
  return value;
}

function parseUnits(
  value: ResourceUnits | undefined,
  option: TransactionResourceOptionKey
): bigint | undefined {
  if (value === undefined) return undefined;
  if (typeof value === 'bigint') {
    if (value < 0n) {
      throw new TransactionOptionsError(
        'invalid_transaction_option',
        `Transaction option '${option}' must not be negative`,
        { option }
      );
    }
    return inRange(value, U32_MAX, option);
  }
  if (typeof value === 'number') {
    if (!Number.isInteger(value) || value < 0) {
      throw new TransactionOptionsError(
        'invalid_transaction_option',
        `Transaction option '${option}' must be a non-negative integer or decimal string, `
        + `received ${value}`,
        { option }
      );
    }
    return inRange(BigInt(value), U32_MAX, option);
  }
  if (typeof value === 'string' && DECIMAL.test(value)) {
    return inRange(BigInt(value), U32_MAX, option);
  }
  throw new TransactionOptionsError(
    'invalid_transaction_option',
    `Transaction option '${option}' must be a non-negative integer or decimal string`,
    { option }
  );
}

function parseLamports(
  value: ResourceLamports | undefined,
  option: TransactionResourceOptionKey
): bigint | undefined {
  if (value === undefined) return undefined;
  if (typeof value === 'number') {
    throw new TransactionOptionsError(
      'invalid_transaction_option',
      `Transaction option '${option}' must be a bigint or decimal string, never a number: `
      + 'every JavaScript number is a double, so a u64 passed as one may already have lost '
      + `precision before it reached the SDK (received ${value})`,
      { option }
    );
  }
  if (typeof value === 'bigint') {
    if (value < 0n) {
      throw new TransactionOptionsError(
        'invalid_transaction_option',
        `Transaction option '${option}' must not be negative`,
        { option }
      );
    }
    return inRange(value, U64_MAX, option);
  }
  if (typeof value === 'string' && DECIMAL.test(value)) {
    return inRange(BigInt(value), U64_MAX, option);
  }
  throw new TransactionOptionsError(
    'invalid_transaction_option',
    `Transaction option '${option}' must be a bigint or decimal string`,
    { option }
  );
}

/**
 * Merge execution defaults with per-call send options.
 *
 * `resources` merges key by key, so a per-call fee does not discard a
 * configured compute budget — except for the two fee fields, which are one
 * slot: setting either clears the other, or the merged budget would hold both
 * and be refused as mutually exclusive. Shared by every entry point that has
 * defaults to merge; duplicating this logic is how `session` came to disagree
 * with `Arete.execute`.
 */
export function mergeSendOptions<T extends TransactionBuildOptions>(
  defaults: T | undefined,
  override: T | undefined
): T | undefined {
  if (!defaults) return override;
  if (!override) return defaults;

  const inherited = { ...defaults.resources };
  if (
    override.resources?.priorityFeeLamports !== undefined ||
    override.resources?.computeUnitPriceMicroLamports !== undefined
  ) {
    delete inherited.priorityFeeLamports;
    delete inherited.computeUnitPriceMicroLamports;
  }
  const resources =
    defaults.resources || override.resources
      ? { ...inherited, ...override.resources }
      : undefined;

  return { ...defaults, ...override, ...(resources ? { resources } : {}) };
}

/**
 * Validate build options against an adapter's declared capability and
 * normalize every quantity to a bigint.
 *
 * Rejections are never silent downgrades: an explicit version the adapter does
 * not advertise fails, and a fee field used against the wrong version fails
 * instead of being converted.
 */
export function resolveTransactionBuildOptions(
  options?: TransactionBuildOptions,
  capability?: TransactionBuildCapability
): ResolvedTransactionBuildOptions {
  const requested = options?.transactionVersion;
  if (requested !== undefined && requested !== 'legacy' && requested !== 0 && requested !== 1) {
    throw new TransactionOptionsError(
      'invalid_transaction_option',
      `Unknown transactionVersion ${JSON.stringify(requested)}: expected 'legacy', 0, or 1`,
      { option: 'transactionVersion' }
    );
  }

  const supported = capability?.supportedTransactionVersions;
  if (requested !== undefined) {
    // Undeclared capability means unknown, not unsupported: old versions keep
    // working, and only an explicit V1 request needs proof of support.
    const unsupported = supported ? !supported.includes(requested) : requested === 1;
    if (unsupported) {
      throw new TransactionOptionsError(
        'unsupported_transaction_version',
        `Wallet adapter does not support transaction version ${JSON.stringify(requested)}`
        + (supported
          ? ` (supported: ${supported.map((version) => JSON.stringify(version)).join(', ')})`
          : ' (adapter declares no supported versions)'),
        { option: 'transactionVersion', requestedVersion: requested, supportedVersions: supported }
      );
    }
  }

  const resources = options?.resources;
  if (resources !== undefined) {
    if (typeof resources !== 'object' || resources === null || Array.isArray(resources)) {
      throw new TransactionOptionsError(
        'invalid_transaction_option',
        'Transaction option \'resources\' must be an object of canonical resource options',
        { option: 'resources' }
      );
    }
    // Closed key set: an unrecognized budget key is a rejection, never a
    // silently ignored passthrough.
    const unknownKey = Object.keys(resources).find(
      (key) => !(TRANSACTION_RESOURCE_OPTION_KEYS as readonly string[]).includes(key)
    );
    if (unknownKey) {
      throw new TransactionOptionsError(
        'unsupported_resource_option',
        `Unknown resource option '${unknownKey}': expected one of `
        + TRANSACTION_RESOURCE_OPTION_KEYS.join(', '),
        { option: unknownKey }
      );
    }
  }

  const honoured = capability?.supportedResourceOptions;
  if (honoured) {
    const rejected = TRANSACTION_RESOURCE_OPTION_KEYS.find(
      (key) => resources?.[key] !== undefined && !honoured.includes(key)
    );
    if (rejected) {
      throw new TransactionOptionsError(
        'unsupported_resource_option',
        `Wallet adapter does not apply the '${rejected}' resource option`,
        { option: rejected }
      );
    }
  }

  const version = requested ?? 0;
  if (
    resources?.priorityFeeLamports !== undefined
    && resources?.computeUnitPriceMicroLamports !== undefined
  ) {
    throw new TransactionOptionsError(
      'invalid_transaction_option',
      'priorityFeeLamports and computeUnitPriceMicroLamports are mutually exclusive',
      { option: 'priorityFeeLamports', requestedVersion: version }
    );
  }
  if (resources?.priorityFeeLamports !== undefined && version !== 1) {
    throw new TransactionOptionsError(
      'unsupported_resource_option',
      `priorityFeeLamports requires transaction version 1, not ${JSON.stringify(version)}`,
      { option: 'priorityFeeLamports', requestedVersion: version }
    );
  }
  if (resources?.computeUnitPriceMicroLamports !== undefined && version === 1) {
    throw new TransactionOptionsError(
      'unsupported_resource_option',
      'computeUnitPriceMicroLamports applies to legacy/v0 only; version 1 uses priorityFeeLamports',
      { option: 'computeUnitPriceMicroLamports', requestedVersion: version }
    );
  }

  return {
    transactionVersion: version,
    resources: {
      computeUnitLimit: parseUnits(resources?.computeUnitLimit, 'computeUnitLimit'),
      loadedAccountsDataSizeLimit: parseUnits(
        resources?.loadedAccountsDataSizeLimit,
        'loadedAccountsDataSizeLimit'
      ),
      heapSize: parseUnits(resources?.heapSize, 'heapSize'),
      priorityFeeLamports: parseLamports(resources?.priorityFeeLamports, 'priorityFeeLamports'),
      computeUnitPriceMicroLamports: parseLamports(
        resources?.computeUnitPriceMicroLamports,
        'computeUnitPriceMicroLamports'
      ),
    },
  };
}

/**
 * Boundary form of a resolved resource budget: flat canonical camelCase keys,
 * every present value a decimal string, absent keys omitted.
 *
 * Mirrors Rust `TransactionResourceOptions::to_json()` and Python `to_wire()`.
 * Key order is not part of the contract (Rust sorts, Python uses declaration
 * order); the keys and their decimal-string values are. bigint stays the
 * in-process type; `JSON.stringify` cannot emit one.
 */
export function toWireResourceOptions(
  resources: ResolvedTransactionResourceOptions
): Record<string, string> {
  const wire: Record<string, string> = {};
  for (const key of TRANSACTION_RESOURCE_OPTION_KEYS) {
    const value = resources[key];
    if (value !== undefined) wire[key] = value.toString(10);
  }
  return wire;
}

/**
 * Options forwarded to the wallet adapter when sending a transaction.
 *
 * The core SDK does not interpret these; it passes them straight through to
 * the adapter, which owns all RPC semantics.
 */
export interface SendOptions extends TransactionBuildOptions {
  /** Confirmation level the adapter should wait for */
  confirmationLevel?: ConfirmationLevel;
  /** Skip the RPC preflight simulation */
  skipPreflight?: boolean;
  /**
   * Optional extra local signers for this send.
   *
   * The concrete signer type depends on the wallet adapter implementation
   * (for example `@solana/web3.js` Signers or `@solana/kit` TransactionSigners).
   */
  signers?: readonly unknown[];
  /** Adapter-specific passthrough options (priority fees, lookup tables, etc.) */
  [key: string]: unknown;
}

/**
 * Result returned by a wallet adapter after broadcasting a transaction.
 */
export interface SendResult {
  /** Transaction signature (base58) */
  signature: string;
  /** Slot in which the transaction landed, if the adapter reports it */
  slot?: number;
}

/**
 * Adapter-specific options for unsigned transaction inspection.
 *
 * Inspection must not sign or submit the transaction. Concrete adapters may
 * accept additional simulation options through this passthrough object.
 */
export interface TransactionInspectionOptions extends TransactionBuildOptions {
  [key: string]: unknown;
}

/**
 * Unsigned transaction inspection returned by a capable wallet adapter.
 */
export interface TransactionInspectionResult {
  /** Estimated transaction fee in lamports, when available. */
  feeLamports?: number;
  /** Program logs produced by simulation, when available. */
  logs?: readonly string[];
  /** Compute units consumed by simulation, when available. */
  computeUnitsConsumed?: number;
  /** RPC context slot for the inspection, when available. */
  contextSlot?: number;
  /** Loaded account data size reported by simulation, when available. */
  loadedAccountsDataSize?: number;
  /** Raw simulation failure, if the inspected transaction would fail. */
  error?: unknown;
  /** Adapter-specific inspection fields. */
  [key: string]: unknown;
}

export interface WalletExecutionContext {
  transactionTransport?: TransactionTransport;
}

/**
 * Wallet adapter interface for signing and sending transactions.
 *
 * Implementations own blockhash fetching, message compilation (legacy or v0),
 * signing, sending, and confirmation. The core SDK only needs `publicKey` for
 * signer-account resolution and `signAndSend` to broadcast built instructions.
 */
export interface WalletAdapter {
  /** The wallet's public key as a base58-encoded string */
  publicKey: string;

  /** Signer addresses the adapter can satisfy without per-send signers. */
  readonly signerAddresses?: readonly string[];

  /**
   * Transaction versions this adapter can build.
   *
   * Omitting the field means unknown, not unsupported: existing operations keep
   * working, but an explicit V1 request is rejected rather than downgraded.
   */
  readonly supportedTransactionVersions?: readonly TransactionVersion[];

  /**
   * Compile, sign, and broadcast one or more built instructions as a single
   * transaction.
   *
   * Accepting an array (rather than a single instruction) makes batching and
   * composition fall out for free.
   *
   * @param instructions - Instructions to include in the transaction, in order
   * @param options - Adapter-specific send/confirmation options
   * @returns The transaction signature (and slot, if known)
   */
  signAndSend(
    instructions: readonly BuiltInstruction[],
    options?: SendOptions,
    context?: WalletExecutionContext
  ): Promise<SendResult>;

  /**
   * Inspect a transaction without signing, submitting, or prompting a wallet.
   *
   * This capability is optional because not every adapter has an RPC-backed
   * unsigned simulation implementation.
   */
  inspectTransaction?(
    instructions: readonly BuiltInstruction[],
    options?: TransactionInspectionOptions,
    context?: WalletExecutionContext
  ): Promise<TransactionInspectionResult>;
}

/**
 * Wallet connection state
 */
export type WalletState = 'disconnected' | 'connecting' | 'connected' | 'error';

/**
 * Options for wallet connection
 */
export interface WalletConnectOptions {
  /** Whether to use the default wallet selection UI if multiple wallets are available */
  useDefaultSelector?: boolean;
  /** Specific wallet provider to use */
  provider?: string;
}
