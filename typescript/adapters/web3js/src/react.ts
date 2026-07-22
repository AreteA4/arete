import { useMemo } from 'react';
import { useWallet } from '@solana/wallet-adapter-react';
import type { VersionedTransaction } from '@solana/web3.js';
import { createWalletAdapter } from './index.js';
import type { AdapterTransportSelection, Web3JsWalletAdapter } from './index.js';

export interface UseSolanaWalletAdapterOptions {
  transport?: AdapterTransportSelection;
}

/**
 * Bridge `@solana/wallet-adapter-react`'s `useWallet` into an Arete wallet
 * adapter. Returns `undefined` until a wallet that can sign is connected, so
 * it can be passed straight to `<AreteProvider wallet={...}>`.
 */
export function useSolanaWalletAdapter(
  options: UseSolanaWalletAdapterOptions = {},
): Web3JsWalletAdapter | undefined {
  const wallet = useWallet();
  const transport = options.transport ?? 'auto';
  return useMemo(() => {
    if (!wallet.publicKey || !wallet.signTransaction) return undefined;
    const { publicKey, signTransaction } = wallet;
    return createWalletAdapter({
      transport,
      signer: {
        publicKey,
        supportedTransactionVersions:
          wallet.wallet?.adapter.supportedTransactionVersions ?? null,
        signTransaction: (tx) =>
          signTransaction(tx as VersionedTransaction) as Promise<VersionedTransaction>,
      },
    });
  }, [wallet.publicKey, wallet.signTransaction, wallet.wallet, transport]);
}
