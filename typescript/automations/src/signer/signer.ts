import type {
  ExecutionIntentV1,
  ProtocolPreparedTransaction,
  ProtocolTransactionFormat,
} from '@usearete/sdk';

/**
 * What a signer is given. Hand it copies and keep your own snapshot of the
 * prepared transactions for `checkSignedTransactions`: `readonly` is not
 * enforced at run time, and a signer that changed the objects it was given
 * could change a shared baseline too. The runtime does this.
 */
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
const REFUSAL_BRAND = Symbol.for('@usearete/automations/SignerRefusal');

export class SignerRefusal extends Error {
  readonly rule: 'provider_policy' | 'signer_unavailable';
  /** Recognised across entry points: each bundle has its own copy of this class. */
  readonly [REFUSAL_BRAND] = true;

  constructor(message: string, rule: 'provider_policy' | 'signer_unavailable' = 'provider_policy') {
    super(message);
    this.name = 'SignerRefusal';
    this.rule = rule;
  }
}

/**
 * Whether `error` is a {@link SignerRefusal}, including one thrown by a
 * signer from another entry point (`@usearete/automations/privy` is a
 * separate bundle, so `instanceof` would miss it).
 */
export function isSignerRefusal(error: unknown): error is SignerRefusal {
  return (
    error instanceof SignerRefusal ||
    (typeof error === 'object' && error !== null && (error as Record<symbol, unknown>)[REFUSAL_BRAND] === true)
  );
}
