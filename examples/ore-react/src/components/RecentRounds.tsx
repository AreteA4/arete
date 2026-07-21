import { useArete } from '@usearete/react';
import { ORE_STREAM_STACK } from '../generated/ore-stack';

// `OreRound/latest` is a sorted history view: right for recent-round lists,
// but never for choosing the active round — that's the Board's job, because
// history delivery can briefly lag Board rollover.
export function RecentRounds() {
  const arete = useArete(ORE_STREAM_STACK);
  const rounds = arete.views.OreRound.latest.use({ take: 8 });
  const recentRounds = rounds.data ?? [];

  return (
    <section className="rounded-2xl bg-white p-4 shadow-sm dark:bg-stone-800 dark:ring-1 dark:ring-stone-700" aria-label="Recent rounds">
      <h2 className="mb-2 font-semibold text-stone-800 dark:text-stone-100">Recent rounds</h2>
      {rounds.isLoading ? (
        <p className="text-xs text-stone-500 dark:text-stone-400" role="status">Loading recent rounds…</p>
      ) : rounds.error ? (
        <div className="text-xs text-red-700 dark:text-red-300" role="alert">
          <p>Unable to load recent rounds: {rounds.error.message}</p>
          {/* refresh rejects for imperative composition; rounds.error owns
              failure presentation for this event handler. */}
          <button
            type="button"
            className="mt-1 font-medium underline underline-offset-2 disabled:opacity-40"
            disabled={rounds.isRefreshing}
            onClick={() => void rounds.refresh().catch(() => undefined)}
          >
            {rounds.isRefreshing ? 'Retrying…' : 'Retry recent rounds'}
          </button>
        </div>
      ) : rounds.isEmpty ? (
        <p className="text-xs text-stone-500 dark:text-stone-400">No recent rounds yet.</p>
      ) : (
        <ul className="divide-y divide-stone-100 text-xs tabular-nums dark:divide-stone-700">
          {recentRounds.map((round) => {
            const roundId = round.id.roundId?.toString() ?? '–';
            const winner = round.results.winningSquare;
            const total = round.state.totalDeployed;
            return (
              <li key={roundId} className="flex items-center justify-between gap-2 py-1.5">
                <span className="font-medium text-stone-700 dark:text-stone-200">Round {roundId}</span>
                <span className="text-stone-400 dark:text-stone-500">
                  {winner != null ? `Square ${Number(winner) + 1} won` : 'In progress'}
                </span>
                <span className="text-stone-500 dark:text-stone-400">
                  {total != null ? `${total.toFixed(2)} SOL` : '–'}
                </span>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
