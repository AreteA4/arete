/**
 * Compile-time checks: the Wallet Standard objects an app holds are accepted
 * by the bridge as they are, without casts. Checked by `tsc --noEmit`; never
 * bundled.
 */

import type { Wallet, WalletAccount } from '@wallet-standard/base';
import {
  createWalletStandardSigner,
  createWalletStandardWalletAdapter,
  type KitWalletAdapter,
} from './index';

declare const wallet: Wallet;
declare const account: WalletAccount;

export const signer = createWalletStandardSigner({ wallet, account, chain: 'solana:devnet' });
export const adapter: KitWalletAdapter = createWalletStandardWalletAdapter({
  wallet,
  account: wallet.accounts[0] ?? account,
  chain: 'solana:mainnet',
  onPromptOpen: () => undefined,
  onPromptClose: ({ signed }) => void signed,
});
