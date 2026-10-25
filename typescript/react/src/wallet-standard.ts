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
  WalletStandardSignerError,
  createWalletStandardWalletAdapter,
  isWalletStandardSignerError,
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
   * Why the account cannot sign for Arete; switch on `error.code`:
   *
   * - `sign_transaction_unsupported`: the wallet lacks
   *   `solana:signTransaction` (it only signs and sends in one step).
   * - `chain_unsupported`: the account does not sign for `options.chain`.
   * - `account_unavailable`: the account could not be read, or its wallet is
   *   no longer registered.
   *
   * A declined prompt is not reported here: it fails the run
   * (`useOperationMutation`'s `wallet-rejected`), and the wallet stays usable.
   */
  readonly error: WalletStandardSignerError | null;
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
  // Open prompts, counted per adapter: a prompt left open by a previous
  // account does not belong to the current one.
  const [prompts, setPrompts] = useState<{
    readonly adapter: KitWalletAdapter | null;
    readonly open: number;
  }>({ adapter: null, open: 0 });
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  let selection: WalletStandardSelection | null = null;
  let resolveError: WalletStandardSignerError | null = null;
  if (source) {
    try {
      selection = resolveSelection(source);
    } catch (cause) {
      resolveError = new WalletStandardSignerError(
        'account_unavailable',
        cause instanceof Error && cause.message
          ? cause.message
          : 'The wallet account is not available',
        { cause }
      );
    }
  }

  const wallet = selection?.wallet;
  const account = selection?.account;
  const { chain, transport } = options;
  const created = useMemo((): {
    adapter?: KitWalletAdapter;
    error: WalletStandardSignerError | null;
  } => {
    if (!wallet || !account) return { error: null };
    let adapter: KitWalletAdapter | null = null;
    const adjust = (delta: number) => {
      if (!mountedRef.current) return;
      setPrompts((current) => {
        const open = current.adapter === adapter ? current.open : 0;
        return { adapter, open: Math.max(0, open + delta) };
      });
    };
    try {
      adapter = createWalletStandardWalletAdapter({
        wallet,
        account,
        chain,
        transport,
        onPromptOpen: () => adjust(1),
        onPromptClose: () => adjust(-1),
      });
      return { adapter, error: null };
    } catch (error) {
      return {
        error: isWalletStandardSignerError(error)
          ? error
          : new WalletStandardSignerError(
            'account_unavailable',
            error instanceof Error ? error.message : String(error),
            { cause: error }
          ),
      };
    }
  }, [wallet, account, chain, transport]);

  return {
    wallet: created.adapter,
    address: account?.address ?? null,
    isPromptOpen: created.adapter !== undefined
      && prompts.adapter === created.adapter
      && prompts.open > 0,
    error: resolveError ?? created.error,
  };
}

export { WalletStandardSignerError, isWalletStandardSignerError } from '@usearete/adapter-kit';
export type {
  KitWalletAdapter,
  WalletStandardAccount,
  WalletStandardSignerErrorCode,
  WalletStandardWallet,
} from '@usearete/adapter-kit';
