import { useState, useEffect } from 'react';
import type { ValidatedOreRound } from '../schemas/ore-round-validated';
import { OreIcon, SolanaIcon } from './icons';

interface StatsPanelProps {
  round: ValidatedOreRound | undefined;
  treasuryMotherlode: bigint | number | null | undefined;
  isConnected: boolean;
}

export function StatsPanel({ round, treasuryMotherlode, isConnected }: StatsPanelProps) {
  const [timeRemaining, setTimeRemaining] = useState<string>('00:00');

  useEffect(() => {
    const expiresAtUnix = round?.state?.estimatedExpiresAtUnix;
    if (!expiresAtUnix) {
      setTimeRemaining('00:00');
      return;
    }

    const updateTimer = () => {
      const now = BigInt(Math.floor(Date.now() / 1000));
      const remaining = expiresAtUnix > now ? expiresAtUnix - now : 0n;

      if (remaining > 300n) {
        setTimeRemaining('00:00');
        return;
      }

      const minutes = Number(remaining / 60n);
      const seconds = Number(remaining % 60n);
      setTimeRemaining(`${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`);
    };

    updateTimer();
    const interval = setInterval(updateTimer, 1000);
    return () => clearInterval(interval);
  }, [round?.state?.estimatedExpiresAtUnix]);

  return (
    <div className="flex flex-col gap-6 h-full">
      <div className="bg-white dark:bg-stone-800 rounded-2xl p-8 shadow-sm dark:shadow-none dark:ring-1 dark:ring-stone-700">
        <div className="flex items-center gap-3 text-5xl font-bold text-stone-800 dark:text-stone-100">
          <OreIcon />
          <span>{treasuryMotherlode != null ? treasuryMotherlode.toString() : '–'}</span>
        </div>
        <div className="text-base text-stone-500 dark:text-stone-400 mt-2">Motherlode</div>
      </div>

      <div className="bg-white dark:bg-stone-800 rounded-2xl p-8 shadow-sm dark:shadow-none dark:ring-1 dark:ring-stone-700">
        <div className="text-5xl font-semibold text-stone-800 dark:text-stone-100 tabular-nums">{timeRemaining}</div>
        <div className="text-base text-stone-500 dark:text-stone-400 mt-2">Time remaining</div>
      </div>

      <div className="grid grid-cols-2 gap-4">
        <div className="bg-white dark:bg-stone-800 rounded-2xl p-6 shadow-sm dark:shadow-none dark:ring-1 dark:ring-stone-700">
          <div className="flex items-center gap-2 text-2xl font-semibold text-stone-800 dark:text-stone-100">
            <SolanaIcon size={20} />
            <span>{round ? round.state?.totalDeployed?.toFixed(4) : '0.0000'}</span>
          </div>
          <div className="text-base text-stone-500 dark:text-stone-400 mt-2">Total deployed</div>
        </div>
        <div className="bg-white dark:bg-stone-800 rounded-2xl p-6 shadow-sm dark:shadow-none dark:ring-1 dark:ring-stone-700">
          <div className="flex items-center gap-2 text-2xl font-semibold text-stone-800 dark:text-stone-100">
            <SolanaIcon size={20} />
            <span>0</span>
          </div>
          <div className="text-base text-stone-500 dark:text-stone-400 mt-2">You deployed</div>
        </div>
      </div>

      <div className="flex items-center gap-4 px-2 text-base text-stone-500 dark:text-stone-400 mt-auto">
        <span>Round {round?.id?.roundId?.toString() ?? '–'}</span>
        {round && (
          <>
            <span className="text-stone-300 dark:text-stone-600">·</span>
            <span>{round.state?.totalMiners?.toString() ?? '0'} miners</span>
          </>
        )}
      </div>

      {!isConnected && (
        <div className="bg-amber-50 dark:bg-amber-900/30 text-amber-700 dark:text-amber-400 p-5 rounded-xl text-center">
          Connecting...
        </div>
      )}
    </div>
  );
}
