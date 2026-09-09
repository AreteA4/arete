import { z } from 'zod';
import { pda, literal, account, programAccountRead, createInstructionHandler, type ErrorMetadata, buildInstruction, PROGRAM_OPERATION_EXTENSIONS, instructionOperation, createPreparedInstruction } from '@usearete/sdk';

export interface OreRoundEntropy {
  entropyEndAt: bigint | null;
  entropySamples: bigint | null;
  entropySeed: string | null;
  entropySlotHash: string | null;
  entropyStartAt: bigint | null;
  entropyValue: string | null;
  entropyVarAddress: string | null;
  resolvedSeed: number[] | null;
}

export interface OreRoundId {
  roundAddress: string | null;
  roundId: bigint | null;
}

export interface OreRoundMetrics {
  checkpointCount: bigint | null;
  deployCount: bigint | null;
}

export interface OreRoundResults {
  didHitMotherlode: boolean | null;
  expiresAtSlotHash: SlotHashBytes | null;
  preRevealRng: bigint | null;
  preRevealRngCandidate: KeccakRngValue | null;
  preRevealWinningSquare: bigint | null;
  rentPayer: string | null;
  rng: KeccakRngValue | null;
  slotHash: string | null;
  topMiner: string | null;
  topMinerReward: number | null;
  winningSquare: bigint | null;
}

export interface OreRoundState {
  closesAt: bigint | null;
  countPerSquare: bigint[] | null;
  deployedPerSquare: bigint[] | null;
  deployedPerSquareUi: number[] | null;
  estimatedExpiresAtUnix: bigint | null;
  expiresAt: bigint | null;
  motherlode: number | null;
  totalDeployed: number | null;
  totalMiners: bigint | null;
  totalVaulted: number | null;
  totalWinnings: number | null;
}

export interface OreRoundTreasury {
  motherlode: number | null;
}

export interface OreRound {
  entropy: OreRoundEntropy;
  id: OreRoundId;
  metrics: OreRoundMetrics;
  results: OreRoundResults;
  state: OreRoundState;
  treasury: OreRoundTreasury;
  oreMetadata: TokenMetadata | null;
}

export interface SlotHashBytes {
  /** 32-byte slot hash as array of numbers (0-255) */
  bytes: number[];
}

export type KeccakRngValue = string;

export interface TokenMetadata {
  mint: string;
  name?: string | null;
  symbol?: string | null;
  decimals?: number | null;
  logoUri?: string | null;
}

export const SlotHashBytesSchema = z.object({
  bytes: z.array(z.number().int().min(0).max(255)).length(32),
});

export const KeccakRngValueSchema = z.string();

export const TokenMetadataSchema = z.object({
  mint: z.string(),
  name: z.string().nullable().optional(),
  symbol: z.string().nullable().optional(),
  decimals: z.number().nullable().optional(),
  logo_uri: z.string().nullable().optional(),
}).transform((value) => ({
  mint: value.mint,
  ...(value.name !== undefined ? { name: value.name } : {}),
  ...(value.symbol !== undefined ? { symbol: value.symbol } : {}),
  ...(value.decimals !== undefined ? { decimals: value.decimals } : {}),
  ...(value.logo_uri !== undefined ? { logoUri: value.logo_uri } : {}),
}));

export const TokenMetadataPatchSchema = z.object({
  mint: z.string().optional(),
  name: z.string().nullable().optional(),
  symbol: z.string().nullable().optional(),
  decimals: z.number().nullable().optional(),
  logo_uri: z.string().nullable().optional(),
}).transform((value) => ({
  ...(value.mint !== undefined ? { mint: value.mint } : {}),
  ...(value.name !== undefined ? { name: value.name } : {}),
  ...(value.symbol !== undefined ? { symbol: value.symbol } : {}),
  ...(value.decimals !== undefined ? { decimals: value.decimals } : {}),
  ...(value.logo_uri !== undefined ? { logoUri: value.logo_uri } : {}),
}));

export const OreRoundEntropySchema = z.object({
  entropy_end_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  entropy_samples: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  entropy_seed: z.string().nullable().optional(),
  entropy_slot_hash: z.string().nullable().optional(),
  entropy_start_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  entropy_value: z.string().nullable().optional(),
  entropy_var_address: z.string().nullable().optional(),
  resolved_seed: z.array(z.number()).nullable().optional(),
}).transform((value) => ({
  entropyEndAt: value.entropy_end_at,
  entropySamples: value.entropy_samples,
  entropySeed: value.entropy_seed,
  entropySlotHash: value.entropy_slot_hash,
  entropyStartAt: value.entropy_start_at,
  entropyValue: value.entropy_value,
  entropyVarAddress: value.entropy_var_address,
  resolvedSeed: value.resolved_seed,
}));

export const OreRoundEntropyPatchSchema = z.object({
  entropy_end_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  entropy_samples: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  entropy_seed: z.string().nullable().optional(),
  entropy_slot_hash: z.string().nullable().optional(),
  entropy_start_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  entropy_value: z.string().nullable().optional(),
  entropy_var_address: z.string().nullable().optional(),
  resolved_seed: z.array(z.number()).nullable().optional(),
}).transform((value) => ({
  ...(value.entropy_end_at !== undefined ? { entropyEndAt: value.entropy_end_at } : {}),
  ...(value.entropy_samples !== undefined ? { entropySamples: value.entropy_samples } : {}),
  ...(value.entropy_seed !== undefined ? { entropySeed: value.entropy_seed } : {}),
  ...(value.entropy_slot_hash !== undefined ? { entropySlotHash: value.entropy_slot_hash } : {}),
  ...(value.entropy_start_at !== undefined ? { entropyStartAt: value.entropy_start_at } : {}),
  ...(value.entropy_value !== undefined ? { entropyValue: value.entropy_value } : {}),
  ...(value.entropy_var_address !== undefined ? { entropyVarAddress: value.entropy_var_address } : {}),
  ...(value.resolved_seed !== undefined ? { resolvedSeed: value.resolved_seed } : {}),
}));

export const OreRoundIdSchema = z.object({
  round_address: z.string().nullable().optional(),
  round_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  roundAddress: value.round_address,
  roundId: value.round_id,
}));

export const OreRoundIdPatchSchema = z.object({
  round_address: z.string().nullable().optional(),
  round_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  ...(value.round_address !== undefined ? { roundAddress: value.round_address } : {}),
  ...(value.round_id !== undefined ? { roundId: value.round_id } : {}),
}));

export const OreRoundMetricsSchema = z.object({
  checkpoint_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  deploy_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  checkpointCount: value.checkpoint_count,
  deployCount: value.deploy_count,
}));

export const OreRoundMetricsPatchSchema = z.object({
  checkpoint_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  deploy_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  ...(value.checkpoint_count !== undefined ? { checkpointCount: value.checkpoint_count } : {}),
  ...(value.deploy_count !== undefined ? { deployCount: value.deploy_count } : {}),
}));

export const OreRoundResultsSchema = z.object({
  did_hit_motherlode: z.boolean().nullable().optional(),
  expires_at_slot_hash: SlotHashBytesSchema.nullable().optional(),
  pre_reveal_rng: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  pre_reveal_rng_candidate: KeccakRngValueSchema.nullable().optional(),
  pre_reveal_winning_square: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  rent_payer: z.string().nullable().optional(),
  rng: KeccakRngValueSchema.nullable().optional(),
  slot_hash: z.string().nullable().optional(),
  top_miner: z.string().nullable().optional(),
  top_miner_reward: z.number().nullable().optional(),
  winning_square: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  didHitMotherlode: value.did_hit_motherlode,
  expiresAtSlotHash: value.expires_at_slot_hash,
  preRevealRng: value.pre_reveal_rng,
  preRevealRngCandidate: value.pre_reveal_rng_candidate,
  preRevealWinningSquare: value.pre_reveal_winning_square,
  rentPayer: value.rent_payer,
  rng: value.rng,
  slotHash: value.slot_hash,
  topMiner: value.top_miner,
  topMinerReward: value.top_miner_reward,
  winningSquare: value.winning_square,
}));

export const OreRoundResultsPatchSchema = z.object({
  did_hit_motherlode: z.boolean().nullable().optional(),
  expires_at_slot_hash: SlotHashBytesSchema.nullable().optional(),
  pre_reveal_rng: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  pre_reveal_rng_candidate: KeccakRngValueSchema.nullable().optional(),
  pre_reveal_winning_square: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  rent_payer: z.string().nullable().optional(),
  rng: KeccakRngValueSchema.nullable().optional(),
  slot_hash: z.string().nullable().optional(),
  top_miner: z.string().nullable().optional(),
  top_miner_reward: z.number().nullable().optional(),
  winning_square: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  ...(value.did_hit_motherlode !== undefined ? { didHitMotherlode: value.did_hit_motherlode } : {}),
  ...(value.expires_at_slot_hash !== undefined ? { expiresAtSlotHash: value.expires_at_slot_hash } : {}),
  ...(value.pre_reveal_rng !== undefined ? { preRevealRng: value.pre_reveal_rng } : {}),
  ...(value.pre_reveal_rng_candidate !== undefined ? { preRevealRngCandidate: value.pre_reveal_rng_candidate } : {}),
  ...(value.pre_reveal_winning_square !== undefined ? { preRevealWinningSquare: value.pre_reveal_winning_square } : {}),
  ...(value.rent_payer !== undefined ? { rentPayer: value.rent_payer } : {}),
  ...(value.rng !== undefined ? { rng: value.rng } : {}),
  ...(value.slot_hash !== undefined ? { slotHash: value.slot_hash } : {}),
  ...(value.top_miner !== undefined ? { topMiner: value.top_miner } : {}),
  ...(value.top_miner_reward !== undefined ? { topMinerReward: value.top_miner_reward } : {}),
  ...(value.winning_square !== undefined ? { winningSquare: value.winning_square } : {}),
}));

export const OreRoundStateSchema = z.object({
  closes_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  count_per_square: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).nullable().optional(),
  deployed_per_square: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).nullable().optional(),
  deployed_per_square_ui: z.array(z.number()).nullable().optional(),
  estimated_expires_at_unix: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  expires_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  motherlode: z.number().nullable().optional(),
  total_deployed: z.number().nullable().optional(),
  total_miners: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  total_vaulted: z.number().nullable().optional(),
  total_winnings: z.number().nullable().optional(),
}).transform((value) => ({
  closesAt: value.closes_at,
  countPerSquare: value.count_per_square,
  deployedPerSquare: value.deployed_per_square,
  deployedPerSquareUi: value.deployed_per_square_ui,
  estimatedExpiresAtUnix: value.estimated_expires_at_unix,
  expiresAt: value.expires_at,
  motherlode: value.motherlode,
  totalDeployed: value.total_deployed,
  totalMiners: value.total_miners,
  totalVaulted: value.total_vaulted,
  totalWinnings: value.total_winnings,
}));

export const OreRoundStatePatchSchema = z.object({
  closes_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  count_per_square: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).nullable().optional(),
  deployed_per_square: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).nullable().optional(),
  deployed_per_square_ui: z.array(z.number()).nullable().optional(),
  estimated_expires_at_unix: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  expires_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  motherlode: z.number().nullable().optional(),
  total_deployed: z.number().nullable().optional(),
  total_miners: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  total_vaulted: z.number().nullable().optional(),
  total_winnings: z.number().nullable().optional(),
}).transform((value) => ({
  ...(value.closes_at !== undefined ? { closesAt: value.closes_at } : {}),
  ...(value.count_per_square !== undefined ? { countPerSquare: value.count_per_square } : {}),
  ...(value.deployed_per_square !== undefined ? { deployedPerSquare: value.deployed_per_square } : {}),
  ...(value.deployed_per_square_ui !== undefined ? { deployedPerSquareUi: value.deployed_per_square_ui } : {}),
  ...(value.estimated_expires_at_unix !== undefined ? { estimatedExpiresAtUnix: value.estimated_expires_at_unix } : {}),
  ...(value.expires_at !== undefined ? { expiresAt: value.expires_at } : {}),
  ...(value.motherlode !== undefined ? { motherlode: value.motherlode } : {}),
  ...(value.total_deployed !== undefined ? { totalDeployed: value.total_deployed } : {}),
  ...(value.total_miners !== undefined ? { totalMiners: value.total_miners } : {}),
  ...(value.total_vaulted !== undefined ? { totalVaulted: value.total_vaulted } : {}),
  ...(value.total_winnings !== undefined ? { totalWinnings: value.total_winnings } : {}),
}));

export const OreRoundTreasurySchema = z.object({
  motherlode: z.number().nullable().optional(),
}).transform((value) => ({
  motherlode: value.motherlode,
}));

export const OreRoundTreasuryPatchSchema = z.object({
  motherlode: z.number().nullable().optional(),
}).transform((value) => ({
  ...(value.motherlode !== undefined ? { motherlode: value.motherlode } : {}),
}));

export const OreRoundSchema = z.object({
  entropy: OreRoundEntropySchema,
  id: OreRoundIdSchema,
  metrics: OreRoundMetricsSchema,
  results: OreRoundResultsSchema,
  state: OreRoundStateSchema,
  treasury: OreRoundTreasurySchema,
  ore_metadata: TokenMetadataSchema.nullable().optional(),
}).transform((value) => ({
  entropy: value.entropy,
  id: value.id,
  metrics: value.metrics,
  results: value.results,
  state: value.state,
  treasury: value.treasury,
  oreMetadata: value.ore_metadata,
}));

export const OreRoundPatchSchema = z.object({
  entropy: OreRoundEntropyPatchSchema.optional(),
  id: OreRoundIdPatchSchema.optional(),
  metrics: OreRoundMetricsPatchSchema.optional(),
  results: OreRoundResultsPatchSchema.optional(),
  state: OreRoundStatePatchSchema.optional(),
  treasury: OreRoundTreasuryPatchSchema.optional(),
  ore_metadata: TokenMetadataPatchSchema.nullable().optional(),
}).transform((value) => ({
  ...(value.entropy !== undefined ? { entropy: value.entropy } : {}),
  ...(value.id !== undefined ? { id: value.id } : {}),
  ...(value.metrics !== undefined ? { metrics: value.metrics } : {}),
  ...(value.results !== undefined ? { results: value.results } : {}),
  ...(value.state !== undefined ? { state: value.state } : {}),
  ...(value.treasury !== undefined ? { treasury: value.treasury } : {}),
  ...(value.ore_metadata !== undefined ? { oreMetadata: value.ore_metadata } : {}),
}));

export const OreRoundCompletedSchema = z.object({
  entropy: OreRoundEntropySchema,
  id: OreRoundIdSchema,
  metrics: OreRoundMetricsSchema,
  results: OreRoundResultsSchema,
  state: OreRoundStateSchema,
  treasury: OreRoundTreasurySchema,
  ore_metadata: TokenMetadataSchema.nullable().optional(),
}).transform((value) => ({
  entropy: value.entropy,
  id: value.id,
  metrics: value.metrics,
  results: value.results,
  state: value.state,
  treasury: value.treasury,
  oreMetadata: value.ore_metadata,
}));

export interface OreBoardId {
  address: string | null;
}

export interface OreBoardState {
  endSlot: bigint | null;
  productionCostEma: bigint | null;
  roundId: bigint | null;
  startSlot: bigint | null;
}

export interface OreBoard {
  id: OreBoardId;
  state: OreBoardState;
  boardSnapshot: CaptureWrapper<Board> | null;
}

export interface Board {
  roundId: bigint;
  startSlot: bigint;
  endSlot: bigint;
  productionCostEma: bigint;
}

/**
 * Wrapper for account data captured with context metadata.
 */
export interface CaptureWrapper<T> {
  /** Unix timestamp when the account was captured */
  timestamp: number;
  /** Base58 account address */
  accountAddress: string;
  /** Captured account data */
  data: T;
  /** Optional blockchain slot number */
  slot?: number;
  /** Optional transaction signature */
  signature?: string;
}

export const CaptureWrapperSchema = <T extends z.ZodTypeAny>(data: T) => z.object({
  timestamp: z.number(),
  account_address: z.string(),
  data,
  slot: z.number().optional(),
  signature: z.string().optional(),
}).transform((value) => ({
  timestamp: value.timestamp,
  accountAddress: value.account_address,
  data: value.data,
  ...(value.slot !== undefined ? { slot: value.slot } : {}),
  ...(value.signature !== undefined ? { signature: value.signature } : {}),
}));

export const BoardSchema = z.object({
  round_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  start_slot: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  end_slot: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  production_cost_ema: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  roundId: value.round_id,
  startSlot: value.start_slot,
  endSlot: value.end_slot,
  productionCostEma: value.production_cost_ema,
}));

export const BoardPatchSchema = z.object({
  round_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  start_slot: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  end_slot: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  production_cost_ema: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
}).transform((value) => ({
  ...(value.round_id !== undefined ? { roundId: value.round_id } : {}),
  ...(value.start_slot !== undefined ? { startSlot: value.start_slot } : {}),
  ...(value.end_slot !== undefined ? { endSlot: value.end_slot } : {}),
  ...(value.production_cost_ema !== undefined ? { productionCostEma: value.production_cost_ema } : {}),
}));

export const OreBoardIdSchema = z.object({
  address: z.string().nullable().optional(),
}).transform((value) => ({
  address: value.address,
}));

export const OreBoardIdPatchSchema = z.object({
  address: z.string().nullable().optional(),
}).transform((value) => ({
  ...(value.address !== undefined ? { address: value.address } : {}),
}));

export const OreBoardStateSchema = z.object({
  end_slot: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  production_cost_ema: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  round_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  start_slot: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  endSlot: value.end_slot,
  productionCostEma: value.production_cost_ema,
  roundId: value.round_id,
  startSlot: value.start_slot,
}));

export const OreBoardStatePatchSchema = z.object({
  end_slot: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  production_cost_ema: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  round_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  start_slot: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  ...(value.end_slot !== undefined ? { endSlot: value.end_slot } : {}),
  ...(value.production_cost_ema !== undefined ? { productionCostEma: value.production_cost_ema } : {}),
  ...(value.round_id !== undefined ? { roundId: value.round_id } : {}),
  ...(value.start_slot !== undefined ? { startSlot: value.start_slot } : {}),
}));

export const OreBoardSchema = z.object({
  id: OreBoardIdSchema,
  state: OreBoardStateSchema,
  board_snapshot: CaptureWrapperSchema(BoardSchema).nullable().optional(),
}).transform((value) => ({
  id: value.id,
  state: value.state,
  boardSnapshot: value.board_snapshot,
}));

export const OreBoardPatchSchema = z.object({
  id: OreBoardIdPatchSchema.optional(),
  state: OreBoardStatePatchSchema.optional(),
  board_snapshot: CaptureWrapperSchema(BoardSchema).nullable().optional(),
}).transform((value) => ({
  ...(value.id !== undefined ? { id: value.id } : {}),
  ...(value.state !== undefined ? { state: value.state } : {}),
  ...(value.board_snapshot !== undefined ? { boardSnapshot: value.board_snapshot } : {}),
}));

export const OreBoardCompletedSchema = z.object({
  id: OreBoardIdSchema,
  state: OreBoardStateSchema,
  board_snapshot: CaptureWrapperSchema(BoardSchema).nullable().optional(),
}).transform((value) => ({
  id: value.id,
  state: value.state,
  boardSnapshot: value.board_snapshot,
}));

export interface OreTreasuryId {
  address: string | null;
}

export interface OreTreasuryState {
  motherlode: number | null;
  totalRefined: number | null;
  totalUnclaimed: number | null;
}

export interface OreTreasury {
  id: OreTreasuryId;
  state: OreTreasuryState;
  treasurySnapshot: CaptureWrapper<Treasury> | null;
}

export interface Treasury {
  motherlode: bigint;
  minerRewardsFactor: Record<string, any>;
  totalRefined: bigint;
  totalUnclaimed: bigint;
}

export const TreasurySchema = z.object({
  motherlode: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  miner_rewards_factor: z.record(z.any()),
  total_refined: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  total_unclaimed: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  motherlode: value.motherlode,
  minerRewardsFactor: value.miner_rewards_factor,
  totalRefined: value.total_refined,
  totalUnclaimed: value.total_unclaimed,
}));

export const TreasuryPatchSchema = z.object({
  motherlode: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  miner_rewards_factor: z.record(z.any()).optional(),
  total_refined: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  total_unclaimed: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
}).transform((value) => ({
  ...(value.motherlode !== undefined ? { motherlode: value.motherlode } : {}),
  ...(value.miner_rewards_factor !== undefined ? { minerRewardsFactor: value.miner_rewards_factor } : {}),
  ...(value.total_refined !== undefined ? { totalRefined: value.total_refined } : {}),
  ...(value.total_unclaimed !== undefined ? { totalUnclaimed: value.total_unclaimed } : {}),
}));

export const OreTreasuryIdSchema = z.object({
  address: z.string().nullable().optional(),
}).transform((value) => ({
  address: value.address,
}));

export const OreTreasuryIdPatchSchema = z.object({
  address: z.string().nullable().optional(),
}).transform((value) => ({
  ...(value.address !== undefined ? { address: value.address } : {}),
}));

export const OreTreasuryStateSchema = z.object({
  motherlode: z.number().nullable().optional(),
  total_refined: z.number().nullable().optional(),
  total_unclaimed: z.number().nullable().optional(),
}).transform((value) => ({
  motherlode: value.motherlode,
  totalRefined: value.total_refined,
  totalUnclaimed: value.total_unclaimed,
}));

export const OreTreasuryStatePatchSchema = z.object({
  motherlode: z.number().nullable().optional(),
  total_refined: z.number().nullable().optional(),
  total_unclaimed: z.number().nullable().optional(),
}).transform((value) => ({
  ...(value.motherlode !== undefined ? { motherlode: value.motherlode } : {}),
  ...(value.total_refined !== undefined ? { totalRefined: value.total_refined } : {}),
  ...(value.total_unclaimed !== undefined ? { totalUnclaimed: value.total_unclaimed } : {}),
}));

export const OreTreasurySchema = z.object({
  id: OreTreasuryIdSchema,
  state: OreTreasuryStateSchema,
  treasury_snapshot: CaptureWrapperSchema(TreasurySchema).nullable().optional(),
}).transform((value) => ({
  id: value.id,
  state: value.state,
  treasurySnapshot: value.treasury_snapshot,
}));

export const OreTreasuryPatchSchema = z.object({
  id: OreTreasuryIdPatchSchema.optional(),
  state: OreTreasuryStatePatchSchema.optional(),
  treasury_snapshot: CaptureWrapperSchema(TreasurySchema).nullable().optional(),
}).transform((value) => ({
  ...(value.id !== undefined ? { id: value.id } : {}),
  ...(value.state !== undefined ? { state: value.state } : {}),
  ...(value.treasury_snapshot !== undefined ? { treasurySnapshot: value.treasury_snapshot } : {}),
}));

export const OreTreasuryCompletedSchema = z.object({
  id: OreTreasuryIdSchema,
  state: OreTreasuryStateSchema,
  treasury_snapshot: CaptureWrapperSchema(TreasurySchema).nullable().optional(),
}).transform((value) => ({
  id: value.id,
  state: value.state,
  treasurySnapshot: value.treasury_snapshot,
}));

export interface OreMinerAutomation {
  amount: bigint | null;
  balance: bigint | null;
  executor: string | null;
  fee: bigint | null;
  mask: bigint | null;
  reload: bigint | null;
  strategy: bigint | null;
}

export interface OreMinerId {
  authority: string | null;
  automationAddress: string | null;
  minerAddress: string | null;
}

export interface OreMinerRewards {
  lifetimeDeployed: bigint | null;
  lifetimeRewardsOre: bigint | null;
  lifetimeRewardsSol: bigint | null;
  refinedOre: bigint | null;
  rewardsOre: bigint | null;
  rewardsSol: bigint | null;
}

export interface OreMinerState {
  checkpointFee: bigint | null;
  checkpointId: bigint | null;
  deployedPerSquare: bigint[] | null;
  deployedPerSquareUi: number[] | null;
  lastClaimOreAt: bigint | null;
  lastClaimSolAt: bigint | null;
  roundId: bigint | null;
  totalDeployed: number | null;
}

export interface OreMiner {
  automation: OreMinerAutomation;
  id: OreMinerId;
  rewards: OreMinerRewards;
  state: OreMinerState;
  minerSnapshot: CaptureWrapper<Miner> | null;
  automationSnapshot: CaptureWrapper<Automation> | null;
}

export interface Miner {
  authority: string;
  autoReturn: bigint;
  checkpointId: bigint;
  checkpointFee: bigint;
  deployed: bigint[];
  mass: bigint[];
  cumulative: bigint[];
  roundId: bigint;
  rewardsFactor: Record<string, any>;
  rewardsSol: bigint;
  refinedOre: bigint;
  rewardsOre: bigint;
  lastClaimOreAt: bigint;
  lastClaimSolAt: bigint;
  lifetimeRewardsOre: bigint;
  lifetimeDeployed: bigint;
  lifetimeRewardsSol: bigint;
}

export interface Automation {
  amount: bigint;
  authority: string;
  balance: bigint;
  executor: string;
  fee: bigint;
  strategy: bigint;
  mask: bigint;
  reload: bigint;
  totalSolSpent: bigint;
  totalOreEarned: bigint;
  conditions: Record<string, any>;
}

export const MinerSchema = z.object({
  authority: z.string(),
  auto_return: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  checkpoint_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  checkpoint_fee: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  deployed: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))),
  mass: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))),
  cumulative: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))),
  round_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  rewards_factor: z.record(z.any()),
  rewards_sol: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  refined_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  rewards_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  last_claim_ore_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  last_claim_sol_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  lifetime_rewards_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  lifetime_deployed: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  lifetime_rewards_sol: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  authority: value.authority,
  autoReturn: value.auto_return,
  checkpointId: value.checkpoint_id,
  checkpointFee: value.checkpoint_fee,
  deployed: value.deployed,
  mass: value.mass,
  cumulative: value.cumulative,
  roundId: value.round_id,
  rewardsFactor: value.rewards_factor,
  rewardsSol: value.rewards_sol,
  refinedOre: value.refined_ore,
  rewardsOre: value.rewards_ore,
  lastClaimOreAt: value.last_claim_ore_at,
  lastClaimSolAt: value.last_claim_sol_at,
  lifetimeRewardsOre: value.lifetime_rewards_ore,
  lifetimeDeployed: value.lifetime_deployed,
  lifetimeRewardsSol: value.lifetime_rewards_sol,
}));

export const AutomationSchema = z.object({
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  authority: z.string(),
  balance: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  executor: z.string(),
  fee: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  strategy: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  mask: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  reload: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  total_sol_spent: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  total_ore_earned: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  conditions: z.record(z.any()),
}).transform((value) => ({
  amount: value.amount,
  authority: value.authority,
  balance: value.balance,
  executor: value.executor,
  fee: value.fee,
  strategy: value.strategy,
  mask: value.mask,
  reload: value.reload,
  totalSolSpent: value.total_sol_spent,
  totalOreEarned: value.total_ore_earned,
  conditions: value.conditions,
}));

export const MinerPatchSchema = z.object({
  authority: z.string().optional(),
  auto_return: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  checkpoint_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  checkpoint_fee: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  deployed: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).optional(),
  mass: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).optional(),
  cumulative: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).optional(),
  round_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  rewards_factor: z.record(z.any()).optional(),
  rewards_sol: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  refined_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  rewards_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  last_claim_ore_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  last_claim_sol_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  lifetime_rewards_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  lifetime_deployed: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  lifetime_rewards_sol: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
}).transform((value) => ({
  ...(value.authority !== undefined ? { authority: value.authority } : {}),
  ...(value.auto_return !== undefined ? { autoReturn: value.auto_return } : {}),
  ...(value.checkpoint_id !== undefined ? { checkpointId: value.checkpoint_id } : {}),
  ...(value.checkpoint_fee !== undefined ? { checkpointFee: value.checkpoint_fee } : {}),
  ...(value.deployed !== undefined ? { deployed: value.deployed } : {}),
  ...(value.mass !== undefined ? { mass: value.mass } : {}),
  ...(value.cumulative !== undefined ? { cumulative: value.cumulative } : {}),
  ...(value.round_id !== undefined ? { roundId: value.round_id } : {}),
  ...(value.rewards_factor !== undefined ? { rewardsFactor: value.rewards_factor } : {}),
  ...(value.rewards_sol !== undefined ? { rewardsSol: value.rewards_sol } : {}),
  ...(value.refined_ore !== undefined ? { refinedOre: value.refined_ore } : {}),
  ...(value.rewards_ore !== undefined ? { rewardsOre: value.rewards_ore } : {}),
  ...(value.last_claim_ore_at !== undefined ? { lastClaimOreAt: value.last_claim_ore_at } : {}),
  ...(value.last_claim_sol_at !== undefined ? { lastClaimSolAt: value.last_claim_sol_at } : {}),
  ...(value.lifetime_rewards_ore !== undefined ? { lifetimeRewardsOre: value.lifetime_rewards_ore } : {}),
  ...(value.lifetime_deployed !== undefined ? { lifetimeDeployed: value.lifetime_deployed } : {}),
  ...(value.lifetime_rewards_sol !== undefined ? { lifetimeRewardsSol: value.lifetime_rewards_sol } : {}),
}));

export const AutomationPatchSchema = z.object({
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  authority: z.string().optional(),
  balance: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  executor: z.string().optional(),
  fee: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  strategy: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  mask: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  reload: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  total_sol_spent: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  total_ore_earned: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  conditions: z.record(z.any()).optional(),
}).transform((value) => ({
  ...(value.amount !== undefined ? { amount: value.amount } : {}),
  ...(value.authority !== undefined ? { authority: value.authority } : {}),
  ...(value.balance !== undefined ? { balance: value.balance } : {}),
  ...(value.executor !== undefined ? { executor: value.executor } : {}),
  ...(value.fee !== undefined ? { fee: value.fee } : {}),
  ...(value.strategy !== undefined ? { strategy: value.strategy } : {}),
  ...(value.mask !== undefined ? { mask: value.mask } : {}),
  ...(value.reload !== undefined ? { reload: value.reload } : {}),
  ...(value.total_sol_spent !== undefined ? { totalSolSpent: value.total_sol_spent } : {}),
  ...(value.total_ore_earned !== undefined ? { totalOreEarned: value.total_ore_earned } : {}),
  ...(value.conditions !== undefined ? { conditions: value.conditions } : {}),
}));

export const OreMinerAutomationSchema = z.object({
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  balance: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  executor: z.string().nullable().optional(),
  fee: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  mask: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  reload: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  strategy: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  amount: value.amount,
  balance: value.balance,
  executor: value.executor,
  fee: value.fee,
  mask: value.mask,
  reload: value.reload,
  strategy: value.strategy,
}));

export const OreMinerAutomationPatchSchema = z.object({
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  balance: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  executor: z.string().nullable().optional(),
  fee: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  mask: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  reload: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  strategy: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  ...(value.amount !== undefined ? { amount: value.amount } : {}),
  ...(value.balance !== undefined ? { balance: value.balance } : {}),
  ...(value.executor !== undefined ? { executor: value.executor } : {}),
  ...(value.fee !== undefined ? { fee: value.fee } : {}),
  ...(value.mask !== undefined ? { mask: value.mask } : {}),
  ...(value.reload !== undefined ? { reload: value.reload } : {}),
  ...(value.strategy !== undefined ? { strategy: value.strategy } : {}),
}));

export const OreMinerIdSchema = z.object({
  authority: z.string().nullable().optional(),
  automation_address: z.string().nullable().optional(),
  miner_address: z.string().nullable().optional(),
}).transform((value) => ({
  authority: value.authority,
  automationAddress: value.automation_address,
  minerAddress: value.miner_address,
}));

export const OreMinerIdPatchSchema = z.object({
  authority: z.string().nullable().optional(),
  automation_address: z.string().nullable().optional(),
  miner_address: z.string().nullable().optional(),
}).transform((value) => ({
  ...(value.authority !== undefined ? { authority: value.authority } : {}),
  ...(value.automation_address !== undefined ? { automationAddress: value.automation_address } : {}),
  ...(value.miner_address !== undefined ? { minerAddress: value.miner_address } : {}),
}));

export const OreMinerRewardsSchema = z.object({
  lifetime_deployed: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  lifetime_rewards_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  lifetime_rewards_sol: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  refined_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  rewards_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  rewards_sol: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  lifetimeDeployed: value.lifetime_deployed,
  lifetimeRewardsOre: value.lifetime_rewards_ore,
  lifetimeRewardsSol: value.lifetime_rewards_sol,
  refinedOre: value.refined_ore,
  rewardsOre: value.rewards_ore,
  rewardsSol: value.rewards_sol,
}));

export const OreMinerRewardsPatchSchema = z.object({
  lifetime_deployed: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  lifetime_rewards_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  lifetime_rewards_sol: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  refined_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  rewards_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  rewards_sol: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  ...(value.lifetime_deployed !== undefined ? { lifetimeDeployed: value.lifetime_deployed } : {}),
  ...(value.lifetime_rewards_ore !== undefined ? { lifetimeRewardsOre: value.lifetime_rewards_ore } : {}),
  ...(value.lifetime_rewards_sol !== undefined ? { lifetimeRewardsSol: value.lifetime_rewards_sol } : {}),
  ...(value.refined_ore !== undefined ? { refinedOre: value.refined_ore } : {}),
  ...(value.rewards_ore !== undefined ? { rewardsOre: value.rewards_ore } : {}),
  ...(value.rewards_sol !== undefined ? { rewardsSol: value.rewards_sol } : {}),
}));

export const OreMinerStateSchema = z.object({
  checkpoint_fee: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  checkpoint_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  deployed_per_square: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).nullable().optional(),
  deployed_per_square_ui: z.array(z.number()).nullable().optional(),
  last_claim_ore_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_claim_sol_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  round_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  total_deployed: z.number().nullable().optional(),
}).transform((value) => ({
  checkpointFee: value.checkpoint_fee,
  checkpointId: value.checkpoint_id,
  deployedPerSquare: value.deployed_per_square,
  deployedPerSquareUi: value.deployed_per_square_ui,
  lastClaimOreAt: value.last_claim_ore_at,
  lastClaimSolAt: value.last_claim_sol_at,
  roundId: value.round_id,
  totalDeployed: value.total_deployed,
}));

export const OreMinerStatePatchSchema = z.object({
  checkpoint_fee: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  checkpoint_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  deployed_per_square: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).nullable().optional(),
  deployed_per_square_ui: z.array(z.number()).nullable().optional(),
  last_claim_ore_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_claim_sol_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  round_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  total_deployed: z.number().nullable().optional(),
}).transform((value) => ({
  ...(value.checkpoint_fee !== undefined ? { checkpointFee: value.checkpoint_fee } : {}),
  ...(value.checkpoint_id !== undefined ? { checkpointId: value.checkpoint_id } : {}),
  ...(value.deployed_per_square !== undefined ? { deployedPerSquare: value.deployed_per_square } : {}),
  ...(value.deployed_per_square_ui !== undefined ? { deployedPerSquareUi: value.deployed_per_square_ui } : {}),
  ...(value.last_claim_ore_at !== undefined ? { lastClaimOreAt: value.last_claim_ore_at } : {}),
  ...(value.last_claim_sol_at !== undefined ? { lastClaimSolAt: value.last_claim_sol_at } : {}),
  ...(value.round_id !== undefined ? { roundId: value.round_id } : {}),
  ...(value.total_deployed !== undefined ? { totalDeployed: value.total_deployed } : {}),
}));

export const OreMinerSchema = z.object({
  automation: OreMinerAutomationSchema,
  id: OreMinerIdSchema,
  rewards: OreMinerRewardsSchema,
  state: OreMinerStateSchema,
  miner_snapshot: CaptureWrapperSchema(MinerSchema).nullable().optional(),
  automation_snapshot: CaptureWrapperSchema(AutomationSchema).nullable().optional(),
}).transform((value) => ({
  automation: value.automation,
  id: value.id,
  rewards: value.rewards,
  state: value.state,
  minerSnapshot: value.miner_snapshot,
  automationSnapshot: value.automation_snapshot,
}));

export const OreMinerPatchSchema = z.object({
  automation: OreMinerAutomationPatchSchema.optional(),
  id: OreMinerIdPatchSchema.optional(),
  rewards: OreMinerRewardsPatchSchema.optional(),
  state: OreMinerStatePatchSchema.optional(),
  miner_snapshot: CaptureWrapperSchema(MinerSchema).nullable().optional(),
  automation_snapshot: CaptureWrapperSchema(AutomationSchema).nullable().optional(),
}).transform((value) => ({
  ...(value.automation !== undefined ? { automation: value.automation } : {}),
  ...(value.id !== undefined ? { id: value.id } : {}),
  ...(value.rewards !== undefined ? { rewards: value.rewards } : {}),
  ...(value.state !== undefined ? { state: value.state } : {}),
  ...(value.miner_snapshot !== undefined ? { minerSnapshot: value.miner_snapshot } : {}),
  ...(value.automation_snapshot !== undefined ? { automationSnapshot: value.automation_snapshot } : {}),
}));

export const OreMinerCompletedSchema = z.object({
  automation: OreMinerAutomationSchema,
  id: OreMinerIdSchema,
  rewards: OreMinerRewardsSchema,
  state: OreMinerStateSchema,
  miner_snapshot: CaptureWrapperSchema(MinerSchema).nullable().optional(),
  automation_snapshot: CaptureWrapperSchema(AutomationSchema).nullable().optional(),
}).transform((value) => ({
  automation: value.automation,
  id: value.id,
  rewards: value.rewards,
  state: value.state,
  minerSnapshot: value.miner_snapshot,
  automationSnapshot: value.automation_snapshot,
}));

export interface AdminConfig {
  authority: string;
  feeCollector: string;
  feeRate: bigint;
}

export interface AutomationConditions {
  maxProductionCost: bigint;
  minMotherlode: bigint;
  maxMotherlode: bigint;
}

export interface Numeric {
  bits: number[];
}

export interface ProtocolConfig {
  authority: string;
  feeCollector: string;
  feeRate: bigint;
  intermissionSlots: bigint;
  roundSlots: bigint;
  entropyVarAddress: string;
  entropyProgramId: string;
}

export interface OreAutomation {
  amount: bigint;
  authority: string;
  balance: bigint;
  executor: string;
  fee: bigint;
  strategy: bigint;
  mask: bigint;
  reload: bigint;
  totalSolSpent: bigint;
  totalOreEarned: bigint;
  conditions: AutomationConditions;
}

export interface OreBoard2 {
  roundId: bigint;
  startSlot: bigint;
  endSlot: bigint;
  productionCostEma: bigint;
}

export interface Config {
  admin: AdminConfig;
  protocol: ProtocolConfig;
}

export interface OreMiner2 {
  authority: string;
  autoReturn: bigint;
  checkpointId: bigint;
  checkpointFee: bigint;
  deployed: bigint[];
  mass: bigint[];
  cumulative: bigint[];
  roundId: bigint;
  rewardsFactor: Numeric;
  rewardsSol: bigint;
  refinedOre: bigint;
  rewardsOre: bigint;
  lastClaimOreAt: bigint;
  lastClaimSolAt: bigint;
  lifetimeRewardsOre: bigint;
  lifetimeDeployed: bigint;
  lifetimeRewardsSol: bigint;
}

export interface Round {
  id: bigint;
  deployed: bigint[];
  mass: bigint[];
  count: bigint[];
  slotHash: number[];
  expiresAt: bigint;
  motherlode: bigint;
  rentPayer: string;
  rewards: bigint[];
  totalVaulted: bigint;
  totalWinnings: bigint;
  totalMiners: bigint;
  topMiner: string;
}

export interface OreTreasury2 {
  motherlode: bigint;
  minerRewardsFactor: Numeric;
  totalRefined: bigint;
  totalUnclaimed: bigint;
}

export interface Var {
  authority: string;
  id: bigint;
  provider: string;
  commit: number[];
  seed: number[];
  slotHash: number[];
  value: number[];
  samples: bigint;
  isAuto: bigint;
  startAt: bigint;
  endAt: bigint;
}

export const AdminConfigSchema = z.object({
  authority: z.string(),
  fee_collector: z.string(),
  fee_rate: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  authority: value.authority,
  feeCollector: value.fee_collector,
  feeRate: value.fee_rate,
}));

export const AutomationConditionsSchema = z.object({
  max_production_cost: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  min_motherlode: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  max_motherlode: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  maxProductionCost: value.max_production_cost,
  minMotherlode: value.min_motherlode,
  maxMotherlode: value.max_motherlode,
}));

export const NumericSchema = z.object({
  bits: z.array(z.number()).length(16),
}).transform((value) => ({
  bits: value.bits,
}));

export const ProtocolConfigSchema = z.object({
  authority: z.string(),
  fee_collector: z.string(),
  fee_rate: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  intermission_slots: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  round_slots: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  entropy_var_address: z.string(),
  entropy_program_id: z.string(),
}).transform((value) => ({
  authority: value.authority,
  feeCollector: value.fee_collector,
  feeRate: value.fee_rate,
  intermissionSlots: value.intermission_slots,
  roundSlots: value.round_slots,
  entropyVarAddress: value.entropy_var_address,
  entropyProgramId: value.entropy_program_id,
}));

export const OreAutomationSchema = z.object({
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  authority: z.string(),
  balance: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  executor: z.string(),
  fee: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  strategy: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  mask: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  reload: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  total_sol_spent: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  total_ore_earned: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  conditions: z.lazy(() => AutomationConditionsSchema),
}).transform((value) => ({
  amount: value.amount,
  authority: value.authority,
  balance: value.balance,
  executor: value.executor,
  fee: value.fee,
  strategy: value.strategy,
  mask: value.mask,
  reload: value.reload,
  totalSolSpent: value.total_sol_spent,
  totalOreEarned: value.total_ore_earned,
  conditions: value.conditions,
}));

export const OreBoard2Schema = z.object({
  round_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  start_slot: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  end_slot: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  production_cost_ema: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  roundId: value.round_id,
  startSlot: value.start_slot,
  endSlot: value.end_slot,
  productionCostEma: value.production_cost_ema,
}));

export const ConfigSchema = z.object({
  admin: z.lazy(() => AdminConfigSchema),
  protocol: z.lazy(() => ProtocolConfigSchema),
}).transform((value) => ({
  admin: value.admin,
  protocol: value.protocol,
}));

export const OreMiner2Schema = z.object({
  authority: z.string(),
  auto_return: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  checkpoint_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  checkpoint_fee: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  deployed: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).length(25),
  mass: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).length(25),
  cumulative: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).length(25),
  round_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  rewards_factor: z.lazy(() => NumericSchema),
  rewards_sol: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  refined_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  rewards_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  last_claim_ore_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  last_claim_sol_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  lifetime_rewards_ore: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  lifetime_deployed: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  lifetime_rewards_sol: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  authority: value.authority,
  autoReturn: value.auto_return,
  checkpointId: value.checkpoint_id,
  checkpointFee: value.checkpoint_fee,
  deployed: value.deployed,
  mass: value.mass,
  cumulative: value.cumulative,
  roundId: value.round_id,
  rewardsFactor: value.rewards_factor,
  rewardsSol: value.rewards_sol,
  refinedOre: value.refined_ore,
  rewardsOre: value.rewards_ore,
  lastClaimOreAt: value.last_claim_ore_at,
  lastClaimSolAt: value.last_claim_sol_at,
  lifetimeRewardsOre: value.lifetime_rewards_ore,
  lifetimeDeployed: value.lifetime_deployed,
  lifetimeRewardsSol: value.lifetime_rewards_sol,
}));

export const RoundSchema = z.object({
  id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  deployed: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).length(25),
  mass: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).length(25),
  count: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).length(25),
  slot_hash: z.array(z.number()).length(32),
  expires_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  motherlode: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  rent_payer: z.string(),
  rewards: z.array(z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))).length(25),
  total_vaulted: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  total_winnings: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  total_miners: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  top_miner: z.string(),
}).transform((value) => ({
  id: value.id,
  deployed: value.deployed,
  mass: value.mass,
  count: value.count,
  slotHash: value.slot_hash,
  expiresAt: value.expires_at,
  motherlode: value.motherlode,
  rentPayer: value.rent_payer,
  rewards: value.rewards,
  totalVaulted: value.total_vaulted,
  totalWinnings: value.total_winnings,
  totalMiners: value.total_miners,
  topMiner: value.top_miner,
}));

export const OreTreasury2Schema = z.object({
  motherlode: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  miner_rewards_factor: z.lazy(() => NumericSchema),
  total_refined: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  total_unclaimed: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  motherlode: value.motherlode,
  minerRewardsFactor: value.miner_rewards_factor,
  totalRefined: value.total_refined,
  totalUnclaimed: value.total_unclaimed,
}));

export const VarSchema = z.object({
  authority: z.string(),
  id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  provider: z.string(),
  commit: z.array(z.number()).length(32),
  seed: z.array(z.number()).length(32),
  slot_hash: z.array(z.number()).length(32),
  value: z.array(z.number()).length(32),
  samples: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  is_auto: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  start_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  end_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  authority: value.authority,
  id: value.id,
  provider: value.provider,
  commit: value.commit,
  seed: value.seed,
  slotHash: value.slot_hash,
  value: value.value,
  samples: value.samples,
  isAuto: value.is_auto,
  startAt: value.start_at,
  endAt: value.end_at,
}));

// ============================================================================
// Instruction Handlers
// ============================================================================

/** Union of all program errors declared across this stack's instructions. */
export type OreStreamOreProgramError =
  | { code: 0; name: 'AmountTooSmall'; msg: string }
  | { code: 1; name: 'NotAuthorized'; msg: string }
  | { code: 2; name: 'InvalidExecutor'; msg: string };

const ORE_STREAM_ORE_PROGRAM_ERRORS: ErrorMetadata[] = [
  { code: 0, name: 'AmountTooSmall', msg: 'Amount too small' },
  { code: 1, name: 'NotAuthorized', msg: 'Not authorized' },
  { code: 2, name: 'InvalidExecutor', msg: 'Invalid executor' },
];

/** Union of all program errors declared across this stack's instructions. */
export type OreStreamEntropyProgramError =
  | { code: 0; name: 'IncompleteDigest'; msg: string }
  | { code: 1; name: 'InvalidSeed'; msg: string };

const ORE_STREAM_ENTROPY_PROGRAM_ERRORS: ErrorMetadata[] = [
  { code: 0, name: 'IncompleteDigest', msg: 'Incomplete digest' },
  { code: 1, name: 'InvalidSeed', msg: 'Invalid seed' },
];

export interface OreAutomateParams {
  amount: bigint;
  deposit: bigint;
  fee: bigint;
  mask: bigint;
  strategy: number;
  reload: bigint;
  signer: string;
  automation: string;
  executor: string;
  miner: string;
}

export type OreAutomateError = OreStreamOreProgramError;

/**
 * Configures or closes a miner automation account.
 * Automation PDA seeds: ["automation", signer].
 * Miner PDA seeds: ["miner", signer].
 * The declared args are the legacy `Automate` layout (41 bytes after the tag). The program first tries `AutomateV2::try_from_bytes` and falls back to `Automate`, so payloads may carry an optional 24-byte `conditions` (AutomationConditions) tail at offset 42. That tail is intentionally left unmodelled in the baseline and reported as trailing bytes; model it in the augmented spec.
 */
export const oreAutomateInstruction = createInstructionHandler<OreAutomateParams, OreAutomateError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [0],
  args: [
    { name: 'amount', type: 'u64' },
    { name: 'deposit', type: 'u64' },
    { name: 'fee', type: 'u64' },
    { name: 'mask', type: 'u64' },
    { name: 'strategy', type: 'u8' },
    { name: 'reload', type: 'u64' },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    // [arete codegen] instruction 'automate': account 'automation' PDA 'automation' degraded to userProvided (seed references account 'authority' not present in this instruction)
    { name: 'automation', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'executor', isSigner: false, isWritable: false, category: 'userProvided' },
    // [arete codegen] instruction 'automate': account 'miner' PDA 'miner' degraded to userProvided (seed references account 'authority' not present in this instruction)
    { name: 'miner', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});

export interface OreCheckpointParams {
  signer: string;
  authority: string;
  automation?: string;
  board?: string;
  miner?: string;
  round: string;
  treasury?: string;
}

export type OreCheckpointError = OreStreamOreProgramError;

/**
 * Settles miner rewards for a completed round.
 * Treasury PDA seeds: ["treasury"].
 */
export const oreCheckpointInstruction = createInstructionHandler<OreCheckpointParams, OreCheckpointError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [2],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'authority', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'automation', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'automation' }, { type: 'accountRef', accountName: 'authority' }] } },
    { name: 'board', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'board' }] } },
    { name: 'miner', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'miner' }, { type: 'accountRef', accountName: 'authority' }] } },
    { name: 'round', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'treasury' }] } },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});

export interface OreClaimSolParams {
  signer: string;
  board?: string;
  miner: string;
}

export type OreClaimSolError = OreStreamOreProgramError;

/**
 * Claims SOL rewards from the miner account.
 */
export const oreClaimSolInstruction = createInstructionHandler<OreClaimSolParams, OreClaimSolError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [3],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'board', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'board' }] } },
    // [arete codegen] instruction 'claimSol': account 'miner' PDA 'miner' degraded to userProvided (seed references account 'authority' not present in this instruction)
    { name: 'miner', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
    { name: 'oreProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv' },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});

export interface OreClaimOreParams {
  signer: string;
  board?: string;
  miner: string;
  recipient: string;
  treasury?: string;
  treasuryTokens: string;
}

export type OreClaimOreError = OreStreamOreProgramError;

/**
 * Claims ORE token rewards from the treasury vault.
 * The baseline payload is tag-only: upstream `ClaimORE` args are parsed with `if let Ok(args) = ClaimORE::try_from_bytes(data)` and default to DENOMINATOR_BPS (10000) when absent, so the program accepts both a 1-byte payload and a 9-byte payload.
 * Optional trailing arg (not modelled in the baseline): `bps: u64` little-endian at offset 1, a discretionary claim percentage in basis points; when omitted the program claims 100% (10000 bps).
 * Both shapes are live on mainnet, so the optional bps tail belongs in the augmented spec; declaring it here would hard-fail the tag-only variant.
 */
export const oreClaimOreInstruction = createInstructionHandler<OreClaimOreParams, OreClaimOreError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [4],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'board', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'board' }] } },
    // [arete codegen] instruction 'claimOre': account 'miner' PDA 'miner' degraded to userProvided (seed references account 'authority' not present in this instruction)
    { name: 'miner', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'mint', isSigner: false, isWritable: true, category: 'known', knownAddress: 'oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp' },
    { name: 'recipient', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'treasury' }] } },
    { name: 'treasuryTokens', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
    { name: 'tokenProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' },
    { name: 'associatedTokenProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL' },
    { name: 'oreProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv' },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});

export interface OreCloseParams {
  signer: string;
  board?: string;
  rentPayer: string;
  round: string;
  treasury?: string;
}

export type OreCloseError = OreStreamOreProgramError;

/**
 * Closes an expired round account and returns rent to the payer.
 * Round PDA seeds: ["round", round_id].
 * Treasury PDA seeds: ["treasury"].
 */
export const oreCloseInstruction = createInstructionHandler<OreCloseParams, OreCloseError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [5],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'board', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'board' }] } },
    { name: 'rentPayer', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'round', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'treasury' }] } },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});

export interface OreDeployParams {
  amount: bigint;
  squares: number;
  signer: string;
  authority: string;
  automation?: string;
  board?: string;
  config?: string;
  miner?: string;
  round: string;
  treasury?: string;
  entropyVar?: string;
  entropyProgram?: string;
}

export type OreDeployError = OreStreamOreProgramError;

/**
 * Deploys SOL to selected squares for the current round.
 * Automation PDA seeds: ["automation", authority].
 * Config PDA seeds: ["config"].
 * Miner PDA seeds: ["miner", authority].
 * Round PDA seeds: ["round", board.round_id].
 */
export const oreDeployInstruction = createInstructionHandler<OreDeployParams, OreDeployError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [6],
  args: [
    { name: 'amount', type: 'u64' },
    { name: 'squares', type: 'u32' },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'authority', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'automation', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'automation' }, { type: 'accountRef', accountName: 'authority' }] } },
    { name: 'board', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'board' }] } },
    { name: 'config', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'config' }] } },
    { name: 'miner', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'miner' }, { type: 'accountRef', accountName: 'authority' }] } },
    { name: 'round', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'treasury' }] } },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
    { name: 'oreProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv' },
    { name: 'entropyVar', isSigner: false, isWritable: true, category: 'userProvided', isOptional: true },
    { name: 'entropyProgram', isSigner: false, isWritable: false, category: 'userProvided', isOptional: true },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});

export interface OreLogParams {
  signer: string;
}

export type OreLogError = OreStreamOreProgramError;

/**
 * Emits an arbitrary log message from the board PDA.
 * Bytes following the discriminator are logged verbatim.
 */
export const oreLogInstruction = createInstructionHandler<OreLogParams, OreLogError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [8],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});

export interface OreResetParams {
  signer: string;
  board?: string;
  config?: string;
  feeCollector: string;
  round: string;
  roundNext: string;
  topMiner: string;
  treasury?: string;
  treasuryTokens: string;
  entropyVar: string;
  mintAuthority: string;
}

export type OreResetError = OreStreamOreProgramError;

/**
 * Finalizes the current round, mints rewards, and opens the next round.
 * Board PDA seeds: ["board"].
 * Treasury PDA seeds: ["treasury"].
 * Round PDA seeds: ["round", board.round_id] and ["round", board.round_id + 1].
 */
export const oreResetInstruction = createInstructionHandler<OreResetParams, OreResetError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [9],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'board', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'board' }] } },
    { name: 'config', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'config' }] } },
    { name: 'feeCollector', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'mint', isSigner: false, isWritable: true, category: 'known', knownAddress: 'oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp' },
    { name: 'round', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'roundNext', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'topMiner', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'treasury' }] } },
    { name: 'treasuryTokens', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
    { name: 'tokenProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' },
    { name: 'oreProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv' },
    { name: 'slotHashesSysvar', isSigner: false, isWritable: false, category: 'known', knownAddress: 'SysvarS1otHashes111111111111111111111111111' },
    { name: 'entropyVar', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'entropyProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X' },
    { name: 'mintAuthority', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'mintProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'mintzxW6Kckmeyh1h6Zfdj9QcYgCzhPSGiC8ChZ6fCx' },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});

export interface OreBuybackParams {
  board?: string;
  config?: string;
  managerSol: string;
  treasury?: string;
  treasuryOre: string;
  treasurySol: string;
  stakeTreasury: string;
  stakeTreasuryOre: string;
  stakeVesting: string;
  oreStakeProgram: string;
}

export type OreBuybackError = OreStreamOreProgramError;

/**
 * Swaps vaulted SOL to ORE through Jupiter, distributes staking yield, and burns the remainder.
 * The 15 declared accounts are followed by Jupiter route accounts, and raw Jupiter instruction data follows the discriminator.
 */
export const oreBuybackInstruction = createInstructionHandler<OreBuybackParams, OreBuybackError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [13],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'known', knownAddress: 'HNWhK5f8RMWBqcA7mXJPaxdTPGrha3rrqUrri7HSKb3T' },
    { name: 'board', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'board' }] } },
    { name: 'config', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'config' }] } },
    { name: 'manager', isSigner: false, isWritable: true, category: 'known', knownAddress: 'DJqfQWB8tZE6fzqWa8okncDh7ciTuD8QQKp1ssNETWee' },
    { name: 'managerSol', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'mint', isSigner: false, isWritable: true, category: 'known', knownAddress: 'oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'treasury' }] } },
    { name: 'treasuryOre', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasurySol', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'stakeTreasury', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'stakeTreasuryOre', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'stakeVesting', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'tokenProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' },
    { name: 'oreProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv' },
    { name: 'oreStakeProgram', isSigner: false, isWritable: false, category: 'userProvided' },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});

export interface OreBuryParams {
  amount: bigint;
  signer: string;
  sender: string;
  board?: string;
  treasury?: string;
  treasuryOre: string;
  stakeTreasury: string;
  stakeTreasuryTokens: string;
  stakeVesting: string;
  oreStakeProgram: string;
}

export type OreBuryError = OreStreamOreProgramError;

/**
 * Burns ORE and distributes yield to stakers.
 * Treasury PDA seeds: ["treasury"].
 */
export const oreBuryInstruction = createInstructionHandler<OreBuryParams, OreBuryError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [24],
  args: [
    { name: 'amount', type: 'u64' },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'sender', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'board', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'board' }] } },
    { name: 'mint', isSigner: false, isWritable: true, category: 'known', knownAddress: 'oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'treasury' }] } },
    { name: 'treasuryOre', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'stakeTreasury', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'stakeTreasuryTokens', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'stakeVesting', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'tokenProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' },
    { name: 'oreProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv' },
    { name: 'oreStakeProgram', isSigner: false, isWritable: false, category: 'userProvided' },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});

export interface OreWrapParams {
  amount: bigint;
  config?: string;
  treasury?: string;
  treasurySol: string;
}

export type OreWrapError = OreStreamOreProgramError;

/**
 * Wraps SOL held by the treasury into WSOL for swapping.
 * Treasury PDA seeds: ["treasury"].
 */
export const oreWrapInstruction = createInstructionHandler<OreWrapParams, OreWrapError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [14],
  args: [
    { name: 'amount', type: 'u64' },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'known', knownAddress: 'HNWhK5f8RMWBqcA7mXJPaxdTPGrha3rrqUrri7HSKb3T' },
    { name: 'config', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'config' }] } },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'treasury' }] } },
    { name: 'treasurySol', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});

export interface OreSetAdminParams {
  admin: string;
  signer: string;
  config?: string;
}

export type OreSetAdminError = OreStreamOreProgramError;

/**
 * Updates the program admin address.
 */
export const oreSetAdminInstruction = createInstructionHandler<OreSetAdminParams, OreSetAdminError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [15],
  args: [
    { name: 'admin', type: 'pubkey' },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'config', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'config' }] } },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});

export interface OreNewVarParams {
  id: bigint;
  commit: number[];
  samples: bigint;
  signer: string;
  board?: string;
  config?: string;
  provider: string;
  var: string;
}

export type OreNewVarError = OreStreamOreProgramError;

/**
 * Creates a new entropy var account through the entropy program.
 */
export const oreNewVarInstruction = createInstructionHandler<OreNewVarParams, OreNewVarError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [19],
  args: [
    { name: 'id', type: 'u64' },
    { name: 'commit', type: { array: ['u8', 32] } },
    { name: 'samples', type: 'u64' },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'board', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'board' }] } },
    { name: 'config', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'config' }] } },
    { name: 'provider', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'var', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
    { name: 'entropyProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X' },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});

export interface OreReloadSolParams {
  signer: string;
  automation: string;
  miner: string;
}

export type OreReloadSolError = OreStreamOreProgramError;

/**
 * Deprecated since 3.8.15; this behavior is now included in checkpoint.
 */
export const oreReloadSolInstruction = createInstructionHandler<OreReloadSolParams, OreReloadSolError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [21],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    // [arete codegen] instruction 'reloadSol': account 'automation' PDA 'automation' degraded to userProvided (seed references account 'authority' not present in this instruction)
    { name: 'automation', isSigner: false, isWritable: true, category: 'userProvided' },
    // [arete codegen] instruction 'reloadSol': account 'miner' PDA 'miner' degraded to userProvided (seed references account 'authority' not present in this instruction)
    { name: 'miner', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});

export interface EntropyOpenParams {
  id: bigint;
  commit: number[];
  isAuto: bigint;
  samples: bigint;
  endAt: bigint;
  authority: string;
  payer: string;
  provider: string;
  var: string;
}

export type EntropyOpenError = OreStreamEntropyProgramError;

/**
 * Creates a new entropy var account.
 * Var PDA seeds: ["var", authority, id].
 */
export const entropyOpenInstruction = createInstructionHandler<EntropyOpenParams, EntropyOpenError>({
  programId: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X',
  discriminator: [0],
  args: [
    { name: 'id', type: 'u64' },
    { name: 'commit', type: { array: ['u8', 32] } },
    { name: 'isAuto', type: 'u64' },
    { name: 'samples', type: 'u64' },
    { name: 'endAt', type: 'u64' },
  ],
  accounts: [
    { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'payer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'provider', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'var', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ORE_STREAM_ENTROPY_PROGRAM_ERRORS,
});

export interface EntropyCloseParams {
  signer: string;
  var: string;
}

export type EntropyCloseError = OreStreamEntropyProgramError;

/**
 * Closes an entropy var account and returns rent to the authority.
 */
export const entropyCloseInstruction = createInstructionHandler<EntropyCloseParams, EntropyCloseError>({
  programId: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X',
  discriminator: [1],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'var', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ORE_STREAM_ENTROPY_PROGRAM_ERRORS,
});

export interface EntropyNextParams {
  endAt: bigint;
  signer: string;
  var: string;
}

export type EntropyNextError = OreStreamEntropyProgramError;

/**
 * Updates the var for the next random value sample.
 * Resets the commit to the previous seed and clears slot_hash, seed, and value.
 */
export const entropyNextInstruction = createInstructionHandler<EntropyNextParams, EntropyNextError>({
  programId: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X',
  discriminator: [2],
  args: [
    { name: 'endAt', type: 'u64' },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'var', isSigner: false, isWritable: true, category: 'userProvided' },
  ],
  errors: ORE_STREAM_ENTROPY_PROGRAM_ERRORS,
});

export interface EntropyRevealParams {
  seed: number[];
  signer: string;
  var: string;
}

export type EntropyRevealError = OreStreamEntropyProgramError;

/**
 * Reveals the seed and finalizes the random value.
 * The seed must hash to the commit stored in the var account.
 */
export const entropyRevealInstruction = createInstructionHandler<EntropyRevealParams, EntropyRevealError>({
  programId: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X',
  discriminator: [4],
  args: [
    { name: 'seed', type: { array: ['u8', 32] } },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'var', isSigner: false, isWritable: true, category: 'userProvided' },
  ],
  errors: ORE_STREAM_ENTROPY_PROGRAM_ERRORS,
});

export interface EntropySampleParams {
  signer: string;
  var: string;
}

export type EntropySampleError = OreStreamEntropyProgramError;

/**
 * Samples the slot hash at the end_at slot.
 * Must be called after the end_at slot has passed.
 */
export const entropySampleInstruction = createInstructionHandler<EntropySampleParams, EntropySampleError>({
  programId: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X',
  discriminator: [5],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'var', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'slotHashesSysvar', isSigner: false, isWritable: false, category: 'known', knownAddress: 'SysvarS1otHashes111111111111111111111111111' },
  ],
  errors: ORE_STREAM_ENTROPY_PROGRAM_ERRORS,
});

// ============================================================================
// View Definition Types (framework-agnostic)
// ============================================================================

export type ViewKeyFields<TKey> = unknown extends TKey
  ? readonly string[]
  : TKey extends object
    ? readonly Extract<keyof TKey, string>[]
    : readonly string[];

/** View definition with embedded entity and state-key types */
export interface ViewDef<T, TMode extends 'state' | 'list', TKey = unknown> {
  readonly mode: TMode;
  readonly view: string;
  readonly keyFields?: ViewKeyFields<TKey>;
  /** Phantom field for type inference - not present at runtime */
  readonly _entity?: T;
  readonly _key?: TKey;
}

/** Helper to create typed state view definitions (keyed lookups) */
function stateView<T, TKey = unknown>(
  view: string,
  keyFields: ViewKeyFields<TKey>
): ViewDef<T, 'state', TKey> {
  return { mode: 'state', view, keyFields } as const;
}

/** Helper to create typed list view definitions (collections) */
function listView<T>(view: string): ViewDef<T, 'list'> {
  return { mode: 'list', view } as const;
}

// ============================================================================
// Stack Definition
// ============================================================================

/** Stack definition for OreStream with 4 entities */
export const ORE_STREAM_STACK_CORE = {
  name: 'ore-stream',
  endpoints: {
    ws: '', // TODO: Set after first deployment or pass useArete(..., { url })
    http: '', // TODO: Set after first deployment or pass useArete(..., { httpUrl })
  },
  views: {
    OreRound: {
      state: stateView<OreRound, { roundId: bigint }>('OreRound/state', ['roundId']),
      list: listView<OreRound>('OreRound/list'),
      latest: listView<OreRound>('OreRound/latest'),
    },
    OreBoard: {
      state: stateView<OreBoard, { address: string }>('OreBoard/state', ['address']),
      list: listView<OreBoard>('OreBoard/list'),
    },
    OreTreasury: {
      state: stateView<OreTreasury, { address: string }>('OreTreasury/state', ['address']),
      list: listView<OreTreasury>('OreTreasury/list'),
    },
    OreMiner: {
      state: stateView<OreMiner, { authority: string }>('OreMiner/state', ['authority']),
      list: listView<OreMiner>('OreMiner/list'),
    },
  },
  schemas: {
    AdminConfig: AdminConfigSchema,
    AutomationConditions: AutomationConditionsSchema,
    Automation: AutomationSchema,
    Board: BoardSchema,
    Config: ConfigSchema,
    Miner: MinerSchema,
    Numeric: NumericSchema,
    OreAutomation: OreAutomationSchema,
    OreBoard2: OreBoard2Schema,
    OreBoardCompleted: OreBoardCompletedSchema,
    OreBoardId: OreBoardIdSchema,
    OreBoard: OreBoardSchema,
    OreBoardState: OreBoardStateSchema,
    OreMiner2: OreMiner2Schema,
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
    OreTreasury2: OreTreasury2Schema,
    OreTreasuryCompleted: OreTreasuryCompletedSchema,
    OreTreasuryId: OreTreasuryIdSchema,
    OreTreasury: OreTreasurySchema,
    OreTreasuryState: OreTreasuryStateSchema,
    ProtocolConfig: ProtocolConfigSchema,
    Round: RoundSchema,
    TokenMetadata: TokenMetadataSchema,
    Treasury: TreasurySchema,
    Var: VarSchema,
  },
  patchSchemas: {
    OreRound: OreRoundPatchSchema,
    OreBoard: OreBoardPatchSchema,
    OreTreasury: OreTreasuryPatchSchema,
    OreMiner: OreMinerPatchSchema,
  },
  programs: {
    ore: {
      name: 'ore',
      programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
      sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:817b583ab4e95b22c94d5a2ed3c519c7672d10048996274d6e183da309f4d24d',
      programSpecHash: 'arete:h1:program-spec:sha256:15f2e0292df1188828dc09afa2b8d4d1475411bf8c91815c19ae2d176647c140',
      idlContentHash: 'arete:h1:idl-content:sha256:47b3625ae54b40c0651153a0d6d337631b4e3428b73f9009a9399af34eb2c764',
      normalizedIdlHash: 'arete:h1:idl-normalized:sha256:137b245aa84f8f759a0d2abbc2459605554219ea73465883c7ff6dc36471b9a8',
      pdas: {
        automation: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('automation'), account('authority')),
        board: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('board')),
        config: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('config')),
        miner: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('miner'), account('authority')),
        treasury: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('treasury')),
      },
      addresses: {
        automation: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('automation'), account('authority')),
        board: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('board')),
        config: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('config')),
        miner: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('miner'), account('authority')),
        treasury: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('treasury')),
      },
      accounts: {
        Automation: programAccountRead<OreAutomation>({ account: 'Automation', schema: OreAutomationSchema }),
        Board: programAccountRead<OreBoard2>({ account: 'Board', schema: OreBoard2Schema }),
        Config: programAccountRead<Config>({ account: 'Config', schema: ConfigSchema }),
        Miner: programAccountRead<OreMiner2>({ account: 'Miner', schema: OreMiner2Schema }),
        Round: programAccountRead<Round>({ account: 'Round', schema: RoundSchema }),
        Treasury: programAccountRead<OreTreasury2>({ account: 'Treasury', schema: OreTreasury2Schema }),
      },
      rawInstructions: {
        automate: oreAutomateInstruction,
        checkpoint: oreCheckpointInstruction,
        claimSol: oreClaimSolInstruction,
        claimOre: oreClaimOreInstruction,
        close: oreCloseInstruction,
        deploy: oreDeployInstruction,
        log: oreLogInstruction,
        reset: oreResetInstruction,
        buyback: oreBuybackInstruction,
        bury: oreBuryInstruction,
        wrap: oreWrapInstruction,
        setAdmin: oreSetAdminInstruction,
        newVar: oreNewVarInstruction,
        reloadSol: oreReloadSolInstruction,
      },
      [PROGRAM_OPERATION_EXTENSIONS]: {
        createOperations() {
          return {
            instructions: {
            automate: instructionOperation(async (params: OreAutomateParams) => {
              const instruction = buildInstruction(oreAutomateInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'automate',
                instruction,
                artifacts: { instruction },
                errors: oreAutomateInstruction.errors,
              });
            }),
            checkpoint: instructionOperation(async (params: OreCheckpointParams) => {
              const instruction = buildInstruction(oreCheckpointInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'checkpoint',
                instruction,
                artifacts: { instruction },
                errors: oreCheckpointInstruction.errors,
              });
            }),
            claimSol: instructionOperation(async (params: OreClaimSolParams) => {
              const instruction = buildInstruction(oreClaimSolInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'claimSol',
                instruction,
                artifacts: { instruction },
                errors: oreClaimSolInstruction.errors,
              });
            }),
            claimOre: instructionOperation(async (params: OreClaimOreParams) => {
              const instruction = buildInstruction(oreClaimOreInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'claimOre',
                instruction,
                artifacts: { instruction },
                errors: oreClaimOreInstruction.errors,
              });
            }),
            close: instructionOperation(async (params: OreCloseParams) => {
              const instruction = buildInstruction(oreCloseInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'close',
                instruction,
                artifacts: { instruction },
                errors: oreCloseInstruction.errors,
              });
            }),
            deploy: instructionOperation(async (params: OreDeployParams) => {
              const instruction = buildInstruction(oreDeployInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'deploy',
                instruction,
                artifacts: { instruction },
                errors: oreDeployInstruction.errors,
              });
            }),
            log: instructionOperation(async (params: OreLogParams) => {
              const instruction = buildInstruction(oreLogInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'log',
                instruction,
                artifacts: { instruction },
                errors: oreLogInstruction.errors,
              });
            }),
            reset: instructionOperation(async (params: OreResetParams) => {
              const instruction = buildInstruction(oreResetInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'reset',
                instruction,
                artifacts: { instruction },
                errors: oreResetInstruction.errors,
              });
            }),
            buyback: instructionOperation(async (params: OreBuybackParams) => {
              const instruction = buildInstruction(oreBuybackInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'buyback',
                instruction,
                artifacts: { instruction },
                errors: oreBuybackInstruction.errors,
              });
            }),
            bury: instructionOperation(async (params: OreBuryParams) => {
              const instruction = buildInstruction(oreBuryInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'bury',
                instruction,
                artifacts: { instruction },
                errors: oreBuryInstruction.errors,
              });
            }),
            wrap: instructionOperation(async (params: OreWrapParams) => {
              const instruction = buildInstruction(oreWrapInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'wrap',
                instruction,
                artifacts: { instruction },
                errors: oreWrapInstruction.errors,
              });
            }),
            setAdmin: instructionOperation(async (params: OreSetAdminParams) => {
              const instruction = buildInstruction(oreSetAdminInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'setAdmin',
                instruction,
                artifacts: { instruction },
                errors: oreSetAdminInstruction.errors,
              });
            }),
            newVar: instructionOperation(async (params: OreNewVarParams) => {
              const instruction = buildInstruction(oreNewVarInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'newVar',
                instruction,
                artifacts: { instruction },
                errors: oreNewVarInstruction.errors,
              });
            }),
            reloadSol: instructionOperation(async (params: OreReloadSolParams) => {
              const instruction = buildInstruction(oreReloadSolInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'reloadSol',
                instruction,
                artifacts: { instruction },
                errors: oreReloadSolInstruction.errors,
              });
            }),
            },
          };
        },
      },
    },
    entropy: {
      name: 'entropy',
      programId: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X',
      sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:de0d12968476603c563d00ca363fc641e1aeabb4b9b6b0d66c720b0499b33cfb',
      programSpecHash: 'arete:h1:program-spec:sha256:b0d48e673ec705cbb6ee41714e660aab9c6398c746b243973fcacd7bc29b7d7b',
      idlContentHash: 'arete:h1:idl-content:sha256:2b5b3ed4de83cd3803bd6b82b33cfbea0e8b7c6a7ada7b138fcb57bb2fe1a01f',
      normalizedIdlHash: 'arete:h1:idl-normalized:sha256:adc67e46a2ffc5e26fcff489fa7e21d5aa0d6338243dc23330ab0e85c3e150fc',
      accounts: {
        Var: programAccountRead<Var>({ account: 'Var', schema: VarSchema }),
      },
      rawInstructions: {
        open: entropyOpenInstruction,
        close: entropyCloseInstruction,
        next: entropyNextInstruction,
        reveal: entropyRevealInstruction,
        sample: entropySampleInstruction,
      },
      [PROGRAM_OPERATION_EXTENSIONS]: {
        createOperations() {
          return {
            instructions: {
            open: instructionOperation(async (params: EntropyOpenParams) => {
              const instruction = buildInstruction(entropyOpenInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'open',
                instruction,
                artifacts: { instruction },
                errors: entropyOpenInstruction.errors,
              });
            }),
            close: instructionOperation(async (params: EntropyCloseParams) => {
              const instruction = buildInstruction(entropyCloseInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'close',
                instruction,
                artifacts: { instruction },
                errors: entropyCloseInstruction.errors,
              });
            }),
            next: instructionOperation(async (params: EntropyNextParams) => {
              const instruction = buildInstruction(entropyNextInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'next',
                instruction,
                artifacts: { instruction },
                errors: entropyNextInstruction.errors,
              });
            }),
            reveal: instructionOperation(async (params: EntropyRevealParams) => {
              const instruction = buildInstruction(entropyRevealInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'reveal',
                instruction,
                artifacts: { instruction },
                errors: entropyRevealInstruction.errors,
              });
            }),
            sample: instructionOperation(async (params: EntropySampleParams) => {
              const instruction = buildInstruction(entropySampleInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'sample',
                instruction,
                artifacts: { instruction },
                errors: entropySampleInstruction.errors,
              });
            }),
            },
          };
        },
      },
    },
  },
  programReads: {
    ore: {
      release: { programReleaseHash: "arete:h1:program-release:sha256:714754ca64a398f5b1614503d393d3179dd95ff072ac68ea6bf5342a9cf3cf7a", programSpecHash: "arete:h1:program-spec:sha256:15f2e0292df1188828dc09afa2b8d4d1475411bf8c91815c19ae2d176647c140" },
      transport: { kind: 'local-http', endpointSource: 'connect-http-url' },
    },
    entropy: {
      release: { programReleaseHash: "arete:h1:program-release:sha256:9e7d6811735b35f9fd144c1eaa21ac1a48720b706d81bd0d0cd9ad6ec7f32b6c", programSpecHash: "arete:h1:program-spec:sha256:b0d48e673ec705cbb6ee41714e660aab9c6398c746b243973fcacd7bc29b7d7b" },
      transport: { kind: 'local-http', endpointSource: 'connect-http-url' },
    },
  },
  addresses: {
    ore: {
      automation: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('automation'), account('authority')),
      board: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('board')),
      config: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('config')),
      miner: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('miner'), account('authority')),
      treasury: pda('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv', literal('treasury')),
    },
  },
} as const;

/** Type alias for the core stack */
export type OreStreamCoreStack = typeof ORE_STREAM_STACK_CORE;

/** Entity types in this stack */
export type OreStreamEntity = OreRound | OreBoard | OreTreasury | OreMiner;

/** Default export for convenience */
export default ORE_STREAM_STACK_CORE;