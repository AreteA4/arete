import { type ReactNode } from 'react';
import { WalletProvider } from '@solana/wallet-adapter-react';
import { WalletModalProvider } from '@solana/wallet-adapter-react-ui';
import { useSolanaWalletAdapter } from '@usearete/adapter-web3js/react';
import { AreteProvider } from '@usearete/react';
import { OreDashboard } from './components/OreDashboard';
import { appConfig } from './config';
import { ORE_STREAM_STACK } from './generated/ore-stack';
import { ThemeProvider } from './hooks/useTheme';

import '@solana/wallet-adapter-react-ui/styles.css';

// `useSolanaWalletAdapter` reads the wallet context, so the AreteProvider
// lives one level below WalletProvider in this small shell. Keeping the stack
// on the provider applies its endpoint overrides; components pass it explicitly
// to `useArete(ORE_STREAM_STACK)` so their data dependency is visible locally.
function AreteShell({ children }: { children: ReactNode }) {
  const wallet = useSolanaWalletAdapter();
  return (
    <AreteProvider
      autoConnect
      stack={ORE_STREAM_STACK}
      stackOptions={appConfig.areteOptions}
      wallet={wallet}
      auth={appConfig.publishableKey ? { publishableKey: appConfig.publishableKey } : undefined}
    >
      {children}
    </AreteProvider>
  );
}

export default function App() {
  if (appConfig.configurationError) {
    return (
      <main className="grid min-h-dvh place-items-center bg-stone-100 p-6 font-sans text-stone-900">
        <section className="max-w-lg rounded-2xl bg-white p-6 shadow-sm" role="alert">
          <h1 className="text-lg font-semibold">Arete configuration required</h1>
          <p className="mt-2 text-sm text-stone-600">{appConfig.configurationError}</p>
          <p className="mt-3 text-sm text-stone-500">
            Add the key to <code>.env.local</code>, then restart the development server.
          </p>
        </section>
      </main>
    );
  }

  return (
    <ThemeProvider>
      <WalletProvider wallets={[]} autoConnect>
        <WalletModalProvider>
          <AreteShell>
            <OreDashboard />
          </AreteShell>
        </WalletModalProvider>
      </WalletProvider>
    </ThemeProvider>
  );
}
