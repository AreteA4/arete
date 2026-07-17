import { useEffect, useRef, useState } from 'react';
import { OreIcon, SolanaIcon } from './icons';

interface StatsPanelProps {
  totalDeployed?: number;
  motherlode?: number;
  estimatedExpiresAtUnix?: number;
  roundId?: string;
  totalMiners?: string;
  youDeployed?: string;
}

export function StatsPanel({
  totalDeployed,
  motherlode,
  estimatedExpiresAtUnix,
  roundId,
  totalMiners,
  youDeployed,
}: StatsPanelProps) {
  const [remainingSeconds, setRemainingSeconds] = useState<number>();
  const activeRoundIdRef = useRef(roundId);
  const acceptedExpiresAtUnixRef = useRef<number | undefined>(undefined);

  useEffect(() => {
    const previousRoundId = activeRoundIdRef.current;
    const roundChanged = roundId !== undefined
      && previousRoundId !== undefined
      && roundId !== previousRoundId;

    if (roundId !== undefined) {
      activeRoundIdRef.current = roundId;
    }
    if (roundChanged) {
      acceptedExpiresAtUnixRef.current = undefined;
    }

    const hasIncomingEstimate = estimatedExpiresAtUnix !== undefined
      && estimatedExpiresAtUnix > 0;
    const previousAcceptedExpiresAtUnix = acceptedExpiresAtUnixRef.current;
    const isFirstEstimateForRound = previousAcceptedExpiresAtUnix === undefined
      && hasIncomingEstimate;

    if (hasIncomingEstimate) {
      acceptedExpiresAtUnixRef.current = previousAcceptedExpiresAtUnix === undefined
        ? estimatedExpiresAtUnix
        : Math.min(previousAcceptedExpiresAtUnix, estimatedExpiresAtUnix);
    }

    const acceptedExpiresAtUnix = acceptedExpiresAtUnixRef.current;
    if (acceptedExpiresAtUnix === undefined) {
      setRemainingSeconds((current) => current === 0 ? 0 : undefined);
      return undefined;
    }

    const calculateRemaining = () => Math.max(
      0,
      acceptedExpiresAtUnix - Math.floor(Date.now() / 1_000),
    );
    const update = () => {
      const remaining = calculateRemaining();
      setRemainingSeconds((current) => current === undefined
        ? remaining
        : Math.min(current, remaining));
    };

    if (isFirstEstimateForRound) {
      setRemainingSeconds(calculateRemaining());
    } else {
      update();
    }
    const timer = window.setInterval(update, 1_000);
    return () => window.clearInterval(timer);
  }, [estimatedExpiresAtUnix, roundId]);

  const minutes = Math.floor((remainingSeconds ?? 0) / 60);
  const seconds = (remainingSeconds ?? 0) % 60;
  const timeRemaining = `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;
  const isUrgent = remainingSeconds !== undefined && remainingSeconds <= 10;

  return (
    <section className="rounded-2xl bg-white p-3 shadow-sm dark:bg-stone-800 dark:ring-1 dark:ring-stone-700 lg:p-4" aria-label="Current round statistics">
      <div className="grid grid-cols-3">
        <div className="min-w-0 pr-2 text-center">
          <div className="flex items-center justify-center gap-1.5 text-lg font-semibold tabular-nums text-stone-800 dark:text-stone-100 lg:text-2xl">
            <SolanaIcon size={18} />
            <span className="truncate">{totalDeployed?.toFixed(2) ?? '0.00'}</span>
          </div>
          <p className="mt-1 text-[10px] font-medium uppercase tracking-wide text-stone-400">Deployed</p>
        </div>

        <div className="min-w-0 border-l border-stone-200 px-2 text-center dark:border-stone-700">
          <div className="flex items-center justify-center gap-1.5 text-lg font-semibold tabular-nums text-amber-600 dark:text-amber-400 lg:text-2xl">
            <OreIcon />
            <span className="truncate">{motherlode?.toLocaleString() ?? '–'}</span>
          </div>
          <p className="mt-1 text-[10px] font-medium uppercase tracking-wide text-stone-400">Motherlode</p>
        </div>

        <div className="min-w-0 border-l border-stone-200 pl-2 text-center dark:border-stone-700">
          <p
            className={`truncate text-lg font-semibold tabular-nums lg:text-2xl ${
              isUrgent ? 'text-red-500 dark:text-red-400' : 'text-stone-800 dark:text-white'
            }`}
          >
            {timeRemaining}
          </p>
          <p className="mt-1 text-[10px] font-medium uppercase tracking-wide text-stone-400">Time</p>
        </div>
      </div>

      <div className="mt-3 flex items-center justify-between gap-3 border-t border-stone-100 pt-3 text-xs text-stone-500 dark:border-stone-700 dark:text-stone-400">
        {youDeployed && (
          <span
            className="group relative inline-flex min-w-0 items-center gap-1.5 rounded-full border border-blue-500/50 bg-blue-500/10 px-2.5 py-1 font-semibold tabular-nums text-blue-600 outline-none dark:text-blue-400"
            aria-label={`${youDeployed} SOL deployed by you this round`}
            tabIndex={0}
          >
            <SolanaIcon size={15} />
            <span className="truncate">{youDeployed}</span>
            <span
              className="pointer-events-none absolute bottom-full left-0 z-10 mb-2 w-max max-w-52 rounded-lg bg-stone-900 px-2.5 py-1.5 text-[11px] font-medium normal-case tracking-normal text-white opacity-0 shadow-lg transition-opacity group-hover:opacity-100 group-focus:opacity-100 dark:bg-stone-100 dark:text-stone-900"
              role="tooltip"
            >
              Total deployed by you this round
            </span>
          </span>
        )}
        <span className="ml-auto truncate text-right">
          Round {roundId ?? '–'} · {totalMiners ?? '0'} miners
        </span>
      </div>
    </section>
  );
}
