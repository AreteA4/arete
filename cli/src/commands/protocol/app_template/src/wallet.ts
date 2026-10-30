/**
 * A browser wallet that injects `window.solana` (Phantom, Solflare,
 * Backpack). Swap in your wallet adapter or Privy's React SDK: the app only
 * needs the address and a way to sign the prepared transactions as they are.
 */
import { VersionedTransaction } from "@solana/web3.js";

interface InjectedWallet {
  connect(): Promise<{ publicKey: { toBase58(): string } }>;
  signAllTransactions(transactions: VersionedTransaction[]): Promise<VersionedTransaction[]>;
}

function injected(): InjectedWallet {
  const wallet = (window as { solana?: InjectedWallet }).solana;
  if (!wallet) throw new Error("no browser wallet found: install Phantom, Solflare or Backpack");
  return wallet;
}

export async function connectWallet(): Promise<string> {
  return (await injected().connect()).publicKey.toBase58();
}

const toBytes = (base64: string) => Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
const toBase64 = (bytes: Uint8Array) => btoa(String.fromCharCode(...bytes));

/** Sign each prepared transaction, in order, keeping the signatures already on it. */
export async function signPrepared(transactions: readonly { transactionBase64: string }[]): Promise<string[]> {
  const decoded = transactions.map((tx) => VersionedTransaction.deserialize(toBytes(tx.transactionBase64)));
  const signed = await injected().signAllTransactions(decoded);
  return signed.map((tx) => toBase64(tx.serialize()));
}
