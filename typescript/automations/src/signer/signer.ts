import type {
  ExecutionIntentV1,
  ProtocolPreparedTransaction,
  ProtocolTransactionFormat,
} from '@usearete/sdk';

export interface SignTransactionsInput {
  /** The wallet whose signature slot to fill. */
  readonly wallet: string;
  /** What the prepare committed to. */
  readonly intent: ExecutionIntentV1;
  /** The prepared transactions, base64 wire bytes, in intent order. */
  readonly transactions: readonly ProtocolPreparedTransaction[];
}

/**
 * Signs prepared protocol transactions for a wallet the backend controls by
 * delegation (a dev keypair, an embedded-wallet provider, a KMS).
 *
 * A signer fills the wallet's signature slot of each transaction and
 * nothing else: it must not change the message, add instructions, touch
 * other signatures, or broadcast. The runtime does not trust this: it
 * re-hashes every returned message against the intent and verifies the
 * wallet's signature before anything is submitted.
 */
export interface ProtocolSigner {
  /** A short name for logs and refusals, for example `keypair` or `privy`. */
  readonly kind: string;
  /** Formats the signer can sign, in order of preference. */
  readonly supportedTransactionFormats: readonly ProtocolTransactionFormat[];
  /** Whether this signer can sign for `wallet` now. */
  canSign(wallet: string): Promise<boolean>;
  /** The signed transactions, base64 wire bytes, one per input in the same order. */
  signTransactions(input: SignTransactionsInput): Promise<string[]>;
}

/**
 * Thrown by a signer that declines to sign, for example because the wallet
 * provider's own policy refused the transaction. The job ends `refused`
 * with this rule instead of being retried.
 */
export class SignerRefusal extends Error {
  readonly rule: 'provider_policy' | 'signer_unavailable';

  constructor(message: string, rule: 'provider_policy' | 'signer_unavailable' = 'provider_policy') {
    super(message);
    this.name = 'SignerRefusal';
    this.rule = rule;
  }
}
