//! Serializable AST types for hyperstack transform pipelines.
//!
//! These types define the intermediate representation used for:
//! - Compile-time AST serialization (from `#[stream_spec]`)
//! - AST-based compilation (via `#[ast_spec]`)
//! - Cross-crate communication (transform-macros -> transform)
//!
//! Note: These types are duplicated from `hyperstack_interpreter::ast` because proc-macro
//! crates cannot depend on their output crates (circular dependency).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use hyperstack_idl::snapshot::*;

// ============================================================================
// Core AST Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FieldPath {
    pub segments: Vec<String>,
    pub offsets: Option<Vec<usize>>,
}

impl FieldPath {
    pub fn new(segments: &[&str]) -> Self {
        FieldPath {
            segments: segments.iter().map(|s| s.to_string()).collect(),
            offsets: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Transformation {
    HexEncode,
    HexDecode,
    Base58Encode,
    Base58Decode,
    ToString,
    ToNumber,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PopulationStrategy {
    SetOnce,
    LastWrite,
    Append,
    Merge,
    Max,
    /// Sum numeric values (accumulator pattern for aggregations)
    Sum,
    /// Count occurrences (increments by 1 for each update)
    Count,
    /// Track minimum value
    Min,
    /// Track unique values and store the count
    /// Internally maintains a HashSet, exposes only the count
    UniqueCount,
}

/// Default discriminant size (8 bytes for Anchor).
/// Used by InstructionDef serde default.
fn default_discriminant_size() -> usize {
    8
}
// ============================================================================
// Computed Field Expression AST
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputedFieldSpec {
    pub target_path: String,
    pub expression: ComputedExpr,
    pub result_type: String,
}

// ==========================================================================
// Resolver Specifications
// ==========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ResolverType {
    Token,
    Url(UrlResolverConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum HttpMethod {
    #[default]
    Get,
    Post,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum UrlTemplatePart {
    Literal(String),
    FieldRef(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum UrlSource {
    FieldPath(String),
    Template(Vec<UrlTemplatePart>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct UrlResolverConfig {
    pub url_source: UrlSource,
    #[serde(default)]
    pub method: HttpMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolverExtractSpec {
    pub target_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transformation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ResolveStrategy {
    #[default]
    SetOnce,
    LastWrite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolverCondition {
    pub field_path: String,
    pub op: ComparisonOp,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolverSpec {
    pub resolver: ResolverType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_value: Option<serde_json::Value>,
    #[serde(default)]
    pub strategy: ResolveStrategy,
    pub extracts: Vec<ResolverExtractSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<ResolverCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComputedExpr {
    // Existing variants
    FieldRef {
        path: String,
    },
    UnwrapOr {
        expr: Box<ComputedExpr>,
        default: serde_json::Value,
    },
    Binary {
        op: BinaryOp,
        left: Box<ComputedExpr>,
        right: Box<ComputedExpr>,
    },
    Cast {
        expr: Box<ComputedExpr>,
        to_type: String,
    },
    MethodCall {
        expr: Box<ComputedExpr>,
        method: String,
        args: Vec<ComputedExpr>,
    },
    ResolverComputed {
        resolver: String,
        method: String,
        args: Vec<ComputedExpr>,
    },
    Literal {
        value: serde_json::Value,
    },
    Paren {
        expr: Box<ComputedExpr>,
    },

    // Variable reference (for let bindings)
    Var {
        name: String,
    },

    // Let binding: let name = value; body
    Let {
        name: String,
        value: Box<ComputedExpr>,
        body: Box<ComputedExpr>,
    },

    // Conditional: if condition { then_branch } else { else_branch }
    If {
        condition: Box<ComputedExpr>,
        then_branch: Box<ComputedExpr>,
        else_branch: Box<ComputedExpr>,
    },

    // Option constructors
    None,
    Some {
        value: Box<ComputedExpr>,
    },

    // Byte/array operations
    Slice {
        expr: Box<ComputedExpr>,
        start: usize,
        end: usize,
    },
    Index {
        expr: Box<ComputedExpr>,
        index: usize,
    },

    // Byte conversion functions
    U64FromLeBytes {
        bytes: Box<ComputedExpr>,
    },
    U64FromBeBytes {
        bytes: Box<ComputedExpr>,
    },

    // Byte array literals: [0u8; 32] or [1, 2, 3]
    ByteArray {
        bytes: Vec<u8>,
    },

    // Closure for map operations: |x| body
    Closure {
        param: String,
        body: Box<ComputedExpr>,
    },

    // Unary operations
    Unary {
        op: UnaryOp,
        expr: Box<ComputedExpr>,
    },

    // JSON array to bytes conversion (for working with captured byte arrays)
    JsonToBytes {
        expr: Box<ComputedExpr>,
    },

    // Context access - slot and timestamp from the update that triggered evaluation
    ContextSlot,
    ContextTimestamp,

    /// Keccak256 hash function for computing Ethereum-compatible hashes
    /// Takes a byte array expression and returns the 32-byte hash as a Vec<u8>
    Keccak256 {
        expr: Box<ComputedExpr>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BinaryOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    // Comparison
    Gt,
    Lt,
    Gte,
    Lte,
    Eq,
    Ne,
    // Logical
    And,
    Or,
    // Bitwise
    Xor,
    BitAnd,
    BitOr,
    Shl,
    Shr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UnaryOp {
    Not,
    ReverseBits,
}

// ============================================================================
// Stream Specification Types
// ============================================================================

/// Current AST version for SerializableStreamSpec and SerializableStackSpec
///
/// ⚠️ IMPORTANT: This constant is duplicated in interpreter/src/ast.rs due to
/// circular dependency between proc-macro crates and their output crates.
/// When bumping this version, you MUST also update the constant in the
/// interpreter crate. A test in versioned.rs verifies they stay in sync.
pub const CURRENT_AST_VERSION: &str = "0.0.1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableStreamSpec {
    /// AST schema version for backward compatibility
    /// Uses semver format (e.g., "0.0.1")
    #[serde(default = "default_ast_version")]
    pub ast_version: String,
    pub state_name: String,
    #[serde(default)]
    pub program_id: Option<String>,
    #[serde(default)]
    pub idl: Option<IdlSnapshot>,
    pub identity: IdentitySpec,
    pub handlers: Vec<SerializableHandlerSpec>,
    pub sections: Vec<EntitySection>,
    pub field_mappings: BTreeMap<String, FieldTypeInfo>,
    pub resolver_hooks: Vec<ResolverHook>,
    pub instruction_hooks: Vec<InstructionHook>,
    #[serde(default)]
    pub resolver_specs: Vec<ResolverSpec>,
    #[serde(default)]
    pub computed_fields: Vec<String>,
    #[serde(default)]
    pub computed_field_specs: Vec<ComputedFieldSpec>,
    /// Deterministic content hash (SHA256 of canonical JSON, excluding this field)
    /// Used for deduplication and version tracking
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub views: Vec<ViewDef>,
}

fn default_ast_version() -> String {
    CURRENT_AST_VERSION.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentitySpec {
    pub primary_keys: Vec<String>,
    pub lookup_indexes: Vec<LookupIndexSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookupIndexSpec {
    pub field_name: String,
    pub temporal_field: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableHandlerSpec {
    pub source: SourceSpec,
    pub key_resolution: KeyResolutionStrategy,
    pub mappings: Vec<SerializableFieldMapping>,
    pub conditions: Vec<Condition>,
    pub emit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyResolutionStrategy {
    Embedded {
        primary_field: FieldPath,
    },
    Lookup {
        primary_field: FieldPath,
    },
    Computed {
        primary_field: FieldPath,
        compute_partition: ComputeFunction,
    },
    TemporalLookup {
        lookup_field: FieldPath,
        timestamp_field: FieldPath,
        index_name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SourceSpec {
    Source {
        program_id: Option<String>,
        discriminator: Option<Vec<u8>>,
        type_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        serialization: Option<IdlSerializationSnapshot>,
        /// True when this handler listens to an account-state event.
        #[serde(default)]
        is_account: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableFieldMapping {
    pub target_path: String,
    pub source: MappingSource,
    pub transform: Option<Transformation>,
    pub population: PopulationStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<ConditionExpr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<String>,
    #[serde(default = "default_emit", skip_serializing_if = "is_true")]
    pub emit: bool,
}

fn default_emit() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MappingSource {
    FromSource {
        path: FieldPath,
        default: Option<serde_json::Value>,
        transform: Option<Transformation>,
    },
    Constant(serde_json::Value),
    Computed {
        inputs: Vec<FieldPath>,
        function: ComputeFunction,
    },
    FromState {
        path: String,
    },
    AsEvent {
        fields: Vec<MappingSource>,
    },
    WholeSource,
    AsCapture {
        field_transforms: std::collections::BTreeMap<String, Transformation>,
    },
    FromContext {
        field: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComputeFunction {
    Sum,
    Concat,
    Format(String),
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    // Simplified for now, expand as needed
}

/// Represents a logical section/group of fields in the entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySection {
    pub name: String,
    pub fields: Vec<FieldTypeInfo>,
    #[serde(default)]
    pub is_nested_struct: bool,
    #[serde(default)]
    pub parent_field: Option<String>,
}

/// Language-agnostic type information for fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldTypeInfo {
    pub field_name: String,
    pub rust_type_name: String,
    pub base_type: BaseType,
    pub is_optional: bool,
    pub is_array: bool,
    #[serde(default)]
    pub inner_type: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    /// Resolved type information for complex types (instructions, accounts, custom types)
    #[serde(default)]
    pub resolved_type: Option<ResolvedStructType>,
    #[serde(default = "default_emit", skip_serializing_if = "is_true")]
    pub emit: bool,
}

/// Resolved structure type with field information from IDL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedStructType {
    pub type_name: String,
    pub fields: Vec<ResolvedField>,
    pub is_instruction: bool,
    pub is_account: bool,
    pub is_event: bool,
    /// If true, this is an enum type and enum_variants should be used instead of fields
    #[serde(default)]
    pub is_enum: bool,
    /// For enum types, list of variant names
    #[serde(default)]
    pub enum_variants: Vec<String>,
}

/// A resolved field within a complex type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedField {
    pub field_name: String,
    pub field_type: String,
    pub base_type: BaseType,
    pub is_optional: bool,
    pub is_array: bool,
}

/// Language-agnostic base type classification
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BaseType {
    Integer,
    Float,
    String,
    Boolean,
    Object,
    Array,
    Binary,
    Timestamp,
    Pubkey, // Solana public key (Base58 encoded)
    Any,
}

// ============================================================================
// Level 1: Declarative Hook Extensions
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolverHook {
    pub account_type: String,
    pub strategy: ResolverStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResolverStrategy {
    PdaReverseLookup {
        lookup_name: String,
        queue_discriminators: Vec<Vec<u8>>,
    },
    DirectField {
        field_path: FieldPath,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionHook {
    pub instruction_type: String,
    pub actions: Vec<HookAction>,
    pub lookup_by: Option<FieldPath>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HookAction {
    RegisterPdaMapping {
        pda_field: FieldPath,
        seed_field: FieldPath,
        lookup_name: String,
    },
    SetField {
        target_field: String,
        source: MappingSource,
        condition: Option<ConditionExpr>,
    },
    IncrementField {
        target_field: String,
        increment_by: i64,
        condition: Option<ConditionExpr>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionExpr {
    pub expression: String,
    pub parsed: Option<ParsedCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParsedCondition {
    Comparison {
        field: FieldPath,
        op: ComparisonOp,
        value: serde_json::Value,
    },
    Logical {
        op: LogicalOp,
        conditions: Vec<ParsedCondition>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComparisonOp {
    Equal,
    NotEqual,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogicalOp {
    And,
    Or,
}

// ============================================================================
// View Pipeline Types - Composable View Definitions
// ============================================================================

/// Sort order for view transforms
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

/// Comparison operators for predicates
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
}

/// Value in a predicate comparison
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PredicateValue {
    /// Literal JSON value
    Literal(serde_json::Value),
    /// Dynamic runtime value (e.g., "now()" for current timestamp)
    Dynamic(String),
    /// Reference to another field
    Field(FieldPath),
}

/// Predicate for filtering entities
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Predicate {
    /// Field comparison: field op value
    Compare {
        field: FieldPath,
        op: CompareOp,
        value: PredicateValue,
    },
    /// Logical AND of predicates
    And(Vec<Predicate>),
    /// Logical OR of predicates
    Or(Vec<Predicate>),
    /// Negation
    Not(Box<Predicate>),
    /// Field exists (is not null)
    Exists { field: FieldPath },
}

/// Transform operation in a view pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ViewTransform {
    /// Filter entities matching a predicate
    Filter { predicate: Predicate },

    /// Sort entities by a field
    Sort {
        key: FieldPath,
        #[serde(default)]
        order: SortOrder,
        #[serde(skip, default)]
        key_span: Option<proc_macro2::Span>,
    },

    /// Take first N entities (after sort)
    Take { count: usize },

    /// Skip first N entities
    Skip { count: usize },

    /// Take only the first entity (after sort) - produces Single output
    First,

    /// Take only the last entity (after sort) - produces Single output
    Last,

    /// Get entity with maximum value for field - produces Single output
    MaxBy {
        key: FieldPath,
        #[serde(skip, default)]
        key_span: Option<proc_macro2::Span>,
    },

    /// Get entity with minimum value for field - produces Single output
    MinBy {
        key: FieldPath,
        #[serde(skip, default)]
        key_span: Option<proc_macro2::Span>,
    },
}

impl PartialEq for ViewTransform {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Filter { predicate: l }, Self::Filter { predicate: r }) => l == r,
            (
                Self::Sort {
                    key: k1, order: o1, ..
                },
                Self::Sort {
                    key: k2, order: o2, ..
                },
            ) => k1 == k2 && o1 == o2,
            (Self::Take { count: l }, Self::Take { count: r }) => l == r,
            (Self::Skip { count: l }, Self::Skip { count: r }) => l == r,
            (Self::First, Self::First) => true,
            (Self::Last, Self::Last) => true,
            (Self::MaxBy { key: k1, .. }, Self::MaxBy { key: k2, .. }) => k1 == k2,
            (Self::MinBy { key: k1, .. }, Self::MinBy { key: k2, .. }) => k1 == k2,
            _ => false,
        }
    }
}

impl Eq for ViewTransform {}

/// Source for a view definition
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ViewSource {
    /// Derive directly from entity mutations
    Entity { name: String },
    /// Derive from another view's output
    View { id: String },
}

/// Output mode for a view
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub enum ViewOutput {
    /// Multiple entities (list-like semantics)
    #[default]
    Collection,
    /// Single entity (state-like semantics)
    Single,
    /// Keyed lookup by a specific field
    Keyed { key_field: FieldPath },
}

/// Definition of a view in the pipeline
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ViewDef {
    /// Unique view identifier (e.g., "OreRound/latest")
    pub id: String,

    /// Source this view derives from
    pub source: ViewSource,

    /// Pipeline of transforms to apply (in order)
    #[serde(default)]
    pub pipeline: Vec<ViewTransform>,

    /// Output mode for this view
    #[serde(default)]
    pub output: ViewOutput,
}

// ============================================================================
// SerializableStreamSpec Implementation
// ============================================================================

impl SerializableStreamSpec {
    /// Compute deterministic content hash (SHA256 of canonical JSON).
    ///
    /// The hash is computed over the entire spec except the content_hash field itself,
    /// ensuring the same AST always produces the same hash regardless of when it was
    /// generated or by whom.
    #[allow(dead_code)]
    pub fn try_compute_content_hash(&self) -> Result<String, serde_json::Error> {
        use sha2::{Digest, Sha256};

        let mut spec_for_hash = self.clone();
        spec_for_hash.content_hash = None;

        let json = serde_json::to_string(&spec_for_hash)?;

        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        let result = hasher.finalize();

        Ok(hex::encode(result))
    }

    #[allow(dead_code)]
    pub fn compute_content_hash(&self) -> String {
        self.try_compute_content_hash()
            .expect("Failed to serialize spec for hashing")
    }

    /// Verify that the content_hash matches the computed hash.
    /// Returns true if hash is valid or not set.
    #[allow(dead_code)]
    pub fn verify_content_hash(&self) -> bool {
        match &self.content_hash {
            Some(hash) => self
                .try_compute_content_hash()
                .map(|computed| hash == &computed)
                .unwrap_or(false),
            None => true, // No hash to verify
        }
    }

    /// Set the content_hash field to the computed hash.
    #[allow(dead_code)]
    pub fn try_with_content_hash(mut self) -> Result<Self, serde_json::Error> {
        self.content_hash = Some(self.try_compute_content_hash()?);
        Ok(self)
    }

    #[allow(dead_code)]
    pub fn with_content_hash(mut self) -> Self {
        self.content_hash = Some(self.compute_content_hash());
        self
    }
}

// ============================================================================
// PDA and Instruction Types — For SDK code generation
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdaDefinition {
    pub name: String,
    pub seeds: Vec<PdaSeedDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PdaSeedDef {
    Literal {
        value: String,
    },
    Bytes {
        value: Vec<u8>,
    },
    ArgRef {
        arg_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arg_type: Option<String>,
    },
    AccountRef {
        account_name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "category", rename_all = "camelCase")]
pub enum AccountResolution {
    Signer,
    Known {
        address: String,
    },
    PdaRef {
        pda_name: String,
    },
    PdaInline {
        seeds: Vec<PdaSeedDef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        program_id: Option<String>,
    },
    UserProvided,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstructionAccountDef {
    pub name: String,
    #[serde(default)]
    pub is_signer: bool,
    #[serde(default)]
    pub is_writable: bool,
    pub resolution: AccountResolution,
    #[serde(default)]
    pub is_optional: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstructionArgDef {
    pub name: String,
    #[serde(rename = "type")]
    pub arg_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstructionDef {
    pub name: String,
    pub discriminator: Vec<u8>,
    #[serde(default = "default_discriminant_size")]
    pub discriminator_size: usize,
    pub accounts: Vec<InstructionAccountDef>,
    pub args: Vec<InstructionArgDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<IdlErrorSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<String>,
}

// ============================================================================
// Stack Spec — Unified multi-entity AST format
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableStackSpec {
    /// AST schema version for backward compatibility
    /// Uses semver format (e.g., "0.0.1")
    #[serde(default = "default_ast_version")]
    pub ast_version: String,
    pub stack_name: String,
    #[serde(default)]
    pub program_ids: Vec<String>,
    #[serde(default)]
    pub idls: Vec<IdlSnapshot>,
    pub entities: Vec<SerializableStreamSpec>,
    /// Outer key = program name, inner key = PDA name
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pdas: BTreeMap<String, BTreeMap<String, PdaDefinition>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instructions: Vec<InstructionDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

impl SerializableStackSpec {
    /// Compute deterministic content hash (SHA256 of canonical JSON).
    #[allow(dead_code)]
    pub fn try_compute_content_hash(&self) -> Result<String, serde_json::Error> {
        use sha2::{Digest, Sha256};

        let mut spec_for_hash = self.clone();
        spec_for_hash.content_hash = None;
        let json = serde_json::to_string(&spec_for_hash)?;
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        Ok(hex::encode(hasher.finalize()))
    }

    #[allow(dead_code)]
    pub fn compute_content_hash(&self) -> String {
        self.try_compute_content_hash()
            .expect("Failed to serialize stack spec for hashing")
    }

    #[allow(dead_code)]
    pub fn try_with_content_hash(mut self) -> Result<Self, serde_json::Error> {
        self.content_hash = Some(self.try_compute_content_hash()?);
        Ok(self)
    }

    #[allow(dead_code)]
    pub fn with_content_hash(mut self) -> Self {
        self.content_hash = Some(self.compute_content_hash());
        self
    }
}
