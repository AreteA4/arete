import { z } from 'zod';
import { OreRoundPatchSchema } from '../generated/ore-stack';

type OreRoundFrame = z.output<typeof OreRoundPatchSchema> & { __seq?: string };
type OreRoundState = NonNullable<OreRoundFrame['state']>;

export type ValidatedOreRound = Omit<OreRoundFrame, '__seq' | 'state'> & {
  sequence?: string;
  state?: Omit<OreRoundState, 'countPerSquare' | 'deployedPerSquare'> & {
    countPerSquare?: number[] | null;
    deployedPerSquare?: number[] | null;
  };
};

function toNumbers(values: bigint[] | null | undefined): number[] | null | undefined {
  if (values == null) return values;
  return values.map(Number);
}

export function toValidatedOreRound(round: OreRoundFrame | undefined): ValidatedOreRound | undefined {
  if (!round) return undefined;

  const { __seq, state, ...rest } = round;
  if (!state) return { ...rest, sequence: __seq };

  const deployedPerSquare = toNumbers(state.deployedPerSquare);

  return {
    ...rest,
    sequence: __seq,
    state: {
      ...state,
      countPerSquare: toNumbers(state.countPerSquare),
      deployedPerSquare,
    },
  };
}
