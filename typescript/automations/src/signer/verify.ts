/**
 * The exact-bytes checks around a delegated signer.
 *
 * Before signing, the prepared transactions must be the ones the intent
 * committed to. After signing, every message must hash to the intent's
 * `messageSha256`, every signature slot but the wallet's must be untouched,
 * and the wallet's slot must hold the wallet's valid signature. A signer
 * that adds a compute-budget instruction, re-encodes, or signs something
 * else fails here, before anything is submitted.
 */

import { ed25519 } from '@noble/curves/ed25519';
import { bytesToHex } from '@noble/hashes/utils';
import { sha256 } from '@noble/hashes/sha2';
import type {
  ExecutionIntentV1,
  ProtocolPreparedTransaction,
  ProtocolTransactionFormat,
} from '@usearete/sdk';

import { type DecodedTransaction, decodeTransaction } from '../policy/decode';
import { decodeBase58, decodeBase64 } from '../policy/encoding';
import type { Refusal } from '../policy/policy';

function messageSha256(bytes: Uint8Array, decoded: DecodedTransaction): string {
  return bytesToHex(sha256(bytes.subarray(decoded.message.start, decoded.message.end)));
}

function decoded(base64: string): { bytes: Uint8Array; decoded: DecodedTransaction } | null {
  const bytes = decodeBase64(base64);
  if (bytes === null) return null;
  try {
    return { bytes, decoded: decodeTransaction(bytes) };
  } catch {
    return null;
  }
}

function signerSlot(transaction: DecodedTransaction, wallet: string): number {
  const required = Math.min(transaction.requiredSignatures, transaction.signatures.length);
  return transaction.staticKeys.slice(0, required).indexOf(wallet);
}

function sameBytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((byte, index) => byte === right[index]);
}

/**
 * Whether the prepared transactions are what `intent` committed to, in the
 * format that was asked for, with `wallet` as a required signer of each.
 * `null` when they are.
 */
export function checkPreparedAgainstIntent(input: {
  readonly wallet: string;
  readonly intent: ExecutionIntentV1;
  readonly transactions: readonly ProtocolPreparedTransaction[];
  readonly format: ProtocolTransactionFormat;
}): Refusal | null {
  const { wallet, intent, transactions, format } = input;
  if (intent.wallet !== wallet) {
    return { rule: 'intent_mismatch', reason: `the intent is for wallet ${intent.wallet}, not ${wallet}` };
  }
  if (intent.transactionFormat !== format) {
    return {
      rule: 'transaction_format',
      reason: `the intent was prepared as ${intent.transactionFormat}, not the negotiated ${format}`,
    };
  }
  if (transactions.length !== intent.transactions.length) {
    return {
      rule: 'intent_mismatch',
      reason: `the prepare returned ${transactions.length} transactions for an intent of ${intent.transactions.length}`,
    };
  }
  for (const [position, transaction] of transactions.entries()) {
    const committed = intent.transactions[position]!;
    if (transaction.index !== committed.index) {
      return {
        rule: 'intent_mismatch',
        reason: `prepared transaction ${position} has index ${transaction.index}, the intent's has ${committed.index}`,
      };
    }
    const read = decoded(transaction.transactionBase64);
    if (read === null) {
      return { rule: 'intent_mismatch', reason: `prepared transaction ${committed.index} does not decode` };
    }
    if (messageSha256(read.bytes, read.decoded) !== committed.messageSha256) {
      return {
        rule: 'intent_mismatch',
        reason: `prepared transaction ${committed.index} is not the message the intent committed to`,
      };
    }
    if (read.decoded.version !== (format === 'v1' ? 1 : 0)) {
      return {
        rule: 'transaction_format',
        reason: `prepared transaction ${committed.index} is not a ${format} transaction`,
      };
    }
    if (signerSlot(read.decoded, wallet) < 0) {
      return {
        rule: 'intent_mismatch',
        reason: `the wallet ${wallet} is not a required signer of prepared transaction ${committed.index}`,
      };
    }
  }
  return null;
}

/**
 * Whether `signed` is `prepared` with only the wallet's signature slot
 * filled, by the wallet. `null` when it is.
 *
 * `prepared` is the baseline: pass a copy the signer never had, because a
 * signer that can change the objects it was given could change the
 * baseline too.
 */
export function checkSignedTransactions(input: {
  readonly wallet: string;
  readonly intent: ExecutionIntentV1;
  readonly prepared: readonly ProtocolPreparedTransaction[];
  readonly signed: readonly string[];
}): Refusal | null {
  const { wallet, intent, prepared, signed } = input;
  if (signed.length !== prepared.length) {
    return {
      rule: 'signer_modified',
      reason: `the signer returned ${signed.length} transactions for ${prepared.length}`,
    };
  }
  const walletKey = decodeBase58(wallet);
  for (const [position, base64] of signed.entries()) {
    const committed = intent.transactions[position]!;
    const index = committed.index;
    const after = decoded(base64);
    const before = decoded(prepared[position]!.transactionBase64);
    if (after === null || before === null) {
      return { rule: 'signer_modified', reason: `signed transaction ${index} does not decode` };
    }
    if (messageSha256(after.bytes, after.decoded) !== committed.messageSha256) {
      return {
        rule: 'signer_modified',
        reason: `the signer changed the message of transaction ${index}; its hash no longer matches the intent`,
      };
    }
    const slot = signerSlot(after.decoded, wallet);
    if (slot < 0 || after.decoded.signatures.length !== before.decoded.signatures.length) {
      return { rule: 'signer_modified', reason: `the signer changed the signers of transaction ${index}` };
    }
    for (const [other, signature] of after.decoded.signatures.entries()) {
      if (other !== slot && !sameBytes(signature, before.decoded.signatures[other]!)) {
        return {
          rule: 'signer_modified',
          reason: `the signer changed signature ${other} of transaction ${index}, which is not the wallet's`,
        };
      }
    }
    // Nothing but the wallet's 64 signature bytes may differ, so the same
    // message and signatures under another encoding are refused too.
    const at = after.decoded.signatureOffset + 64 * slot;
    const beforeSlot = signerSlot(before.decoded, wallet);
    if (
      after.bytes.length !== before.bytes.length ||
      before.decoded.signatureOffset + 64 * beforeSlot !== at ||
      !sameBytes(after.bytes.subarray(0, at), before.bytes.subarray(0, at)) ||
      !sameBytes(after.bytes.subarray(at + 64), before.bytes.subarray(at + 64))
    ) {
      return {
        rule: 'signer_modified',
        reason: `the signer changed bytes of transaction ${index} outside the wallet's signature`,
      };
    }
    const message = after.bytes.subarray(after.decoded.message.start, after.decoded.message.end);
    const signature = after.decoded.signatures[slot]!;
    let valid = false;
    try {
      valid = walletKey !== null && ed25519.verify(signature, message, walletKey);
    } catch {
      valid = false;
    }
    if (!valid) {
      return {
        rule: 'signer_signature',
        reason: `transaction ${index} does not carry a valid signature by ${wallet}`,
      };
    }
  }
  return null;
}
