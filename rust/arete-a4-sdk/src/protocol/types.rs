//! Protocol definitions and the hosted API's request and response shapes.
//!
//! Mirror of `typescript/core/src/protocol/types.ts`. Generated protocol
//! crates (`a4 install protocol <ref> --rust`) emit a [`ProtocolDefinition`]
//! constant plus typed args, guard codes, states and event names on top of
//! these types.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// An integer argument or parameter wider than a JSON number holds exactly
/// (`u64`, `u128`, `i64`, unix timestamps, ...). It travels as a decimal
/// string; build one from any Rust integer and read it back with
/// [`Self::parse`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct ProtocolInteger(String);

impl ProtocolInteger {
    /// A decimal string, checked to be an optionally negative integer.
    pub fn new(decimal: impl Into<String>) -> Result<Self, String> {
        let decimal = decimal.into();
        let digits = decimal.strip_prefix('-').unwrap_or(&decimal);
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(format!("'{decimal}' is not a decimal integer"));
        }
        Ok(Self(decimal))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The value as any integer type it fits.
    pub fn parse<T: std::str::FromStr>(&self) -> Result<T, T::Err> {
        self.0.parse()
    }
}

impl fmt::Display for ProtocolInteger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

macro_rules! protocol_integer_from {
    ($($ty:ty),*) => {$(
        impl From<$ty> for ProtocolInteger {
            fn from(value: $ty) -> Self {
                Self(value.to_string())
            }
        }
    )*};
}
protocol_integer_from!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

impl Serialize for ProtocolInteger {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ProtocolInteger {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::String(text) => Self::new(text).map_err(serde::de::Error::custom),
            Value::Number(number) if number.is_u64() || number.is_i64() => {
                Ok(Self(number.to_string()))
            }
            other => Err(serde::de::Error::custom(format!(
                "expected a decimal integer string, got {other}"
            ))),
        }
    }
}

/// The class of API key a protocol client authenticates with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolKeyClass {
    /// Origin-bound publishable key: `public` steps only.
    Publishable,
    /// Secret or agent key: every client-callable step.
    Secret,
}

/// Which machine a step acts on: a new instance, an instance, or a participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolStepScope {
    Create,
    Instance,
    Participant,
}

/// `public` steps accept publishable keys; `secret` steps need a secret key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolStepAccess {
    Public,
    Secret,
}

/// The machine an event or transition belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolMachineKind {
    Instance,
    Participant,
}

/// Whether a deployment is open to every account or only its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolDeploymentAccess {
    Private,
    Public,
}

/// Whether instances are shared across accounts or scoped to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolInstanceVisibility {
    Shared,
    Account,
}

/// Who may write a metadata field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolMetadataWriter {
    Creator,
    Participant,
}

/// Where a protocol deployment is served. Never carries credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolDeploymentDescriptor {
    pub deployment_id: &'static str,
    pub access: ProtocolDeploymentAccess,
    /// Base URL of the hosted API, e.g. `https://api.arete.run/api`.
    pub api_url: &'static str,
    /// Where session tokens are minted, e.g. `https://api.arete.run/ws/sessions`.
    pub session_endpoint: &'static str,
}

/// Runtime description of one client-callable step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolStepDefinition {
    pub name: &'static str,
    pub scope: ProtocolStepScope,
    pub access: ProtocolStepAccess,
    /// Guard codes the step can reject with, in evaluation order.
    pub guards: &'static [&'static str],
    /// Argument names.
    pub args: &'static [&'static str],
}

/// One event a spec's `emit` reactions publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolEventDefinition {
    pub name: &'static str,
    pub machine: ProtocolMachineKind,
}

/// One metadata field: who writes it, and whether instances filter by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolMetadataFieldDefinition {
    pub name: &'static str,
    pub writer: ProtocolMetadataWriter,
    pub filterable: bool,
}

/// The protocol's own generated stack view that mirrors an instance live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolLiveStackDescriptor {
    /// Key of the stack among the protocol's hoisted stacks.
    pub stack: &'static str,
    /// `<Entity>/<mode>` of the state view, e.g. `TokenRaiseInstance/state`.
    pub view: &'static str,
    /// Instance account whose address keys the view row.
    pub key_account: &'static str,
}

/// A generated protocol definition. `a4 install protocol <ref> --rust` emits
/// one as `<NAME>_PROTOCOL`; pass it to [`super::ProtocolClient::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolDefinition {
    /// The spec name, e.g. `tokenRaise`.
    pub name: &'static str,
    pub version: &'static str,
    /// Registry package name, e.g. `token-raise`.
    pub package: &'static str,
    pub package_release_hash: &'static str,
    pub protocol_spec_hash: &'static str,
    pub protocol_release_hash: &'static str,
    pub deployment: ProtocolDeploymentDescriptor,
    pub instance_visibility: ProtocolInstanceVisibility,
    /// Client-callable steps only.
    pub steps: &'static [ProtocolStepDefinition],
    pub events: &'static [ProtocolEventDefinition],
    pub instance_metadata: &'static [ProtocolMetadataFieldDefinition],
    pub participant_metadata: &'static [ProtocolMetadataFieldDefinition],
    pub live_stack: Option<ProtocolLiveStackDescriptor>,
}

impl ProtocolDefinition {
    /// The client-callable step named `name`.
    pub fn step(&self, name: &str) -> Option<&ProtocolStepDefinition> {
        self.steps.iter().find(|step| step.name == name)
    }

    /// The event named `name`.
    pub fn event(&self, name: &str) -> Option<&ProtocolEventDefinition> {
        self.events.iter().find(|event| event.name == name)
    }
}

// ---------------------------------------------------------------------------
// Rejections
// ---------------------------------------------------------------------------

/// Access refusals a prepare can return instead of a guard rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProtocolAccessRejectionCode {
    /// A publishable key asked for a `secret` step.
    StepRequiresSecretKey,
    /// The session was not minted for this step.
    StepNotInSessionScope,
    /// The step is not callable by clients (keeper, role or Squads steps).
    StepNotClientCallable,
}

impl ProtocolAccessRejectionCode {
    pub const ALL: [Self; 3] = [
        Self::StepRequiresSecretKey,
        Self::StepNotInSessionScope,
        Self::StepNotClientCallable,
    ];

    pub fn as_wire(self) -> &'static str {
        match self {
            Self::StepRequiresSecretKey => "step_requires_secret_key",
            Self::StepNotInSessionScope => "step_not_in_session_scope",
            Self::StepNotClientCallable => "step_not_client_callable",
        }
    }

    /// Parse a wire code, ignoring case and surrounding whitespace.
    pub fn from_wire(code: &str) -> Option<Self> {
        let code = code.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_wire() == code)
    }
}

impl fmt::Display for ProtocolAccessRejectionCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_wire())
    }
}

/// Lifecycle rejections a prepare can return instead of a guard rejection.
/// Both carry `detail.state`, the terminal state; `InstanceTerminal` is
/// checked first. Step listings report the same codes as `reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProtocolStateRejectionCode {
    /// The instance is in a terminal state: no instance or participant step
    /// can prepare.
    InstanceTerminal,
    /// The preparing wallet's participant is in a terminal state: its
    /// participant steps cannot prepare, unless the step declares a
    /// participant transition out of that state.
    ParticipantTerminal,
}

impl ProtocolStateRejectionCode {
    pub const ALL: [Self; 2] = [Self::InstanceTerminal, Self::ParticipantTerminal];

    pub fn as_wire(self) -> &'static str {
        match self {
            Self::InstanceTerminal => "instance_terminal",
            Self::ParticipantTerminal => "participant_terminal",
        }
    }

    /// Parse a wire code, ignoring case and surrounding whitespace.
    pub fn from_wire(code: &str) -> Option<Self> {
        let code = code.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_wire() == code)
    }
}

impl fmt::Display for ProtocolStateRejectionCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_wire())
    }
}

/// A step's guard codes. Generated protocol crates implement it for one
/// enum per step; `String` accepts any code.
pub trait ProtocolGuardCode: Sized + Clone + fmt::Debug + Send + Sync + 'static {
    /// The code for `wire`, or `None` when the step declares no such guard.
    fn from_wire(wire: &str) -> Option<Self>;
    fn as_wire(&self) -> &str;
}

impl ProtocolGuardCode for String {
    fn from_wire(wire: &str) -> Option<Self> {
        Some(wire.to_string())
    }

    fn as_wire(&self) -> &str {
        self
    }
}

/// Why a step did not prepare.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolRejectionCode<G = String> {
    /// A guard of the step failed.
    Guard(G),
    /// The key or session may not prepare the step.
    Access(ProtocolAccessRejectionCode),
    /// The instance, or the wallet's participant, is in a terminal state.
    State(ProtocolStateRejectionCode),
    /// A code this client's generated types do not know (a newer release).
    Unrecognized(String),
}

impl<G: ProtocolGuardCode> ProtocolRejectionCode<G> {
    /// Classify a wire code: access codes first, then state codes, then the
    /// step's guards.
    pub fn from_wire(code: &str) -> Self {
        if let Some(access) = ProtocolAccessRejectionCode::from_wire(code) {
            return Self::Access(access);
        }
        if let Some(state) = ProtocolStateRejectionCode::from_wire(code) {
            return Self::State(state);
        }
        match G::from_wire(code) {
            Some(guard) => Self::Guard(guard),
            None => Self::Unrecognized(code.to_string()),
        }
    }

    pub fn as_wire(&self) -> &str {
        match self {
            Self::Guard(guard) => guard.as_wire(),
            Self::Access(access) => access.as_wire(),
            Self::State(state) => state.as_wire(),
            Self::Unrecognized(code) => code,
        }
    }
}

/// A step that did not prepare: a guard failed, access was refused, or the
/// instance or participant is in a terminal state.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolRejection<G = String> {
    /// The guard code, an access refusal code, or a state rejection code.
    pub code: ProtocolRejectionCode<G>,
    pub step: String,
    /// Source of the guard expression that failed.
    pub guard: Option<String>,
    /// The evaluated operands of the guard, or `{"state": ...}` for a state
    /// rejection.
    pub detail: Option<Map<String, Value>>,
}

impl<G: ProtocolGuardCode> ProtocolRejection<G> {
    /// The wire code, e.g. `deadline_passed` or `step_requires_secret_key`.
    pub fn rejected(&self) -> &str {
        self.code.as_wire()
    }
}

impl ProtocolRejection<String> {
    /// Re-read the code with a step's generated guard type.
    pub fn typed<G: ProtocolGuardCode>(self) -> ProtocolRejection<G> {
        let code = match self.code {
            ProtocolRejectionCode::Guard(code) | ProtocolRejectionCode::Unrecognized(code) => {
                ProtocolRejectionCode::from_wire(&code)
            }
            ProtocolRejectionCode::Access(access) => ProtocolRejectionCode::Access(access),
            ProtocolRejectionCode::State(state) => ProtocolRejectionCode::State(state),
        };
        ProtocolRejection {
            code,
            step: self.step,
            guard: self.guard,
            detail: self.detail,
        }
    }
}

// ---------------------------------------------------------------------------
// Execution intents
// ---------------------------------------------------------------------------

/// Schema id of [`ExecutionIntentV1`].
pub const EXECUTION_INTENT_SCHEMA_V1: &str = "arete.execution-intent/v1";

/// Transaction format a prepare builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolTransactionFormat {
    /// Solana transaction v1 (the default): one transaction per step.
    #[default]
    V1,
    /// Versioned v0 transactions; a step may need several.
    V0,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentDeploymentV1 {
    pub deployment_id: String,
    pub revision: u64,
    pub release_hash: String,
}

/// Asset a fee is paid in: `{kind: "native"}` or `{kind: "spl", mint}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum IntentFeeAssetV1 {
    Native,
    Spl { mint: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentFeesV1 {
    pub asset: IntentFeeAssetV1,
    pub gross: String,
    pub total: String,
    pub creator: String,
    pub platform: String,
    /// `before` or `after` the step's instructions.
    pub placement: String,
    pub platform_destination: String,
    pub creator_destination: Option<String>,
    /// Set when a cap or an override changed the platform leg.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_adjustment: Option<FeeAdjustmentV1>,
}

/// Why the platform leg differs from the fee policy's rate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeAdjustmentV1 {
    /// `cap` or `override`.
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentSignerV1 {
    /// `wallet`, `ephemeral`, `attestation`, `keeper` or `squadsProposer`.
    pub class: String,
    pub pubkey: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentTransactionV1 {
    pub index: u32,
    pub message_sha256: String,
    pub signers: Vec<String>,
}

/// What a prepare committed to: the transactions, signers, fees and expiry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionIntentV1 {
    pub schema: String,
    pub execution_id: String,
    pub deployment: IntentDeploymentV1,
    pub step: String,
    #[serde(default)]
    pub instance_id: Option<String>,
    #[serde(default)]
    pub participant_id: Option<String>,
    pub wallet: String,
    #[serde(default)]
    pub role: Option<String>,
    pub caller_account: String,
    pub session_key_class: ProtocolKeyClass,
    pub transaction_format: ProtocolTransactionFormat,
    pub input_sha256: String,
    #[serde(default)]
    pub resource_hashes: Vec<String>,
    pub provider_snapshot_hash: String,
    #[serde(default)]
    pub fees: Option<IntentFeesV1>,
    pub signers: Vec<IntentSignerV1>,
    pub transactions: Vec<IntentTransactionV1>,
    pub blockhash: String,
    pub last_valid_block_height: String,
    pub expires_at: u64,
    pub created_at: u64,
}

/// One partially signed transaction for the wallet to sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolPreparedTransaction {
    pub index: u32,
    pub transaction_base64: String,
}

impl ProtocolPreparedTransaction {
    /// The transaction's bytes.
    pub fn bytes(&self) -> Result<Vec<u8>, base64::DecodeError> {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.decode(&self.transaction_base64)
    }
}

/// A prepared step: the intent and the transactions for the wallet to sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolPrepared {
    pub intent: ExecutionIntentV1,
    pub transactions: Vec<ProtocolPreparedTransaction>,
}

/// What a prepare resolves with.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // Built once per prepare; boxing buys nothing.
pub enum ProtocolPrepareResult<G = String> {
    Prepared(ProtocolPrepared),
    Rejected(ProtocolRejection<G>),
}

impl<G> ProtocolPrepareResult<G> {
    pub fn prepared(self) -> Option<ProtocolPrepared> {
        match self {
            Self::Prepared(prepared) => Some(prepared),
            Self::Rejected(_) => None,
        }
    }

    pub fn rejection(&self) -> Option<&ProtocolRejection<G>> {
        match self {
            Self::Prepared(_) => None,
            Self::Rejected(rejection) => Some(rejection),
        }
    }
}

impl ProtocolPrepareResult<String> {
    /// Re-read a rejection with a step's generated guard type.
    pub fn typed<G: ProtocolGuardCode>(self) -> ProtocolPrepareResult<G> {
        match self {
            Self::Prepared(prepared) => ProtocolPrepareResult::Prepared(prepared),
            Self::Rejected(rejection) => ProtocolPrepareResult::Rejected(rejection.typed()),
        }
    }
}

/// Where an execution intent stands.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProtocolIntentStatus {
    Prepared,
    Submitting,
    PartiallyCompleted,
    Completed,
    ChainFailed,
    /// The transactions were sent but their outcome is unknown. Never sign
    /// or submit the step again: read the intent until it settles.
    SubmittedUnknown,
    Expired,
    Rejected,
    /// A status this client does not know.
    Other(String),
}

impl ProtocolIntentStatus {
    pub fn from_wire(value: &str) -> Self {
        match value {
            "prepared" => Self::Prepared,
            "submitting" => Self::Submitting,
            "partially_completed" => Self::PartiallyCompleted,
            "completed" => Self::Completed,
            "chain_failed" => Self::ChainFailed,
            "submitted_unknown" => Self::SubmittedUnknown,
            "expired" => Self::Expired,
            "rejected" => Self::Rejected,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn as_wire(&self) -> &str {
        match self {
            Self::Prepared => "prepared",
            Self::Submitting => "submitting",
            Self::PartiallyCompleted => "partially_completed",
            Self::Completed => "completed",
            Self::ChainFailed => "chain_failed",
            Self::SubmittedUnknown => "submitted_unknown",
            Self::Expired => "expired",
            Self::Rejected => "rejected",
            Self::Other(other) => other,
        }
    }

    /// No further transition will happen.
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::ChainFailed | Self::Expired | Self::Rejected
        )
    }
}

impl Serialize for ProtocolIntentStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_wire())
    }
}

impl<'de> Deserialize<'de> for ProtocolIntentStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_wire(&String::deserialize(deserializer)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolIntentTransactionState {
    pub index: u32,
    #[serde(default)]
    pub signature: Option<String>,
    pub state: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// An execution intent's state after a submit or read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolIntentState {
    pub execution_id: String,
    pub state: ProtocolIntentStatus,
    #[serde(default)]
    pub transactions: Vec<ProtocolIntentTransactionState>,
    /// Where the signed receipt of a completed intent can be fetched, when
    /// the API issues receipts for this deployment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Instances and participants
// ---------------------------------------------------------------------------

/// Whether a step can be called now, and why not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolStepAvailability {
    pub step: String,
    pub allowed: bool,
    /// Guard code or access code when not allowed.
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub detail: Option<Map<String, Value>>,
    /// Guards that depend on arguments and were not evaluated.
    #[serde(default)]
    pub args_dependent: Option<Vec<String>>,
    /// Which machine the step acts on. Absent from older API responses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ProtocolStepScope>,
    /// The step's typed arguments, as the protocol spec declares them.
    /// Empty when the API response does not carry them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<ProtocolStepArgDescriptor>,
}

/// One declared argument of a step: its name, whether it may be omitted,
/// and its field schema as the spec declares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolStepArgDescriptor {
    pub name: String,
    /// The schema's JSON type: `string`, `integer`, `boolean`, `array` or
    /// `object`. Solana amounts and keys are strings with a `format`.
    #[serde(rename = "type")]
    pub arg_type: String,
    /// The string format, e.g. `u64`, `i64`, `pubkey` or `unixTimestamp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default)]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The rest of the field schema when the spec declares it: `enum`,
    /// `minimum`, `maximum`, `maxLength`, `items`, `maxItems`,
    /// `properties`, `required`.
    #[serde(flatten)]
    pub schema: Map<String, Value>,
}

impl ProtocolStepArgDescriptor {
    /// The type as a protocol author writes it: the `format` when there is
    /// one (`u64`, `pubkey`), else the JSON type.
    pub fn declared_type(&self) -> &str {
        self.format.as_deref().unwrap_or(&self.arg_type)
    }
}

/// What a wallet can do on a deployment: the create steps it can call and,
/// per instance it is related to, the steps it can call there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolWalletActions {
    pub deployment_id: String,
    pub wallet: String,
    /// Availability of the deployment's `create` steps for the wallet.
    #[serde(default)]
    pub create_steps: Vec<ProtocolStepAvailability>,
    #[serde(default)]
    pub instances: Vec<ProtocolWalletInstanceActions>,
    /// Pass to the next read to continue the instance list.
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// One instance a wallet is related to, and the steps it can call there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolWalletInstanceActions {
    pub instance_id: String,
    pub state: String,
    /// How the wallet relates to the instance: `participant`, `creator`, or
    /// `role:<name>` for each role the wallet holds.
    #[serde(default)]
    pub relation: Vec<String>,
    /// The wallet's participant state, when it is a participant.
    #[serde(default)]
    pub participant_state: Option<String>,
    #[serde(default)]
    pub steps: Vec<ProtocolStepAvailability>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolInstanceSummary {
    pub id: String,
    pub deployment_id: String,
    pub revision: u64,
    pub state: String,
    pub terminal: bool,
    pub visibility: ProtocolInstanceVisibility,
    #[serde(default)]
    pub accounts: BTreeMap<String, String>,
    #[serde(default)]
    pub roles: BTreeMap<String, String>,
    pub created_by_wallet: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub closed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolInstancePage {
    pub instances: Vec<ProtocolInstanceSummary>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolInstanceDetail {
    #[serde(flatten)]
    pub summary: ProtocolInstanceSummary,
    pub release_hash: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub last_error_code: Option<String>,
    #[serde(default)]
    pub derived: Value,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default)]
    pub metadata_version: Option<u64>,
    #[serde(default)]
    pub pending_timers: Vec<Value>,
    #[serde(default)]
    pub allowed_steps: Vec<ProtocolStepAvailability>,
}

impl std::ops::Deref for ProtocolInstanceDetail {
    type Target = ProtocolInstanceSummary;

    fn deref(&self) -> &Self::Target {
        &self.summary
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolParticipantDetail {
    pub instance_id: String,
    pub wallet: String,
    #[serde(default)]
    pub participant_id: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default)]
    pub history: Vec<Value>,
    #[serde(default)]
    pub allowed_steps: Vec<ProtocolStepAvailability>,
}

/// A metadata document and its version (`None` for participant documents).
#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolMetadataState {
    pub metadata: Map<String, Value>,
    pub version: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolMetadataUpdateResult {
    pub instance_id: String,
    #[serde(default)]
    pub participant_id: Option<String>,
    pub version: u64,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// The JSON body of an emitted event, as delivered over webhooks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolEventPayloadV1 {
    /// Unique emission id.
    pub id: String,
    /// The spec's event name, such as `raise.succeeded`.
    pub event: String,
    pub deployment_id: String,
    pub instance_id: String,
    /// Set for participant events, `None` for instance events.
    pub participant_id: Option<String>,
    /// The state of the event's machine when it was emitted.
    pub state: String,
    /// RFC 3339 UTC time of the emission.
    pub occurred_at: String,
}

/// Data of an `emit` event on an instance's event stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolEmitEventData {
    /// The emission's id (a server sequence number).
    pub id: i64,
    pub event: String,
    pub reaction: String,
    pub instance_id: String,
    #[serde(default)]
    pub participant_id: Option<String>,
    /// The transition that triggered the emission; absent for emissions
    /// triggered by a confirmed step rather than a transition.
    #[serde(default)]
    pub transition_id: Option<i64>,
    pub at: String,
}

/// SSE event type of [`ProtocolSquadsProposerMissingData`].
pub const SQUADS_PROPOSER_MISSING_EVENT: &str = "arete.squads.proposerMissing";

/// Data of the system event `arete.squads.proposerMissing`: an admin step's
/// Squads proposal is parked because the calling account's proposer is not an
/// Initiate-only member of the multisig. It is retried with backoff for up to
/// 24 hours; add the proposer ([`ProtocolSquadsAddProposerRequest`]) to
/// unblock it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolSquadsProposerMissingData {
    pub id: i64,
    pub deployment_id: String,
    pub instance_id: String,
    pub outbox_id: i64,
    pub step: String,
    pub multisig: String,
    pub proposer: String,
    /// `not_member` or `permissions_invalid`.
    pub problem: String,
    pub at: String,
}

/// The calling account's Squads proposer for a deployment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolSquadsProposer {
    pub pubkey: String,
    /// Always `initiate`: the proposer may propose and nothing else.
    pub permissions: String,
    /// The root the proposer derives from; `None` for an environment key.
    #[serde(default)]
    pub root_key_id: Option<String>,
}

/// Body of `squads/add-proposer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolSquadsAddProposerRequest {
    /// The team's Squads v4 multisig.
    pub multisig: String,
    /// A member with the Initiate permission who signs and pays.
    pub member: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_format: Option<ProtocolTransactionFormat>,
}

impl ProtocolSquadsAddProposerRequest {
    pub fn new(multisig: impl Into<String>, member: impl Into<String>) -> Self {
        Self {
            multisig: multisig.into(),
            member: member.into(),
            transaction_format: None,
        }
    }

    pub fn with_transaction_format(mut self, format: ProtocolTransactionFormat) -> Self {
        self.transaction_format = Some(format);
        self
    }
}

/// An unsigned transaction creating a Squads config proposal that adds the
/// proposer with Initiate only. `member` signs and sends it; the team then
/// approves and executes the proposal in Squads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolSquadsAddProposerResult {
    pub transaction_base64: String,
    pub transaction_format: ProtocolTransactionFormat,
    pub transaction_index: u64,
    pub proposal: String,
    pub config_transaction: String,
    pub multisig: String,
    pub member: String,
    pub proposer: String,
    pub last_valid_block_height: u64,
}

/// Whether the calling account's proposer can propose to a multisig.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolSquadsStatus {
    /// A member with exactly the Initiate permission.
    pub ready: bool,
    pub member: bool,
    #[serde(default)]
    pub mask: Option<u8>,
    pub threshold: u16,
    pub multisig: String,
    pub proposer: String,
    /// `not_member` or `permissions_invalid`; absent when ready.
    #[serde(default)]
    pub problem: Option<String>,
}

/// One event from an instance's event stream. `id` is the resume cursor.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ProtocolStreamEvent {
    Emit {
        id: String,
        data: ProtocolEmitEventData,
    },
    /// `arete.squads.proposerMissing` (a system event).
    SquadsProposerMissing {
        id: String,
        data: ProtocolSquadsProposerMissingData,
    },
    Transition {
        id: String,
        data: Map<String, Value>,
    },
    Participant {
        id: String,
        data: Map<String, Value>,
    },
    Observation {
        id: String,
        data: Map<String, Value>,
    },
}

impl ProtocolStreamEvent {
    /// The resume cursor.
    pub fn id(&self) -> &str {
        match self {
            Self::Emit { id, .. }
            | Self::SquadsProposerMissing { id, .. }
            | Self::Transition { id, .. }
            | Self::Participant { id, .. }
            | Self::Observation { id, .. } => id,
        }
    }

    /// The SSE event type: `emit`, `transition`, `participant`,
    /// `observation` or a system event such as `arete.squads.proposerMissing`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Emit { .. } => "emit",
            Self::SquadsProposerMissing { .. } => SQUADS_PROPOSER_MISSING_EVENT,
            Self::Transition { .. } => "transition",
            Self::Participant { .. } => "participant",
            Self::Observation { .. } => "observation",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_integers_travel_as_decimal_strings() {
        let value = ProtocolInteger::from(u128::MAX);
        assert_eq!(
            serde_json::to_value(&value).unwrap(),
            Value::String(u128::MAX.to_string())
        );
        assert_eq!(value.parse::<u128>().unwrap(), u128::MAX);
        let read: ProtocolInteger = serde_json::from_str("\"-42\"").unwrap();
        assert_eq!(read.parse::<i64>().unwrap(), -42);
        let read: ProtocolInteger = serde_json::from_str("7").unwrap();
        assert_eq!(read.as_str(), "7");
        assert!(serde_json::from_str::<ProtocolInteger>("\"1.5\"").is_err());
        assert!(ProtocolInteger::new("-").is_err());
    }
}
