/**
 * Signs for every dev wallet of the project (`.arete/dev/wallets/*.json`,
 * the keypairs `a4 dev wallet` creates), so several dev wallets can stand in
 * for users. Wallets created while it runs are picked up at the next
 * signature. Dev environments only: the keys are plain files.
 */

import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import { ed25519 } from '@noble/curves/ed25519';

import { keypairSigner } from '../keypair';
import type { ProtocolSigner } from '../signer/signer';

type Keypair = ProtocolSigner & {
  readonly publicKey: string;
  /** Sign a message (a consent proof) as the wallet's owner would. */
  signMessage(message: Uint8Array): Uint8Array;
};

/** The dev wallets in `dir`, by public key and by file name. */
export function readDevWallets(dir: string): { byAddress: Map<string, Keypair>; byName: Map<string, Keypair> } {
  const byAddress = new Map<string, Keypair>();
  const byName = new Map<string, Keypair>();
  let files: string[];
  try {
    files = readdirSync(dir).filter((file) => file.endsWith('.json'));
  } catch {
    files = [];
  }
  for (const file of files) {
    try {
      const secret = Uint8Array.from(JSON.parse(readFileSync(join(dir, file), 'utf8')) as number[]);
      const seed = secret.slice(0, 32);
      const signer: Keypair = {
        ...keypairSigner(secret),
        signMessage: (message) => ed25519.sign(message, seed),
      };
      secret.fill(0);
      byAddress.set(signer.publicKey, signer);
      byName.set(file.slice(0, -'.json'.length), signer);
    } catch {
      // Not a keypair file; skipped.
    }
  }
  return { byAddress, byName };
}

/** The signer over the dev wallets in `dir`; `@usearete/automations/dev` wraps it. */
export function walletsSigner(dir: string): ProtocolSigner {
  const lookup = (wallet: string) => readDevWallets(dir).byAddress.get(wallet);
  return {
    kind: 'dev-wallets',
    supportedTransactionFormats: ['v1', 'v0'],
    async canSign(wallet) {
      return lookup(wallet) !== undefined;
    },
    async delegationStatus(wallet) {
      return {
        delegated: lookup(wallet) !== undefined,
        providerPolicy: false,
        detail: 'a dev wallet: a4 dev holds its key, for the dev environment only',
      };
    },
    async signTransactions(input) {
      const signer = lookup(input.wallet);
      if (!signer) throw new Error(`${input.wallet} is not a dev wallet of this project`);
      return signer.signTransactions(input);
    },
  };
}
