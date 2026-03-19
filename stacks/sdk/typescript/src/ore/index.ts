import { z } from 'zod';

export interface OreRoundEntropy {
  entropy_end_at?: number | null;
  entropy_samples?: number | null;
  entropy_seed?: string | null;
  entropy_slot_hash?: string | null;
  entropy_start_at?: number | null;
  entropy_value?: string | null;
  entropy_var_address?: string | null;
  resolved_seed?: any[] | null;
}

export interface OreRoundId {
  round_address?: string | null;
  round_id?: number | null;
}

export interface OreRoundMetrics {
  checkpoint_count?: number | null;
  deploy_count?: number | null;
}

export interface OreRoundResults {
  did_hit_motherlode?: boolean | null;
  expires_at_slot_hash?: any[] | null;
  pre_reveal_rng?: number | null;
  pre_reveal_winning_square?: number | null;
  rent_payer?: string | null;
  rng?: number | null;
  slot_hash?: string | null;
  top_miner?: string | null;
  top_miner_reward?: number | null;
  winning_square?: number | null;
}

export interface OreRoundState {
  count_per_square?: any[] | null;
  deployed_per_square?: any[] | null;
  deployed_per_square_ui?: any[] | null;
  estimated_expires_at_unix?: number | null;
  expires_at?: number | null;
  motherlode?: number | null;
  total_deployed?: number | null;
  total_miners?: number | null;
  total_vaulted?: number | null;
  total_winnings?: number | null;
}

export interface OreRoundTreasury {
  motherlode?: number | null;
}

export interface OreRound {
  entropy?: OreRoundEntropy;
  id?: OreRoundId;
  metrics?: OreRoundMetrics;
  results?: OreRoundResults;
  state?: OreRoundState;
  treasury?: OreRoundTreasury;
  ore_metadata?: TokenMetadata | null;
}

export interface TokenMetadata {
  mint: string;
  name?: string | null;
  symbol?: string | null;
  decimals?: number | null;
  logo_uri?: string | null;
}

export const TokenMetadataSchema = z.object({
  mint: z.string(),
  name: z.string().nullable().optional(),
  symbol: z.string().nullable().optional(),
  decimals: z.number().nullable().optional(),
  logo_uri: z.string().nullable().optional(),
});

export const OreRoundEntropySchema = z.object({
  entropy_end_at: z.number().nullable().optional(),
  entropy_samples: z.number().nullable().optional(),
  entropy_seed: z.string().nullable().optional(),
  entropy_slot_hash: z.string().nullable().optional(),
  entropy_start_at: z.number().nullable().optional(),
  entropy_value: z.string().nullable().optional(),
  entropy_var_address: z.string().nullable().optional(),
  resolved_seed: z.array(z.any()).nullable().optional(),
});

export const OreRoundIdSchema = z.object({
  round_address: z.string().nullable().optional(),
  round_id: z.number().nullable().optional(),
});

export const OreRoundMetricsSchema = z.object({
  checkpoint_count: z.number().nullable().optional(),
  deploy_count: z.number().nullable().optional(),
});

export const OreRoundResultsSchema = z.object({
  did_hit_motherlode: z.boolean().nullable().optional(),
  expires_at_slot_hash: z.array(z.any()).nullable().optional(),
  pre_reveal_rng: z.number().nullable().optional(),
  pre_reveal_winning_square: z.number().nullable().optional(),
  rent_payer: z.string().nullable().optional(),
  rng: z.number().nullable().optional(),
  slot_hash: z.string().nullable().optional(),
  top_miner: z.string().nullable().optional(),
  top_miner_reward: z.number().nullable().optional(),
  winning_square: z.number().nullable().optional(),
});

export const OreRoundStateSchema = z.object({
  count_per_square: z.array(z.any()).nullable().optional(),
  deployed_per_square: z.array(z.any()).nullable().optional(),
  deployed_per_square_ui: z.array(z.any()).nullable().optional(),
  estimated_expires_at_unix: z.number().nullable().optional(),
  expires_at: z.number().nullable().optional(),
  motherlode: z.number().nullable().optional(),
  total_deployed: z.number().nullable().optional(),
  total_miners: z.number().nullable().optional(),
  total_vaulted: z.number().nullable().optional(),
  total_winnings: z.number().nullable().optional(),
});

export const OreRoundTreasurySchema = z.object({
  motherlode: z.number().nullable().optional(),
});

export const OreRoundSchema = z.object({
  entropy: OreRoundEntropySchema.optional(),
  id: OreRoundIdSchema.optional(),
  metrics: OreRoundMetricsSchema.optional(),
  results: OreRoundResultsSchema.optional(),
  state: OreRoundStateSchema.optional(),
  treasury: OreRoundTreasurySchema.optional(),
  ore_metadata: TokenMetadataSchema.nullable().optional(),
});

export const OreRoundCompletedSchema = z.object({
  entropy: OreRoundEntropySchema,
  id: OreRoundIdSchema,
  metrics: OreRoundMetricsSchema,
  results: OreRoundResultsSchema,
  state: OreRoundStateSchema,
  treasury: OreRoundTreasurySchema,
  ore_metadata: TokenMetadataSchema,
});

export interface OreTreasuryId {
  address?: string | null;
}

export interface OreTreasuryState {
  balance?: number | null;
  motherlode?: number | null;
  total_refined?: number | null;
  total_staked?: number | null;
  total_unclaimed?: number | null;
}

export interface OreTreasury {
  id?: OreTreasuryId;
  state?: OreTreasuryState;
  treasury_snapshot?: Treasury | null;
}

export interface Treasury {
  balance?: number;
  buffer_a?: number;
  motherlode?: number;
  miner_rewards_factor?: Record<string, any>;
  stake_rewards_factor?: Record<string, any>;
  buffer_b?: number;
  total_refined?: number;
  total_staked?: number;
  total_unclaimed?: number;
}

export const TreasurySchema = z.object({
  balance: z.number().optional(),
  buffer_a: z.number().optional(),
  motherlode: z.number().optional(),
  miner_rewards_factor: z.record(z.any()).optional(),
  stake_rewards_factor: z.record(z.any()).optional(),
  buffer_b: z.number().optional(),
  total_refined: z.number().optional(),
  total_staked: z.number().optional(),
  total_unclaimed: z.number().optional(),
});

export const OreTreasuryIdSchema = z.object({
  address: z.string().nullable().optional(),
});

export const OreTreasuryStateSchema = z.object({
  balance: z.number().nullable().optional(),
  motherlode: z.number().nullable().optional(),
  total_refined: z.number().nullable().optional(),
  total_staked: z.number().nullable().optional(),
  total_unclaimed: z.number().nullable().optional(),
});

export const OreTreasurySchema = z.object({
  id: OreTreasuryIdSchema.optional(),
  state: OreTreasuryStateSchema.optional(),
  treasury_snapshot: TreasurySchema.nullable().optional(),
});

export const OreTreasuryCompletedSchema = z.object({
  id: OreTreasuryIdSchema,
  state: OreTreasuryStateSchema,
  treasury_snapshot: TreasurySchema,
});

export interface OreMinerAutomation {
  amount?: number | null;
  balance?: number | null;
  executor?: string | null;
  fee?: number | null;
  mask?: number | null;
  reload?: number | null;
  strategy?: number | null;
}

export interface OreMinerId {
  authority?: string | null;
  automation_address?: string | null;
  miner_address?: string | null;
}

export interface OreMinerRewards {
  lifetime_deployed?: number | null;
  lifetime_rewards_ore?: number | null;
  lifetime_rewards_sol?: number | null;
  refined_ore?: number | null;
  rewards_ore?: number | null;
  rewards_sol?: number | null;
}

export interface OreMinerState {
  checkpoint_fee?: number | null;
  checkpoint_id?: number | null;
  last_claim_ore_at?: number | null;
  last_claim_sol_at?: number | null;
  round_id?: number | null;
}

export interface OreMiner {
  automation?: OreMinerAutomation;
  id?: OreMinerId;
  rewards?: OreMinerRewards;
  state?: OreMinerState;
  miner_snapshot?: Miner | null;
  automation_snapshot?: Automation | null;
}

export interface Miner {
  authority?: string;
  deployed?: number[];
  cumulative?: number[];
  checkpoint_fee?: number;
  checkpoint_id?: number;
  last_claim_ore_at?: number;
  last_claim_sol_at?: number;
  rewards_factor?: Record<string, any>;
  rewards_sol?: number;
  rewards_ore?: number;
  refined_ore?: number;
  round_id?: number;
  lifetime_rewards_sol?: number;
  lifetime_rewards_ore?: number;
  lifetime_deployed?: number;
}

export interface Automation {
  amount?: number;
  authority?: string;
  balance?: number;
  executor?: string;
  fee?: number;
  strategy?: number;
  mask?: number;
  reload?: number;
}

export const MinerSchema = z.object({
  authority: z.string().optional(),
  deployed: z.array(z.number()).optional(),
  cumulative: z.array(z.number()).optional(),
  checkpoint_fee: z.number().optional(),
  checkpoint_id: z.number().optional(),
  last_claim_ore_at: z.number().optional(),
  last_claim_sol_at: z.number().optional(),
  rewards_factor: z.record(z.any()).optional(),
  rewards_sol: z.number().optional(),
  rewards_ore: z.number().optional(),
  refined_ore: z.number().optional(),
  round_id: z.number().optional(),
  lifetime_rewards_sol: z.number().optional(),
  lifetime_rewards_ore: z.number().optional(),
  lifetime_deployed: z.number().optional(),
});

export const AutomationSchema = z.object({
  amount: z.number().optional(),
  authority: z.string().optional(),
  balance: z.number().optional(),
  executor: z.string().optional(),
  fee: z.number().optional(),
  strategy: z.number().optional(),
  mask: z.number().optional(),
  reload: z.number().optional(),
});

export const OreMinerAutomationSchema = z.object({
  amount: z.number().nullable().optional(),
  balance: z.number().nullable().optional(),
  executor: z.string().nullable().optional(),
  fee: z.number().nullable().optional(),
  mask: z.number().nullable().optional(),
  reload: z.number().nullable().optional(),
  strategy: z.number().nullable().optional(),
});

export const OreMinerIdSchema = z.object({
  authority: z.string().nullable().optional(),
  automation_address: z.string().nullable().optional(),
  miner_address: z.string().nullable().optional(),
});

export const OreMinerRewardsSchema = z.object({
  lifetime_deployed: z.number().nullable().optional(),
  lifetime_rewards_ore: z.number().nullable().optional(),
  lifetime_rewards_sol: z.number().nullable().optional(),
  refined_ore: z.number().nullable().optional(),
  rewards_ore: z.number().nullable().optional(),
  rewards_sol: z.number().nullable().optional(),
});

export const OreMinerStateSchema = z.object({
  checkpoint_fee: z.number().nullable().optional(),
  checkpoint_id: z.number().nullable().optional(),
  last_claim_ore_at: z.number().nullable().optional(),
  last_claim_sol_at: z.number().nullable().optional(),
  round_id: z.number().nullable().optional(),
});

export const OreMinerSchema = z.object({
  automation: OreMinerAutomationSchema.optional(),
  id: OreMinerIdSchema.optional(),
  rewards: OreMinerRewardsSchema.optional(),
  state: OreMinerStateSchema.optional(),
  miner_snapshot: MinerSchema.nullable().optional(),
  automation_snapshot: AutomationSchema.nullable().optional(),
});

export const OreMinerCompletedSchema = z.object({
  automation: OreMinerAutomationSchema,
  id: OreMinerIdSchema,
  rewards: OreMinerRewardsSchema,
  state: OreMinerStateSchema,
  miner_snapshot: MinerSchema,
  automation_snapshot: AutomationSchema,
});

// ============================================================================
// View Definition Types (framework-agnostic)
// ============================================================================

/** View definition with embedded entity type */
export interface ViewDef<T, TMode extends 'state' | 'list'> {
  readonly mode: TMode;
  readonly view: string;
  /** Phantom field for type inference - not present at runtime */
  readonly _entity?: T;
}

/** Helper to create typed state view definitions (keyed lookups) */
function stateView<T>(view: string): ViewDef<T, 'state'> {
  return { mode: 'state', view } as const;
}

/** Helper to create typed list view definitions (collections) */
function listView<T>(view: string): ViewDef<T, 'list'> {
  return { mode: 'list', view } as const;
}

// ============================================================================
// Stack Definition
// ============================================================================

/** Stack definition for OreStream with 3 entities */
export const ORE_STREAM_STACK = {
  name: 'ore-stream',
  url: 'wss://ore.stack.usehyperstack.com',
  views: {
    OreRound: {
      state: stateView<OreRound>('OreRound/state'),
      list: listView<OreRound>('OreRound/list'),
      latest: listView<OreRound>('OreRound/latest'),
    },
    OreTreasury: {
      state: stateView<OreTreasury>('OreTreasury/state'),
      list: listView<OreTreasury>('OreTreasury/list'),
    },
    OreMiner: {
      state: stateView<OreMiner>('OreMiner/state'),
      list: listView<OreMiner>('OreMiner/list'),
    },
  },
  schemas: {
    Automation: AutomationSchema,
    Miner: MinerSchema,
    OreMinerAutomation: OreMinerAutomationSchema,
    OreMinerCompleted: OreMinerCompletedSchema,
    OreMinerId: OreMinerIdSchema,
    OreMinerRewards: OreMinerRewardsSchema,
    OreMiner: OreMinerSchema,
    OreMinerState: OreMinerStateSchema,
    OreRoundCompleted: OreRoundCompletedSchema,
    OreRoundEntropy: OreRoundEntropySchema,
    OreRoundId: OreRoundIdSchema,
    OreRoundMetrics: OreRoundMetricsSchema,
    OreRoundResults: OreRoundResultsSchema,
    OreRound: OreRoundSchema,
    OreRoundState: OreRoundStateSchema,
    OreRoundTreasury: OreRoundTreasurySchema,
    OreTreasuryCompleted: OreTreasuryCompletedSchema,
    OreTreasuryId: OreTreasuryIdSchema,
    OreTreasury: OreTreasurySchema,
    OreTreasuryState: OreTreasuryStateSchema,
    TokenMetadata: TokenMetadataSchema,
    Treasury: TreasurySchema,
  },
} as const;

/** Type alias for the stack */
export type OreStreamStack = typeof ORE_STREAM_STACK;

/** Entity types in this stack */
export type OreStreamEntity = OreRound | OreTreasury | OreMiner;

/** Default export for convenience */
export default ORE_STREAM_STACK;