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
  /** Default commitment used when the caller does not specify one. */
  defaultCommitment?: Commitment;
}

function toCommitment(
  level: ConfirmationLevel | undefined,
  fallback: Commitment
): Commitment {
  return (level as Commitment | undefined) ?? fallback;
}

/** Map an Arete account meta to a kit AccountRole. */
function toAccountRole(meta: BuiltAccountMeta): AccountRole {
  if (meta.isSigner && meta.isWritable) return AccountRole.WRITABLE_SIGNER;
  if (meta.isSigner && !meta.isWritable) return AccountRole.READONLY_SIGNER;
  if (!meta.isSigner && meta.isWritable) return AccountRole.WRITABLE;
  return AccountRole.READONLY;
}

/** Convert an Arete BuiltInstruction to a kit IInstruction. */
function toKitInstruction(ix: BuiltInstruction): IInstruction {
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

/**
 * Create a {@link WalletAdapter} from a kit RPC pair and a signer.
 */
export function createWalletAdapter(config: KitAdapterConfig): WalletAdapter {
  const { rpc, rpcSubscriptions, signer } = config;
  const fallbackCommitment = config.defaultCommitment ?? 'confirmed';
  const sendAndConfirm = sendAndConfirmTransactionFactory({ rpc, rpcSubscriptions });

  return {
    publicKey: signer.address,

    async signAndSend(
      instructions: BuiltInstruction[],
      options?: SendOptions
    ): Promise<SendResult> {
      if (instructions.length === 0) {
        throw new Error('signAndSend requires at least one instruction');
      }

      const commitment = toCommitment(options?.confirmationLevel, fallbackCommitment);
      const { value: latestBlockhash } = await rpc
        .getLatestBlockhash({ commitment })
        .send();

      const message = pipe(
        createTransactionMessage({ version: 0 }),
        (m) => setTransactionMessageFeePayerSigner(signer, m),
        (m) => setTransactionMessageLifetimeUsingBlockhash(latestBlockhash, m),
        (m) => appendTransactionMessageInstructions(instructions.map(toKitInstruction), m)
      );

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
