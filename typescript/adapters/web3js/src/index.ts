/**
 * @usearete/adapter-web3js
 *
 * A reference {@link WalletAdapter} implementation backed by @solana/web3.js.
 *
 * The Arete core SDK is RPC-free: it only builds `BuiltInstruction` objects.
 * This adapter owns everything network-related: fetching a recent blockhash,
 * compiling a v0 message, signing, sending, and confirming.
 *
 * Two construction helpers are provided:
 * - {@link createKeypairWalletAdapter} for Node scripts / bots (signs with a Keypair).
 * - {@link createWalletAdapter} for browser / wallet-standard signers.
 */

import {
  Connection,
  PublicKey,
  TransactionInstruction,
  TransactionMessage,
  VersionedTransaction,
  type Keypair,
  type Commitment,
} from '@solana/web3.js';
import type {
  WalletAdapter,
  BuiltInstruction,
  SendOptions,
  SendResult,
  ConfirmationLevel,
} from '@usearete/sdk';

/**
 * Minimal signer interface. A browser wallet, wallet-standard signer, or a
 * Keypair wrapper can all satisfy this.
 */
export interface VersionedTransactionSigner {
  publicKey: PublicKey;
  signTransaction(tx: VersionedTransaction): Promise<VersionedTransaction>;
}

export interface Web3JsAdapterConfig {
  /** A connected @solana/web3.js Connection. */
  connection: Connection;
  /** The signer that will sign compiled transactions. */
  signer: VersionedTransactionSigner;
  /** Default commitment used when the caller does not specify one. */
  defaultCommitment?: Commitment;
}

/** Convert a confirmation level to a web3.js Commitment (they share names). */
function toCommitment(
  level: ConfirmationLevel | undefined,
  fallback: Commitment
): Commitment {
  return (level as Commitment | undefined) ?? fallback;
}

/** Convert an Arete BuiltInstruction to a web3.js TransactionInstruction. */
function toTransactionInstruction(ix: BuiltInstruction): TransactionInstruction {
  return new TransactionInstruction({
    programId: new PublicKey(ix.programId),
    keys: ix.keys.map((k) => ({
      pubkey: new PublicKey(k.pubkey),
      isSigner: k.isSigner,
      isWritable: k.isWritable,
    })),
    data: Buffer.from(ix.data),
  });
}

/**
 * Create a {@link WalletAdapter} from a connection and a signer.
 */
export function createWalletAdapter(config: Web3JsAdapterConfig): WalletAdapter {
  const { connection, signer } = config;
  const fallbackCommitment = config.defaultCommitment ?? 'confirmed';

  return {
    publicKey: signer.publicKey.toBase58(),

    async signAndSend(
      instructions: BuiltInstruction[],
      options?: SendOptions
    ): Promise<SendResult> {
      if (instructions.length === 0) {
        throw new Error('signAndSend requires at least one instruction');
      }

      const commitment = toCommitment(options?.confirmationLevel, fallbackCommitment);
      const { blockhash, lastValidBlockHeight } =
        await connection.getLatestBlockhash(commitment);

      const message = new TransactionMessage({
        payerKey: signer.publicKey,
        recentBlockhash: blockhash,
        instructions: instructions.map(toTransactionInstruction),
      }).compileToV0Message();

      const transaction = new VersionedTransaction(message);
      const signed = await signer.signTransaction(transaction);

      const signature = await connection.sendRawTransaction(signed.serialize(), {
        skipPreflight: options?.skipPreflight ?? false,
        preflightCommitment: commitment,
      });

      const confirmation = await connection.confirmTransaction(
        { signature, blockhash, lastValidBlockHeight },
        commitment
      );

      if (confirmation.value.err) {
        throw confirmation.value.err;
      }

      return { signature, slot: confirmation.context.slot };
    },
  };
}

/**
 * Create a {@link WalletAdapter} that signs with a local Keypair.
 * Convenient for Node scripts, bots, and tests.
 */
export function createKeypairWalletAdapter(config: {
  connection: Connection;
  keypair: Keypair;
  defaultCommitment?: Commitment;
}): WalletAdapter {
  const { connection, keypair, defaultCommitment } = config;
  return createWalletAdapter({
    connection,
    defaultCommitment,
    signer: {
      publicKey: keypair.publicKey,
      async signTransaction(tx: VersionedTransaction): Promise<VersionedTransaction> {
        tx.sign([keypair]);
        return tx;
      },
    },
  });
}
