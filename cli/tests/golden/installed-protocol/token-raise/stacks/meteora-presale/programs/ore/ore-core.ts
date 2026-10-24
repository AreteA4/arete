import { z } from 'zod';
import { programAccountRead, createInstructionHandler, type ErrorMetadata, buildInstruction, PROGRAM_OPERATION_EXTENSIONS, instructionOperation, createPreparedInstruction } from '@usearete/sdk/program';

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
  conditions: AutomationConditions;
}

export interface Board {
  roundId: bigint;
  startSlot: bigint;
  endSlot: bigint;
  productionCostEma: bigint;
}

export interface Config {
  admin: AdminConfig;
  protocol: ProtocolConfig;
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
  totalReturnedSol: bigint;
  totalMiners: bigint;
  topMiner: string;
}

export interface Treasury {
  motherlode: bigint;
  minerRewardsFactor: Numeric;
  totalRefined: bigint;
  totalUnclaimed: bigint;
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

export const ConfigSchema = z.object({
  admin: z.lazy(() => AdminConfigSchema),
  protocol: z.lazy(() => ProtocolConfigSchema),
}).transform((value) => ({
  admin: value.admin,
  protocol: value.protocol,
}));

export const MinerSchema = z.object({
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
  total_returned_sol: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
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
  totalReturnedSol: value.total_returned_sol,
  totalMiners: value.total_miners,
  topMiner: value.top_miner,
}));

export const TreasurySchema = z.object({
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

// ============================================================================
// Instruction Handlers
// ============================================================================

/** Union of all program errors declared across this stack's instructions. */
export type OreProgramError =
  | { code: 0; name: 'AmountTooSmall'; msg: string }
  | { code: 1; name: 'NotAuthorized'; msg: string }
  | { code: 2; name: 'InvalidExecutor'; msg: string };

const ORE_PROGRAM_ERRORS: ErrorMetadata[] = [
  { code: 0, name: 'AmountTooSmall', msg: 'Amount too small' },
  { code: 1, name: 'NotAuthorized', msg: 'Not authorized' },
  { code: 2, name: 'InvalidExecutor', msg: 'Invalid executor' },
];

export interface AutomateParams {
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

export type AutomateError = OreProgramError;

/**
 * Configures or closes a miner automation account.
 * Automation PDA seeds: ["automation", signer].
 * Miner PDA seeds: ["miner", signer].
 */
export const automateInstruction = createInstructionHandler<AutomateParams, AutomateError, 'signer' | 'automation' | 'executor' | 'miner'>({
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
    { name: 'automation', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'executor', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'miner', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ORE_PROGRAM_ERRORS,
});

export interface CheckpointParams {
  signer: string;
  authority: string;
  automation: string;
  board: string;
  miner: string;
  round: string;
  treasury: string;
}

export type CheckpointError = OreProgramError;

/**
 * Settles miner rewards for a completed round.
 * Treasury PDA seeds: ["treasury"].
 */
export const checkpointInstruction = createInstructionHandler<CheckpointParams, CheckpointError, 'signer' | 'authority' | 'automation' | 'board' | 'miner' | 'round' | 'treasury'>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [2],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'authority', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'automation', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'board', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'miner', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'round', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ORE_PROGRAM_ERRORS,
});

export interface ClaimSolParams {
  signer: string;
  board: string;
  miner: string;
}

export type ClaimSolError = OreProgramError;

/**
 * Claims SOL rewards from the miner account.
 */
export const claimSolInstruction = createInstructionHandler<ClaimSolParams, ClaimSolError, 'signer' | 'board' | 'miner'>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [3],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'board', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'miner', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
    { name: 'oreProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv' },
  ],
  errors: ORE_PROGRAM_ERRORS,
});

export interface ClaimOreParams {
  signer: string;
  board: string;
  miner: string;
  recipient: string;
  treasury: string;
  treasuryTokens: string;
}

export type ClaimOreError = OreProgramError;

/**
 * Claims ORE token rewards from the treasury vault.
 */
export const claimOreInstruction = createInstructionHandler<ClaimOreParams, ClaimOreError, 'signer' | 'board' | 'miner' | 'recipient' | 'treasury' | 'treasuryTokens'>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [4],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'board', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'miner', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'mint', isSigner: false, isWritable: true, category: 'known', knownAddress: 'oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp' },
    { name: 'recipient', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasuryTokens', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
    { name: 'tokenProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' },
    { name: 'associatedTokenProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL' },
    { name: 'oreProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv' },
  ],
  errors: ORE_PROGRAM_ERRORS,
});

export interface CloseParams {
  signer: string;
  board: string;
  rentPayer: string;
  round: string;
  treasury: string;
}

export type CloseError = OreProgramError;

/**
 * Closes an expired round account and returns rent to the payer.
 * Round PDA seeds: ["round", round_id].
 * Treasury PDA seeds: ["treasury"].
 */
export const closeInstruction = createInstructionHandler<CloseParams, CloseError, 'signer' | 'board' | 'rentPayer' | 'round' | 'treasury'>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [5],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'board', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'rentPayer', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'round', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ORE_PROGRAM_ERRORS,
});

export interface DeployParams {
  amount: bigint;
  squares: number;
  signer: string;
  authority: string;
  automation: string;
  board: string;
  config: string;
  miner: string;
  round: string;
  treasury: string;
  entropyVar?: string;
  entropyProgram?: string;
}

export type DeployError = OreProgramError;

/**
 * Deploys SOL to selected squares for the current round.
 * Automation PDA seeds: ["automation", authority].
 * Config PDA seeds: ["config"].
 * Miner PDA seeds: ["miner", authority].
 * Round PDA seeds: ["round", board.round_id].
 */
export const deployInstruction = createInstructionHandler<DeployParams, DeployError, 'signer' | 'authority' | 'automation' | 'board' | 'config' | 'miner' | 'round' | 'treasury' | 'entropyVar' | 'entropyProgram'>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [6],
  args: [
    { name: 'amount', type: 'u64' },
    { name: 'squares', type: 'u32' },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'authority', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'automation', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'board', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'config', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'miner', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'round', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
    { name: 'oreProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv' },
    { name: 'entropyVar', isSigner: false, isWritable: true, category: 'userProvided', isOptional: true },
    { name: 'entropyProgram', isSigner: false, isWritable: false, category: 'userProvided', isOptional: true },
  ],
  errors: ORE_PROGRAM_ERRORS,
});

export interface LogParams {
  signer: string;
}

export type LogError = OreProgramError;

/**
 * Emits an arbitrary log message from the board PDA.
 * Bytes following the discriminator are logged verbatim.
 */
export const logInstruction = createInstructionHandler<LogParams, LogError, 'signer'>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [8],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
  ],
  errors: ORE_PROGRAM_ERRORS,
});

export interface ResetParams {
  signer: string;
  board: string;
  config: string;
  feeCollector: string;
  round: string;
  roundNext: string;
  topMiner: string;
  treasury: string;
  treasuryTokens: string;
  entropyVar: string;
  mintAuthority: string;
}

export type ResetError = OreProgramError;

/**
 * Finalizes the current round, mints rewards, and opens the next round.
 * Board PDA seeds: ["board"].
 * Treasury PDA seeds: ["treasury"].
 * Round PDA seeds: ["round", board.round_id] and ["round", board.round_id + 1].
 */
export const resetInstruction = createInstructionHandler<ResetParams, ResetError, 'signer' | 'board' | 'config' | 'feeCollector' | 'round' | 'roundNext' | 'topMiner' | 'treasury' | 'treasuryTokens' | 'entropyVar' | 'mintAuthority'>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [9],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'board', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'config', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'feeCollector', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'mint', isSigner: false, isWritable: true, category: 'known', knownAddress: 'oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp' },
    { name: 'round', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'roundNext', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'topMiner', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'userProvided' },
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
  errors: ORE_PROGRAM_ERRORS,
});

export interface BuryParams {
  amount: bigint;
  signer: string;
  sender: string;
  board: string;
  treasury: string;
  treasuryOre: string;
  stakeTreasury: string;
  stakeTreasuryTokens: string;
  stakeVesting: string;
  oreStakeProgram: string;
}

export type BuryError = OreProgramError;

/**
 * Burns ORE and distributes yield to stakers.
 * Treasury PDA seeds: ["treasury"].
 */
export const buryInstruction = createInstructionHandler<BuryParams, BuryError, 'signer' | 'sender' | 'board' | 'treasury' | 'treasuryOre' | 'stakeTreasury' | 'stakeTreasuryTokens' | 'stakeVesting' | 'oreStakeProgram'>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [24],
  args: [
    { name: 'amount', type: 'u64' },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'sender', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'board', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'mint', isSigner: false, isWritable: true, category: 'known', knownAddress: 'oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasuryOre', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'stakeTreasury', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'stakeTreasuryTokens', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'stakeVesting', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'tokenProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' },
    { name: 'oreProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv' },
    { name: 'oreStakeProgram', isSigner: false, isWritable: false, category: 'userProvided' },
  ],
  errors: ORE_PROGRAM_ERRORS,
});

export interface BuybackParams {
  board: string;
  config: string;
  treasury: string;
  treasuryOre: string;
  treasurySol: string;
  stakeTreasury: string;
  stakeTreasuryOre: string;
  stakeVesting: string;
  oreStakeProgram: string;
}

export type BuybackError = OreProgramError;

/**
 * Swaps vaulted SOL to ORE through Jupiter, distributes staking yield, and burns the remainder.
 * The 13 declared accounts are followed by Jupiter route accounts, and raw Jupiter instruction data follows the discriminator.
 */
export const buybackInstruction = createInstructionHandler<BuybackParams, BuybackError, 'board' | 'config' | 'treasury' | 'treasuryOre' | 'treasurySol' | 'stakeTreasury' | 'stakeTreasuryOre' | 'stakeVesting' | 'oreStakeProgram'>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [13],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'known', knownAddress: 'HNWhK5f8RMWBqcA7mXJPaxdTPGrha3rrqUrri7HSKb3T' },
    { name: 'board', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'config', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'mint', isSigner: false, isWritable: true, category: 'known', knownAddress: 'oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasuryOre', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasurySol', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'stakeTreasury', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'stakeTreasuryOre', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'stakeVesting', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'tokenProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' },
    { name: 'oreProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv' },
    { name: 'oreStakeProgram', isSigner: false, isWritable: false, category: 'userProvided' },
  ],
  errors: ORE_PROGRAM_ERRORS,
});

export interface WrapParams {
  amount: bigint;
  config: string;
  treasury: string;
  treasurySol: string;
}

export type WrapError = OreProgramError;

/**
 * Wraps SOL held by the treasury into WSOL for swapping.
 * Treasury PDA seeds: ["treasury"].
 */
export const wrapInstruction = createInstructionHandler<WrapParams, WrapError, 'config' | 'treasury' | 'treasurySol'>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [14],
  args: [
    { name: 'amount', type: 'u64' },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'known', knownAddress: 'HNWhK5f8RMWBqcA7mXJPaxdTPGrha3rrqUrri7HSKb3T' },
    { name: 'config', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasurySol', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ORE_PROGRAM_ERRORS,
});

export interface SetAdminParams {
  admin: string;
  signer: string;
  config: string;
}

export type SetAdminError = OreProgramError;

/**
 * Updates the program admin address.
 */
export const setAdminInstruction = createInstructionHandler<SetAdminParams, SetAdminError, 'signer' | 'config'>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [15],
  args: [
    { name: 'admin', type: 'pubkey' },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'config', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ORE_PROGRAM_ERRORS,
});

export interface NewVarParams {
  id: bigint;
  commit: number[];
  samples: bigint;
  signer: string;
  board: string;
  config: string;
  provider: string;
  var: string;
}

export type NewVarError = OreProgramError;

/**
 * Creates a new entropy var account through the entropy program.
 */
export const newVarInstruction = createInstructionHandler<NewVarParams, NewVarError, 'signer' | 'board' | 'config' | 'provider' | 'var'>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [17],
  args: [
    { name: 'id', type: 'u64' },
    { name: 'commit', type: { array: ['u8', 32] } },
    { name: 'samples', type: 'u64' },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'board', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'config', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'provider', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'var', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
    { name: 'entropyProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X' },
  ],
  errors: ORE_PROGRAM_ERRORS,
});

// ============================================================================
// Program Definitions
// ============================================================================

/** Standalone program SDK for 'ore' */
export const ORE = {
  name: 'ore',
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:ef9ffcad57827ba42287b07574960cc1967a8d9d984a970c01f840143e39743a',
  programSpecHash: 'arete:h1:program-spec:sha256:41a3e99a926050fd86b09761b829570a0a2086c10766e1b6e328b901dd856f72',
  idlContentHash: 'arete:h1:idl-content:sha256:7a14fb6c2c406d74ac61bf93ff14949da4677a5f6a3d53058e550f1fe86f7bf3',
  normalizedIdlHash: 'arete:h1:idl-normalized:sha256:b16f15a8d4ed1ce44127170eaf2050349573759961c33bd010a4cf59df787157',
  gateway: {"chain":{"auth":{"acceptedKeyClasses":["anonymous","publishable","secret"],"audience":"arete:solana-gateway","jwksUrl":"https://api.example.test/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["read"],"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"sgb_00000000000000000000000000000001","targetKind":"solana-gateway-binding","tokenTransport":"bearer","transactionEntitlementRequired":false},"authPolicy":"signed_session","cluster":"mainnet-beta","endpoint":"https://solana.example.test/gateway/","region":"us-west-1","solanaGatewayBindingId":"sgb_00000000000000000000000000000001"},"transactions":{"auth":{"acceptedKeyClasses":["publishable","secret"],"audience":"arete:solana-gateway","jwksUrl":"https://api.example.test/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["transaction:inspect","transaction:send"],"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"sgb_00000000000000000000000000000001","targetKind":"solana-gateway-binding","tokenTransport":"bearer","transactionEntitlementRequired":true},"authPolicy":"signed_session","cluster":"mainnet-beta","endpoint":"https://solana.example.test/gateway/","region":"us-west-1","solanaGatewayBindingId":"sgb_00000000000000000000000000000001"}},
  accounts: {
    Automation: programAccountRead<Automation>({ account: 'Automation', schema: AutomationSchema }),
    Board: programAccountRead<Board>({ account: 'Board', schema: BoardSchema }),
    Config: programAccountRead<Config>({ account: 'Config', schema: ConfigSchema }),
    Miner: programAccountRead<Miner>({ account: 'Miner', schema: MinerSchema }),
    Round: programAccountRead<Round>({ account: 'Round', schema: RoundSchema }),
    Treasury: programAccountRead<Treasury>({ account: 'Treasury', schema: TreasurySchema }),
  },
  rawInstructions: {
    automate: automateInstruction,
    checkpoint: checkpointInstruction,
    claimSol: claimSolInstruction,
    claimOre: claimOreInstruction,
    close: closeInstruction,
    deploy: deployInstruction,
    log: logInstruction,
    reset: resetInstruction,
    bury: buryInstruction,
    buyback: buybackInstruction,
    wrap: wrapInstruction,
    setAdmin: setAdminInstruction,
    newVar: newVarInstruction,
  },
  [PROGRAM_OPERATION_EXTENSIONS]: {
    createOperations() {
      return {
        instructions: {
        automate: instructionOperation(async (params: AutomateParams) => {
          const instruction = buildInstruction(automateInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'automate',
            instruction,
            artifacts: { instruction },
            errors: automateInstruction.errors,
          });
        }),
        checkpoint: instructionOperation(async (params: CheckpointParams) => {
          const instruction = buildInstruction(checkpointInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'checkpoint',
            instruction,
            artifacts: { instruction },
            errors: checkpointInstruction.errors,
          });
        }),
        claimSol: instructionOperation(async (params: ClaimSolParams) => {
          const instruction = buildInstruction(claimSolInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'claimSol',
            instruction,
            artifacts: { instruction },
            errors: claimSolInstruction.errors,
          });
        }),
        claimOre: instructionOperation(async (params: ClaimOreParams) => {
          const instruction = buildInstruction(claimOreInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'claimOre',
            instruction,
            artifacts: { instruction },
            errors: claimOreInstruction.errors,
          });
        }),
        close: instructionOperation(async (params: CloseParams) => {
          const instruction = buildInstruction(closeInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'close',
            instruction,
            artifacts: { instruction },
            errors: closeInstruction.errors,
          });
        }),
        deploy: instructionOperation(async (params: DeployParams) => {
          const instruction = buildInstruction(deployInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'deploy',
            instruction,
            artifacts: { instruction },
            errors: deployInstruction.errors,
          });
        }),
        log: instructionOperation(async (params: LogParams) => {
          const instruction = buildInstruction(logInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'log',
            instruction,
            artifacts: { instruction },
            errors: logInstruction.errors,
          });
        }),
        reset: instructionOperation(async (params: ResetParams) => {
          const instruction = buildInstruction(resetInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'reset',
            instruction,
            artifacts: { instruction },
            errors: resetInstruction.errors,
          });
        }),
        bury: instructionOperation(async (params: BuryParams) => {
          const instruction = buildInstruction(buryInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'bury',
            instruction,
            artifacts: { instruction },
            errors: buryInstruction.errors,
          });
        }),
        buyback: instructionOperation(async (params: BuybackParams) => {
          const instruction = buildInstruction(buybackInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'buyback',
            instruction,
            artifacts: { instruction },
            errors: buybackInstruction.errors,
          });
        }),
        wrap: instructionOperation(async (params: WrapParams) => {
          const instruction = buildInstruction(wrapInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'wrap',
            instruction,
            artifacts: { instruction },
            errors: wrapInstruction.errors,
          });
        }),
        setAdmin: instructionOperation(async (params: SetAdminParams) => {
          const instruction = buildInstruction(setAdminInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'setAdmin',
            instruction,
            artifacts: { instruction },
            errors: setAdminInstruction.errors,
          });
        }),
        newVar: instructionOperation(async (params: NewVarParams) => {
          const instruction = buildInstruction(newVarInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'newVar',
            instruction,
            artifacts: { instruction },
            errors: newVarInstruction.errors,
          });
        }),
        },
      };
    },
  },
} as const;

/** Instruction shapes for 'ore', by IDL instruction name: argument and account names, and which accounts may be left out */
export const ORE_INSTRUCTION_SHAPES = {
  automate: {
    args: ['amount', 'deposit', 'fee', 'mask', 'strategy', 'reload'],
    accounts: {
      signer: { optional: false, derive: null },
      automation: { optional: false, derive: null },
      executor: { optional: false, derive: null },
      miner: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
    },
  },
  checkpoint: {
    args: [],
    accounts: {
      signer: { optional: false, derive: null },
      authority: { optional: false, derive: null },
      automation: { optional: false, derive: null },
      board: { optional: false, derive: null },
      miner: { optional: false, derive: null },
      round: { optional: false, derive: null },
      treasury: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
    },
  },
  claimSol: {
    args: [],
    accounts: {
      signer: { optional: false, derive: null },
      board: { optional: false, derive: null },
      miner: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
      oreProgram: { optional: false, derive: [] },
    },
  },
  claimOre: {
    args: [],
    accounts: {
      signer: { optional: false, derive: null },
      board: { optional: false, derive: null },
      miner: { optional: false, derive: null },
      mint: { optional: false, derive: [] },
      recipient: { optional: false, derive: null },
      treasury: { optional: false, derive: null },
      treasuryTokens: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
      tokenProgram: { optional: false, derive: [] },
      associatedTokenProgram: { optional: false, derive: [] },
      oreProgram: { optional: false, derive: [] },
    },
  },
  close: {
    args: [],
    accounts: {
      signer: { optional: false, derive: null },
      board: { optional: false, derive: null },
      rentPayer: { optional: false, derive: null },
      round: { optional: false, derive: null },
      treasury: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
    },
  },
  deploy: {
    args: ['amount', 'squares'],
    accounts: {
      signer: { optional: false, derive: null },
      authority: { optional: false, derive: null },
      automation: { optional: false, derive: null },
      board: { optional: false, derive: null },
      config: { optional: false, derive: null },
      miner: { optional: false, derive: null },
      round: { optional: false, derive: null },
      treasury: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
      oreProgram: { optional: false, derive: [] },
      entropyVar: { optional: true, derive: null },
      entropyProgram: { optional: true, derive: null },
    },
  },
  log: {
    args: [],
    accounts: {
      signer: { optional: false, derive: null },
    },
  },
  reset: {
    args: [],
    accounts: {
      signer: { optional: false, derive: null },
      board: { optional: false, derive: null },
      config: { optional: false, derive: null },
      feeCollector: { optional: false, derive: null },
      mint: { optional: false, derive: [] },
      round: { optional: false, derive: null },
      roundNext: { optional: false, derive: null },
      topMiner: { optional: false, derive: null },
      treasury: { optional: false, derive: null },
      treasuryTokens: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
      tokenProgram: { optional: false, derive: [] },
      oreProgram: { optional: false, derive: [] },
      slotHashesSysvar: { optional: false, derive: [] },
      entropyVar: { optional: false, derive: null },
      entropyProgram: { optional: false, derive: [] },
      mintAuthority: { optional: false, derive: null },
      mintProgram: { optional: false, derive: [] },
    },
  },
  bury: {
    args: ['amount'],
    accounts: {
      signer: { optional: false, derive: null },
      sender: { optional: false, derive: null },
      board: { optional: false, derive: null },
      mint: { optional: false, derive: [] },
      treasury: { optional: false, derive: null },
      treasuryOre: { optional: false, derive: null },
      stakeTreasury: { optional: false, derive: null },
      stakeTreasuryTokens: { optional: false, derive: null },
      stakeVesting: { optional: false, derive: null },
      tokenProgram: { optional: false, derive: [] },
      oreProgram: { optional: false, derive: [] },
      oreStakeProgram: { optional: false, derive: null },
    },
  },
  buyback: {
    args: [],
    accounts: {
      signer: { optional: false, derive: [] },
      board: { optional: false, derive: null },
      config: { optional: false, derive: null },
      mint: { optional: false, derive: [] },
      treasury: { optional: false, derive: null },
      treasuryOre: { optional: false, derive: null },
      treasurySol: { optional: false, derive: null },
      stakeTreasury: { optional: false, derive: null },
      stakeTreasuryOre: { optional: false, derive: null },
      stakeVesting: { optional: false, derive: null },
      tokenProgram: { optional: false, derive: [] },
      oreProgram: { optional: false, derive: [] },
      oreStakeProgram: { optional: false, derive: null },
    },
  },
  wrap: {
    args: ['amount'],
    accounts: {
      signer: { optional: false, derive: [] },
      config: { optional: false, derive: null },
      treasury: { optional: false, derive: null },
      treasurySol: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
    },
  },
  setAdmin: {
    args: ['admin'],
    accounts: {
      signer: { optional: false, derive: null },
      config: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
    },
  },
  newVar: {
    args: ['id', 'commit', 'samples'],
    accounts: {
      signer: { optional: false, derive: null },
      board: { optional: false, derive: null },
      config: { optional: false, derive: null },
      provider: { optional: false, derive: null },
      var: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
      entropyProgram: { optional: false, derive: [] },
    },
  },
} as const;

/** Release and explicit read transport for 'ore' */
export const ORE_READ = {
release: { programReleaseHash: "arete:h1:program-release:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", programSpecHash: "arete:h1:program-spec:sha256:41a3e99a926050fd86b09761b829570a0a2086c10766e1b6e328b901dd856f72" },
transport: { kind: 'hosted-binding', binding: { endpoint: "https://reads.example.test/private/", programReadBindingId: "prb_00000000000000000000000000000077", auth: {"acceptedKeyClasses":["publishable","secret"],"mode":"signed_session","required":true,"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"prb_00000000000000000000000000000077","targetKind":"program-read-binding"} } },
} as const;

/** All portable programs from the Ore stack */
export const ORE_PROGRAMS = {
  ore: ORE,
} as const;

/** Parallel release/read metadata keyed identically to ORE_PROGRAMS */
export const ORE_PROGRAM_READS = {
  ore: ORE_READ,
} as const;

export type OrePrograms = typeof ORE_PROGRAMS;

export default ORE_PROGRAMS;