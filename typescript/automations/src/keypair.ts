/**
 * A signer over a local ed25519 keypair, for development, tests and dev
 * wallets. Production automations sign through a provider whose own policy
 * bounds what the key may sign.
 */

import { ed25519 } from '@noble/curves/ed25519';
import type { ProtocolTransactionFormat } from '@usearete/sdk';

import { decodeTransaction } from './policy/decode';
import { decodeBase64, encodeBase58, encodeBase64 } from './policy/encoding';
import type { ProtocolSigner, SignTransactionsInput } from './signer/signer';

export interface KeypairSignerOptions {
  /** Formats to offer. Defaults to both. */
  readonly transactionFormats?: readonly ProtocolTransactionFormat[];
}

/**
 * A {@link ProtocolSigner} for one keypair: a Solana CLI secret key (64
 * bytes, the seed then the public key, or the JSON array of them) or a
 * 32-byte seed. The key is held only for signing; it never appears in
 * errors or in the signer's properties.
 */
export function keypairSigner(
  secretKey: Uint8Array | readonly number[],
  options: KeypairSignerOptions = {},
): ProtocolSigner & { readonly publicKey: string } {
  const bytes = Uint8Array.from(secretKey);
  if (bytes.length !== 64 && bytes.length !== 32) {
    throw new Error('a keypair secret key is 64 bytes (seed and public key) or a 32-byte seed');
  }
  const seed = bytes.slice(0, 32);
  const publicKeyBytes = ed25519.getPublicKey(seed);
  if (bytes.length === 64 && !bytes.subarray(32).every((byte, index) => byte === publicKeyBytes[index])) {
    throw new Error("the keypair's public key does not match its secret key");
  }
  bytes.fill(0);
  const publicKey = encodeBase58(publicKeyBytes);
  return {
    kind: 'keypair',
    publicKey,
    supportedTransactionFormats: options.transactionFormats ?? ['v1', 'v0'],
    async canSign(wallet) {
      return wallet === publicKey;
    },
    async signTransactions({ wallet, transactions }: SignTransactionsInput) {
      if (wallet !== publicKey) {
        throw new Error(`this keypair signs for ${publicKey}, not ${wallet}`);
      }
      return transactions.map(({ index, transactionBase64 }) => {
        const wire = decodeBase64(transactionBase64);
        if (wire === null) throw new Error(`prepared transaction ${index} is not base64`);
        const decoded = decodeTransaction(wire);
        const required = Math.min(decoded.requiredSignatures, decoded.signatures.length);
        const slot = decoded.staticKeys.slice(0, required).indexOf(publicKey);
        if (slot < 0) {
          throw new Error(`the wallet ${publicKey} is not a required signer of transaction ${index}`);
        }
        const signature = ed25519.sign(wire.subarray(decoded.message.start, decoded.message.end), seed);
        const signed = wire.slice();
        signed.set(signature, decoded.signatureOffset + 64 * slot);
        return encodeBase64(signed);
      });
    },
  };
}
