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
  pipe,
  addSignersToTransactionMessage,
  createTransactionMessage,
  setTransactionMessageFeePayer,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  appendTransactionMessageInstructions,
  compileTransaction,
  getBase64Decoder,
  getBase64EncodedWireTransaction,
  signTransactionMessageWithSigners,
  getSignatureFromTransaction,
  isSolanaError,
  sendAndConfirmTransactionFactory,
  SOLANA_ERROR__JSON_RPC__SERVER_ERROR_SEND_TRANSACTION_PREFLIGHT_FAILURE,
  AccountRole,
  type Rpc,
  type RpcSubscriptions,
  type SolanaRpcApi,
  type SolanaRpcSubscriptionsApi,
  type TransactionSigner,
  type IInstruction,
  type IAccountMeta,
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
  WalletExecutionContext,
} from '@usearete/sdk';
import { TransactionTransportError } from '@usearete/sdk';

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
}

export interface KitWalletAdapter extends WalletAdapter {
  readonly signerAddresses: readonly string[];
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

function resolveTransport(
  selection: AdapterTransportSelection | undefined,
  rpc: Rpc<SolanaRpcApi> | undefined,
  rpcSubscriptions: RpcSubscriptions<SolanaRpcSubscriptionsApi> | undefined,
  context: WalletExecutionContext | undefined
): { kind: 'arete'; transport: TransactionTransport } | {
  kind: 'direct'; rpc: Rpc<SolanaRpcApi>; rpcSubscriptions: RpcSubscriptions<SolanaRpcSubscriptionsApi>;
} {
  if (typeof selection === 'object') return { kind: 'arete', transport: selection };
  if (selection === 'direct') {
    if (!rpc || !rpcSubscriptions) throw new Error('Kit direct transport requires RPC and RPC subscriptions');
    return { kind: 'direct', rpc, rpcSubscriptions };
  }
  if (context?.transactionTransport) return { kind: 'arete', transport: context.transactionTransport };
  if (rpc && rpcSubscriptions) return { kind: 'direct', rpc, rpcSubscriptions };
  throw new Error('No transaction transport is available; connect through Arete or configure direct RPC');
}

async function pollAreteStatus(
  transport: TransactionTransport,
  signature: string,
  commitment: Commitment,
  lastValidBlockHeight: bigint,
  options?: KitSendOptions
): Promise<SendResult> {
  const deadline = Date.now() + (options?.confirmationTimeoutMs ?? 60_000);
  const interval = options?.statusPollIntervalMs ?? 500;
  let emptyStatusPolls = 0;
  while (Date.now() <= deadline) {
    const status = await transport.getSignatureStatus(signature, {
      commitment, searchTransactionHistory: true,
    });
    if (status?.err) {
      throw new KitTransactionExecutionError({
        status: 'chain-failed', phase: 'confirmation', signature,
        slot: status.slot === null ? undefined : Number(status.slot), cause: status.err,
      });
    }
    if (status && hasReachedCommitment(status.confirmationStatus, commitment)) {
      return { signature, slot: status.slot === null ? undefined : Number(status.slot) };
    }
    if (await transport.getBlockHeight({ commitment }) > lastValidBlockHeight) {
      throw new KitTransactionExecutionError({
        status: 'submitted-unknown', phase: 'confirmation', signature,
        slot: status?.slot === null ? undefined : Number(status?.slot),
        cause: new Error('Transaction blockhash expired before confirmation'),
      });
    }
    emptyStatusPolls = status ? 0 : emptyStatusPolls + 1;
    const delayMs = Math.min(interval * (2 ** Math.min(emptyStatusPolls, 3)), 4_000);
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

/** Convert an Arete BuiltInstruction to a kit IInstruction. */
export function toKitInstruction(ix: BuiltInstruction): IInstruction {
  const accounts: IAccountMeta[] = ix.keys.map((k) => ({
    address: address(k.pubkey),
    role: toAccountRole(k),
  }));
  return {
    programAddress: address(ix.programId),
    accounts,
    data: ix.data,
  };
}

/** Convert a kit IInstruction to an Arete BuiltInstruction. */
export function fromKitInstruction(ix: IInstruction): BuiltInstruction {
  return {
    programId: ix.programAddress,
    keys: (ix.accounts ?? []).map((account) => ({
      pubkey: account.address,
      ...fromAccountRole(account.role),
    })),
    data: ix.data ? new Uint8Array(ix.data) : new Uint8Array(0),
  };
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

    async signAndSend(
      instructions: readonly BuiltInstruction[],
      options?: SendOptions,
      context?: WalletExecutionContext
    ): Promise<SendResult> {
      if (instructions.length === 0) {
        const cause = new Error('signAndSend requires at least one instruction');
        throw new KitTransactionExecutionError({
          status: 'not-submitted',
          phase: 'build',
          cause,
        });
      }

      const sendOptions = options as KitSendOptions | undefined;
      const feePayer = sendOptions?.feePayer ?? signer;
      const commitment = toCommitment(options?.confirmationLevel, fallbackCommitment);
      let resolved: ReturnType<typeof resolveTransport>;
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
        const latestBlockhash = resolved.kind === 'arete'
          ? await resolved.transport.getLatestBlockhash({ commitment })
          : (await resolved.rpc.getLatestBlockhash({ commitment }).send()).value;
        lastValidBlockHeight = BigInt(latestBlockhash.lastValidBlockHeight);
        const messageWithFeePayer = pipe(
          createTransactionMessage({ version: 0 }),
          (m) => setTransactionMessageFeePayerSigner(feePayer, m),
          (m) => setTransactionMessageLifetimeUsingBlockhash(
            latestBlockhash as Parameters<typeof setTransactionMessageLifetimeUsingBlockhash>[0],
            m
          ),
          (m) => appendTransactionMessageInstructions(instructions.map(toKitInstruction), m)
        );
        message = addSignersToTransactionMessage(attachedSigners, messageWithFeePayer);
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
        signature = getSignatureFromTransaction(signedTransaction);
      } catch (cause) {
        throw new KitTransactionExecutionError({
          status: 'not-submitted',
          phase: 'wallet',
          cause,
        });
      }

      if (resolved.kind === 'arete') {
        let submittedSignature: string;
        try {
          const sent = await resolved.transport.sendTransaction(
            getBase64EncodedWireTransaction(signedTransaction),
            { skipPreflight: options?.skipPreflight ?? false, preflightCommitment: commitment }
          );
          submittedSignature = sent.signature;
        } catch (cause) {
          if (cause instanceof TransactionTransportError && cause.submissionState === 'not_submitted') {
            throw new KitTransactionExecutionError({ status: 'not-submitted', phase: 'send', cause });
          }
          throw new KitTransactionExecutionError({
            status: 'submitted-unknown', phase: 'send',
            signature: cause instanceof TransactionTransportError && cause.signature
              ? cause.signature : signature,
            cause,
          });
        }
        try {
          return await pollAreteStatus(
            resolved.transport, submittedSignature, commitment, lastValidBlockHeight, sendOptions
          );
        } catch (cause) {
          if (cause instanceof KitTransactionExecutionError) throw cause;
          throw new KitTransactionExecutionError({
            status: 'submitted-unknown', phase: 'confirmation',
            signature: submittedSignature, cause,
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
    ): Promise<TransactionInspectionResult> {
      if (instructions.length === 0) {
        throw new Error('inspectTransaction requires at least one instruction');
      }

      const inspectionOptions = options as KitTransactionInspectionOptions | undefined;
      const commitment = inspectionOptions?.commitment ?? fallbackCommitment;
      const minContextSlot = inspectionOptions?.minContextSlot;
      const resolved = resolveTransport(config.transport, rpc, rpcSubscriptions, context);
      const latestBlockhash = resolved.kind === 'arete'
        ? await resolved.transport.getLatestBlockhash({ commitment, minContextSlot })
        : (await resolved.rpc.getLatestBlockhash({ commitment, minContextSlot }).send()).value;
      const message = pipe(
        createTransactionMessage({ version: 0 }),
        (m) => setTransactionMessageFeePayer(
          address(inspectionOptions?.feePayer ?? signer.address),
          m
        ),
        (m) => setTransactionMessageLifetimeUsingBlockhash(
          latestBlockhash as Parameters<typeof setTransactionMessageLifetimeUsingBlockhash>[0],
          m
        ),
        (m) => appendTransactionMessageInstructions(instructions.map(toKitInstruction), m)
      );
      const unsignedTransaction = compileTransaction(message);
      const wireTransaction = getBase64EncodedWireTransaction(unsignedTransaction);
      const encodedMessage = getBase64Decoder().decode(
        unsignedTransaction.messageBytes
      ) as TransactionMessageBytesBase64;
      if (resolved.kind === 'arete') {
        const [fee, simulation] = await Promise.all([
          resolved.transport.getFeeForMessage(encodedMessage, { commitment, minContextSlot }),
          resolved.transport.simulateTransaction(wireTransaction, { commitment, minContextSlot }),
        ]);
        return {
          feeLamports: fee.feeLamports === null ? undefined : toNumber(fee.feeLamports),
          logs: simulation.logs ?? undefined,
          computeUnitsConsumed: simulation.unitsConsumed === undefined
            ? undefined : toNumber(simulation.unitsConsumed),
          contextSlot: toNumber(simulation.contextSlot),
          error: simulation.err ?? undefined,
          feeContextSlot: toNumber(fee.contextSlot),
        };
      }

      const [fee, simulation] = await Promise.all([
        resolved.rpc.getFeeForMessage(encodedMessage, { commitment, minContextSlot }).send(),
        resolved.rpc.simulateTransaction(wireTransaction, {
          commitment,
          encoding: 'base64',
          minContextSlot,
          sigVerify: false,
        }).send(),
      ]);

      return {
        feeLamports: fee.value === null ? undefined : toNumber(fee.value),
        logs: simulation.value.logs ?? undefined,
        computeUnitsConsumed: simulation.value.unitsConsumed === undefined
          ? undefined
          : toNumber(simulation.value.unitsConsumed),
        contextSlot: toNumber(simulation.context.slot),
        error: simulation.value.err ?? undefined,
        feeContextSlot: toNumber(fee.context.slot),
      };
    },
  };
}
