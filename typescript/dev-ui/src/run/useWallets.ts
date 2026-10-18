import { useCallback, useEffect, useState } from "react";

import { listWallets, WALLETS_CHANGED, type DevWalletBalance } from "./runApi";

/** The dev wallets with balances, refreshed whenever one changes. */
export function useWallets(): { wallets: DevWalletBalance[]; refresh(): void } {
  const [wallets, setWallets] = useState<DevWalletBalance[]>([]);
  const refresh = useCallback(() => {
    listWallets().then(
      (result) => setWallets(result.wallets),
      () => undefined,
    );
  }, []);
  useEffect(() => {
    refresh();
    window.addEventListener(WALLETS_CHANGED, refresh);
    return () => window.removeEventListener(WALLETS_CHANGED, refresh);
  }, [refresh]);
  return { wallets, refresh };
}

export function sol(lamports: number | null | undefined): string {
  if (lamports === null || lamports === undefined) return "?";
  return `${(lamports / 1e9).toLocaleString(undefined, { maximumFractionDigits: 4 })} SOL`;
}
