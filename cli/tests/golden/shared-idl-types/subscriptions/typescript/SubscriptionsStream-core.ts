import { z } from 'zod';
import { pda, literal, arg, programAccountRead, createInstructionHandler, type ErrorMetadata, buildInstruction, PROGRAM_OPERATION_EXTENSIONS, instructionOperation, createPreparedInstruction } from '@usearete/sdk';

export interface SubscriptionPlanId {
  address: string | null;
  owner: string | null;
  planPdaAddress: string | null;
}

export interface SubscriptionPlanMetrics {
  cancelCount: bigint | null;
  deletedAt: bigint | null;
  lastActivityAt: bigint | null;
  lastCancelledSubscriptionExpiresAtTs: bigint | null;
  lastCreatedSubscriber: string | null;
  lastSubscriptionCreatedAt: bigint | null;
  lastSubscriptionResumedAt: bigint | null;
  lastTransferAmountRaw: bigint | null;
  lastTransferPeriodEndTs: bigint | null;
  lastTransferPeriodStartTs: bigint | null;
  lastTransferReceiver: string | null;
  lastTransferSubscription: string | null;
  pullCount: bigint | null;
  resumeCount: bigint | null;
  subscriptionCount: bigint | null;
  transferVolumeRaw: bigint | null;
  updateCount: bigint | null;
}

export interface SubscriptionPlanState {
  data: PlanData | null;
  isActive: boolean | null;
  isSunset: boolean | null;
  mint: string | null;
  snapshot: Plan | null;
  status: number | null;
}

export interface SubscriptionPlan {
  id: SubscriptionPlanId;
  metrics: SubscriptionPlanMetrics;
  state: SubscriptionPlanState;
  tokenMetadata: TokenMetadata | null;
}

export interface PlanData {
  planId: bigint;
  mint: string;
  terms: Record<string, any>;
  endTs: bigint;
  destinations: string[];
  pullers: string[];
  metadataUri: number[];
}

export interface Plan {
  discriminator: number;
  owner: string;
  bump: number;
  status: number;
  data: Record<string, any>;
}

export type AccountDiscriminator = "SubscriptionAuthority" | "Plan" | "FixedDelegation" | "RecurringDelegation" | "SubscriptionDelegation";

export type PlanStatus = "Sunset" | "Active";

export interface TokenMetadata {
  mint: string;
  name?: string | null;
  symbol?: string | null;
  decimals?: number | null;
  logoUri?: string | null;
}

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

export const PlanDataSchema = z.object({
  planId: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  mint: z.string(),
  terms: z.record(z.any()),
  endTs: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  destinations: z.array(z.string()),
  pullers: z.array(z.string()),
  metadataUri: z.array(z.number()),
}).transform((value) => ({
  planId: value.planId,
  mint: value.mint,
  terms: value.terms,
  endTs: value.endTs,
  destinations: value.destinations,
  pullers: value.pullers,
  metadataUri: value.metadataUri,
}));

export const PlanSchema = z.object({
  discriminator: z.number(),
  owner: z.string(),
  bump: z.number(),
  status: z.number(),
  data: z.record(z.any()),
}).transform((value) => ({
  discriminator: value.discriminator,
  owner: value.owner,
  bump: value.bump,
  status: value.status,
  data: value.data,
}));

export const PlanDataPatchSchema = z.object({
  planId: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  mint: z.string().optional(),
  terms: z.record(z.any()).optional(),
  endTs: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  destinations: z.array(z.string()).optional(),
  pullers: z.array(z.string()).optional(),
  metadataUri: z.array(z.number()).optional(),
}).transform((value) => ({
  ...(value.planId !== undefined ? { planId: value.planId } : {}),
  ...(value.mint !== undefined ? { mint: value.mint } : {}),
  ...(value.terms !== undefined ? { terms: value.terms } : {}),
  ...(value.endTs !== undefined ? { endTs: value.endTs } : {}),
  ...(value.destinations !== undefined ? { destinations: value.destinations } : {}),
  ...(value.pullers !== undefined ? { pullers: value.pullers } : {}),
  ...(value.metadataUri !== undefined ? { metadataUri: value.metadataUri } : {}),
}));

export const PlanPatchSchema = z.object({
  discriminator: z.number().optional(),
  owner: z.string().optional(),
  bump: z.number().optional(),
  status: z.number().optional(),
  data: z.record(z.any()).optional(),
}).transform((value) => ({
  ...(value.discriminator !== undefined ? { discriminator: value.discriminator } : {}),
  ...(value.owner !== undefined ? { owner: value.owner } : {}),
  ...(value.bump !== undefined ? { bump: value.bump } : {}),
  ...(value.status !== undefined ? { status: value.status } : {}),
  ...(value.data !== undefined ? { data: value.data } : {}),
}));

export const AccountDiscriminatorSchema = z.enum(["SubscriptionAuthority", "Plan", "FixedDelegation", "RecurringDelegation", "SubscriptionDelegation"]);

export const PlanStatusSchema = z.enum(["Sunset", "Active"]);

export const SubscriptionPlanIdSchema = z.object({
  address: z.string().nullable().optional(),
  owner: z.string().nullable().optional(),
  planPda_address: z.string().nullable().optional(),
}).transform((value) => ({
  address: value.address,
  owner: value.owner,
  planPdaAddress: value.planPda_address,
}));

export const SubscriptionPlanIdPatchSchema = z.object({
  address: z.string().nullable().optional(),
  owner: z.string().nullable().optional(),
  planPda_address: z.string().nullable().optional(),
}).transform((value) => ({
  ...(value.address !== undefined ? { address: value.address } : {}),
  ...(value.owner !== undefined ? { owner: value.owner } : {}),
  ...(value.planPda_address !== undefined ? { planPdaAddress: value.planPda_address } : {}),
}));

export const SubscriptionPlanMetricsSchema = z.object({
  cancel_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  deleted_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_activity_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_cancelled_subscription_expires_at_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_created_subscriber: z.string().nullable().optional(),
  last_subscription_created_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_subscription_resumed_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_amount_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_period_end_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_period_start_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_receiver: z.string().nullable().optional(),
  last_transfer_subscription: z.string().nullable().optional(),
  pull_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  resume_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  subscription_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  transfer_volume_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  update_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  cancelCount: value.cancel_count,
  deletedAt: value.deleted_at,
  lastActivityAt: value.last_activity_at,
  lastCancelledSubscriptionExpiresAtTs: value.last_cancelled_subscription_expires_at_ts,
  lastCreatedSubscriber: value.last_created_subscriber,
  lastSubscriptionCreatedAt: value.last_subscription_created_at,
  lastSubscriptionResumedAt: value.last_subscription_resumed_at,
  lastTransferAmountRaw: value.last_transfer_amount_raw,
  lastTransferPeriodEndTs: value.last_transfer_period_end_ts,
  lastTransferPeriodStartTs: value.last_transfer_period_start_ts,
  lastTransferReceiver: value.last_transfer_receiver,
  lastTransferSubscription: value.last_transfer_subscription,
  pullCount: value.pull_count,
  resumeCount: value.resume_count,
  subscriptionCount: value.subscription_count,
  transferVolumeRaw: value.transfer_volume_raw,
  updateCount: value.update_count,
}));

export const SubscriptionPlanMetricsPatchSchema = z.object({
  cancel_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  deleted_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_activity_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_cancelled_subscription_expires_at_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_created_subscriber: z.string().nullable().optional(),
  last_subscription_created_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_subscription_resumed_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_amount_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_period_end_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_period_start_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_receiver: z.string().nullable().optional(),
  last_transfer_subscription: z.string().nullable().optional(),
  pull_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  resume_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  subscription_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  transfer_volume_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  update_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  ...(value.cancel_count !== undefined ? { cancelCount: value.cancel_count } : {}),
  ...(value.deleted_at !== undefined ? { deletedAt: value.deleted_at } : {}),
  ...(value.last_activity_at !== undefined ? { lastActivityAt: value.last_activity_at } : {}),
  ...(value.last_cancelled_subscription_expires_at_ts !== undefined ? { lastCancelledSubscriptionExpiresAtTs: value.last_cancelled_subscription_expires_at_ts } : {}),
  ...(value.last_created_subscriber !== undefined ? { lastCreatedSubscriber: value.last_created_subscriber } : {}),
  ...(value.last_subscription_created_at !== undefined ? { lastSubscriptionCreatedAt: value.last_subscription_created_at } : {}),
  ...(value.last_subscription_resumed_at !== undefined ? { lastSubscriptionResumedAt: value.last_subscription_resumed_at } : {}),
  ...(value.last_transfer_amount_raw !== undefined ? { lastTransferAmountRaw: value.last_transfer_amount_raw } : {}),
  ...(value.last_transfer_period_end_ts !== undefined ? { lastTransferPeriodEndTs: value.last_transfer_period_end_ts } : {}),
  ...(value.last_transfer_period_start_ts !== undefined ? { lastTransferPeriodStartTs: value.last_transfer_period_start_ts } : {}),
  ...(value.last_transfer_receiver !== undefined ? { lastTransferReceiver: value.last_transfer_receiver } : {}),
  ...(value.last_transfer_subscription !== undefined ? { lastTransferSubscription: value.last_transfer_subscription } : {}),
  ...(value.pull_count !== undefined ? { pullCount: value.pull_count } : {}),
  ...(value.resume_count !== undefined ? { resumeCount: value.resume_count } : {}),
  ...(value.subscription_count !== undefined ? { subscriptionCount: value.subscription_count } : {}),
  ...(value.transfer_volume_raw !== undefined ? { transferVolumeRaw: value.transfer_volume_raw } : {}),
  ...(value.update_count !== undefined ? { updateCount: value.update_count } : {}),
}));

export const SubscriptionPlanStateSchema = z.object({
  data: PlanDataSchema.nullable().optional(),
  is_active: z.boolean().nullable().optional(),
  is_sunset: z.boolean().nullable().optional(),
  mint: z.string().nullable().optional(),
  snapshot: PlanSchema.nullable().optional(),
  status: z.number().nullable().optional(),
}).transform((value) => ({
  data: value.data,
  isActive: value.is_active,
  isSunset: value.is_sunset,
  mint: value.mint,
  snapshot: value.snapshot,
  status: value.status,
}));

export const SubscriptionPlanStatePatchSchema = z.object({
  data: PlanDataPatchSchema.nullable().optional(),
  is_active: z.boolean().nullable().optional(),
  is_sunset: z.boolean().nullable().optional(),
  mint: z.string().nullable().optional(),
  snapshot: PlanPatchSchema.nullable().optional(),
  status: z.number().nullable().optional(),
}).transform((value) => ({
  ...(value.data !== undefined ? { data: value.data } : {}),
  ...(value.is_active !== undefined ? { isActive: value.is_active } : {}),
  ...(value.is_sunset !== undefined ? { isSunset: value.is_sunset } : {}),
  ...(value.mint !== undefined ? { mint: value.mint } : {}),
  ...(value.snapshot !== undefined ? { snapshot: value.snapshot } : {}),
  ...(value.status !== undefined ? { status: value.status } : {}),
}));

export const SubscriptionPlanSchema = z.object({
  id: SubscriptionPlanIdSchema,
  metrics: SubscriptionPlanMetricsSchema,
  state: SubscriptionPlanStateSchema,
  token_metadata: TokenMetadataSchema.nullable().optional(),
}).transform((value) => ({
  id: value.id,
  metrics: value.metrics,
  state: value.state,
  tokenMetadata: value.token_metadata,
}));

export const SubscriptionPlanPatchSchema = z.object({
  id: SubscriptionPlanIdPatchSchema.optional(),
  metrics: SubscriptionPlanMetricsPatchSchema.optional(),
  state: SubscriptionPlanStatePatchSchema.optional(),
  token_metadata: TokenMetadataPatchSchema.nullable().optional(),
}).transform((value) => ({
  ...(value.id !== undefined ? { id: value.id } : {}),
  ...(value.metrics !== undefined ? { metrics: value.metrics } : {}),
  ...(value.state !== undefined ? { state: value.state } : {}),
  ...(value.token_metadata !== undefined ? { tokenMetadata: value.token_metadata } : {}),
}));

export const SubscriptionPlanCompletedSchema = z.object({
  id: SubscriptionPlanIdSchema,
  metrics: SubscriptionPlanMetricsSchema,
  state: SubscriptionPlanStateSchema,
  token_metadata: TokenMetadataSchema.nullable().optional(),
}).transform((value) => ({
  id: value.id,
  metrics: value.metrics,
  state: value.state,
  tokenMetadata: value.token_metadata,
}));

export interface SubscriptionAuthorityId {
  address: string | null;
  subscriptionAuthorityAddress: string | null;
  tokenMint: string | null;
  user: string | null;
}

export interface SubscriptionAuthorityMetrics {
  closedAt: bigint | null;
  fixedDelegationCount: bigint | null;
  fixedPullCount: bigint | null;
  lastActivityAt: bigint | null;
  recurringDelegationCount: bigint | null;
  recurringPullCount: bigint | null;
  subscriptionPullCount: bigint | null;
}

export interface SubscriptionAuthorityState {
  bump: number | null;
  initId: bigint | null;
  payer: string | null;
  snapshot: SubscriptionAuthorityAccount | null;
}

export interface SubscriptionAuthority {
  id: SubscriptionAuthorityId;
  metrics: SubscriptionAuthorityMetrics;
  state: SubscriptionAuthorityState;
  tokenMetadata: TokenMetadata | null;
}

export interface SubscriptionAuthorityAccount {
  discriminator: number;
  user: string;
  tokenMint: string;
  payer: string;
  bump: number;
  initId: bigint;
}

export const SubscriptionAuthorityAccountSchema = z.object({
  discriminator: z.number(),
  user: z.string(),
  tokenMint: z.string(),
  payer: z.string(),
  bump: z.number(),
  initId: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  discriminator: value.discriminator,
  user: value.user,
  tokenMint: value.tokenMint,
  payer: value.payer,
  bump: value.bump,
  initId: value.initId,
}));

export const SubscriptionAuthorityAccountPatchSchema = z.object({
  discriminator: z.number().optional(),
  user: z.string().optional(),
  tokenMint: z.string().optional(),
  payer: z.string().optional(),
  bump: z.number().optional(),
  initId: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
}).transform((value) => ({
  ...(value.discriminator !== undefined ? { discriminator: value.discriminator } : {}),
  ...(value.user !== undefined ? { user: value.user } : {}),
  ...(value.tokenMint !== undefined ? { tokenMint: value.tokenMint } : {}),
  ...(value.payer !== undefined ? { payer: value.payer } : {}),
  ...(value.bump !== undefined ? { bump: value.bump } : {}),
  ...(value.initId !== undefined ? { initId: value.initId } : {}),
}));

export const SubscriptionAuthorityIdSchema = z.object({
  address: z.string().nullable().optional(),
  subscriptionAuthority_address: z.string().nullable().optional(),
  token_mint: z.string().nullable().optional(),
  user: z.string().nullable().optional(),
}).transform((value) => ({
  address: value.address,
  subscriptionAuthorityAddress: value.subscriptionAuthority_address,
  tokenMint: value.token_mint,
  user: value.user,
}));

export const SubscriptionAuthorityIdPatchSchema = z.object({
  address: z.string().nullable().optional(),
  subscriptionAuthority_address: z.string().nullable().optional(),
  token_mint: z.string().nullable().optional(),
  user: z.string().nullable().optional(),
}).transform((value) => ({
  ...(value.address !== undefined ? { address: value.address } : {}),
  ...(value.subscriptionAuthority_address !== undefined ? { subscriptionAuthorityAddress: value.subscriptionAuthority_address } : {}),
  ...(value.token_mint !== undefined ? { tokenMint: value.token_mint } : {}),
  ...(value.user !== undefined ? { user: value.user } : {}),
}));

export const SubscriptionAuthorityMetricsSchema = z.object({
  closed_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  fixed_delegation_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  fixed_pull_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_activity_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  recurring_delegation_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  recurring_pull_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  subscription_pull_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  closedAt: value.closed_at,
  fixedDelegationCount: value.fixed_delegation_count,
  fixedPullCount: value.fixed_pull_count,
  lastActivityAt: value.last_activity_at,
  recurringDelegationCount: value.recurring_delegation_count,
  recurringPullCount: value.recurring_pull_count,
  subscriptionPullCount: value.subscription_pull_count,
}));

export const SubscriptionAuthorityMetricsPatchSchema = z.object({
  closed_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  fixed_delegation_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  fixed_pull_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_activity_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  recurring_delegation_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  recurring_pull_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  subscription_pull_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  ...(value.closed_at !== undefined ? { closedAt: value.closed_at } : {}),
  ...(value.fixed_delegation_count !== undefined ? { fixedDelegationCount: value.fixed_delegation_count } : {}),
  ...(value.fixed_pull_count !== undefined ? { fixedPullCount: value.fixed_pull_count } : {}),
  ...(value.last_activity_at !== undefined ? { lastActivityAt: value.last_activity_at } : {}),
  ...(value.recurring_delegation_count !== undefined ? { recurringDelegationCount: value.recurring_delegation_count } : {}),
  ...(value.recurring_pull_count !== undefined ? { recurringPullCount: value.recurring_pull_count } : {}),
  ...(value.subscription_pull_count !== undefined ? { subscriptionPullCount: value.subscription_pull_count } : {}),
}));

export const SubscriptionAuthorityStateSchema = z.object({
  bump: z.number().nullable().optional(),
  init_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  payer: z.string().nullable().optional(),
  snapshot: SubscriptionAuthorityAccountSchema.nullable().optional(),
}).transform((value) => ({
  bump: value.bump,
  initId: value.init_id,
  payer: value.payer,
  snapshot: value.snapshot,
}));

export const SubscriptionAuthorityStatePatchSchema = z.object({
  bump: z.number().nullable().optional(),
  init_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  payer: z.string().nullable().optional(),
  snapshot: SubscriptionAuthorityAccountPatchSchema.nullable().optional(),
}).transform((value) => ({
  ...(value.bump !== undefined ? { bump: value.bump } : {}),
  ...(value.init_id !== undefined ? { initId: value.init_id } : {}),
  ...(value.payer !== undefined ? { payer: value.payer } : {}),
  ...(value.snapshot !== undefined ? { snapshot: value.snapshot } : {}),
}));

export const SubscriptionAuthoritySchema = z.object({
  id: SubscriptionAuthorityIdSchema,
  metrics: SubscriptionAuthorityMetricsSchema,
  state: SubscriptionAuthorityStateSchema,
  token_metadata: TokenMetadataSchema.nullable().optional(),
}).transform((value) => ({
  id: value.id,
  metrics: value.metrics,
  state: value.state,
  tokenMetadata: value.token_metadata,
}));

export const SubscriptionAuthorityPatchSchema = z.object({
  id: SubscriptionAuthorityIdPatchSchema.optional(),
  metrics: SubscriptionAuthorityMetricsPatchSchema.optional(),
  state: SubscriptionAuthorityStatePatchSchema.optional(),
  token_metadata: TokenMetadataPatchSchema.nullable().optional(),
}).transform((value) => ({
  ...(value.id !== undefined ? { id: value.id } : {}),
  ...(value.metrics !== undefined ? { metrics: value.metrics } : {}),
  ...(value.state !== undefined ? { state: value.state } : {}),
  ...(value.token_metadata !== undefined ? { tokenMetadata: value.token_metadata } : {}),
}));

export const SubscriptionAuthorityCompletedSchema = z.object({
  id: SubscriptionAuthorityIdSchema,
  metrics: SubscriptionAuthorityMetricsSchema,
  state: SubscriptionAuthorityStateSchema,
  token_metadata: TokenMetadataSchema.nullable().optional(),
}).transform((value) => ({
  id: value.id,
  metrics: value.metrics,
  state: value.state,
  tokenMetadata: value.token_metadata,
}));

export interface SubscriptionInstanceActivity {
  cancelCount: bigint | null;
  lastAmountPulledInPeriodRaw: bigint | null;
  lastStateChangeAt: bigint | null;
  lastTransferAmountRaw: bigint | null;
  lastTransferPeriodEndTs: bigint | null;
  lastTransferPeriodStartTs: bigint | null;
  lastTransferReceiver: string | null;
  pullCount: bigint | null;
  resumeCount: bigint | null;
  subscribeCount: bigint | null;
  totalTransferredRaw: bigint | null;
}

export interface SubscriptionInstanceId {
  address: string | null;
  merchant: string | null;
  planPda: string | null;
  subscriber: string | null;
  subscriptionAuthority: string | null;
  subscriptionPdaAddress: string | null;
}

export interface SubscriptionInstanceState {
  amountPulledInPeriod: bigint | null;
  currentPeriodStartTs: bigint | null;
  expiresAtTs: bigint | null;
  header: Header | null;
  snapshot: SubscriptionDelegation | null;
  terms: PlanTerms | null;
}

export interface SubscriptionInstance {
  activity: SubscriptionInstanceActivity;
  id: SubscriptionInstanceId;
  state: SubscriptionInstanceState;
}

export interface Header {
  discriminator: number;
  version: number;
  bump: number;
  delegator: string;
  delegatee: string;
  payer: string;
  initId: bigint;
}

export interface PlanTerms {
  amount: bigint;
  periodHours: bigint;
  createdAt: bigint;
}

export interface SubscriptionDelegation {
  header: Record<string, any>;
  terms: Record<string, any>;
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
  initId: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  discriminator: value.discriminator,
  version: value.version,
  bump: value.bump,
  delegator: value.delegator,
  delegatee: value.delegatee,
  payer: value.payer,
  initId: value.initId,
}));

export const PlanTermsSchema = z.object({
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  periodHours: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  createdAt: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  amount: value.amount,
  periodHours: value.periodHours,
  createdAt: value.createdAt,
}));

export const SubscriptionDelegationSchema = z.object({
  header: z.record(z.any()),
  terms: z.record(z.any()),
  amountPulledInPeriod: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  currentPeriodStartTs: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  expiresAtTs: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  header: value.header,
  terms: value.terms,
  amountPulledInPeriod: value.amountPulledInPeriod,
  currentPeriodStartTs: value.currentPeriodStartTs,
  expiresAtTs: value.expiresAtTs,
}));

export const HeaderPatchSchema = z.object({
  discriminator: z.number().optional(),
  version: z.number().optional(),
  bump: z.number().optional(),
  delegator: z.string().optional(),
  delegatee: z.string().optional(),
  payer: z.string().optional(),
  initId: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
}).transform((value) => ({
  ...(value.discriminator !== undefined ? { discriminator: value.discriminator } : {}),
  ...(value.version !== undefined ? { version: value.version } : {}),
  ...(value.bump !== undefined ? { bump: value.bump } : {}),
  ...(value.delegator !== undefined ? { delegator: value.delegator } : {}),
  ...(value.delegatee !== undefined ? { delegatee: value.delegatee } : {}),
  ...(value.payer !== undefined ? { payer: value.payer } : {}),
  ...(value.initId !== undefined ? { initId: value.initId } : {}),
}));

export const PlanTermsPatchSchema = z.object({
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  periodHours: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  createdAt: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
}).transform((value) => ({
  ...(value.amount !== undefined ? { amount: value.amount } : {}),
  ...(value.periodHours !== undefined ? { periodHours: value.periodHours } : {}),
  ...(value.createdAt !== undefined ? { createdAt: value.createdAt } : {}),
}));

export const SubscriptionDelegationPatchSchema = z.object({
  header: z.record(z.any()).optional(),
  terms: z.record(z.any()).optional(),
  amountPulledInPeriod: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  currentPeriodStartTs: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  expiresAtTs: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
}).transform((value) => ({
  ...(value.header !== undefined ? { header: value.header } : {}),
  ...(value.terms !== undefined ? { terms: value.terms } : {}),
  ...(value.amountPulledInPeriod !== undefined ? { amountPulledInPeriod: value.amountPulledInPeriod } : {}),
  ...(value.currentPeriodStartTs !== undefined ? { currentPeriodStartTs: value.currentPeriodStartTs } : {}),
  ...(value.expiresAtTs !== undefined ? { expiresAtTs: value.expiresAtTs } : {}),
}));

export const SubscriptionInstanceActivitySchema = z.object({
  cancel_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_amount_pulled_in_period_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_state_change_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_amount_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_period_end_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_period_start_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_receiver: z.string().nullable().optional(),
  pull_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  resume_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  subscribe_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  total_transferred_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  cancelCount: value.cancel_count,
  lastAmountPulledInPeriodRaw: value.last_amount_pulled_in_period_raw,
  lastStateChangeAt: value.last_state_change_at,
  lastTransferAmountRaw: value.last_transfer_amount_raw,
  lastTransferPeriodEndTs: value.last_transfer_period_end_ts,
  lastTransferPeriodStartTs: value.last_transfer_period_start_ts,
  lastTransferReceiver: value.last_transfer_receiver,
  pullCount: value.pull_count,
  resumeCount: value.resume_count,
  subscribeCount: value.subscribe_count,
  totalTransferredRaw: value.total_transferred_raw,
}));

export const SubscriptionInstanceActivityPatchSchema = z.object({
  cancel_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_amount_pulled_in_period_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_state_change_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_amount_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_period_end_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_period_start_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_receiver: z.string().nullable().optional(),
  pull_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  resume_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  subscribe_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  total_transferred_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  ...(value.cancel_count !== undefined ? { cancelCount: value.cancel_count } : {}),
  ...(value.last_amount_pulled_in_period_raw !== undefined ? { lastAmountPulledInPeriodRaw: value.last_amount_pulled_in_period_raw } : {}),
  ...(value.last_state_change_at !== undefined ? { lastStateChangeAt: value.last_state_change_at } : {}),
  ...(value.last_transfer_amount_raw !== undefined ? { lastTransferAmountRaw: value.last_transfer_amount_raw } : {}),
  ...(value.last_transfer_period_end_ts !== undefined ? { lastTransferPeriodEndTs: value.last_transfer_period_end_ts } : {}),
  ...(value.last_transfer_period_start_ts !== undefined ? { lastTransferPeriodStartTs: value.last_transfer_period_start_ts } : {}),
  ...(value.last_transfer_receiver !== undefined ? { lastTransferReceiver: value.last_transfer_receiver } : {}),
  ...(value.pull_count !== undefined ? { pullCount: value.pull_count } : {}),
  ...(value.resume_count !== undefined ? { resumeCount: value.resume_count } : {}),
  ...(value.subscribe_count !== undefined ? { subscribeCount: value.subscribe_count } : {}),
  ...(value.total_transferred_raw !== undefined ? { totalTransferredRaw: value.total_transferred_raw } : {}),
}));

export const SubscriptionInstanceIdSchema = z.object({
  address: z.string().nullable().optional(),
  merchant: z.string().nullable().optional(),
  planPda: z.string().nullable().optional(),
  subscriber: z.string().nullable().optional(),
  subscriptionAuthority: z.string().nullable().optional(),
  subscriptionPda_address: z.string().nullable().optional(),
}).transform((value) => ({
  address: value.address,
  merchant: value.merchant,
  planPda: value.planPda,
  subscriber: value.subscriber,
  subscriptionAuthority: value.subscriptionAuthority,
  subscriptionPdaAddress: value.subscriptionPda_address,
}));

export const SubscriptionInstanceIdPatchSchema = z.object({
  address: z.string().nullable().optional(),
  merchant: z.string().nullable().optional(),
  planPda: z.string().nullable().optional(),
  subscriber: z.string().nullable().optional(),
  subscriptionAuthority: z.string().nullable().optional(),
  subscriptionPda_address: z.string().nullable().optional(),
}).transform((value) => ({
  ...(value.address !== undefined ? { address: value.address } : {}),
  ...(value.merchant !== undefined ? { merchant: value.merchant } : {}),
  ...(value.planPda !== undefined ? { planPda: value.planPda } : {}),
  ...(value.subscriber !== undefined ? { subscriber: value.subscriber } : {}),
  ...(value.subscriptionAuthority !== undefined ? { subscriptionAuthority: value.subscriptionAuthority } : {}),
  ...(value.subscriptionPda_address !== undefined ? { subscriptionPdaAddress: value.subscriptionPda_address } : {}),
}));

export const SubscriptionInstanceStateSchema = z.object({
  amount_pulled_in_period: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  current_period_start_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  expires_at_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  header: HeaderSchema.nullable().optional(),
  snapshot: SubscriptionDelegationSchema.nullable().optional(),
  terms: PlanTermsSchema.nullable().optional(),
}).transform((value) => ({
  amountPulledInPeriod: value.amount_pulled_in_period,
  currentPeriodStartTs: value.current_period_start_ts,
  expiresAtTs: value.expires_at_ts,
  header: value.header,
  snapshot: value.snapshot,
  terms: value.terms,
}));

export const SubscriptionInstanceStatePatchSchema = z.object({
  amount_pulled_in_period: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  current_period_start_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  expires_at_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  header: HeaderPatchSchema.nullable().optional(),
  snapshot: SubscriptionDelegationPatchSchema.nullable().optional(),
  terms: PlanTermsPatchSchema.nullable().optional(),
}).transform((value) => ({
  ...(value.amount_pulled_in_period !== undefined ? { amountPulledInPeriod: value.amount_pulled_in_period } : {}),
  ...(value.current_period_start_ts !== undefined ? { currentPeriodStartTs: value.current_period_start_ts } : {}),
  ...(value.expires_at_ts !== undefined ? { expiresAtTs: value.expires_at_ts } : {}),
  ...(value.header !== undefined ? { header: value.header } : {}),
  ...(value.snapshot !== undefined ? { snapshot: value.snapshot } : {}),
  ...(value.terms !== undefined ? { terms: value.terms } : {}),
}));

export const SubscriptionInstanceSchema = z.object({
  activity: SubscriptionInstanceActivitySchema,
  id: SubscriptionInstanceIdSchema,
  state: SubscriptionInstanceStateSchema,
}).transform((value) => ({
  activity: value.activity,
  id: value.id,
  state: value.state,
}));

export const SubscriptionInstancePatchSchema = z.object({
  activity: SubscriptionInstanceActivityPatchSchema.optional(),
  id: SubscriptionInstanceIdPatchSchema.optional(),
  state: SubscriptionInstanceStatePatchSchema.optional(),
}).transform((value) => ({
  ...(value.activity !== undefined ? { activity: value.activity } : {}),
  ...(value.id !== undefined ? { id: value.id } : {}),
  ...(value.state !== undefined ? { state: value.state } : {}),
}));

export const SubscriptionInstanceCompletedSchema = z.object({
  activity: SubscriptionInstanceActivitySchema,
  id: SubscriptionInstanceIdSchema,
  state: SubscriptionInstanceStateSchema,
}).transform((value) => ({
  activity: value.activity,
  id: value.id,
  state: value.state,
}));

export interface FixedDelegationActivity {
  createdAt: bigint | null;
  lastRemainingAmountRaw: bigint | null;
  lastTransferAmountRaw: bigint | null;
  lastTransferAt: bigint | null;
  lastTransferReceiver: string | null;
  revokedAt: bigint | null;
  totalTransferredRaw: bigint | null;
  transferCount: bigint | null;
}

export interface FixedDelegationId {
  address: string | null;
  delegatee: string | null;
  delegationAccountAddress: string | null;
  delegationPdaAddress: string | null;
  delegator: string | null;
}

export interface FixedDelegationState {
  amount: bigint | null;
  amountUi: number | null;
  expiryTs: bigint | null;
  header: Header | null;
  mint: string | null;
  snapshot: FixedDelegationAccount | null;
  subscriptionAuthority: string | null;
}

export interface FixedDelegation {
  activity: FixedDelegationActivity;
  id: FixedDelegationId;
  state: FixedDelegationState;
  tokenMetadata: TokenMetadata | null;
}

export interface FixedDelegationAccount {
  header: Record<string, any>;
  subscriptionAuthority: string;
  mint: string;
  amount: bigint;
  expiryTs: bigint;
}

export const FixedDelegationAccountSchema = z.object({
  header: z.record(z.any()),
  subscriptionAuthority: z.string(),
  mint: z.string(),
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  expiryTs: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  header: value.header,
  subscriptionAuthority: value.subscriptionAuthority,
  mint: value.mint,
  amount: value.amount,
  expiryTs: value.expiryTs,
}));

export const FixedDelegationAccountPatchSchema = z.object({
  header: z.record(z.any()).optional(),
  subscriptionAuthority: z.string().optional(),
  mint: z.string().optional(),
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  expiryTs: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
}).transform((value) => ({
  ...(value.header !== undefined ? { header: value.header } : {}),
  ...(value.subscriptionAuthority !== undefined ? { subscriptionAuthority: value.subscriptionAuthority } : {}),
  ...(value.mint !== undefined ? { mint: value.mint } : {}),
  ...(value.amount !== undefined ? { amount: value.amount } : {}),
  ...(value.expiryTs !== undefined ? { expiryTs: value.expiryTs } : {}),
}));

export const FixedDelegationActivitySchema = z.object({
  created_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_remaining_amount_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_amount_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_receiver: z.string().nullable().optional(),
  revoked_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  total_transferred_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  transfer_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  createdAt: value.created_at,
  lastRemainingAmountRaw: value.last_remaining_amount_raw,
  lastTransferAmountRaw: value.last_transfer_amount_raw,
  lastTransferAt: value.last_transfer_at,
  lastTransferReceiver: value.last_transfer_receiver,
  revokedAt: value.revoked_at,
  totalTransferredRaw: value.total_transferred_raw,
  transferCount: value.transfer_count,
}));

export const FixedDelegationActivityPatchSchema = z.object({
  created_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_remaining_amount_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_amount_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_receiver: z.string().nullable().optional(),
  revoked_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  total_transferred_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  transfer_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  ...(value.created_at !== undefined ? { createdAt: value.created_at } : {}),
  ...(value.last_remaining_amount_raw !== undefined ? { lastRemainingAmountRaw: value.last_remaining_amount_raw } : {}),
  ...(value.last_transfer_amount_raw !== undefined ? { lastTransferAmountRaw: value.last_transfer_amount_raw } : {}),
  ...(value.last_transfer_at !== undefined ? { lastTransferAt: value.last_transfer_at } : {}),
  ...(value.last_transfer_receiver !== undefined ? { lastTransferReceiver: value.last_transfer_receiver } : {}),
  ...(value.revoked_at !== undefined ? { revokedAt: value.revoked_at } : {}),
  ...(value.total_transferred_raw !== undefined ? { totalTransferredRaw: value.total_transferred_raw } : {}),
  ...(value.transfer_count !== undefined ? { transferCount: value.transfer_count } : {}),
}));

export const FixedDelegationIdSchema = z.object({
  address: z.string().nullable().optional(),
  delegatee: z.string().nullable().optional(),
  delegationAccount_address: z.string().nullable().optional(),
  delegationPda_address: z.string().nullable().optional(),
  delegator: z.string().nullable().optional(),
}).transform((value) => ({
  address: value.address,
  delegatee: value.delegatee,
  delegationAccountAddress: value.delegationAccount_address,
  delegationPdaAddress: value.delegationPda_address,
  delegator: value.delegator,
}));

export const FixedDelegationIdPatchSchema = z.object({
  address: z.string().nullable().optional(),
  delegatee: z.string().nullable().optional(),
  delegationAccount_address: z.string().nullable().optional(),
  delegationPda_address: z.string().nullable().optional(),
  delegator: z.string().nullable().optional(),
}).transform((value) => ({
  ...(value.address !== undefined ? { address: value.address } : {}),
  ...(value.delegatee !== undefined ? { delegatee: value.delegatee } : {}),
  ...(value.delegationAccount_address !== undefined ? { delegationAccountAddress: value.delegationAccount_address } : {}),
  ...(value.delegationPda_address !== undefined ? { delegationPdaAddress: value.delegationPda_address } : {}),
  ...(value.delegator !== undefined ? { delegator: value.delegator } : {}),
}));

export const FixedDelegationStateSchema = z.object({
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  amount_ui: z.number().nullable().optional(),
  expiry_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  header: HeaderSchema.nullable().optional(),
  mint: z.string().nullable().optional(),
  snapshot: FixedDelegationAccountSchema.nullable().optional(),
  subscription_authority: z.string().nullable().optional(),
}).transform((value) => ({
  amount: value.amount,
  amountUi: value.amount_ui,
  expiryTs: value.expiry_ts,
  header: value.header,
  mint: value.mint,
  snapshot: value.snapshot,
  subscriptionAuthority: value.subscription_authority,
}));

export const FixedDelegationStatePatchSchema = z.object({
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  amount_ui: z.number().nullable().optional(),
  expiry_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  header: HeaderPatchSchema.nullable().optional(),
  mint: z.string().nullable().optional(),
  snapshot: FixedDelegationAccountPatchSchema.nullable().optional(),
  subscription_authority: z.string().nullable().optional(),
}).transform((value) => ({
  ...(value.amount !== undefined ? { amount: value.amount } : {}),
  ...(value.amount_ui !== undefined ? { amountUi: value.amount_ui } : {}),
  ...(value.expiry_ts !== undefined ? { expiryTs: value.expiry_ts } : {}),
  ...(value.header !== undefined ? { header: value.header } : {}),
  ...(value.mint !== undefined ? { mint: value.mint } : {}),
  ...(value.snapshot !== undefined ? { snapshot: value.snapshot } : {}),
  ...(value.subscription_authority !== undefined ? { subscriptionAuthority: value.subscription_authority } : {}),
}));

export const FixedDelegationSchema = z.object({
  activity: FixedDelegationActivitySchema,
  id: FixedDelegationIdSchema,
  state: FixedDelegationStateSchema,
  token_metadata: TokenMetadataSchema.nullable().optional(),
}).transform((value) => ({
  activity: value.activity,
  id: value.id,
  state: value.state,
  tokenMetadata: value.token_metadata,
}));

export const FixedDelegationPatchSchema = z.object({
  activity: FixedDelegationActivityPatchSchema.optional(),
  id: FixedDelegationIdPatchSchema.optional(),
  state: FixedDelegationStatePatchSchema.optional(),
  token_metadata: TokenMetadataPatchSchema.nullable().optional(),
}).transform((value) => ({
  ...(value.activity !== undefined ? { activity: value.activity } : {}),
  ...(value.id !== undefined ? { id: value.id } : {}),
  ...(value.state !== undefined ? { state: value.state } : {}),
  ...(value.token_metadata !== undefined ? { tokenMetadata: value.token_metadata } : {}),
}));

export const FixedDelegationCompletedSchema = z.object({
  activity: FixedDelegationActivitySchema,
  id: FixedDelegationIdSchema,
  state: FixedDelegationStateSchema,
  token_metadata: TokenMetadataSchema.nullable().optional(),
}).transform((value) => ({
  activity: value.activity,
  id: value.id,
  state: value.state,
  tokenMetadata: value.token_metadata,
}));

export interface RecurringDelegationActivity {
  createdAt: bigint | null;
  lastAmountPulledInPeriodRaw: bigint | null;
  lastTransferAmountRaw: bigint | null;
  lastTransferAt: bigint | null;
  lastTransferPeriodEndTs: bigint | null;
  lastTransferPeriodStartTs: bigint | null;
  lastTransferReceiver: string | null;
  revokedAt: bigint | null;
  totalTransferredRaw: bigint | null;
  transferCount: bigint | null;
}

export interface RecurringDelegationId {
  address: string | null;
  delegatee: string | null;
  delegationAccountAddress: string | null;
  delegationPdaAddress: string | null;
  delegator: string | null;
}

export interface RecurringDelegationState {
  amountPerPeriod: bigint | null;
  amountPerPeriodUi: number | null;
  amountPulledInPeriod: bigint | null;
  amountPulledInPeriodUi: number | null;
  currentPeriodStartTs: bigint | null;
  expiryTs: bigint | null;
  header: Header | null;
  mint: string | null;
  periodLengthS: bigint | null;
  snapshot: RecurringDelegationAccount | null;
  subscriptionAuthority: string | null;
}

export interface RecurringDelegation {
  activity: RecurringDelegationActivity;
  id: RecurringDelegationId;
  state: RecurringDelegationState;
  tokenMetadata: TokenMetadata | null;
}

export interface RecurringDelegationAccount {
  header: Record<string, any>;
  subscriptionAuthority: string;
  mint: string;
  currentPeriodStartTs: bigint;
  periodLengthS: bigint;
  expiryTs: bigint;
  amountPerPeriod: bigint;
  amountPulledInPeriod: bigint;
}

export const RecurringDelegationAccountSchema = z.object({
  header: z.record(z.any()),
  subscriptionAuthority: z.string(),
  mint: z.string(),
  currentPeriodStartTs: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  periodLengthS: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  expiryTs: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  amountPerPeriod: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  amountPulledInPeriod: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  header: value.header,
  subscriptionAuthority: value.subscriptionAuthority,
  mint: value.mint,
  currentPeriodStartTs: value.currentPeriodStartTs,
  periodLengthS: value.periodLengthS,
  expiryTs: value.expiryTs,
  amountPerPeriod: value.amountPerPeriod,
  amountPulledInPeriod: value.amountPulledInPeriod,
}));

export const RecurringDelegationAccountPatchSchema = z.object({
  header: z.record(z.any()).optional(),
  subscriptionAuthority: z.string().optional(),
  mint: z.string().optional(),
  currentPeriodStartTs: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  periodLengthS: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  expiryTs: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  amountPerPeriod: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
  amountPulledInPeriod: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).optional(),
}).transform((value) => ({
  ...(value.header !== undefined ? { header: value.header } : {}),
  ...(value.subscriptionAuthority !== undefined ? { subscriptionAuthority: value.subscriptionAuthority } : {}),
  ...(value.mint !== undefined ? { mint: value.mint } : {}),
  ...(value.currentPeriodStartTs !== undefined ? { currentPeriodStartTs: value.currentPeriodStartTs } : {}),
  ...(value.periodLengthS !== undefined ? { periodLengthS: value.periodLengthS } : {}),
  ...(value.expiryTs !== undefined ? { expiryTs: value.expiryTs } : {}),
  ...(value.amountPerPeriod !== undefined ? { amountPerPeriod: value.amountPerPeriod } : {}),
  ...(value.amountPulledInPeriod !== undefined ? { amountPulledInPeriod: value.amountPulledInPeriod } : {}),
}));

export const RecurringDelegationActivitySchema = z.object({
  created_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_amount_pulled_in_period_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_amount_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_period_end_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_period_start_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_receiver: z.string().nullable().optional(),
  revoked_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  total_transferred_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  transfer_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  createdAt: value.created_at,
  lastAmountPulledInPeriodRaw: value.last_amount_pulled_in_period_raw,
  lastTransferAmountRaw: value.last_transfer_amount_raw,
  lastTransferAt: value.last_transfer_at,
  lastTransferPeriodEndTs: value.last_transfer_period_end_ts,
  lastTransferPeriodStartTs: value.last_transfer_period_start_ts,
  lastTransferReceiver: value.last_transfer_receiver,
  revokedAt: value.revoked_at,
  totalTransferredRaw: value.total_transferred_raw,
  transferCount: value.transfer_count,
}));

export const RecurringDelegationActivityPatchSchema = z.object({
  created_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_amount_pulled_in_period_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_amount_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_period_end_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_period_start_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  last_transfer_receiver: z.string().nullable().optional(),
  revoked_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  total_transferred_raw: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  transfer_count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  ...(value.created_at !== undefined ? { createdAt: value.created_at } : {}),
  ...(value.last_amount_pulled_in_period_raw !== undefined ? { lastAmountPulledInPeriodRaw: value.last_amount_pulled_in_period_raw } : {}),
  ...(value.last_transfer_amount_raw !== undefined ? { lastTransferAmountRaw: value.last_transfer_amount_raw } : {}),
  ...(value.last_transfer_at !== undefined ? { lastTransferAt: value.last_transfer_at } : {}),
  ...(value.last_transfer_period_end_ts !== undefined ? { lastTransferPeriodEndTs: value.last_transfer_period_end_ts } : {}),
  ...(value.last_transfer_period_start_ts !== undefined ? { lastTransferPeriodStartTs: value.last_transfer_period_start_ts } : {}),
  ...(value.last_transfer_receiver !== undefined ? { lastTransferReceiver: value.last_transfer_receiver } : {}),
  ...(value.revoked_at !== undefined ? { revokedAt: value.revoked_at } : {}),
  ...(value.total_transferred_raw !== undefined ? { totalTransferredRaw: value.total_transferred_raw } : {}),
  ...(value.transfer_count !== undefined ? { transferCount: value.transfer_count } : {}),
}));

export const RecurringDelegationIdSchema = z.object({
  address: z.string().nullable().optional(),
  delegatee: z.string().nullable().optional(),
  delegationAccount_address: z.string().nullable().optional(),
  delegationPda_address: z.string().nullable().optional(),
  delegator: z.string().nullable().optional(),
}).transform((value) => ({
  address: value.address,
  delegatee: value.delegatee,
  delegationAccountAddress: value.delegationAccount_address,
  delegationPdaAddress: value.delegationPda_address,
  delegator: value.delegator,
}));

export const RecurringDelegationIdPatchSchema = z.object({
  address: z.string().nullable().optional(),
  delegatee: z.string().nullable().optional(),
  delegationAccount_address: z.string().nullable().optional(),
  delegationPda_address: z.string().nullable().optional(),
  delegator: z.string().nullable().optional(),
}).transform((value) => ({
  ...(value.address !== undefined ? { address: value.address } : {}),
  ...(value.delegatee !== undefined ? { delegatee: value.delegatee } : {}),
  ...(value.delegationAccount_address !== undefined ? { delegationAccountAddress: value.delegationAccount_address } : {}),
  ...(value.delegationPda_address !== undefined ? { delegationPdaAddress: value.delegationPda_address } : {}),
  ...(value.delegator !== undefined ? { delegator: value.delegator } : {}),
}));

export const RecurringDelegationStateSchema = z.object({
  amount_per_period: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  amount_per_period_ui: z.number().nullable().optional(),
  amount_pulled_in_period: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  amount_pulled_in_period_ui: z.number().nullable().optional(),
  current_period_start_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  expiry_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  header: HeaderSchema.nullable().optional(),
  mint: z.string().nullable().optional(),
  period_length_s: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  snapshot: RecurringDelegationAccountSchema.nullable().optional(),
  subscription_authority: z.string().nullable().optional(),
}).transform((value) => ({
  amountPerPeriod: value.amount_per_period,
  amountPerPeriodUi: value.amount_per_period_ui,
  amountPulledInPeriod: value.amount_pulled_in_period,
  amountPulledInPeriodUi: value.amount_pulled_in_period_ui,
  currentPeriodStartTs: value.current_period_start_ts,
  expiryTs: value.expiry_ts,
  header: value.header,
  mint: value.mint,
  periodLengthS: value.period_length_s,
  snapshot: value.snapshot,
  subscriptionAuthority: value.subscription_authority,
}));

export const RecurringDelegationStatePatchSchema = z.object({
  amount_per_period: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  amount_per_period_ui: z.number().nullable().optional(),
  amount_pulled_in_period: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  amount_pulled_in_period_ui: z.number().nullable().optional(),
  current_period_start_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  expiry_ts: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  header: HeaderPatchSchema.nullable().optional(),
  mint: z.string().nullable().optional(),
  period_length_s: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
  snapshot: RecurringDelegationAccountPatchSchema.nullable().optional(),
  subscription_authority: z.string().nullable().optional(),
}).transform((value) => ({
  ...(value.amount_per_period !== undefined ? { amountPerPeriod: value.amount_per_period } : {}),
  ...(value.amount_per_period_ui !== undefined ? { amountPerPeriodUi: value.amount_per_period_ui } : {}),
  ...(value.amount_pulled_in_period !== undefined ? { amountPulledInPeriod: value.amount_pulled_in_period } : {}),
  ...(value.amount_pulled_in_period_ui !== undefined ? { amountPulledInPeriodUi: value.amount_pulled_in_period_ui } : {}),
  ...(value.current_period_start_ts !== undefined ? { currentPeriodStartTs: value.current_period_start_ts } : {}),
  ...(value.expiry_ts !== undefined ? { expiryTs: value.expiry_ts } : {}),
  ...(value.header !== undefined ? { header: value.header } : {}),
  ...(value.mint !== undefined ? { mint: value.mint } : {}),
  ...(value.period_length_s !== undefined ? { periodLengthS: value.period_length_s } : {}),
  ...(value.snapshot !== undefined ? { snapshot: value.snapshot } : {}),
  ...(value.subscription_authority !== undefined ? { subscriptionAuthority: value.subscription_authority } : {}),
}));

export const RecurringDelegationSchema = z.object({
  activity: RecurringDelegationActivitySchema,
  id: RecurringDelegationIdSchema,
  state: RecurringDelegationStateSchema,
  token_metadata: TokenMetadataSchema.nullable().optional(),
}).transform((value) => ({
  activity: value.activity,
  id: value.id,
  state: value.state,
  tokenMetadata: value.token_metadata,
}));

export const RecurringDelegationPatchSchema = z.object({
  activity: RecurringDelegationActivityPatchSchema.optional(),
  id: RecurringDelegationIdPatchSchema.optional(),
  state: RecurringDelegationStatePatchSchema.optional(),
  token_metadata: TokenMetadataPatchSchema.nullable().optional(),
}).transform((value) => ({
  ...(value.activity !== undefined ? { activity: value.activity } : {}),
  ...(value.id !== undefined ? { id: value.id } : {}),
  ...(value.state !== undefined ? { state: value.state } : {}),
  ...(value.token_metadata !== undefined ? { tokenMetadata: value.token_metadata } : {}),
}));

export const RecurringDelegationCompletedSchema = z.object({
  activity: RecurringDelegationActivitySchema,
  id: RecurringDelegationIdSchema,
  state: RecurringDelegationStateSchema,
  token_metadata: TokenMetadataSchema.nullable().optional(),
}).transform((value) => ({
  activity: value.activity,
  id: value.id,
  state: value.state,
  tokenMetadata: value.token_metadata,
}));

export interface SubscriptionsHeader {
  discriminator: number;
  version: number;
  bump: number;
  delegator: string;
  delegatee: string;
  payer: string;
  initId: bigint;
}

export interface SubscriptionsPlanData {
  planId: bigint;
  mint: string;
  terms: SubscriptionsPlanTerms;
  endTs: bigint;
  destinations: string[];
  pullers: string[];
  metadataUri: number[];
}

export interface SubscriptionsPlanTerms {
  amount: bigint;
  periodHours: bigint;
  createdAt: bigint;
}

export interface SubscriptionsFixedDelegation {
  header: SubscriptionsHeader;
  subscriptionAuthority: string;
  mint: string;
  amount: bigint;
  expiryTs: bigint;
}

export interface SubscriptionsPlan {
  discriminator: number;
  owner: string;
  bump: number;
  status: number;
  data: SubscriptionsPlanData;
}

export interface SubscriptionsRecurringDelegation {
  header: SubscriptionsHeader;
  subscriptionAuthority: string;
  mint: string;
  currentPeriodStartTs: bigint;
  periodLengthS: bigint;
  expiryTs: bigint;
  amountPerPeriod: bigint;
  amountPulledInPeriod: bigint;
}

export interface SubscriptionsSubscriptionAuthority {
  discriminator: number;
  user: string;
  tokenMint: string;
  payer: string;
  bump: number;
  initId: bigint;
}

export interface SubscriptionsSubscriptionDelegation {
  header: SubscriptionsHeader;
  terms: SubscriptionsPlanTerms;
  amountPulledInPeriod: bigint;
  currentPeriodStartTs: bigint;
  expiresAtTs: bigint;
}

export const SubscriptionsHeaderSchema = z.object({
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

export const SubscriptionsPlanDataSchema = z.object({
  plan_id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  mint: z.string(),
  terms: z.lazy(() => SubscriptionsPlanTermsSchema),
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

export const SubscriptionsPlanTermsSchema = z.object({
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  period_hours: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  created_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  amount: value.amount,
  periodHours: value.period_hours,
  createdAt: value.created_at,
}));

export const SubscriptionsFixedDelegationSchema = z.object({
  header: z.lazy(() => SubscriptionsHeaderSchema),
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

export const SubscriptionsPlanSchema = z.object({
  discriminator: z.number(),
  owner: z.string(),
  bump: z.number(),
  status: z.number(),
  data: z.lazy(() => SubscriptionsPlanDataSchema),
}).transform((value) => ({
  discriminator: value.discriminator,
  owner: value.owner,
  bump: value.bump,
  status: value.status,
  data: value.data,
}));

export const SubscriptionsRecurringDelegationSchema = z.object({
  header: z.lazy(() => SubscriptionsHeaderSchema),
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

export const SubscriptionsSubscriptionAuthoritySchema = z.object({
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

export const SubscriptionsSubscriptionDelegationSchema = z.object({
  header: z.lazy(() => SubscriptionsHeaderSchema),
  terms: z.lazy(() => SubscriptionsPlanTermsSchema),
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
export type SubscriptionsStreamProgramError =
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

const SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS: ErrorMetadata[] = [
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

export type InitSubscriptionAuthorityError = SubscriptionsStreamProgramError;

export const initSubscriptionAuthorityInstruction = createInstructionHandler<InitSubscriptionAuthorityParams, InitSubscriptionAuthorityError>({
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
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
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

export type CreateFixedDelegationError = SubscriptionsStreamProgramError;

export const createFixedDelegationInstruction = createInstructionHandler<CreateFixedDelegationParams, CreateFixedDelegationError>({
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
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
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

export type CreateRecurringDelegationError = SubscriptionsStreamProgramError;

export const createRecurringDelegationInstruction = createInstructionHandler<CreateRecurringDelegationParams, CreateRecurringDelegationError>({
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
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
});

export interface RevokeDelegationParams {
  authority: string;
  delegationAccount: string;
}

export type RevokeDelegationError = SubscriptionsStreamProgramError;

export const revokeDelegationInstruction = createInstructionHandler<RevokeDelegationParams, RevokeDelegationError>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [3],
  args: [],
  accounts: [
    { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'delegationAccount', isSigner: false, isWritable: true, category: 'userProvided' },
  ],
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
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

export type TransferFixedError = SubscriptionsStreamProgramError;

export const transferFixedInstruction = createInstructionHandler<TransferFixedParams, TransferFixedError>({
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
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
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

export type TransferRecurringError = SubscriptionsStreamProgramError;

export const transferRecurringInstruction = createInstructionHandler<TransferRecurringParams, TransferRecurringError>({
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
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
});

export interface CloseSubscriptionAuthorityParams {
  user: string;
  subscriptionAuthority?: string;
  resolve?: {
    tokenMint?: string;
    user?: string;
  };
}

export type CloseSubscriptionAuthorityError = SubscriptionsStreamProgramError;

export const closeSubscriptionAuthorityInstruction = createInstructionHandler<CloseSubscriptionAuthorityParams, CloseSubscriptionAuthorityError>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [6],
  args: [],
  accounts: [
    { name: 'user', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'subscriptionAuthority', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'SubscriptionAuthority' }, { type: 'argRef', argName: 'user', argType: 'pubkey' }, { type: 'argRef', argName: 'tokenMint', argType: 'pubkey' }] } },
  ],
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
});

export interface CreatePlanParams {
  planData: PlanDataInput;
  merchant: string;
  planPda: string;
  tokenMint: string;
}

export type CreatePlanError = SubscriptionsStreamProgramError;

export const createPlanInstruction = createInstructionHandler<CreatePlanParams, CreatePlanError>({
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
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
});

export interface UpdatePlanParams {
  updatePlanData: UpdatePlanData;
  owner: string;
  planPda: string;
  eventAuthority?: string;
}

export type UpdatePlanError = SubscriptionsStreamProgramError;

export const updatePlanInstruction = createInstructionHandler<UpdatePlanParams, UpdatePlanError>({
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
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
});

export interface DeletePlanParams {
  owner: string;
  planPda: string;
}

export type DeletePlanError = SubscriptionsStreamProgramError;

export const deletePlanInstruction = createInstructionHandler<DeletePlanParams, DeletePlanError>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [9],
  args: [],
  accounts: [
    { name: 'owner', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'planPda', isSigner: false, isWritable: true, category: 'userProvided' },
  ],
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
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

export type TransferSubscriptionError = SubscriptionsStreamProgramError;

export const transferSubscriptionInstruction = createInstructionHandler<TransferSubscriptionParams, TransferSubscriptionError>({
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
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
});

export interface SubscribeParams {
  subscribeData: SubscribeData;
  subscriber: string;
  merchant: string;
  planPda: string;
  subscriptionPda?: string;
  subscriptionAuthorityPda: string;
}

export type SubscribeError = SubscriptionsStreamProgramError;

export const subscribeInstruction = createInstructionHandler<SubscribeParams, SubscribeError>({
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
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
});

export interface CancelSubscriptionParams {
  subscriber: string;
  planPda: string;
  subscriptionPda?: string;
}

export type CancelSubscriptionError = SubscriptionsStreamProgramError;

export const cancelSubscriptionInstruction = createInstructionHandler<CancelSubscriptionParams, CancelSubscriptionError>({
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
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
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

export type ResumeSubscriptionError = SubscriptionsStreamProgramError;

export const resumeSubscriptionInstruction = createInstructionHandler<ResumeSubscriptionParams, ResumeSubscriptionError>({
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
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
});

export interface RevokeSubscriptionAuthorityParams {
  user: string;
  userAta: string;
  tokenMint: string;
  tokenProgram: string;
  subscriptionAuthority?: string;
}

export type RevokeSubscriptionAuthorityError = SubscriptionsStreamProgramError;

export const revokeSubscriptionAuthorityInstruction = createInstructionHandler<RevokeSubscriptionAuthorityParams, RevokeSubscriptionAuthorityError>({
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
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
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

export type RevokeAbandonedDelegationError = SubscriptionsStreamProgramError;

export const revokeAbandonedDelegationInstruction = createInstructionHandler<RevokeAbandonedDelegationParams, RevokeAbandonedDelegationError>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [15],
  args: [],
  accounts: [
    { name: 'payer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'delegationAccount', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'subscriptionAuthority', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'SubscriptionAuthority' }, { type: 'argRef', argName: 'user', argType: 'pubkey' }, { type: 'argRef', argName: 'tokenMint', argType: 'pubkey' }] } },
  ],
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
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

export type RevokeAbandonedSubscriptionError = SubscriptionsStreamProgramError;

export const revokeAbandonedSubscriptionInstruction = createInstructionHandler<RevokeAbandonedSubscriptionParams, RevokeAbandonedSubscriptionError>({
  programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
  discriminator: [16],
  args: [],
  accounts: [
    { name: 'payer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'subscriptionAccount', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'subscriptionAuthority', isSigner: false, isWritable: false, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'SubscriptionAuthority' }, { type: 'argRef', argName: 'user', argType: 'pubkey' }, { type: 'argRef', argName: 'tokenMint', argType: 'pubkey' }] } },
    { name: 'planPda', isSigner: false, isWritable: false, category: 'userProvided' },
  ],
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
});

export interface CancelSubscriptionNowParams {
  cancelSubscriptionNowData: CancelSubscriptionNowData;
  subscriber: string;
  merchant: string;
  planPda: string;
  subscriptionPda?: string;
  eventAuthority?: string;
}

export type CancelSubscriptionNowError = SubscriptionsStreamProgramError;

export const cancelSubscriptionNowInstruction = createInstructionHandler<CancelSubscriptionNowParams, CancelSubscriptionNowError>({
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
  errors: SUBSCRIPTIONS_STREAM_PROGRAM_ERRORS,
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

/** Stack definition for SubscriptionsStream with 5 entities */
export const SUBSCRIPTIONS_STREAM_STACK_CORE = {
  name: 'subscriptions-stream',
  endpoints: {
    ws: '', // TODO: Set after first deployment or pass useArete(..., { url })
    http: '', // TODO: Set after first deployment or pass useArete(..., { httpUrl })
  },
  views: {
    SubscriptionPlan: {
      state: stateView<SubscriptionPlan, { address: string }>('SubscriptionPlan/state', ['address']),
      list: listView<SubscriptionPlan>('SubscriptionPlan/list'),
      recent_activity: listView<SubscriptionPlan>('SubscriptionPlan/recent_activity'),
      most_subscribed: listView<SubscriptionPlan>('SubscriptionPlan/most_subscribed'),
      highest_volume: listView<SubscriptionPlan>('SubscriptionPlan/highest_volume'),
    },
    SubscriptionAuthority: {
      state: stateView<SubscriptionAuthority, { address: string }>('SubscriptionAuthority/state', ['address']),
      list: listView<SubscriptionAuthority>('SubscriptionAuthority/list'),
      recent_activity: listView<SubscriptionAuthority>('SubscriptionAuthority/recent_activity'),
    },
    SubscriptionInstance: {
      state: stateView<SubscriptionInstance, { address: string }>('SubscriptionInstance/state', ['address']),
      list: listView<SubscriptionInstance>('SubscriptionInstance/list'),
      recent_activity: listView<SubscriptionInstance>('SubscriptionInstance/recent_activity'),
      expiring: listView<SubscriptionInstance>('SubscriptionInstance/expiring'),
    },
    FixedDelegation: {
      state: stateView<FixedDelegation, { address: string }>('FixedDelegation/state', ['address']),
      list: listView<FixedDelegation>('FixedDelegation/list'),
      expiring: listView<FixedDelegation>('FixedDelegation/expiring'),
      recent_transfers: listView<FixedDelegation>('FixedDelegation/recent_transfers'),
    },
    RecurringDelegation: {
      state: stateView<RecurringDelegation, { address: string }>('RecurringDelegation/state', ['address']),
      list: listView<RecurringDelegation>('RecurringDelegation/list'),
      expiring: listView<RecurringDelegation>('RecurringDelegation/expiring'),
      recent_transfers: listView<RecurringDelegation>('RecurringDelegation/recent_transfers'),
    },
  },
  schemas: {
    AccountDiscriminator: AccountDiscriminatorSchema,
    FixedDelegationAccount: FixedDelegationAccountSchema,
    FixedDelegationActivity: FixedDelegationActivitySchema,
    FixedDelegationCompleted: FixedDelegationCompletedSchema,
    FixedDelegationId: FixedDelegationIdSchema,
    FixedDelegation: FixedDelegationSchema,
    FixedDelegationState: FixedDelegationStateSchema,
    Header: HeaderSchema,
    PlanData: PlanDataSchema,
    Plan: PlanSchema,
    PlanStatus: PlanStatusSchema,
    PlanTerms: PlanTermsSchema,
    RecurringDelegationAccount: RecurringDelegationAccountSchema,
    RecurringDelegationActivity: RecurringDelegationActivitySchema,
    RecurringDelegationCompleted: RecurringDelegationCompletedSchema,
    RecurringDelegationId: RecurringDelegationIdSchema,
    RecurringDelegation: RecurringDelegationSchema,
    RecurringDelegationState: RecurringDelegationStateSchema,
    SubscriptionAuthorityAccount: SubscriptionAuthorityAccountSchema,
    SubscriptionAuthorityCompleted: SubscriptionAuthorityCompletedSchema,
    SubscriptionAuthorityId: SubscriptionAuthorityIdSchema,
    SubscriptionAuthorityMetrics: SubscriptionAuthorityMetricsSchema,
    SubscriptionAuthority: SubscriptionAuthoritySchema,
    SubscriptionAuthorityState: SubscriptionAuthorityStateSchema,
    SubscriptionDelegation: SubscriptionDelegationSchema,
    SubscriptionInstanceActivity: SubscriptionInstanceActivitySchema,
    SubscriptionInstanceCompleted: SubscriptionInstanceCompletedSchema,
    SubscriptionInstanceId: SubscriptionInstanceIdSchema,
    SubscriptionInstance: SubscriptionInstanceSchema,
    SubscriptionInstanceState: SubscriptionInstanceStateSchema,
    SubscriptionPlanCompleted: SubscriptionPlanCompletedSchema,
    SubscriptionPlanId: SubscriptionPlanIdSchema,
    SubscriptionPlanMetrics: SubscriptionPlanMetricsSchema,
    SubscriptionPlan: SubscriptionPlanSchema,
    SubscriptionPlanState: SubscriptionPlanStateSchema,
    SubscriptionsFixedDelegation: SubscriptionsFixedDelegationSchema,
    SubscriptionsHeader: SubscriptionsHeaderSchema,
    SubscriptionsPlanData: SubscriptionsPlanDataSchema,
    SubscriptionsPlan: SubscriptionsPlanSchema,
    SubscriptionsPlanTerms: SubscriptionsPlanTermsSchema,
    SubscriptionsRecurringDelegation: SubscriptionsRecurringDelegationSchema,
    SubscriptionsSubscriptionAuthority: SubscriptionsSubscriptionAuthoritySchema,
    SubscriptionsSubscriptionDelegation: SubscriptionsSubscriptionDelegationSchema,
    TokenMetadata: TokenMetadataSchema,
  },
  patchSchemas: {
    SubscriptionPlan: SubscriptionPlanPatchSchema,
    SubscriptionAuthority: SubscriptionAuthorityPatchSchema,
    SubscriptionInstance: SubscriptionInstancePatchSchema,
    FixedDelegation: FixedDelegationPatchSchema,
    RecurringDelegation: RecurringDelegationPatchSchema,
  },
  programs: {
    subscriptions: {
      name: 'subscriptions',
      programId: 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44',
      sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:e6ec5a8aa5b6e300143af55295f28ab56076f97ce722fac0f1d7df980eea7958',
      programSpecHash: 'arete:h1:program-spec:sha256:a10a30f17be97429474e990fdc24b87edf51e4c5fe0021b0651d434d11f3e698',
      idlContentHash: 'arete:h1:idl-content:sha256:6b1a0054e474098d8e646ca0238f197ea5d31fa93ca79c4dd466c6edf4fef554',
      normalizedIdlHash: 'arete:h1:idl-normalized:sha256:c12d4e2b8c21cd79a97e3e9dd698cc60dd1a511c1ef4e31dbc502a8fd89485c4',
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
        fixedDelegation: programAccountRead<SubscriptionsFixedDelegation>({ account: 'fixedDelegation', schema: SubscriptionsFixedDelegationSchema }),
        plan: programAccountRead<SubscriptionsPlan>({ account: 'plan', schema: SubscriptionsPlanSchema }),
        recurringDelegation: programAccountRead<SubscriptionsRecurringDelegation>({ account: 'recurringDelegation', schema: SubscriptionsRecurringDelegationSchema }),
        subscriptionAuthority: programAccountRead<SubscriptionsSubscriptionAuthority>({ account: 'subscriptionAuthority', schema: SubscriptionsSubscriptionAuthoritySchema }),
        subscriptionDelegation: programAccountRead<SubscriptionsSubscriptionDelegation>({ account: 'subscriptionDelegation', schema: SubscriptionsSubscriptionDelegationSchema }),
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
    },
  },
  programReads: {
    subscriptions: {
      release: { programReleaseHash: "arete:h1:program-release:sha256:73c10c463d6d8f3734e2869959e20a5e27986bf68c7a16d415eb5f6aabe2a6de", programSpecHash: "arete:h1:program-spec:sha256:a10a30f17be97429474e990fdc24b87edf51e4c5fe0021b0651d434d11f3e698" },
      transport: { kind: 'local-http', endpointSource: 'connect-http-url' },
    },
  },
  addresses: {
    eventAuthority: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('event_authority')),
    fixedDelegation: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('delegation'), arg('subscriptionAuthority', 'publicKey'), arg('delegator', 'publicKey'), arg('delegatee', 'publicKey'), arg('nonce', 'u64')),
    plan: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('plan'), arg('owner', 'publicKey'), arg('planId', 'u64')),
    recurringDelegation: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('delegation'), arg('subscriptionAuthority', 'publicKey'), arg('delegator', 'publicKey'), arg('delegatee', 'publicKey'), arg('nonce', 'u64')),
    subscriptionAuthority: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('SubscriptionAuthority'), arg('user', 'publicKey'), arg('tokenMint', 'publicKey')),
    subscriptionDelegation: pda('De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44', literal('subscription'), arg('planPda', 'publicKey'), arg('subscriber', 'publicKey')),
  },
} as const;

/** Type alias for the core stack */
export type SubscriptionsStreamCoreStack = typeof SUBSCRIPTIONS_STREAM_STACK_CORE;

/** Entity types in this stack */
export type SubscriptionsStreamEntity = SubscriptionPlan | SubscriptionAuthority | SubscriptionInstance | FixedDelegation | RecurringDelegation;

/** Default export for convenience */
export default SUBSCRIPTIONS_STREAM_STACK_CORE;