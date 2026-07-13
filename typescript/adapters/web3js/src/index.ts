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
  type Signer,
} from '@solana/web3.js';
import type {
  AccountLoader,
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
}

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

/** Convert a confirmation level to a web3.js Commitment (they share names). */
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

/** Convert an Arete BuiltInstruction to a web3.js TransactionInstruction. */
export function toTransactionInstruction(ix: BuiltInstruction): TransactionInstruction {
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

/** Convert a web3.js TransactionInstruction to an Arete BuiltInstruction. */
export function fromTransactionInstruction(ix: TransactionInstruction): BuiltInstruction {
  return {
    programId: ix.programId.toBase58(),
    keys: ix.keys.map((k) => ({
      pubkey: k.pubkey.toBase58(),
      isSigner: k.isSigner,
      isWritable: k.isWritable,
    })),
    data: new Uint8Array(ix.data),
  };
}

/**
 * Create a {@link WalletAdapter} from a connection and a signer.
 */
export function createWalletAdapter(config: Web3JsAdapterConfig): WalletAdapter {
  const { connection, signer } = config;
  const configuredLocalSigners = config.additionalSigners ?? [];
  const fallbackCommitment = config.defaultCommitment ?? 'confirmed';
  const signerAddresses = [
    signer.publicKey.toBase58(),
    ...configuredLocalSigners.map(signerAddress),
  ];

  return {
    publicKey: signer.publicKey.toBase58(),
    signerAddresses: [...new Set(signerAddresses)],

    async signAndSend(
      instructions: readonly BuiltInstruction[],
      options?: SendOptions
    ): Promise<SendResult> {
      if (instructions.length === 0) {
        throw new Error('signAndSend requires at least one instruction');
      }

      const sendOptions = options as Web3JsSendOptions | undefined;
      const feePayer = sendOptions?.feePayer ?? signer;
      const requiredSignerAddresses = collectRequiredSignerAddresses(
        instructions,
        feePayer.publicKey
      );
      const primarySignerAddress = signerAddress(signer);
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

      const localSigners = [...requiredSignerAddresses]
        .filter((address) => address !== primarySignerAddress)
        .map((address) => localSignerMap.get(address)!)
        .filter((candidate, index, all) => all.indexOf(candidate) === index);

      const commitment = toCommitment(options?.confirmationLevel, fallbackCommitment);
      const { blockhash, lastValidBlockHeight } =
        await connection.getLatestBlockhash(commitment);

      const message = new TransactionMessage({
        payerKey: feePayer.publicKey,
        recentBlockhash: blockhash,
        instructions: instructions.map(toTransactionInstruction),
      }).compileToV0Message();

      const transaction = new VersionedTransaction(message);
      if (localSigners.length > 0) {
        transaction.sign(localSigners);
      }

      const signed = requiredSignerAddresses.has(primarySignerAddress)
        ? await signer.signTransaction(transaction)
        : transaction;

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
  additionalSigners?: readonly Signer[];
  defaultCommitment?: Commitment;
}): WalletAdapter {
  const { connection, keypair, additionalSigners, defaultCommitment } = config;
  return createWalletAdapter({
    connection,
    additionalSigners,
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
