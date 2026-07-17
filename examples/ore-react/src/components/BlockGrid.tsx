import { formatRawToUi } from '@usearete/sdk';
import type { SquareIndex } from '../generated/ore-devex';
import { MinerIcon, SolanaIcon } from './icons';

interface BlockGridProps {
  deployedPerSquare: readonly string[];
  countPerSquare: readonly string[];
  preRevealWinningSquare?: number;
  winningSquare?: number;
  selected: readonly SquareIndex[];
  currentDeployment: readonly string[];
  onToggle: (square: SquareIndex) => void;
}

export function BlockGrid({
  deployedPerSquare,
  countPerSquare,
  preRevealWinningSquare,
  winningSquare,
  selected,
  currentDeployment,
  onToggle,
}: BlockGridProps) {
  const selectedSet = new Set<number>(selected);
  return (
    <section className="ore-board-container min-h-0 w-full" aria-label="ORE mining board">
      <div className="ore-board-grid grid grid-cols-5 gap-1.5 sm:gap-2" role="group" aria-label="ORE squares 1 through 25">
        {Array.from({ length: 25 }, (_, index) => {
          const square = index as SquareIndex;
          const isSelected = selectedSet.has(index);
          const mine = BigInt(currentDeployment[index] ?? '0');
          const isMine = mine > 0n;
          const isCandidate = preRevealWinningSquare === index;
          const isWinner = winningSquare === index;
          const deployed = BigInt(deployedPerSquare[index] ?? '0');
          const miners = countPerSquare[index] ?? '0';
          const deployedText = formatRawToUi(deployed, 9);
          const mineText = formatRawToUi(mine, 9);
          const displayedAmount = deployed === 0n ? '0' : Number(deployedText).toFixed(4);
          const displayedMine = mine === 0n ? '0' : Number(mineText).toFixed(4);
          const highlightClass = isWinner
            ? 'bg-emerald-50 outline outline-[3px] outline-offset-[-3px] outline-emerald-500 shadow-lg dark:bg-emerald-900/30 dark:outline-emerald-400'
            : isCandidate
              ? 'bg-amber-50 outline outline-[3px] outline-offset-[-3px] outline-amber-400 dark:bg-amber-900/20 dark:outline-amber-400'
              : isMine
                ? 'bg-sky-100 outline outline-2 outline-offset-[-2px] outline-sky-500 dark:bg-sky-950/60 dark:outline-sky-400'
                : isSelected
                  ? 'outline outline-2 outline-offset-[-2px] outline-stone-700 dark:outline-stone-200'
                  : '';
          const stateText = [
            isSelected ? 'selected' : '',
            isMine ? 'your current position' : '',
            isCandidate ? 'pre-reveal candidate' : '',
            isWinner ? 'finalized winner' : '',
          ].filter(Boolean).join(', ');
          return (
            <button
              type="button"
              className={`relative flex aspect-square min-h-11 min-w-0 flex-col justify-between overflow-hidden rounded-xl bg-white p-2 text-left shadow-sm transition hover:-translate-y-0.5 hover:shadow-md dark:bg-stone-800 dark:shadow-none dark:ring-1 dark:ring-stone-700 sm:rounded-2xl sm:p-4 ${highlightClass}`}
              key={index}
              aria-pressed={isSelected}
              aria-label={`Square ${index + 1}, protocol index ${index}, ${deployedText} SOL total${isMine ? `, ${mineText} SOL yours` : ''}, ${miners} miners${stateText ? `, ${stateText}` : ''}`}
              data-testid={`tile-${index + 1}`}
              data-winning-state={isWinner ? 'final' : isCandidate ? 'candidate' : undefined}
              onClick={() => onToggle(square)}
            >
              <span className="flex items-start justify-between gap-1 text-[10px] font-medium text-stone-400 dark:text-stone-500 sm:text-sm">
                <span>{index + 1}</span>
                <span className="flex items-center gap-1">
                  {miners}
                  <MinerIcon />
                </span>
              </span>
              <span className="grid min-w-0 gap-0.5">
                {isMine && (
                  <span className="flex min-w-0 items-center gap-1 text-[8px] font-semibold tabular-nums text-sky-600 dark:text-sky-400 sm:text-xs lg:text-sm">
                    <SolanaIcon size={14} />
                    <span className="min-w-0 truncate">{displayedMine}</span>
                  </span>
                )}
                <span className="flex min-w-0 items-center gap-1 text-[8px] font-semibold tabular-nums text-stone-700 dark:text-stone-100 sm:text-xs lg:text-sm">
                  <SolanaIcon size={14} />
                  <span className="min-w-0 truncate">{displayedAmount}</span>
                </span>
              </span>
            </button>
          );
        })}
      </div>
    </section>
  );
}
