/**
 * @usearete/adapter-web3js
 *
 * An Arete WalletAdapter backed by @solana/web3.js. Transactions are compiled
 * as v0 messages, signed once, submitted once, and then confirmed or
 * classified without resubmission.
 */

import bs58 from 'bs58';
import {
  Connection,
  PublicKey,
  SendTransactionError,
  TransactionInstruction,
  TransactionMessage,
  VersionedTransaction,
  type Commitment,
  type Keypair,
  type SignatureStatus,
  type Signer,
  type TransactionVersion,
} from '@solana/web3.js';
import type {
  AccountLoader,
  BuiltInstruction,
  ConfirmationLevel,
  ProgramError,
  SendOptions,
  SendResult,
  TransactionFailureOutcome,
  TransactionInspectionOptions,
  TransactionInspectionResult,
  TransactionTransport,
  WalletAdapter,
  WalletExecutionContext,
  TransactionBuildCapability,
} from '@usearete/sdk';
import { TransactionTransportError, resolveTransactionBuildOptions } from '@usearete/sdk';

/**
 * web3.js 1.x compiles v0 messages and nothing newer, and this adapter adds no
 * compute-budget instructions of its own, so every resource option is rejected
 * rather than silently dropped.
 */
const CAPABILITY: TransactionBuildCapability = {
  supportedTransactionVersions: [0],
  supportedResourceOptions: [],
};

export type AdapterTransportSelection = 'auto' | 'direct' | TransactionTransport;

/**
 * The transaction-signing surface exposed by compatible browser wallets.
 *
 * This matches wallet-adapter-style signers. Raw Wallet Standard features use
 * byte-array request/response types and need an explicit bridge before they
 * satisfy this interface.
 */
export interface VersionedTransactionSigner {
  publicKey: PublicKey;
  signTransaction(tx: VersionedTransaction): Promise<VersionedTransaction>;
  /** `null` means legacy-only; omitted means the caller guarantees v0 support. */
  supportedTransactionVersions?: ReadonlySet<TransactionVersion> | null;
}

export interface Web3JsAdapterConfig {
  /** Direct Solana RPC dependency. Required by direct mode and standalone auto mode. */
  connection?: Connection;
  /** Arete when connected, direct when explicitly selected, or a custom transport. */
  transport?: AdapterTransportSelection;
  /** A signer that accepts v0 VersionedTransaction instances. */
  signer: VersionedTransactionSigner;
  /** Optional local signers that can satisfy additional required signatures. */
  additionalSigners?: readonly Signer[];
  /** Default commitment used when the caller does not specify one. */
  defaultCommitment?: Commitment;
}

export interface Web3JsSendOptions extends SendOptions {
  /** Extra local signers for this send only. */
  additionalSigners?: readonly Signer[];
  /** Override the fee payer with a local signer. */
  feePayer?: Signer;
  confirmationTimeoutMs?: number;
  statusPollIntervalMs?: number;
}

export interface Web3JsInspectionOptions extends TransactionInspectionOptions {
  /** Commitment used for blockhash, fee, and simulation RPCs. */
  commitment?: Commitment;
  /** Fee payer used to compile the unsigned v0 message. */
  feePayer?: PublicKey;
  /** Minimum slot the simulation RPC may evaluate. */
  minContextSlot?: number;
  /** Accounts to return from simulation. */
  accounts?: readonly string[];
  /** Include inner instructions in the simulation response. */
  innerInstructions?: boolean;
}

export interface Web3JsInspectionResult extends TransactionInspectionResult {
  /** Context slot returned by fee estimation, when different from simulation. */
  feeContextSlot?: number;
  /** Generic custom-program error parsed from the simulation response. */
  programError?: ProgramError;
}

export interface Web3JsWalletAdapter extends WalletAdapter {
  readonly supportedTransactionVersions: readonly [0];
  signAndSend(
    instructions: readonly BuiltInstruction[],
    options?: Web3JsSendOptions,
    context?: WalletExecutionContext
  ): Promise<SendResult>;
  inspectTransaction(
    instructions: readonly BuiltInstruction[],
    options?: Web3JsInspectionOptions,
    context?: WalletExecutionContext
  ): Promise<Web3JsInspectionResult>;
}

type AdapterTransactionError = Error & {
  readonly outcome: TransactionFailureOutcome;
  readonly cause: unknown;
  readonly signature?: string;
  readonly slot?: number;
  readonly programError?: ProgramError;
};

export function connectionAccountLoader(
  connection: Pick<Connection, 'getAccountInfo'>
): AccountLoader {
  return {
    async getAccount(address: string) {
      const account = await connection.getAccountInfo(new PublicKey(address), 'confirmed');
      return account ? { data: new Uint8Array(account.data) } : null;
    },
  };
}

function toCommitment(
  level: ConfirmationLevel | undefined,
  fallback: Commitment
): Commitment {
  return (level as Commitment | undefined) ?? fallback;
}

function signerAddress(signer: { publicKey: PublicKey }): string {
  return signer.publicKey.toBase58();
}

function collectRequiredSignerAddresses(
  instructions: readonly BuiltInstruction[],
  feePayer: PublicKey
): Set<string> {
  const required = new Set<string>([feePayer.toBase58()]);

  for (const instruction of instructions) {
    for (const key of instruction.keys) {
      if (key.isSigner) {
        required.add(key.pubkey);
      }
    }
  }

  return required;
}

function indexLocalSigners(signers: readonly Signer[]): Map<string, Signer> {
  const indexed = new Map<string, Signer>();
  for (const signer of signers) {
    indexed.set(signerAddress(signer), signer);
  }
  return indexed;
}

function outcomeMessage(outcome: TransactionFailureOutcome): string {
  if (outcome.status === 'not-submitted') {
    return `Transaction was not submitted during ${outcome.phase}`;
  }
  if (outcome.status === 'submitted-unknown') {
    return `Transaction ${outcome.signature} was submitted but its status is unknown`;
  }
  return outcome.signature
    ? `Transaction ${outcome.signature} failed on chain`
    : 'Transaction failed on chain';
}

function transactionError(
  outcome: TransactionFailureOutcome,
  programError?: ProgramError
): AdapterTransactionError {
  const causeMessage = outcome.cause instanceof Error ? outcome.cause.message : undefined;
  const error = new Error(causeMessage || outcomeMessage(outcome)) as AdapterTransactionError;
  Object.defineProperties(error, {
    name: { value: 'TransactionExecutionError', configurable: true },
    cause: { value: outcome.cause, enumerable: true },
    outcome: { value: outcome, enumerable: true },
    signature: {
      value: 'signature' in outcome ? outcome.signature : undefined,
      enumerable: true,
    },
    slot: {
      value: 'slot' in outcome ? outcome.slot : undefined,
      enumerable: true,
    },
    programError: { value: programError, enumerable: true },
  });
  return error;
}

function customErrorCode(value: unknown, seen = new Set<object>()): number | undefined {
  if (typeof value === 'string') {
    const match = /custom program error:\s*0x([\da-f]+)/i.exec(value);
    return match?.[1] ? Number.parseInt(match[1], 16) : undefined;
  }
  if (typeof value !== 'object' || value === null || seen.has(value)) {
    return undefined;
  }
  seen.add(value);
  if (Array.isArray(value)) {
    for (const item of value) {
      const code = customErrorCode(item, seen);
      if (code !== undefined) {
        return code;
      }
    }
    return undefined;
  }
  const candidate = value as Record<string, unknown>;
  if (Array.isArray(candidate.InstructionError)) {
    const detail = candidate.InstructionError[1];
    if (typeof detail === 'object' && detail !== null) {
      const code = (detail as { Custom?: unknown }).Custom;
      if (typeof code === 'number') {
        return code;
      }
    }
  }
  if (typeof candidate.code === 'number') {
    return candidate.code;
  }
  for (const key of [
    'cause',
    'error',
    'err',
    'value',
    'data',
    'message',
    'transactionMessage',
    'transactionLogs',
    'logs',
  ]) {
    const code = customErrorCode(candidate[key], seen);
    if (code !== undefined) {
      return code;
    }
  }
  return undefined;
}

function parseProgramError(error: unknown): ProgramError | undefined {
  const code = customErrorCode(error);
  return code === undefined
    ? undefined
    : {
        code,
        name: `CustomError${code}`,
        message: `Unknown error with code ${code}`,
      };
}

function signatureFromTransaction(transaction: VersionedTransaction): string | undefined {
  const signature = transaction.signatures[0];
  if (!signature || signature.every((byte) => byte === 0)) {
    return undefined;
  }
  return bs58.encode(signature);
}

function hasAllRequiredSignatures(transaction: VersionedTransaction): boolean {
  const required = transaction.message.header.numRequiredSignatures;
  return transaction.signatures
    .slice(0, required)
    .every((signature) => signature.some((byte) => byte !== 0));
}

function commitmentRank(commitment: Commitment): number {
  if (commitment === 'finalized' || commitment === 'max' || commitment === 'root') {
    return 2;
  }
  if (
    commitment === 'confirmed'
    || commitment === 'single'
    || commitment === 'singleGossip'
  ) {
    return 1;
  }
  return 0;
}

function statusRank(status: SignatureStatus): number {
  if (status.confirmationStatus === 'finalized' || status.confirmations === null) {
    return 2;
  }
  if (status.confirmationStatus === 'confirmed' || (status.confirmations ?? 0) > 0) {
    return 1;
  }
  return 0;
}

function encodeBase64(bytes: Uint8Array): string {
  const bufferCtor = (globalThis as { Buffer?: typeof Buffer }).Buffer;
  if (bufferCtor) return bufferCtor.from(bytes).toString('base64');
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function resolveTransport(
  selection: AdapterTransportSelection | undefined,
  connection: Connection | undefined,
  context: WalletExecutionContext | undefined
): { kind: 'arete'; transport: TransactionTransport } | { kind: 'direct'; connection: Connection } {
  if (typeof selection === 'object') return { kind: 'arete', transport: selection };
  if (selection === 'direct') {
    if (!connection) throw new Error('Web3.js direct transaction transport requires a Connection');
    return { kind: 'direct', connection };
  }
  if (context?.transactionTransport) return { kind: 'arete', transport: context.transactionTransport };
  if (connection) return { kind: 'direct', connection };
  throw new Error('No transaction transport is available; connect through Arete or configure direct RPC');
}

async function pollAreteStatus(
  transport: TransactionTransport,
  signature: string,
  commitment: Commitment,
  lastValidBlockHeight: bigint,
  options?: Web3JsSendOptions
): Promise<SendResult> {
  const timeoutMs = options?.confirmationTimeoutMs ?? 60_000;
  const intervalMs = options?.statusPollIntervalMs ?? 500;
  const deadline = Date.now() + timeoutMs;
  let emptyStatusPolls = 0;
  while (Date.now() <= deadline) {
    const status = await transport.getSignatureStatus(signature, {
      commitment: commitment as 'processed' | 'confirmed' | 'finalized',
      searchTransactionHistory: true,
    });
    if (status?.err) {
      throw transactionError({
        status: 'chain-failed', phase: 'chain', signature,
        slot: status.slot === null ? undefined : Number(status.slot),
        programError: parseProgramError(status.err), cause: status.err,
      });
    }
    if (status && commitmentRank(status.confirmationStatus as Commitment) >= commitmentRank(commitment)) {
      return { signature, slot: status.slot === null ? undefined : Number(status.slot) };
    }
    if (await transport.getBlockHeight({ commitment: commitment as 'processed' | 'confirmed' | 'finalized' }) > lastValidBlockHeight) {
      throw transactionError({
        status: 'submitted-unknown', phase: 'confirmation', signature,
        slot: status?.slot === null ? undefined : Number(status?.slot),
        cause: new Error('Transaction blockhash expired before confirmation'),
      });
    }
    emptyStatusPolls = status ? 0 : emptyStatusPolls + 1;
    const delayMs = Math.min(intervalMs * (2 ** Math.min(emptyStatusPolls, 3)), 4_000);
    await new Promise((resolve) => setTimeout(resolve, delayMs));
  }
  throw transactionError({
    status: 'submitted-unknown', phase: 'confirmation', signature,
    cause: new Error('Transaction confirmation timed out'),
  });
}

async function resolveUncertainSubmission(
  connection: Pick<Connection, 'getSignatureStatuses'>,
  signature: string,
  commitment: Commitment,
  phase: 'send' | 'confirmation',
  cause: unknown
): Promise<SendResult> {
  let status: SignatureStatus | null = null;
  try {
    const response = await connection.getSignatureStatuses(
      [signature],
      { searchTransactionHistory: true }
    );
    status = response.value[0] ?? null;
  } catch {
    // The original submission/confirmation error remains the useful cause.
  }

  if (status?.err) {
    throw transactionError({
      status: 'chain-failed',
      phase: 'chain',
      signature,
      slot: status.slot,
      programError: parseProgramError(status.err),
      cause,
    });
  }
  if (status && statusRank(status) >= commitmentRank(commitment)) {
    return { signature, slot: status.slot };
  }
  throw transactionError({
    status: 'submitted-unknown',
    phase,
    signature,
    slot: status?.slot,
    cause,
  });
}

/** Convert an Arete BuiltInstruction to a web3.js TransactionInstruction. */
export function toTransactionInstruction(ix: BuiltInstruction): TransactionInstruction {
  return new TransactionInstruction({
    programId: new PublicKey(ix.programId),
    keys: ix.keys.map((key) => ({
      pubkey: new PublicKey(key.pubkey),
      isSigner: key.isSigner,
      isWritable: key.isWritable,
    })),
    // web3.js types this field as Buffer, but its transaction compiler consumes
    // Uint8Array. Avoid importing the Node `buffer` builtin into browser apps.
    data: Uint8Array.from(ix.data) as TransactionInstruction['data'],
  });
}

/** Convert a web3.js TransactionInstruction to an Arete BuiltInstruction. */
export function fromTransactionInstruction(ix: TransactionInstruction): BuiltInstruction {
  return {
    programId: ix.programId.toBase58(),
    keys: ix.keys.map((key) => ({
      pubkey: key.pubkey.toBase58(),
      isSigner: key.isSigner,
      isWritable: key.isWritable,
    })),
    data: new Uint8Array(ix.data),
  };
}

/** Create a WalletAdapter from a connection and a v0-capable signer. */
export function createWalletAdapter(config: Web3JsAdapterConfig): Web3JsWalletAdapter {
  const { signer } = config;
  const configuredLocalSigners = config.additionalSigners ?? [];
  const fallbackCommitment = config.defaultCommitment ?? 'confirmed';
  const signerAddresses = [
    signer.publicKey.toBase58(),
    ...configuredLocalSigners.map(signerAddress),
  ];

  return {
    publicKey: signer.publicKey.toBase58(),
    signerAddresses: [...new Set(signerAddresses)],
    supportedTransactionVersions: CAPABILITY.supportedTransactionVersions as readonly [0],

    async inspectTransaction(
      instructions: readonly BuiltInstruction[],
      options?: Web3JsInspectionOptions,
      context?: WalletExecutionContext
    ): Promise<Web3JsInspectionResult> {
      resolveTransactionBuildOptions(options, CAPABILITY);
      if (instructions.length === 0) {
        throw new Error('inspectTransaction requires at least one instruction');
      }
      const commitment = options?.commitment ?? fallbackCommitment;
      const feePayer = options?.feePayer ?? signer.publicKey;
      const resolved = resolveTransport(config.transport, config.connection, context);
      if (resolved.kind === 'arete') {
        const latest = await resolved.transport.getLatestBlockhash({
          commitment: commitment as 'processed' | 'confirmed' | 'finalized',
          minContextSlot: options?.minContextSlot === undefined ? undefined : BigInt(options.minContextSlot),
        });
        const message = new TransactionMessage({
          payerKey: feePayer,
          recentBlockhash: latest.blockhash,
          instructions: instructions.map(toTransactionInstruction),
        }).compileToV0Message();
        const transaction = new VersionedTransaction(message);
        const [fee, simulation] = await Promise.all([
          resolved.transport.getFeeForMessage(encodeBase64(message.serialize()), {
            commitment: commitment as 'processed' | 'confirmed' | 'finalized',
            minContextSlot: options?.minContextSlot === undefined ? undefined : BigInt(options.minContextSlot),
          }),
          resolved.transport.simulateTransaction(encodeBase64(transaction.serialize()), {
            commitment: commitment as 'processed' | 'confirmed' | 'finalized',
            minContextSlot: options?.minContextSlot === undefined ? undefined : BigInt(options.minContextSlot),
            accounts: options?.accounts,
            innerInstructions: options?.innerInstructions,
          }),
        ]);
        return {
          feeLamports: fee.feeLamports === null ? undefined : Number(fee.feeLamports),
          feeContextSlot: Number(fee.contextSlot), logs: simulation.logs ?? undefined,
          computeUnitsConsumed: simulation.unitsConsumed === undefined ? undefined : Number(simulation.unitsConsumed),
          contextSlot: Number(simulation.contextSlot), error: simulation.err ?? undefined,
          loadedAccountsDataSize: simulation.loadedAccountsDataSize === undefined
            ? undefined : Number(simulation.loadedAccountsDataSize),
          programError: parseProgramError(simulation.err),
        };
      }
      const connection = resolved.connection;
      const { blockhash } = await connection.getLatestBlockhash(commitment);
      const message = new TransactionMessage({
        payerKey: feePayer,
        recentBlockhash: blockhash,
        instructions: instructions.map(toTransactionInstruction),
      }).compileToV0Message();
      const transaction = new VersionedTransaction(message);
      const [fee, simulation] = await Promise.all([
        connection.getFeeForMessage(message, commitment),
        connection.simulateTransaction(transaction, {
          commitment,
          sigVerify: false,
          replaceRecentBlockhash: false,
          minContextSlot: options?.minContextSlot,
          innerInstructions: options?.innerInstructions,
          accounts: options?.accounts
            ? { encoding: 'base64', addresses: [...options.accounts] }
            : undefined,
        }),
      ]);
      // The RPC reports the loaded-accounts budget that web3.js 1.x does not type.
      const { loadedAccountsDataSize } = simulation.value as { loadedAccountsDataSize?: number };

      return {
        feeLamports: fee.value ?? undefined,
        feeContextSlot: fee.context.slot,
        logs: simulation.value.logs ?? undefined,
        computeUnitsConsumed: simulation.value.unitsConsumed,
        contextSlot: simulation.context.slot,
        loadedAccountsDataSize,
        error: simulation.value.err ?? undefined,
        programError: parseProgramError(simulation.value.err),
      };
    },

    async signAndSend(
      instructions: readonly BuiltInstruction[],
      options?: Web3JsSendOptions,
      context?: WalletExecutionContext
    ): Promise<SendResult> {
      // web3.js 1.x can only build v0: an explicit legacy/V1 request or any
      // resource option is rejected before the wallet is prompted.
      resolveTransactionBuildOptions(options, CAPABILITY);
      const sendOptions = options;
      let transaction: VersionedTransaction;
      let blockhash: string;
      let lastValidBlockHeight: number;
      let requiredSignerAddresses: Set<string>;
      let localSigners: Signer[];
      const commitment = toCommitment(options?.confirmationLevel, fallbackCommitment);
      let resolved: ReturnType<typeof resolveTransport>;
      try {
        resolved = resolveTransport(config.transport, config.connection, context);
      } catch (cause) {
        throw transactionError({ status: 'not-submitted', phase: 'build', cause });
      }
      const feePayer = sendOptions?.feePayer ?? signer;
      const primarySignerAddress = signerAddress(signer);

      try {
        if (instructions.length === 0) {
          throw new Error('signAndSend requires at least one instruction');
        }
        requiredSignerAddresses = collectRequiredSignerAddresses(instructions, feePayer.publicKey);
        const localSignerMap = indexLocalSigners([
          ...configuredLocalSigners,
          ...((sendOptions?.signers ?? []) as readonly Signer[]),
          ...(sendOptions?.additionalSigners ?? []),
          ...(sendOptions?.feePayer ? [sendOptions.feePayer] : []),
        ]);
        const missingSignerAddresses = [...requiredSignerAddresses].filter(
          (address) => address !== primarySignerAddress && !localSignerMap.has(address)
        );
        if (missingSignerAddresses.length > 0) {
          throw new Error(
            `Missing signer(s) for transaction: ${missingSignerAddresses.join(', ')}`
          );
        }
        localSigners = [...requiredSignerAddresses]
          .filter((address) => address !== primarySignerAddress)
          .map((address) => localSignerMap.get(address)!)
          .filter((candidate, index, all) => all.indexOf(candidate) === index);

        const latestBlockhash = resolved.kind === 'arete'
          ? await resolved.transport.getLatestBlockhash({ commitment: commitment as 'processed' | 'confirmed' | 'finalized' })
          : await resolved.connection.getLatestBlockhash(commitment);
        blockhash = latestBlockhash.blockhash;
        lastValidBlockHeight = Number(latestBlockhash.lastValidBlockHeight);
        const message = new TransactionMessage({
          payerKey: feePayer.publicKey,
          recentBlockhash: blockhash,
          instructions: instructions.map(toTransactionInstruction),
        }).compileToV0Message();
        transaction = new VersionedTransaction(message);
      } catch (cause) {
        throw transactionError({ status: 'not-submitted', phase: 'build', cause });
      }

      try {
        if (localSigners.length > 0) {
          transaction.sign(localSigners);
        }
        if (requiredSignerAddresses.has(primarySignerAddress)) {
          const supportedVersions = signer.supportedTransactionVersions;
          if (supportedVersions === null || (supportedVersions && !supportedVersions.has(0))) {
            throw new Error(
              'The configured wallet does not support v0 VersionedTransaction signing'
            );
          }
          const signed = await signer.signTransaction(transaction);
          if (!signed || typeof signed.serialize !== 'function') {
            throw new Error(
              'The configured wallet did not return a signed v0 VersionedTransaction'
            );
          }
          // Wallet adapters can use a different web3.js module instance, making
          // instanceof checks reject otherwise valid signed transactions.
          transaction = VersionedTransaction.deserialize(signed.serialize());
        }
        if (!hasAllRequiredSignatures(transaction)) {
          throw new Error('The configured signers did not provide every required signature');
        }
      } catch (cause) {
        throw transactionError({ status: 'not-submitted', phase: 'wallet', cause });
      }

      const localSignature = signatureFromTransaction(transaction)!;
      if (resolved.kind === 'arete') {
        let signature: string;
        try {
          const sent = await resolved.transport.sendTransaction(
            encodeBase64(transaction.serialize()),
            {
              skipPreflight: options?.skipPreflight ?? false,
              preflightCommitment: commitment as 'processed' | 'confirmed' | 'finalized',
            }
          );
          signature = sent.signature;
        } catch (cause) {
          if (cause instanceof TransactionTransportError && cause.submissionState === 'not_submitted') {
            throw transactionError({ status: 'not-submitted', phase: 'send', cause });
          }
          throw transactionError({
            status: 'submitted-unknown', phase: 'send',
            signature: cause instanceof TransactionTransportError && cause.signature
              ? cause.signature : localSignature,
            cause,
          });
        }
        try {
          return await pollAreteStatus(
            resolved.transport, signature, commitment, BigInt(lastValidBlockHeight), options
          );
        } catch (cause) {
          if (typeof cause === 'object' && cause !== null && 'outcome' in cause) throw cause;
          throw transactionError({
            status: 'submitted-unknown', phase: 'confirmation', signature, cause,
          });
        }
      }
      const connection = resolved.connection;
      let signature: string;
      try {
        signature = await connection.sendRawTransaction(transaction.serialize(), {
          skipPreflight: options?.skipPreflight ?? false,
          preflightCommitment: commitment,
        });
      } catch (cause) {
        if (cause instanceof SendTransactionError) {
          throw transactionError(
            { status: 'not-submitted', phase: 'send', cause },
            parseProgramError(cause)
          );
        }
        return resolveUncertainSubmission(
          connection,
          localSignature,
          commitment,
          'send',
          cause
        );
      }

      try {
        const confirmation = await connection.confirmTransaction(
          { signature, blockhash, lastValidBlockHeight },
          commitment
        );
        if (confirmation.value.err) {
          throw transactionError({
            status: 'chain-failed',
            phase: 'chain',
            signature,
            slot: confirmation.context.slot,
            programError: parseProgramError(confirmation.value.err),
            cause: confirmation.value.err,
          });
        }
        return { signature, slot: confirmation.context.slot };
      } catch (cause) {
        if (
          typeof cause === 'object'
          && cause !== null
          && 'outcome' in cause
          && (cause as { outcome?: { status?: unknown } }).outcome?.status === 'chain-failed'
        ) {
          throw cause;
        }
        return resolveUncertainSubmission(
          connection,
          signature,
          commitment,
          'confirmation',
          cause
        );
      }
    },
  };
}

/** Create a WalletAdapter backed by a local Keypair. */
export function createKeypairWalletAdapter(config: {
  connection?: Connection;
  transport?: AdapterTransportSelection;
  keypair: Keypair;
  additionalSigners?: readonly Signer[];
  defaultCommitment?: Commitment;
}): Web3JsWalletAdapter {
  const { connection, transport, keypair, additionalSigners, defaultCommitment } = config;
  return createWalletAdapter({
    connection,
    transport,
    additionalSigners,
    defaultCommitment,
    signer: {
      publicKey: keypair.publicKey,
      supportedTransactionVersions: new Set<TransactionVersion>([0]),
      async signTransaction(tx: VersionedTransaction): Promise<VersionedTransaction> {
        tx.sign([keypair]);
        return tx;
      },
    },
  });
}
