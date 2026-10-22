import type { ProgramSdkDefinition, StackDefinition } from '../types';

/**
 * An integer argument or parameter wider than a JavaScript number can hold
 * exactly (`u64`, `u128`, `i64`, unix timestamps, ...). Send a decimal string
 * or a bigint; the hosted API returns decimal strings.
 */
export type ProtocolInteger = string | bigint;

/** The class of API key a protocol client authenticates with. */
export type ProtocolKeyClass = 'publishable' | 'secret';

/** Which machine a step acts on: a new instance, an instance, or a participant. */
export type ProtocolStepScope = 'create' | 'instance' | 'participant';

/** `public` steps accept publishable keys; `secret` steps need a secret key. */
export type ProtocolStepAccess = 'public' | 'secret';

/** The machine an event or transition belongs to. */
export type ProtocolMachineKind = 'instance' | 'participant';

/** Type-level description of one client-callable step. */
export interface ProtocolStepTypes {
  readonly scope: ProtocolStepScope;
  readonly access: ProtocolStepAccess;
  readonly args: object;
  readonly guardCode: string;
}

/**
 * Type-level description of a protocol, emitted by `a4 install protocol` as
 * `<Name>ProtocolTypes`. Runtime code never reads it.
 */
export interface ProtocolTypeMap {
  readonly params: object;
  readonly instanceState: string;
  readonly participantState: string;
  readonly instanceMetadata: object;
  readonly participantMetadata: object;
  readonly steps: { readonly [step: string]: ProtocolStepTypes };
  readonly events: { readonly [event: string]: { readonly machine: ProtocolMachineKind } };
}

/** Runtime description of one client-callable step. */
export interface ProtocolStepDefinition<
  TScope extends ProtocolStepScope = ProtocolStepScope,
  TAccess extends ProtocolStepAccess = ProtocolStepAccess,
> {
  readonly scope: TScope;
  readonly access: TAccess;
  /** Guard codes the step can reject with, in evaluation order. */
  readonly guards: readonly string[];
  /** Argument names. */
  readonly args: readonly string[];
}

/** Who may write a metadata field, and whether instances can be filtered by it. */
export interface ProtocolMetadataFieldDefinition {
  readonly writer: 'creator' | 'participant';
  readonly filterable: boolean;
}

/** Where a protocol deployment is served. Never carries credentials. */
export interface ProtocolDeploymentDescriptor {
  readonly deploymentId: string;
  readonly access: 'private' | 'public';
  /** Base URL of the hosted API, e.g. `https://api.arete.run/api`. */
  readonly apiUrl: string;
  /** Where session tokens are minted, e.g. `https://api.arete.run/ws/sessions`. */
  readonly sessionEndpoint: string;
  /**
   * `'dev'` when `a4 dev` generated the client for the deployment in a dev
   * environment (a fork of mainnet). Absent for a published deployment.
   */
  readonly environment?: 'dev';
}

/** The protocol's own generated stack view that mirrors an instance live. */
export interface ProtocolLiveStackDescriptor {
  /** Key of the stack in the definition's `stacks`. */
  readonly stack: string;
  /** `<Entity>/<mode>` of the state view, e.g. `TokenRaiseInstance/state`. */
  readonly view: string;
  /** Instance account whose address keys the view row. */
  readonly keyAccount: string;
}

type MetadataFieldDefinitions<TMetadata> = {
  readonly [K in keyof TMetadata]?: ProtocolMetadataFieldDefinition;
};

/**
 * A generated protocol module's default export. Created by
 * `a4 install protocol <ref> --ts`; pass it to {@link createProtocolClient}
 * or to `createSession({ protocols })`.
 */
export interface ProtocolDefinition<
  TTypes extends ProtocolTypeMap = ProtocolTypeMap,
  TStacks extends Record<string, StackDefinition> = Record<string, StackDefinition>,
  TPrograms extends Record<string, ProgramSdkDefinition> = Record<string, ProgramSdkDefinition>,
> {
  readonly kind: 'protocol';
  /** The spec name, e.g. `tokenRaise`. */
  readonly name: string;
  readonly version: string;
  /** Registry package name, e.g. `token-raise`. */
  readonly package: string;
  readonly packageReleaseHash: string;
  readonly protocolSpecHash: string;
  readonly protocolReleaseHash: string;
  readonly deployment: ProtocolDeploymentDescriptor;
  readonly instanceVisibility: 'shared' | 'account';
  /** Client-callable steps only. */
  readonly steps: {
    readonly [K in keyof TTypes['steps']]: ProtocolStepDefinition<
      TTypes['steps'][K]['scope'],
      TTypes['steps'][K]['access']
    >;
  };
  readonly events: {
    readonly [K in keyof TTypes['events']]: { readonly machine: TTypes['events'][K]['machine'] };
  };
  readonly metadata: {
    readonly instance?: MetadataFieldDefinitions<TTypes['instanceMetadata']>;
    readonly participant?: MetadataFieldDefinitions<TTypes['participantMetadata']>;
  };
  readonly liveStack?: ProtocolLiveStackDescriptor;
  /** Stacks the protocol reads, hoisted so a session connects each once. */
  readonly stacks: TStacks;
  /** Programs the protocol uses, hoisted so a session serves each once. */
  readonly programs: TPrograms;
}

/** Any protocol definition, whatever its generated types. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type AnyProtocolDefinition = ProtocolDefinition<any, any, any>;

/** The type map of a protocol definition. */
export type ProtocolTypesOf<TDef> = TDef extends ProtocolDefinition<infer TTypes, any, any>
  ? TTypes
  : ProtocolTypeMap;
/** The hoisted stacks of a protocol definition. */
export type ProtocolStacksOf<TDef> = TDef extends ProtocolDefinition<any, infer TStacks, any>
  ? TStacks
  : Record<string, StackDefinition>;
/** The hoisted programs of a protocol definition. */
export type ProtocolProgramsOf<TDef> = TDef extends ProtocolDefinition<any, any, infer TPrograms>
  ? TPrograms
  : Record<string, ProgramSdkDefinition>;

// ---------------------------------------------------------------------------
// Rejections
// ---------------------------------------------------------------------------

/** Access refusals a prepare can return instead of a guard rejection. */
export const PROTOCOL_ACCESS_REJECTION_CODES = [
  'step_requires_secret_key',
  'step_not_in_session_scope',
  'step_not_client_callable',
] as const;
export type ProtocolAccessRejectionCode = (typeof PROTOCOL_ACCESS_REJECTION_CODES)[number];

/**
 * Lifecycle rejections a prepare can return instead of a guard rejection.
 * `instance_terminal`: the instance is in a terminal state. `participant_terminal`:
 * the preparing wallet's participant is in a terminal state. Both carry
 * `detail.state`; `instance_terminal` is checked first. `allowedSteps` reports
 * the same codes as `reason`.
 */
export const PROTOCOL_STATE_REJECTION_CODES = ['instance_terminal', 'participant_terminal'] as const;
export type ProtocolStateRejectionCode = (typeof PROTOCOL_STATE_REJECTION_CODES)[number];

/** A step that did not prepare: a guard failed, access was refused, or the lifecycle is over. */
export interface ProtocolRejection<TCode extends string = string> {
  /** The guard code, an access refusal code, or a state rejection code. */
  readonly rejected: TCode;
  readonly step: string;
  /** Source of the guard expression that failed. */
  readonly guard?: string;
  /** The evaluated operands of the guard. */
  readonly detail?: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// Execution intents
// ---------------------------------------------------------------------------

export const EXECUTION_INTENT_SCHEMA_V1 = 'arete.execution-intent/v1' as const;
export type ProtocolTransactionFormat = 'v1' | 'v0';
export type ExecutionIntentSignerClass =
  | 'wallet'
  | 'ephemeral'
  | 'attestation'
  | 'keeper'
  | 'squadsProposer';

export interface IntentDeploymentV1 {
  deploymentId: string;
  revision: number;
  releaseHash: string;
}

export interface IntentFeesV1 {
  asset: { kind: 'native' } | { kind: 'spl'; mint: string };
  gross: string;
  total: string;
  creator: string;
  platform: string;
  placement: 'before' | 'after';
  platformDestination: string;
  creatorDestination: string | null;
  /** Set when a cap or an override changed the platform leg. */
  feeAdjustment?: { kind: 'cap' | 'override'; id: string };
}

export interface IntentSignerV1 {
  class: ExecutionIntentSignerClass;
  pubkey: string;
  name: string | null;
}

export interface IntentTransactionV1 {
  index: number;
  messageSha256: string;
  signers: string[];
}

/** What a prepare committed to: the transactions, signers, fees and expiry. */
export interface ExecutionIntentV1 {
  schema: typeof EXECUTION_INTENT_SCHEMA_V1;
  executionId: string;
  deployment: IntentDeploymentV1;
  step: string;
  instanceId: string | null;
  participantId: string | null;
  wallet: string;
  role: string | null;
  callerAccount: string;
  sessionKeyClass: ProtocolKeyClass;
  transactionFormat: ProtocolTransactionFormat;
  inputSha256: string;
  resourceHashes: string[];
  providerSnapshotHash: string;
  fees: IntentFeesV1 | null;
  signers: IntentSignerV1[];
  transactions: IntentTransactionV1[];
  blockhash: string;
  lastValidBlockHeight: string;
  expiresAt: number;
  createdAt: number;
}

/** One partially signed transaction for the wallet to sign. */
export interface ProtocolPreparedTransaction {
  readonly index: number;
  readonly transactionBase64: string;
}

export type ProtocolPrepareResult<TCode extends string = string> =
  | {
      readonly ok: true;
      readonly intent: ExecutionIntentV1;
      readonly transactions: ProtocolPreparedTransaction[];
    }
  | {
      readonly ok: false;
      readonly rejection: ProtocolRejection<
        TCode | ProtocolAccessRejectionCode | ProtocolStateRejectionCode
      >;
    };

export type ProtocolIntentStatus =
  | 'prepared'
  | 'submitting'
  | 'partially_completed'
  | 'completed'
  | 'chain_failed'
  | 'submitted_unknown'
  | 'expired'
  | 'rejected';

export interface ProtocolIntentTransactionState {
  readonly index: number;
  readonly signature: string | null;
  readonly state: string;
  readonly error?: string | null;
}

export interface ProtocolIntentState {
  readonly executionId: string;
  readonly state: ProtocolIntentStatus;
  readonly transactions: ProtocolIntentTransactionState[];
  /** Where the signed receipt of a completed intent can be fetched, when the API issues one. */
  readonly receiptUrl?: string;
}

// ---------------------------------------------------------------------------
// Instances and participants
// ---------------------------------------------------------------------------

/** One typed argument of a step, as the protocol spec declares it. */
export interface ProtocolStepArgDescriptor {
  readonly name: string;
  /**
   * The field schema's JSON type: `string`, `integer`, `boolean`, `array` or
   * `object`. Solana amounts and keys are strings with a `format`.
   */
  readonly type: string;
  /** The string format, e.g. `u64`, `i64`, `pubkey` or `unixTimestamp`. */
  readonly format?: string;
  readonly optional: boolean;
  readonly description?: string;
  readonly enum?: readonly string[];
  readonly minimum?: number;
  readonly maximum?: number;
  readonly maxLength?: number;
  /** The element schema of an `array` arg. */
  readonly items?: Omit<ProtocolStepArgDescriptor, 'name' | 'optional'>;
  readonly maxItems?: number;
  /** The fields of an `object` arg. */
  readonly properties?: Readonly<Record<string, Omit<ProtocolStepArgDescriptor, 'name' | 'optional'>>>;
  readonly required?: readonly string[];
}

/** Whether a step can be called now, and why not. */
export interface ProtocolStepAvailability<
  TStep extends string = string,
  TReason extends string = string,
> {
  readonly step: TStep;
  readonly allowed: boolean;
  /** Guard code or access code when not allowed. */
  readonly reason?: TReason;
  readonly detail?: Record<string, unknown>;
  /**
   * Guards that were not evaluated because they read the step's arguments
   * (or, for create steps, the instance or participant that does not exist
   * yet). `allowed` holds only if they pass too.
   */
  readonly argsDependent?: string[];
  /** Which machine the step acts on. */
  readonly scope?: ProtocolStepScope;
  /** The step's typed arguments. */
  readonly args?: ProtocolStepArgDescriptor[];
}

type GuardCodesOf<TTypes extends ProtocolTypeMap> = {
  [K in keyof TTypes['steps']]: TTypes['steps'][K]['guardCode'];
}[keyof TTypes['steps']];

/** Step availability typed by a protocol's step names and guard codes. */
export type ProtocolStepAvailabilityFor<TTypes extends ProtocolTypeMap> = ProtocolStepAvailability<
  Extract<keyof TTypes['steps'], string> | (string & {}),
  GuardCodesOf<TTypes> | ProtocolAccessRejectionCode | ProtocolStateRejectionCode | (string & {})
>;

export interface ProtocolInstanceSummary<TState extends string = string> {
  readonly id: string;
  readonly deploymentId: string;
  readonly revision: number;
  readonly state: TState;
  readonly terminal: boolean;
  readonly visibility: 'shared' | 'account';
  readonly accounts: Record<string, string>;
  readonly roles: Record<string, string>;
  readonly createdByWallet: string;
  readonly createdAt: string;
  readonly updatedAt: string;
  readonly closedAt: string | null;
}

export interface ProtocolInstancePage<TState extends string = string> {
  readonly instances: ProtocolInstanceSummary<TState>[];
  readonly nextCursor: string | null;
}

export interface ProtocolInstanceDetail<TTypes extends ProtocolTypeMap = ProtocolTypeMap>
  extends ProtocolInstanceSummary<TTypes['instanceState']> {
  readonly releaseHash: string;
  readonly params: TTypes['params'];
  readonly lastErrorCode: string | null;
  readonly derived: unknown;
  readonly metadata: Partial<TTypes['instanceMetadata']>;
  readonly metadataVersion: number | null;
  readonly pendingTimers: unknown[];
  readonly allowedSteps: ProtocolStepAvailabilityFor<TTypes>[];
}

export interface ProtocolParticipantDetail<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  readonly instanceId: string;
  readonly wallet: string;
  readonly participantId: string | null;
  readonly state: TTypes['participantState'] | null;
  readonly metadata: Partial<TTypes['participantMetadata']>;
  readonly history: unknown[];
  readonly allowedSteps: ProtocolStepAvailabilityFor<TTypes>[];
}

/**
 * How a wallet relates to an instance: `participant`, `creator`, or
 * `role:<name>` for each instance role the wallet holds.
 */
export type ProtocolWalletRelation = 'participant' | 'creator' | `role:${string}`;

/** One instance a wallet relates to, with the steps it can call there. */
export interface ProtocolWalletInstanceActions<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  readonly instanceId: string;
  readonly state: TTypes['instanceState'];
  readonly relation: ProtocolWalletRelation[];
  /** The wallet's participant state; `null` when it is not a participant. */
  readonly participantState: TTypes['participantState'] | null;
  /** Instance and participant steps, evaluated for the wallet. */
  readonly steps: ProtocolStepAvailabilityFor<TTypes>[];
}

/** Which steps a wallet can call across a deployment, one page of instances at a time. */
export interface ProtocolWalletActions<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  readonly deploymentId: string;
  readonly wallet: string;
  /**
   * Create steps, evaluated without an instance. Guards that read the
   * arguments or the instance-to-be are listed in `argsDependent`.
   */
  readonly createSteps: ProtocolStepAvailabilityFor<TTypes>[];
  readonly instances: ProtocolWalletInstanceActions<TTypes>[];
  /** Pass as `cursor` for the next page of instances; `null` on the last page. */
  readonly nextCursor: string | null;
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/** The JSON body of an emitted event, as delivered over webhooks. */
export interface ProtocolEventPayloadV1<TEvent extends string = string, TState extends string = string> {
  /** Unique emission id. */
  id: string;
  /** The spec's event name, such as `raise.succeeded`. */
  event: TEvent;
  deploymentId: string;
  instanceId: string;
  /** Set for participant events, `null` for instance events. */
  participantId: string | null;
  /** The state of the event's machine when it was emitted. */
  state: TState;
  /** RFC 3339 UTC time of the emission. */
  occurredAt: string;
}

/** A webhook payload typed by a protocol's event names and machine states. */
export type ProtocolWebhookPayload<TTypes extends ProtocolTypeMap> = {
  [E in Extract<keyof TTypes['events'], string>]: ProtocolEventPayloadV1<
    E,
    TTypes['events'][E]['machine'] extends 'participant'
      ? TTypes['participantState']
      : TTypes['instanceState']
  >;
}[Extract<keyof TTypes['events'], string>];

/** Data of an `emit` event on an instance's event stream. */
export interface ProtocolEmitEventData<TEvent extends string = string> {
  /** The emission's id (a server sequence number). */
  readonly id: number;
  readonly event: TEvent;
  readonly reaction: string;
  readonly instanceId: string;
  readonly participantId: string | null;
  /** The triggering transition; null for emissions from a confirmed step. */
  readonly transitionId: number | null;
  readonly at: string;
}

/** System event: a Squads proposal waits for the integrator's proposer. */
export const SQUADS_PROPOSER_MISSING_EVENT = 'arete.squads.proposerMissing' as const;

/** Why the proposer cannot propose to a multisig yet. */
export type ProtocolSquadsProposerProblem = 'not_member' | 'permissions_invalid';

/**
 * Data of `arete.squads.proposerMissing`: an admin step's proposal is parked
 * because the calling account's proposer is not an Initiate-only member of
 * the multisig. It is retried with backoff for up to 24 hours; add the
 * proposer with `squads.addProposer` to unblock it.
 */
export interface ProtocolSquadsProposerMissingData {
  readonly id: number;
  readonly deploymentId: string;
  readonly instanceId: string;
  readonly outboxId: number;
  readonly step: string;
  readonly multisig: string;
  readonly proposer: string;
  readonly problem: ProtocolSquadsProposerProblem;
  readonly at: string;
}

/** One event from an instance's event stream. */
export type ProtocolStreamEvent<TTypes extends ProtocolTypeMap = ProtocolTypeMap> =
  | {
      readonly type: 'emit';
      /** Resume cursor. */
      readonly id: string;
      readonly event: Extract<keyof TTypes['events'], string>;
      readonly data: ProtocolEmitEventData<Extract<keyof TTypes['events'], string>>;
    }
  | {
      readonly type: typeof SQUADS_PROPOSER_MISSING_EVENT;
      /** Resume cursor. */
      readonly id: string;
      readonly data: ProtocolSquadsProposerMissingData;
    }
  | {
      readonly type: 'transition' | 'participant' | 'observation';
      /** Resume cursor. */
      readonly id: string;
      readonly data: Record<string, unknown>;
    };

// ---------------------------------------------------------------------------
// Squads onboarding
// ---------------------------------------------------------------------------

/** The calling account's Squads proposer for a deployment. */
export interface ProtocolSquadsProposer {
  readonly pubkey: string;
  /** Always `initiate`: the proposer may propose and nothing else. */
  readonly permissions: 'initiate';
  /** The root the proposer derives from; `null` for an environment key. */
  readonly rootKeyId: string | null;
}

export interface ProtocolSquadsAddProposerInput {
  /** The team's Squads v4 multisig. */
  readonly multisig: string;
  /** A member with the Initiate permission who signs and pays. */
  readonly member: string;
  readonly transactionFormat?: ProtocolTransactionFormat;
  readonly signal?: AbortSignal;
}

/**
 * An unsigned transaction creating a Squads config proposal that adds the
 * proposer with Initiate only. `member` signs and sends it; the team then
 * approves and executes the proposal in Squads.
 */
export interface ProtocolSquadsAddProposerResult {
  readonly transactionBase64: string;
  readonly transactionFormat: ProtocolTransactionFormat;
  readonly transactionIndex: number;
  readonly proposal: string;
  readonly configTransaction: string;
  readonly multisig: string;
  readonly member: string;
  readonly proposer: string;
  readonly lastValidBlockHeight: number;
}

/** Whether the calling account's proposer can propose to a multisig. */
export interface ProtocolSquadsStatus {
  /** A member with exactly the Initiate permission. */
  readonly ready: boolean;
  readonly member: boolean;
  readonly mask: number | null;
  readonly threshold: number;
  readonly multisig: string;
  readonly proposer: string;
  readonly problem?: ProtocolSquadsProposerProblem;
}
