import { useWallet } from '@solana/wallet-adapter-react';
import { useArete } from '@usearete/react';
import { transactionExplorerUrl } from '../config';
import { ORE_STREAM_STACK } from '../generated/ore-stack';

export function RewardsPanel() {
  const arete = useArete(ORE_STREAM_STACK);
  const { publicKey } = useWallet();
  const authority = publicKey?.toBase58();

  // One-shot stack reads expose a React hook: keyed on its arguments and
  // disabled while any argument is missing (e.g. no wallet connected).
  const preview = arete.read.solClaimPreview.use(authority);

  // Claiming SOL takes up to two instructions (checkpoint, then claimSol); the
  // stack composes them into a single transaction based on the preview's action.
  const claim = arete.programs.ore.transactions.rewards.claimSolWithCheckpoint.useMutation();

  const totalSol = preview.data?.totalClaimableSol ?? 0n;
  const action = preview.data?.action;
  // A failed refresh may retain the previous preview. Only a ready preview may
  // authorize a new wallet request.
  const canClaim = Boolean(
    authority
    && arete.isConnected
    && preview.isReady
    && !preview.error
    && action
    && action !== 'none'
    && totalSol > 0n
    && !claim.isLoading
    && claim.reconciliationError === null,
  );
  const submit = () => {
    if (!canClaim || !authority || !action || action === 'none') return;
    claim.mutate(
      { signer: authority, authority, action },
      // Default reconciliation waits for the stream, then refreshes the preview.
      { reconcile: { refresh: preview } },
    );
  };

  const busyLabel = claim.phase === 'awaiting-wallet'
    ? 'Confirm in your wallet…'
    : claim.phase === 'reconciling'
      ? 'Syncing…'
      : 'Claiming…';
  const mutationError = claim.displayError
    ?? (claim.reconciliationError
      ? `Claim confirmed, but live rewards could not be synchronized: ${claim.reconciliationError.message}`
      : null);

  if (totalSol <= 0n) return null;

  return (
    <section className="rounded-2xl border border-sky-200 bg-sky-50 p-4 shadow-sm dark:border-sky-900 dark:bg-sky-950/40" aria-label="Claimable SOL rewards">
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0">
          <p className="text-[11px] font-semibold uppercase tracking-wide text-sky-600 dark:text-sky-400">Claimable SOL</p>
          <p className="mt-1 truncate text-xl font-semibold tabular-nums text-stone-900 dark:text-white">
            {preview.data?.totalClaimableSolUi ?? '0'} SOL
          </p>
        </div>
        <button
          type="button"
          className="min-h-11 flex-none rounded-xl bg-sky-600 px-4 text-sm font-semibold text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-40"
          disabled={!canClaim}
          onClick={submit}
        >
          {!authority
            ? 'Connect wallet'
            : preview.isPending
              ? 'Checking…'
              : claim.isLoading ? busyLabel : 'Claim SOL'}
        </button>
      </div>

      {!authority && (
        <p className="mt-3 text-xs leading-5 text-sky-700 dark:text-sky-300">
          Connect a wallet to check claimable rewards.
        </p>
      )}
      {authority && preview.isPending && (
        <p className="mt-3 text-xs leading-5 text-stone-500 dark:text-stone-400" role="status">
          Checking claimable rewards…
        </p>
      )}
      {authority && preview.isReady && preview.isEmpty && (
        <p className="mt-3 text-xs leading-5 text-stone-500 dark:text-stone-400">
          No miner rewards account is available yet.
        </p>
      )}
      {authority && preview.isReady && !preview.isEmpty && totalSol <= 0n && (
        <p className="mt-3 text-xs leading-5 text-stone-500 dark:text-stone-400">
          No SOL rewards are currently claimable.
        </p>
      )}

      {mutationError && (
        <div className="mt-3 text-xs leading-5 text-red-700 dark:text-red-300" role="alert">
          <p>{mutationError}</p>
          {claim.signature && (
            <a
              className="font-medium underline underline-offset-2"
              href={transactionExplorerUrl(claim.signature)}
              target="_blank"
              rel="noreferrer"
            >
              View {claim.signature.slice(0, 8)}... on explorer
            </a>
          )}
          {claim.canRetryReconciliation && (
            <button
              type="button"
              className="ml-2 font-medium underline underline-offset-2 disabled:opacity-40"
              disabled={claim.isReconciling}
              onClick={() => {
                // The mutation stores retry failures in reconciliationError.
                void claim.retryReconciliation().catch(() => undefined);
              }}
            >
              {claim.isReconciling ? 'Syncing rewards…' : 'Retry synchronization'}
            </button>
          )}
        </div>
      )}

      {preview.error && (
        <p className="mt-3 text-xs leading-5 text-red-700 dark:text-red-300" role="alert">
          Unable to load miner rewards: {preview.error.message}
        </p>
      )}
    </section>
  );
}
