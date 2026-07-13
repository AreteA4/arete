import { z } from 'zod';
import {
  OreRoundEntropySchema,
  OreRoundMetricsSchema,
  OreRoundResultsSchema,
  OreRoundTreasurySchema,
  TokenMetadataSchema,
} from '../generated/ore-stack';

const ValidatedOreRoundIdSchema = z.object({
  roundAddress: z.string().nullable().optional(),
  roundId: z.bigint().nullable().optional(),
});

const ValidatedOreRoundStateSchema = z.object({
  countPerSquare: z.array(z.number()).length(25).nullable().optional(),
  deployedPerSquare: z.array(z.any()).nullable().optional(),
  deployedPerSquareUi: z.array(z.number()).length(25).nullable().optional(),
  estimatedExpiresAtUnix: z.bigint().nullable().optional(),
  expiresAt: z.bigint().nullable().optional(),
  motherlode: z.number().nullable().optional(),
  totalDeployed: z.number().nullable().optional(),
  totalMiners: z.bigint().nullable().optional(),
  totalVaulted: z.number().nullable().optional(),
  totalWinnings: z.number().nullable().optional(),
});

export const ValidatedOreRoundSchema = z.object({
  id: ValidatedOreRoundIdSchema,
  state: ValidatedOreRoundStateSchema,
  entropy: OreRoundEntropySchema.optional(),
  metrics: OreRoundMetricsSchema.optional(),
  results: OreRoundResultsSchema.optional(),
  treasury: OreRoundTreasurySchema.optional(),
  oreMetadata: TokenMetadataSchema.nullable().optional(),
});

export type ValidatedOreRound = z.infer<typeof ValidatedOreRoundSchema>;
