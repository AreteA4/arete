import { useState } from 'react';
import { useWallet } from '@solana/wallet-adapter-react';
import { useWalletModal } from '@solana/wallet-adapter-react-ui';
import { safeToRawAmount, useArete } from '@usearete/react';
import { transactionExplorerUrl } from '../config';
import {
  SQUARE_COUNT,
  SOL_DECIMALS,
  type SquareIndex,
} from '../generated/ore-devex';
import type { ManualDeploymentQuoteReadInput } from '../generated/ore-stack-extensions';
import { ORE_STREAM_STACK } from '../generated/ore-stack';

interface DeploymentPanelProps {
  currentRoundId: bigint | undefined;
  selected: readonly SquareIndex[];
  onDeployed: () => void;
}

function createQuoteInput(
  authority: string | undefined,
  roundId: bigint | undefined,
  totalSol: string,
  selected: readonly SquareIndex[],
): { input: ManualDeploymentQuoteReadInput | null; error: string | null } {
  if (!authority || roundId === undefined || selected.length === 0 || !totalSol.trim()) {
    return { input: null, error: null };
  }
  const totalPrincipal = safeToRawAmount({ ui: totalSol }, SOL_DECIMALS);
  if (!totalPrincipal.success) {
    const error = totalPrincipal.error;
    return { input: null, error: error instanceof Error ? error.message : String(error) };
  }
  return {
    input: {
      authority,
      roundId,
      totalPrincipal: totalPrincipal.data,
      selectedSquares: selected,
    },
    error: null,
  };
}

export function DeploymentPanel({
  currentRoundId,
  selected,
  onDeployed,
}: DeploymentPanelProps) {
  const arete = useArete(ORE_STREAM_STACK);
  const { publicKey } = useWallet();
  const authority = publicKey?.toBase58();
  const { setVisible: showWalletModal } = useWalletModal();

  const [totalSol, setTotalSol] = useState('0.01');
  const { input: quoteInput, error: amountError } = createQuoteInput(
    authority,
    currentRoundId,
    totalSol,
    selected,
  );
  // One-shot reads are keyed on their arguments: when the inputs change, the
  // old quote disappears immediately and `isLoading` covers the refetch, so
  // `quote` always matches the current inputs.
  const quoteRead = arete.read.quoteManualDeployment.use(
    quoteInput,
    { debounceMs: 300 },
  );
  const quote = quoteRead.data ?? null;
  const quoteError = amountError
    ?? (quoteInput === null ? null : quoteRead.error?.message)
    ?? null;
  const quoteBusy = quoteInput !== null
    && !quoteError
    && (quoteRead.isPending || quoteRead.isRefreshing);

  // Every operation on the stack exposes a ready-made, fully typed mutation hook.
  const deploy = arete.programs.ore.transactions.mining.deployWithCheckpoint.useMutation();

  // Retained read data remains visible after a refresh failure, so writes must
  // require the current read to be ready rather than checking data alone.
  const canDeploy = Boolean(
    authority
    && quoteRead.isReady
    && !quoteError
    && quote
    && quote.effectiveSquareCount > 0
    && !quote.requiresDisableBeforeDeployment
    && !quoteBusy
    && deploy.reconciliationError === null
    && arete.isConnected,
  );

  const submit = () => {
    if (!authority) {
      showWalletModal(true);
      return;
    }
    if (!canDeploy || !quote) return;
    deploy.mutate(
      {
        signer: authority,
        roundId: quote.roundId,
        squares: quote.effectiveSquares,
        amountPerSquare: quote.amountPerSquare,
      },
      {
        // After confirmation the hook waits for the stream to process the
        // transaction's slot, then refreshes these targets. View hooks are
        // themselves refresh targets: they refresh that view's active
        // subscriptions (kept alive by OreDashboard), so this panel needs no
        // subscriptions of its own.
        reconcile: {
          refresh: [
            arete.views.OreBoard.state,
            arete.views.OreRound.state,
            arete.views.OreMiner.state,
            quoteRead,
          ],
        },
        // onSuccess runs only after stream reconciliation succeeds.
        onSuccess: onDeployed,
      },
    );
  };

  // `phase` is the mutation's discriminated status — use it for busy labels.
  const busyLabel = deploy.phase === 'awaiting-wallet'
    ? 'Confirm in your wallet…'
    : deploy.phase === 'reconciling'
      ? 'Syncing…'
      : 'Deploying…';
  const mutationError = deploy.displayError
    ?? (deploy.reconciliationError
      ? `Deployment confirmed, but live data could not be synchronized: ${deploy.reconciliationError.message}`
      : null);

  return (
    <section className="rounded-2xl bg-white p-4 shadow-sm dark:bg-stone-800 dark:ring-1 dark:ring-stone-700 lg:p-5" aria-labelledby="deployment-title">
      <div className="mb-3 flex items-start justify-between gap-4 lg:mb-4">
        <h2 id="deployment-title" className="font-semibold text-stone-800 dark:text-stone-100">Deployment</h2>
        <span className="rounded-full bg-stone-100 px-2.5 py-1 text-xs font-medium tabular-nums text-stone-600 dark:bg-stone-700 dark:text-stone-300">
          {selected.length}/{SQUARE_COUNT}
        </span>
      </div>

      <label className="block text-sm font-medium text-stone-700 dark:text-stone-300">
        Total amount to deploy
        <span className="relative mt-2 block">
          <input
            className="min-h-11 w-full rounded-xl border border-stone-200 bg-stone-50 px-3 pr-12 text-base tabular-nums text-stone-900 outline-none transition focus:border-stone-500 focus:ring-2 focus:ring-stone-300 dark:border-stone-700 dark:bg-stone-900 dark:text-stone-100 dark:focus:border-stone-500 dark:focus:ring-stone-700"
            aria-label="Total principal in SOL"
            inputMode="decimal"
            value={totalSol}
            onChange={(event) => {
              if (!deploy.reconciliationError) deploy.reset();
              setTotalSol(event.target.value);
            }}
          />
          <span className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-xs font-semibold text-stone-400">SOL</span>
        </span>
      </label>

      {quoteBusy && (
        <p className="mt-2 text-xs text-stone-500 dark:text-stone-400" aria-live="polite">
          Checking current deployment…
        </p>
      )}
      {quoteError && <p className="mt-2 text-sm text-red-600 dark:text-red-400" role="alert">{quoteError}</p>}
      {quote && (
        <div className="mt-2 space-y-1 text-xs leading-5 text-stone-500 dark:text-stone-400" aria-live="polite">
          <p>
            {quote.effectiveSquareCount} of {quote.requestedSquareCount} selected squares available for this deployment.
          </p>
          {quote.alreadyDeployedSquares.length > 0 && (
            <p>
              Existing positions on {quote.alreadyDeployedSquares.length} selected {quote.alreadyDeployedSquares.length === 1 ? 'square are' : 'squares are'} excluded.
            </p>
          )}
          {quote.checkpointReserve > 0n && (
            <p>
               Includes up to {quote.checkpointReserveUi} SOL for the checkpoint reserve.
            </p>
          )}
        </div>
      )}
      {quote?.hasActiveAutomation && (
        <p className="mt-2 text-sm text-amber-700 dark:text-amber-300" role={quote.requiresDisableBeforeDeployment ? 'alert' : undefined}>
          {quote.requiresDisableBeforeDeployment
            ? 'Disable active automation before deploying manually.'
            : 'Active automation is configured for this miner.'}
        </p>
      )}
      {mutationError && (
        <div className="mt-2 rounded-xl bg-red-50 p-2.5 text-xs leading-5 text-red-700 dark:bg-red-950/40 dark:text-red-300" role="alert">
          <p>{mutationError}</p>
          {deploy.signature && (
            <a
              className="mt-1 inline-block font-medium underline underline-offset-2"
              href={transactionExplorerUrl(deploy.signature)}
              target="_blank"
              rel="noreferrer"
            >
              View {deploy.signature.slice(0, 8)}... on explorer
            </a>
          )}
          {deploy.canRetryReconciliation && (
            <button
              type="button"
              className="ml-2 font-medium underline underline-offset-2 disabled:opacity-40"
              disabled={deploy.isReconciling}
              onClick={() => {
                // The mutation stores retry failures in reconciliationError.
                void deploy.retryReconciliation().catch(() => undefined);
              }}
            >
              {deploy.isReconciling ? 'Syncing deployment…' : 'Retry synchronization'}
            </button>
          )}
        </div>
      )}

      <button
        type="button"
        className="mt-3 min-h-11 w-full rounded-xl bg-stone-800 px-4 text-sm font-semibold text-white transition hover:bg-stone-700 disabled:cursor-not-allowed disabled:opacity-40 dark:bg-stone-100 dark:text-stone-900 dark:hover:bg-white"
        disabled={deploy.isLoading || (Boolean(authority) && !canDeploy)}
        onClick={submit}
      >
        {!authority
          ? 'Connect wallet to continue'
          : deploy.isLoading
            ? busyLabel
            : quoteBusy
              ? 'Checking deployment…'
              : 'Deploy'}
      </button>
    </section>
  );
}
