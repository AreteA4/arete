import { useMemo, type ReactNode } from 'react';
import { ConnectionProvider, WalletProvider, useConnection, useWallet } from '@solana/wallet-adapter-react';
import { WalletModalProvider } from '@solana/wallet-adapter-react-ui';
import type { TransactionVersion, VersionedTransaction } from '@solana/web3.js';
import { createWalletAdapter } from '@usearete/adapter-web3js';
import { AreteProvider } from '@usearete/react';
import { OreDashboard } from './components';
import { appConfig } from './config';
import { ThemeProvider } from './hooks/useTheme';

import '@solana/wallet-adapter-react-ui/styles.css';

function signerFromWallet(wallet: ReturnType<typeof useWallet>) {
  if (!wallet.publicKey || !wallet.signTransaction) return undefined;
  return {
    publicKey: wallet.publicKey,
    supportedTransactionVersions: wallet.wallet?.adapter.supportedTransactionVersions as
      | ReadonlySet<TransactionVersion>
      | null
      | undefined,
    signTransaction: (transaction: VersionedTransaction) =>
      wallet.signTransaction!(transaction) as Promise<VersionedTransaction>,
  };
}

function Provider({ wallet, children }: {
  wallet: ReturnType<typeof createWalletAdapter> | undefined;
  children: ReactNode;
}) {
  return (
    <AreteProvider
      autoConnect
      wallet={wallet}
      auth={appConfig.publishableKey ? { publishableKey: appConfig.publishableKey } : undefined}
    >
      {children}
    </AreteProvider>
  );
}

function AutoAreteWalletBridge({ children }: { children: ReactNode }) {
  const wallet = useWallet();
  const areteWallet = useMemo(() => {
    const signer = signerFromWallet(wallet);
    if (!signer) return undefined;
    return createWalletAdapter({
      transport: 'auto',
      signer,
    });
  }, [wallet.publicKey, wallet.signTransaction, wallet.wallet]);
  return <Provider wallet={areteWallet}>{children}</Provider>;
}

function DirectAreteWalletBridge({ children }: { children: ReactNode }) {
  const { connection } = useConnection();
  const wallet = useWallet();
  const areteWallet = useMemo(() => {
    const signer = signerFromWallet(wallet);
    if (!signer) return undefined;
    return createWalletAdapter({ connection, transport: 'direct', signer });
  }, [connection, wallet.publicKey, wallet.signTransaction, wallet.wallet]);
  return <Provider wallet={areteWallet}>{children}</Provider>;
}

function WalletShell({ children }: { children: ReactNode }) {
  return (
    <WalletProvider wallets={[]} autoConnect>
      <WalletModalProvider>{children}</WalletModalProvider>
    </WalletProvider>
  );
}

export default function App() {
  return (
    <ThemeProvider>
      {appConfig.transactionTransport === 'direct' ? (
        <ConnectionProvider endpoint={appConfig.solanaRpcUrl!} config={{ commitment: 'confirmed' }}>
          <WalletShell><DirectAreteWalletBridge><OreDashboard /></DirectAreteWalletBridge></WalletShell>
        </ConnectionProvider>
      ) : (
        <WalletShell><AutoAreteWalletBridge><OreDashboard /></AutoAreteWalletBridge></WalletShell>
      )}
    </ThemeProvider>
  );
}
