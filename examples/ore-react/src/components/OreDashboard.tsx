import { useEffect, useRef, useState } from 'react';
import { useWallet } from '@solana/wallet-adapter-react';
import { useWalletModal } from '@solana/wallet-adapter-react-ui';
import { getTransactionFailureOutcome, useArete } from '@usearete/react';
import {
  createPreparedTransaction,
  formatRawToUi,
  toRawAmount,
  type PreparedOperation,
} from '@usearete/sdk';
import { appConfig, transactionExplorerUrl } from '../config';
import {
  SOL_DECIMALS,
  quoteManualDeployment,
  type ManualDeploymentQuote,
  type SolClaimPreview,
  type SquareIndex,
} from '../generated/ore-devex';
import { ORE_STREAM_STACK, type OreMiner2 } from '../generated/ore-stack';
import { BlockGrid } from './BlockGrid';
import { ConnectionBadge } from './ConnectionBadge';
import { StatsPanel } from './StatsPanel';
import { ThemeToggle } from './ThemeToggle';

interface PreparedDeployment {
  prepared: PreparedOperation;
  quote: ManualDeploymentQuote;
  roundId: bigint;
}

interface TransactionStatus {
  message: string;
  signature?: string;
  tone: 'neutral' | 'error';
}

interface ConfirmedDeployment {
  roundId: bigint;
  amounts: readonly bigint[];
}

interface ClaimStatus {
  message: string;
  signature?: string;
  uncertain?: boolean;
}

const EMPTY_KEY = '__not-connected__';

export function OreDashboard() {
  const wallet = useWallet();
  const { setVisible } = useWalletModal();
  const authority = wallet.publicKey?.toBase58();
  const [selectedSquares, setSelectedSquares] = useState<SquareIndex[]>([]);
  const [totalSol, setTotalSol] = useState('0.01');
  const [formError, setFormError] = useState<string | null>(null);
  const [isPreparing, setIsPreparing] = useState(false);
  const [isExecuting, setIsExecuting] = useState(false);
  const [transactionStatus, setTransactionStatus] = useState<TransactionStatus | null>(null);
  const [confirmedDeployment, setConfirmedDeployment] = useState<ConfirmedDeployment | null>(null);
  const [claimedRewardSnapshot, setClaimedRewardSnapshot] = useState<bigint | null>(null);
  const [claimStatus, setClaimStatus] = useState<ClaimStatus | null>(null);
  const [isClaimingSol, setIsClaimingSol] = useState(false);
  const [minerAccount, setMinerAccount] = useState<OreMiner2 | null>(null);
  const [solClaimPreview, setSolClaimPreview] = useState<SolClaimPreview | null>(null);
  const [rewardLoadError, setRewardLoadError] = useState<string | null>(null);
  const [rewardRefreshNonce, setRewardRefreshNonce] = useState(0);
  const submissionRef = useRef(false);
  const activeRoundIdRef = useRef<bigint | undefined>(undefined);

  const arete = useArete(ORE_STREAM_STACK, {
    url: appConfig.areteWsUrl,
    httpUrl: appConfig.areteHttpUrl,
  });

  const boardAddress = arete.addresses.board();
  const boardView = arete.views.OreBoard.state.use({ address: boardAddress });
  const board = boardView.data;
  const boardRoundId = board?.state?.roundId;
  const latestRoundView = arete.views.OreRound.latest.useOne();
  const minerView = arete.views.OreMiner.state.use(
    { authority: authority ?? EMPTY_KEY },
    { enabled: Boolean(authority) },
  );
  const round = latestRoundView.data;
  const miner = minerView.data;
  const streamedRewardsSol = solClaimPreview?.checkpointedRewardsSol
    ?? miner?.rewards?.rewardsSol
    ?? minerAccount?.rewardsSol
    ?? 0n;
  const pendingRewardsSol = claimedRewardSnapshot !== null
    && claimedRewardSnapshot === streamedRewardsSol
    ? 0n
    : streamedRewardsSol;
  const pendingCheckpointSol = solClaimPreview?.unresolvedRewardsSol ?? 0n;
  const totalPendingSol = pendingRewardsSol + pendingCheckpointSol;
  const solClaimAction = solClaimPreview?.action
    ?? (pendingRewardsSol > 0n ? 'claim' : 'none');
  const liveRoundId = round?.id?.roundId ?? boardRoundId;

  const streamedDeployment = minerAccount?.roundId !== undefined
    && minerAccount.roundId === liveRoundId
    ? minerAccount?.deployed ?? []
    : [];
  const confirmedAmounts = confirmedDeployment && confirmedDeployment.roundId === liveRoundId
    ? confirmedDeployment.amounts
    : [];
  const currentDeployment = Array.from({ length: 25 }, (_, index) => {
    const streamed = streamedDeployment[index] ?? 0n;
    const confirmed = confirmedAmounts[index] ?? 0n;
    return streamed > confirmed ? streamed : confirmed;
  });
  const youDeployed = currentDeployment.reduce((total, amount) => total + amount, 0n);

  let totalPrincipal: bigint | null = null;
  let amountError: string | null = null;
  try {
    totalPrincipal = toRawAmount({ ui: totalSol }, SOL_DECIMALS);
  } catch (error) {
    amountError = error instanceof Error ? error.message : String(error);
  }

  let previewQuote: ManualDeploymentQuote | null = null;
  if (totalPrincipal !== null && selectedSquares.length > 0) {
    try {
      previewQuote = quoteManualDeployment({
        roundId: liveRoundId ?? 0n,
        totalPrincipal,
        selectedSquares,
      });
    } catch (error) {
      amountError = error instanceof Error ? error.message : String(error);
    }
  }

  useEffect(() => {
    if (liveRoundId == null) return;
    const previousRoundId = activeRoundIdRef.current;
    activeRoundIdRef.current = liveRoundId;
    if (previousRoundId !== undefined && previousRoundId !== liveRoundId) {
      setFormError(null);
      minerView.refresh();
      setRewardRefreshNonce((value) => value + 1);
    }
  }, [liveRoundId]);

  useEffect(() => {
    if (!confirmedDeployment) return;
    if (liveRoundId !== confirmedDeployment.roundId) {
      setConfirmedDeployment(null);
      return;
    }
    const isStreamed = confirmedDeployment.amounts.every(
      (amount, index) => amount === 0n || (streamedDeployment[index] ?? 0n) >= amount,
    );
    if (isStreamed) setConfirmedDeployment(null);
  }, [confirmedDeployment, liveRoundId, streamedDeployment]);

  useEffect(() => {
    setClaimedRewardSnapshot(null);
    setClaimStatus(null);
  }, [authority]);

  useEffect(() => {
    const read = arete.read;
    if (!authority || !read) {
      setMinerAccount(null);
      setSolClaimPreview(null);
      setRewardLoadError(null);
      return undefined;
    }

    let cancelled = false;
    void (async () => {
      try {
        const [nextMiner, nextSolClaimPreview] = await Promise.all([
          read.miner(authority),
          read.solClaimPreview(authority),
        ]);
        if (cancelled) return;
        setRewardLoadError(null);
        setMinerAccount(nextMiner);
        setSolClaimPreview(nextSolClaimPreview);
      } catch (error) {
        if (!cancelled) {
          setMinerAccount(null);
          setSolClaimPreview(null);
          setRewardLoadError(error instanceof Error ? error.message : String(error));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [authority, arete.read, liveRoundId, miner?.state?.roundId, rewardRefreshNonce]);

  useEffect(() => {
    if (claimedRewardSnapshot !== null && streamedRewardsSol !== claimedRewardSnapshot) {
      setClaimedRewardSnapshot(null);
      setClaimStatus(null);
    }
  }, [claimedRewardSnapshot, streamedRewardsSol]);

  const toggleSquare = (square: SquareIndex) => {
    setFormError(null);
    setSelectedSquares((current) => current.includes(square)
      ? current.filter((value) => value !== square)
      : [...current, square].sort((left, right) => left - right));
  };

  const deploy = async () => {
    setFormError(null);
    if (!wallet.connected) {
      setVisible(true);
      return;
    }
    if (!authority || !arete.client || !arete.chain || !arete.read || !arete.isConnected) {
      setFormError('Arete must be connected before preparing a deployment.');
      return;
    }
    if (totalPrincipal === null || selectedSquares.length === 0) {
      setFormError(amountError ?? 'Select at least one square and enter a valid principal.');
      return;
    }

    setIsPreparing(true);
    try {
      const boardAccount = await arete.read.board();
      if (!boardAccount) {
        throw new Error('The ORE Board account is unavailable.');
      }
      const [clock, exactQuote] = await Promise.all([
        arete.chain.clock(),
        arete.read.quoteManualDeployment({
          authority,
          roundId: boardAccount.roundId,
          totalPrincipal,
          selectedSquares,
        }),
      ]);
      const phase = arete.math.round.phase(boardAccount, BigInt(clock.slot));
      if (
        phase.kind !== 'active'
        && phase.kind !== 'waitingForFirstDeploy'
      ) {
        throw new Error('The current round is not accepting deployments.');
      }
      if (exactQuote.roundId !== boardAccount.roundId) {
        throw new Error('The board advanced while the deployment quote was loading.');
      }
      if (exactQuote.requiresDisableBeforeDeployment) {
        throw new Error('Active automation must be disabled and reconciled before a manual deployment.');
      }
      if (exactQuote.effectiveSquareCount === 0) {
        throw new Error('The selected squares already have a deployment in this round.');
      }

      const prepared = await arete.programs.ore.transactions.mining.deployWithCheckpoint.prepare({
        signer: authority,
        authority,
        squares: exactQuote.effectiveSquares,
        amountPerSquare: exactQuote.amountPerSquare,
      });
      const artifacts = prepared.artifacts as {
        roundId: bigint;
      };
      if (artifacts.roundId !== exactQuote.roundId) {
        throw new Error('The board advanced while the transaction was being prepared.');
      }

      const inspection = await arete.client.inspectOperation(prepared);
      if (inspection.transaction.error !== undefined) {
        throw new Error(`Deployment simulation failed: ${String(inspection.transaction.error)}`);
      }
      await executeDeployment({
        prepared,
        quote: exactQuote,
        roundId: artifacts.roundId,
      });
    } catch (error) {
      setFormError(error instanceof Error ? error.message : String(error));
    } finally {
      setIsPreparing(false);
    }
  };

  const executeDeployment = async (deployment: PreparedDeployment) => {
    if (!authority || !arete.chain || !arete.client || !arete.read) return;
    if (appConfig.automatedTestMode) {
      setFormError('Transactions are disabled in automated test mode.');
      return;
    }
    if (submissionRef.current) return;
    submissionRef.current = true;
    setFormError(null);
    setIsExecuting(true);
    setTransactionStatus(null);

    let confirmedSignature: string | undefined;
    try {
      const [boardAccount, clock, activeAutomation] = await Promise.all([
        arete.read.board(),
        arete.chain.clock(),
        arete.read.automation(authority),
      ]);
      const phase = boardAccount
        ? arete.math.round.phase(boardAccount, BigInt(clock.slot))
        : null;
      if (
        !boardAccount
        || boardAccount.roundId !== deployment.roundId
        || (liveRoundId !== null && liveRoundId !== undefined && liveRoundId !== deployment.roundId)
      ) {
        throw new Error('The board advanced. The deployment was not submitted.');
      }
      if (
        !phase
        || (phase.kind !== 'active' && phase.kind !== 'waitingForFirstDeploy')
      ) {
        throw new Error('The round stopped accepting deployments. Nothing was submitted.');
      }
      if (activeAutomation) {
        throw new Error('Active automation was detected. Nothing was submitted.');
      }

      // Execute the exact object inspected above; never rebuild or retry it.
      const receipt = await arete.client.execute(deployment.prepared);
      confirmedSignature = receipt.signatures[receipt.signatures.length - 1];
      const confirmedAmounts = Array<bigint>(25).fill(0n);
      for (const square of deployment.quote.effectiveSquares) {
        confirmedAmounts[square] = deployment.quote.amountPerSquare;
      }
      const confirmedSquares = new Set(deployment.quote.effectiveSquares);
      setConfirmedDeployment({ roundId: deployment.roundId, amounts: confirmedAmounts });
      setSelectedSquares((current) => current.filter((square) => !confirmedSquares.has(square)));
      setTransactionStatus(null);
      const transaction = receipt.kind === 'flow'
        ? receipt.transactions[receipt.transactions.length - 1]
        : receipt.transaction;
      const confirmedSlot = transaction?.slot;

      if (confirmedSlot === undefined) {
        boardView.refresh();
        latestRoundView.refresh();
        minerView.refresh();
        setRewardRefreshNonce((value) => value + 1);
        return;
      }

      await arete.client.waitForProcessedSlot(confirmedSlot, {
        timeoutMs: 30_000,
      });
      boardView.refresh();
      latestRoundView.refresh();
      minerView.refresh();
      setRewardRefreshNonce((value) => value + 1);
    } catch (error) {
      const outcome = getTransactionFailureOutcome(error);
      const cause = error instanceof Error ? error.message : String(error);
      if (confirmedSignature) {
        boardView.refresh();
        latestRoundView.refresh();
        minerView.refresh();
        setRewardRefreshNonce((value) => value + 1);
        setTransactionStatus({
          message: `Confirmed, but stream reconciliation is pending: ${cause}`,
          signature: confirmedSignature,
          tone: 'neutral',
        });
      } else if (outcome?.status === 'submitted-unknown') {
        setTransactionStatus({
          message: `Submitted, but confirmation is unknown. It was not retried: ${cause}`,
          signature: outcome.signature,
          tone: 'error',
        });
      } else if (outcome?.status === 'chain-failed') {
        setTransactionStatus({
          message: outcome.programError
            ? `${outcome.programError.name}: ${outcome.programError.message}`
            : `The transaction failed on chain: ${cause}`,
          signature: outcome.signature,
          tone: 'error',
        });
      } else {
        setTransactionStatus({
          message: outcome?.status === 'not-submitted'
            ? `Not submitted during ${outcome.phase}: ${cause}`
            : cause,
          tone: 'error',
        });
      }
    } finally {
      submissionRef.current = false;
      setIsExecuting(false);
    }
  };

  const claimSol = async () => {
    if (!authority || !arete.client || !arete.programs || totalPendingSol <= 0n) return;
    if (appConfig.automatedTestMode) {
      setClaimStatus({ message: 'Transactions are disabled in automated test mode.' });
      return;
    }
    if (isClaimingSol || claimStatus?.uncertain) return;

    setIsClaimingSol(true);
    setClaimStatus(null);
    let confirmed = false;
    try {
      const operations = [];
      if (solClaimAction === 'checkpoint' || solClaimAction === 'checkpointAndClaim') {
        operations.push(
          await arete.programs.ore.instructions.miner.checkpoint.prepare({
            signer: authority,
            authority,
          }),
        );
      }
      if (solClaimAction === 'claim' || solClaimAction === 'checkpointAndClaim') {
        operations.push(
          await arete.programs.ore.instructions.rewards.claimSol.prepare({ authority }),
        );
      }
      const prepared = operations.length === 1
        ? operations[0]!
        : createPreparedTransaction({
          name: 'checkpointAndClaimSol',
          operations,
          artifacts: { authority },
        });
      const inspection = await arete.client.inspectOperation(prepared);
      if (inspection.transaction.error !== undefined) {
        throw new Error(`Claim simulation failed: ${String(inspection.transaction.error)}`);
      }

      const receipt = await arete.client.execute(prepared);
      confirmed = true;
      setClaimedRewardSnapshot(streamedRewardsSol);
      setSolClaimPreview(null);

      if (receipt.transaction.slot !== undefined) {
        await arete.client.waitForProcessedSlot(receipt.transaction.slot, { timeoutMs: 30_000 });
      }
      minerView.refresh();
      setRewardRefreshNonce((value) => value + 1);
    } catch (error) {
      if (confirmed) {
        minerView.refresh();
        return;
      }
      const outcome = getTransactionFailureOutcome(error);
      const message = error instanceof Error ? error.message : String(error);
      if (outcome?.status === 'submitted-unknown') {
        setClaimStatus({
          message: `Claim submitted, but confirmation is unknown. It was not retried: ${message}`,
          signature: outcome.signature,
          uncertain: true,
        });
      } else if (outcome?.status === 'chain-failed') {
        setClaimStatus({
          message: outcome.programError
            ? `${outcome.programError.name}: ${outcome.programError.message}`
            : `The claim failed on chain: ${message}`,
          signature: outcome.signature,
        });
      } else {
        setClaimStatus({ message });
      }
    } finally {
      setIsClaimingSol(false);
    }
  };

  const canDeploy = Boolean(
    previewQuote
    && previewQuote.effectiveSquareCount > 0
    && arete.isConnected,
  );

  return (
    <div className="flex h-dvh flex-col overflow-hidden bg-stone-100 p-3 font-sans text-stone-900 transition-colors dark:bg-stone-900 dark:text-stone-100 sm:p-6">
      <header className="mx-auto mb-3 flex w-full max-w-7xl flex-none items-start justify-between gap-4 sm:mb-6">
        <div>
          <h1 className="text-xl font-semibold text-stone-800 dark:text-stone-100">Ore Mining</h1>
          <p className="mt-1 text-sm text-stone-500 dark:text-stone-400">
            Live ORE rounds powered by{' '}
            <a className="underline underline-offset-2" href="https://docs.arete.run" target="_blank" rel="noreferrer">
              Arete
            </a>
          </p>
        </div>
        <div className="flex items-center gap-2">
          <ConnectionBadge
            isConnected={arete.isConnected}
            connectionState={arete.connectionState}
            error={arete.error}
          />
          <ThemeToggle />
        </div>
      </header>

      <main className="mx-auto grid min-h-0 w-full max-w-7xl flex-1 grid-rows-[minmax(0,1fr)_auto] gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(320px,400px)] lg:grid-rows-1 lg:gap-8">
        <BlockGrid
          deployedPerSquare={round?.state?.deployedPerSquare?.map(String) ?? []}
          countPerSquare={round?.state?.countPerSquare?.map(String) ?? []}
          preRevealWinningSquare={round?.results?.preRevealWinningSquare === null
            || round?.results?.preRevealWinningSquare === undefined
            ? undefined
            : Number(round.results.preRevealWinningSquare)}
          winningSquare={round?.results?.winningSquare === null
            || round?.results?.winningSquare === undefined
            ? undefined
            : Number(round.results.winningSquare)}
          selected={selectedSquares}
          currentDeployment={currentDeployment.map(String)}
          onToggle={toggleSquare}
        />

        <aside className="grid min-w-0 gap-2 lg:content-start lg:gap-5" aria-label="Round statistics and manual deployment">
          <StatsPanel
            totalDeployed={round?.state?.totalDeployed ?? undefined}
            motherlode={round?.treasury?.motherlode ?? undefined}
            estimatedExpiresAtUnix={round?.state?.estimatedExpiresAtUnix === null
              || round?.state?.estimatedExpiresAtUnix === undefined
              ? undefined
              : Number(round.state.estimatedExpiresAtUnix)}
            roundId={(round?.id?.roundId ?? boardRoundId)?.toString()}
            totalMiners={round?.state?.totalMiners?.toString()}
            youDeployed={youDeployed > 0n
              ? formatRawToUi(youDeployed, SOL_DECIMALS)
              : undefined}
          />

          {(totalPendingSol > 0n || isClaimingSol || claimStatus || rewardLoadError) && (
            <section className="rounded-2xl border border-sky-200 bg-sky-50 p-4 shadow-sm dark:border-sky-900 dark:bg-sky-950/40" aria-label="Claimable SOL rewards">
              <div className="flex items-center justify-between gap-4">
                <div className="min-w-0">
                  <p className="text-[11px] font-semibold uppercase tracking-wide text-sky-600 dark:text-sky-400">Claimable SOL</p>
                  <p className="mt-1 truncate text-xl font-semibold tabular-nums text-stone-900 dark:text-white">
                    {formatRawToUi(totalPendingSol, SOL_DECIMALS)} SOL
                  </p>
                </div>
                <button
                  type="button"
                  className="min-h-11 flex-none rounded-xl bg-sky-600 px-4 text-sm font-semibold text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-40"
                  disabled={totalPendingSol <= 0n || isClaimingSol || Boolean(claimStatus?.uncertain)}
                  onClick={() => void claimSol()}
                >
                  {isClaimingSol ? 'Claiming...' : 'Claim SOL'}
                </button>
              </div>
              {claimStatus && (
                <div className="mt-3 text-xs leading-5 text-red-700 dark:text-red-300" role="alert">
                  <p>{claimStatus.message}</p>
                  {claimStatus.signature && (
                    <a
                      className="font-medium underline underline-offset-2"
                      href={transactionExplorerUrl(appConfig, claimStatus.signature)}
                      target="_blank"
                      rel="noreferrer"
                    >
                      View {claimStatus.signature.slice(0, 8)}... on explorer
                    </a>
                  )}
                </div>
              )}
              {rewardLoadError && (
                <p className="mt-3 text-xs leading-5 text-red-700 dark:text-red-300" role="alert">
                  Unable to load miner rewards: {rewardLoadError}
                </p>
              )}
            </section>
          )}

          <section className="rounded-2xl bg-white p-4 shadow-sm dark:bg-stone-800 dark:ring-1 dark:ring-stone-700 lg:p-5" aria-labelledby="deployment-title">
            <div className="mb-3 flex items-start justify-between gap-4 lg:mb-4">
              <div>
                <h2 id="deployment-title" className="font-semibold text-stone-800 dark:text-stone-100">Deployment</h2>
              </div>
              <span className="rounded-full bg-stone-100 px-2.5 py-1 text-xs font-medium tabular-nums text-stone-600 dark:bg-stone-700 dark:text-stone-300">
                {selectedSquares.length}/25
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
                    setFormError(null);
                    setTotalSol(event.target.value);
                  }}
                />
                <span className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-xs font-semibold text-stone-400">SOL</span>
              </span>
            </label>

            {amountError && <p className="mt-2 text-sm text-red-600 dark:text-red-400" role="alert">{amountError}</p>}
            {formError && <p className="mt-2 text-sm text-red-600 dark:text-red-400" role="alert">{formError}</p>}

            <button
              type="button"
              className="mt-3 min-h-11 w-full rounded-xl bg-stone-800 px-4 text-sm font-semibold text-white transition hover:bg-stone-700 disabled:cursor-not-allowed disabled:opacity-40 dark:bg-stone-100 dark:text-stone-900 dark:hover:bg-white"
              disabled={wallet.connected && (!canDeploy || isPreparing || isExecuting)}
              onClick={() => void deploy()}
            >
              {!wallet.connected
                ? 'Connect wallet to continue'
                : isPreparing || isExecuting
                  ? 'Deploying...'
                  : 'Deploy'}
            </button>

            {transactionStatus && (
              <div
                className={`mt-2 rounded-xl p-2.5 text-xs leading-5 ${transactionStatus.tone === 'error'
                  ? 'bg-red-50 text-red-700 dark:bg-red-950/40 dark:text-red-300'
                  : 'bg-stone-100 text-stone-600 dark:bg-stone-700 dark:text-stone-300'
                  }`}
                aria-live="polite"
              >
                <p>{transactionStatus.message}</p>
                {transactionStatus.signature && (
                  <a
                    className="mt-1 inline-block font-medium underline underline-offset-2"
                    href={transactionExplorerUrl(appConfig, transactionStatus.signature)}
                    target="_blank"
                    rel="noreferrer"
                  >
                    View {transactionStatus.signature.slice(0, 8)}... on explorer
                  </a>
                )}
              </div>
            )}
          </section>
        </aside>
      </main>

    </div>
  );
}
