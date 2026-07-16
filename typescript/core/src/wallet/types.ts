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
 * Options forwarded to the wallet adapter when sending a transaction.
 *
 * The core SDK does not interpret these; it passes them straight through to
 * the adapter, which owns all RPC semantics.
 */
export interface SendOptions {
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
export interface TransactionInspectionOptions {
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
