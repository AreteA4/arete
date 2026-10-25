/**
 * `@usearete/react/wallet-standard`: the connected Wallet Standard account as
 * the wallet Arete signs with.
 *
 * Kept out of the main entry so apps that do not use Wallet Standard do not
 * need its packages: this entry needs `@usearete/adapter-kit` (and its
 * `@solana/kit` peer) and `@wallet-standard/ui-registry`, which every
 * `@wallet-standard/react` app already has.
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import {
  createWalletStandardWalletAdapter,
  type AdapterTransportSelection,
  type KitWalletAdapter,
  type WalletStandardAccount,
  type WalletStandardWallet,
} from '@usearete/adapter-kit';
import type { UiWalletAccount } from '@wallet-standard/ui-core';
import {
  getWalletAccountForUiWalletAccount,
  getWalletForHandle,
} from '@wallet-standard/ui-registry';

/** A Wallet Standard wallet and one of its accounts, outside React's UI handles. */
export interface WalletStandardSelection {
  readonly wallet: WalletStandardWallet;
  readonly account: WalletStandardAccount;
}

export interface UseWalletStandardWalletOptions {
  /** CAIP-2 chain the wallet signs for, e.g. `solana:mainnet`. */
  readonly chain?: string;
  /**
   * Where the signed transaction is submitted. Defaults to `'auto'`: the
   * Arete client's or session's transaction transport.
   */
  readonly transport?: AdapterTransportSelection;
}

export interface UseWalletStandardWalletResult {
  /**
   * The sign-only wallet for `AreteProvider wallet={...}`, `createSession`
   * or `execute(prepared, { wallet })`. `undefined` while nothing is
   * connected or when the account cannot be used (see `error`).
   */
  readonly wallet: KitWalletAdapter | undefined;
  /** The connected account's address. */
  readonly address: string | null;
  /** The wallet's approval prompt is open: show "check your wallet". */
  readonly isPromptOpen: boolean;
  /**
   * Why the account cannot sign for Arete, for example a
   * `WalletStandardSignerError` with code `sign_transaction_unsupported`
   * (the wallet only signs and sends in one step).
   */
  readonly error: Error | null;
}

function isSelection(
  source: UiWalletAccount | WalletStandardSelection
): source is WalletStandardSelection {
  return 'wallet' in source && 'account' in source;
}

function resolveSelection(
  source: UiWalletAccount | WalletStandardSelection
): WalletStandardSelection {
  if (isSelection(source)) return source;
  // The registry hands back the wallet's own objects: the wallet may compare
  // the account it is asked to sign for by identity.
  return {
    wallet: getWalletForHandle(source),
    account: getWalletAccountForUiWalletAccount(source),
  };
}

/**
 * The connected Wallet Standard account as the wallet the Arete provider and
 * sessions sign with. Sign only: the wallet signs (`solana:signTransaction`)
 * and Arete submits the signed bytes through the transport it already uses.
 *
 * ```tsx
 * const { wallets } = useWallets(); // @wallet-standard/react
 * const account = selectedWallet?.accounts[0] ?? null;
 * const { wallet, isPromptOpen, error } = useWalletStandardWallet(account, { chain: 'solana:mainnet' });
 * return <AreteProvider wallet={wallet}>…{isPromptOpen && 'Check your wallet'}</AreteProvider>;
 * ```
 *
 * Accepts a `UiWalletAccount` from `@wallet-standard/react`, or a raw
 * `{ wallet, account }` pair. The returned adapter is stable while the
 * account, chain and transport stay the same.
 */
export function useWalletStandardWallet(
  source: UiWalletAccount | WalletStandardSelection | null | undefined,
  options: UseWalletStandardWalletOptions = {}
): UseWalletStandardWalletResult {
  const [openPrompts, setOpenPrompts] = useState(0);
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  let selection: WalletStandardSelection | null = null;
  let resolveError: Error | null = null;
  if (source) {
    try {
      selection = resolveSelection(source);
    } catch (error) {
      resolveError = error instanceof Error ? error : new Error(String(error));
    }
  }

  const wallet = selection?.wallet;
  const account = selection?.account;
  const { chain, transport } = options;
  const created = useMemo((): { adapter?: KitWalletAdapter; error: Error | null } => {
    if (!wallet || !account) return { error: null };
    const adjust = (delta: number) => {
      if (mountedRef.current) setOpenPrompts((count) => Math.max(0, count + delta));
    };
    try {
      return {
        adapter: createWalletStandardWalletAdapter({
          wallet,
          account,
          chain,
          transport,
          onPromptOpen: () => adjust(1),
          onPromptClose: () => adjust(-1),
        }),
        error: null,
      };
    } catch (error) {
      return { error: error instanceof Error ? error : new Error(String(error)) };
    }
  }, [wallet, account, chain, transport]);

  // A prompt left open by a previous account does not belong to this one.
  useEffect(() => {
    setOpenPrompts(0);
  }, [created]);

  return {
    wallet: created.adapter,
    address: account?.address ?? null,
    isPromptOpen: openPrompts > 0,
    error: resolveError ?? created.error,
  };
}

export type {
  KitWalletAdapter,
  WalletStandardAccount,
  WalletStandardWallet,
} from '@usearete/adapter-kit';
