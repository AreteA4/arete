import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import type { Wallet, WalletAccount } from "@wallet-standard/base";

import { postJson } from "../api";
import type { PreparedPreview, StepResult } from "../run/runApi";
import { airdrop } from "../run/runApi";
import { connect, disconnect, onAccountsChange, signTransactions, watchWallets } from "./standard";

export interface BrowserWalletState {
  wallets: Wallet[];
  wallet: Wallet | null;
  account: WalletAccount | null;
  connect(wallet: Wallet): Promise<void>;
  disconnect(): Promise<void>;
  /** Prepare for the connected account, sign in the wallet, submit through a4 dev. */
  run(
    input: { step: string; instance: string | null; args: Record<string, unknown> },
    onPreview: (preview: PreparedPreview) => void,
  ): Promise<StepResult>;
}

const Context = createContext<BrowserWalletState | null>(null);

export function useBrowserWallet(): BrowserWalletState | null {
  return useContext(Context);
}

export function BrowserWalletProvider({ children }: { children: ReactNode }) {
  const [wallets, setWallets] = useState<Wallet[]>([]);
  const [wallet, setWallet] = useState<Wallet | null>(null);
  const [account, setAccount] = useState<WalletAccount | null>(null);

  useEffect(() => watchWallets(setWallets), []);
  useEffect(() => {
    if (!wallet) return;
    return onAccountsChange(wallet, (accounts) => setAccount(accounts[0] ?? null));
  }, [wallet]);

  const connectWallet = useCallback(async (next: Wallet) => {
    const connected = await connect(next);
    setWallet(next);
    setAccount(connected);
  }, []);

  const disconnectWallet = useCallback(async () => {
    if (wallet) await disconnect(wallet).catch(() => undefined);
    setWallet(null);
    setAccount(null);
  }, [wallet]);

  const run = useCallback<BrowserWalletState["run"]>(
    async (input, onPreview) => {
      if (!wallet || !account) throw new Error("Connect a wallet first");
      const prepared = await postJson<PreparedPreview | StepResult>("/dev/v1/wallet/prepare", { ...input, pubkey: account.address });
      if (prepared.state !== "prepared") return prepared as StepResult;
      const preview = prepared as PreparedPreview;
      onPreview(preview);
      const transactions = preview.transactions.map((transaction) => transaction.transactionBase64 ?? "");
      const signed = await signTransactions(wallet, account, transactions);
      return postJson<StepResult>("/dev/v1/wallet/submit", { executionId: preview.executionId, signedTransactions: signed });
    },
    [wallet, account],
  );

  const value = useMemo<BrowserWalletState>(
    () => ({ wallets, wallet, account, connect: connectWallet, disconnect: disconnectWallet, run }),
    [wallets, wallet, account, connectWallet, disconnectWallet, run],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

/**
 * Always shown with the wallet controls. Wallets are asked for the
 * `solana:mainnet` chain (they know no dev environment); `a4 dev` only hands
 * them transactions on the environment's synthetic blockhashes, which
 * mainnet rejects.
 */
export function LocalNetworkNotice() {
  return (
    <span className="wallet-notice small" role="note" data-testid="local-network-notice">
      Dev environment: transactions here use synthetic blockhashes and cannot land on mainnet. Use a dev or burner wallet. Wallets that
      change a transaction before signing (adding instructions or priority fees) are refused.
    </span>
  );
}

function short(address: string): string {
  return `${address.slice(0, 4)}…${address.slice(-4)}`;
}

/** The toolbar's wallet control: connect, airdrop to the connected address, disconnect. */
export function WalletBar() {
  const state = useBrowserWallet();
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  if (!state) return null;
  const { wallets, wallet, account } = state;
  const guard = (action: () => Promise<unknown>) => {
    setError(null);
    action().catch((failure) => setError((failure as Error).message));
  };
  if (!wallet || !account) {
    return (
      <span className="wallet-bar" data-testid="wallet-bar">
        {wallets.length === 0 ? (
          <span className="muted small">No browser wallet found (Wallet Standard)</span>
        ) : (
          wallets.map((candidate) => (
            <button key={candidate.name} onClick={() => guard(() => state.connect(candidate))} data-testid="connect-wallet">
              {candidate.icon ? <img src={candidate.icon} alt="" width={14} height={14} /> : null} Connect {candidate.name}
            </button>
          ))
        )}
        {wallets.length > 0 ? <LocalNetworkNotice /> : null}
        {error ? <span className="error-text small">{error}</span> : null}
      </span>
    );
  }
  return (
    <span className="wallet-bar" data-testid="wallet-bar">
      <span className="pill mono" title={account.address} data-testid="connected-wallet">
        {wallet.name} {short(account.address)}
      </span>
      <button
        data-testid="airdrop-connected"
        onClick={() =>
          guard(async () => {
            const result = await airdrop({ pubkey: account.address }, 10);
            setNote(`+10 dev SOL (${((result.lamports ?? 0) / 1e9).toFixed(2)} SOL)`);
          })
        }
      >
        Airdrop 10 SOL
      </button>
      <button onClick={() => guard(state.disconnect)}>Disconnect</button>
      {note ? <span className="tool-ok small">{note}</span> : null}
      <span className="muted small" title="The wallet simulates against its own cluster, not this dev environment: it may warn, or show balances that differ from the environment's.">
        ⓘ the wallet simulates against its own cluster
      </span>
      <LocalNetworkNotice />
      {error ? <span className="error-text small">{error}</span> : null}
    </span>
  );
}
