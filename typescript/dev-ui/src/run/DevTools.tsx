import { useState, type ReactNode } from "react";

import { airdrop, createWallet, timeTravel } from "./runApi";
import { sol, useWallets } from "./useWallets";

/** Dev environment tools: time travel, airdrops and new dev wallets. */
export function DevTools({ children }: { children?: ReactNode }) {
  const { wallets } = useWallets();
  const [travel, setTravel] = useState("+1h");
  const [target, setTarget] = useState("");
  const [amount, setAmount] = useState("10");
  const [newName, setNewName] = useState("");
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  const act = async (action: () => Promise<string>) => {
    setBusy(true);
    try {
      setMessage({ ok: true, text: await action() });
    } catch (error) {
      setMessage({ ok: false, text: (error as Error).message });
    } finally {
      setBusy(false);
    }
  };

  const walletTarget = target || wallets[0]?.name || "";

  return (
    <div className="dev-tools" data-testid="dev-tools">
      <span className="tool">
        <input value={travel} onChange={(event) => setTravel(event.target.value)} size={6} aria-label="Time travel" />
        <button
          disabled={busy}
          data-testid="time-travel"
          onClick={() =>
            act(async () => {
              const result = await timeTravel(travel);
              return `Clock ${new Date(result.before * 1000).toISOString()} → ${new Date(result.unixTimestamp * 1000).toISOString()}`;
            })
          }
        >
          Time travel
        </button>
      </span>
      <span className="tool">
        <select value={walletTarget} onChange={(event) => setTarget(event.target.value)} aria-label="Airdrop to">
          {wallets.map((wallet) => (
            <option key={wallet.name} value={wallet.name}>
              {wallet.name} · {sol(wallet.lamports)}
            </option>
          ))}
        </select>
        <input value={amount} onChange={(event) => setAmount(event.target.value)} size={4} aria-label="SOL" />
        <button
          disabled={busy || !walletTarget}
          data-testid="airdrop"
          onClick={() =>
            act(async () => {
              const result = await airdrop({ wallet: walletTarget }, Number(amount));
              return `Airdropped ${result.airdropped / 1e9} SOL to ${walletTarget}`;
            })
          }
        >
          Airdrop
        </button>
      </span>
      <span className="tool">
        <input value={newName} placeholder="wallet name" onChange={(event) => setNewName(event.target.value)} size={10} aria-label="New wallet" />
        <button
          disabled={busy || !newName}
          data-testid="create-wallet"
          onClick={() =>
            act(async () => {
              const wallet = await createWallet(newName);
              setNewName("");
              return `Created ${wallet.name} (${wallet.pubkey.slice(0, 8)}…)`;
            })
          }
        >
          New dev wallet
        </button>
      </span>
      {children}
      {message ? (
        <span className={message.ok ? "tool-ok small" : "error-text small"} data-testid="tool-message">
          {message.text}
        </span>
      ) : null}
    </div>
  );
}
