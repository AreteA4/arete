use std::collections::{BTreeMap, BTreeSet};

use arete_hash::{hash_jcs, AccountResolutionV1, HashId, LiveSpec, PdaDefinitionV1, ProgramSpec};
use arete_idl::snapshot::IdlSerializationSnapshot;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    json_error, reject_private_fields, validate_envelope_version, validate_kind, ArtifactError,
    ARTIFACT_VERSION_V1, LIVE_COMPILER_CONTRACT_V1, LIVE_SPEC_KIND, LIVE_SPEC_SCHEMA_V2,
    LIVE_WIRE_CONTRACT_V1,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableFieldPath {
    pub segments: Vec<String>,
    pub offsets: Option<Vec<usize>>,
}

impl PortableFieldPath {
    pub fn new(segments: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            segments: segments.into_iter().map(Into::into).collect(),
            offsets: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PortableTransformation {
    HexEncode,
    HexDecode,
    Base58Encode,
    Base58Decode,
    ToString,
    ToNumber,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PortablePopulationStrategy {
    SetOnce,
    LastWrite,
    Append,
    Merge,
    Max,
    Sum,
    Count,
    Min,
    UniqueCount,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableComputedFieldSpec {
    pub target_path: String,
    pub expression: PortableComputedExpr,
    pub result_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PortableComputedExpr {
    FieldRef {
        path: String,
    },
    UnwrapOr {
        expr: Box<Self>,
        default: Value,
    },
    Binary {
        op: PortableBinaryOp,
        left: Box<Self>,
        right: Box<Self>,
    },
    Cast {
        expr: Box<Self>,
        to_type: String,
    },
    MethodCall {
        expr: Box<Self>,
        method: String,
        args: Vec<Self>,
    },
    ResolverComputed {
        resolver: String,
        method: String,
        args: Vec<Self>,
    },
    Literal {
        value: Value,
    },
    Paren {
        expr: Box<Self>,
    },
    Var {
        name: String,
    },
    Let {
        name: String,
        value: Box<Self>,
        body: Box<Self>,
    },
    If {
        condition: Box<Self>,
        then_branch: Box<Self>,
        else_branch: Box<Self>,
    },
    None,
    Some {
        value: Box<Self>,
    },
    Slice {
        expr: Box<Self>,
        start: usize,
        end: usize,
    },
    Index {
        expr: Box<Self>,
        index: usize,
    },
    U64FromLeBytes {
        bytes: Box<Self>,
    },
    U64FromBeBytes {
        bytes: Box<Self>,
    },
    ByteArray {
        bytes: Vec<u8>,
    },
    Closure {
        param: String,
        body: Box<Self>,
    },
    Unary {
        op: PortableUnaryOp,
        expr: Box<Self>,
    },
    JsonToBytes {
        expr: Box<Self>,
    },
    ContextSlot,
    ContextTimestamp,
    Keccak256 {
        expr: Box<Self>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PortableBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Gt,
    Lt,
    Gte,
    Lte,
    Eq,
    Ne,
    And,
    Or,
    Xor,
    BitAnd,
    BitOr,
    Shl,
    Shr,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PortableUnaryOp {
    Not,
    ReverseBits,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum PortableResolverType {
    Token,
    Url(PortableUrlResolverConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum PortableHttpMethod {
    #[default]
    Get,
    Post,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum PortableUrlTemplatePart {
    Literal(String),
    FieldRef(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum PortableUrlSource {
    FieldPath(String),
    Template(Vec<PortableUrlTemplatePart>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PortableUrlResolverConfig {
    pub url_source: PortableUrlSource,
    #[serde(default)]
    pub method: PortableHttpMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableResolverExtractSpec {
    pub target_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<PortableTransformation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum PortableResolveStrategy {
    #[default]
    SetOnce,
    LastWrite,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableResolverCondition {
    pub field_path: String,
    pub op: PortableComparisonOp,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableResolverSpec {
    pub resolver: PortableResolverType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_value: Option<Value>,
    #[serde(default)]
    pub strategy: PortableResolveStrategy,
    pub extracts: Vec<PortableResolverExtractSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<PortableResolverCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortableIdentitySpec {
    pub primary_keys: Vec<String>,
    pub lookup_indexes: Vec<PortableLookupIndexSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortableLookupIndexSpec {
    pub field_name: String,
    pub temporal_field: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableHandlerSpec {
    pub source: PortableSourceSpec,
    pub key_resolution: PortableKeyResolutionStrategy,
    pub mappings: Vec<PortableFieldMapping>,
    pub conditions: Vec<PortableCondition>,
    pub emit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PortableKeyResolutionStrategy {
    Embedded {
        primary_field: PortableFieldPath,
    },
    Lookup {
        primary_field: PortableFieldPath,
        /// The lookup index this handler declares (`<field>_lookup_index`);
        /// see the interpreter's `KeyResolutionStrategy::Lookup`. Omitted
        /// when absent so undeclared lookups keep their bytes and hashes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index_name: Option<String>,
    },
    Computed {
        primary_field: PortableFieldPath,
        compute_partition: PortableComputeFunction,
    },
    TemporalLookup {
        lookup_field: PortableFieldPath,
        timestamp_field: PortableFieldPath,
        index_name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PortableSourceSpec {
    Source {
        program_id: Option<String>,
        discriminator: Option<Vec<u8>>,
        type_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        serialization: Option<IdlSerializationSnapshot>,
        #[serde(default)]
        is_account: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableFieldMapping {
    pub target_path: String,
    pub source: PortableMappingSource,
    pub transform: Option<PortableTransformation>,
    pub population: PortablePopulationStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<PortableConditionExpr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<String>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub emit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PortableMappingSource {
    FromSource {
        path: PortableFieldPath,
        default: Option<Value>,
        transform: Option<PortableTransformation>,
    },
    Constant(Value),
    Computed {
        inputs: Vec<PortableFieldPath>,
        function: PortableComputeFunction,
    },
    FromState {
        path: String,
    },
    AsEvent {
        fields: Vec<PortableMappingSource>,
    },
    WholeSource,
    AsCapture {
        field_transforms: BTreeMap<String, PortableTransformation>,
    },
    FromContext {
        field: String,
    },
    FromExpr {
        expr: PortableComputedExpr,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PortableComputeFunction {
    Sum,
    Concat,
    Format(String),
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableCondition {
    pub field: PortableFieldPath,
    pub operator: PortableConditionOp,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PortableConditionOp {
    Equals,
    NotEquals,
    GreaterThan,
    LessThan,
    Contains,
    Exists,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableEntitySection {
    pub name: String,
    pub fields: Vec<PortableFieldTypeInfo>,
    #[serde(default)]
    pub is_nested_struct: bool,
    #[serde(default)]
    pub parent_field: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableFieldTypeInfo {
    pub field_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_name: Option<String>,
    pub rust_type_name: String,
    pub base_type: PortableBaseType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integer_kind: Option<PortableIntegerKind>,
    pub is_optional: bool,
    pub is_array: bool,
    #[serde(default)]
    pub inner_type: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub resolved_type: Option<PortableResolvedStructType>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub emit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableResolvedStructType {
    pub type_name: String,
    pub fields: Vec<PortableResolvedField>,
    pub is_instruction: bool,
    pub is_account: bool,
    pub is_event: bool,
    #[serde(default)]
    pub is_enum: bool,
    #[serde(default)]
    pub enum_variants: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableResolvedField {
    pub field_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_name: Option<String>,
    pub field_type: String,
    pub base_type: PortableBaseType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integer_kind: Option<PortableIntegerKind>,
    pub is_optional: bool,
    pub is_array: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PortableIntegerKind {
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PortableBaseType {
    Integer,
    Float,
    String,
    Boolean,
    Object,
    Array,
    Binary,
    Timestamp,
    Pubkey,
    Any,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableResolverHook {
    pub account_type: String,
    pub strategy: PortableResolverStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PortableResolverStrategy {
    PdaReverseLookup {
        lookup_name: String,
        queue_discriminators: Vec<Vec<u8>>,
    },
    DirectField {
        field_path: PortableFieldPath,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableInstructionHook {
    pub instruction_type: String,
    pub actions: Vec<PortableHookAction>,
    pub lookup_by: Option<PortableFieldPath>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PortableHookAction {
    RegisterPdaMapping {
        pda_field: PortableFieldPath,
        seed_field: PortableFieldPath,
        lookup_name: String,
        /// Read `pda_field` and/or `seed_field` from sibling instructions of
        /// the same transaction. Omitted for same-instruction registration.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sibling: Option<PortableSiblingSelector>,
        /// The seed as an expression over the registering instruction,
        /// replacing `seed_field`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seed_expr: Option<PortableComputedExpr>,
        /// Register only when this boolean expression over the registering
        /// instruction is `true`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guard: Option<PortableComputedExpr>,
    },
    SetField {
        target_field: String,
        source: PortableMappingSource,
        condition: Option<PortableConditionExpr>,
    },
    IncrementField {
        target_field: String,
        increment_by: i64,
        condition: Option<PortableConditionExpr>,
    },
}

/// The sibling instructions a `RegisterPdaMapping` reads from: every
/// instruction of `instruction_type` in the registering instruction's
/// transaction, whichever arrives first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortableSiblingSelector {
    pub instruction_type: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pda_from_sibling: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub seed_from_sibling: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableConditionExpr {
    pub expression: String,
    pub parsed: Option<PortableParsedCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PortableParsedCondition {
    Comparison {
        field: PortableFieldPath,
        op: PortableComparisonOp,
        value: Value,
    },
    Logical {
        op: PortableLogicalOp,
        conditions: Vec<Self>,
    },
    /// Write-gating condition (v2): a boolean expression over the incoming
    /// update. The mapping applies only when it is `true`, for every
    /// population strategy. Runtimes without it reject the artifact.
    Guard { expr: PortableComputedExpr },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PortableComparisonOp {
    Equal,
    NotEqual,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PortableLogicalOp {
    And,
    Or,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PortableSortOrder {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PortableCompareOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PortablePredicateValue {
    Literal(Value),
    Dynamic(String),
    Field(PortableFieldPath),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PortablePredicate {
    Compare {
        field: PortableFieldPath,
        op: PortableCompareOp,
        value: PortablePredicateValue,
    },
    And(Vec<Self>),
    Or(Vec<Self>),
    Not(Box<Self>),
    Exists {
        field: PortableFieldPath,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PortableViewTransform {
    Filter {
        predicate: PortablePredicate,
    },
    Sort {
        key: PortableFieldPath,
        #[serde(default)]
        order: PortableSortOrder,
    },
    Take {
        count: usize,
    },
    Skip {
        count: usize,
    },
    First,
    Last,
    MaxBy {
        key: PortableFieldPath,
    },
    MinBy {
        key: PortableFieldPath,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PortableViewSource {
    Entity { name: String },
    View { id: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub enum PortableViewOutput {
    #[default]
    Collection,
    Single,
    Keyed {
        key_field: PortableFieldPath,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableView {
    pub id: String,
    pub source: PortableViewSource,
    #[serde(default)]
    pub pipeline: Vec<PortableViewTransform>,
    #[serde(default)]
    pub output: PortableViewOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableEntity {
    pub state_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    pub identity: PortableIdentitySpec,
    pub handlers: Vec<PortableHandlerSpec>,
    pub sections: Vec<PortableEntitySection>,
    pub field_mappings: BTreeMap<String, PortableFieldTypeInfo>,
    pub resolver_hooks: Vec<PortableResolverHook>,
    pub instruction_hooks: Vec<PortableInstructionHook>,
    #[serde(default)]
    pub resolver_specs: Vec<PortableResolverSpec>,
    #[serde(default)]
    pub computed_fields: Vec<String>,
    #[serde(default)]
    pub computed_field_specs: Vec<PortableComputedFieldSpec>,
    #[serde(default)]
    pub views: Vec<PortableView>,
}

impl PortableEntity {
    pub fn new(state_name: impl Into<String>, primary_key: impl Into<String>) -> Self {
        Self {
            state_name: state_name.into(),
            program_id: None,
            identity: PortableIdentitySpec {
                primary_keys: vec![primary_key.into()],
                lookup_indexes: Vec::new(),
            },
            handlers: Vec::new(),
            sections: Vec::new(),
            field_mappings: BTreeMap::new(),
            resolver_hooks: Vec::new(),
            instruction_hooks: Vec::new(),
            resolver_specs: Vec::new(),
            computed_fields: Vec::new(),
            computed_field_specs: Vec::new(),
            views: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), ArtifactError> {
        if self.state_name.is_empty() {
            return Err(ArtifactError::InvalidArtifact(
                "entity stateName must not be empty".to_string(),
            ));
        }
        let primary_keys = self
            .identity
            .primary_keys
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if primary_keys.len() != self.identity.primary_keys.len() || primary_keys.contains("") {
            return Err(ArtifactError::InvalidArtifact(format!(
                "entity '{}' primary keys must be unique and non-empty",
                self.state_name
            )));
        }
        let mut view_ids = BTreeSet::new();
        for view in &self.views {
            if view.id.is_empty() || !view_ids.insert(view.id.as_str()) {
                return Err(ArtifactError::InvalidArtifact(format!(
                    "entity '{}' contains an empty or duplicate view ID",
                    self.state_name
                )));
            }
            if !matches!(
                &view.source,
                PortableViewSource::Entity { name } if name == &self.state_name
            ) && !matches!(&view.source, PortableViewSource::View { .. })
            {
                return Err(ArtifactError::InvalidArtifact(format!(
                    "view '{}' references a different entity",
                    view.id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramRequirementV2 {
    pub program_id: String,
    pub program_spec_hash: HashId<ProgramSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionResolutionAdapterV2 {
    pub instruction: String,
    pub accounts: BTreeMap<String, AccountResolutionV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramAdapterV2 {
    pub program_spec_hash: HashId<ProgramSpec>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pdas: BTreeMap<String, PdaDefinitionV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instruction_resolutions: Vec<InstructionResolutionAdapterV2>,
}

/// The largest PDA reverse lookup capacity a LiveSpec may request.
pub const MAX_LIVE_PDA_REVERSE_LOOKUP_ENTRIES: u64 = 1_000_000;

/// Per-stack runtime capacity overrides. Every field is optional and an
/// absent field keeps the runtime default; an empty `limits` is rejected so a
/// LiveSpec has one canonical form.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveLimitsV2 {
    /// Entries each PDA reverse lookup table keeps (`register_from`
    /// registrations) before evicting the least recently used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pda_reverse_lookup_entries: Option<u64>,
}

impl LiveLimitsV2 {
    pub fn validate(&self) -> Result<(), ArtifactError> {
        let Some(entries) = self.pda_reverse_lookup_entries else {
            return Err(ArtifactError::InvalidArtifact(
                "live limits must set at least one capacity".to_string(),
            ));
        };
        if !(1..=MAX_LIVE_PDA_REVERSE_LOOKUP_ENTRIES).contains(&entries) {
            return Err(ArtifactError::InvalidArtifact(format!(
                "pdaReverseLookupEntries must be between 1 and {MAX_LIVE_PDA_REVERSE_LOOKUP_ENTRIES}"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveSpecV2 {
    pub schema: String,
    pub compiler_contract_version: String,
    pub wire_contract_version: String,
    pub programs: Vec<ProgramRequirementV2>,
    pub entities: Vec<PortableEntity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub program_adapters: Vec<ProgramAdapterV2>,
    /// Runtime capacity overrides. Omitted when the stack sets none, so
    /// LiveSpecs without them keep their bytes and hashes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<LiveLimitsV2>,
    /// Row scoping: entity name -> dotted path of the string field that
    /// holds each row's scope id. A runtime delivers a row of a listed entity
    /// only to sessions whose row scopes cover it; rows with an empty scope
    /// are public. Omitted when no entity is scoped, so LiveSpecs without it
    /// keep their bytes and hashes. Runtimes that predate it reject a
    /// LiveSpec carrying it (`deny_unknown_fields`) rather than serve scoped
    /// rows to everyone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_scopes: Option<BTreeMap<String, String>>,
}

/// The longest scope field path a LiveSpec may name, in bytes.
pub const MAX_LIVE_ROW_SCOPE_PATH_BYTES: usize = 256;

impl LiveSpecV2 {
    pub fn new(
        programs: Vec<ProgramRequirementV2>,
        entities: Vec<PortableEntity>,
        program_adapters: Vec<ProgramAdapterV2>,
    ) -> Self {
        Self {
            schema: LIVE_SPEC_SCHEMA_V2.to_string(),
            compiler_contract_version: LIVE_COMPILER_CONTRACT_V1.to_string(),
            wire_contract_version: LIVE_WIRE_CONTRACT_V1.to_string(),
            programs,
            entities,
            program_adapters,
            limits: None,
            row_scopes: None,
        }
    }

    pub fn validate(&self) -> Result<(), ArtifactError> {
        if self.schema != LIVE_SPEC_SCHEMA_V2 {
            return Err(ArtifactError::UnsupportedVersion {
                artifact: LIVE_SPEC_KIND,
                version: self.schema.clone(),
            });
        }
        if self.compiler_contract_version.is_empty() || self.wire_contract_version.is_empty() {
            return Err(ArtifactError::InvalidArtifact(
                "live compiler and wire contract versions must not be empty".to_string(),
            ));
        }
        if let Some(limits) = &self.limits {
            limits.validate()?;
        }
        let mut program_hashes = BTreeSet::new();
        for program in &self.programs {
            if program.program_id.is_empty()
                || !program_hashes.insert(program.program_spec_hash.to_string())
            {
                return Err(ArtifactError::InvalidArtifact(
                    "live program requirements must have unique hashes and non-empty program IDs"
                        .to_string(),
                ));
            }
        }
        let mut entity_names = BTreeSet::new();
        let mut all_view_ids = BTreeSet::new();
        for entity in &self.entities {
            entity.validate()?;
            if !entity_names.insert(entity.state_name.as_str()) {
                return Err(ArtifactError::InvalidArtifact(format!(
                    "duplicate entity '{}'",
                    entity.state_name
                )));
            }
            for view in &entity.views {
                if !all_view_ids.insert(view.id.as_str()) {
                    return Err(ArtifactError::InvalidArtifact(format!(
                        "duplicate LiveSpec view ID '{}'",
                        view.id
                    )));
                }
            }
        }
        if let Some(row_scopes) = &self.row_scopes {
            self.validate_row_scopes(row_scopes)?;
        }
        let mut adapter_hashes = BTreeSet::new();
        for adapter in &self.program_adapters {
            if !program_hashes.contains(&adapter.program_spec_hash.to_string())
                || !adapter_hashes.insert(adapter.program_spec_hash.to_string())
            {
                return Err(ArtifactError::InvalidArtifact(
                    "program adapters must uniquely reference a required ProgramSpec".to_string(),
                ));
            }
            let mut instruction_names = BTreeSet::new();
            for instruction in &adapter.instruction_resolutions {
                if instruction.instruction.is_empty()
                    || instruction.accounts.is_empty()
                    || !instruction_names.insert(instruction.instruction.as_str())
                {
                    return Err(ArtifactError::InvalidArtifact(
                        "instruction adapters must have unique names and non-empty account refinements"
                            .to_string(),
                    ));
                }
            }
        }
        reject_private_fields(&serde_json::to_value(self).map_err(json_error)?)
    }

    fn validate_row_scopes(
        &self,
        row_scopes: &BTreeMap<String, String>,
    ) -> Result<(), ArtifactError> {
        if row_scopes.is_empty() {
            return Err(ArtifactError::InvalidArtifact(
                "rowScopes must name at least one entity".to_string(),
            ));
        }
        for (entity_name, path) in row_scopes {
            let entity = self
                .entities
                .iter()
                .find(|entity| &entity.state_name == entity_name)
                .ok_or_else(|| {
                    ArtifactError::InvalidArtifact(format!(
                        "rowScopes names unknown entity '{entity_name}'"
                    ))
                })?;
            if path.is_empty()
                || path.len() > MAX_LIVE_ROW_SCOPE_PATH_BYTES
                || path.split('.').any(str::is_empty)
            {
                return Err(ArtifactError::InvalidArtifact(format!(
                    "rowScopes path for '{entity_name}' must be a dotted field path"
                )));
            }
            let field = entity.field_mappings.get(path).ok_or_else(|| {
                ArtifactError::InvalidArtifact(format!(
                    "rowScopes path '{path}' is not a field of '{entity_name}'"
                ))
            })?;
            if field.base_type != PortableBaseType::String || field.is_array {
                return Err(ArtifactError::InvalidArtifact(format!(
                    "rowScopes field '{entity_name}.{path}' must be a string"
                )));
            }
        }
        Ok(())
    }

    pub fn view_ids(&self) -> BTreeSet<&str> {
        self.entities
            .iter()
            .flat_map(|entity| entity.views.iter().map(|view| view.id.as_str()))
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveSpecArtifactV2 {
    pub artifact_version: String,
    pub kind: String,
    pub artifact_hash: HashId<LiveSpec>,
    pub payload: LiveSpecV2,
}

impl LiveSpecArtifactV2 {
    pub fn new(payload: LiveSpecV2) -> Result<Self, ArtifactError> {
        payload.validate()?;
        let artifact_hash = hash_jcs(&LiveSpecProjection {
            artifact_version: ARTIFACT_VERSION_V1,
            kind: LIVE_SPEC_KIND,
            payload: &payload,
        })?;
        Ok(Self {
            artifact_version: ARTIFACT_VERSION_V1.to_string(),
            kind: LIVE_SPEC_KIND.to_string(),
            artifact_hash,
            payload,
        })
    }

    pub fn validate(&self) -> Result<(), ArtifactError> {
        validate_envelope_version(&self.artifact_version, LIVE_SPEC_KIND)?;
        validate_kind(&self.kind, LIVE_SPEC_KIND)?;
        self.payload.validate()?;
        let expected = hash_jcs(&LiveSpecProjection {
            artifact_version: &self.artifact_version,
            kind: LIVE_SPEC_KIND,
            payload: &self.payload,
        })?;
        if expected != self.artifact_hash {
            return Err(ArtifactError::HashMismatch);
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ArtifactError> {
        self.validate()?;
        arete_hash::canonicalize_jcs(self).map_err(Into::into)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveSpecProjection<'a> {
    artifact_version: &'a str,
    kind: &'static str,
    payload: &'a LiveSpecV2,
}

pub fn load_live_spec_v2(
    bytes: &[u8],
) -> Result<crate::LoadedArtifact<LiveSpecArtifactV2>, ArtifactError> {
    let value = arete_hash::parse_json_bytes_strict(bytes)?;
    let artifact: LiveSpecArtifactV2 = serde_json::from_value(value).map_err(json_error)?;
    artifact.validate()?;
    Ok(crate::LoadedArtifact {
        artifact,
        original_bytes: bytes.to_vec(),
        source_hash: arete_hash::hash_raw_bytes(bytes)?,
    })
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_false(value: &bool) -> bool {
    !*value
}
