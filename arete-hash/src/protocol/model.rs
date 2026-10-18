//! Typed models for `arete.protocol-spec/v1`.
//!
//! Deserialize these only from a document that `validate_protocol_spec_value`
//! accepted. Every collection is required and every optional scalar is
//! omitted rather than `null`, so a validated document re-serializes to the
//! same canonical bytes it was hashed from.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{HashId, ProgramRelease, ProgramSpec, StackManifest};

pub const PROTOCOL_SPEC_SCHEMA_V1: &str = "arete.protocol-spec/v1";
pub const PROTOCOL_HANDLER_ABI_V1: &str = "arete.protocol-handler/v1";
pub const PROTOCOL_CTX_V1: &str = "arete.protocol-ctx/v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolSpecV1 {
    pub schema: String,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler_abi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx_version: Option<String>,
    pub programs: BTreeMap<String, ProtocolProgramV1>,
    pub stacks: BTreeMap<String, ProtocolStackDependencyV1>,
    pub params: ObjectSchemaV1,
    pub roles: BTreeMap<String, RoleV1>,
    pub resources: BTreeMap<String, ResourceDeclV1>,
    pub instance_visibility: InstanceVisibility,
    pub reads: BTreeMap<String, ReadV1>,
    pub machines: MachinesV1,
    pub steps: BTreeMap<String, StepV1>,
    pub reactions: Vec<ReactionV1>,
    pub events: BTreeMap<String, EventV1>,
    pub metadata: MetadataV1,
    pub stack: StackSurfaceV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ProtocolProgramV1 {
    /// A program with a pinned ProgramSpec. Templates resolve against it.
    #[serde(rename_all = "camelCase")]
    ProgramSpec {
        program_id: String,
        program_spec_hash: HashId<ProgramSpec>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        program_release_hash: Option<HashId<ProgramRelease>>,
    },
    /// A runtime program whose instructions trusted Rust builds itself.
    Builtin { builtin: BuiltinProgram },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BuiltinProgram {
    System,
    SplToken,
    AssociatedToken,
    ComputeBudget,
    Memo,
}

impl BuiltinProgram {
    pub const ALL: [&'static str; 5] = [
        "system",
        "splToken",
        "associatedToken",
        "computeBudget",
        "memo",
    ];

    pub const fn program_id(self) -> &'static str {
        match self {
            Self::System => "11111111111111111111111111111111",
            Self::SplToken => "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
            Self::AssociatedToken => "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
            Self::ComputeBudget => "ComputeBudget111111111111111111111111111111",
            Self::Memo => "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolStackDependencyV1 {
    pub stack_manifest_hash: HashId<StackManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
}

/// A restricted JSON Schema subset for args, params, resources and metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectSchemaV1 {
    #[serde(rename = "type")]
    pub schema_type: ObjectType,
    pub properties: BTreeMap<String, FieldSchemaV1>,
    pub required: Vec<String>,
    pub additional_properties: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectType {
    Object,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldSchemaV1 {
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<StringFormat>,
    #[serde(default, rename = "enum", skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<FieldSchemaV1>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<BTreeMap<String, FieldSchemaV1>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_properties: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldType {
    String,
    Boolean,
    Integer,
    Array,
    Object,
}

/// String formats. Integer base units are decimal strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StringFormat {
    Pubkey,
    U8,
    U16,
    U32,
    U64,
    U128,
    I64,
    /// Unix seconds as a canonical decimal string.
    UnixTimestamp,
    Uri,
}

impl StringFormat {
    pub const ALL: [&'static str; 9] = [
        "pubkey",
        "u8",
        "u16",
        "u32",
        "u64",
        "u128",
        "i64",
        "unixTimestamp",
        "uri",
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceDeclV1 {
    pub schema: FieldSchemaV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstanceVisibility {
    /// Every integrator of the deployment sees the instance.
    Shared,
    /// Only the integrator account that created the instance sees it.
    Account,
}

/// A declared chain read. Guards, derivations and templates see its result
/// as `reads.<name>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ReadV1 {
    /// A program account decoded with the program's ProgramSpec.
    Account {
        program: String,
        account: String,
        address: String,
        optional: bool,
    },
    /// An SPL token account (amount, mint, owner).
    TokenAccount { address: String, optional: bool },
    /// The lamport balance of any account.
    Lamports { address: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MachinesV1 {
    pub instance: MachineV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant: Option<MachineV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MachineKind {
    Instance,
    Participant,
}

impl MachineKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Instance => "instance",
            Self::Participant => "participant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MachineV1 {
    pub initial: String,
    pub states: BTreeMap<String, StateV1>,
    /// Derivation rules, evaluated in order after every observation. The
    /// first rule whose `from` contains the current state and whose `when`
    /// holds moves the machine.
    pub derive: Vec<DeriveRuleV1>,
    /// Reads the derivations need.
    pub reads: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateV1 {
    pub kind: StateKind,
    pub terminal: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StateKind {
    /// Set by a confirmed step's transition.
    Recorded,
    /// Set by a derivation rule over reads and time.
    Derived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeriveRuleV1 {
    pub from: Vec<String>,
    pub to: String,
    pub when: String,
    /// When `when` depends on time, the instant at which the engine must
    /// re-evaluate it. The engine schedules a timer for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepScope {
    Create,
    Instance,
    Participant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum AuthorityV1 {
    /// The wallet that creates the instance. Create steps only.
    Creator,
    /// The participant's own wallet. Participant steps only.
    Participant,
    /// A named role wallet bound at create. Guarded by
    /// `wallet == instance.roles.<role>`.
    Role { role: String },
    /// An Arete keeper fee payer. Compiler-proven permissionless steps only.
    Keeper,
    /// A Squads proposer that may propose but never vote or execute.
    #[serde(rename_all = "camelCase")]
    SquadsProposer { multisig: String },
}

impl AuthorityV1 {
    pub fn signer_class(&self) -> SignerClass {
        match self {
            Self::Creator | Self::Participant | Self::Role { .. } => SignerClass::Wallet,
            Self::Keeper => SignerClass::Keeper,
            Self::SquadsProposer { .. } => SignerClass::SquadsProposer,
        }
    }

    /// Wallet steps are the only steps an app can prepare.
    pub fn is_client_callable(&self) -> bool {
        matches!(self.signer_class(), SignerClass::Wallet)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SignerClass {
    Wallet,
    Ephemeral,
    Attestation,
    Keeper,
    SquadsProposer,
}

impl SignerClass {
    pub const ALL: [&'static str; 5] = [
        "wallet",
        "ephemeral",
        "attestation",
        "keeper",
        "squadsProposer",
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepAccess {
    Public,
    Secret,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepV1 {
    pub scope: StepScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub authority: AuthorityV1,
    /// The maximum access. Present exactly on client-callable steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<StepAccess>,
    pub args: ObjectSchemaV1,
    pub guards: Vec<GuardV1>,
    pub reads: Vec<String>,
    /// Names of fresh keypairs allocated per intent, as `ephemeral.<name>`.
    pub ephemeral: Vec<String>,
    pub fee: FeeBasisV1,
    pub transitions: Vec<TransitionV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<BindV1>,
    pub glue: GlueV1,
    pub build: BuildV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuardV1 {
    /// The typed rejection reason.
    pub code: String,
    pub expr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum FeeBasisV1 {
    None,
    /// A percentage of an input amount argument. The revision's fee policy
    /// supplies the rate and destinations.
    #[serde(rename_all = "camelCase")]
    Input {
        amount_arg: String,
        asset: FeeAssetV1,
    },
    /// A fixed amount chosen by the revision's fee policy.
    Fixed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum FeeAssetV1 {
    Native,
    #[serde(rename_all = "camelCase")]
    Spl {
        mint: String,
        source_arg: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransitionV1 {
    pub machine: MachineKind,
    pub from: Vec<String>,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindV1 {
    /// Instance account name → expression over the create transaction's
    /// outputs (ephemeral keys, derived addresses, args, params).
    pub accounts: BTreeMap<String, String>,
    /// Role → expression over args or params. Immutable for the instance.
    pub roles: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlueV1 {
    /// Idempotent associated token accounts to create before the step.
    pub atas: Vec<AtaGlueV1>,
    /// Wrap native SOL into the fee payer's wSOL account before the step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_sol: Option<WrapSolV1>,
    /// Close the fee payer's wSOL account after the step.
    pub unwrap_sol: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AtaGlueV1 {
    pub owner: String,
    pub mint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WrapSolV1 {
    pub amount: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum BuildV1 {
    Template {
        instructions: Vec<TemplateInstructionV1>,
    },
    /// The escape hatch: a reviewed handler export in the release's bundle.
    /// `programs` is the allowlist the handler's output is checked against.
    Handler {
        export: String,
        programs: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateInstructionV1 {
    pub program: String,
    pub instruction: String,
    pub args: BTreeMap<String, String>,
    pub accounts: BTreeMap<String, String>,
    /// Trailing accounts no IDL names, appended after the instruction's IDL
    /// accounts in this order. Omitted when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remaining_accounts: Vec<RemainingAccountV1>,
}

/// One trailing account of a template instruction. `pubkey` is an account
/// expression; remaining accounts never sign, so `is_signer` is always false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemainingAccountV1 {
    pub pubkey: String,
    pub is_writable: bool,
    pub is_signer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReactionV1 {
    pub id: String,
    pub trigger: TriggerV1,
    pub action: ActionV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum TriggerV1 {
    /// Fires once at `time` while the machine is in one of `in`.
    At {
        machine: MachineKind,
        #[serde(rename = "in")]
        in_states: Vec<String>,
        time: String,
    },
    /// Fires when `condition` becomes true while the machine is in `in`.
    When {
        machine: MachineKind,
        #[serde(rename = "in")]
        in_states: Vec<String>,
        condition: String,
    },
    /// Fires on a transition into one of `to` (from one of `from`, if given).
    OnTransition {
        machine: MachineKind,
        from: Vec<String>,
        to: Vec<String>,
    },
    /// Fires when an intent for `step` confirms.
    OnConfirmed { step: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ActionV1 {
    RunKeeperStep { step: String },
    ProposeSquads { step: String },
    Emit { event: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventV1 {
    pub machine: MachineKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<MetadataSchemaV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant: Option<MetadataSchemaV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataSchemaV1 {
    pub fields: BTreeMap<String, MetadataFieldV1>,
    pub max_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataFieldV1 {
    pub schema: FieldSchemaV1,
    pub writer: MetadataWriter,
    pub filterable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataWriter {
    /// The account that created the instance (or the publisher).
    Creator,
    /// The participant's wallet, with a wallet proof.
    Participant,
}

/// What the generated protocol stack tracks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackSurfaceV1 {
    /// Bound instance accounts the stack decodes, by bound account name.
    pub instance_accounts: BTreeMap<String, StackInstanceAccountV1>,
    /// Machine-visible values mapped from the decoded instance accounts.
    pub instance_fields: BTreeMap<String, StackInstanceFieldV1>,
    /// Program instructions that count as protocol activity.
    pub activity: Vec<StackActivityV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackInstanceAccountV1 {
    pub program: String,
    pub account: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackInstanceFieldV1 {
    /// A key of `instanceAccounts`.
    pub account: String,
    /// A field path in that account's decoded data.
    pub field: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackActivityV1 {
    /// The protocol step this instruction belongs to.
    pub step: String,
    pub program: String,
    pub instruction: String,
    /// The instruction account that is a registered instance account.
    pub instance_account: String,
    /// The instruction account holding the acting wallet.
    pub wallet_account: String,
    /// An instruction argument summed into participant and instance totals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_arg: Option<String>,
}

/// Parsed form of the canonical JSON value of any contract.
pub type ContractValue = Value;
