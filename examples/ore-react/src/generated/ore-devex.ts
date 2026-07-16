import {
  buildInstruction,
  createPublicKeySeed,
  createSeed,
  deriveAssociatedTokenAccount,
  findProgramAddressSync,
  formatRawToUi,
  toRawAmount,
  type AmountInput,
  type BuiltInstruction,
  type InstructionHandler,
} from '@usearete/sdk';

import * as low from './ore-stack-core.js';

export const ORE_PROGRAM_ADDRESS =
  'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv';
export const ENTROPY_PROGRAM_ADDRESS =
  '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X';
export const ORE_MINT_ADDRESS =
  'oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp';
export const SYSTEM_PROGRAM_ADDRESS = '11111111111111111111111111111111';
export const SPL_TOKEN_PROGRAM_ADDRESS =
  'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA';
export const PERMISSIONLESS_EXECUTOR_ADDRESS =
  'executor11111111111111111111111111111111112';
export const SPLIT_REWARD_ADDRESS =
  'SpLiT11111111111111111111111111111111111112';
export const ADMIN_FEE_COLLECTOR_ADDRESS =
  'DyB4Kv6V613gp2LWQTq1dwDYHGKuUEoDHnCouGUtxFiX';

export const ORE_DECIMALS = 11;
export const SOL_DECIMALS = 9;
export const GRAMS_PER_ORE = 100_000_000_000n;
export const LAMPORTS_PER_SOL = 1_000_000_000n;
export const BPS_DENOMINATOR = 10_000n;
export const SQUARE_COUNT = 25;
export const ALL_SQUARES_MASK = 2 ** SQUARE_COUNT - 1;
export const ROUND_SLOTS = 150n;
export const INTERMISSION_SLOTS = 35n;
export const ROUND_EXPIRY_SLOTS = 216_000n;
export const CHECKPOINT_BOT_WINDOW_SLOTS = 108_000n;
export const CHECKPOINT_FEE_LAMPORTS = 10_000n;
export const PERMISSIONLESS_EXECUTOR_FEE_LAMPORTS = 5_000n;
export const MOTHERLODE_ODDS_V2_ROUND = 335_000n;
export const U64_MAX = 0xffff_ffff_ffff_ffffn;

export const AutomationStrategy = {
  Random: 0,
  Preferred: 1,
  Discretionary: 2,
} as const;

export type Address = string;
export type AutomationStrategyValue =
  (typeof AutomationStrategy)[keyof typeof AutomationStrategy];
export type SquareIndex =
  | 0
  | 1
  | 2
  | 3
  | 4
  | 5
  | 6
  | 7
  | 8
  | 9
  | 10
  | 11
  | 12
  | 13
  | 14
  | 15
  | 16
  | 17
  | 18
  | 19
  | 20
  | 21
  | 22
  | 23
  | 24;

export type AutomationSelection =
  | { kind: 'random'; squareCount: number }
  | { kind: 'preferred'; squares: readonly number[] }
  | { kind: 'discretionary' };

export type RoundPhase =
  | { kind: 'waitingForStart'; slotsRemaining: bigint }
  | { kind: 'waitingForFirstDeploy' }
  | { kind: 'active'; slotsRemaining: bigint }
  | { kind: 'intermission'; slotsRemaining: bigint }
  | { kind: 'resettable' };

export interface PreparedOreInstruction<
  TParams extends Record<string, unknown> = Record<string, unknown>,
> {
  handler: InstructionHandler<unknown, unknown>;
  params: TParams;
  signers: Record<string, Address>;
}

export interface PrepareDeployInput {
  signer: Address;
  authority?: Address;
  amountPerSquare: bigint;
  squares: readonly number[];
  roundId: bigint;
  automation?: Address;
  board?: Address;
  config?: Address;
  miner?: Address;
  round?: Address;
  entropyVar?: Address;
}

export interface PrepareCheckpointInput {
  signer: Address;
  authority: Address;
  roundId: bigint;
  automation?: Address;
  board?: Address;
  miner?: Address;
  round?: Address;
  treasury?: Address;
}

export interface PrepareClaimOreInput {
  authority: Address;
  bps?: bigint | number;
  board?: Address;
  miner?: Address;
  recipient?: Address;
  treasury?: Address;
  treasuryTokens?: Address;
}

export interface PrepareClaimSolInput {
  authority: Address;
  board?: Address;
  miner?: Address;
}

export interface PrepareConfigureAutomationInput {
  authority: Address;
  executor: Address | 'permissionless';
  amountPerSquare: bigint;
  deposit: bigint;
  executorFee: bigint;
  selection: AutomationSelection;
  reloadWinnings?: boolean;
  automation?: Address;
  miner?: Address;
}

export interface PrepareDisableAutomationInput {
  authority: Address;
  automation?: Address;
  miner?: Address;
}

export interface PrepareCloseExpiredRoundInput {
  signer: Address;
  roundId: bigint;
  rentPayer: Address;
  board?: Address;
  round?: Address;
  treasury?: Address;
}

export interface OreClaimPreview {
  bps: bigint;
  pendingRefinedOre: bigint;
  refinedClaim: bigint;
  unrefinedClaim: bigint;
  refiningFee: bigint;
  netAmount: bigint;
  remainingRefined: bigint;
  remainingUnrefined: bigint;
}

export interface OreCheckpointPreview {
  status:
    | 'alreadyCheckpointed'
    | 'currentRound'
    | 'wrongRound'
    | 'notResolved'
    | 'expired'
    | 'claimable';
  rewardsSol: bigint;
  rewardsOre: bigint;
  botFee: bigint;
  winningSquare: SquareIndex | null;
  solDestination: 'authority' | 'automation' | 'miner' | null;
}

export interface SolClaimPreview {
  checkpointedRewardsSol: bigint;
  unresolvedRewardsSol: bigint;
  totalClaimableSol: bigint;
  checkpoint: OreCheckpointPreview | null;
  action: 'none' | 'claim' | 'checkpoint' | 'checkpointAndClaim';
}

export interface QuoteMinerState {
  roundId: bigint;
  checkpointFee: bigint;
  deployed: readonly bigint[];
}

export interface QuoteAutomationState {
  balance: bigint;
}

export interface QuoteManualDeploymentInput {
  roundId: bigint;
  totalPrincipal: bigint;
  selectedSquares: readonly number[];
  miner?: QuoteMinerState | null;
  automation?: QuoteAutomationState | null;
}

export interface ManualDeploymentQuote {
  roundId: bigint;
  totalPrincipal: bigint;
  requestedSquares: readonly SquareIndex[];
  effectiveSquares: readonly SquareIndex[];
  alreadyDeployedSquares: readonly SquareIndex[];
  existingMinerDeployment: readonly bigint[];
  requestedSquareMask: bigint;
  effectiveSquareMask: bigint;
  requestedSquareCount: number;
  effectiveSquareCount: number;
  amountPerSquare: bigint;
  allocatedPrincipal: bigint;
  roundingRemainder: bigint;
  maximumDeploymentTransfer: bigint;
  unspentPrincipal: bigint;
  checkpointReserve: bigint;
  maximumWalletDebit: bigint;
  hasActiveAutomation: boolean;
  requiresDisableBeforeDeployment: boolean;
  includesNetworkFee: false;
  includesAccountRent: false;
}

export interface QuoteAutomationFundingInput {
  totalPrincipal: bigint;
  rounds: bigint;
  preferredSquares: readonly number[];
  miner?: Pick<QuoteMinerState, 'checkpointFee'> | null;
  automation?: QuoteAutomationState | null;
}

export interface AutomationFundingQuote {
  totalPrincipal: bigint;
  rounds: bigint;
  preferredSquares: readonly SquareIndex[];
  preferredSquareCount: number;
  preferredSquareMask: bigint;
  amountPerSquare: bigint;
  principalPerRound: bigint;
  allocatedPrincipal: bigint;
  roundingRemainder: bigint;
  executorFeePerRound: bigint;
  totalExecutorFees: bigint;
  existingAutomationBalance: bigint;
  targetAutomationBalance: bigint;
  incrementalDeposit: bigint;
  resultingAutomationBalance: bigint;
  checkpointReserve: bigint;
  maximumWalletDebit: bigint;
  existingFundedRounds: bigint;
  existingFundingRemainder: bigint;
  estimatedFundedRounds: bigint;
  fundingRemainder: bigint;
  hasActiveAutomation: boolean;
  requiresDisableBeforeReplacement: boolean;
  includesNetworkFee: false;
  includesAccountRent: false;
}

function deriveProgramAddress(
  seeds: Uint8Array[],
  programId = ORE_PROGRAM_ADDRESS,
): Address {
  return findProgramAddressSync(seeds, programId)[0];
}

function assertU64(value: bigint, label: string): void {
  if (value < 0n || value > U64_MAX) {
    throw new Error(`${label} must be an unsigned 64-bit integer`);
  }
}

function assertPositiveU64(value: bigint, label: string): void {
  assertU64(value, label);
  if (value === 0n) {
    throw new Error(`${label} must be greater than zero`);
  }
}

function checkedU64Add(left: bigint, right: bigint, label: string): bigint {
  const value = left + right;
  assertU64(value, label);
  return value;
}

function checkedU64Multiply(
  left: bigint,
  right: bigint,
  label: string,
): bigint {
  const value = left * right;
  assertU64(value, label);
  return value;
}

function checkpointReserve(
  miner: Pick<QuoteMinerState, 'checkpointFee'> | null | undefined,
): bigint {
  if (miner) {
    assertU64(miner.checkpointFee, 'miner.checkpointFee');
  }
  return !miner || miner.checkpointFee === 0n ? CHECKPOINT_FEE_LAMPORTS : 0n;
}

function normalizeMinerDeployment(
  miner: QuoteMinerState | null | undefined,
  roundId: bigint,
): readonly bigint[] {
  if (!miner) {
    return Array<bigint>(SQUARE_COUNT).fill(0n);
  }
  assertU64(miner.roundId, 'miner.roundId');
  if (miner.deployed.length !== SQUARE_COUNT) {
    throw new Error(`miner.deployed must contain ${SQUARE_COUNT} square amounts`);
  }
  for (const amount of miner.deployed) {
    assertU64(amount, 'miner.deployed amount');
  }
  return miner.roundId === roundId
    ? [...miner.deployed]
    : Array<bigint>(SQUARE_COUNT).fill(0n);
}

export function assertSquareIndex(value: number): asserts value is SquareIndex {
  if (!Number.isInteger(value) || value < 0 || value >= SQUARE_COUNT) {
    throw new Error(`square index must be an integer between 0 and 24, got ${value}`);
  }
}

export function encodeSquareMask(squares: readonly number[]): number {
  if (squares.length === 0) {
    throw new Error('at least one square must be selected');
  }

  let mask = 0;
  for (const square of squares) {
    assertSquareIndex(square);
    const bit = 2 ** square;
    if ((mask & bit) !== 0) {
      throw new Error(`square ${square} was selected more than once`);
    }
    mask += bit;
  }
  return mask;
}

export function decodeSquareMask(mask: number | bigint): SquareIndex[] {
  const value = BigInt(mask);
  if (value < 0n || value > BigInt(ALL_SQUARES_MASK)) {
    throw new Error('square mask may only contain bits 0 through 24');
  }

  const squares: SquareIndex[] = [];
  for (let index = 0; index < SQUARE_COUNT; index += 1) {
    if ((value & (1n << BigInt(index))) !== 0n) {
      squares.push(index as SquareIndex);
    }
  }
  return squares;
}

export function countSquareMask(mask: number | bigint): number {
  return decodeSquareMask(mask).length;
}

export function quoteManualDeployment(
  input: QuoteManualDeploymentInput,
): ManualDeploymentQuote {
  assertU64(input.roundId, 'roundId');
  assertPositiveU64(input.totalPrincipal, 'totalPrincipal');
  const requestedSquareMask = BigInt(encodeSquareMask(input.selectedSquares));
  const requestedSquares = decodeSquareMask(requestedSquareMask);
  const existingMinerDeployment = normalizeMinerDeployment(input.miner, input.roundId);
  const alreadyDeployedSquares = requestedSquares.filter(
    (square) => existingMinerDeployment[square]! > 0n,
  );
  const effectiveSquares = requestedSquares.filter(
    (square) => existingMinerDeployment[square] === 0n,
  );
  const effectiveSquareMask = effectiveSquares.reduce(
    (mask, square) => mask | (1n << BigInt(square)),
    0n,
  );
  const requestedSquareCount = requestedSquares.length;
  const amountPerSquare =
    input.totalPrincipal / BigInt(requestedSquareCount);
  if (amountPerSquare === 0n) {
    throw new Error('totalPrincipal must allocate at least one lamport per selected square');
  }
  assertU64(amountPerSquare, 'amountPerSquare');
  const allocatedPrincipal = checkedU64Multiply(
    amountPerSquare,
    BigInt(requestedSquareCount),
    'allocatedPrincipal',
  );
  const maximumDeploymentTransfer = checkedU64Multiply(
    amountPerSquare,
    BigInt(effectiveSquares.length),
    'maximumDeploymentTransfer',
  );
  const reserve = checkpointReserve(input.miner);
  const maximumWalletDebit = checkedU64Add(
    maximumDeploymentTransfer,
    reserve,
    'maximumWalletDebit',
  );
  const hasActiveAutomation = input.automation != null;

  return {
    roundId: input.roundId,
    totalPrincipal: input.totalPrincipal,
    requestedSquares,
    effectiveSquares,
    alreadyDeployedSquares,
    existingMinerDeployment,
    requestedSquareMask,
    effectiveSquareMask,
    requestedSquareCount,
    effectiveSquareCount: effectiveSquares.length,
    amountPerSquare,
    allocatedPrincipal,
    roundingRemainder: input.totalPrincipal - allocatedPrincipal,
    maximumDeploymentTransfer,
    unspentPrincipal: input.totalPrincipal - maximumDeploymentTransfer,
    checkpointReserve: reserve,
    maximumWalletDebit,
    hasActiveAutomation,
    requiresDisableBeforeDeployment: hasActiveAutomation,
    includesNetworkFee: false,
    includesAccountRent: false,
  };
}

export function quoteAutomationFunding(
  input: QuoteAutomationFundingInput,
): AutomationFundingQuote {
  assertPositiveU64(input.totalPrincipal, 'totalPrincipal');
  assertPositiveU64(input.rounds, 'rounds');
  const preferredSquareMask = BigInt(encodeSquareMask(input.preferredSquares));
  const preferredSquares = decodeSquareMask(preferredSquareMask);
  const preferredSquareCount = preferredSquares.length;
  const amountPerSquare =
    input.totalPrincipal /
    (input.rounds * BigInt(preferredSquareCount));
  if (amountPerSquare === 0n) {
    throw new Error(
      'totalPrincipal must allocate at least one lamport per preferred square and funded round',
    );
  }
  assertU64(amountPerSquare, 'amountPerSquare');
  const principalPerRound = checkedU64Multiply(
    amountPerSquare,
    BigInt(preferredSquareCount),
    'principalPerRound',
  );
  const allocatedPrincipal = checkedU64Multiply(
    principalPerRound,
    input.rounds,
    'allocatedPrincipal',
  );
  const costPerRound = checkedU64Add(
    principalPerRound,
    PERMISSIONLESS_EXECUTOR_FEE_LAMPORTS,
    'automation cost per round',
  );
  const totalExecutorFees = checkedU64Multiply(
    PERMISSIONLESS_EXECUTOR_FEE_LAMPORTS,
    input.rounds,
    'totalExecutorFees',
  );
  const targetAutomationBalance = checkedU64Multiply(
    costPerRound,
    input.rounds,
    'targetAutomationBalance',
  );
  const existingAutomationBalance = input.automation?.balance ?? 0n;
  assertU64(existingAutomationBalance, 'automation.balance');
  const incrementalDeposit =
    targetAutomationBalance > existingAutomationBalance
      ? targetAutomationBalance - existingAutomationBalance
      : 0n;
  const resultingAutomationBalance = checkedU64Add(
    existingAutomationBalance,
    incrementalDeposit,
    'resultingAutomationBalance',
  );
  const reserve = checkpointReserve(input.miner);
  const maximumWalletDebit = checkedU64Add(
    incrementalDeposit,
    reserve,
    'maximumWalletDebit',
  );
  const hasActiveAutomation = input.automation != null;

  return {
    totalPrincipal: input.totalPrincipal,
    rounds: input.rounds,
    preferredSquares,
    preferredSquareCount,
    preferredSquareMask,
    amountPerSquare,
    principalPerRound,
    allocatedPrincipal,
    roundingRemainder: input.totalPrincipal - allocatedPrincipal,
    executorFeePerRound: PERMISSIONLESS_EXECUTOR_FEE_LAMPORTS,
    totalExecutorFees,
    existingAutomationBalance,
    targetAutomationBalance,
    incrementalDeposit,
    resultingAutomationBalance,
    checkpointReserve: reserve,
    maximumWalletDebit,
    existingFundedRounds: existingAutomationBalance / costPerRound,
    existingFundingRemainder: existingAutomationBalance % costPerRound,
    estimatedFundedRounds: resultingAutomationBalance / costPerRound,
    fundingRemainder: resultingAutomationBalance % costPerRound,
    hasActiveAutomation,
    requiresDisableBeforeReplacement: hasActiveAutomation,
    includesNetworkFee: false,
    includesAccountRent: false,
  };
}

export function getBoardPda(): Address {
  return deriveProgramAddress([createSeed('board')]);
}

export function getConfigPda(): Address {
  return deriveProgramAddress([createSeed('config')]);
}

export function getTreasuryPda(): Address {
  return deriveProgramAddress([createSeed('treasury')]);
}

export function getMinerPda(authority: Address): Address {
  return deriveProgramAddress([
    createSeed('miner'),
    createPublicKeySeed(authority),
  ]);
}

export function getAutomationPda(authority: Address): Address {
  return deriveProgramAddress([
    createSeed('automation'),
    createPublicKeySeed(authority),
  ]);
}

export function getRoundPda(roundId: bigint): Address {
  assertU64(roundId, 'roundId');
  return deriveProgramAddress([createSeed('round'), createSeed(roundId)]);
}

export function getEntropyVarPda(): Address {
  return deriveProgramAddress(
    [
      createSeed('var'),
      createPublicKeySeed(getBoardPda()),
      createSeed(0n),
    ],
    ENTROPY_PROGRAM_ADDRESS,
  );
}

export function getOreTokenAccount(owner: Address): Address {
  return deriveAssociatedTokenAccount({
    owner,
    mint: ORE_MINT_ADDRESS,
    tokenProgram: SPL_TOKEN_PROGRAM_ADDRESS,
  });
}

export function getTreasuryOreTokenAccount(): Address {
  return getOreTokenAccount(getTreasuryPda());
}

export function getRoundPhase(
  board: Pick<low.Board, 'startSlot' | 'endSlot'>,
  currentSlot: bigint,
): RoundPhase {
  if (board.endSlot === U64_MAX) {
    if (currentSlot < board.startSlot) {
      return {
        kind: 'waitingForStart',
        slotsRemaining: board.startSlot - currentSlot,
      };
    }
    return { kind: 'waitingForFirstDeploy' };
  }
  if (currentSlot < board.endSlot) {
    return { kind: 'active', slotsRemaining: board.endSlot - currentSlot };
  }

  const resetSlot = board.endSlot + INTERMISSION_SLOTS;
  if (currentSlot < resetSlot) {
    return { kind: 'intermission', slotsRemaining: resetSlot - currentSlot };
  }
  return { kind: 'resettable' };
}

function readU64Le(bytes: readonly number[], offset: number): bigint {
  let value = 0n;
  for (let index = 0; index < 8; index += 1) {
    value |= BigInt(bytes[offset + index]!) << BigInt(index * 8);
  }
  return value;
}

export function rngFromEntropyValue(bytes: readonly number[]): bigint | null {
  if (bytes.length !== 32) {
    throw new Error(`entropy value must contain 32 bytes, got ${bytes.length}`);
  }
  for (const byte of bytes) {
    if (!Number.isInteger(byte) || byte < 0 || byte > 255) {
      throw new Error('entropy value bytes must be integers between 0 and 255');
    }
  }
  if (bytes.every((byte) => byte === 0) || bytes.every((byte) => byte === 255)) {
    return null;
  }
  return (
    readU64Le(bytes, 0) ^
    readU64Le(bytes, 8) ^
    readU64Le(bytes, 16) ^
    readU64Le(bytes, 24)
  );
}

export function reverseBits64(value: bigint): bigint {
  assertU64(value, 'value');
  let source = value;
  let reversed = 0n;
  for (let index = 0; index < 64; index += 1) {
    reversed = (reversed << 1n) | (source & 1n);
    source >>= 1n;
  }
  return reversed;
}

export function getWinningSquare(rng: bigint): SquareIndex {
  assertU64(rng, 'rng');
  return Number(rng % BigInt(SQUARE_COUNT)) as SquareIndex;
}

export function getTopMinerSample(
  rng: bigint,
  deployedOnWinningSquare: bigint,
): bigint {
  if (deployedOnWinningSquare < 0n) {
    throw new Error('deployedOnWinningSquare must be non-negative');
  }
  if (deployedOnWinningSquare === 0n) {
    return 0n;
  }
  return reverseBits64(rng) % deployedOnWinningSquare;
}

export function isSplitReward(rng: bigint): boolean {
  const reversed = reverseBits64(rng);
  const folded =
    Number(reversed & 0xffffn) ^
    Number((reversed >> 16n) & 0xffffn) ^
    Number((reversed >> 32n) & 0xffffn) ^
    Number((reversed >> 48n) & 0xffffn);
  return folded % 2 === 0;
}

export function didHitMotherlode(rng: bigint, roundId: bigint): boolean {
  assertU64(roundId, 'roundId');
  const odds = roundId >= MOTHERLODE_ODDS_V2_ROUND ? 500n : 625n;
  return reverseBits64(rng) % odds === 0n;
}

function numericToRaw(value: low.Numeric): bigint {
  if (value.bits.length !== 16) {
    throw new Error(`Numeric must contain 16 bytes, got ${value.bits.length}`);
  }
  let raw = 0n;
  for (let index = 0; index < 16; index += 1) {
    const byte = value.bits[index]!;
    if (!Number.isInteger(byte) || byte < 0 || byte > 255) {
      throw new Error('Numeric bytes must be integers between 0 and 255');
    }
    raw |= BigInt(byte) << BigInt(index * 8);
  }
  return (raw & (1n << 127n)) === 0n ? raw : raw - (1n << 128n);
}

export function previewOreClaim(
  miner: low.OreMiner2,
  treasury: low.OreTreasury2,
  bps: bigint | number = BPS_DENOMINATOR,
): OreClaimPreview {
  const normalizedBps = resolveBps(bps);

  const rewardsFactorDelta =
    numericToRaw(treasury.minerRewardsFactor) - numericToRaw(miner.rewardsFactor);
  const pendingRefinedOre =
    rewardsFactorDelta > 0n
      ? (rewardsFactorDelta * miner.rewardsOre) >> 48n
      : 0n;
  const updatedRefined = miner.refinedOre + pendingRefinedOre;
  const refinedClaim = (updatedRefined * normalizedBps) / BPS_DENOMINATOR;
  const unrefinedClaim = (miner.rewardsOre * normalizedBps) / BPS_DENOMINATOR;
  const remainingUnrefined = miner.rewardsOre - unrefinedClaim;
  const treasuryUnclaimedAfter = treasury.totalUnclaimed - unrefinedClaim;
  const refiningFee =
    unrefinedClaim > 0n && treasuryUnclaimedAfter > 0n
      ? 1n > unrefinedClaim / 10n
        ? 1n
        : unrefinedClaim / 10n
      : 0n;

  return {
    bps: normalizedBps,
    pendingRefinedOre,
    refinedClaim,
    unrefinedClaim,
    refiningFee,
    netAmount: refinedClaim + unrefinedClaim - refiningFee,
    remainingRefined: updatedRefined - refinedClaim,
    remainingUnrefined,
  };
}

export function previewCheckpoint(input: {
  miner: low.OreMiner2;
  round: low.Round;
  boardRoundId: bigint;
  currentSlot: bigint;
  automation?: low.OreAutomation | null;
}): OreCheckpointPreview {
  const empty = {
    rewardsSol: 0n,
    rewardsOre: 0n,
    botFee: 0n,
    winningSquare: null,
    solDestination: null,
  } as const;
  if (input.miner.checkpointId === input.miner.roundId) {
    return { status: 'alreadyCheckpointed', ...empty };
  }
  if (input.round.id === input.boardRoundId) {
    return { status: 'currentRound', ...empty };
  }
  if (input.round.id !== input.miner.roundId) {
    return { status: 'wrongRound', ...empty };
  }
  if (input.round.slotHash.every((byte) => byte === 0)) {
    return { status: 'notResolved', ...empty };
  }
  if (input.currentSlot >= input.round.expiresAt) {
    return { status: 'expired', ...empty };
  }

  const botFeeWindowStart =
    input.round.expiresAt > CHECKPOINT_BOT_WINDOW_SLOTS
      ? input.round.expiresAt - CHECKPOINT_BOT_WINDOW_SLOTS
      : 0n;
  const botFee =
    input.currentSlot >= botFeeWindowStart ? input.miner.checkpointFee : 0n;
  const rng = rngFromEntropyValue(input.round.slotHash);
  let rewardsSol = 0n;
  let rewardsOre = 0n;
  let winningSquare: SquareIndex | null = null;

  if (rng === null) {
    rewardsSol = input.miner.deployed.reduce((sum, amount) => sum + amount, 0n);
  } else {
    winningSquare = getWinningSquare(rng);
    const minerDeployed = input.miner.deployed[winningSquare]!;
    const roundDeployed = input.round.deployed[winningSquare]!;
    if (minerDeployed > 0n && roundDeployed > 0n) {
      const adminFee = minerDeployed / 100n > 1n ? minerDeployed / 100n : 1n;
      rewardsSol =
        minerDeployed -
        adminFee +
        (input.round.totalWinnings * minerDeployed) / roundDeployed;

      if (input.round.topMiner === SPLIT_REWARD_ADDRESS) {
        const roundReward = input.round.rewards.reduce(
          (sum, amount) => sum + amount,
          0n,
        );
        rewardsOre += (roundReward * minerDeployed) / roundDeployed;
      } else {
        const sample = getTopMinerSample(rng, roundDeployed);
        const cumulative = input.miner.cumulative[winningSquare]!;
        if (sample >= cumulative && sample < cumulative + minerDeployed) {
          rewardsOre += input.round.rewards.reduce(
            (sum, amount) => sum + amount,
            0n,
          );
        }
      }
      rewardsOre += (input.round.motherlode * minerDeployed) / roundDeployed;
    }
  }

  const solDestination =
    rewardsSol === 0n
      ? null
      : input.automation && input.automation.reload > 0n
        ? 'automation'
        : input.miner.autoReturn > 0n
          ? 'authority'
          : 'miner';

  return {
    status: 'claimable',
    rewardsSol,
    rewardsOre,
    botFee,
    winningSquare,
    solDestination,
  };
}

export function previewSolClaim(input: {
  checkpointedRewardsSol: bigint;
  checkpoint?: OreCheckpointPreview | null;
}): SolClaimPreview {
  const checkpoint = input.checkpoint ?? null;
  const unresolvedRewardsSol = checkpoint?.status === 'claimable'
    ? checkpoint.rewardsSol
    : 0n;
  const requiresCheckpoint = unresolvedRewardsSol > 0n;
  const requiresClaim = input.checkpointedRewardsSol > 0n
    || (requiresCheckpoint && checkpoint?.solDestination === 'miner');
  const action = requiresCheckpoint
    ? requiresClaim ? 'checkpointAndClaim' : 'checkpoint'
    : requiresClaim ? 'claim' : 'none';
  return {
    checkpointedRewardsSol: input.checkpointedRewardsSol,
    unresolvedRewardsSol,
    totalClaimableSol: input.checkpointedRewardsSol + unresolvedRewardsSol,
    checkpoint,
    action,
  };
}

export function resolveSolAmount(amount: AmountInput): bigint {
  const resolved = toRawAmount(amount, SOL_DECIMALS);
  assertU64(resolved, 'SOL amount');
  return resolved;
}

export function resolveBps(
  value: bigint | number = BPS_DENOMINATOR,
): bigint {
  if (typeof value === 'number' && !Number.isInteger(value)) {
    throw new Error('bps must be an integer between 0 and 10000');
  }
  const bps = BigInt(value);
  if (bps < 0n || bps > BPS_DENOMINATOR) {
    throw new Error('bps must be an integer between 0 and 10000');
  }
  return bps;
}

export function formatSolAmount(amount: bigint): string {
  return formatRawToUi(amount, SOL_DECIMALS);
}

export function formatOreAmount(amount: bigint): string {
  return formatRawToUi(amount, ORE_DECIMALS);
}

export function buildPreparedOreInstruction(
  prepared: PreparedOreInstruction<Record<string, unknown>>,
): BuiltInstruction {
  return buildInstruction(prepared.handler, {
    ...prepared.params,
    ...prepared.signers,
  });
}

export function getRequiredSignerAddresses(
  prepared:
    | PreparedOreInstruction<Record<string, unknown>>
    | readonly PreparedOreInstruction<Record<string, unknown>>[],
): Address[] {
  const instructions = (
    Array.isArray(prepared) ? prepared : [prepared]
  ) as readonly PreparedOreInstruction<Record<string, unknown>>[];
  return [
    ...new Set(instructions.flatMap((instruction) => Object.values(instruction.signers))),
  ];
}

export function prepareDeploy(
  input: PrepareDeployInput,
): PreparedOreInstruction<Record<string, unknown>> {
  assertPositiveU64(input.amountPerSquare, 'amountPerSquare');
  const authority = input.authority ?? input.signer;
  return {
    handler: low.oreDeployInstruction,
    params: {
      amount: input.amountPerSquare,
      squares: encodeSquareMask(input.squares),
      authority,
      automation: input.automation ?? getAutomationPda(authority),
      board: input.board ?? getBoardPda(),
      config: input.config ?? getConfigPda(),
      miner: input.miner ?? getMinerPda(authority),
      round: input.round ?? getRoundPda(input.roundId),
      entropyVar: input.entropyVar ?? getEntropyVarPda(),
    },
    signers: { signer: input.signer },
  };
}

export function prepareCheckpoint(
  input: PrepareCheckpointInput,
): PreparedOreInstruction<Record<string, unknown>> {
  return {
    handler: low.oreCheckpointInstruction,
    params: {
      authority: input.authority,
      automation: input.automation ?? getAutomationPda(input.authority),
      board: input.board ?? getBoardPda(),
      miner: input.miner ?? getMinerPda(input.authority),
      round: input.round ?? getRoundPda(input.roundId),
      treasury: input.treasury ?? getTreasuryPda(),
    },
    signers: { signer: input.signer },
  };
}

export function prepareClaimOre(
  input: PrepareClaimOreInput,
): PreparedOreInstruction<Record<string, unknown>> {
  const bps = resolveBps(input.bps);
  const treasury = input.treasury ?? getTreasuryPda();
  return {
    handler: low.oreClaimOreInstruction,
    params: {
      bps,
      board: input.board ?? getBoardPda(),
      miner: input.miner ?? getMinerPda(input.authority),
      recipient: input.recipient ?? getOreTokenAccount(input.authority),
      treasury,
      treasuryTokens:
        input.treasuryTokens ?? getOreTokenAccount(treasury),
    },
    signers: { signer: input.authority },
  };
}

export function prepareClaimSol(
  input: PrepareClaimSolInput,
): PreparedOreInstruction<Record<string, unknown>> {
  return {
    handler: low.oreClaimSolInstruction,
    params: {
      board: input.board ?? getBoardPda(),
      miner: input.miner ?? getMinerPda(input.authority),
    },
    signers: { signer: input.authority },
  };
}

function encodeAutomationSelection(selection: AutomationSelection): {
  strategy: AutomationStrategyValue;
  mask: bigint;
} {
  switch (selection.kind) {
    case 'random':
      if (
        !Number.isInteger(selection.squareCount) ||
        selection.squareCount < 1 ||
        selection.squareCount > SQUARE_COUNT
      ) {
        throw new Error('random squareCount must be an integer between 1 and 25');
      }
      return {
        strategy: AutomationStrategy.Random,
        mask: BigInt(selection.squareCount),
      };
    case 'preferred':
      return {
        strategy: AutomationStrategy.Preferred,
        mask: BigInt(encodeSquareMask(selection.squares)),
      };
    case 'discretionary':
      return { strategy: AutomationStrategy.Discretionary, mask: 0n };
  }
}

export function prepareConfigureAutomation(
  input: PrepareConfigureAutomationInput,
): PreparedOreInstruction<Record<string, unknown>> {
  assertPositiveU64(input.amountPerSquare, 'amountPerSquare');
  assertU64(input.deposit, 'deposit');
  assertU64(input.executorFee, 'executorFee');
  const executor =
    input.executor === 'permissionless'
      ? PERMISSIONLESS_EXECUTOR_ADDRESS
      : input.executor;
  const selection = encodeAutomationSelection(input.selection);
  if (
    selection.strategy === AutomationStrategy.Discretionary &&
    executor === PERMISSIONLESS_EXECUTOR_ADDRESS
  ) {
    throw new Error('discretionary automation requires an explicit executor');
  }

  return {
    handler: low.oreAutomateInstruction,
    params: {
      amount: input.amountPerSquare,
      deposit: input.deposit,
      fee: input.executorFee,
      mask: selection.mask,
      strategy: selection.strategy,
      reload: input.reloadWinnings ? 1n : 0n,
      automation: input.automation ?? getAutomationPda(input.authority),
      executor,
      miner: input.miner ?? getMinerPda(input.authority),
    },
    signers: { signer: input.authority },
  };
}

export function prepareDisableAutomation(
  input: PrepareDisableAutomationInput,
): PreparedOreInstruction<Record<string, unknown>> {
  return {
    handler: low.oreAutomateInstruction,
    params: {
      amount: 0n,
      deposit: 0n,
      fee: 0n,
      mask: 0n,
      strategy: AutomationStrategy.Random,
      reload: 0n,
      automation: input.automation ?? getAutomationPda(input.authority),
      executor: SYSTEM_PROGRAM_ADDRESS,
      miner: input.miner ?? getMinerPda(input.authority),
    },
    signers: { signer: input.authority },
  };
}

export function prepareCloseExpiredRound(
  input: PrepareCloseExpiredRoundInput,
): PreparedOreInstruction<Record<string, unknown>> {
  return {
    handler: low.oreCloseInstruction,
    params: {
      board: input.board ?? getBoardPda(),
      rentPayer: input.rentPayer,
      round: input.round ?? getRoundPda(input.roundId),
      treasury: input.treasury ?? getTreasuryPda(),
    },
    signers: { signer: input.signer },
  };
}

export const oreDevex = {
  constants: {
    ORE_PROGRAM_ADDRESS,
    ENTROPY_PROGRAM_ADDRESS,
    ORE_MINT_ADDRESS,
    SYSTEM_PROGRAM_ADDRESS,
    SPL_TOKEN_PROGRAM_ADDRESS,
    PERMISSIONLESS_EXECUTOR_ADDRESS,
    SPLIT_REWARD_ADDRESS,
    ADMIN_FEE_COLLECTOR_ADDRESS,
    ORE_DECIMALS,
    SOL_DECIMALS,
    GRAMS_PER_ORE,
    LAMPORTS_PER_SOL,
    BPS_DENOMINATOR,
    SQUARE_COUNT,
    ALL_SQUARES_MASK,
    ROUND_SLOTS,
    INTERMISSION_SLOTS,
    ROUND_EXPIRY_SLOTS,
    CHECKPOINT_BOT_WINDOW_SLOTS,
    CHECKPOINT_FEE_LAMPORTS,
    PERMISSIONLESS_EXECUTOR_FEE_LAMPORTS,
    MOTHERLODE_ODDS_V2_ROUND,
    U64_MAX,
    AutomationStrategy,
  },
  addresses: {
    board: getBoardPda,
    config: getConfigPda,
    treasury: getTreasuryPda,
    treasuryOreToken: getTreasuryOreTokenAccount,
    miner: getMinerPda,
    automation: getAutomationPda,
    round: getRoundPda,
    entropyVar: getEntropyVarPda,
    oreTokenAccount: getOreTokenAccount,
  },
  math: {
    quoteManualDeployment,
    quoteAutomationFunding,
    amounts: {
      resolveSol: resolveSolAmount,
      resolveBps,
      formatSol: formatSolAmount,
      formatOre: formatOreAmount,
    },
    squares: {
      encode: encodeSquareMask,
      decode: decodeSquareMask,
      count: countSquareMask,
    },
    round: {
      phase: getRoundPhase,
      rng: rngFromEntropyValue,
      winningSquare: getWinningSquare,
      topMinerSample: getTopMinerSample,
      isSplitReward,
      didHitMotherlode,
    },
    miner: {
      previewCheckpoint,
      previewSolClaim,
      previewOreClaim,
    },
  },
  prepare: {
    deploy: prepareDeploy,
    checkpoint: prepareCheckpoint,
    claimOre: prepareClaimOre,
    claimSol: prepareClaimSol,
    configureAutomation: prepareConfigureAutomation,
    disableAutomation: prepareDisableAutomation,
    closeExpiredRound: prepareCloseExpiredRound,
  },
  buildPreparedInstruction: buildPreparedOreInstruction,
  getRequiredSignerAddresses,
} as const;

export default oreDevex;
