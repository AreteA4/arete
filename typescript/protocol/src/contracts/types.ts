/**
 * TypeScript models of the Arete Protocols public contracts.
 *
 * These mirror the Rust models in `arete-hash/src/protocol/` field for
 * field (camelCase JSON). Optional fields are omitted rather than `null`,
 * except where the contract says a field is present and nullable.
 */

import type {
  CompilerHash,
  ProgramReleaseHash,
  ProgramSpecHash,
  ProtocolHandlerBundleHash,
  ProtocolReleaseHash,
  ProtocolResourceHash,
  ProtocolSpecHash,
  ProtocolStackSourceHash,
  ProviderSnapshotHash,
  SdkExtensionHash,
  StackManifestHash,
} from "@usearete/hash";

export const PROTOCOL_SPEC_SCHEMA_V1 = "arete.protocol-spec/v1";
export const PROTOCOL_HANDLER_ABI_V1 = "arete.protocol-handler/v1";
export const PROTOCOL_CTX_V1 = "arete.protocol-ctx/v1";
export const PROTOCOL_STACK_SOURCE_SCHEMA_V1 = "arete.protocol-stack-source/v1";
export const PROTOCOL_RELEASE_SCHEMA_V1 = "arete.protocol-release/v1";
export const PROTOCOL_RELEASE_REVIEW_SCHEMA_V1 = "arete.protocol-release-review/v1";
export const PROTOCOL_BUILD_RECEIPT_SCHEMA_V1 = "arete.protocol-build-receipt/v1";
export const PROTOCOL_RESOURCE_SCHEMA_V1 = "arete.protocol-resource/v1";
export const STACK_SOURCE_SCHEMA_V1 = "arete.stack-source/v1";
export const PROVIDER_SNAPSHOT_SCHEMA_V1 = "arete.provider-snapshot/v1";
export const EXECUTION_INTENT_SCHEMA_V1 = "arete.execution-intent/v1";
export const PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1 = "arete.protocol-execution-receipt/v1";
export const PROTOCOL_RECEIPT_KEYS_SCHEMA_V1 = "arete.protocol-receipt-keys/v1";

// ---------------------------------------------------------------------------
// Spec
// ---------------------------------------------------------------------------

export interface ProtocolSpecV1 {
  schema: typeof PROTOCOL_SPEC_SCHEMA_V1;
  name: string;
  version: string;
  description?: string;
  handlerAbi?: typeof PROTOCOL_HANDLER_ABI_V1;
  ctxVersion?: typeof PROTOCOL_CTX_V1;
  programs: Record<string, ProtocolProgramV1>;
  stacks: Record<string, ProtocolStackDependencyV1>;
  params: ObjectSchemaV1;
  roles: Record<string, RoleV1>;
  resources: Record<string, ResourceDeclV1>;
  instanceVisibility: InstanceVisibility;
  reads: Record<string, ReadV1>;
  machines: MachinesV1;
  steps: Record<string, StepV1>;
  reactions: ReactionV1[];
  events: Record<string, EventV1>;
  metadata: MetadataV1;
  stack: StackSurfaceV1;
}

export const BUILTIN_PROGRAMS = [
  "system",
  "splToken",
  "associatedToken",
  "computeBudget",
  "memo",
] as const;
export type BuiltinProgram = (typeof BUILTIN_PROGRAMS)[number];

export const BUILTIN_PROGRAM_IDS: Readonly<Record<BuiltinProgram, string>> = {
  system: "11111111111111111111111111111111",
  splToken: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
  associatedToken: "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
  computeBudget: "ComputeBudget111111111111111111111111111111",
  memo: "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr",
};

export type ProtocolProgramV1 =
  | {
      kind: "programSpec";
      programId: string;
      programSpecHash: ProgramSpecHash;
      programReleaseHash?: ProgramReleaseHash;
    }
  | { kind: "builtin"; builtin: BuiltinProgram };

/** The program id a declared program resolves to. */
export function programIdOf(program: ProtocolProgramV1): string {
  return program.kind === "programSpec" ? program.programId : BUILTIN_PROGRAM_IDS[program.builtin];
}

export interface ProtocolStackDependencyV1 {
  stackManifestHash: StackManifestHash;
  package?: string;
}

/** A restricted JSON Schema subset for args, params, resources and metadata. */
export interface ObjectSchemaV1 {
  type: "object";
  properties: Record<string, FieldSchemaV1>;
  required: string[];
  additionalProperties: false;
}

export type FieldType = "string" | "boolean" | "integer" | "array" | "object";

export const STRING_FORMATS = [
  "pubkey",
  "u8",
  "u16",
  "u32",
  "u64",
  "u128",
  "i64",
  "unixTimestamp",
  "uri",
] as const;
export type StringFormat = (typeof STRING_FORMATS)[number];

export interface FieldSchemaV1 {
  type: FieldType;
  format?: StringFormat;
  enum?: string[];
  maxLength?: number;
  minimum?: number;
  maximum?: number;
  items?: FieldSchemaV1;
  maxItems?: number;
  properties?: Record<string, FieldSchemaV1>;
  required?: string[];
  additionalProperties?: boolean;
  description?: string;
}

export interface RoleV1 {
  description?: string;
}

export interface ResourceDeclV1 {
  schema: FieldSchemaV1;
  description?: string;
}

export type InstanceVisibility = "shared" | "account";

/** A declared chain read. Guards, derivations and templates see its result as `reads.<name>`. */
export type ReadV1 =
  | { kind: "account"; program: string; account: string; address: string; optional: boolean }
  | { kind: "tokenAccount"; address: string; optional: boolean }
  | { kind: "lamports"; address: string };

export interface MachinesV1 {
  instance: MachineV1;
  participant?: MachineV1;
}

export type MachineKind = "instance" | "participant";

export interface MachineV1 {
  initial: string;
  states: Record<string, StateV1>;
  derive: DeriveRuleV1[];
  reads: string[];
}

export type StateKind = "recorded" | "derived";

export interface StateV1 {
  kind: StateKind;
  terminal: boolean;
  description?: string;
}

export interface DeriveRuleV1 {
  from: string[];
  to: string;
  when: string;
  at?: string;
}

export type StepScope = "create" | "instance" | "participant";

export type AuthorityV1 =
  | { kind: "creator" }
  | { kind: "participant" }
  | { kind: "role"; role: string }
  | { kind: "keeper" }
  | { kind: "squadsProposer"; multisig: string };

export const SIGNER_CLASSES = [
  "wallet",
  "ephemeral",
  "attestation",
  "keeper",
  "squadsProposer",
] as const;
export type SignerClass = (typeof SIGNER_CLASSES)[number];

export function signerClassOf(authority: AuthorityV1): SignerClass {
  switch (authority.kind) {
    case "creator":
    case "participant":
    case "role":
      return "wallet";
    case "keeper":
      return "keeper";
    case "squadsProposer":
      return "squadsProposer";
  }
}

/** Wallet steps are the only steps an app can prepare. */
export function isClientCallable(authority: AuthorityV1): boolean {
  return signerClassOf(authority) === "wallet";
}

export type StepAccess = "public" | "secret";

export interface StepV1 {
  scope: StepScope;
  title?: string;
  description?: string;
  authority: AuthorityV1;
  /** The maximum access. Present exactly on client-callable steps. */
  access?: StepAccess;
  args: ObjectSchemaV1;
  guards: GuardV1[];
  reads: string[];
  /** Names of fresh keypairs allocated per intent, as `ephemeral.<name>`. */
  ephemeral: string[];
  fee: FeeBasisV1;
  transitions: TransitionV1[];
  bind?: BindV1;
  glue: GlueV1;
  build: BuildV1;
}

export interface GuardV1 {
  /** The typed rejection reason. */
  code: string;
  expr: string;
  message?: string;
}

export type FeeBasisV1 =
  | { kind: "none" }
  | { kind: "input"; amountArg: string; asset: FeeAssetV1 }
  | { kind: "fixed" };

export type FeeAssetV1 = { kind: "native" } | { kind: "spl"; mint: string; sourceArg: string };

export interface TransitionV1 {
  machine: MachineKind;
  from: string[];
  to: string;
}

export interface BindV1 {
  /** Instance account name → expression over the create transaction's outputs. */
  accounts: Record<string, string>;
  /** Role → expression over args or params. Immutable for the instance. */
  roles: Record<string, string>;
}

export interface GlueV1 {
  atas: AtaGlueV1[];
  wrapSol?: WrapSolV1;
  unwrapSol: boolean;
}

export interface AtaGlueV1 {
  owner: string;
  mint: string;
}

export interface WrapSolV1 {
  amount: string;
}

export type BuildV1 =
  | { kind: "template"; instructions: TemplateInstructionV1[] }
  | { kind: "handler"; export: string; programs: string[] };

export interface TemplateInstructionV1 {
  program: string;
  instruction: string;
  args: Record<string, string>;
  accounts: Record<string, string>;
  /** Trailing accounts no IDL names, appended after the IDL accounts in order. Omitted when empty. */
  remainingAccounts?: RemainingAccountV1[];
}

/** One trailing account: `pubkey` is an account expression. Remaining accounts never sign. */
export interface RemainingAccountV1 {
  pubkey: string;
  isWritable: boolean;
  isSigner: false;
}

export interface ReactionV1 {
  id: string;
  trigger: TriggerV1;
  action: ActionV1;
}

export type TriggerV1 =
  | { kind: "at"; machine: MachineKind; in: string[]; time: string }
  | { kind: "when"; machine: MachineKind; in: string[]; condition: string }
  | { kind: "onTransition"; machine: MachineKind; from: string[]; to: string[] }
  | { kind: "onConfirmed"; step: string };

export type ActionV1 =
  | { kind: "runKeeperStep"; step: string }
  | { kind: "proposeSquads"; step: string }
  | { kind: "emit"; event: string };

export interface EventV1 {
  machine: MachineKind;
  description?: string;
}

export interface MetadataV1 {
  instance?: MetadataSchemaV1;
  participant?: MetadataSchemaV1;
}

export interface MetadataSchemaV1 {
  fields: Record<string, MetadataFieldV1>;
  maxBytes: number;
}

export interface MetadataFieldV1 {
  schema: FieldSchemaV1;
  writer: "creator" | "participant";
  filterable: boolean;
}

/** What the generated protocol stack tracks. */
export interface StackSurfaceV1 {
  instanceAccounts: Record<string, StackInstanceAccountV1>;
  instanceFields: Record<string, StackInstanceFieldV1>;
  activity: StackActivityV1[];
}

export interface StackInstanceAccountV1 {
  program: string;
  account: string;
}

export interface StackInstanceFieldV1 {
  account: string;
  field: string;
}

export interface StackActivityV1 {
  step: string;
  program: string;
  instruction: string;
  instanceAccount: string;
  walletAccount: string;
  amountArg?: string;
}

// ---------------------------------------------------------------------------
// Release side
// ---------------------------------------------------------------------------

export type StackProfile = "interim" | "full";

export interface GeneratorV1 {
  name: string;
  version: string;
}

export interface ProtocolStackSourceV1 {
  schema: typeof PROTOCOL_STACK_SOURCE_SCHEMA_V1;
  generator: GeneratorV1;
  profile: StackProfile;
  /** The generated `arete.stack-source/v1` document. */
  stackSource: { schema: typeof STACK_SOURCE_SCHEMA_V1 } & Record<string, unknown>;
}

export interface ProtocolReleaseV1 {
  schema: typeof PROTOCOL_RELEASE_SCHEMA_V1;
  protocol: string;
  version: string;
  specHash: ProtocolSpecHash;
  /** `null` for template-only protocols. */
  handlerBundleHash: ProtocolHandlerBundleHash | null;
  stackSourceHash: ProtocolStackSourceHash;
  compilerHash: CompilerHash;
  dependencies: ReleaseDependenciesV1;
}

export interface ReleaseDependenciesV1 {
  programs: ProgramDependencyV1[];
  stacks: StackDependencyV1[];
  sdkExtensions: SdkExtensionDependencyV1[];
}

export interface ProgramDependencyV1 {
  alias: string;
  programId: string;
  programSpecHash: ProgramSpecHash;
  programReleaseHash?: ProgramReleaseHash;
}

export interface StackDependencyV1 {
  alias: string;
  stackManifestHash: StackManifestHash;
}

export interface SdkExtensionDependencyV1 {
  alias: string;
  sdkExtensionHash: SdkExtensionHash;
}

/**
 * The trust tier a deployment runs at. `reviewed`: a reviewer signed the
 * release, so handlers may run. `unreviewed`: no review; only template-only
 * releases on private deployments run at this tier, so no author code runs
 * unreviewed.
 */
export type TrustTier = "reviewed" | "unreviewed";

export const TRUST_TIERS: readonly TrustTier[] = ["reviewed", "unreviewed"];

export function isTrustTier(value: unknown): value is TrustTier {
  return value === "reviewed" || value === "unreviewed";
}

export interface ProtocolReleaseReviewV1 {
  schema: typeof PROTOCOL_RELEASE_REVIEW_SCHEMA_V1;
  releaseHash: ProtocolReleaseHash;
  reviewKeyId: string;
  /** A review always attests `reviewed`; `unreviewed` is the absence of a review. */
  trustTier: "reviewed";
  /** Base64 (standard, padded) Ed25519 signature over `reviewSigningMessage`. */
  signature: string;
}

export interface ProtocolBuildReceiptV1 {
  schema: typeof PROTOCOL_BUILD_RECEIPT_SCHEMA_V1;
  sourceTreeSha256: string;
  lockfileSha256: string;
  dependencies: BuildDependencyV1[];
  compiler: BuildCompilerV1;
  outputs: BuildOutputsV1;
}

export interface BuildDependencyV1 {
  name: string;
  version: string;
  integrity: string;
}

export interface BuildCompilerV1 {
  name: string;
  version: string;
  esbuild: string;
}

export interface BuildOutputsV1 {
  specHash: ProtocolSpecHash;
  handlerBundleHash: ProtocolHandlerBundleHash | null;
  stackSourceHash: ProtocolStackSourceHash;
  releaseHash: ProtocolReleaseHash;
}

export interface ProtocolResourceV1 {
  schema: typeof PROTOCOL_RESOURCE_SCHEMA_V1;
  name: string;
  /** Lowercase hex SHA-256 of the canonical JSON of the spec's resource field schema. */
  schemaSha256: string;
  value: unknown;
}

// ---------------------------------------------------------------------------
// Execution side
// ---------------------------------------------------------------------------

export const CTX_METHODS = [
  "solana.account",
  "solana.accounts",
  "solana.tokenAccountsByOwner",
  "stacks.view",
] as const;
export type CtxMethod = (typeof CTX_METHODS)[number];

export interface ProviderSnapshotV1 {
  schema: typeof PROVIDER_SNAPSHOT_SCHEMA_V1;
  contextSlot: string;
  reads: SnapshotReadV1[];
  ctxCalls: CtxCallRecordV1[];
  observedAt: number;
  expiresAt: number;
}

export interface SnapshotReadV1 {
  name: string;
  result: AccountReadResultV1;
}

/** One account as read from chain. A missing account has `exists: false` and no data fields. */
export interface AccountReadResultV1 {
  address: string;
  exists: boolean;
  slot: string;
  lamports?: string;
  owner?: string;
  dataBase64?: string;
}

export interface CtxCallRecordV1 {
  seq: number;
  method: CtxMethod;
  args: unknown;
  /** Lowercase hex SHA-256 of the canonical JSON result. */
  resultSha256: string;
}

export type SessionKeyClass = "publishable" | "secret";
export type TransactionFormat = "v1" | "v0";
export type FeePlacement = "before" | "after";

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
  sessionKeyClass: SessionKeyClass;
  transactionFormat: TransactionFormat;
  inputSha256: string;
  resourceHashes: ProtocolResourceHash[];
  providerSnapshotHash: ProviderSnapshotHash;
  fees: IntentFeesV1 | null;
  signers: IntentSignerV1[];
  transactions: IntentTransactionV1[];
  blockhash: string;
  lastValidBlockHeight: string;
  expiresAt: number;
  createdAt: number;
}

export interface IntentDeploymentV1 {
  deploymentId: string;
  revision: number;
  releaseHash: ProtocolReleaseHash;
}

export interface IntentFeesV1 {
  asset: { kind: "native" } | { kind: "spl"; mint: string };
  gross: string;
  total: string;
  creator: string;
  platform: string;
  placement: FeePlacement;
  platformDestination: string;
  creatorDestination: string | null;
  /** Set when a cap or an override changed the platform leg. Additive. */
  feeAdjustment?: FeeAdjustmentV1;
}

/** Why the platform leg differs from the fee policy's rate. */
export interface FeeAdjustmentV1 {
  kind: "cap" | "override";
  /** `^[A-Za-z0-9_-]{1,64}$`. */
  id: string;
}

export interface IntentSignerV1 {
  class: SignerClass;
  pubkey: string;
  name: string | null;
}

export interface IntentTransactionV1 {
  index: number;
  messageSha256: string;
  signers: string[];
}

// ---------------------------------------------------------------------------
// Handler wire ABI
// ---------------------------------------------------------------------------

/** What a handler returns (`arete.protocol-handler/v1`). */
export interface PreparedOperationV1 {
  abi: typeof PROTOCOL_HANDLER_ABI_V1;
  transactions: PreparedTransactionV1[];
}

export interface PreparedTransactionV1 {
  name: string;
  instructions: PortableInstructionV1[];
}

export interface PortableInstructionV1 {
  programId: string;
  keys: PortableAccountMetaV1[];
  dataBase64: string;
}

export interface PortableAccountMetaV1 {
  pubkey: string;
  isSigner: boolean;
  isWritable: boolean;
}

// ---------------------------------------------------------------------------
// Execution receipts
// ---------------------------------------------------------------------------

/**
 * A signed statement that one transaction of an execution intent landed
 * with its deployment's attested memo. Optional fields are omitted, never
 * `null`.
 */
export interface ProtocolExecutionReceiptV1 {
  schema: typeof PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1;
  /** The transaction's first signature, base58. */
  signature: string;
  /** Decimal string. */
  slot: string;
  deployment: {
    deploymentId: string;
    /** The deployment short id the memo carries. */
    shortId: string;
    revision: number;
    releaseHash: string;
  };
  trustTier: TrustTier;
  /** Present exactly when `trustTier` is `reviewed`. */
  reviewHash?: string;
  step: string;
  instanceId: string;
  participantId?: string;
  /** `index` counts from 0; the memo's `<n>/<count>` from 1. */
  transaction: { index: number; count: number };
  /** Lowercase hex SHA-256 of the exact message bytes. */
  messageSha256: string;
  intentHash: string;
  attestationPubkey: string;
  fees?: {
    asset: { kind: "native" } | { kind: "spl"; mint: string };
    platform: string;
    creator: string;
  };
  status: "confirmed" | "finalized";
  /** Unix seconds. */
  issuedAt: number;
  /** Sorted by `keyId`; base64 Ed25519 over `receiptSigningMessage`. */
  signatures: Array<{ keyId: string; signature: string }>;
}

/** The published receipt keys. A retired key verifies receipts issued before `retiredAt`. */
export interface ProtocolReceiptKeySetV1 {
  schema: typeof PROTOCOL_RECEIPT_KEYS_SCHEMA_V1;
  keys: Array<{ keyId: string; publicKey: string; retiredAt?: number }>;
}
