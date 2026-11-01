import { z } from 'zod';
import { pda, literal, arg, programAccountRead, createInstructionHandler, type ErrorMetadata, buildInstruction, PROGRAM_OPERATION_EXTENSIONS, instructionOperation, createPreparedInstruction } from '@usearete/sdk/program';

export interface Header {
  discriminator: number;
  version: number;
  bump: number;
  delegator: string;
  delegatee: string;
  payer: string;
  initId: bigint;
}

export interface PlanData {
  planId: bigint;
  mint: string;
  terms: PlanTerms;
  endTs: bigint;
  destinations: string[];
  pullers: string[];
  metadataUri: number[];
}

export interface PlanTerms {
  amount: bigint;
  periodHours: bigint;
  createdAt: bigint;
}

export interface FixedDelegation {
  header: Header;
  subscriptionAuthority: string;
  mint: string;
  amount: bigint;
  expiryTs: bigint;
}

export interface Plan {
  discriminator: number;
  owner: string;
  bump: number;
  status: number;
  data: PlanData;
}

export interface RecurringDelegation {
  header: Header;
  subscriptionAuthority: string;
  mint: string;
  currentPeriodStartTs: bigint;
  periodLengthS: bigint;
  expiryTs: bigint;
  amountPerPeriod: bigint;
  amountPulledInPeriod: bigint;
}

export interface SubscriptionAuthority {
  discriminator: number;
  user: string;
  tokenMint: string;
  payer: string;
  bump: number;
  initId: bigint;
}

export interface SubscriptionDelegation {
  header: Header;
  terms: PlanTerms;
  amountPulledInPeriod: bigint;
  currentPeriodStartTs: bigint;
  expiresAtTs: bigint;
}

export const HeaderSchema = z.object({
  discriminator: z.number(),
  version: z.number(),
  bump: z.number(),
  delegator: z.string(),
  delegatee: z.string(),
  payer: z.string(),
  init_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  discriminator: value.discriminator,
  version: value.version,
  bump: value.bump,
  delegator: value.delegator,
  delegatee: value.delegatee,
  payer: value.payer,
  initId: value.init_id,
}));

export const PlanDataSchema = z.object({
  plan_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  mint: z.string(),
  terms: z.lazy(() => PlanTermsSchema),
  end_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  destinations: z.array(z.string()).length(4),
  pullers: z.array(z.string()).length(4),
  metadata_uri: z.array(z.number()).length(128),
}).transform((value) => ({
  planId: value.plan_id,
  mint: value.mint,
  terms: value.terms,
  endTs: value.end_ts,
  destinations: value.destinations,
  pullers: value.pullers,
  metadataUri: value.metadata_uri,
}));

export const PlanTermsSchema = z.object({
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  period_hours: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  created_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  amount: value.amount,
  periodHours: value.period_hours,
  createdAt: value.created_at,
}));

export const FixedDelegationSchema = z.object({
  header: z.lazy(() => HeaderSchema),
  subscription_authority: z.string(),
  mint: z.string(),
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  expiry_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  header: value.header,
  subscriptionAuthority: value.subscription_authority,
  mint: value.mint,
  amount: value.amount,
  expiryTs: value.expiry_ts,
}));

export const PlanSchema = z.object({
  discriminator: z.number(),
  owner: z.string(),
  bump: z.number(),
  status: z.number(),
  data: z.lazy(() => PlanDataSchema),
}).transform((value) => ({
  discriminator: value.discriminator,
  owner: value.owner,
  bump: value.bump,
  status: value.status,
  data: value.data,
}));

export const RecurringDelegationSchema = z.object({
  header: z.lazy(() => HeaderSchema),
  subscription_authority: z.string(),
  mint: z.string(),
  current_period_start_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  period_length_s: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  expiry_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  amount_per_period: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  amount_pulled_in_period: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  header: value.header,
  subscriptionAuthority: value.subscription_authority,
  mint: value.mint,
  currentPeriodStartTs: value.current_period_start_ts,
  periodLengthS: value.period_length_s,
  expiryTs: value.expiry_ts,
  amountPerPeriod: value.amount_per_period,
  amountPulledInPeriod: value.amount_pulled_in_period,
}));

export const SubscriptionAuthoritySchema = z.object({
  discriminator: z.number(),
  user: z.string(),
  token_mint: z.string(),
  payer: z.string(),
  bump: z.number(),
  init_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  discriminator: value.discriminator,
  user: value.user,
  tokenMint: value.token_mint,
  payer: value.payer,
  bump: value.bump,
  initId: value.init_id,
}));

export const SubscriptionDelegationSchema = z.object({
  header: z.lazy(() => HeaderSchema),
  terms: z.lazy(() => PlanTermsSchema),
  amount_pulled_in_period: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  current_period_start_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  expires_at_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  header: value.header,
  terms: value.terms,
  amountPulledInPeriod: value.amount_pulled_in_period,
  currentPeriodStartTs: value.current_period_start_ts,
  expiresAtTs: value.expires_at_ts,
}));

// ============================================================================
// Instruction Handlers
// ============================================================================

/** Union of all program errors declared across this stack's instructions. */
export type SubscriptionsProgramError =
  | { code: 100; name: 'notSigner'; msg: string }
  | { code: 101; name: 'invalidAddress'; msg: string }
  | { code: 102; name: 'invalidEscrowPda'; msg: string }
  | { code: 103; name: 'invalidSubscriptionAuthorityPda'; msg: string }
  | { code: 104; name: 'notSystemProgram'; msg: string }
  | { code: 105; name: 'invalidTokenProgram'; msg: string }
  | { code: 106; name: 'invalidToken2022MintAccountData'; msg: string }
  | { code: 107; name: 'invalidToken2022TokenAccountData'; msg: string }
  | { code: 108; name: 'invalidAssociatedTokenAccountDerivedAddress'; msg: string }
  | { code: 109; name: 'invalidTokenSplMintAccountData'; msg: string }
  | { code: 110; name: 'invalidTokenSplTokenAccountData'; msg: string }
  | { code: 111; name: 'invalidAccountData'; msg: string }
  | { code: 112; name: 'invalidInstructionData'; msg: string }
  | { code: 113; name: 'notEnoughAccountKeys'; msg: string }
  | { code: 114; name: 'invalidInstruction'; msg: string }
  | { code: 115; name: 'arithmeticOverflow'; msg: string }
  | { code: 116; name: 'arithmeticUnderflow'; msg: string }
  | { code: 117; name: 'invalidAccountDiscriminator'; msg: string }
  | { code: 118; name: 'mintHasConfidentialTransfer'; msg: string }
  | { code: 119; name: 'mintHasNonTransferable'; msg: string }
  | { code: 120; name: 'mintHasPermanentDelegate'; msg: string }
  | { code: 121; name: 'mintHasTransferHook'; msg: string }
  | { code: 122; name: 'mintHasTransferFee'; msg: string }
  | { code: 123; name: 'mintHasMintCloseAuthority'; msg: string }
  | { code: 124; name: 'mintHasPausable'; msg: string }
  | { code: 125; name: 'mintMismatch'; msg: string }
  | { code: 126; name: 'invalidDelegatePda'; msg: string }
  | { code: 127; name: 'invalidHeaderData'; msg: string }
  | { code: 128; name: 'delegationExpired'; msg: string }
  | { code: 129; name: 'invalidAmount'; msg: string }
  | { code: 130; name: 'unauthorized'; msg: string }
  | { code: 131; name: 'accountNotWritable'; msg: string }
  | { code: 132; name: 'ataOwnerMismatch'; msg: string }
  | { code: 133; name: 'delegationVersionMismatch'; msg: string }
  | { code: 134; name: 'migrationRequired'; msg: string }
  | { code: 135; name: 'delegationAlreadyExists'; msg: string }
  | { code: 136; name: 'staleSubscriptionAuthority'; msg: string }
  | { code: 300; name: 'amountExceedsLimit'; msg: string }
  | { code: 301; name: 'fixedDelegationExpiryInPast'; msg: string }
  | { code: 302; name: 'fixedDelegationAmountZero'; msg: string }
  | { code: 400; name: 'amountExceedsPeriodLimit'; msg: string }
  | { code: 401; name: 'periodNotElapsed'; msg: string }
  | { code: 402; name: 'invalidPeriodLength'; msg: string }
  | { code: 403; name: 'invalidPayerData'; msg: string }
  | { code: 404; name: 'recurringDelegationStartTimeInPast'; msg: string }
  | { code: 405; name: 'recurringDelegationStartTimeGreaterThanExpiry'; msg: string }
  | { code: 406; name: 'recurringDelegationAmountZero'; msg: string }
  | { code: 407; name: 'delegationNotStarted'; msg: string }
  | { code: 500; name: 'planSunset'; msg: string }
  | { code: 501; name: 'planExpired'; msg: string }
  | { code: 502; name: 'invalidPlanPda'; msg: string }
  | { code: 503; name: 'invalidSubscriptionPda'; msg: string }
  | { code: 504; name: 'notPlanOwner'; msg: string }
  | { code: 505; name: 'subscriptionPlanMismatch'; msg: string }
  | { code: 506; name: 'unauthorizedDestination'; msg: string }
  | { code: 507; name: 'invalidNumDestinations'; msg: string }
  | { code: 508; name: 'subscriptionCancelled'; msg: string }
  | { code: 509; name: 'subscriptionAlreadyCancelled'; msg: string }
  | { code: 510; name: 'subscriptionNotCancelled'; msg: string }
  | { code: 511; name: 'invalidEndTs'; msg: string }
  | { code: 512; name: 'invalidPlanStatus'; msg: string }
  | { code: 513; name: 'planImmutableAfterSunset'; msg: string }
  | { code: 514; name: 'sunsetRequiresEndTs'; msg: string }
  | { code: 515; name: 'planNotExpired'; msg: string }
  | { code: 516; name: 'planClosed'; msg: string }
  | { code: 517; name: 'alreadySubscribed'; msg: string }
  | { code: 518; name: 'planAlreadyExists'; msg: string }
  | { code: 519; name: 'planTermsMismatch'; msg: string }
  | { code: 600; name: 'invalidEventAuthority'; msg: string }
  | { code: 601; name: 'invalidEventData'; msg: string }
  | { code: 602; name: 'invalidEventTag'; msg: string }
  | { code: 603; name: 'invalidEventDiscriminator'; msg: string };

const SUBSCRIPTIONS_PROGRAM_ERRORS: ErrorMetadata[] = [
  { code: 100, name: 'notSigner', msg: 'Account must be a signer' },
  { code: 101, name: 'invalidAddress', msg: 'Invalid account address' },
  { code: 102, name: 'invalidEscrowPda', msg: 'Invalid escrow PDA derivation' },
  { code: 103, name: 'invalidSubscriptionAuthorityPda', msg: 'Invalid subscription-authority PDA derivation' },
  { code: 104, name: 'notSystemProgram', msg: 'Expected system program' },
  { code: 105, name: 'invalidTokenProgram', msg: 'Token Program does not match other accounts' },
  { code: 106, name: 'invalidToken2022MintAccountData', msg: 'Invalid Token-2022 mint account data' },
  { code: 107, name: 'invalidToken2022TokenAccountData', msg: 'Invalid Token-2022 token account data' },
  { code: 108, name: 'invalidAssociatedTokenAccountDerivedAddress', msg: 'Invalid associated token account address' },
  { code: 109, name: 'invalidTokenSplMintAccountData', msg: 'Invalid SPL Token mint account data' },
  { code: 110, name: 'invalidTokenSplTokenAccountData', msg: 'Invalid SPL Token account data' },
  { code: 111, name: 'invalidAccountData', msg: 'Invalid account data' },
  { code: 112, name: 'invalidInstructionData', msg: 'Invalid instruction data' },
  { code: 113, name: 'notEnoughAccountKeys', msg: 'Not enough account keys provided' },
  { code: 114, name: 'invalidInstruction', msg: 'Invalid instruction' },
  { code: 115, name: 'arithmeticOverflow', msg: 'Arithmetic Overflow' },
  { code: 116, name: 'arithmeticUnderflow', msg: 'Arithmetic Underflow' },
  { code: 117, name: 'invalidAccountDiscriminator', msg: 'Invalid account discriminator' },
  { code: 118, name: 'mintHasConfidentialTransfer', msg: 'Mint has ConfidentialTransfer extension' },
  { code: 119, name: 'mintHasNonTransferable', msg: 'Mint has NonTransferable extension' },
  { code: 120, name: 'mintHasPermanentDelegate', msg: 'Mint has PermanentDelegate extension' },
  { code: 121, name: 'mintHasTransferHook', msg: 'Mint has TransferHook extension' },
  { code: 122, name: 'mintHasTransferFee', msg: 'Mint has TransferFee extension' },
  { code: 123, name: 'mintHasMintCloseAuthority', msg: 'Mint has MintCloseAuthority extension' },
  { code: 124, name: 'mintHasPausable', msg: 'Mint has Pausable extension' },
  { code: 125, name: 'mintMismatch', msg: 'Token mint mismatch' },
  { code: 126, name: 'invalidDelegatePda', msg: 'Invalid delegation PDA derivation' },
  { code: 127, name: 'invalidHeaderData', msg: 'Invalid header data' },
  { code: 128, name: 'delegationExpired', msg: 'Delegation has expired' },
  { code: 129, name: 'invalidAmount', msg: 'Invalid amount specified' },
  { code: 130, name: 'unauthorized', msg: 'Caller not authorized for this action' },
  { code: 131, name: 'accountNotWritable', msg: 'Account must be writable' },
  { code: 132, name: 'ataOwnerMismatch', msg: 'Token account owner does not match expected' },
  { code: 133, name: 'delegationVersionMismatch', msg: 'Delegation header version is not compatible' },
  { code: 134, name: 'migrationRequired', msg: 'Account requires explicit migration' },
  { code: 135, name: 'delegationAlreadyExists', msg: 'Delegation account already exists' },
  { code: 136, name: 'staleSubscriptionAuthority', msg: 'Delegation init_id does not match current SubscriptionAuthority' },
  { code: 300, name: 'amountExceedsLimit', msg: 'Transfer amount exceeds delegation limit' },
  { code: 301, name: 'fixedDelegationExpiryInPast', msg: 'Expiry time specified is less than current time' },
  { code: 302, name: 'fixedDelegationAmountZero', msg: 'zero amount specified' },
  { code: 400, name: 'amountExceedsPeriodLimit', msg: 'Transfer amount exceeds period limit' },
  { code: 401, name: 'periodNotElapsed', msg: 'Period has not elapsed yet' },
  { code: 402, name: 'invalidPeriodLength', msg: 'Invalid Period length' },
  { code: 403, name: 'invalidPayerData', msg: 'Payer provided does not match delegation' },
  { code: 404, name: 'recurringDelegationStartTimeInPast', msg: 'Past start time specified' },
  { code: 405, name: 'recurringDelegationStartTimeGreaterThanExpiry', msg: 'start time specified is greater than expiry' },
  { code: 406, name: 'recurringDelegationAmountZero', msg: 'zero amount specified' },
  { code: 407, name: 'delegationNotStarted', msg: 'Delegation period has not started yet' },
  { code: 500, name: 'planSunset', msg: 'Plan is in sunset status' },
  { code: 501, name: 'planExpired', msg: 'Plan has expired' },
  { code: 502, name: 'invalidPlanPda', msg: 'Invalid Plan PDA derivation' },
  { code: 503, name: 'invalidSubscriptionPda', msg: 'Invalid subscription PDA derivation' },
  { code: 504, name: 'notPlanOwner', msg: 'Caller is not the plan owner' },
  { code: 505, name: 'subscriptionPlanMismatch', msg: 'Subscription does not belong to this plan' },
  { code: 506, name: 'unauthorizedDestination', msg: 'Destination not in plan whitelist' },
  { code: 507, name: 'invalidNumDestinations', msg: 'No valid destinations provided' },
  { code: 508, name: 'subscriptionCancelled', msg: 'Subscription cancelled and past valid period' },
  { code: 509, name: 'subscriptionAlreadyCancelled', msg: 'Subscription already cancelled' },
  { code: 510, name: 'subscriptionNotCancelled', msg: 'Subscription is not cancelled' },
  { code: 511, name: 'invalidEndTs', msg: 'End timestamp must be zero or in the future' },
  { code: 512, name: 'invalidPlanStatus', msg: 'Invalid plan status value' },
  { code: 513, name: 'planImmutableAfterSunset', msg: 'Plan cannot be updated after sunset' },
  { code: 514, name: 'sunsetRequiresEndTs', msg: 'Sunset requires a non-zero end timestamp' },
  { code: 515, name: 'planNotExpired', msg: 'Plan must be expired to delete' },
  { code: 516, name: 'planClosed', msg: 'Plan account has been closed' },
  { code: 517, name: 'alreadySubscribed', msg: 'Already subscribed to this plan' },
  { code: 518, name: 'planAlreadyExists', msg: 'Plan account already exists' },
  { code: 519, name: 'planTermsMismatch', msg: 'Subscription plan terms do not match the current plan' },
  { code: 600, name: 'invalidEventAuthority', msg: 'Invalid event authority PDA' },
  { code: 601, name: 'invalidEventData', msg: 'Invalid event data' },
  { code: 602, name: 'invalidEventTag', msg: 'Invalid event tag prefix' },
  { code: 603, name: 'invalidEventDiscriminator', msg: 'Unknown event discriminator' },
];

export interface CreateFixedDelegationData {
  nonce: bigint;
  amount: bigint;
  expiryTs: bigint;
  expectedSubscriptionAuthorityInitId: bigint;
}

export interface CreateRecurringDelegationData {
  nonce: bigint;
  amountPerPeriod: bigint;
  periodLengthS: bigint;
  startTs: bigint;
  expiryTs: bigint;
  expectedSubscriptionAuthorityInitId: bigint;
}

export interface TransferData {
  amount: bigint;
  delegator: string;
  mint: string;
}

export interface PlanTermsInput {
  amount: bigint;
  periodHours: bigint;
  createdAt: bigint;
}

export interface PlanDataInput {
  planId: bigint;
  mint: string;
  terms: PlanTermsInput;
  endTs: bigint;
  destinations: string[];
  pullers: string[];
  metadataUri: number[];
}

export interface UpdatePlanData {
  status: number;
  endTs: bigint;
  pullers: string[];
  metadataUri: number[];
  expectedCreatedAt: bigint;
  expectedEndTs: bigint;
  expectedPullers: string[];
  expectedMetadataUri: number[];
}

export interface SubscribeData {
  planId: bigint;
  planBump: number;
  expectedMint: string;
  expectedAmount: bigint;
  expectedPeriodHours: bigint;
  expectedCreatedAt: bigint;
  expectedSubscriptionAuthorityInitId: bigint;
}

export interface ResumeData {
  expectedExpiresAtTs: bigint;
}

export interface CancelSubscriptionNowData {
  expectedCurrentPeriodStartTs: bigint;
}

export interface InitSubscriptionAuthorityParams {
  owner: string;
  subscriptionAuthority?: string;
  tokenMint: string;
  userAta: string;
  tokenProgram: string;
}

export type InitSubscriptionAuthorityError = SubscriptionsProgramError;

export const initSubscriptionAuthorityInstruction = createInstructionHandler<InitSubscriptionAuthorityParams, InitSubscriptionAuthorityError, 'owner' | 'subscriptionAuthority' | 'tokenMint' | 'userAta' | 'tokenProgram'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [0],
  args: [],
  accounts: [
    { name: 'owner', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'subscriptionAuthority', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'SubscriptionAuthority' }, { type: 'accountRef', accountName: 'owner' }, { type: 'accountRef', accountName: 'tokenMint' }] } },
    { name: 'tokenMint', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'userAta', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
    { name: 'tokenProgram', isSigner: false, isWritable: false, category: 'userProvided' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface CreateFixedDelegationParams {
  fixedDelegation: CreateFixedDelegationData;
  delegator: string;
  subscriptionAuthority?: string;
  delegationAccount: string;
  delegatee: string;
  resolve?: {
    tokenMint?: string;
    user?: string;
  };
}

export type CreateFixedDelegationError = SubscriptionsProgramError;

export const createFixedDelegationInstruction = createInstructionHandler<CreateFixedDelegationParams, CreateFixedDelegationError, 'delegator' | 'subscriptionAuthority' | 'delegationAccount' | 'delegatee'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [1],
  args: [
    { name: 'fixedDelegation', type: { struct: [{ name: 'nonce', type: 'u64' }, { name: 'amount', type: 'u64' }, { name: 'expiryTs', type: 'i64' }, { name: 'expectedSubscriptionAuthorityInitId', type: 'i64' }] } },
  ],
  accounts: [
    { name: 'delegator', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'subscriptionAuthority', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'SubscriptionAuthority' }, { type: 'argRef', argName: 'user', argType: 'pubkey' }, { type: 'argRef', argName: 'tokenMint', argType: 'pubkey' }] } },
    { name: 'delegationAccount', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'delegatee', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface CreateRecurringDelegationParams {
  recurringDelegation: CreateRecurringDelegationData;
  delegator: string;
  subscriptionAuthority?: string;
  delegationAccount: string;
  delegatee: string;
  resolve?: {
    tokenMint?: string;
    user?: string;
  };
}

export type CreateRecurringDelegationError = SubscriptionsProgramError;

export const createRecurringDelegationInstruction = createInstructionHandler<CreateRecurringDelegationParams, CreateRecurringDelegationError, 'delegator' | 'subscriptionAuthority' | 'delegationAccount' | 'delegatee'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [2],
  args: [
    { name: 'recurringDelegation', type: { struct: [{ name: 'nonce', type: 'u64' }, { name: 'amountPerPeriod', type: 'u64' }, { name: 'periodLengthS', type: 'u64' }, { name: 'startTs', type: 'i64' }, { name: 'expiryTs', type: 'i64' }, { name: 'expectedSubscriptionAuthorityInitId', type: 'i64' }] } },
  ],
  accounts: [
    { name: 'delegator', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'subscriptionAuthority', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'SubscriptionAuthority' }, { type: 'argRef', argName: 'user', argType: 'pubkey' }, { type: 'argRef', argName: 'tokenMint', argType: 'pubkey' }] } },
    { name: 'delegationAccount', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'delegatee', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface RevokeDelegationParams {
  authority: string;
  delegationAccount: string;
}

export type RevokeDelegationError = SubscriptionsProgramError;

export const revokeDelegationInstruction = createInstructionHandler<RevokeDelegationParams, RevokeDelegationError, 'authority' | 'delegationAccount'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [3],
  args: [],
  accounts: [
    { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'delegationAccount', isSigner: false, isWritable: true, category: 'userProvided' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface TransferFixedParams {
  transferData: TransferData;
  delegationPda: string;
  subscriptionAuthority?: string;
  delegatorAta: string;
  receiverAta: string;
  tokenMint: string;
  tokenProgram: string;
  delegatee: string;
  resolve?: {
    tokenMint?: string;
    user?: string;
  };
}

export type TransferFixedError = SubscriptionsProgramError;

export const transferFixedInstruction = createInstructionHandler<TransferFixedParams, TransferFixedError, 'delegationPda' | 'subscriptionAuthority' | 'delegatorAta' | 'receiverAta' | 'tokenMint' | 'tokenProgram' | 'delegatee'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [4],
  args: [
    { name: 'transferData', type: { struct: [{ name: 'amount', type: 'u64' }, { name: 'delegator', type: 'pubkey' }, { name: 'mint', type: 'pubkey' }] } },
  ],
  accounts: [
    { name: 'delegationPda', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'subscriptionAuthority', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'SubscriptionAuthority' }, { type: 'argRef', argName: 'user', argType: 'pubkey' }, { type: 'argRef', argName: 'tokenMint', argType: 'pubkey' }] } },
    { name: 'delegatorAta', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'receiverAta', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'tokenMint', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'tokenProgram', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'delegatee', isSigner: true, isWritable: false, category: 'signer', signerKind: 'provided' },
    { name: 'eventAuthority', isSigner: false, isWritable: false, category: 'known', knownAddress: '3Hnj4BYoDgtpBuqXfiy7Y8cNa3jXaNd4oqgSXBzkMcH7' },
    { name: 'selfProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface TransferRecurringParams {
  transferData: TransferData;
  delegationPda: string;
  subscriptionAuthority?: string;
  delegatorAta: string;
  receiverAta: string;
  tokenMint: string;
  tokenProgram: string;
  delegatee: string;
  resolve?: {
    tokenMint?: string;
    user?: string;
  };
}

export type TransferRecurringError = SubscriptionsProgramError;

export const transferRecurringInstruction = createInstructionHandler<TransferRecurringParams, TransferRecurringError, 'delegationPda' | 'subscriptionAuthority' | 'delegatorAta' | 'receiverAta' | 'tokenMint' | 'tokenProgram' | 'delegatee'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [5],
  args: [
    { name: 'transferData', type: { struct: [{ name: 'amount', type: 'u64' }, { name: 'delegator', type: 'pubkey' }, { name: 'mint', type: 'pubkey' }] } },
  ],
  accounts: [
    { name: 'delegationPda', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'subscriptionAuthority', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'SubscriptionAuthority' }, { type: 'argRef', argName: 'user', argType: 'pubkey' }, { type: 'argRef', argName: 'tokenMint', argType: 'pubkey' }] } },
    { name: 'delegatorAta', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'receiverAta', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'tokenMint', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'tokenProgram', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'delegatee', isSigner: true, isWritable: false, category: 'signer', signerKind: 'provided' },
    { name: 'eventAuthority', isSigner: false, isWritable: false, category: 'known', knownAddress: '3Hnj4BYoDgtpBuqXfiy7Y8cNa3jXaNd4oqgSXBzkMcH7' },
    { name: 'selfProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface CloseSubscriptionAuthorityParams {
  user: string;
  subscriptionAuthority?: string;
  resolve?: {
    tokenMint?: string;
    user?: string;
  };
}

export type CloseSubscriptionAuthorityError = SubscriptionsProgramError;

export const closeSubscriptionAuthorityInstruction = createInstructionHandler<CloseSubscriptionAuthorityParams, CloseSubscriptionAuthorityError, 'user' | 'subscriptionAuthority'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [6],
  args: [],
  accounts: [
    { name: 'user', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'subscriptionAuthority', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'SubscriptionAuthority' }, { type: 'argRef', argName: 'user', argType: 'pubkey' }, { type: 'argRef', argName: 'tokenMint', argType: 'pubkey' }] } },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface CreatePlanParams {
  planData: PlanDataInput;
  merchant: string;
  planPda: string;
  tokenMint: string;
}

export type CreatePlanError = SubscriptionsProgramError;

export const createPlanInstruction = createInstructionHandler<CreatePlanParams, CreatePlanError, 'merchant' | 'planPda' | 'tokenMint'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [7],
  args: [
    { name: 'planData', type: { struct: [{ name: 'planId', type: 'u64' }, { name: 'mint', type: 'pubkey' }, { name: 'terms', type: { struct: [{ name: 'amount', type: 'u64' }, { name: 'periodHours', type: 'u64' }, { name: 'createdAt', type: 'i64' }] } }, { name: 'endTs', type: 'i64' }, { name: 'destinations', type: { array: ['pubkey', 4] } }, { name: 'pullers', type: { array: ['pubkey', 4] } }, { name: 'metadataUri', type: { array: ['u8', 128] } }] } },
  ],
  accounts: [
    { name: 'merchant', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'planPda', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'tokenMint', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
    { name: 'tokenProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface UpdatePlanParams {
  updatePlanData: UpdatePlanData;
  owner: string;
  planPda: string;
  eventAuthority?: string;
}

export type UpdatePlanError = SubscriptionsProgramError;

export const updatePlanInstruction = createInstructionHandler<UpdatePlanParams, UpdatePlanError, 'owner' | 'planPda' | 'eventAuthority'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [8],
  args: [
    { name: 'updatePlanData', type: { struct: [{ name: 'status', type: 'u8' }, { name: 'endTs', type: 'i64' }, { name: 'pullers', type: { array: ['pubkey', 4] } }, { name: 'metadataUri', type: { array: ['u8', 128] } }, { name: 'expectedCreatedAt', type: 'i64' }, { name: 'expectedEndTs', type: 'i64' }, { name: 'expectedPullers', type: { array: ['pubkey', 4] } }, { name: 'expectedMetadataUri', type: { array: ['u8', 128] } }] } },
  ],
  accounts: [
    { name: 'owner', isSigner: true, isWritable: false, category: 'signer', signerKind: 'provided' },
    { name: 'planPda', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'eventAuthority', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'event_authority' }] } },
    { name: 'selfProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface DeletePlanParams {
  owner: string;
  planPda: string;
}

export type DeletePlanError = SubscriptionsProgramError;

export const deletePlanInstruction = createInstructionHandler<DeletePlanParams, DeletePlanError, 'owner' | 'planPda'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [9],
  args: [],
  accounts: [
    { name: 'owner', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'planPda', isSigner: false, isWritable: true, category: 'userProvided' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface TransferSubscriptionParams {
  transferData: TransferData;
  subscriptionPda: string;
  planPda: string;
  subscriptionAuthority?: string;
  delegatorAta: string;
  receiverAta: string;
  caller: string;
  tokenMint: string;
  tokenProgram: string;
  resolve?: {
    tokenMint?: string;
    user?: string;
  };
}

export type TransferSubscriptionError = SubscriptionsProgramError;

export const transferSubscriptionInstruction = createInstructionHandler<TransferSubscriptionParams, TransferSubscriptionError, 'subscriptionPda' | 'planPda' | 'subscriptionAuthority' | 'delegatorAta' | 'receiverAta' | 'caller' | 'tokenMint' | 'tokenProgram'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [10],
  args: [
    { name: 'transferData', type: { struct: [{ name: 'amount', type: 'u64' }, { name: 'delegator', type: 'pubkey' }, { name: 'mint', type: 'pubkey' }] } },
  ],
  accounts: [
    { name: 'subscriptionPda', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'planPda', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'subscriptionAuthority', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'SubscriptionAuthority' }, { type: 'argRef', argName: 'user', argType: 'pubkey' }, { type: 'argRef', argName: 'tokenMint', argType: 'pubkey' }] } },
    { name: 'delegatorAta', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'receiverAta', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'caller', isSigner: true, isWritable: false, category: 'signer', signerKind: 'provided' },
    { name: 'tokenMint', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'tokenProgram', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'eventAuthority', isSigner: false, isWritable: false, category: 'known', knownAddress: '3Hnj4BYoDgtpBuqXfiy7Y8cNa3jXaNd4oqgSXBzkMcH7' },
    { name: 'selfProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface SubscribeParams {
  subscribeData: SubscribeData;
  subscriber: string;
  merchant: string;
  planPda: string;
  subscriptionPda?: string;
  subscriptionAuthorityPda: string;
}

export type SubscribeError = SubscriptionsProgramError;

export const subscribeInstruction = createInstructionHandler<SubscribeParams, SubscribeError, 'subscriber' | 'merchant' | 'planPda' | 'subscriptionPda' | 'subscriptionAuthorityPda'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [11],
  args: [
    { name: 'subscribeData', type: { struct: [{ name: 'planId', type: 'u64' }, { name: 'planBump', type: 'u8' }, { name: 'expectedMint', type: 'pubkey' }, { name: 'expectedAmount', type: 'u64' }, { name: 'expectedPeriodHours', type: 'u64' }, { name: 'expectedCreatedAt', type: 'i64' }, { name: 'expectedSubscriptionAuthorityInitId', type: 'i64' }] } },
  ],
  accounts: [
    { name: 'subscriber', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'merchant', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'planPda', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'subscriptionPda', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'subscription' }, { type: 'accountRef', accountName: 'planPda' }, { type: 'accountRef', accountName: 'subscriber' }] } },
    { name: 'subscriptionAuthorityPda', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
    { name: 'eventAuthority', isSigner: false, isWritable: false, category: 'known', knownAddress: '3Hnj4BYoDgtpBuqXfiy7Y8cNa3jXaNd4oqgSXBzkMcH7' },
    { name: 'selfProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface CancelSubscriptionParams {
  subscriber: string;
  planPda: string;
  subscriptionPda?: string;
}

export type CancelSubscriptionError = SubscriptionsProgramError;

export const cancelSubscriptionInstruction = createInstructionHandler<CancelSubscriptionParams, CancelSubscriptionError, 'subscriber' | 'planPda' | 'subscriptionPda'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [12],
  args: [],
  accounts: [
    { name: 'subscriber', isSigner: true, isWritable: false, category: 'signer', signerKind: 'provided' },
    { name: 'planPda', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'subscriptionPda', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'subscription' }, { type: 'accountRef', accountName: 'planPda' }, { type: 'accountRef', accountName: 'subscriber' }] } },
    { name: 'eventAuthority', isSigner: false, isWritable: false, category: 'known', knownAddress: '3Hnj4BYoDgtpBuqXfiy7Y8cNa3jXaNd4oqgSXBzkMcH7' },
    { name: 'selfProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface ResumeSubscriptionParams {
  resumeData: ResumeData;
  subscriber: string;
  planPda: string;
  subscriptionPda?: string;
  subscriptionAuthority?: string;
  eventAuthority?: string;
  resolve?: {
    tokenMint?: string;
    user?: string;
  };
}

export type ResumeSubscriptionError = SubscriptionsProgramError;

export const resumeSubscriptionInstruction = createInstructionHandler<ResumeSubscriptionParams, ResumeSubscriptionError, 'subscriber' | 'planPda' | 'subscriptionPda' | 'subscriptionAuthority' | 'eventAuthority'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [13],
  args: [
    { name: 'resumeData', type: { struct: [{ name: 'expectedExpiresAtTs', type: 'i64' }] } },
  ],
  accounts: [
    { name: 'subscriber', isSigner: true, isWritable: false, category: 'signer', signerKind: 'provided' },
    { name: 'planPda', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'subscriptionPda', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'subscription' }, { type: 'accountRef', accountName: 'planPda' }, { type: 'accountRef', accountName: 'subscriber' }] } },
    { name: 'subscriptionAuthority', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'SubscriptionAuthority' }, { type: 'argRef', argName: 'user', argType: 'pubkey' }, { type: 'argRef', argName: 'tokenMint', argType: 'pubkey' }] } },
    { name: 'eventAuthority', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'event_authority' }] } },
    { name: 'selfProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface RevokeSubscriptionAuthorityParams {
  user: string;
  userAta: string;
  tokenMint: string;
  tokenProgram: string;
  subscriptionAuthority?: string;
}

export type RevokeSubscriptionAuthorityError = SubscriptionsProgramError;

export const revokeSubscriptionAuthorityInstruction = createInstructionHandler<RevokeSubscriptionAuthorityParams, RevokeSubscriptionAuthorityError, 'user' | 'userAta' | 'tokenMint' | 'tokenProgram' | 'subscriptionAuthority'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [14],
  args: [],
  accounts: [
    { name: 'user', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'userAta', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'tokenMint', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'tokenProgram', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'subscriptionAuthority', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'SubscriptionAuthority' }, { type: 'accountRef', accountName: 'user' }, { type: 'accountRef', accountName: 'tokenMint' }] } },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface RevokeAbandonedDelegationParams {
  payer: string;
  delegationAccount: string;
  subscriptionAuthority?: string;
  resolve?: {
    tokenMint?: string;
    user?: string;
  };
}

export type RevokeAbandonedDelegationError = SubscriptionsProgramError;

export const revokeAbandonedDelegationInstruction = createInstructionHandler<RevokeAbandonedDelegationParams, RevokeAbandonedDelegationError, 'payer' | 'delegationAccount' | 'subscriptionAuthority'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [15],
  args: [],
  accounts: [
    { name: 'payer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'delegationAccount', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'subscriptionAuthority', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'SubscriptionAuthority' }, { type: 'argRef', argName: 'user', argType: 'pubkey' }, { type: 'argRef', argName: 'tokenMint', argType: 'pubkey' }] } },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface RevokeAbandonedSubscriptionParams {
  payer: string;
  subscriptionAccount: string;
  subscriptionAuthority?: string;
  planPda: string;
  resolve?: {
    tokenMint?: string;
    user?: string;
  };
}

export type RevokeAbandonedSubscriptionError = SubscriptionsProgramError;

export const revokeAbandonedSubscriptionInstruction = createInstructionHandler<RevokeAbandonedSubscriptionParams, RevokeAbandonedSubscriptionError, 'payer' | 'subscriptionAccount' | 'subscriptionAuthority' | 'planPda'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [16],
  args: [],
  accounts: [
    { name: 'payer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'subscriptionAccount', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'subscriptionAuthority', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'SubscriptionAuthority' }, { type: 'argRef', argName: 'user', argType: 'pubkey' }, { type: 'argRef', argName: 'tokenMint', argType: 'pubkey' }] } },
    { name: 'planPda', isSigner: false, isWritable: false, category: 'userProvided' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

export interface CancelSubscriptionNowParams {
  cancelSubscriptionNowData: CancelSubscriptionNowData;
  subscriber: string;
  merchant: string;
  planPda: string;
  subscriptionPda?: string;
  eventAuthority?: string;
}

export type CancelSubscriptionNowError = SubscriptionsProgramError;

export const cancelSubscriptionNowInstruction = createInstructionHandler<CancelSubscriptionNowParams, CancelSubscriptionNowError, 'subscriber' | 'merchant' | 'planPda' | 'subscriptionPda' | 'eventAuthority'>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [17],
  args: [
    { name: 'cancelSubscriptionNowData', type: { struct: [{ name: 'expectedCurrentPeriodStartTs', type: 'i64' }] } },
  ],
  accounts: [
    { name: 'subscriber', isSigner: true, isWritable: false, category: 'signer', signerKind: 'provided' },
    { name: 'merchant', isSigner: true, isWritable: false, category: 'signer', signerKind: 'provided' },
    { name: 'planPda', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'subscriptionPda', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'subscription' }, { type: 'accountRef', accountName: 'planPda' }, { type: 'accountRef', accountName: 'subscriber' }] } },
    { name: 'eventAuthority', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'event_authority' }] } },
    { name: 'selfProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44' },
  ],
  errors: SUBSCRIPTIONS_PROGRAM_ERRORS,
});

// ============================================================================
// Program Definitions
// ============================================================================

/** Standalone program SDK for 'subscriptions' */
export const SUBSCRIPTIONS = {
  name: 'subscriptions',
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:399cc366ded28f42d7c85e6ea9a11bd16bd200e382ffd662f417700be5d6d32d',
  programSpecHash: 'arete:h1:program-spec:sha256:a10a30f17be97429474e990fdc24b87edf51e4c5fe0021b0651d434d11f3e698',
  idlContentHash: 'arete:h1:idl-content:sha256:6b1a0054e474098d8e646ca0238f197ea5d31fa93ca79c4dd466c6edf4fef554',
  normalizedIdlHash: 'arete:h1:idl-normalized:sha256:c12d4e2b8c21cd79a97e3e9dd698cc60dd1a511c1ef4e31dbc502a8fd89485c4',
  gateway: {"chain":{"auth":{"acceptedKeyClasses":["anonymous","publishable","secret"],"audience":"arete:solana-gateway","jwksUrl":"https://api.arete.run/ws/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["read"],"sessionEndpoint":"https://api.arete.run/ws/sessions","targetId":"sgb_2f6497e233adc32154d1aded91486dfb","targetKind":"solana-gateway-binding","tokenTransport":"bearer","transactionEntitlementRequired":false},"authPolicy":"signed_session","cluster":"mainnet-beta","endpoint":"https://solana.arete.run","region":"us-west-1","solanaGatewayBindingId":"sgb_2f6497e233adc32154d1aded91486dfb"},"transactions":{"auth":{"acceptedKeyClasses":["publishable","secret"],"audience":"arete:solana-gateway","jwksUrl":"https://api.arete.run/ws/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["transaction:inspect","transaction:send"],"sessionEndpoint":"https://api.arete.run/ws/sessions","targetId":"sgb_2f6497e233adc32154d1aded91486dfb","targetKind":"solana-gateway-binding","tokenTransport":"bearer","transactionEntitlementRequired":true},"authPolicy":"signed_session","cluster":"mainnet-beta","endpoint":"https://solana.arete.run","region":"us-west-1","solanaGatewayBindingId":"sgb_2f6497e233adc32154d1aded91486dfb"}},
  pdas: {
    eventAuthority: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('event_authority')),
    fixedDelegation: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('delegation'), arg('subscriptionAuthority', 'publicKey'), arg('delegator', 'publicKey'), arg('delegatee', 'publicKey'), arg('nonce', 'u64')),
    plan: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('plan'), arg('owner', 'publicKey'), arg('planId', 'u64')),
    recurringDelegation: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('delegation'), arg('subscriptionAuthority', 'publicKey'), arg('delegator', 'publicKey'), arg('delegatee', 'publicKey'), arg('nonce', 'u64')),
    subscriptionAuthority: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('SubscriptionAuthority'), arg('user', 'publicKey'), arg('tokenMint', 'publicKey')),
    subscriptionDelegation: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('subscription'), arg('planPda', 'publicKey'), arg('subscriber', 'publicKey')),
  },
  addresses: {
    eventAuthority: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('event_authority')),
    fixedDelegation: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('delegation'), arg('subscriptionAuthority', 'publicKey'), arg('delegator', 'publicKey'), arg('delegatee', 'publicKey'), arg('nonce', 'u64')),
    plan: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('plan'), arg('owner', 'publicKey'), arg('planId', 'u64')),
    recurringDelegation: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('delegation'), arg('subscriptionAuthority', 'publicKey'), arg('delegator', 'publicKey'), arg('delegatee', 'publicKey'), arg('nonce', 'u64')),
    subscriptionAuthority: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('SubscriptionAuthority'), arg('user', 'publicKey'), arg('tokenMint', 'publicKey')),
    subscriptionDelegation: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('subscription'), arg('planPda', 'publicKey'), arg('subscriber', 'publicKey')),
  },
  accounts: {
    fixedDelegation: programAccountRead<FixedDelegation>({ account: 'fixedDelegation', schema: FixedDelegationSchema }),
    plan: programAccountRead<Plan>({ account: 'plan', schema: PlanSchema }),
    recurringDelegation: programAccountRead<RecurringDelegation>({ account: 'recurringDelegation', schema: RecurringDelegationSchema }),
    subscriptionAuthority: programAccountRead<SubscriptionAuthority>({ account: 'subscriptionAuthority', schema: SubscriptionAuthoritySchema }),
    subscriptionDelegation: programAccountRead<SubscriptionDelegation>({ account: 'subscriptionDelegation', schema: SubscriptionDelegationSchema }),
  },
  rawInstructions: {
    initSubscriptionAuthority: initSubscriptionAuthorityInstruction,
    createFixedDelegation: createFixedDelegationInstruction,
    createRecurringDelegation: createRecurringDelegationInstruction,
    revokeDelegation: revokeDelegationInstruction,
    transferFixed: transferFixedInstruction,
    transferRecurring: transferRecurringInstruction,
    closeSubscriptionAuthority: closeSubscriptionAuthorityInstruction,
    createPlan: createPlanInstruction,
    updatePlan: updatePlanInstruction,
    deletePlan: deletePlanInstruction,
    transferSubscription: transferSubscriptionInstruction,
    subscribe: subscribeInstruction,
    cancelSubscription: cancelSubscriptionInstruction,
    resumeSubscription: resumeSubscriptionInstruction,
    revokeSubscriptionAuthority: revokeSubscriptionAuthorityInstruction,
    revokeAbandonedDelegation: revokeAbandonedDelegationInstruction,
    revokeAbandonedSubscription: revokeAbandonedSubscriptionInstruction,
    cancelSubscriptionNow: cancelSubscriptionNowInstruction,
  },
  [PROGRAM_OPERATION_EXTENSIONS]: {
    createOperations() {
      return {
        instructions: {
        initSubscriptionAuthority: instructionOperation(async (params: InitSubscriptionAuthorityParams) => {
          const instruction = buildInstruction(initSubscriptionAuthorityInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'initSubscriptionAuthority',
            instruction,
            artifacts: { instruction },
            errors: initSubscriptionAuthorityInstruction.errors,
          });
        }),
        createFixedDelegation: instructionOperation(async (params: CreateFixedDelegationParams) => {
          const instruction = buildInstruction(createFixedDelegationInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'createFixedDelegation',
            instruction,
            artifacts: { instruction },
            errors: createFixedDelegationInstruction.errors,
          });
        }),
        createRecurringDelegation: instructionOperation(async (params: CreateRecurringDelegationParams) => {
          const instruction = buildInstruction(createRecurringDelegationInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'createRecurringDelegation',
            instruction,
            artifacts: { instruction },
            errors: createRecurringDelegationInstruction.errors,
          });
        }),
        revokeDelegation: instructionOperation(async (params: RevokeDelegationParams) => {
          const instruction = buildInstruction(revokeDelegationInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'revokeDelegation',
            instruction,
            artifacts: { instruction },
            errors: revokeDelegationInstruction.errors,
          });
        }),
        transferFixed: instructionOperation(async (params: TransferFixedParams) => {
          const instruction = buildInstruction(transferFixedInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'transferFixed',
            instruction,
            artifacts: { instruction },
            errors: transferFixedInstruction.errors,
          });
        }),
        transferRecurring: instructionOperation(async (params: TransferRecurringParams) => {
          const instruction = buildInstruction(transferRecurringInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'transferRecurring',
            instruction,
            artifacts: { instruction },
            errors: transferRecurringInstruction.errors,
          });
        }),
        closeSubscriptionAuthority: instructionOperation(async (params: CloseSubscriptionAuthorityParams) => {
          const instruction = buildInstruction(closeSubscriptionAuthorityInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'closeSubscriptionAuthority',
            instruction,
            artifacts: { instruction },
            errors: closeSubscriptionAuthorityInstruction.errors,
          });
        }),
        createPlan: instructionOperation(async (params: CreatePlanParams) => {
          const instruction = buildInstruction(createPlanInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'createPlan',
            instruction,
            artifacts: { instruction },
            errors: createPlanInstruction.errors,
          });
        }),
        updatePlan: instructionOperation(async (params: UpdatePlanParams) => {
          const instruction = buildInstruction(updatePlanInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'updatePlan',
            instruction,
            artifacts: { instruction },
            errors: updatePlanInstruction.errors,
          });
        }),
        deletePlan: instructionOperation(async (params: DeletePlanParams) => {
          const instruction = buildInstruction(deletePlanInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'deletePlan',
            instruction,
            artifacts: { instruction },
            errors: deletePlanInstruction.errors,
          });
        }),
        transferSubscription: instructionOperation(async (params: TransferSubscriptionParams) => {
          const instruction = buildInstruction(transferSubscriptionInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'transferSubscription',
            instruction,
            artifacts: { instruction },
            errors: transferSubscriptionInstruction.errors,
          });
        }),
        subscribe: instructionOperation(async (params: SubscribeParams) => {
          const instruction = buildInstruction(subscribeInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'subscribe',
            instruction,
            artifacts: { instruction },
            errors: subscribeInstruction.errors,
          });
        }),
        cancelSubscription: instructionOperation(async (params: CancelSubscriptionParams) => {
          const instruction = buildInstruction(cancelSubscriptionInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'cancelSubscription',
            instruction,
            artifacts: { instruction },
            errors: cancelSubscriptionInstruction.errors,
          });
        }),
        resumeSubscription: instructionOperation(async (params: ResumeSubscriptionParams) => {
          const instruction = buildInstruction(resumeSubscriptionInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'resumeSubscription',
            instruction,
            artifacts: { instruction },
            errors: resumeSubscriptionInstruction.errors,
          });
        }),
        revokeSubscriptionAuthority: instructionOperation(async (params: RevokeSubscriptionAuthorityParams) => {
          const instruction = buildInstruction(revokeSubscriptionAuthorityInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'revokeSubscriptionAuthority',
            instruction,
            artifacts: { instruction },
            errors: revokeSubscriptionAuthorityInstruction.errors,
          });
        }),
        revokeAbandonedDelegation: instructionOperation(async (params: RevokeAbandonedDelegationParams) => {
          const instruction = buildInstruction(revokeAbandonedDelegationInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'revokeAbandonedDelegation',
            instruction,
            artifacts: { instruction },
            errors: revokeAbandonedDelegationInstruction.errors,
          });
        }),
        revokeAbandonedSubscription: instructionOperation(async (params: RevokeAbandonedSubscriptionParams) => {
          const instruction = buildInstruction(revokeAbandonedSubscriptionInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'revokeAbandonedSubscription',
            instruction,
            artifacts: { instruction },
            errors: revokeAbandonedSubscriptionInstruction.errors,
          });
        }),
        cancelSubscriptionNow: instructionOperation(async (params: CancelSubscriptionNowParams) => {
          const instruction = buildInstruction(cancelSubscriptionNowInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'cancelSubscriptionNow',
            instruction,
            artifacts: { instruction },
            errors: cancelSubscriptionNowInstruction.errors,
          });
        }),
        },
      };
    },
  },
} as const;

/** Instruction shapes for 'subscriptions', by IDL instruction name: argument and account names, and which accounts may be left out */
export const SUBSCRIPTIONS_INSTRUCTION_SHAPES = {
  initSubscriptionAuthority: {
    args: [],
    accounts: {
      owner: { optional: false, derive: null },
      subscriptionAuthority: { optional: false, derive: ['owner', 'tokenMint'] },
      tokenMint: { optional: false, derive: null },
      userAta: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
      tokenProgram: { optional: false, derive: null },
    },
  },
  createFixedDelegation: {
    args: ['fixedDelegation'],
    accounts: {
      delegator: { optional: false, derive: null },
      subscriptionAuthority: { optional: false, derive: null },
      delegationAccount: { optional: false, derive: null },
      delegatee: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
    },
  },
  createRecurringDelegation: {
    args: ['recurringDelegation'],
    accounts: {
      delegator: { optional: false, derive: null },
      subscriptionAuthority: { optional: false, derive: null },
      delegationAccount: { optional: false, derive: null },
      delegatee: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
    },
  },
  revokeDelegation: {
    args: [],
    accounts: {
      authority: { optional: false, derive: null },
      delegationAccount: { optional: false, derive: null },
    },
  },
  transferFixed: {
    args: ['transferData'],
    accounts: {
      delegationPda: { optional: false, derive: null },
      subscriptionAuthority: { optional: false, derive: null },
      delegatorAta: { optional: false, derive: null },
      receiverAta: { optional: false, derive: null },
      tokenMint: { optional: false, derive: null },
      tokenProgram: { optional: false, derive: null },
      delegatee: { optional: false, derive: null },
      eventAuthority: { optional: false, derive: [] },
      selfProgram: { optional: false, derive: [] },
    },
  },
  transferRecurring: {
    args: ['transferData'],
    accounts: {
      delegationPda: { optional: false, derive: null },
      subscriptionAuthority: { optional: false, derive: null },
      delegatorAta: { optional: false, derive: null },
      receiverAta: { optional: false, derive: null },
      tokenMint: { optional: false, derive: null },
      tokenProgram: { optional: false, derive: null },
      delegatee: { optional: false, derive: null },
      eventAuthority: { optional: false, derive: [] },
      selfProgram: { optional: false, derive: [] },
    },
  },
  closeSubscriptionAuthority: {
    args: [],
    accounts: {
      user: { optional: false, derive: null },
      subscriptionAuthority: { optional: false, derive: null },
    },
  },
  createPlan: {
    args: ['planData'],
    accounts: {
      merchant: { optional: false, derive: null },
      planPda: { optional: false, derive: null },
      tokenMint: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
      tokenProgram: { optional: false, derive: [] },
    },
  },
  updatePlan: {
    args: ['updatePlanData'],
    accounts: {
      owner: { optional: false, derive: null },
      planPda: { optional: false, derive: null },
      eventAuthority: { optional: false, derive: [] },
      selfProgram: { optional: false, derive: [] },
    },
  },
  deletePlan: {
    args: [],
    accounts: {
      owner: { optional: false, derive: null },
      planPda: { optional: false, derive: null },
    },
  },
  transferSubscription: {
    args: ['transferData'],
    accounts: {
      subscriptionPda: { optional: false, derive: null },
      planPda: { optional: false, derive: null },
      subscriptionAuthority: { optional: false, derive: null },
      delegatorAta: { optional: false, derive: null },
      receiverAta: { optional: false, derive: null },
      caller: { optional: false, derive: null },
      tokenMint: { optional: false, derive: null },
      tokenProgram: { optional: false, derive: null },
      eventAuthority: { optional: false, derive: [] },
      selfProgram: { optional: false, derive: [] },
    },
  },
  subscribe: {
    args: ['subscribeData'],
    accounts: {
      subscriber: { optional: false, derive: null },
      merchant: { optional: false, derive: null },
      planPda: { optional: false, derive: null },
      subscriptionPda: { optional: false, derive: ['planPda', 'subscriber'] },
      subscriptionAuthorityPda: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
      eventAuthority: { optional: false, derive: [] },
      selfProgram: { optional: false, derive: [] },
    },
  },
  cancelSubscription: {
    args: [],
    accounts: {
      subscriber: { optional: false, derive: null },
      planPda: { optional: false, derive: null },
      subscriptionPda: { optional: false, derive: ['planPda', 'subscriber'] },
      eventAuthority: { optional: false, derive: [] },
      selfProgram: { optional: false, derive: [] },
    },
  },
  resumeSubscription: {
    args: ['resumeData'],
    accounts: {
      subscriber: { optional: false, derive: null },
      planPda: { optional: false, derive: null },
      subscriptionPda: { optional: false, derive: ['planPda', 'subscriber'] },
      subscriptionAuthority: { optional: false, derive: null },
      eventAuthority: { optional: false, derive: [] },
      selfProgram: { optional: false, derive: [] },
    },
  },
  revokeSubscriptionAuthority: {
    args: [],
    accounts: {
      user: { optional: false, derive: null },
      userAta: { optional: false, derive: null },
      tokenMint: { optional: false, derive: null },
      tokenProgram: { optional: false, derive: null },
      subscriptionAuthority: { optional: false, derive: ['user', 'tokenMint'] },
    },
  },
  revokeAbandonedDelegation: {
    args: [],
    accounts: {
      payer: { optional: false, derive: null },
      delegationAccount: { optional: false, derive: null },
      subscriptionAuthority: { optional: false, derive: null },
    },
  },
  revokeAbandonedSubscription: {
    args: [],
    accounts: {
      payer: { optional: false, derive: null },
      subscriptionAccount: { optional: false, derive: null },
      subscriptionAuthority: { optional: false, derive: null },
      planPda: { optional: false, derive: null },
    },
  },
  cancelSubscriptionNow: {
    args: ['cancelSubscriptionNowData'],
    accounts: {
      subscriber: { optional: false, derive: null },
      merchant: { optional: false, derive: null },
      planPda: { optional: false, derive: null },
      subscriptionPda: { optional: false, derive: ['planPda', 'subscriber'] },
      eventAuthority: { optional: false, derive: [] },
      selfProgram: { optional: false, derive: [] },
    },
  },
} as const;

/** Release and explicit read transport for 'subscriptions' */
export const SUBSCRIPTIONS_READ = {
release: { programReleaseHash: "arete:h1:program-release:sha256:6b4aab0353344e865113725a2737d41fa2416a25720bd8bb59c7d60698dabfdb", programSpecHash: "arete:h1:program-spec:sha256:a10a30f17be97429474e990fdc24b87edf51e4c5fe0021b0651d434d11f3e698" },
transport: { kind: 'hosted-binding', binding: { endpoint: "https://programs.arete.run", programReadBindingId: "prb_eecefc9161054eff9f46061b98097c46", auth: {"acceptedKeyClasses":["anonymous","publishable","secret"],"audience":"arete:program-read","jwksUrl":"https://api.arete.run/ws/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["read"],"sessionEndpoint":"https://api.arete.run/ws/sessions","targetId":"prb_eecefc9161054eff9f46061b98097c46","targetKind":"program-read-binding","tokenTransport":"bearer"} } },
} as const;

/** All portable programs from the Subscriptions stack */
export const SUBSCRIPTIONS_PROGRAMS = {
  subscriptions: SUBSCRIPTIONS,
} as const;

/** Parallel release/read metadata keyed identically to SUBSCRIPTIONS_PROGRAMS */
export const SUBSCRIPTIONS_PROGRAM_READS = {
  subscriptions: SUBSCRIPTIONS_READ,
} as const;

export type SubscriptionsPrograms = typeof SUBSCRIPTIONS_PROGRAMS;

export default SUBSCRIPTIONS_PROGRAMS;