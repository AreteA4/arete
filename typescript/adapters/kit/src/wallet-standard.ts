/**
 * Wallet Standard → kit signer bridge.
 *
 * The handoff is byte-oriented on purpose: the app picks the transaction
 * format, the wallet decodes the version out of the bytes it is given, and
 * Arete forwards whatever came back without converting versions. Nothing
 * here routes a transaction through a web3.js 1.x wrapper, which cannot
 * represent V1 at all.
 *
 * The capability check is a local fact about the wallet, not proof the
 * cluster has activated a version: the account must advertise
 * `solana:signTransaction`, and *that* feature must advertise the version
 * being signed. Sign-and-send support is never read as sign-only support —
 * a wallet that can only sign-and-send cannot hand bytes back for Arete to
 * relay.
 *
 * As of Kit 8, the upstream Wallet Standard `signTransaction` feature still
 * types `supportedTransactionVersions` as `legacy | 0`. The interfaces below
 * are therefore declared structurally rather than imported, so a wallet that
 * does advertise `1` is usable without a cast and one that does not is
 * refused before it is ever prompted.
 */

import {
  getTransactionDecoder,
  getTransactionEncoder,
  getTransactionVersionDecoder,
  type Address,
  type SignatureBytes,
  type Transaction,
  type TransactionSigner,
} from '@solana/kit';

/** The feature name a sign-only handoff requires. */
export const SOLANA_SIGN_TRANSACTION = 'solana:signTransaction';

/** The feature name that is explicitly *not* evidence of sign-only support. */
export const SOLANA_SIGN_AND_SEND_TRANSACTION = 'solana:signAndSendTransaction';

export type WalletStandardTransactionVersion = 'legacy' | 0 | 1;

export interface WalletStandardAccount {
  readonly address: string;
  readonly features: readonly string[];
}

export interface WalletStandardSignTransactionInput {
  readonly account: WalletStandardAccount;
  readonly transaction: Uint8Array;
  readonly chain?: string;
}

export interface WalletStandardSignTransactionOutput {
  readonly signedTransaction: Uint8Array;
}

export interface WalletStandardSignTransactionFeature {
  readonly version: string;
  readonly supportedTransactionVersions: readonly WalletStandardTransactionVersion[];
  signTransaction(
    ...inputs: readonly WalletStandardSignTransactionInput[]
  ): Promise<readonly WalletStandardSignTransactionOutput[]>;
}

export interface WalletStandardWallet {
  readonly features: Readonly<Record<string, unknown>>;
}

export type WalletStandardSignerErrorCode =
  | 'sign_transaction_unsupported'
  | 'transaction_version_unsupported'
  | 'wallet_returned_invalid_transaction';

/** Typed refusal from the Wallet Standard bridge. */
export class WalletStandardSignerError extends Error {
  readonly code: WalletStandardSignerErrorCode;

  constructor(code: WalletStandardSignerErrorCode, message: string) {
    super(message);
    this.name = 'WalletStandardSignerError';
    this.code = code;
  }
}

export interface WalletStandardSignerConfig {
  readonly wallet: WalletStandardWallet;
  readonly account: WalletStandardAccount;
  /** CAIP-2 chain forwarded to the wallet, e.g. `solana:devnet`. */
  readonly chain?: string;
}

function readSignTransactionFeature(
  config: WalletStandardSignerConfig
): WalletStandardSignTransactionFeature {
  if (!config.account.features.includes(SOLANA_SIGN_TRANSACTION)) {
    const signAndSendOnly = config.account.features.includes(SOLANA_SIGN_AND_SEND_TRANSACTION);
    throw new WalletStandardSignerError(
      'sign_transaction_unsupported',
      `Account ${config.account.address} does not support '${SOLANA_SIGN_TRANSACTION}'`
      + (signAndSendOnly
        ? `; it advertises only '${SOLANA_SIGN_AND_SEND_TRANSACTION}', which cannot return `
          + 'signed bytes for Arete to relay'
        : '')
    );
  }
  const feature = config.wallet.features[SOLANA_SIGN_TRANSACTION] as
    | WalletStandardSignTransactionFeature
    | undefined;
  if (!feature || typeof feature.signTransaction !== 'function') {
    throw new WalletStandardSignerError(
      'sign_transaction_unsupported',
      `Wallet does not implement the '${SOLANA_SIGN_TRANSACTION}' feature`
    );
  }
  return feature;
}

/**
 * A kit {@link TransactionSigner} backed by a Wallet Standard account.
 *
 * The feature is resolved once, at creation, so a wallet that cannot sign at
 * all fails before any transaction is built. The per-version check happens
 * per transaction, against the version decoded from the bytes about to be
 * handed over — so a v0-only wallet refuses a V1 transaction without ever
 * showing the user a prompt.
 */
export function createWalletStandardSigner(
  config: WalletStandardSignerConfig
): TransactionSigner {
  const feature = readSignTransactionFeature(config);
  const address = config.account.address as Address;

  return {
    address,
    async signTransactions(transactions: readonly Transaction[]) {
      if (transactions.length === 0) return [];

      const inputs = transactions.map((transaction) => {
        const version = getTransactionVersionDecoder().decode(transaction.messageBytes);
        if (!feature.supportedTransactionVersions.includes(version)) {
          throw new WalletStandardSignerError(
            'transaction_version_unsupported',
            `Wallet does not support signing transaction version ${JSON.stringify(version)} `
            + `(supported: ${feature.supportedTransactionVersions
              .map((supported) => JSON.stringify(supported))
              .join(', ')})`
          );
        }
        return {
          account: config.account,
          chain: config.chain,
          transaction: getTransactionEncoder().encode(transaction) as Uint8Array,
        };
      });

      const outputs = await feature.signTransaction(...inputs);
      if (outputs.length !== transactions.length) {
        throw new WalletStandardSignerError(
          'wallet_returned_invalid_transaction',
          `Wallet returned ${outputs.length} signed transactions for ${transactions.length} `
          + 'inputs'
        );
      }

      return outputs.map((output, index) => {
        const returned = getTransactionDecoder().decode(output.signedTransaction);
        // A wallet may add signatures; it may not rewrite the message the
        // app compiled. Anything else and the bytes Arete would relay are
        // not the bytes the caller approved.
        const original = transactions[index].messageBytes;
        if (
          returned.messageBytes.length !== original.length
          || returned.messageBytes.some((byte, offset) => byte !== original[offset])
        ) {
          throw new WalletStandardSignerError(
            'wallet_returned_invalid_transaction',
            'Wallet returned a transaction whose message differs from the one it was given'
          );
        }
        const signature = returned.signatures[address];
        if (!signature) {
          throw new WalletStandardSignerError(
            'wallet_returned_invalid_transaction',
            `Wallet returned no signature for ${address}`
          );
        }
        return { [address]: signature } as Readonly<Record<Address, SignatureBytes>>;
      });
    },
  } as TransactionSigner;
}
