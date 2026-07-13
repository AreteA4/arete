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
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  appendTransactionMessageInstructions,
  signTransactionMessageWithSigners,
  getSignatureFromTransaction,
  sendAndConfirmTransactionFactory,
  AccountRole,
  type Rpc,
  type RpcSubscriptions,
  type SolanaRpcApi,
  type SolanaRpcSubscriptionsApi,
  type TransactionSigner,
  type IInstruction,
  type IAccountMeta,
  type Commitment,
} from '@solana/kit';
import type {
  WalletAdapter,
  BuiltInstruction,
  BuiltAccountMeta,
  SendOptions,
  SendResult,
  ConfirmationLevel,
} from '@usearete/sdk';

export interface KitAdapterConfig {
  /** A Solana RPC client (from `createSolanaRpc`). */
  rpc: Rpc<SolanaRpcApi>;
  /** A Solana RPC subscriptions client (from `createSolanaRpcSubscriptions`). */
  rpcSubscriptions: RpcSubscriptions<SolanaRpcSubscriptionsApi>;
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
export function createWalletAdapter(config: KitAdapterConfig): WalletAdapter {
  const { rpc, rpcSubscriptions, signer } = config;
  const configuredLocalSigners = config.additionalSigners ?? [];
  const fallbackCommitment = config.defaultCommitment ?? 'confirmed';
  const sendAndConfirm = sendAndConfirmTransactionFactory({ rpc, rpcSubscriptions });

  return {
    publicKey: signer.address,

    async signAndSend(
      instructions: readonly BuiltInstruction[],
      options?: SendOptions
    ): Promise<SendResult> {
      if (instructions.length === 0) {
        throw new Error('signAndSend requires at least one instruction');
      }

      const sendOptions = options as KitSendOptions | undefined;
      const feePayer = sendOptions?.feePayer ?? signer;
      const requiredSignerAddresses = collectRequiredSignerAddresses(
        instructions,
        feePayer.address
      );
      const primarySignerAddress = signer.address;
      const localSignerMap = indexLocalSigners([
        ...configuredLocalSigners,
        ...((sendOptions?.signers ?? []) as readonly TransactionSigner[]),
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

      const localSigners = [...requiredSignerAddresses]
        .filter((address) => address !== primarySignerAddress)
        .map((address) => localSignerMap.get(address)!)
        .filter((candidate, index, all) => all.indexOf(candidate) === index);

      const commitment = toCommitment(options?.confirmationLevel, fallbackCommitment);
      const { value: latestBlockhash } = await rpc
        .getLatestBlockhash({ commitment })
        .send();

      const messageWithFeePayer = pipe(
        createTransactionMessage({ version: 0 }),
        (m) => setTransactionMessageFeePayerSigner(feePayer, m),
        (m) => setTransactionMessageLifetimeUsingBlockhash(latestBlockhash, m),
        (m) => appendTransactionMessageInstructions(instructions.map(toKitInstruction), m)
      );

      const message = addSignersToTransactionMessage(localSigners, messageWithFeePayer);

      const signedTransaction = await signTransactionMessageWithSigners(message);

      await sendAndConfirm(signedTransaction, {
        commitment,
        skipPreflight: options?.skipPreflight ?? false,
      });

      const signature = getSignatureFromTransaction(signedTransaction);
      return { signature };
    },
  };
}
