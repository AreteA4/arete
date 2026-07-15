import { z } from 'zod';
import { OreRoundPatchSchema } from '../generated/ore-stack';

type OreRoundFrame = z.output<typeof OreRoundPatchSchema> & { __seq?: string };

export type ValidatedOreRound = Omit<OreRoundFrame, '__seq'> & {
  sequence?: string;
};

export function toValidatedOreRound(round: OreRoundFrame | undefined): ValidatedOreRound | undefined {
  if (!round) return undefined;

  const { __seq, ...rest } = round;

  return {
    ...rest,
    sequence: __seq,
  };
}
