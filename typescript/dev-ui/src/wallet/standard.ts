/**
 * Browser wallets through the Wallet Standard: discovery, connect and
 * `solana:signTransaction`. The dev UI only ever asks a wallet to sign
 * (never to sign and send): the signed bytes go back to `a4 dev`, which
 * submits them through the Arete API exactly as signed.
 */

import { getWallets } from "@wallet-standard/app";
import type { Wallet, WalletAccount } from "@wallet-standard/base";

export const CONNECT = "standard:connect";
export const DISCONNECT = "standard:disconnect";
export const EVENTS = "standard:events";
export const SIGN_TRANSACTION = "solana:signTransaction";

interface ConnectFeature {
  version: string;
  connect(input?: { silent?: boolean }): Promise<{ accounts: readonly WalletAccount[] }>;
}

interface DisconnectFeature {
  version: string;
  disconnect(): Promise<void>;
}

interface SignTransactionFeature {
  version: string;
  supportedTransactionVersions: ReadonlyArray<"legacy" | 0>;
  signTransaction(
    ...inputs: Array<{ account: WalletAccount; transaction: Uint8Array; chain?: string }>
  ): Promise<ReadonlyArray<{ signedTransaction: Uint8Array }>>;
}

interface EventsFeature {
  version: string;
  on(event: "change", listener: (properties: { accounts?: readonly WalletAccount[] }) => void): () => void;
}

function feature<T>(wallet: Wallet, name: string): T | undefined {
  return (wallet.features as Record<string, unknown>)[name] as T | undefined;
}

/** Wallets that can connect and sign v0 Solana transactions. */
export function isUsable(wallet: Wallet): boolean {
  const sign = feature<SignTransactionFeature>(wallet, SIGN_TRANSACTION);
  return (
    feature<ConnectFeature>(wallet, CONNECT) !== undefined &&
    sign !== undefined &&
    sign.supportedTransactionVersions.includes(0) &&
    wallet.chains.some((chain) => chain.startsWith("solana:"))
  );
}

/** The registered usable wallets, now and as they register. */
export function watchWallets(onChange: (wallets: Wallet[]) => void): () => void {
  const api = getWallets();
  const emit = () => onChange(api.get().filter(isUsable));
  emit();
  const offRegister = api.on("register", emit);
  const offUnregister = api.on("unregister", emit);
  return () => {
    offRegister();
    offUnregister();
  };
}

export async function connect(wallet: Wallet): Promise<WalletAccount> {
  const connector = feature<ConnectFeature>(wallet, CONNECT);
  if (!connector) throw new Error(`${wallet.name} cannot connect`);
  const { accounts } = await connector.connect();
  const account = accounts.find((candidate) => candidate.chains.some((chain) => chain.startsWith("solana:"))) ?? accounts[0];
  if (!account) throw new Error(`${wallet.name} returned no account`);
  return account;
}

export async function disconnect(wallet: Wallet): Promise<void> {
  await feature<DisconnectFeature>(wallet, DISCONNECT)?.disconnect();
}

export function onAccountsChange(wallet: Wallet, listener: (accounts: readonly WalletAccount[]) => void): () => void {
  const events = feature<EventsFeature>(wallet, EVENTS);
  if (!events) return () => undefined;
  return events.on("change", (properties) => {
    if (properties.accounts) listener(properties.accounts);
  });
}

/**
 * The chain to name when signing. A dev environment (a fork) has no chain
 * id of its own; the wallet sees the cluster the fork copies (mainnet
 * unless the account only lists another).
 */
export function chainFor(account: WalletAccount): string {
  return account.chains.find((chain) => chain === "solana:mainnet") ?? account.chains.find((chain) => chain.startsWith("solana:")) ?? "solana:mainnet";
}

export function base64ToBytes(text: string): Uint8Array {
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

export function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

/** Sign each transaction (base64) with the connected account; returns base64. */
export async function signTransactions(wallet: Wallet, account: WalletAccount, transactions: string[]): Promise<string[]> {
  const signer = feature<SignTransactionFeature>(wallet, SIGN_TRANSACTION);
  if (!signer) throw new Error(`${wallet.name} cannot sign transactions`);
  const chain = chainFor(account);
  const outputs = await signer.signTransaction(
    ...transactions.map((transaction) => ({ account, transaction: base64ToBytes(transaction), chain })),
  );
  if (outputs.length !== transactions.length) throw new Error(`${wallet.name} returned ${outputs.length} of ${transactions.length} signed transactions`);
  return outputs.map((output) => bytesToBase64(output.signedTransaction));
}
