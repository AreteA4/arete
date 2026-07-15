import type { ValidatedOreRound } from '../schemas/ore-round-validated';
import { MinerIcon, SolanaIcon } from './icons';

interface BlockGridProps {
  round: ValidatedOreRound | undefined;
}

function formatLamports(lamports: bigint) {
  const roundedUnits = (lamports + 50_000n) / 100_000n;
  const whole = roundedUnits / 10_000n;
  const fraction = (roundedUnits % 10_000n).toString().padStart(4, '0');
  return `${whole}.${fraction}`;
}

export function BlockGrid({ round }: BlockGridProps) {
  const blocks = Array.from({ length: 25 }, (_, i) => {
    const deployedUi = round?.state?.deployedPerSquareUi?.[i];
    const deployedLamports = round?.state?.deployedPerSquare?.[i];

    return {
      id: i + 1,
      minerCount: (round?.state?.countPerSquare?.[i] ?? 0n).toString(),
      deployed: deployedUi == null
        ? formatLamports(deployedLamports ?? 0n)
        : Number(deployedUi).toFixed(4),
      isWinner:
        round?.results?.winningSquare === BigInt(i)
        || round?.results?.preRevealWinningSquare === BigInt(i),
    };
  });

  return (
    <div
      className="grid grid-cols-5 gap-2"
      style={{
        height: 'calc(100vh - 120px)',
        width: 'calc((100vh - 120px - 4 * 0.5rem) / 5 * 5 + 4 * 0.5rem)'
      }}
    >
      {blocks.map((block) => (
        <div
          key={block.id}
          style={{ aspectRatio: '1' }}
          className={`
            bg-white dark:bg-stone-800 rounded-2xl p-4 flex flex-col justify-between
            transition-all duration-200 hover:shadow-md dark:hover:bg-stone-750
            ${block.isWinner
              ? 'bg-amber-50 dark:bg-amber-900/30 ring-2 ring-amber-400 shadow-lg'
              : 'shadow-sm dark:shadow-none dark:ring-1 dark:ring-stone-700'
            }
          `}
        >
          <div className="flex justify-between items-start">
            <span className="text-stone-400 dark:text-stone-500 text-sm font-medium">{block.id}</span>
            <div className="flex items-center gap-1.5 text-stone-400 dark:text-stone-500">
              <span className="text-sm">{block.minerCount}</span>
              <MinerIcon />
            </div>
          </div>
          <div className="flex items-center gap-2 text-xl font-semibold text-stone-800 dark:text-stone-100">
            <SolanaIcon size={18} />
            <span>{block.deployed}</span>
          </div>
        </div>
      ))}
    </div>
  );
}
