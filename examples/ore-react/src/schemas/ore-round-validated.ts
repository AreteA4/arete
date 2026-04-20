import { z } from 'zod';
import {
  OreRoundIdSchema,
  OreRoundStateSchema,
  OreRoundResultsSchema,
} from '@usearete/stacks/ore';

const ValidatedOreRoundIdSchema = OreRoundIdSchema.extend({
  round_id: z.number(),
});

const ValidatedOreRoundStateSchema = OreRoundStateSchema.extend({
  expires_at: z.number(),
  estimated_expires_at_unix: z.number().nullable().optional(),
  deployed_per_square_ui: z.array(z.number()).length(25),
  count_per_square: z.array(z.number()).length(25),
  total_deployed: z.number(),
  total_miners: z.number(),
});

export const ValidatedOreRoundSchema = z.object({
  id: ValidatedOreRoundIdSchema,
  state: ValidatedOreRoundStateSchema,
  results: OreRoundResultsSchema.optional(),
});

export type ValidatedOreRound = z.infer<typeof ValidatedOreRoundSchema>;

