import { useEffect, useState, type ReactNode } from 'react';
import { useWallet } from '@solana/wallet-adapter-react';
import { summarizeStatuses, useArete } from '@usearete/react';
import type { SquareIndex } from '../generated/ore-devex';
import { ORE_STREAM_STACK } from '../generated/ore-stack';
import { BlockGrid } from './BlockGrid';
import { ConnectionBadge } from './ConnectionBadge';
import { DeploymentPanel } from './DeploymentPanel';
import { RecentRounds } from './RecentRounds';
import { RewardsPanel } from './RewardsPanel';
import { StatsPanel } from './StatsPanel';
import { ThemeToggle } from './ThemeToggle';

export function OreDashboard() {
  // Repeating the generated stack at each call keeps the data dependency
  // explicit; the provider still shares one client for this stack.
  const arete = useArete(ORE_STREAM_STACK);
  const { publicKey } = useWallet();
  const authority = publicKey?.toBase58();

  // The Board is the authority for the active round: follow its roundId and
  // key the Round subscription with it. Keyed hooks only return data for the
  // key you passed, so a stale round is never rendered during rollover.
  const board = arete.views.OreBoard.state.use({ address: arete.addresses.board() });
  const currentRoundId = board.data?.state.roundId ?? undefined;
  const round = arete.views.OreRound.state.use(
    currentRoundId === undefined ? undefined : { roundId: currentRoundId },
  );
  const treasury = arete.views.OreTreasury.state.use({ address: arete.addresses.treasury() });
  // Passing undefined disables the subscription while no wallet is connected.
  const miner = arete.views.OreMiner.state.use(
    authority ? { authority } : undefined,
  );
  // A miner's deployment only counts for the round it was made in.
  const currentMinerState = currentRoundId !== undefined
    && miner.data?.state.roundId === currentRoundId
    ? miner.data.state
    : undefined;

  // Derive plain display values once so the JSX stays focused on presentation.
  const roundState = round.data?.state;
  const roundResults = round.data?.results;
  const deployedPerSquare = roundState?.deployedPerSquareUi ?? undefined;
  const countPerSquare = roundState?.countPerSquare?.map(Number);
  const myDeploymentPerSquare = currentMinerState?.deployedPerSquareUi ?? undefined;
  const myDeploymentTotal = currentMinerState?.totalDeployed ?? undefined;
  const preRevealWinningSquare = roundResults?.preRevealWinningSquare == null
    ? undefined
    : Number(roundResults.preRevealWinningSquare);
  const winningSquare = roundResults?.winningSquare == null
    ? undefined
    : Number(roundResults.winningSquare);
  const roundId = round.data?.id.roundId?.toString();
  const estimatedExpiresAtUnix = roundState?.estimatedExpiresAtUnix == null
    ? undefined
    : Number(roundState.estimatedExpiresAtUnix);
  const motherlode = treasury.data?.state?.motherlode
    ?? round.data?.treasury?.motherlode
    ?? undefined;
  const totalMiners = roundState?.totalMiners?.toString();

  // Normal loading and refresh transitions stay inline in the controls instead
  // of mounting banners that shift the dashboard. Only actionable failures are
  // surfaced globally.
  const streams = summarizeStatuses({
    Connection: arete,
    Board: board,
    Round: round,
    Treasury: treasury,
    Miner: miner,
  });
  const boardUnavailable = board.isEmpty ? 'Board state is unavailable.' : null;
  const activeRoundUnavailable = board.isReady
    && !board.isEmpty
    && currentRoundId === undefined
    ? 'Board state does not identify an active round.'
    : null;
  const roundUnavailable = currentRoundId !== undefined && round.isEmpty
    ? `Round ${currentRoundId} is not indexed yet.`
    : null;
  const unavailable = [boardUnavailable, activeRoundUnavailable, roundUnavailable]
    .filter((message): message is string => message !== null);

  const [selected, setSelected] = useState<SquareIndex[]>([]);
  // Round rollover clears only the local deployment form selection.
  useEffect(() => setSelected([]), [currentRoundId]);
  const toggleSquare = (square: SquareIndex) =>
    setSelected((current) =>
      current.includes(square)
        ? current.filter((s) => s !== square)
        : [...current, square].sort((a, b) => a - b),
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
          <ConnectionBadge />
          <ThemeToggle />
        </div>
      </header>

      {streams.errors.length > 0 && (
        <StatusBanner role="alert" tone="error">
          Live data error: {streams.errors.join(' · ')}
        </StatusBanner>
      )}
      {unavailable.length > 0 && (
        <StatusBanner tone="warning">
          {unavailable.join(' ')}
        </StatusBanner>
      )}
      {arete.socketIssue && (
        <StatusBanner role="alert" tone="warning">
          Stream issue: {arete.socketIssue.message}
        </StatusBanner>
      )}

      <main className="mx-auto grid min-h-0 w-full max-w-7xl flex-1 grid-rows-[minmax(0,1fr)_auto] gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(320px,400px)] lg:grid-rows-1 lg:gap-8">
        <BlockGrid
          deployedPerSquare={deployedPerSquare}
          countPerSquare={countPerSquare}
          myDeployment={myDeploymentPerSquare}
          preRevealWinningSquare={preRevealWinningSquare}
          winningSquare={winningSquare}
          selected={selected}
          onToggle={toggleSquare}
        />
        <aside className="grid min-w-0 gap-2 lg:content-start lg:gap-5" aria-label="Round statistics and manual deployment">
          <StatsPanel
            roundId={roundId}
            estimatedExpiresAtUnix={estimatedExpiresAtUnix}
            totalDeployed={roundState?.totalDeployed ?? undefined}
            // The dedicated treasury stream is the freshest motherlode source;
            // the round stream embeds a copy to fall back to while it loads.
            motherlode={motherlode}
            totalMiners={totalMiners}
            myDeploymentTotal={myDeploymentTotal}
          />
          <RewardsPanel />
          <DeploymentPanel
            currentRoundId={currentRoundId}
            selected={selected}
            onDeployed={() => setSelected([])}
          />
          <RecentRounds />
        </aside>
      </main>
    </div>
  );
}

const statusToneClasses = {
  warning: 'bg-amber-50 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300',
  error: 'bg-red-50 text-red-700 dark:bg-red-950/40 dark:text-red-300',
} as const;

function StatusBanner({
  children,
  role = 'status',
  tone,
}: {
  children: ReactNode;
  role?: 'alert' | 'status';
  tone: keyof typeof statusToneClasses;
}) {
  return (
    <p
      className={`mx-auto mb-3 w-full max-w-7xl flex-none rounded-xl px-3 py-2 text-xs ${statusToneClasses[tone]}`}
      role={role}
    >
      {children}
    </p>
  );
}
