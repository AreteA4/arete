use crate::ast::*;
use crate::identifiers::{rust as rust_ident, IdentifierCase, IdentifierScope};
use crate::idl_models::{
    bind_idl_models, DeclaredModel, IdlModel, IdlModelKind, ModelField, ModelLanguage, WireType,
};
use crate::stack_types::{
    account_reader_programs, entity_program_name, resolved_type_namespaces, AccountModels,
    ProgramTypeDefs, StackResolvedTypes,
};
use crate::typescript_instructions::{
    dedupe_errors_by_code, disambiguate_instruction_account_names, normalize_seed_arg_type,
    split_generic,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct RustOutput {
    pub cargo_toml: String,
    pub lib_rs: String,
    pub types_rs: String,
    pub entity_rs: String,
    /// Generated program SDK module (`programs.rs`). `None` when the stack
    /// spec declares no instructions.
    pub programs_rs: Option<String>,
    /// The program modules `programs.rs` declares, in order. A program
    /// package extension configured in
    /// [`RustStackConfig::program_extensions`] is staged under
    /// `programs/<module_name>/`, next to `programs.rs`.
    pub program_modules: Vec<RustProgramModule>,
}

/// One program module of a generated `programs.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustProgramModule {
    pub program_id: String,
    pub module_name: String,
}

/// Rust output for a standalone program SDK.
///
/// Unlike [`RustOutput`], this deliberately has no entity/view module or
/// generated `Stack` implementation. The exported
/// aggregate implements `arete_sdk::ProgramSdk`, so it can be connected on
/// its own or added to a session alongside live stacks.
#[derive(Debug, Clone)]
pub struct RustProgramOutput {
    pub cargo_toml: String,
    pub lib_rs: String,
    pub types_rs: String,
    pub programs_rs: String,
}

impl RustOutput {
    pub fn full_lib(&self) -> String {
        let mut output = format!(
            "{}\n\n// types.rs\n{}\n\n// entity.rs\n{}",
            self.lib_rs, self.types_rs, self.entity_rs
        );
        if let Some(programs) = &self.programs_rs {
            output.push_str("\n\n// programs.rs\n");
            output.push_str(programs);
        }
        output
    }

    pub fn mod_rs(&self) -> String {
        self.lib_rs.clone()
    }
}

/// The `arete-a4-sdk` dependency version emitted into generated Rust
/// program and stack crates.
///
/// `arete-interpreter` and `arete-a4-sdk` are released together in the
/// `arete` linked-version group (`release-please-config.json`), so the
/// interpreter's own package version is the SDK version a generated crate
/// must build against. CI enforces that linkage with
/// `scripts/check-linked-release-versions.mjs`; callers that deliberately
/// need a different version still override `sdk_version` explicitly.
pub const GENERATED_RUST_SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct RustConfig {
    pub crate_name: String,
    pub sdk_version: String,
    pub module_mode: bool,
    /// WebSocket URL for the stack. If None, generates a placeholder comment.
    pub url: Option<String>,
}

impl Default for RustConfig {
    fn default() -> Self {
        Self {
            crate_name: "generated-stack".to_string(),
            sdk_version: GENERATED_RUST_SDK_VERSION.to_string(),
            module_mode: false,
            url: None,
        }
    }
}

pub fn compile_serializable_spec(
    spec: SerializableStreamSpec,
    entity_name: String,
    config: Option<RustConfig>,
) -> Result<RustOutput, String> {
    let config = config.unwrap_or_default();
    let compiler = RustCompiler::new(spec, entity_name, config);
    Ok(compiler.compile())
}

pub fn write_rust_crate(
    output: &RustOutput,
    crate_dir: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(crate_dir.join("src"))?;
    std::fs::write(crate_dir.join("Cargo.toml"), &output.cargo_toml)?;
    std::fs::write(crate_dir.join("src/lib.rs"), &output.lib_rs)?;
    std::fs::write(crate_dir.join("src/types.rs"), &output.types_rs)?;
    std::fs::write(crate_dir.join("src/entity.rs"), &output.entity_rs)?;
    if let Some(programs) = &output.programs_rs {
        std::fs::write(crate_dir.join("src/programs.rs"), programs)?;
    }
    Ok(())
}

pub fn write_rust_module(
    output: &RustOutput,
    module_dir: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(module_dir)?;
    std::fs::write(module_dir.join("mod.rs"), output.mod_rs())?;
    std::fs::write(module_dir.join("types.rs"), &output.types_rs)?;
    std::fs::write(module_dir.join("entity.rs"), &output.entity_rs)?;
    if let Some(programs) = &output.programs_rs {
        std::fs::write(module_dir.join("programs.rs"), programs)?;
    }
    Ok(())
}

pub fn write_rust_program_crate(
    output: &RustProgramOutput,
    crate_dir: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(crate_dir.join("src"))?;
    std::fs::write(crate_dir.join("Cargo.toml"), &output.cargo_toml)?;
    std::fs::write(crate_dir.join("src/lib.rs"), &output.lib_rs)?;
    std::fs::write(crate_dir.join("src/types.rs"), &output.types_rs)?;
    std::fs::write(crate_dir.join("src/programs.rs"), &output.programs_rs)?;
    Ok(())
}

pub fn write_rust_program_module(
    output: &RustProgramOutput,
    module_dir: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(module_dir)?;
    std::fs::write(module_dir.join("mod.rs"), &output.lib_rs)?;
    std::fs::write(module_dir.join("types.rs"), &output.types_rs)?;
    std::fs::write(module_dir.join("programs.rs"), &output.programs_rs)?;
    Ok(())
}

/// Runtime envelope a resolved-struct field arrives in. Mirror of the
/// TypeScript generator's `EventWrapper<T>` / `CaptureWrapper<T>` selection in
/// `field_type_info_to_typescript` (and of `python::WrapperKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WrapperKind {
    None,
    Capture,
    Event,
}

/// Target paths fed by an `AsCapture` mapping. Mirror of the TypeScript
/// generator's `is_capture_field` and of `python::capture_field_targets`:
/// those fields arrive wrapped in a `CaptureWrapper` envelope
/// (`{timestamp, account_address, data, slot?, signature?}`) rather than as
/// the bare account struct.
pub(crate) fn capture_field_targets(spec: &SerializableStreamSpec) -> HashSet<String> {
    let mut targets = HashSet::new();
    for handler in &spec.handlers {
        for mapping in &handler.mappings {
            if matches!(&mapping.source, MappingSource::AsCapture { .. }) {
                targets.insert(mapping.target_path.clone());
            }
        }
    }
    targets
}

/// Which runtime envelope a resolved-struct field arrives in. Mirror of the
/// TypeScript generator: `#[capture]`-fed account fields arrive as
/// `CaptureWrapper<T>` and event/instruction-list fields as `EventWrapper<T>`,
/// never as the bare struct.
pub(crate) fn wrapper_kind_for(
    field: &FieldTypeInfo,
    resolved: &ResolvedStructType,
    capture_fields: &HashSet<String>,
) -> WrapperKind {
    if resolved.is_event || (resolved.is_instruction && field.is_array) {
        return WrapperKind::Event;
    }
    if resolved.is_account
        && (capture_fields.contains(&field.field_name)
            || capture_fields.contains(field.raw_field_name()))
    {
        return WrapperKind::Capture;
    }
    WrapperKind::None
}

/// The runtime envelopes every generated `types.rs` carries. Mirrors
/// `arete_interpreter::{EventWrapper, CaptureWrapper}` and the TypeScript
/// `EventWrapper<T>` / `CaptureWrapper<T>` interfaces: capture/event-fed fields
/// arrive wrapped on the wire, so the generated field types name the envelope
/// and expose the provenance (`timestamp`, `account_address`, `slot`,
/// `signature`, `event_index`, `ix_path`) alongside the decoded `data`.
const WRAPPER_TYPES: &str = r#"/// Wrapper for event data that includes context metadata.
/// Events are automatically wrapped in this structure at runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventWrapper<T> {
    /// Unix timestamp when the event was processed.
    #[serde(default, deserialize_with = "serde_utils::deserialize_i64")]
    pub timestamp: i64,
    /// The event-specific data.
    pub data: T,
    /// Optional blockchain slot number.
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub slot: Option<u64>,
    /// Optional transaction signature.
    #[serde(default)]
    pub signature: Option<String>,
    /// Absolute log-line index; set only for events decoded from a log.
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub event_index: Option<u64>,
    /// 0-based instruction path within the transaction (e.g. `"0.1"`).
    #[serde(default)]
    pub ix_path: Option<String>,
}

impl<T: Default> Default for EventWrapper<T> {
    fn default() -> Self {
        Self {
            timestamp: 0,
            data: T::default(),
            slot: None,
            signature: None,
            event_index: None,
            ix_path: None,
        }
    }
}

/// Wrapper for account data captured with `#[capture]`, including context
/// metadata. Captured accounts are automatically wrapped in this structure at
/// runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureWrapper<T> {
    /// Unix timestamp when the account was captured.
    #[serde(default, deserialize_with = "serde_utils::deserialize_i64")]
    pub timestamp: i64,
    /// The account address (base58 encoded public key).
    #[serde(default)]
    pub account_address: String,
    /// The captured account data.
    pub data: T,
    /// Optional blockchain slot number.
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub slot: Option<u64>,
    /// Optional transaction signature.
    #[serde(default)]
    pub signature: Option<String>,
}

impl<T: Default> Default for CaptureWrapper<T> {
    fn default() -> Self {
        Self {
            timestamp: 0,
            account_address: String::new(),
            data: T::default(),
            slot: None,
            signature: None,
        }
    }
}
"#;

/// Rust definitions for the builtin resolver output types a generated SDK can
/// name, in emission order. Mirror of the `typescript_interface()` blocks the
/// resolvers in [`crate::resolvers`] register: `SlotHashBytes` (the
/// `{ bytes }` wire shape of `ResolvedSlotHash`) and `TokenMetadata`. Field
/// names are the snake_case wire keys the runtime emits, so no rename
/// attributes are needed.
///
/// `KeccakRngValue` is deliberately absent even though it is a registered
/// resolver output type. It is a `u64`, and TypeScript models it as
/// `export type KeccakRngValue = string` only because the canonical numeric
/// rule (docs/internal/sdk-core-api.md §2) puts `u64` on the wire as a decimal
/// string. Rust decodes that back to a real `u64` via
/// `serde_utils::deserialize_option_*_u64`, so `KeccakRngValue`-typed fields
/// keep their integer typing instead of degrading to `String`.
const BUILTIN_RESOLVER_STRUCTS: &[(&str, &str)] = &[
    (
        "SlotHashBytes",
        r#"/// Slot hash resolved by the builtin `SlotHash` resolver.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SlotHashBytes {
    /// 32-byte slot hash.
    #[serde(default)]
    pub bytes: Vec<u8>,
}"#,
    ),
    (
        "TokenMetadata",
        r#"/// Token metadata resolved by the builtin `TokenMetadata` resolver.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenMetadata {
    #[serde(default)]
    pub mint: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub decimals: Option<u8>,
    #[serde(default)]
    pub logo_uri: Option<String>,
}"#,
    ),
];

/// The generated struct name for a builtin resolver output type named by a
/// field's `inner_type`, or `None`. The registry is the authority (mirror of
/// `typescript::is_builtin_resolver_type`), narrowed to the types
/// [`BUILTIN_RESOLVER_STRUCTS`] can express as a Rust struct.
pub(crate) fn builtin_resolver_struct(inner_type: Option<&str>) -> Option<&'static str> {
    let inner = inner_type?;
    if !crate::resolvers::is_resolver_output_type(inner) {
        return None;
    }
    BUILTIN_RESOLVER_STRUCTS
        .iter()
        .find(|(name, _)| *name == inner)
        .map(|(name, _)| *name)
}

/// Render the builtin resolver structs a generated `types.rs` actually
/// references. Each block is followed by a blank line.
fn render_builtin_resolver_structs(used: &BTreeSet<&'static str>) -> String {
    let mut output = String::new();
    for (name, definition) in BUILTIN_RESOLVER_STRUCTS {
        if used.contains(name) {
            output.push_str(definition);
            output.push_str("\n\n");
        }
    }
    output
}

/// Map the element of a `Vec<T>` scalar array to its Rust primitive. Mirror of
/// `typescript::typescript_scalar_array_element` and
/// `python::py_scalar_array_element`; accepts both stored forms of the inner
/// type (`"Vec < f64 >"` and the bare `"f64"`) and returns `None` for
/// non-scalar elements.
fn rust_scalar_array_element(inner_type: &str) -> Option<&'static str> {
    let trimmed = inner_type.trim();
    let element = trimmed
        .strip_prefix("Vec <")
        .and_then(|rest| rest.strip_suffix('>'))
        .or_else(|| {
            trimmed
                .strip_prefix("Vec<")
                .and_then(|rest| rest.strip_suffix('>'))
        })
        .map(str::trim)
        .unwrap_or(trimmed);
    match element {
        "f32" | "f64" => Some("f64"),
        "bool" => Some("bool"),
        "String" | "&str" | "str" => Some("String"),
        _ => None,
    }
}

/// Rust type plus `serde_utils` requirement for a non-resolved (scalar /
/// scalar-array) field. Type and `#[serde(...)]` attribute are derived from
/// one place so an integer vector can never be typed `Vec<u64>` while its
/// deserializer stays scalar (or vice versa).
struct RustScalarShape {
    /// The bare Rust type, before the patch `Option<..>` wrapping.
    rust_type: String,
    /// Normalized integer kind whose `serde_utils` deserializer this field
    /// needs, or `None` for a plain `#[serde(default)]`.
    integer_kind: Option<&'static str>,
    /// Whether that deserializer must be the `_vec_` variant.
    is_vec: bool,
}

/// Shape of a non-resolved field. Shared by the entity-section path and the
/// IDL `ResolvedField` path so the same on-chain array is typed identically
/// whichever way it is reached (mirror of `python::py_scalar_field_shape`).
///
/// `Vec<u64>`-shaped fields are stored as `BaseType::Array` with an explicit
/// `integer_kind`, so the integer check has to consult `integer_kind` and not
/// just `base_type`. The guard stays tighter than the TypeScript one
/// (`BaseType::Array` only, never "any field carrying an `integer_kind`") so
/// `BaseType::Binary` fields keep their `Vec<u8>`.
fn rust_scalar_field_shape(
    base_type: &BaseType,
    integer_kind: Option<IntegerKind>,
    is_array: bool,
    inner_type: Option<&str>,
    rust_type_name: &str,
) -> RustScalarShape {
    if is_array && matches!(base_type, BaseType::Array) {
        if let Some(kind) = integer_kind {
            let kind = normalized_integer_kind_of(kind);
            return RustScalarShape {
                rust_type: format!("Vec<{kind}>"),
                integer_kind: Some(kind),
                is_vec: true,
            };
        }
        if let Some(element) = inner_type.and_then(rust_scalar_array_element) {
            return RustScalarShape {
                rust_type: format!("Vec<{element}>"),
                integer_kind: None,
                is_vec: false,
            };
        }
    }

    // Only integer and timestamp types need the string-or-number treatment.
    let kind = match base_type {
        BaseType::Integer => Some(normalized_integer_kind(rust_type_name)),
        BaseType::Timestamp => Some("i64"),
        _ => None,
    };
    let is_vec = is_array && !matches!(base_type, BaseType::Array);
    let base = base_type_to_rust(base_type, rust_type_name);
    RustScalarShape {
        rust_type: if is_vec { format!("Vec<{base}>") } else { base },
        integer_kind: kind,
        is_vec,
    }
}

/// The `serde_utils::deserialize_*` function a shape needs, or `None` when a
/// plain `#[serde(default)]` suffices.
fn deserialize_with_for_shape(shape: &RustScalarShape, is_optional: bool) -> Option<String> {
    let kind = shape.integer_kind?;
    Some(match (is_optional, shape.is_vec) {
        (false, false) => format!("serde_utils::deserialize_option_{kind}"),
        (true, false) => format!("serde_utils::deserialize_option_option_{kind}"),
        (false, true) => format!("serde_utils::deserialize_option_vec_{kind}"),
        (true, true) => format!("serde_utils::deserialize_option_option_vec_{kind}"),
    })
}

fn base_type_to_rust(base_type: &BaseType, rust_type_name: &str) -> String {
    match base_type {
        BaseType::Integer => normalized_integer_kind(rust_type_name).to_string(),
        BaseType::Float => "f64".to_string(),
        BaseType::String => "String".to_string(),
        BaseType::Boolean => "bool".to_string(),
        BaseType::Timestamp => "i64".to_string(),
        BaseType::Binary => "Vec<u8>".to_string(),
        BaseType::Pubkey => "String".to_string(),
        BaseType::Array => "Vec<serde_json::Value>".to_string(),
        BaseType::Object => "serde_json::Value".to_string(),
        BaseType::Any => "serde_json::Value".to_string(),
    }
}

pub(crate) struct RustCompiler {
    spec: SerializableStreamSpec,
    entity_name: String,
    config: RustConfig,
    /// Field targets fed by an `AsCapture` mapping in this entity's handlers.
    capture_fields: HashSet<String>,
}

impl RustCompiler {
    pub(crate) fn new(
        spec: SerializableStreamSpec,
        entity_name: String,
        config: RustConfig,
    ) -> Self {
        let capture_fields = capture_field_targets(&spec);
        Self {
            spec,
            entity_name,
            config,
            capture_fields,
        }
    }

    fn compile(&self) -> RustOutput {
        RustOutput {
            cargo_toml: self.generate_cargo_toml(),
            lib_rs: self.generate_lib_rs(),
            types_rs: self.generate_types_rs(),
            entity_rs: self.generate_entity_rs(),
            programs_rs: None,
            program_modules: Vec::new(),
        }
    }

    fn generate_cargo_toml(&self) -> String {
        format!(
            r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
arete-sdk = {{ package = "arete-a4-sdk", version = "{}" }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#,
            self.config.crate_name, self.config.sdk_version
        )
    }

    fn generate_lib_rs(&self) -> String {
        let stack_name = self.derive_stack_name();
        let entity_name = &self.entity_name;

        format!(
            r#"mod entity;
mod types;

pub use entity::{{{stack_name}Stack, {stack_name}StackViews, {entity_name}EntityViews}};
pub use types::*;

pub use arete_sdk::{{ConnectionState, Arete, Stack, Update, Views}};
"#,
            stack_name = stack_name,
            entity_name = entity_name
        )
    }

    fn generate_types_rs(&self) -> String {
        let mut output = String::new();
        output.push_str("use serde::{Deserialize, Serialize};\n");
        output.push_str("use arete_sdk::serde_utils;\n\n");

        let resolved_name_map = self.build_resolved_type_name_map();
        let mut generated = HashSet::new();

        for section in &self.spec.sections {
            if !Self::is_root_section(&section.name)
                && section.fields.iter().any(|field| field.emit)
                && generated.insert(section.name.clone())
            {
                output.push_str(&self.generate_struct_for_section(section, &resolved_name_map));
                output.push_str("\n\n");
            }
        }

        output.push_str(&self.generate_main_entity_struct(&resolved_name_map));
        output.push_str(&self.generate_resolved_types(&resolved_name_map, &mut generated));

        let builtins = render_builtin_resolver_structs(&self.used_builtin_resolver_types());
        if !builtins.is_empty() {
            output.push_str("\n\n");
            output.push_str(builtins.trim_end());
        }

        output.push_str(&self.generate_wrapper_types());

        output
    }

    pub(crate) fn generate_struct_for_section(
        &self,
        section: &EntitySection,
        resolved_name_map: &HashMap<String, String>,
    ) -> String {
        let struct_name = format!("{}{}", self.entity_name, to_pascal_case(&section.name));
        let mut fields = Vec::new();

        for field in &section.fields {
            if !field.emit {
                continue;
            }
            let field_name = to_snake_case(&field.field_name);
            let rust_type = self.field_type_to_rust(field, &section.name, resolved_name_map);
            let serde_attr = self.serde_attr_for_field(field, &section.name);

            fields.push(format!(
                "    {}\n    pub {}: {},",
                serde_attr, field_name, rust_type
            ));
        }

        format!(
            "#[derive(Debug, Clone, Serialize, Deserialize, Default)]\npub struct {} {{\n{}\n}}",
            struct_name,
            fields.join("\n")
        )
    }

    pub(crate) fn is_root_section(name: &str) -> bool {
        name.eq_ignore_ascii_case("root")
    }

    pub(crate) fn generate_main_entity_struct(
        &self,
        resolved_name_map: &HashMap<String, String>,
    ) -> String {
        let mut fields = Vec::new();

        for section in &self.spec.sections {
            if !Self::is_root_section(&section.name)
                && section.fields.iter().any(|field| field.emit)
            {
                let field_name = to_snake_case(&section.name);
                let type_name = format!("{}{}", self.entity_name, to_pascal_case(&section.name));
                fields.push(format!(
                    "    #[serde(default)]\n    pub {}: {},",
                    field_name, type_name
                ));
            }
        }

        for section in &self.spec.sections {
            if Self::is_root_section(&section.name) {
                for field in &section.fields {
                    if !field.emit {
                        continue;
                    }
                    let field_name = to_snake_case(&field.field_name);
                    let rust_type =
                        self.field_type_to_rust(field, &section.name, resolved_name_map);
                    let serde_attr = self.serde_attr_for_field(field, &section.name);
                    fields.push(format!(
                        "    {}\n    pub {}: {},",
                        serde_attr, field_name, rust_type
                    ));
                }
            }
        }

        format!(
            "#[derive(Debug, Clone, Serialize, Deserialize, Default)]\npub struct {} {{\n{}\n}}",
            self.entity_name,
            fields.join("\n")
        )
    }

    pub(crate) fn generate_resolved_types(
        &self,
        resolved_name_map: &HashMap<String, String>,
        generated: &mut HashSet<String>,
    ) -> String {
        let mut output = String::new();

        for section in &self.spec.sections {
            for field in &section.fields {
                if !field.emit {
                    continue;
                }
                if let Some(resolved) = &field.resolved_type {
                    let emitted_name = self.resolved_type_to_rust_name(resolved, resolved_name_map);
                    if generated.insert(emitted_name.clone()) {
                        output.push_str("\n\n");
                        output.push_str(&self.generate_resolved_struct(resolved, &emitted_name));
                    }
                }
            }
        }

        output
    }

    fn generate_resolved_struct(
        &self,
        resolved: &ResolvedStructType,
        emitted_name: &str,
    ) -> String {
        render_resolved_struct(resolved, emitted_name)
    }

    /// Produce stable, distinct Rust identifiers for resolved IDL fields.
    ///
    /// The general snake-case helper collapses leading punctuation, so
    /// `padding_0` and `_padding_0` otherwise become the same struct field.
    /// Preserve leading underscores when they distinguish colliding names and
    /// use a numeric suffix for any remaining normalization collision.
    fn canonical_resolved_field_names(fields: &[ResolvedField]) -> Vec<String> {
        let base_names = fields
            .iter()
            .map(|field| to_snake_case(field.raw_field_name()))
            .collect::<Vec<_>>();
        let mut base_counts = BTreeMap::<String, usize>::new();
        for base_name in &base_names {
            *base_counts.entry(base_name.clone()).or_default() += 1;
        }

        let mut used_names = HashSet::new();
        fields
            .iter()
            .zip(base_names)
            .map(|(field, base_name)| {
                let leading_underscores = field
                    .raw_field_name()
                    .chars()
                    .take_while(|character| *character == '_')
                    .count();
                let preferred = if base_counts.get(&base_name).copied().unwrap_or_default() > 1
                    && leading_underscores > 0
                {
                    format!("{}{}", "_".repeat(leading_underscores), base_name)
                } else {
                    base_name
                };

                if used_names.insert(preferred.clone()) {
                    return preferred;
                }

                let mut suffix = 2;
                loop {
                    let candidate = format!("{preferred}_{suffix}");
                    if used_names.insert(candidate.clone()) {
                        return candidate;
                    }
                    suffix += 1;
                }
            })
            .collect()
    }

    /// Wire names remain snake_case but retain meaningful leading underscores.
    fn resolved_field_wire_name(field: &ResolvedField) -> String {
        let raw_name = field.raw_field_name();
        let leading_underscores = raw_name
            .chars()
            .take_while(|character| *character == '_')
            .count();
        format!(
            "{}{}",
            "_".repeat(leading_underscores),
            to_snake_case(raw_name)
        )
    }

    fn generate_wrapper_types(&self) -> String {
        format!("\n\n{WRAPPER_TYPES}")
    }

    fn generate_entity_rs(&self) -> String {
        let entity_name = &self.entity_name;
        let stack_name = self.derive_stack_name();
        let stack_name_kebab = to_kebab_case(entity_name);
        let entity_snake = to_snake_case(entity_name);

        let types_import = if self.config.module_mode {
            "super::types"
        } else {
            "crate::types"
        };

        // Generate URL line - either actual URL or placeholder comment
        let url_impl = match &self.config.url {
            Some(url) => format!(
                r#"fn url() -> &'static str {{
        "{}"
    }}"#,
                url
            ),
            None => r#"fn url() -> &'static str {
        "" // TODO: Set URL after first deployment in arete.toml
    }"#
            .to_string(),
        };

        let entity_views = self.generate_entity_views_struct();

        format!(
            r#"use {types_import}::{entity_name};
use arete_sdk::{{Stack, StateView, ViewBuilder, ViewHandle, Views}};

pub struct {stack_name}Stack;

impl Stack for {stack_name}Stack {{
    type Views = {stack_name}StackViews;
    type Programs = ();

    fn name() -> &'static str {{
        "{stack_name_kebab}"
    }}

    {url_impl}
}}

pub struct {stack_name}StackViews {{
    pub {entity_snake}: {entity_name}EntityViews,
}}

impl Views for {stack_name}StackViews {{
    fn from_builder(builder: ViewBuilder) -> Self {{
        Self {{
            {entity_snake}: {entity_name}EntityViews {{ builder }},
        }}
    }}
}}
{entity_views}"#,
            types_import = types_import,
            entity_name = entity_name,
            stack_name = stack_name,
            stack_name_kebab = stack_name_kebab,
            entity_snake = entity_snake,
            url_impl = url_impl,
            entity_views = entity_views
        )
    }

    fn generate_entity_views_struct(&self) -> String {
        let entity_name = &self.entity_name;

        let derived: Vec<_> = self
            .spec
            .views
            .iter()
            .filter(|v| {
                !v.id.ends_with("/state")
                    && !v.id.ends_with("/list")
                    && v.id.starts_with(entity_name)
            })
            .collect();

        let mut derived_methods = String::new();
        for view in &derived {
            let view_name = view.id.split('/').nth(1).unwrap_or("unknown");
            let method_name = to_snake_case(view_name);

            derived_methods.push_str(&format!(
                r#"
    pub fn {method_name}(&self) -> ViewHandle<{entity_name}> {{
        self.builder.view("{view_id}")
    }}
"#,
                method_name = method_name,
                entity_name = entity_name,
                view_id = view.id
            ));
        }

        format!(
            r#"
pub struct {entity_name}EntityViews {{
    builder: ViewBuilder,
}}

impl {entity_name}EntityViews {{
    pub fn state(&self) -> StateView<{entity_name}> {{
        StateView::new(
            self.builder.connection().clone(),
            self.builder.store().clone(),
            "{entity_name}/state".to_string(),
            self.builder.initial_data_timeout(),
        )
    }}

    pub fn list(&self) -> ViewHandle<{entity_name}> {{
        self.builder.view("{entity_name}/list")
    }}
{derived_methods}}}"#,
            entity_name = entity_name,
            derived_methods = derived_methods
        )
    }

    /// Derive stack name from entity name.
    /// E.g., "OreRound" -> "Ore", "PumpfunToken" -> "Pumpfun"
    fn derive_stack_name(&self) -> String {
        let entity_name = &self.entity_name;

        // Common suffixes to strip
        let suffixes = ["Round", "Token", "Game", "State", "Entity", "Data"];

        for suffix in suffixes {
            if entity_name.ends_with(suffix) && entity_name.len() > suffix.len() {
                return entity_name[..entity_name.len() - suffix.len()].to_string();
            }
        }

        // If no suffix matched, use the full entity name
        entity_name.clone()
    }

    /// Generate Rust type for a field.
    ///
    /// All fields are wrapped in Option<T> because we receive partial patches,
    /// so any field may not yet be present.
    ///
    /// - Non-optional spec fields become `Option<T>`:
    ///   - `None` = not yet received in any patch
    ///   - `Some(value)` = has value
    ///
    /// - Optional spec fields become `Option<Option<T>>`:
    ///   - `None` = not yet received in any patch
    ///   - `Some(None)` = explicitly set to null
    ///   - `Some(Some(value))` = has value
    fn field_type_to_rust(
        &self,
        field: &FieldTypeInfo,
        section_name: &str,
        resolved_name_map: &HashMap<String, String>,
    ) -> String {
        // Fields backed by a resolved IDL struct are typed against the emitted
        // struct, wrapped in the runtime envelope they actually arrive in.
        // Mirror of `typescript::field_type_info_to_typescript`.
        let typed = if let Some(resolved) = &field.resolved_type {
            let name = self.resolved_type_to_rust_name(resolved, resolved_name_map);
            let element = match wrapper_kind_for(field, resolved, &self.capture_fields) {
                WrapperKind::None => name,
                WrapperKind::Capture => format!("CaptureWrapper<{}>", name),
                WrapperKind::Event => format!("EventWrapper<{}>", name),
            };
            if field.is_array {
                format!("Vec<{}>", element)
            } else {
                element
            }
        } else if let Some(builtin) = self.builtin_type_for_field(section_name, field) {
            // Builtin resolver outputs are typed against the generated struct.
            // Mirror of `typescript::field_type_info_to_typescript`.
            if field.is_array {
                format!("Vec<{}>", builtin)
            } else {
                builtin.to_string()
            }
        } else {
            self.scalar_shape_for_field(field).rust_type
        };

        // All fields wrapped in Option since we receive patches
        // Optional spec fields get Option<Option<T>> to distinguish "not received" from "explicitly null"
        if field.is_optional {
            format!("Option<Option<{}>>", typed)
        } else {
            format!("Option<{}>", typed)
        }
    }

    /// The builtin resolver struct a section field is typed against, if any.
    ///
    /// Mirror of the TypeScript generator's "effective field info" override in
    /// `add_unmapped_fields`: a computed field keeps the *user's* declared Rust
    /// type in the section (`ResolvedSlotHash`), and only the `field_mappings`
    /// entry records the resolver output type (`SlotHashBytes`), so both have
    /// to be consulted.
    fn builtin_type_for_field(
        &self,
        section_name: &str,
        field: &FieldTypeInfo,
    ) -> Option<&'static str> {
        if let Some(name) = builtin_resolver_struct(field.inner_type.as_deref()) {
            return Some(name);
        }
        let field_path = format!("{}.{}", section_name, field.field_name);
        self.spec
            .field_mappings
            .get(&field_path)
            .and_then(|mapping| builtin_resolver_struct(mapping.inner_type.as_deref()))
    }

    /// Builtin resolver structs referenced by this entity's emitted fields.
    pub(crate) fn used_builtin_resolver_types(&self) -> BTreeSet<&'static str> {
        let mut used = BTreeSet::new();
        for section in &self.spec.sections {
            for field in &section.fields {
                if !field.emit || field.resolved_type.is_some() {
                    continue;
                }
                if let Some(name) = self.builtin_type_for_field(&section.name, field) {
                    used.insert(name);
                }
            }
        }
        used
    }

    fn scalar_shape_for_field(&self, field: &FieldTypeInfo) -> RustScalarShape {
        rust_scalar_field_shape(
            &field.base_type,
            field.effective_integer_kind(),
            field.is_array,
            field
                .inner_type
                .as_deref()
                .or(Some(field.rust_type_name.as_str())),
            &field.rust_type_name,
        )
    }

    /// Return the `#[serde(...)]` attribute for a field.
    /// Integer fields get a `deserialize_with` pointing to the appropriate
    /// `serde_utils` function so that string-encoded big integers are handled.
    fn serde_attr_for_field(&self, field: &FieldTypeInfo, section_name: &str) -> String {
        if field.resolved_type.is_some()
            || self.builtin_type_for_field(section_name, field).is_some()
        {
            return "#[serde(default)]".to_string();
        }
        let shape = self.scalar_shape_for_field(field);
        match deserialize_with_for_shape(&shape, field.is_optional) {
            Some(deser_fn) => format!("#[serde(default, deserialize_with = \"{}\")]", deser_fn),
            None => "#[serde(default)]".to_string(),
        }
    }

    fn build_resolved_type_name_map(&self) -> HashMap<String, String> {
        self.build_stack_resolved_type_name_map(&StackResolvedTypes::default(), None)
    }

    /// Names this entity's resolved types, sharing or renaming around the
    /// resolved types earlier entities of the stack declared in `types.rs`.
    fn build_stack_resolved_type_name_map(
        &self,
        stack_types: &StackResolvedTypes,
        program_name: Option<&str>,
    ) -> HashMap<String, String> {
        let namespaces = resolved_type_namespaces(
            program_name.or(self.spec.idl.as_ref().map(|idl| idl.name.as_str())),
            &self.entity_name,
        );
        let mut reserved_names = HashSet::from([
            self.entity_name.clone(),
            "EventWrapper".to_string(),
            "CaptureWrapper".to_string(),
        ]);

        // Builtin resolver structs share the `types.rs` namespace, so a
        // same-named IDL type has to be renamed around them. Mirror of the
        // TypeScript generator reserving `TokenMetadata`.
        for (name, _) in BUILTIN_RESOLVER_STRUCTS {
            reserved_names.insert((*name).to_string());
        }

        for section in &self.spec.sections {
            if !Self::is_root_section(&section.name)
                && section.fields.iter().any(|field| field.emit)
            {
                reserved_names.insert(format!(
                    "{}{}",
                    self.entity_name,
                    to_pascal_case(&section.name)
                ));
            }
        }

        let mut resolved_name_map = HashMap::new();

        for section in &self.spec.sections {
            for field in &section.fields {
                if !field.emit {
                    continue;
                }

                let Some(resolved) = &field.resolved_type else {
                    continue;
                };

                if resolved_name_map.contains_key(&resolved.type_name) {
                    continue;
                }

                let emitted_name = unique_resolved_type_name(resolved, &mut reserved_names);
                let emitted_name = stack_types
                    .claim(
                        resolved,
                        emitted_name,
                        &to_pascal_case(&resolved.type_name),
                        &namespaces,
                        &mut reserved_names,
                    )
                    .into_name();
                resolved_name_map.insert(resolved.type_name.clone(), emitted_name);
            }
        }

        resolved_name_map
    }

    fn resolved_type_to_rust_name(
        &self,
        resolved: &ResolvedStructType,
        resolved_name_map: &HashMap<String, String>,
    ) -> String {
        resolved_name_map
            .get(&resolved.type_name)
            .cloned()
            .unwrap_or_else(|| to_pascal_case(&resolved.type_name))
    }
}

/// Render one resolved IDL type (struct or fieldless enum) as `types.rs`
/// declares it. Every resolved type of a Rust SDK, whether an entity maps it
/// or it is a program account's model, is declared through this one renderer,
/// so the same IDL type always gets the same fields, types and serde
/// attributes.
pub(crate) fn render_resolved_struct(resolved: &ResolvedStructType, emitted_name: &str) -> String {
    if resolved.is_enum {
        let variants: Vec<String> = resolved
            .enum_variants
            .iter()
            .map(|v| format!("    {},", to_pascal_case(v)))
            .collect();

        return format!(
            "#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]\npub enum {} {{\n{}\n}}",
            emitted_name,
            variants.join("\n")
        );
    }
    let fields: Vec<String> = resolved
        .fields
        .iter()
        .zip(RustCompiler::canonical_resolved_field_names(
            &resolved.fields,
        ))
        .map(|(f, field_name)| {
            let rust_type = resolved_field_to_rust(f);
            let mut serde_attrs = vec![serde_attr_for_resolved_field(f)];
            let wire_name = RustCompiler::resolved_field_wire_name(f);
            if field_name != wire_name {
                serde_attrs.push(format!(
                    "#[serde(rename = {})]",
                    rust_string_literal(&wire_name)
                ));
            }
            format!(
                "    {}\n    pub {}: {},",
                serde_attrs.join("\n    "),
                field_name,
                rust_type
            )
        })
        .collect();

    format!(
        "#[derive(Debug, Clone, Serialize, Deserialize, Default)]\npub struct {} {{\n{}\n}}",
        emitted_name,
        fields.join("\n")
    )
}

/// A declared model under `name`: an entity's resolved type, or an
/// IDL-derived model.
fn render_declared_model(model: &DeclaredModel, name: &str) -> String {
    match model {
        DeclaredModel::Resolved(resolved) => render_resolved_struct(resolved, name),
        DeclaredModel::Idl(model) => render_idl_model(model, name),
    }
}

/// The Rust declaration of an IDL-derived model (see [`crate::idl_models`]).
///
/// A struct is a serde struct whose fields are all `Option`s, as
/// [`render_resolved_struct`] renders one (a struct of flat fields renders
/// exactly as it does). An enum is externally tagged, the Program Read wire
/// shape: a unit variant is its name, a data variant a one-key object from
/// its name to its fields (tuple fields keyed `field_<index>`).
pub(crate) fn render_idl_model(model: &IdlModel, name: &str) -> String {
    match &model.kind {
        IdlModelKind::Struct(fields) => format!(
            "#[derive(Debug, Clone, Serialize, Deserialize, Default)]\npub struct {} {{\n{}\n}}",
            name,
            render_idl_model_fields(fields, "    ", "pub ").join("\n")
        ),
        IdlModelKind::Enum(variants) => {
            let derive = if variants.iter().all(|variant| variant.fields.is_empty()) {
                "#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]"
            } else {
                "#[derive(Debug, Clone, Serialize, Deserialize)]"
            };
            let mut used = HashSet::new();
            let variants = variants
                .iter()
                .map(|variant| {
                    let base = rust_ident::identifier(&variant.name, IdentifierCase::Pascal);
                    let ident = std::iter::once(base.clone())
                        .chain((2usize..).map(|index| format!("{base}{index}")))
                        .find(|candidate| used.insert(candidate.clone()))
                        .expect("the numbered candidates are unbounded");
                    let mut lines = Vec::new();
                    if ident != variant.name {
                        lines.push(format!(
                            "    #[serde(rename = {})]",
                            rust_string_literal(&variant.name)
                        ));
                    }
                    if variant.fields.is_empty() {
                        lines.push(format!("    {ident},"));
                    } else {
                        lines.push(format!(
                            "    {ident} {{\n{}\n    }},",
                            render_idl_model_fields(&variant.fields, "        ", "").join("\n")
                        ));
                    }
                    lines.join("\n")
                })
                .collect::<Vec<_>>();
            format!("{derive}\npub enum {name} {{\n{}\n}}", variants.join("\n"))
        }
    }
}

/// The fields of an IDL-derived struct or enum variant. A field keeps its
/// flat rendering when the flat projection types it; otherwise it is typed
/// from its wire shape. Fields are renamed to their snake_case wire key and
/// also accept the IDL's own spelling (the key the server sends).
fn render_idl_model_fields(fields: &[ModelField], indent: &str, visibility: &str) -> Vec<String> {
    let flats = fields
        .iter()
        .map(|field| field.flat.clone())
        .collect::<Vec<_>>();
    fields
        .iter()
        .zip(RustCompiler::canonical_resolved_field_names(&flats))
        .map(|(field, ident)| {
            let (rust_type, mut attrs) = match &field.typed {
                None => (
                    resolved_field_to_rust(&field.flat),
                    vec![serde_attr_for_resolved_field(&field.flat)],
                ),
                Some(typed) => rust_typed_field(typed),
            };
            let wire = field.wire_name();
            let raw = field.flat.raw_field_name();
            if ident != wire {
                attrs.push(format!("#[serde(rename = {})]", rust_string_literal(&wire)));
            }
            if raw != wire {
                attrs.push(format!("#[serde(alias = {})]", rust_string_literal(raw)));
            }
            format!(
                "{indent}{}\n{indent}{visibility}{ident}: {rust_type},",
                attrs.join(&format!("\n{indent}"))
            )
        })
        .collect()
}

/// The type and serde attribute of a field typed from its wire shape: an
/// `Option` (not received), or `Option<Option<..>>` for an IDL option.
fn rust_typed_field(typed: &WireType) -> (String, Vec<String>) {
    let (inner, optional) = match typed {
        WireType::Option(inner) => (inner.as_ref(), true),
        other => (other, false),
    };
    let rust = rust_wire_type(inner);
    let rust_type = if optional {
        format!("Option<Option<{rust}>>")
    } else {
        format!("Option<{rust}>")
    };
    let attr = match rust_wire_shape(inner) {
        None => "#[serde(default)]".to_string(),
        Some(shape) => format!(
            "#[serde(default, deserialize_with = \"serde_utils::deserialize_wire_option{}::<_, _, {shape}>\")]",
            if optional { "_option" } else { "" }
        ),
    };
    (rust_type, vec![attr])
}

/// The Rust type of a wire shape. Integers take the flat convention's
/// normalized kinds.
fn rust_wire_type(wire: &WireType) -> String {
    match wire {
        WireType::Scalar {
            base_type: BaseType::Integer,
            integer_kind: Some(kind),
        } => normalized_integer_kind_of(*kind).to_string(),
        WireType::Scalar { base_type, .. } => base_type_to_rust(base_type, "i64"),
        WireType::Option(inner) => format!("Option<{}>", rust_wire_type(inner)),
        WireType::List(inner) => format!("Vec<{}>", rust_wire_type(inner)),
        WireType::Map(inner) => format!(
            "std::collections::BTreeMap<String, {}>",
            rust_wire_type(inner)
        ),
        WireType::Tuple(elements) => {
            let elements = elements.iter().map(rust_wire_type).collect::<Vec<_>>();
            match elements.as_slice() {
                [only] => format!("({only},)"),
                elements => format!("({})", elements.join(", ")),
            }
        }
        WireType::Model(name) => name.clone(),
        WireType::Json => "serde_json::Value".to_string(),
    }
}

/// The `serde_utils::wire` shape a value decodes with, or `None` when its
/// own `Deserialize` impl suffices (it holds no integer outside a model).
fn rust_wire_shape(wire: &WireType) -> Option<String> {
    if !wire.has_integer() {
        return None;
    }
    let shape = |inner: &WireType| {
        rust_wire_shape(inner).unwrap_or_else(|| "serde_utils::wire::Plain".to_string())
    };
    Some(match wire {
        WireType::Scalar { .. } => "serde_utils::wire::Int".to_string(),
        WireType::Option(inner) => format!("serde_utils::wire::Opt<{}>", shape(inner)),
        WireType::List(inner) => format!("serde_utils::wire::List<{}>", shape(inner)),
        WireType::Map(inner) => format!("serde_utils::wire::Map<{}>", shape(inner)),
        WireType::Tuple(elements) => {
            let elements = elements.iter().map(shape).collect::<Vec<_>>();
            match elements.as_slice() {
                [only] => format!("({only},)"),
                elements => format!("({})", elements.join(", ")),
            }
        }
        WireType::Model(_) | WireType::Json => return None,
    })
}

fn scalar_shape_for_resolved_field(field: &ResolvedField) -> RustScalarShape {
    rust_scalar_field_shape(
        &field.base_type,
        field.effective_integer_kind(),
        field.is_array,
        Some(field.field_type.as_str()),
        &field.field_type,
    )
}

/// The `#[serde(...)]` attribute of a resolved struct field: integers get a
/// `serde_utils` deserializer so string-encoded big integers parse.
fn serde_attr_for_resolved_field(field: &ResolvedField) -> String {
    let shape = scalar_shape_for_resolved_field(field);
    match deserialize_with_for_shape(&shape, field.is_optional) {
        Some(deser_fn) => format!("#[serde(default, deserialize_with = \"{}\")]", deser_fn),
        None => "#[serde(default)]".to_string(),
    }
}

fn resolved_field_to_rust(field: &ResolvedField) -> String {
    let typed = scalar_shape_for_resolved_field(field).rust_type;
    if field.is_optional {
        format!("Option<Option<{}>>", typed)
    } else {
        format!("Option<{}>", typed)
    }
}

fn unique_resolved_type_name(
    resolved: &ResolvedStructType,
    reserved_names: &mut HashSet<String>,
) -> String {
    let base_name = to_pascal_case(&resolved.type_name);
    if reserved_names.insert(base_name.clone()) {
        return base_name;
    }

    let suffix = if resolved.is_account {
        "Account"
    } else if resolved.is_event {
        "Event"
    } else if resolved.is_instruction {
        "Instruction"
    } else {
        "Type"
    };

    let preferred = format!("{}{}", base_name, suffix);
    if reserved_names.insert(preferred.clone()) {
        return preferred;
    }

    let mut index = 2;
    loop {
        let candidate = format!("{}{}{}", base_name, suffix, index);
        if reserved_names.insert(candidate.clone()) {
            return candidate;
        }
        index += 1;
    }
}

/// [`normalized_integer_kind`] for an already-classified [`IntegerKind`].
/// Kept byte-for-byte equivalent to the string-sniffing version: only
/// `u64`/`i64`/`u32`/`i32`/`u128`/`i128` have `serde_utils` deserializers, so
/// unsigned small ints widen to `u64` and signed small ints to `i64`.
fn normalized_integer_kind_of(kind: IntegerKind) -> &'static str {
    match kind {
        IntegerKind::U64 => "u64",
        IntegerKind::U32 => "u32",
        IntegerKind::I32 => "i32",
        // 128-bit values arrive as decimal strings beyond any 64-bit range.
        IntegerKind::U128 => "u128",
        IntegerKind::I128 => "i128",
        IntegerKind::U8 | IntegerKind::U16 | IntegerKind::Usize => "u64",
        // Signed small ints (i16/i8/isize) widen to i64.
        _ => "i64",
    }
}

fn normalized_integer_kind(rust_type_name: &str) -> &'static str {
    if rust_type_name.contains("u128") {
        "u128"
    } else if rust_type_name.contains("i128") {
        "i128"
    } else if rust_type_name.contains("u64") {
        "u64"
    } else if rust_type_name.contains("i64") {
        "i64"
    } else if rust_type_name.contains("u32") {
        "u32"
    } else if rust_type_name.contains("i32") {
        "i32"
    } else if rust_type_name.contains("u16")
        || rust_type_name.contains("u8")
        || rust_type_name.contains("usize")
    {
        "u64"
    } else {
        // Signed small ints (i16/i8/isize) and anything unknown widen to i64.
        "i64"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::process::Command;

    fn identity_spec() -> IdentitySpec {
        IdentitySpec {
            primary_keys: vec!["id.address".to_string()],
            lookup_indexes: vec![],
        }
    }

    #[test]
    fn rust_generator_renames_account_types_on_collision() {
        let plan_field = FieldTypeInfo {
            field_name: "plan".to_string(),
            raw_name: Some("plan".to_string()),
            canonical_name: Some("plan".to_string()),
            rust_type_name: "Option<serde_json::Value>".to_string(),
            base_type: BaseType::Object,
            integer_kind: None,
            is_optional: false,
            is_array: false,
            inner_type: Some("Value".to_string()),
            source_path: None,
            resolved_type: Some(ResolvedStructType {
                type_name: "plan".to_string(),
                fields: vec![],
                is_instruction: false,
                is_account: true,
                is_event: false,
                is_enum: false,
                enum_variants: vec![],
            }),
            emit: true,
        };

        let spec = SerializableStreamSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            state_name: "Plan".to_string(),
            program_id: None,
            idl: None,
            identity: identity_spec(),
            handlers: vec![],
            sections: vec![
                EntitySection {
                    name: "id".to_string(),
                    fields: vec![FieldTypeInfo::new(
                        "address".to_string(),
                        "String".to_string(),
                    )],
                    is_nested_struct: false,
                    parent_field: None,
                },
                EntitySection {
                    name: "plan".to_string(),
                    fields: vec![plan_field],
                    is_nested_struct: false,
                    parent_field: None,
                },
            ],
            field_mappings: BTreeMap::new(),
            resolver_hooks: vec![],
            instruction_hooks: vec![],
            resolver_specs: vec![],
            computed_fields: vec![],
            computed_field_specs: vec![],
            content_hash: None,
            views: vec![],
        };

        let output = compile_serializable_spec(spec, "Plan".to_string(), None)
            .expect("rust sdk generation should succeed");

        // The resolved struct is renamed away from the entity struct, and the
        // field is typed against the renamed struct (no `AsCapture` mapping
        // feeds it, so it stays a bare struct — see
        // `rust_generator_wraps_capture_and_event_fields`).
        assert!(output.types_rs.contains("pub struct PlanAccount"));
        assert!(output.types_rs.contains("pub plan: Option<PlanAccount>"));
        assert!(!output
            .types_rs
            .contains("pub plan: Option<serde_json::Value>"));
        assert!(
            !output.types_rs.contains("pub struct Plan {\n    #[serde(default, deserialize_with = \"serde_utils::deserialize_option_u64\")]\n    pub discriminator")
        );
    }

    #[test]
    fn rust_generator_keeps_unsigned_numeric_fields_unsigned() {
        let spec = SerializableStreamSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            state_name: "Plan".to_string(),
            program_id: None,
            idl: None,
            identity: identity_spec(),
            handlers: vec![],
            sections: vec![
                EntitySection {
                    name: "id".to_string(),
                    fields: vec![FieldTypeInfo::new(
                        "address".to_string(),
                        "String".to_string(),
                    )],
                    is_nested_struct: false,
                    parent_field: None,
                },
                EntitySection {
                    name: "state".to_string(),
                    fields: vec![FieldTypeInfo::new(
                        "status".to_string(),
                        "Option<u8>".to_string(),
                    )],
                    is_nested_struct: false,
                    parent_field: None,
                },
            ],
            field_mappings: BTreeMap::new(),
            resolver_hooks: vec![],
            instruction_hooks: vec![],
            resolver_specs: vec![],
            computed_fields: vec![],
            computed_field_specs: vec![],
            content_hash: None,
            views: vec![],
        };

        let output = compile_serializable_spec(spec, "Plan".to_string(), None)
            .expect("rust sdk generation should succeed");

        assert!(
            output.types_rs.contains("pub status: Option<Option<u64>>"),
            "expected unsigned optional field, got:\n{}",
            output.types_rs
        );
    }

    /// Scalar arrays must land on a real Rust element type. `Vec<u64>`-shaped
    /// fields reach the generator as `BaseType::Array` + `integer_kind`, so the
    /// integer check has to consult `integer_kind` (TypeScript emits
    /// `bigint[]`, Python `List[int]`); non-integer scalar arrays keep their
    /// element type instead of degrading to `Vec<serde_json::Value>`.
    /// Rust twin of `python::tests::python_generator_converts_u64_arrays`.
    #[test]
    fn rust_generator_types_scalar_arrays() {
        let mut entity = minimal_entity("OreRound");
        entity.sections.push(EntitySection {
            name: "state".to_string(),
            fields: vec![
                FieldTypeInfo::new(
                    "deployed_per_square".to_string(),
                    "Option<Vec<u64>>".to_string(),
                ),
                FieldTypeInfo::new(
                    "deployed_per_square_ui".to_string(),
                    "Option<Vec<f64>>".to_string(),
                ),
                FieldTypeInfo::new("flags".to_string(), "Option<Vec<bool>>".to_string()),
                FieldTypeInfo::new("labels".to_string(), "Option<Vec<String>>".to_string()),
                FieldTypeInfo::new("resolved_seed".to_string(), "Option<Vec<u8>>".to_string()),
                // A `#[binary]` blob keeps `Vec<u8>`: the integer guard is
                // `BaseType::Array`-only, never "any field with an
                // `integer_kind`".
                FieldTypeInfo::new("payload".to_string(), "Option<Vec<u8>>".to_string()),
            ],
            is_nested_struct: false,
            parent_field: None,
        });
        // The interpreter records the element kind explicitly for `Vec<u64>`.
        for field in &mut entity.sections[1].fields {
            match field.field_name.as_str() {
                "deployed_per_square" => {
                    field.base_type = BaseType::Array;
                    field.integer_kind = Some(IntegerKind::U64);
                    field.is_array = true;
                    field.inner_type = Some("Vec < u64 >".to_string());
                }
                "resolved_seed" => {
                    field.base_type = BaseType::Array;
                    field.integer_kind = Some(IntegerKind::U8);
                    field.is_array = true;
                    field.inner_type = Some("Vec < u8 >".to_string());
                }
                "deployed_per_square_ui" => {
                    field.base_type = BaseType::Array;
                    field.is_array = true;
                    field.inner_type = Some("Vec < f64 >".to_string());
                }
                "flags" => {
                    field.base_type = BaseType::Array;
                    field.is_array = true;
                    field.inner_type = Some("Vec < bool >".to_string());
                }
                "labels" => {
                    field.base_type = BaseType::Array;
                    field.is_array = true;
                    field.inner_type = Some("Vec < String >".to_string());
                }
                "payload" => {
                    field.base_type = BaseType::Binary;
                    field.integer_kind = Some(IntegerKind::U8);
                    field.is_array = false;
                    field.inner_type = Some("Vec < u8 >".to_string());
                }
                _ => {}
            }
        }

        let output = compile_stack_spec(stack_of("OreRound", entity), None)
            .expect("rust stack generation should succeed");
        let types = &output.types_rs;

        assert!(
            !types.contains("Vec<serde_json::Value>"),
            "scalar arrays should not fall back to untyped values:\n{types}"
        );

        // u64 arrives on the wire as decimal strings (canonical numeric rule),
        // so the typed vector needs the string-or-number vector deserializer.
        assert!(
            types.contains(
                "#[serde(default, deserialize_with = \"serde_utils::deserialize_option_option_vec_u64\")]\n    pub deployed_per_square: Option<Option<Vec<u64>>>,"
            ),
            "expected a typed u64 vector with its deserializer:\n{types}"
        );
        // Small unsigned ints widen to u64, matching the scalar policy in
        // `normalized_integer_kind` (only u64/i64/u32/i32 have deserializers).
        assert!(
            types.contains(
                "#[serde(default, deserialize_with = \"serde_utils::deserialize_option_option_vec_u64\")]\n    pub resolved_seed: Option<Option<Vec<u64>>>,"
            ),
            "expected u8 arrays to widen to Vec<u64>:\n{types}"
        );

        // Non-integer scalar arrays keep their element type and need no
        // custom deserializer.
        assert!(types.contains(
            "#[serde(default)]\n    pub deployed_per_square_ui: Option<Option<Vec<f64>>>,"
        ));
        assert!(types.contains("#[serde(default)]\n    pub flags: Option<Option<Vec<bool>>>,"));
        assert!(types.contains("#[serde(default)]\n    pub labels: Option<Option<Vec<String>>>,"));

        // `BaseType::Binary` is untouched by the integer-array branch.
        assert!(
            types.contains("#[serde(default)]\n    pub payload: Option<Option<Vec<u8>>>,"),
            "binary fields must keep Vec<u8>:\n{types}"
        );
    }

    /// Builtin resolver outputs are typed against generated structs, matching
    /// TypeScript's `oreMetadata: TokenMetadata | null` /
    /// `expiresAtSlotHash: SlotHashBytes | null`. `expires_at_slot_hash` only
    /// names the resolver output type in `field_mappings` (the section keeps
    /// the user's declared `ResolvedSlotHash`), which is the TypeScript
    /// "effective field info" override.
    #[test]
    fn rust_generator_types_builtin_resolver_fields() {
        let mut entity = minimal_entity("OreRound");
        let mut ore_metadata = FieldTypeInfo::new(
            "ore_metadata".to_string(),
            "Option<TokenMetadata>".to_string(),
        );
        ore_metadata.base_type = BaseType::Object;
        ore_metadata.is_optional = true;
        ore_metadata.inner_type = Some("TokenMetadata".to_string());

        let mut expires_at_slot_hash = FieldTypeInfo::new(
            "expires_at_slot_hash".to_string(),
            "Option<ResolvedSlotHash>".to_string(),
        );
        expires_at_slot_hash.base_type = BaseType::Object;
        expires_at_slot_hash.is_optional = true;
        expires_at_slot_hash.inner_type = Some("ResolvedSlotHash".to_string());

        // `KeccakRngValue` is a registered resolver output type, but it is a
        // u64 that the wire spells as a decimal string; Rust decodes it.
        let mut rng = FieldTypeInfo::new("rng".to_string(), "Option<u64>".to_string());
        rng.is_optional = true;
        rng.inner_type = Some("KeccakRngValue".to_string());
        rng.integer_kind = Some(IntegerKind::U64);

        entity.sections.push(EntitySection {
            name: "results".to_string(),
            fields: vec![expires_at_slot_hash.clone(), rng],
            is_nested_struct: false,
            parent_field: None,
        });
        entity.sections.push(EntitySection {
            name: "root".to_string(),
            fields: vec![ore_metadata],
            is_nested_struct: false,
            parent_field: None,
        });

        let mut slot_hash_mapping = expires_at_slot_hash;
        slot_hash_mapping.base_type = BaseType::Any;
        slot_hash_mapping.inner_type = Some("SlotHashBytes".to_string());
        entity.field_mappings.insert(
            "results.expires_at_slot_hash".to_string(),
            slot_hash_mapping,
        );

        let output = compile_stack_spec(stack_of("OreRound", entity), None)
            .expect("rust stack generation should succeed");
        let types = &output.types_rs;

        assert!(
            types.contains("pub ore_metadata: Option<Option<TokenMetadata>>,"),
            "expected a typed TokenMetadata field:\n{types}"
        );
        assert!(
            types.contains("pub expires_at_slot_hash: Option<Option<SlotHashBytes>>,"),
            "expected the field_mappings override to type the slot hash:\n{types}"
        );
        assert!(!types.contains("pub ore_metadata: Option<Option<serde_json::Value>>,"));
        assert!(!types.contains("pub expires_at_slot_hash: Option<Option<serde_json::Value>>,"));

        // The structs themselves are emitted once, with the snake_case wire keys.
        assert_eq!(types.matches("pub struct TokenMetadata {").count(), 1);
        assert_eq!(types.matches("pub struct SlotHashBytes {").count(), 1);
        assert!(types.contains("    pub logo_uri: Option<String>,"));
        assert!(types.contains("    pub bytes: Vec<u8>,"));

        // `KeccakRngValue` stays a real u64 rather than degrading to String.
        assert!(
            types.contains(
                "#[serde(default, deserialize_with = \"serde_utils::deserialize_option_option_u64\")]\n    pub rng: Option<Option<u64>>,"
            ),
            "KeccakRngValue fields must stay integers:\n{types}"
        );
        assert!(!types.contains("pub struct KeccakRngValue"));
    }

    /// Unused builtin resolver structs are not emitted.
    #[test]
    fn rust_generator_omits_unused_builtin_resolver_structs() {
        let output = compile_stack_spec(stack_of("OreTreasury", capture_entity()), None)
            .expect("rust stack generation should succeed");

        assert!(!output.types_rs.contains("pub struct TokenMetadata"));
        assert!(!output.types_rs.contains("pub struct SlotHashBytes"));
    }

    #[test]
    fn generated_manifest_uses_published_arete_sdk_package() {
        let manifest = generate_stack_cargo_toml(&RustStackConfig::default());

        let expected = format!(
            "arete-sdk = {{ package = \"arete-a4-sdk\", version = {:?} }}",
            GENERATED_RUST_SDK_VERSION
        );
        assert!(manifest.contains(&expected), "{manifest}");
        assert!(!manifest.contains("\"0.4\""), "{manifest}");
    }

    #[test]
    fn generated_sdk_version_is_the_linked_interpreter_release() {
        // `arete-interpreter` and `arete-a4-sdk` share the `arete` linked
        // release group, so the interpreter version is the SDK version.
        assert_eq!(GENERATED_RUST_SDK_VERSION, env!("CARGO_PKG_VERSION"));
        assert!(semver_like(GENERATED_RUST_SDK_VERSION));
        assert_eq!(
            RustConfig::default().sdk_version,
            GENERATED_RUST_SDK_VERSION,
            "program crates default to the linked SDK version"
        );
        assert_eq!(
            RustStackConfig::default().sdk_version,
            GENERATED_RUST_SDK_VERSION,
            "stack crates default to the linked SDK version"
        );
    }

    #[test]
    fn explicit_sdk_version_override_is_preserved() {
        let program = RustCompiler::new(
            capture_entity(),
            "OreTreasury".to_string(),
            RustConfig {
                sdk_version: "9.8.7-override".to_string(),
                ..RustConfig::default()
            },
        )
        .generate_cargo_toml();
        assert!(
            program.contains("version = \"9.8.7-override\""),
            "{program}"
        );

        let stack = generate_stack_cargo_toml(&RustStackConfig {
            sdk_version: "9.8.7-override".to_string(),
            ..RustStackConfig::default()
        });
        assert!(stack.contains("version = \"9.8.7-override\""), "{stack}");
    }

    fn semver_like(value: &str) -> bool {
        let mut parts = value.split('.');
        let numeric = |part: Option<&str>| {
            part.is_some_and(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        };
        numeric(parts.next())
            && numeric(parts.next())
            && numeric(parts.next())
            && parts.next().is_none()
    }

    fn resolved_field_of(name: &str, field_type: &str, base_type: BaseType) -> ResolvedField {
        ResolvedField {
            field_name: name.to_string(),
            raw_name: Some(name.to_string()),
            canonical_name: None,
            field_type: field_type.to_string(),
            base_type,
            integer_kind: IntegerKind::from_rust_type(field_type),
            is_optional: false,
            is_array: false,
        }
    }

    /// A root-section field backed by a resolved struct. Mirror of the Python
    /// generator's `snapshot_field` fixture.
    fn snapshot_field(
        field_name: &str,
        type_name: &str,
        is_account: bool,
        is_event: bool,
    ) -> FieldTypeInfo {
        FieldTypeInfo {
            field_name: field_name.to_string(),
            raw_name: Some(field_name.to_string()),
            canonical_name: None,
            rust_type_name: "Option<serde_json::Value>".to_string(),
            base_type: BaseType::Object,
            integer_kind: None,
            is_optional: true,
            is_array: false,
            inner_type: Some("Value".to_string()),
            source_path: None,
            resolved_type: Some(ResolvedStructType {
                type_name: type_name.to_string(),
                fields: vec![
                    resolved_field_of("motherlode", "u64", BaseType::Integer),
                    resolved_field_of("owner", "publicKey", BaseType::Pubkey),
                ],
                is_instruction: false,
                is_account,
                is_event,
                is_enum: false,
                enum_variants: vec![],
            }),
            emit: true,
        }
    }

    /// The handler mapping that feeds a field via `#[capture]`.
    fn capture_handler(target_path: &str) -> SerializableHandlerSpec {
        SerializableHandlerSpec {
            source: SourceSpec::Source {
                program_id: None,
                discriminator: None,
                type_name: "Treasury".to_string(),
                serialization: None,
                is_account: true,
            },
            key_resolution: KeyResolutionStrategy::Embedded {
                primary_field: FieldPath::new(&["id", "address"]),
            },
            mappings: vec![SerializableFieldMapping {
                target_path: target_path.to_string(),
                source: MappingSource::AsCapture {
                    field_transforms: BTreeMap::new(),
                },
                transform: None,
                population: PopulationStrategy::LastWrite,
                condition: None,
                when: None,
                stop: None,
                emit: true,
            }],
            conditions: vec![],
            emit: true,
        }
    }

    fn stack_of(name: &str, entity: SerializableStreamSpec) -> SerializableStackSpec {
        SerializableStackSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            stack_name: name.to_string(),
            program_ids: vec![],
            idls: vec![],
            program_specs: vec![],
            entities: vec![entity],
            pdas: BTreeMap::new(),
            instructions: vec![],
            content_hash: None,
        }
    }

    fn capture_entity() -> SerializableStreamSpec {
        let mut entity = minimal_entity("OreTreasury");
        entity.handlers.push(capture_handler("treasury_snapshot"));
        entity.sections.push(EntitySection {
            name: "root".to_string(),
            fields: vec![
                snapshot_field("treasury_snapshot", "Treasury", true, false),
                // Same struct kind, but no AsCapture mapping: stays unwrapped.
                snapshot_field("plain_account", "Vault", true, false),
                snapshot_field("deposit_event", "DepositEvent", false, true),
            ],
            is_nested_struct: false,
            parent_field: None,
        });
        entity
    }

    /// `#[capture]`-fed account fields and event fields arrive wrapped on the
    /// wire (`{timestamp, account_address, data: {...}, slot?, signature?}`).
    /// Emitting them as untyped `serde_json::Value` loses the typing TS and
    /// Python give; the envelope itself stays exposed because the provenance
    /// is unrecoverable elsewhere.
    #[test]
    fn rust_generator_wraps_capture_and_event_fields() {
        let output = compile_stack_spec(stack_of("OreTreasury", capture_entity()), None)
            .expect("rust stack generation should succeed");
        let types = &output.types_rs;

        // Both envelopes are emitted once, with the full provenance surface.
        assert!(types.contains("pub struct EventWrapper<T> {"));
        assert!(types.contains("pub struct CaptureWrapper<T> {"));
        assert!(types.contains("    pub account_address: String,"));
        assert!(types.contains("    pub data: T,"));
        assert!(types.contains("    pub slot: Option<u64>,"));
        assert!(types.contains("    pub signature: Option<String>,"));
        assert_eq!(types.matches("pub struct CaptureWrapper<T>").count(), 1);

        // Capture-fed account field: typed envelope, not an untyped blob.
        assert!(
            types.contains("pub treasury_snapshot: Option<Option<CaptureWrapper<Treasury>>>,"),
            "expected a typed capture envelope, got:\n{types}"
        );
        assert!(!types.contains("pub treasury_snapshot: Option<Option<serde_json::Value>>,"));

        // Event field: EventWrapper envelope.
        assert!(types.contains("pub deposit_event: Option<Option<EventWrapper<DepositEvent>>>,"));

        // Unmapped account field keeps the bare-struct shape.
        assert!(types.contains("pub plain_account: Option<Option<Vault>>,"));
        assert!(!types.contains("CaptureWrapper<Vault>"));

        // The inner structs are still emitted so the envelopes resolve.
        assert!(types.contains("pub struct Treasury {"));
        assert!(types.contains("pub struct Vault {"));
        assert!(types.contains("pub struct DepositEvent {"));
    }

    /// The single-entity path (`compile_serializable_spec`) emits the same
    /// envelopes as the stack path.
    #[test]
    fn rust_generator_wraps_capture_fields_in_single_entity_mode() {
        let output = compile_serializable_spec(capture_entity(), "OreTreasury".to_string(), None)
            .expect("rust sdk generation should succeed");
        let types = &output.types_rs;

        assert!(types.contains("pub struct CaptureWrapper<T> {"));
        assert!(types.contains("pub struct EventWrapper<T> {"));
        assert!(types.contains("pub treasury_snapshot: Option<Option<CaptureWrapper<Treasury>>>,"));
        assert!(types.contains("pub deposit_event: Option<Option<EventWrapper<DepositEvent>>>,"));
        assert!(types.contains("pub plain_account: Option<Option<Vault>>,"));
    }

    /// `CaptureWrapper` is reserved in the resolved-type name map, so an IDL
    /// struct actually named `CaptureWrapper` is renamed instead of shadowing
    /// the envelope.
    #[test]
    fn rust_generator_reserves_wrapper_type_names() {
        let mut entity = minimal_entity("OreTreasury");
        entity.sections.push(EntitySection {
            name: "root".to_string(),
            fields: vec![
                snapshot_field("wrapped", "CaptureWrapper", true, false),
                snapshot_field("evented", "EventWrapper", true, false),
            ],
            is_nested_struct: false,
            parent_field: None,
        });

        let output = compile_stack_spec(stack_of("OreTreasury", entity), None)
            .expect("rust stack generation should succeed");
        let types = &output.types_rs;

        assert!(types.contains("pub struct CaptureWrapperAccount {"));
        assert!(types.contains("pub struct EventWrapperAccount {"));
        assert!(types.contains("pub wrapped: Option<Option<CaptureWrapperAccount>>,"));
        assert!(types.contains("pub evented: Option<Option<EventWrapperAccount>>,"));
        assert_eq!(types.matches("pub struct CaptureWrapper<T>").count(), 1);
    }

    const TEST_PROGRAM_ID: &str = "Prog111111111111111111111111111111111111111";

    fn minimal_entity(name: &str) -> SerializableStreamSpec {
        SerializableStreamSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            state_name: name.to_string(),
            program_id: None,
            idl: None,
            identity: identity_spec(),
            handlers: vec![],
            sections: vec![EntitySection {
                name: "id".to_string(),
                fields: vec![FieldTypeInfo::new(
                    "address".to_string(),
                    "String".to_string(),
                )],
                is_nested_struct: false,
                parent_field: None,
            }],
            field_mappings: BTreeMap::new(),
            resolver_hooks: vec![],
            instruction_hooks: vec![],
            resolver_specs: vec![],
            computed_fields: vec![],
            computed_field_specs: vec![],
            content_hash: None,
            views: vec![],
        }
    }

    fn test_idl() -> IdlSnapshot {
        IdlSnapshot {
            name: "demo".to_string(),
            program_id: Some(TEST_PROGRAM_ID.to_string()),
            version: "0.1.0".to_string(),
            accounts: vec![],
            instructions: vec![],
            types: vec![],
            events: vec![],
            errors: vec![IdlErrorSnapshot {
                code: 6000,
                name: "SlippageExceeded".to_string(),
                msg: Some("Slippage exceeded".to_string()),
            }],
            discriminant_size: 8,
        }
    }

    fn instruction_account(name: &str, resolution: AccountResolution) -> InstructionAccountDef {
        InstructionAccountDef {
            name: name.to_string(),
            is_signer: matches!(resolution, AccountResolution::Signer),
            is_writable: true,
            resolution,
            is_optional: false,
            docs: vec![],
        }
    }

    fn instruction_arg(name: &str, arg_type: &str) -> InstructionArgDef {
        InstructionArgDef {
            name: name.to_string(),
            arg_type: arg_type.to_string(),
            docs: vec![],
            amount_hint: None,
        }
    }

    fn programs_stack_spec() -> SerializableStackSpec {
        let mut demo_pdas = BTreeMap::new();
        demo_pdas.insert(
            "counter".to_string(),
            PdaDefinition {
                name: "counter".to_string(),
                seeds: vec![
                    PdaSeedDef::Literal {
                        value: "counter".to_string(),
                    },
                    PdaSeedDef::AccountRef {
                        account_name: "authority".to_string(),
                    },
                ],
                program_id: None,
                program: None,
            },
        );
        let mut pdas = BTreeMap::new();
        pdas.insert("demo".to_string(), demo_pdas);

        SerializableStackSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            stack_name: "Demo".to_string(),
            program_ids: vec![TEST_PROGRAM_ID.to_string()],
            idls: vec![test_idl()],
            program_specs: vec![],
            entities: vec![minimal_entity("DemoThing")],
            pdas,
            instructions: vec![InstructionDef {
                name: "doThing".to_string(),
                discriminator: vec![12, 34],
                discriminator_size: 2,
                accounts: vec![
                    instruction_account("signer", AccountResolution::Signer),
                    instruction_account("authority", AccountResolution::UserProvided),
                    instruction_account(
                        "counter",
                        AccountResolution::PdaRef {
                            pda_name: "counter".to_string(),
                        },
                    ),
                    instruction_account(
                        "systemProgram",
                        AccountResolution::Known {
                            address: "11111111111111111111111111111111".to_string(),
                        },
                    ),
                ],
                args: vec![
                    instruction_arg("roundId", "u64"),
                    instruction_arg("admin", "solana_pubkey::Pubkey"),
                    instruction_arg("tip", "Option<u64>"),
                ],
                errors: vec![],
                program_id: Some(TEST_PROGRAM_ID.to_string()),
                docs: vec!["Does the thing.".to_string()],
            }],
            content_hash: None,
        }
    }

    #[test]
    fn rust_generator_supports_path_qualified_defined_types() {
        let qualified_name =
            "sb_on_demand::actions::pull_feed::pull_feed_submit_response_action::Submission";
        let emitted_name = "SbOnDemandActionsPullFeedPullFeedSubmitResponseActionSubmission";
        assert_eq!(to_pascal_case(qualified_name), emitted_name);

        let mut spec = programs_stack_spec();
        spec.idls[0].types.push(IdlTypeDefSnapshot {
            name: qualified_name.to_string(),
            docs: vec![],
            serialization: None,
            type_def: IdlTypeDefKindSnapshot::Struct {
                kind: "struct".to_string(),
                fields: vec![IdlFieldSnapshot {
                    name: "value".to_string(),
                    type_: IdlTypeSnapshot::Simple("u64".to_string()),
                    amount_hint: None,
                }],
            },
        });
        spec.instructions.push(InstructionDef {
            name: "submit".to_string(),
            discriminator: vec![7],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![instruction_arg("submission", qualified_name)],
            errors: vec![],
            program_id: Some(TEST_PROGRAM_ID.to_string()),
            docs: vec![],
        });
        spec.entities[0].sections.push(EntitySection {
            name: "root".to_string(),
            fields: vec![snapshot_field("submission", qualified_name, false, false)],
            is_nested_struct: false,
            parent_field: None,
        });

        let output = compile_stack_spec(spec, None).expect("qualified types should generate");
        assert!(output
            .types_rs
            .contains(&format!("pub struct {emitted_name} {{")));
        assert!(!output.types_rs.contains("::Submission"));
        let programs = output.programs_rs.expect("program module");
        assert!(programs.contains("pub struct SubmitParams {"));
        assert!(programs.contains("pub submission: serde_json::Value,"));
        assert!(programs.contains("ArgField { name: \"value\".to_string(), ty: ArgType::U64 }"));
        assert!(!programs.contains("`submit`: arg 'submission' has unsupported type"));
    }

    /// The `use arete_sdk::instruction::{…}` line of the first program module.
    fn instruction_imports(programs: &str) -> Vec<String> {
        let line = programs
            .lines()
            .find(|line| {
                line.trim_start()
                    .starts_with("use arete_sdk::instruction::{")
            })
            .expect("instruction import");
        line.trim()
            .trim_start_matches("use arete_sdk::instruction::{")
            .trim_end_matches("};")
            .split(", ")
            .map(str::to_string)
            .collect()
    }

    /// A program module imports exactly the schema items its instruction
    /// handlers name, so `-D warnings` builds of generated crates stay clean:
    /// struct-only defined types import `ArgField` alone, and only an inlined
    /// enum imports `EnumVariantDef` and `EnumVariantKind`.
    #[test]
    fn rust_program_module_imports_only_the_schema_items_it_emits() {
        let struct_type = |name: &str| IdlTypeDefSnapshot {
            name: name.to_string(),
            docs: vec![],
            serialization: None,
            type_def: IdlTypeDefKindSnapshot::Struct {
                kind: "struct".to_string(),
                fields: vec![IdlFieldSnapshot {
                    name: "value".to_string(),
                    type_: IdlTypeSnapshot::Simple("u64".to_string()),
                    amount_hint: None,
                }],
            },
        };
        let instruction = |name: &str, arg_type: &str| InstructionDef {
            name: name.to_string(),
            discriminator: vec![7],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![instruction_arg("value", arg_type)],
            errors: vec![],
            program_id: Some(TEST_PROGRAM_ID.to_string()),
            docs: vec![],
        };

        let plain = compile_stack_spec(programs_stack_spec(), None).unwrap();
        let imports = instruction_imports(&plain.programs_rs.unwrap());
        assert!(!imports.iter().any(|item| item == "ArgField"));
        assert!(!imports.iter().any(|item| item.starts_with("EnumVariant")));

        let mut spec = programs_stack_spec();
        spec.idls[0].types.push(struct_type("FixedPoint"));
        spec.instructions.push(instruction("swap", "FixedPoint"));
        let structs = compile_stack_spec(spec.clone(), None).unwrap();
        let imports = instruction_imports(&structs.programs_rs.unwrap());
        assert!(imports.iter().any(|item| item == "ArgField"), "{imports:?}");
        assert!(
            !imports.iter().any(|item| item.starts_with("EnumVariant")),
            "{imports:?}"
        );

        spec.idls[0].types.push(IdlTypeDefSnapshot {
            name: "Side".to_string(),
            docs: vec![],
            serialization: None,
            type_def: IdlTypeDefKindSnapshot::Enum {
                kind: "enum".to_string(),
                variants: vec![IdlEnumVariantSnapshot {
                    name: "Bid".to_string(),
                    fields: vec![],
                }],
            },
        });
        spec.instructions.push(instruction("place", "Side"));
        let enums = compile_stack_spec(spec, None).unwrap();
        let imports = instruction_imports(&enums.programs_rs.unwrap());
        for item in ["ArgField", "EnumVariantDef", "EnumVariantKind"] {
            assert!(imports.iter().any(|import| import == item), "{imports:?}");
        }
    }

    #[test]
    fn rust_generator_supports_inline_tuples_from_idl_snapshots() {
        let mut spec = programs_stack_spec();
        spec.idls[0].types = vec![
            IdlTypeDefSnapshot {
                name: "HookableLifecycleEvent".to_string(),
                docs: vec![],
                serialization: None,
                type_def: IdlTypeDefKindSnapshot::Enum {
                    kind: "enum".to_string(),
                    variants: vec![
                        IdlEnumVariantSnapshot {
                            name: "Create".to_string(),
                            fields: vec![],
                        },
                        IdlEnumVariantSnapshot {
                            name: "Transfer".to_string(),
                            fields: vec![],
                        },
                    ],
                },
            },
            IdlTypeDefSnapshot {
                name: "ExternalCheckResult".to_string(),
                docs: vec![],
                serialization: None,
                type_def: IdlTypeDefKindSnapshot::Struct {
                    kind: "struct".to_string(),
                    fields: vec![IdlFieldSnapshot {
                        name: "flags".to_string(),
                        type_: IdlTypeSnapshot::Simple("u32".to_string()),
                        amount_hint: None,
                    }],
                },
            },
            IdlTypeDefSnapshot {
                name: "AgentIdentityInitInfo".to_string(),
                docs: vec![],
                serialization: None,
                type_def: IdlTypeDefKindSnapshot::Struct {
                    kind: "struct".to_string(),
                    fields: vec![IdlFieldSnapshot {
                        name: "lifecycleChecks".to_string(),
                        type_: IdlTypeSnapshot::Vec(IdlVecTypeSnapshot {
                            vec: Box::new(IdlTypeSnapshot::Tuple(IdlTupleTypeSnapshot {
                                tuple: vec![
                                    IdlTypeSnapshot::Defined(IdlDefinedTypeSnapshot {
                                        defined: IdlDefinedInnerSnapshot::Named {
                                            name: "HookableLifecycleEvent".to_string(),
                                        },
                                    }),
                                    IdlTypeSnapshot::Defined(IdlDefinedTypeSnapshot {
                                        defined: IdlDefinedInnerSnapshot::Named {
                                            name: "ExternalCheckResult".to_string(),
                                        },
                                    }),
                                ],
                            })),
                            length_prefix: None,
                        }),
                        amount_hint: None,
                    }],
                },
            },
        ];
        spec.idls[0].instructions.push(IdlInstructionSnapshot {
            name: "tupleThing".to_string(),
            discriminator: vec![7],
            discriminant: None,
            docs: vec![],
            accounts: vec![],
            args: vec![
                IdlFieldSnapshot {
                    name: "payload".to_string(),
                    type_: IdlTypeSnapshot::Defined(IdlDefinedTypeSnapshot {
                        defined: IdlDefinedInnerSnapshot::Named {
                            name: "AgentIdentityInitInfo".to_string(),
                        },
                    }),
                    amount_hint: None,
                },
                IdlFieldSnapshot {
                    name: "pair".to_string(),
                    type_: IdlTypeSnapshot::Tuple(IdlTupleTypeSnapshot {
                        tuple: vec![
                            IdlTypeSnapshot::Simple("u8".to_string()),
                            IdlTypeSnapshot::Simple("u16".to_string()),
                        ],
                    }),
                    amount_hint: None,
                },
            ],
        });
        spec.instructions.push(InstructionDef {
            name: "tupleThing".to_string(),
            discriminator: vec![7],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![
                instruction_arg("payload", "AgentIdentityInitInfo"),
                instruction_arg("pair", "(u8, u16)"),
            ],
            errors: vec![],
            program_id: Some(TEST_PROGRAM_ID.to_string()),
            docs: vec![],
        });

        let output = compile_stack_spec(spec, None).expect("inline tuples should generate");
        let programs = output.programs_rs.expect("program module");
        assert!(programs.contains("pub struct TupleThingParams {"));
        assert!(programs.contains("pub pair: serde_json::Value,"));
        assert!(programs.contains("ArgType::Tuple(vec![ArgType::U8, ArgType::U16])"));
        assert!(programs.contains("ArgType::Vec(Box::new(ArgType::Tuple(vec![ArgType::Enum("));
        assert!(!programs.contains("`tupleThing`: arg 'pair' has unsupported type"));
    }

    #[test]
    fn rust_generator_aliases_arg_account_collisions_and_pda_refs() {
        let mut spec = programs_stack_spec();
        spec.instructions = vec![InstructionDef {
            name: "decompressV1".to_string(),
            discriminator: vec![8],
            discriminator_size: 1,
            accounts: vec![
                instruction_account("metadata", AccountResolution::UserProvided),
                instruction_account(
                    "record",
                    AccountResolution::PdaInline {
                        seeds: vec![PdaSeedDef::AccountRef {
                            account_name: "metadata".to_string(),
                        }],
                        program_id: None,
                        program: None,
                    },
                ),
            ],
            args: vec![instruction_arg("metadata", "u8")],
            errors: vec![],
            program_id: Some(TEST_PROGRAM_ID.to_string()),
            docs: vec![],
        }];

        let output = compile_stack_spec(spec, None).expect("collision should generate");
        let programs = output.programs_rs.expect("program module");
        assert!(programs.contains("pub metadata: u8,"));
        assert!(programs.contains("#[serde(rename = \"metadataAccount\")]"));
        assert!(programs.contains("pub metadata_account: String,"));
        assert!(programs.contains("name: \"metadataAccount\".to_string(),"));
        assert!(programs.contains("PdaSeed::AccountRef(\"metadataAccount\".to_string())"));
        assert!(programs.contains(
            "account `metadata` collides with an instruction arg and is exposed as `metadataAccount`"
        ));
        assert!(!programs.contains("has no typed override field"));
    }

    #[test]
    fn rust_generator_disambiguates_leading_underscore_fields() {
        let mut metadata = snapshot_field(
            "migrationMetadata",
            "MeteoraDammMigrationMetadata",
            true,
            false,
        );
        metadata.resolved_type.as_mut().unwrap().fields = vec![
            resolved_field_of("padding_0", "u8", BaseType::Integer),
            resolved_field_of("_padding_0", "u8", BaseType::Integer),
        ];
        let mut entity = minimal_entity("Migration");
        entity.sections.push(EntitySection {
            name: "root".to_string(),
            fields: vec![metadata],
            is_nested_struct: false,
            parent_field: None,
        });

        let output = compile_stack_spec(stack_of("Migration", entity), None)
            .expect("padding fields should generate");
        let types = output.types_rs;
        assert!(types.contains("pub struct MeteoraDammMigrationMetadata {"));
        assert_eq!(types.matches("    pub padding_0: Option<u64>,").count(), 1);
        assert_eq!(types.matches("    pub _padding_0: Option<u64>,").count(), 1);
    }

    #[test]
    fn rust_generator_emits_program_sdk_module() {
        let output = compile_stack_spec(programs_stack_spec(), None)
            .expect("rust stack generation should succeed");
        let programs = output
            .programs_rs
            .expect("programs.rs should be generated for stacks with instructions");

        assert!(programs.contains("pub mod demo {"));
        assert!(programs.contains(&format!(
            "pub const PROGRAM_ID: &str = \"{}\";",
            TEST_PROGRAM_ID
        )));

        // Typed params: args (with serde renames) then account overrides.
        assert!(programs.contains("pub struct DoThingParams {"));
        assert!(programs.contains("#[serde(rename = \"roundId\")]"));
        assert!(programs.contains("pub round_id: u64,"));
        assert!(programs.contains("pub admin: String,"));
        assert!(programs.contains("pub tip: Option<u64>,"));
        assert!(programs.contains("pub signer: Option<String>,"));
        assert!(programs.contains("pub authority: String,"));
        assert!(programs.contains("#[serde(skip_serializing_if = \"Option::is_none\")]"));

        // Handler literal fragments.
        assert!(programs.contains("discriminator: vec![12, 34]"));
        assert!(programs.contains("resolution: AccountResolution::Signer,"));
        assert!(programs.contains(
            "AccountResolution::Known(\"11111111111111111111111111111111\".to_string())"
        ));
        assert!(programs.contains(
            "AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal(\"counter\".to_string()), PdaSeed::AccountRef(\"authority\".to_string())] })"
        ));
        assert!(programs.contains("ArgSchema { name: \"roundId\".to_string(), ty: ArgType::U64 }"));
        assert!(programs.contains("ty: ArgType::Option(Box::new(ArgType::U64))"));
        assert!(programs.contains("ty: ArgType::Pubkey"));
        assert!(programs.contains(
            "ErrorMetadata { code: 6000, name: \"SlippageExceeded\".to_string(), msg: \"Slippage exceeded\".to_string() }"
        ));

        // PDA helper fn.
        assert!(programs
            .contains("pub fn counter(authority: &str) -> Result<(Pubkey, u8), InstructionError>"));

        // Program accessor struct carries the runtime + typed builder.
        assert!(programs.contains("pub struct DemoProgram {"));
        assert!(programs.contains("builder: arete_sdk::ProgramBuilder,"));
        assert!(
            programs.contains("pub fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self")
        );
        assert!(programs.contains(
            "pub fn do_thing(params: DoThingParams) -> Result<BuiltInstruction, InstructionError>"
        ));
        assert!(programs.contains("pub fn do_thing_handler() -> InstructionHandler"));

        // No program spec recorded: the read layer is omitted with a doc note.
        assert!(programs.contains(
            "/// Program read layer omitted: no program specification was recorded for this program."
        ));
        assert!(!programs.contains("pub const PROGRAM_SPEC_HASH"));
        assert!(!programs.contains("pub fn read_descriptor"));

        // Stack wiring.
        assert!(output
            .entity_rs
            .contains("type Programs = DemoStackPrograms;"));
        assert!(output
            .entity_rs
            .contains("pub demo: crate::programs::demo::DemoProgram,"));
        assert!(output
            .entity_rs
            .contains("demo: crate::programs::demo::DemoProgram::from_builder(builder),"));
        assert!(output
            .entity_rs
            .contains("impl arete_sdk::Programs for DemoStackPrograms"));
        assert!(output.lib_rs.contains("pub mod programs;"));
        assert!(output.lib_rs.contains("DemoStackPrograms"));
    }

    #[test]
    fn rust_program_compiler_emits_no_view_or_stack_shell() {
        let mut stack = programs_stack_spec();
        let mut metadata = snapshot_field(
            "migrationMetadata",
            "MeteoraDammMigrationMetadata",
            true,
            false,
        );
        metadata.resolved_type.as_mut().unwrap().fields = vec![
            resolved_field_of("padding_0", "u8", BaseType::Integer),
            resolved_field_of("_padding_0", "u8", BaseType::Integer),
        ];
        stack.entities[0].sections.push(EntitySection {
            name: "root".to_string(),
            fields: vec![metadata],
            is_nested_struct: false,
            parent_field: None,
        });
        let output = compile_program_modules(
            stack,
            Some(RustStackConfig {
                crate_name: "demo-program".to_string(),
                sdk_version: GENERATED_RUST_SDK_VERSION.to_string(),
                program_reads: vec![RustProgramReadConfig {
                    program_id: TEST_PROGRAM_ID.to_string(),
                    program_spec_hash: "arete:h1:program-spec:sha256:test".to_string(),
                    program_release_hash: "arete:h1:program-release:sha256:test".to_string(),
                    descriptor: Some(serde_json::json!({
                        "release": {
                            "programReleaseHash": "arete:h1:program-release:sha256:test",
                            "programSpecHash": "arete:h1:program-spec:sha256:test"
                        },
                        "transport": {
                            "kind": "hosted-binding",
                            "binding": {
                                "endpoint": "https://reads.example.test",
                                "programReadBindingId": "prb_00000000000000000000000000000001",
                                "auth": {
                                    "sessionEndpoint": "https://auth.example.test/session",
                                    "targetKind": "program-read-binding",
                                    "targetId": "prb_00000000000000000000000000000001"
                                }
                            }
                        }
                    })),
                    package_release_hash: None,
                }],
                ..Default::default()
            }),
        )
        .expect("standalone program generation should succeed");

        assert!(!output.lib_rs.contains("mod entity"));
        assert!(!output.lib_rs.contains("StackViews"));
        assert!(!output.programs_rs.contains("EntityViews"));
        assert!(output.lib_rs.contains("pub use programs::DemoPrograms;"));
        assert!(output
            .programs_rs
            .contains("impl arete_sdk::ProgramSdk for DemoPrograms"));
        assert!(output
            .programs_rs
            .contains("impl arete_sdk::Programs for DemoPrograms"));

        let base = std::env::temp_dir().join(format!(
            "arete-rust-program-codegen-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        write_rust_program_crate(&output, &base).expect("program crate should write");
        assert!(base.join("src/programs.rs").is_file());
        assert!(base.join("src/types.rs").is_file());
        assert!(!base.join("src/entity.rs").exists());

        // Consumer smoke test: compile the generated crate against this
        // checkout's runtime. This catches invalid generated expressions,
        // module paths, re-exports, and dependency declarations.
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("interpreter crate lives in the repo root");
        let runtime_path = repo_root.join("rust/arete-a4-sdk");
        let manifest_path = base.join("Cargo.toml");
        let mut manifest = std::fs::read_to_string(&manifest_path).expect("generated Cargo.toml");
        let generated_dependency = format!(
            "arete-sdk = {{ package = \"arete-a4-sdk\", version = {:?} }}",
            GENERATED_RUST_SDK_VERSION
        );
        assert!(
            manifest.contains(&generated_dependency),
            "generated manifest must expose the expected runtime dependency"
        );
        manifest.push_str(&format!(
            "\n[patch.crates-io]\narete-a4-sdk = {{ path = {:?} }}\n",
            runtime_path
        ));
        std::fs::write(&manifest_path, manifest).expect("localize generated dependency");

        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let checked = Command::new(cargo)
            .args(["check", "--quiet", "--offline", "--manifest-path"])
            .arg(&manifest_path)
            .env("CARGO_TARGET_DIR", base.join("target"))
            .output()
            .expect("cargo must be available for generated consumer smoke tests");
        assert!(
            checked.status.success(),
            "generated standalone Rust crate failed cargo check:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&checked.stdout),
            String::from_utf8_lossy(&checked.stderr),
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rust_program_compiler_keeps_idl_only_programs() {
        let mut spec = programs_stack_spec();
        spec.instructions.clear();
        let output = compile_program_modules(spec, None)
            .expect("an IDL-only standalone program should still be emitted");
        assert!(output.programs_rs.contains("pub mod demo"));
        assert!(output.programs_rs.contains("pub struct DemoProgram"));
        assert!(output.programs_rs.contains("pub struct DemoPrograms"));
    }

    #[test]
    fn rust_generator_without_instructions_binds_unit_programs() {
        let mut spec = programs_stack_spec();
        spec.instructions.clear();

        let output = compile_stack_spec(spec, None).expect("rust stack generation should succeed");

        assert!(output.programs_rs.is_none());
        assert!(output.entity_rs.contains("type Programs = ();"));
        assert!(!output.lib_rs.contains("pub mod programs;"));
        assert!(!output.entity_rs.contains("StackPrograms"));
    }

    #[test]
    fn rust_generator_notes_skipped_instructions() {
        let mut spec = programs_stack_spec();
        spec.instructions.push(InstructionDef {
            name: "badThing".to_string(),
            discriminator: vec![9],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![instruction_arg("payload", "MysteryType")],
            errors: vec![],
            program_id: Some(TEST_PROGRAM_ID.to_string()),
            docs: vec![],
        });

        let output = compile_stack_spec(spec, None).expect("rust stack generation should succeed");
        let programs = output.programs_rs.expect("programs.rs should be generated");

        assert!(programs.contains("/// Skipped instructions (unsupported by instruction codegen):"));
        assert!(
            programs.contains("/// - `badThing`: arg 'payload' has unsupported type 'MysteryType'")
        );
        assert!(!programs.contains("BadThingParams"));
        // The supported instruction is still emitted.
        assert!(programs.contains("pub struct DoThingParams {"));
    }

    #[test]
    fn rust_generator_emits_program_read_layer() {
        // Build a stack spec through the real program-spec pipeline so the
        // recorded ProgramSpecV1 hashes exactly like production specs.
        let idl_json = format!(
            r#"{{
              "address": "{TEST_PROGRAM_ID}",
              "version": "0.1.0",
              "name": "demo",
              "instructions": [
                {{
                  "name": "doThing",
                  "accounts": [{{ "name": "payer", "isMut": true, "isSigner": true }}],
                  "args": [{{ "name": "amount", "type": "u64" }}],
                  "discriminant": {{ "type": "u8", "value": 1 }}
                }}
              ],
              "accounts": [
                {{
                  "name": "Counter",
                  "type": {{
                    "kind": "struct",
                    "fields": [{{ "name": "count", "type": "u64" }}]
                  }}
                }}
              ],
              "types": [],
              "events": [],
              "errors": []
            }}"#
        );
        let mut spec = crate::program_sdk::build_program_only_stack_spec_from_idl_bytes(
            idl_json.as_bytes(),
            None,
            "Demo",
        )
        .expect("program-only stack spec should build");
        let expected_spec_hash = spec.program_specs[0].hash().unwrap().to_string();
        let expected_release_hash = spec.program_specs[0]
            .oss_release_hash()
            .unwrap()
            .to_string();

        // One entity whose raw account struct (`Counter`) is emitted in types.rs.
        let mut entity = minimal_entity("DemoThing");
        entity.sections.push(EntitySection {
            name: "state".to_string(),
            fields: vec![FieldTypeInfo {
                field_name: "counter".to_string(),
                raw_name: Some("counter".to_string()),
                canonical_name: Some("counter".to_string()),
                rust_type_name: "Option<serde_json::Value>".to_string(),
                base_type: BaseType::Object,
                integer_kind: None,
                is_optional: false,
                is_array: false,
                inner_type: Some("Value".to_string()),
                source_path: None,
                resolved_type: Some(ResolvedStructType {
                    type_name: "Counter".to_string(),
                    // The IDL layout of `Counter`, as the entity maps it.
                    fields: vec![resolved_field_of("count", "u64", BaseType::Integer)],
                    is_instruction: false,
                    is_account: true,
                    is_event: false,
                    is_enum: false,
                    enum_variants: vec![],
                }),
                emit: true,
            }],
            is_nested_struct: false,
            parent_field: None,
        });
        spec.entities.push(entity);

        let output = compile_stack_spec(spec, None).expect("rust stack generation should succeed");
        let programs = output.programs_rs.expect("programs.rs should be generated");

        // Release identity consts + descriptor.
        assert!(programs.contains(&format!(
            "pub const PROGRAM_SPEC_HASH: &str = \"{expected_spec_hash}\";"
        )));
        assert!(programs.contains(&format!(
            "pub const PROGRAM_RELEASE_HASH: &str = \"{expected_release_hash}\";"
        )));
        assert!(programs.contains("pub fn read_descriptor() -> arete_sdk::ProgramReadDescriptor"));
        assert!(programs.contains("arete_sdk::ProgramReadDescriptor::LocalHttp"));
        assert!(!programs.contains("Program read layer omitted"));

        // Typed account reader for the emitted `Counter` struct.
        assert!(output.types_rs.contains("pub struct Counter"));
        assert!(programs.contains(
            "pub fn counter_accounts(&self) -> Result<arete_sdk::AccountReader<crate::types::Counter>, arete_sdk::AreteError>"
        ));
        assert!(programs.contains("self.builder.account_transport(\"demo\", &read_descriptor())?"));
        assert!(programs.contains("arete_sdk::AccountReader::new(\n                \"Counter\","));
    }

    /// A keyword name is escaped where it is the whole identifier (`use_`),
    /// and a suffixed name is built from the unescaped stem (`use_handler`,
    /// `type_accounts`): `use__handler` trips `non_snake_case`.
    #[test]
    fn rust_generator_suffixes_keyword_names_from_their_unescaped_stem() {
        let idl_json = format!(
            r#"{{
              "address": "{TEST_PROGRAM_ID}",
              "version": "0.1.0",
              "name": "demo",
              "instructions": [
                {{
                  "name": "use",
                  "accounts": [{{ "name": "authority", "isMut": true, "isSigner": true }}],
                  "args": [{{ "name": "numberOfUses", "type": "u64" }}],
                  "discriminant": {{ "type": "u8", "value": 1 }}
                }}
              ],
              "accounts": [
                {{
                  "name": "Type",
                  "type": {{
                    "kind": "struct",
                    "fields": [{{ "name": "count", "type": "u64" }}]
                  }}
                }}
              ],
              "types": [],
              "events": [],
              "errors": []
            }}"#
        );
        let spec = crate::program_sdk::build_program_only_stack_spec_from_idl_bytes(
            idl_json.as_bytes(),
            None,
            "Demo",
        )
        .expect("program-only stack spec should build");
        let output = compile_program_modules(spec, None).expect("program SDK");
        let programs = output.programs_rs;

        assert!(
            programs.contains("pub fn use_(params: UseParams)"),
            "{programs}"
        );
        assert!(programs.contains("pub fn use_handler() -> InstructionHandler"));
        assert!(programs.contains("use_handler().build(params)"));
        assert!(
            programs.contains("pub fn type_accounts(&self)"),
            "{programs}"
        );
        assert!(!programs.contains("__handler"));
        assert!(!programs.contains("__accounts"));
    }

    #[test]
    fn rust_generator_emits_platform_release_override() {
        let idl_json = format!(
            r#"{{
              "address": "{TEST_PROGRAM_ID}",
              "version": "0.1.0",
              "name": "demo",
              "instructions": [
                {{
                  "name": "doThing",
                  "accounts": [{{ "name": "payer", "isMut": true, "isSigner": true }}],
                  "args": [{{ "name": "amount", "type": "u64" }}],
                  "discriminant": {{ "type": "u8", "value": 1 }}
                }}
              ],
              "accounts": [],
              "types": [],
              "events": [],
              "errors": []
            }}"#
        );
        let spec = crate::program_sdk::build_program_only_stack_spec_from_idl_bytes(
            idl_json.as_bytes(),
            None,
            "Demo",
        )
        .expect("program-only stack spec should build");

        // A platform release override must win over the OSS-derived hash.
        let platform_spec = "arete:h1:program-spec:sha256:platformspec".to_string();
        let platform_release = "arete:h1:program-release:sha256:platformrelease".to_string();
        let config = RustStackConfig {
            program_reads: vec![RustProgramReadConfig {
                program_id: TEST_PROGRAM_ID.to_string(),
                program_spec_hash: platform_spec.clone(),
                program_release_hash: platform_release.clone(),
                descriptor: Some(serde_json::json!({
                    "release": {
                        "programReleaseHash": platform_release.clone(),
                        "programSpecHash": platform_spec.clone(),
                    },
                    "transport": {"kind": "hosted-binding", "binding": {
                        "endpoint": "https://reads.example.test",
                        "programReadBindingId": "prb_00000000000000000000000000000001",
                        "auth": {
                            "sessionEndpoint": "https://auth.example.test/session",
                            "targetKind": "program-read-binding",
                            "targetId": "prb_00000000000000000000000000000001"
                        }
                    }}
                })),
                package_release_hash: None,
            }],
            ..Default::default()
        };

        let output =
            compile_stack_spec(spec, Some(config)).expect("rust stack generation should succeed");
        let programs = output.programs_rs.expect("programs.rs should be generated");

        assert!(programs.contains(&format!(
            "pub const PROGRAM_SPEC_HASH: &str = \"{platform_spec}\";"
        )));
        assert!(programs.contains(&format!(
            "pub const PROGRAM_RELEASE_HASH: &str = \"{platform_release}\";"
        )));
        assert!(programs.contains("pub fn read_descriptor() -> arete_sdk::ProgramReadDescriptor"));
        assert!(programs.contains("\\\"kind\\\":\\\"hosted-binding\\\""));
    }

    #[test]
    fn rust_generator_emits_stack_http_url_override() {
        let output = compile_stack_spec(programs_stack_spec(), None)
            .expect("rust stack generation should succeed");
        assert!(!output.entity_rs.contains("fn http_url"));

        let config = RustStackConfig {
            http_url: Some("https://demo.stack.example".to_string()),
            ..Default::default()
        };
        let output = compile_stack_spec(programs_stack_spec(), Some(config))
            .expect("rust stack generation should succeed");
        assert!(output.entity_rs.contains(
            "fn http_url() -> &'static str {\n        \"https://demo.stack.example\"\n    }"
        ));
    }

    #[test]
    fn rust_generator_wires_extension_modules_after_generated_decls() {
        let config = RustStackConfig {
            module_mode: true,
            extension_modules: vec!["devex".to_string(), "extensions".to_string()],
            extension_entry: Some("extensions".to_string()),
            ..Default::default()
        };
        let output = compile_stack_spec(programs_stack_spec(), Some(config))
            .expect("rust stack generation should succeed");
        let mod_rs = output.mod_rs();

        assert!(mod_rs.contains(
            "// Hand-authored devex extensions (staged from extensions.json; not generated)."
        ));
        let sdk_reexport = mod_rs.find("pub use arete_sdk::").expect("sdk re-export");
        let devex = mod_rs.find("pub mod devex;").expect("devex module decl");
        let entry = mod_rs
            .find("pub mod extensions;")
            .expect("entry module decl");
        let entry_reexport = mod_rs
            .find("pub use extensions::*;")
            .expect("entry glob re-export");
        assert!(sdk_reexport < devex);
        assert!(devex < entry);
        assert!(entry < entry_reexport);
        assert!(!mod_rs.contains("pub use devex::*;"));
    }

    #[test]
    fn rust_generator_omits_extension_wiring_without_entry() {
        let output = compile_stack_spec(programs_stack_spec(), None)
            .expect("rust stack generation should succeed");

        assert!(!output.mod_rs().contains("Hand-authored devex extensions"));
        assert!(!output.mod_rs().contains("pub mod extensions;"));
    }

    #[test]
    fn rust_generator_rejects_extension_module_collisions() {
        for reserved in ["entity", "types", "programs", "generated", "models"] {
            let config = RustStackConfig {
                extension_modules: vec![reserved.to_string(), "extensions".to_string()],
                extension_entry: Some("extensions".to_string()),
                ..Default::default()
            };
            let error = compile_stack_spec(programs_stack_spec(), Some(config))
                .expect_err("collision with a generated module must fail");
            assert!(
                error.contains(&format!("'{reserved}.rs'")),
                "collision error should name the file: {error}"
            );
        }

        let duplicate = RustStackConfig {
            extension_modules: vec![
                "devex".to_string(),
                "devex".to_string(),
                "extensions".to_string(),
            ],
            extension_entry: Some("extensions".to_string()),
            ..Default::default()
        };
        assert!(compile_stack_spec(programs_stack_spec(), Some(duplicate)).is_err());

        let entry_not_last = RustStackConfig {
            extension_modules: vec!["extensions".to_string(), "devex".to_string()],
            extension_entry: Some("extensions".to_string()),
            ..Default::default()
        };
        assert!(compile_stack_spec(programs_stack_spec(), Some(entry_not_last)).is_err());
    }

    #[test]
    fn rust_program_accessors_produce_a_program_context() {
        let programs = compile_stack_spec(programs_stack_spec(), None)
            .expect("rust stack generation should succeed")
            .programs_rs
            .expect("programs.rs should be generated");
        assert!(programs.contains(
            "        pub fn context(&self) -> arete_sdk::ProgramContext<'_, DemoProgram> {\n            arete_sdk::ProgramContext::new(self)\n        }"
        ), "{programs}");
        assert!(programs.contains(
            "    impl arete_sdk::ProgramAccessor for DemoProgram {\n        fn program_builder(&self) -> &arete_sdk::ProgramBuilder {\n            &self.builder\n        }\n    }"
        ), "{programs}");
        assert!(
            !programs.contains("#[allow(dead_code)]\n        builder"),
            "{programs}"
        );

        // An instruction named `context` keeps its builder method name.
        let mut spec = programs_stack_spec();
        spec.instructions[0].name = "context".to_string();
        let programs = compile_stack_spec(spec, None).unwrap().programs_rs.unwrap();
        assert!(
            !programs.contains("arete_sdk::ProgramContext::new(self)"),
            "{programs}"
        );
        assert!(
            programs.contains("pub fn context(&self, params: ContextParams)"),
            "{programs}"
        );
        assert!(
            programs.contains("`context()` is not generated"),
            "{programs}"
        );
        assert!(programs.contains("impl arete_sdk::ProgramAccessor for DemoProgram"));
    }

    #[test]
    fn rust_program_crate_binds_its_bundle_at_the_crate_root() {
        let config = RustStackConfig {
            crate_name: "demo-program".to_string(),
            extension_modules: vec!["demo_math".to_string(), "extensions".to_string()],
            extension_entry: Some("extensions".to_string()),
            ..Default::default()
        };
        let output = compile_program_modules(programs_stack_spec(), Some(config))
            .expect("program crate generation should succeed");
        assert!(output.lib_rs.contains(
            "#[allow(unused_imports)]\nmod generated {\n    pub use super::programs::demo::*;\n    pub use super::types::*;\n}\npub mod demo_math;\npub mod extensions;\npub use extensions::*;\n"
        ), "{}", output.lib_rs);

        // A program bundle must also compile beside a stack program module's
        // generated `pdas`, and never shadow `generated`.
        for reserved in ["pdas", "generated", "models", "views"] {
            let config = RustStackConfig {
                extension_modules: vec![reserved.to_string(), "extensions".to_string()],
                extension_entry: Some("extensions".to_string()),
                ..Default::default()
            };
            let error = compile_program_modules(programs_stack_spec(), Some(config))
                .expect_err("a reserved module name is refused");
            assert!(error.contains(&format!("'{reserved}.rs'")), "{error}");
        }
    }

    #[test]
    fn rust_stack_embeds_a_program_package_extension_in_its_program_module() {
        let config = RustStackConfig {
            program_extensions: vec![RustProgramExtensionConfig {
                program_id: TEST_PROGRAM_ID.to_string(),
                modules: vec!["demo_math".to_string(), "extensions".to_string()],
                entry: "extensions".to_string(),
            }],
            ..Default::default()
        };
        let output = compile_stack_spec(programs_stack_spec(), Some(config))
            .expect("rust stack generation should succeed");
        assert_eq!(
            output.program_modules,
            vec![RustProgramModule {
                program_id: TEST_PROGRAM_ID.to_string(),
                module_name: "demo".to_string(),
            }]
        );
        let programs = output.programs_rs.as_deref().unwrap();
        let wiring = "    // Hand-authored program package extension (staged from programs/demo/extensions.json; not generated).\n    /// Generated items, as the extension bundle beside this module imports them\n    /// (`super::generated::…`).\n    #[allow(unused_imports)]\n    mod generated {\n        pub use super::*;\n        pub use crate::types::*;\n    }\n    pub mod demo_math;\n    pub mod extensions;\n    pub use extensions::*;\n}";
        assert!(programs.contains(wiring), "{programs}");
        // The stack root has no bundle of its own, so no root `generated`.
        assert!(
            !output.lib_rs.contains("mod generated"),
            "{}",
            output.lib_rs
        );

        // Module mode reaches the stack's types from the nested module.
        let config = RustStackConfig {
            module_mode: true,
            program_extensions: vec![RustProgramExtensionConfig {
                program_id: TEST_PROGRAM_ID.to_string(),
                modules: vec!["extensions".to_string()],
                entry: "extensions".to_string(),
            }],
            ..Default::default()
        };
        let programs = compile_stack_spec(programs_stack_spec(), Some(config))
            .unwrap()
            .programs_rs
            .unwrap();
        assert!(
            programs.contains("        pub use super::super::super::types::*;"),
            "{programs}"
        );

        let unknown = RustStackConfig {
            program_extensions: vec![RustProgramExtensionConfig {
                program_id: "Other11111111111111111111111111111111111111".to_string(),
                modules: vec!["extensions".to_string()],
                entry: "extensions".to_string(),
            }],
            ..Default::default()
        };
        let error = compile_stack_spec(programs_stack_spec(), Some(unknown)).unwrap_err();
        assert!(
            error.contains("names a program this stack does not generate"),
            "{error}"
        );

        let pdas = RustStackConfig {
            program_extensions: vec![RustProgramExtensionConfig {
                program_id: TEST_PROGRAM_ID.to_string(),
                modules: vec!["pdas".to_string(), "extensions".to_string()],
                entry: "extensions".to_string(),
            }],
            ..Default::default()
        };
        let error = compile_stack_spec(programs_stack_spec(), Some(pdas)).unwrap_err();
        assert!(error.contains("'pdas.rs' collides"), "{error}");
    }

    /// A program-only spec for the Meteora DLMM fixture: twelve Anchor
    /// accounts declared through `types`, `u128` fields and arrays, PDAs.
    fn dlmm_program_spec() -> SerializableStackSpec {
        crate::program_sdk::build_program_only_stack_spec_from_idl_bytes(
            include_bytes!("../../arete-idl/tests/fixtures/meteora_dlmm.json"),
            None,
            "MeteoraDlmm",
        )
        .expect("the DLMM fixture builds a program-only stack spec")
    }

    #[test]
    fn rust_program_sdk_reads_every_program_spec_account() {
        let spec = dlmm_program_spec();
        let idl = spec.idls[0].clone();
        let spec_pdas = crate::program_sdk::program_spec_pdas(&spec.program_specs[0]);
        assert!(idl.accounts.len() >= 12);
        assert!(
            !spec_pdas.is_empty(),
            "the fixture's ProgramSpec declares PDAs"
        );

        let output = compile_program_modules(spec, None).expect("standalone program SDK");
        for account in &idl.accounts {
            let (reader, model) = (to_snake_case(&account.name), to_pascal_case(&account.name));
            assert!(
                output.programs_rs.contains(&format!(
                    "pub fn {reader}_accounts(&self) -> Result<arete_sdk::AccountReader<crate::types::{model}>, arete_sdk::AreteError>"
                )),
                "no typed reader for {}",
                account.name
            );
            assert!(
                output.types_rs.contains(&format!("pub struct {model} {{")),
                "no model for {}",
                account.name
            );
        }
        // IDL `u128`s decode losslessly from decimal strings.
        assert!(output.types_rs.contains(
            "#[serde(default, deserialize_with = \"serde_utils::deserialize_option_u128\")]\n    pub permission: Option<u128>,"
        ));
        assert!(output.types_rs.contains(
            "#[serde(default, deserialize_with = \"serde_utils::deserialize_option_vec_u128\")]\n    pub liquidity_shares: Option<Vec<u128>>,"
        ));
        // Every ProgramSpec PDA is derivable from `pdas`.
        for pda in spec_pdas.values() {
            assert!(
                output
                    .programs_rs
                    .contains(&format!("        pub fn {}(", to_snake_case(&pda.name))),
                "no PDA helper for {}",
                pda.name
            );
        }
    }

    #[test]
    fn rust_stack_program_module_is_a_superset_of_the_standalone_program_sdk() {
        let standalone = compile_program_modules(dlmm_program_spec(), None).unwrap();

        // A stack over the same program: its entity `LbPair` takes the
        // account's own name, and an entity captures `Oracle` exactly as the
        // IDL lays it out. The stack declares no PDAs of its own.
        let mut spec = dlmm_program_spec();
        let idl = spec.idls[0].clone();
        let program_id = spec.program_ids[0].clone();
        spec.pdas.clear();
        let oracle = idl
            .accounts
            .iter()
            .find(|account| account.name == "Oracle")
            .unwrap();
        let mut entity = minimal_entity("LbPair");
        entity.program_id = Some(program_id.clone());
        entity.sections.push(EntitySection {
            name: "state".to_string(),
            fields: vec![FieldTypeInfo {
                resolved_type: crate::stack_types::idl_account_model(&idl, oracle),
                ..FieldTypeInfo::new("oracle".to_string(), "Oracle".to_string())
            }],
            is_nested_struct: false,
            parent_field: None,
        });
        spec.entities.push(entity);
        let config = RustStackConfig {
            program_extensions: vec![RustProgramExtensionConfig {
                program_id,
                modules: vec!["extensions".to_string()],
                entry: "extensions".to_string(),
            }],
            ..Default::default()
        };
        let output = compile_stack_spec(spec, Some(config)).expect("stack SDK");
        let programs = output.programs_rs.unwrap();

        // Every reader of the standalone SDK, and every PDA helper.
        for line in standalone.programs_rs.lines().filter(|line| {
            line.contains("_accounts(&self)")
                || line.starts_with("        pub fn ") && !line.contains("&self")
        }) {
            let signature = line.split("->").next().unwrap();
            assert!(programs.contains(signature), "stack lacks `{signature}`");
        }
        // `Oracle` shares the entity's type; `LbPair` is renamed around the
        // entity, and `generated` re-exports it under its standalone name.
        assert!(programs.contains("AccountReader<crate::types::Oracle>"));
        assert_eq!(output.types_rs.matches("pub struct Oracle {").count(), 1);
        assert!(programs.contains("AccountReader<crate::types::LbClmmLbPair>"));
        assert!(programs.contains("LbClmmLbPair as LbPair, "), "{programs}");
        assert!(
            programs.contains("        pub use crate::types::{BinArray, "),
            "{programs}"
        );
        assert!(programs.contains(", Oracle, "), "{programs}");

        // The IDL declares `InitializeLbPair2Params` (the type of
        // `initialize_lb_pair2`'s `params` argument), so that instruction's
        // typed params take the TypeScript fallback name in both contexts,
        // and the name stays free for the IDL type.
        for module in [&standalone.programs_rs, &programs] {
            assert!(module.contains("pub struct InitializeLbPair2InstructionParams {"));
            assert!(module.contains(
                "pub fn initialize_lb_pair2(params: InitializeLbPair2InstructionParams)"
            ));
            assert!(!module.contains("pub struct InitializeLbPair2Params {"));
            assert!(module.contains("pub struct InitializeLbPairParams {"));
        }
    }

    /// A standalone program SDK declares every model (its accounts' and the
    /// IDL types they reach) under its stable name, so its `generated`
    /// renames nothing.
    #[test]
    fn rust_standalone_models_are_declared_under_their_stable_names() {
        use crate::idl_models::tests::{mpl_core_stack, ore_stack, pyth_rec_stack};
        for spec in [
            dlmm_program_spec(),
            ore_stack(),
            pyth_rec_stack(),
            mpl_core_stack(),
        ] {
            let programs = spec.idls.iter().map(|idl| idl.name.clone()).collect();
            let (_, models) = generate_stack_types_rs(&[], &[], &spec.idls, &programs);
            let names = models.program_model_names(&spec.idls[0].name);
            assert!(names.len() > spec.idls[0].accounts.len() / 2);
            for (stable, declared) in names {
                assert_eq!(stable, declared, "{}", spec.stack_name);
            }
        }
    }

    #[test]
    fn rust_program_sdk_types_nested_defined_types() {
        let output = compile_program_modules(crate::idl_models::tests::ore_stack(), None).unwrap();
        assert!(output.types_rs.contains(
            "/// IDL type `AdminConfig` as program reads decode it.\n#[derive(Debug, Clone, Serialize, Deserialize, Default)]\npub struct AdminConfig {\n    #[serde(default)]\n    pub authority: Option<String>,\n    #[serde(default)]\n    pub fee_collector: Option<String>,\n    #[serde(default, deserialize_with = \"serde_utils::deserialize_option_u64\")]\n    pub fee_rate: Option<u64>,\n}"
        ), "{}", output.types_rs);
        assert!(output.types_rs.contains(
            "pub struct Config {\n    #[serde(default)]\n    pub admin: Option<AdminConfig>,\n    #[serde(default)]\n    pub protocol: Option<ProtocolConfig>,\n}"
        ));
        assert!(output
            .types_rs
            .contains("    #[serde(default)]\n    pub miner_rewards_factor: Option<Numeric>,"));
        assert!(output.types_rs.contains(
            "pub struct Numeric {\n    #[serde(default, deserialize_with = \"serde_utils::deserialize_option_vec_u64\")]\n    pub bits: Option<Vec<u64>>,\n}"
        ));
        assert!(output
            .programs_rs
            .contains("AccountReader<crate::types::Config>"));
    }

    #[test]
    fn rust_program_sdk_decodes_enums_as_the_wire_tags_them() {
        use crate::idl_models::tests::{mpl_core_stack, pyth_rec_stack};
        let pyth = compile_program_modules(pyth_rec_stack(), None).unwrap();
        // A data variant is a one-key object; its fields accept the IDL's
        // own (camelCase) keys.
        assert!(pyth.types_rs.contains(
            "#[derive(Debug, Clone, Serialize, Deserialize)]\npub enum VerificationLevel {\n    Partial {\n        #[serde(default, deserialize_with = \"serde_utils::deserialize_option_u64\")]\n        #[serde(alias = \"numSignatures\")]\n        num_signatures: Option<u64>,\n    },\n    Full,\n}"
        ), "{}", pyth.types_rs);
        assert!(pyth.types_rs.contains(
            "    #[serde(default)]\n    #[serde(alias = \"verificationLevel\")]\n    pub verification_level: Option<VerificationLevel>,"
        ));
        assert!(pyth
            .programs_rs
            .contains("AccountReader<crate::types::PriceUpdateV2>"));

        let core = compile_program_modules(mpl_core_stack(), None).unwrap();
        // Tuple variants are keyed `field_<index>` on the wire.
        assert!(core.types_rs.contains(
            "pub enum UpdateAuthority {\n    None,\n    Address {\n        #[serde(default)]\n        field_0: Option<String>,\n    },\n    Collection {\n        #[serde(default)]\n        field_0: Option<String>,\n    },\n}"
        ), "{}", core.types_rs);
        assert!(core.types_rs.contains(
            "#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]\npub enum HookableLifecycleEvent {"
        ));
        assert!(core.types_rs.contains(
            "    #[serde(default)]\n    #[serde(alias = \"lifecycleChecks\")]\n    pub lifecycle_checks: Option<Option<Vec<(HookableLifecycleEvent, ExternalCheckResult)>>>,"
        ));
        assert!(core
            .types_rs
            .contains("    #[serde(default)]\n    pub registry: Option<Vec<RegistryRecord>>,"));
    }

    #[test]
    fn rust_models_decode_integers_nested_in_containers() {
        let idl: IdlSnapshot = serde_json::from_value(serde_json::json!({
            "name": "nested",
            "version": "0.1.0",
            "accounts": [{
                "name": "Holder",
                "discriminator": [1, 0, 0, 0, 0, 0, 0, 0],
                "fields": [
                    { "name": "amounts", "type": { "vec": { "option": "u64" } } },
                    { "name": "pair", "type": { "tuple": ["u64", "publicKey"] } },
                    { "name": "balances", "type": { "hashMap": ["string", "u128"] } },
                    { "name": "grid", "type": { "array": [{ "vec": "i64" }, 2] } },
                    { "name": "flags", "type": { "option": { "vec": { "tuple": ["u8", "bool"] } } } },
                    { "name": "names", "type": { "tuple": ["string", "bool"] } }
                ]
            }],
            "instructions": [],
            "types": [],
            "discriminant_size": 8
        }))
        .unwrap();
        let (types_rs, _) =
            generate_stack_types_rs(&[], &[], &[idl], &HashSet::from(["nested".to_string()]));
        for expected in [
            "    #[serde(default, deserialize_with = \"serde_utils::deserialize_wire_option::<_, _, serde_utils::wire::List<serde_utils::wire::Opt<serde_utils::wire::Int>>>\")]\n    pub amounts: Option<Vec<Option<u64>>>,",
            "    #[serde(default, deserialize_with = \"serde_utils::deserialize_wire_option::<_, _, (serde_utils::wire::Int, serde_utils::wire::Plain)>\")]\n    pub pair: Option<(u64, String)>,",
            "    #[serde(default, deserialize_with = \"serde_utils::deserialize_wire_option::<_, _, serde_utils::wire::Map<serde_utils::wire::Int>>\")]\n    pub balances: Option<std::collections::BTreeMap<String, u128>>,",
            "    #[serde(default, deserialize_with = \"serde_utils::deserialize_wire_option::<_, _, serde_utils::wire::List<serde_utils::wire::List<serde_utils::wire::Int>>>\")]\n    pub grid: Option<Vec<Vec<i64>>>,",
            "    #[serde(default, deserialize_with = \"serde_utils::deserialize_wire_option_option::<_, _, serde_utils::wire::List<(serde_utils::wire::Int, serde_utils::wire::Plain)>>\")]\n    pub flags: Option<Option<Vec<(u64, bool)>>>,",
            // No integer inside: its own `Deserialize` suffices.
            "    #[serde(default)]\n    pub names: Option<(String, bool)>,",
        ] {
            assert!(types_rs.contains(expected), "missing:\n{expected}\nin:\n{types_rs}");
        }
    }

    /// A stack keeps the flat types its entities capture accounts into
    /// (nested types as JSON values, unchanged); the account readers read
    /// into their own typed models, named as TypeScript names them, and the
    /// program module's `generated` re-exports every model under its
    /// standalone name.
    #[test]
    fn rust_stack_keeps_entity_types_flat_and_reads_accounts_into_typed_models() {
        let spec = crate::public_artifacts::ore_stack_spec_from_exact_artifacts();
        let config = RustStackConfig {
            program_extensions: vec![RustProgramExtensionConfig {
                program_id: "oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv".to_string(),
                modules: vec!["extensions".to_string()],
                entry: "extensions".to_string(),
            }],
            ..Default::default()
        };
        let output = compile_stack_spec(spec, Some(config)).unwrap();
        assert!(output.types_rs.contains(
            "pub struct Treasury {\n    #[serde(default, deserialize_with = \"serde_utils::deserialize_option_u64\")]\n    pub motherlode: Option<u64>,\n    #[serde(default)]\n    pub miner_rewards_factor: Option<serde_json::Value>,"
        ));
        assert!(output.types_rs.contains(
            "/// Account `Treasury` as program reads decode it.\n#[derive(Debug, Clone, Serialize, Deserialize, Default)]\npub struct OreTreasuryAccount {\n    #[serde(default, deserialize_with = \"serde_utils::deserialize_option_u64\")]\n    pub motherlode: Option<u64>,\n    #[serde(default)]\n    pub miner_rewards_factor: Option<Numeric>,"
        ));
        let programs = output.programs_rs.unwrap();
        assert!(programs.contains(
            "pub fn treasury_accounts(&self) -> Result<arete_sdk::AccountReader<crate::types::OreTreasuryAccount>, arete_sdk::AreteError>"
        ));
        // `Board` renders the same as the entity's and stays one type.
        assert!(programs.contains("AccountReader<crate::types::Board>"));
        let generated = programs
            .lines()
            .find(|line| line.contains("pub use crate::types::{"))
            .expect("the ore module's generated re-exports its models");
        for name in [
            "OreTreasuryAccount as Treasury",
            "OreMinerAccount as Miner",
            "Board",
            "Config",
            "AdminConfig",
            "ProtocolConfig",
            "Numeric",
        ] {
            assert!(
                generated.contains(&format!("{name},")) || generated.contains(&format!("{name}}}")),
                "`generated` lacks {name}: {generated}"
            );
        }
    }

    #[test]
    fn rust_stack_bundle_imports_the_stack_through_generated() {
        let config = RustStackConfig {
            extension_modules: vec!["extensions".to_string()],
            extension_entry: Some("extensions".to_string()),
            ..Default::default()
        };
        let output = compile_stack_spec(programs_stack_spec(), Some(config)).unwrap();
        assert!(output.lib_rs.contains(
            "mod generated {\n    pub use super::entity::*;\n    pub use super::types::*;\n    pub use super::programs;\n}\npub mod extensions;\npub use extensions::*;\n"
        ), "{}", output.lib_rs);
    }

    #[test]
    fn rust_generator_selects_u64_length_prefixed_vec_schema() {
        let mut parser = RustDefinedTypes::new(&[]);
        let vec_of_pubkey = |length_prefix| {
            IdlTypeSnapshot::Vec(IdlVecTypeSnapshot {
                vec: Box::new(IdlTypeSnapshot::Simple("publicKey".to_string())),
                length_prefix,
            })
        };

        assert_eq!(
            parser.parse_snapshot_type(&vec_of_pubkey(None)).schema,
            "ArgType::Vec(Box::new(ArgType::Pubkey))"
        );
        assert_eq!(
            parser
                .parse_snapshot_type(&vec_of_pubkey(Some(arete_idl::types::IdlLengthPrefix::U32)))
                .schema,
            "ArgType::Vec(Box::new(ArgType::Pubkey))"
        );
        assert_eq!(
            parser
                .parse_snapshot_type(&vec_of_pubkey(Some(arete_idl::types::IdlLengthPrefix::U64)))
                .schema,
            "ArgType::VecU64Len(Box::new(ArgType::Pubkey))"
        );
    }

    /// Instruction args round-trip through `InstructionArgDef::arg_type` as a string.
    #[test]
    fn rust_generator_round_trips_u64_length_prefixed_vec_args() {
        let vec_of_pubkey = |length_prefix| {
            IdlTypeSnapshot::Vec(IdlVecTypeSnapshot {
                vec: Box::new(IdlTypeSnapshot::Simple("publicKey".to_string())),
                length_prefix,
            })
        };
        let borsh = idl_type_snapshot_to_rust_string(&vec_of_pubkey(None));
        let bincode = idl_type_snapshot_to_rust_string(&vec_of_pubkey(Some(
            arete_idl::types::IdlLengthPrefix::U64,
        )));
        assert_eq!(borsh, "Vec<solana_pubkey::Pubkey>");
        assert_eq!(bincode, "VecU64Len<solana_pubkey::Pubkey>");

        let mut parser = RustDefinedTypes::new(&[]);
        let parsed = parser.parse_arg_type(&bincode);
        assert_eq!(
            parsed.schema,
            "ArgType::VecU64Len(Box::new(ArgType::Pubkey))"
        );
        assert_eq!(parsed.param_type, "Vec<String>");
        assert_eq!(
            parser.parse_arg_type(&borsh).schema,
            "ArgType::Vec(Box::new(ArgType::Pubkey))"
        );

        let mut spec = programs_stack_spec();
        spec.instructions[0]
            .args
            .push(instruction_arg("newAddresses", &bincode));
        let programs = compile_stack_spec(spec, None)
            .expect("rust stack generation should succeed")
            .programs_rs
            .expect("programs.rs should be generated");
        assert!(
            programs.contains("ArgType::VecU64Len(Box::new(ArgType::Pubkey))"),
            "u64-prefixed vec arg should reach the generated schema:\n{programs}"
        );
    }

    /// Regeneration helper for the checked-in ore example. Run with:
    /// `cargo test -p arete-interpreter regenerate_ore_example -- --ignored`
    ///
    /// Rewrites `examples/ore-rust/src/generated/ore/{mod,types,entity,programs}.rs`
    /// from the checked-in Ore StackManifest artifact closure.
    ///
    /// Extension wiring reuses the `extensions.json` staged in the output
    /// directory (files sorted, entry last, stems via [`rust_module_name`]) —
    /// a faithful replica of the CLI's output-dir manifest resolution step,
    /// which lives in `a4-cli` and cannot be called from this crate. Staged
    /// extension files are preserved verbatim, so a second run is a byte-stable
    /// fixed point.
    #[test]
    #[ignore = "writes into examples/ore-rust; run explicitly to regenerate"]
    fn regenerate_ore_example() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("interpreter crate lives in the repo root")
            .to_path_buf();
        let spec = crate::public_artifacts::ore_stack_spec_from_exact_artifacts();

        let out_dir = repo_root.join("examples/ore-rust/src/generated/ore");
        let (extension_modules, extension_entry) =
            match std::fs::read_to_string(out_dir.join("extensions.json")) {
                Ok(manifest_json) => {
                    let manifest: serde_json::Value = serde_json::from_str(&manifest_json)
                        .expect("staged extensions.json should parse");
                    let language = manifest["language"].as_str();
                    assert!(
                        language.is_none() || language == Some("rust"),
                        "staged ore extensions must be a Rust bundle"
                    );
                    let entry_stem = rust_module_name(
                        manifest["entry"]
                            .as_str()
                            .and_then(|entry| entry.strip_suffix(".rs"))
                            .expect("extensions entry should be a .rs file"),
                    );
                    let mut stems: Vec<String> = manifest["files"]
                        .as_array()
                        .expect("extensions files should be an array")
                        .iter()
                        .map(|file| {
                            rust_module_name(
                                file.as_str()
                                    .and_then(|file| file.strip_suffix(".rs"))
                                    .expect("extension files should be .rs files"),
                            )
                        })
                        .filter(|stem| stem != &entry_stem)
                        .collect();
                    stems.sort();
                    stems.dedup();
                    stems.push(entry_stem.clone());
                    (stems, Some(entry_stem))
                }
                Err(_) => (Vec::new(), None),
            };

        let config = RustStackConfig {
            crate_name: "ore-stack".to_string(),
            sdk_version: GENERATED_RUST_SDK_VERSION.to_string(),
            module_mode: true,
            url: Some("wss://ore.stack.arete.run".to_string()),
            http_url: Some("https://ore.stack.arete.run".to_string()),
            extension_modules,
            extension_entry,
            program_extensions: Vec::new(),
            program_reads: Vec::new(),
            gateway: None,
            release: None,
        };
        let output =
            compile_stack_spec(spec, Some(config)).expect("ore stack should compile to Rust");

        std::fs::write(out_dir.join("mod.rs"), output.mod_rs()).unwrap();
        std::fs::write(out_dir.join("types.rs"), &output.types_rs).unwrap();
        std::fs::write(out_dir.join("entity.rs"), &output.entity_rs).unwrap();
        std::fs::write(
            out_dir.join("programs.rs"),
            output.programs_rs.as_deref().expect("ore has instructions"),
        )
        .unwrap();
    }

    /// Address Lookup Table: `lookup_table` is derived on create only; every
    /// other instruction takes it from the caller, and `extend_lookup_table`
    /// keeps its bincode `u64` vector length prefix.
    #[test]
    fn address_lookup_table_program_sdk_derives_lookup_table_on_create_only() {
        let spec = crate::program_sdk::build_program_only_stack_spec_from_idl_bytes(
            include_bytes!("../../arete-idl/tests/fixtures/address-lookup-table.json"),
            None,
            "AddressLookupTable",
        )
        .expect("ALT program-only stack spec should build");
        let output = compile_program_modules(spec, None).expect("rust program SDK generation");
        let programs = output.programs_rs;

        for tag in 0u8..5 {
            assert!(
                programs.contains(&format!("discriminator: vec![{tag}, 0, 0, 0],")),
                "instruction tag {tag} must be a u32-LE bincode discriminator:\n{programs}"
            );
        }
        assert!(
            programs.contains("ArgType::VecU64Len(Box::new(ArgType::Pubkey))"),
            "extend_lookup_table must keep its u64 vector length prefix:\n{programs}"
        );

        let lookup_table_metas: Vec<&str> = programs
            .split("AccountMeta {")
            .skip(1)
            .filter(|block| {
                block
                    .split("resolution")
                    .next()
                    .map(|head| {
                        head.to_ascii_lowercase()
                            .replace('_', "")
                            .contains("lookuptable")
                    })
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(lookup_table_metas.len(), 5, "{programs}");
        let derived = lookup_table_metas
            .iter()
            .filter(|block| block.contains("resolution: AccountResolution::Pda("))
            .count();
        let caller_provided = lookup_table_metas
            .iter()
            .filter(|block| block.contains("resolution: AccountResolution::UserProvided,"))
            .count();
        assert_eq!((derived, caller_provided), (1, 4), "{programs}");
    }
}

// ============================================================================
// Stack-level compilation (multi-entity)
// ============================================================================

#[derive(Debug, Clone)]
pub struct RustStackConfig {
    pub crate_name: String,
    pub sdk_version: String,
    pub module_mode: bool,
    pub url: Option<String>,
    /// HTTP base URL for the stack (account reads / queries / chain reads).
    /// When set and non-empty, the generated Stack impl overrides
    /// `Stack::http_url`; otherwise the runtime derives the HTTP endpoint
    /// from the WebSocket URL.
    pub http_url: Option<String>,
    /// Module stems of hand-authored devex extension files staged next to the
    /// generated output (one `pub mod <stem>;` each, in order, entry last).
    /// Stems are derived from the staged file names via [`rust_module_name`].
    pub extension_modules: Vec<String>,
    /// Module stem of the extension entry file. When set, the generated
    /// `mod.rs`/`lib.rs` declares a `generated` re-export module for the
    /// bundle (`super::generated`) and re-exports the entry at the SDK root
    /// (`pub use <entry>::*;`), so the bundle's namespace modules resolve as
    /// `<sdk root>::<namespace>`.
    pub extension_entry: Option<String>,
    /// Program package extensions embedded in a stack crate: each is wired
    /// into its program's module in `programs.rs` (files staged under
    /// `programs/<module>/`), with its own `generated` re-export module, so
    /// the bundle compiles unchanged from its standalone program crate.
    pub program_extensions: Vec<RustProgramExtensionConfig>,
    /// Published-platform program read overrides keyed by program ID. When an
    /// entry matches a program, its exact `program_spec_hash` /
    /// `program_release_hash` (from a hosted platform release) are used for
    /// the program read layer instead of the OSS-derived release identity.
    /// Absent programs keep the default OSS/local-HTTP read layer. Published
    /// standalone programs may additionally carry their exact hosted-binding
    /// descriptor so they do not inherit a stack HTTP endpoint.
    pub program_reads: Vec<RustProgramReadConfig>,
    /// Managed-hosting transports. Local generation leaves this unset.
    pub gateway: Option<serde_json::Value>,
    /// Served version emitted as `Stack::stack_manifest_hash` /
    /// `Stack::live_alias`. Only StackManifest generation for a hosted
    /// endpoint sets it; without it the generated impl is unchanged.
    pub release: Option<crate::public_artifacts::StackRelease>,
}

/// One program package's own extension bundle, embedded in a stack crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustProgramExtensionConfig {
    /// The program the bundle extends.
    pub program_id: String,
    /// Module stems of the staged files, helpers first and the entry last
    /// (one `pub mod <stem>;` each inside the program's module).
    pub modules: Vec<String>,
    /// Module stem of the entry, re-exported into the program's module.
    pub entry: String,
}

#[derive(Debug, Clone)]
pub struct RustProgramReadConfig {
    pub program_id: String,
    pub program_spec_hash: String,
    pub program_release_hash: String,
    /// Exact wire descriptor for a published hosted binding. `None` keeps
    /// the local-HTTP descriptor used by locally generated stack SDKs.
    pub descriptor: Option<serde_json::Value>,
    /// The program package release the program SDK was generated from,
    /// emitted as `PACKAGE_RELEASE_HASH`. Local builds leave it unset.
    pub package_release_hash: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RustCompositionConfig {
    pub stack: RustStackConfig,
    pub live_urls: BTreeMap<String, String>,
    /// The served version of an alias whose URL is a deployment of another
    /// StackManifest, such as a composed stack's alias reading its source
    /// stack's deployment. Other bound aliases serve their own alias of this
    /// manifest.
    pub live_releases: BTreeMap<String, crate::public_artifacts::StackRelease>,
}

#[derive(Debug, Clone)]
pub struct RustAliasedStackOutput {
    pub alias: String,
    pub module_name: String,
    pub output: RustOutput,
}

#[derive(Debug, Clone)]
pub struct RustCompositionOutput {
    pub name: String,
    pub cargo_toml: String,
    pub lib_rs: String,
    pub live_stacks: Vec<RustAliasedStackOutput>,
}

impl Default for RustStackConfig {
    fn default() -> Self {
        Self {
            crate_name: "generated-stack".to_string(),
            sdk_version: GENERATED_RUST_SDK_VERSION.to_string(),
            module_mode: false,
            url: None,
            http_url: None,
            extension_modules: Vec::new(),
            extension_entry: None,
            program_extensions: Vec::new(),
            program_reads: Vec::new(),
            gateway: None,
            release: None,
        }
    }
}

/// Compile a full SerializableStackSpec (multi-entity) into unified Rust output.
///
/// Generates types.rs with ALL entity structs, entity.rs with a single Stack impl
/// and per-entity EntityViews, and mod.rs/lib.rs re-exporting everything.
pub fn compile_stack_spec(
    stack_spec: SerializableStackSpec,
    config: Option<RustStackConfig>,
) -> Result<RustOutput, String> {
    compile_stack_spec_with_view_selection(stack_spec, config, false)
}

fn compile_stack_spec_with_view_selection(
    stack_spec: SerializableStackSpec,
    config: Option<RustStackConfig>,
    exact_views: bool,
) -> Result<RustOutput, String> {
    let config = config.unwrap_or_default();
    let stack_name = &stack_spec.stack_name;
    let stack_kebab = to_kebab_case(stack_name);
    // `{stack}Stack`, `{stack}StackViews`, `{stack}StackPrograms`.
    let stack_ident = rust_ident::identifier_stem(stack_name, IdentifierCase::Preserve);

    let mut entity_names: Vec<String> = Vec::new();
    let mut entity_specs: Vec<SerializableStreamSpec> = Vec::new();

    for mut spec in stack_spec.entities {
        if spec.idl.is_none() {
            spec.idl = crate::stack_types::entity_idl(&spec, &stack_spec.idls).cloned();
        }
        entity_names.push(spec.state_name.clone());
        entity_specs.push(spec);
    }
    validate_entity_identifiers(&entity_names)?;

    let view_entity_names = entity_specs
        .iter()
        .zip(&entity_names)
        .filter(|(spec, _)| !exact_views || !spec.views.is_empty())
        .map(|(_, name)| name.clone())
        .collect::<Vec<_>>();

    let reader_programs = rust_reader_programs(
        &stack_spec.idls,
        &stack_spec.program_ids,
        &stack_spec.instructions,
        &stack_spec.program_specs,
        &config.program_reads,
        false,
    );
    let (types_rs, account_structs) = generate_stack_types_rs(
        &entity_specs,
        &entity_names,
        &stack_spec.idls,
        &reader_programs,
    );

    validate_program_extensions(&config.program_extensions)?;
    let programs = generate_stack_programs_rs(
        stack_name,
        &stack_spec.instructions,
        &stack_spec.idls,
        &stack_spec.pdas,
        &stack_spec.program_ids,
        &stack_spec.program_specs,
        &account_structs,
        config.module_mode,
        &config.program_reads,
        false,
        &config.program_extensions,
    );
    check_program_extensions_bound(&config.program_extensions, programs.as_ref())?;
    let entity_rs = generate_stack_entity_rs(
        &stack_ident,
        &stack_kebab,
        &entity_specs,
        &entity_names,
        &config,
        exact_views,
        programs.as_ref(),
    );
    validate_extension_modules(&config, &[])?;
    check_stack_identifiers(
        stack_name,
        &stack_ident,
        &entity_names,
        &types_rs,
        programs.is_some(),
    )?;
    let lib_rs = generate_stack_lib_rs(
        &stack_ident,
        &view_entity_names,
        config.module_mode,
        programs.is_some(),
        &config.extension_modules,
        config.extension_entry.as_deref(),
    );
    let cargo_toml = generate_stack_cargo_toml(&config);
    let program_modules = programs
        .as_ref()
        .map(ProgramsCodegen::program_modules)
        .unwrap_or_default();

    Ok(RustOutput {
        cargo_toml,
        lib_rs,
        types_rs,
        entity_rs,
        programs_rs: programs.map(|codegen| codegen.code),
        program_modules,
    })
}

/// Compile a stack model whose `views` have already been projected by a
/// StackManifest selected-view allowlist.
pub fn compile_stack_spec_with_exact_views(
    stack_spec: SerializableStackSpec,
    config: Option<RustStackConfig>,
) -> Result<RustOutput, String> {
    compile_stack_spec_with_view_selection(stack_spec, config, true)
}

/// Compile only the portable program surface: generated account/model types,
/// instruction builders, PDA helpers, read descriptors, and a `ProgramSdk`
/// aggregate. No entity, view, or stack binding is emitted.
/// The program package release a standalone program SDK was generated from:
/// set when its one program has a registry release, unset for local builds.
fn standalone_package_release_hash<'a>(
    stack_spec: &SerializableStackSpec,
    config: &'a RustStackConfig,
) -> Option<&'a str> {
    let [program_id] = stack_spec.program_ids.as_slice() else {
        return None;
    };
    config
        .program_reads
        .iter()
        .find(|read| &read.program_id == program_id)
        .and_then(|read| read.package_release_hash.as_deref())
}

pub fn compile_program_modules(
    stack_spec: SerializableStackSpec,
    config: Option<RustStackConfig>,
) -> Result<RustProgramOutput, String> {
    let config = config.unwrap_or_default();
    if stack_spec.idls.is_empty() {
        return Err(format!(
            "Stack '{}' carries no IDLs; a program-only SDK has nothing to emit",
            stack_spec.stack_name
        ));
    }

    let entity_names = stack_spec
        .entities
        .iter()
        .map(|entity| entity.state_name.clone())
        .collect::<Vec<_>>();
    validate_entity_identifiers(&entity_names)?;
    if !config.program_extensions.is_empty() {
        return Err(
            "a standalone program SDK carries its package extension at the crate root (extension_modules/extension_entry), not as an embedded program extension".to_string(),
        );
    }
    let reader_programs = rust_reader_programs(
        &stack_spec.idls,
        &stack_spec.program_ids,
        &stack_spec.instructions,
        &stack_spec.program_specs,
        &config.program_reads,
        true,
    );
    let (types_rs, account_structs) = generate_stack_types_rs(
        &stack_spec.entities,
        &entity_names,
        &stack_spec.idls,
        &reader_programs,
    );
    let mut programs = generate_stack_programs_rs(
        &stack_spec.stack_name,
        &stack_spec.instructions,
        &stack_spec.idls,
        &stack_spec.pdas,
        &stack_spec.program_ids,
        &stack_spec.program_specs,
        &account_structs,
        config.module_mode,
        &config.program_reads,
        true,
        &[],
    )
    .ok_or_else(|| {
        format!(
            "Stack '{}' contains no programs to emit",
            stack_spec.stack_name
        )
    })?;

    // The bundle of a program package resolves its bindings at the crate
    // root, and must compile unchanged inside a stack's program module, where
    // the generated `pdas` module sits beside it.
    validate_extension_modules(&config, &["pdas"])?;
    let aggregate_name = format!(
        "{}Programs",
        rust_ident::identifier_stem(&stack_spec.stack_name, IdentifierCase::Pascal)
    );
    // `pub use programs::<Aggregate>` would silently shadow a generated type
    // of the same name re-exported through `pub use types::*`.
    let mut scope = IdentifierScope::new("Rust");
    claim_generated_types(&mut scope, &entity_names, &types_rs)?;
    scope.claim(
        &aggregate_name,
        &format!("stack name '{}'", stack_spec.stack_name),
    )?;
    programs.code.push('\n');
    programs.code.push_str(&generate_programs_accessor_struct(
        &aggregate_name,
        &programs,
        "self",
    ));
    let gateway_impl = rust_gateway_impl(config.gateway.as_ref(), "    ");
    let package_release_impl = standalone_package_release_hash(&stack_spec, &config)
        .map(|hash| {
            format!(
                "\n\n    fn package_release_hash() -> Option<&'static str> {{\n        Some({})\n    }}",
                rust_string_literal(hash)
            )
        })
        .unwrap_or_default();
    programs.code.push_str(&format!(
        "\n\nimpl arete_sdk::ProgramSdk for {aggregate_name} {{\n    fn name() -> &'static str {{\n        {}\n    }}{gateway_impl}{package_release_impl}\n}}\n",
        rust_string_literal(&to_kebab_case(&stack_spec.stack_name)),
    ));

    let mut lib_rs = format!(
        "mod types;\npub mod programs;\n\npub use programs::{aggregate_name};\npub use types::*;\n\npub use arete_sdk::{{ProgramSdk, Programs}};\n"
    );
    // `generated` for the program package's bundle: the program module's
    // items and the generated types, as the same bundle sees them inside a
    // stack's program module.
    let mut generated = vec![
        match programs.modules.as_slice() {
            [program] => format!("super::programs::{}::*", program.module_name),
            _ => "super::programs::*".to_string(),
        },
        "super::types::*".to_string(),
    ];
    if let [idl] = stack_spec.idls.as_slice() {
        generated.extend(account_model_reexports(
            &account_structs,
            Some(&idl.name),
            "super::types",
        ));
    }
    append_rust_extension_exports(
        &mut lib_rs,
        &config.extension_modules,
        config.extension_entry.as_deref(),
        &generated,
    );

    Ok(RustProgramOutput {
        cargo_toml: generate_stack_cargo_toml(&config),
        lib_rs,
        types_rs,
        programs_rs: programs.code,
    })
}

/// Compile Rust output from an explicit StackManifest and its public dependencies.
pub fn compile_public_artifacts(
    programs: &[arete_artifacts::ProgramSpecArtifact],
    live_spec: &arete_artifacts::LiveSpecArtifact,
    manifest: &arete_artifacts::StackManifestArtifact,
    config: Option<RustStackConfig>,
) -> Result<RustOutput, String> {
    let stack_spec =
        crate::public_artifacts::stack_spec_from_artifacts(programs, live_spec, manifest)?;
    compile_stack_spec(stack_spec, config)
}

/// Compile typed V2 public artifacts through the current single-live generator.
pub fn compile_public_artifacts_v2(
    programs: &[arete_artifacts::ProgramSpecArtifact],
    live_spec: &arete_artifacts::LiveSpecArtifactV2,
    manifest: &arete_artifacts::StackManifestArtifactV2,
    config: Option<RustStackConfig>,
) -> Result<RustOutput, String> {
    let stack_spec =
        crate::public_artifacts::stack_spec_from_artifacts_v2(programs, live_spec, manifest)?;
    let mut config = config.unwrap_or_default();
    // The served version belongs to the stack bound to a WebSocket endpoint.
    if config.release.is_none() && config.url.is_some() {
        config.release = crate::public_artifacts::StackRelease::for_single_live(manifest);
    }
    compile_stack_spec_with_view_selection(stack_spec, Some(config), true)
}

/// Generate one namespaced Rust stack module per live alias plus a manifest
/// module that preserves alias boundaries instead of flattening views/adapters.
///
/// Each alias bound to a URL in `live_urls` is generated with its served
/// version (overridden by `live_releases`); unbound aliases get none.
pub fn compile_composed_public_artifacts_v2(
    programs: &[arete_artifacts::ProgramSpecArtifact],
    live_specs: &[(String, arete_artifacts::LiveSpecArtifactV2)],
    manifest: &arete_artifacts::StackManifestArtifactV2,
    config: Option<RustCompositionConfig>,
) -> Result<RustCompositionOutput, String> {
    let composed =
        crate::public_artifacts::stack_specs_from_artifacts_v2(programs, live_specs, manifest)?;
    if composed.live_specs.is_empty() {
        return Err(
            "Rust composition generation requires at least one aliased LiveSpec".to_string(),
        );
    }
    let config = config.unwrap_or_default();
    if !config.stack.extension_modules.is_empty()
        || config.stack.extension_entry.is_some()
        || !config.stack.program_extensions.is_empty()
    {
        return Err(
            "Rust composition SDKs do not support stack or program package extensions; extensions attach to a single-live stack module".to_string(),
        );
    }
    let mut live_stacks = Vec::with_capacity(composed.live_specs.len());
    for live in composed.live_specs {
        let module_name = rust_module_name(&live.alias);
        let mut live_config = config.stack.clone();
        live_config.module_mode = true;
        live_config.url = config.live_urls.get(&live.alias).cloned();
        live_config.release = live_config.url.is_some().then(|| {
            config
                .live_releases
                .get(&live.alias)
                .cloned()
                .unwrap_or_else(|| {
                    crate::public_artifacts::StackRelease::for_alias(manifest, &live.alias)
                })
        });
        let output =
            compile_stack_spec_with_view_selection(live.stack_spec, Some(live_config), true)?;
        live_stacks.push(RustAliasedStackOutput {
            alias: live.alias,
            module_name,
            output,
        });
    }
    let lib_rs = live_stacks
        .iter()
        .map(|live| format!("pub mod {};", live.module_name))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(RustCompositionOutput {
        name: composed.name,
        cargo_toml: generate_stack_cargo_toml(&config.stack),
        lib_rs: format!("{lib_rs}\n"),
        live_stacks,
    })
}

pub fn write_rust_composition_crate(
    output: &RustCompositionOutput,
    crate_dir: &std::path::Path,
) -> Result<(), std::io::Error> {
    let source = crate_dir.join("src");
    std::fs::create_dir_all(&source)?;
    std::fs::write(crate_dir.join("Cargo.toml"), &output.cargo_toml)?;
    std::fs::write(source.join("lib.rs"), &output.lib_rs)?;
    for live in &output.live_stacks {
        write_rust_module(&live.output, &source.join(&live.module_name))?;
    }
    Ok(())
}

pub fn write_rust_composition_module(
    output: &RustCompositionOutput,
    module_dir: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(module_dir)?;
    std::fs::write(module_dir.join("mod.rs"), &output.lib_rs)?;
    for live in &output.live_stacks {
        write_rust_module(&live.output, &module_dir.join(&live.module_name))?;
    }
    Ok(())
}

fn generate_stack_cargo_toml(config: &RustStackConfig) -> String {
    format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
arete-sdk = {{ package = "arete-a4-sdk", version = "{}" }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#,
        config.crate_name, config.sdk_version
    )
}

/// Module names reserved for generated code in every extension bundle
/// (`docs/internal/sdk-core-api.md` §9): the SDK's own modules and the
/// `generated` re-export module the bundle imports through.
pub const RESERVED_EXTENSION_MODULES: &[&str] = &[
    "entity",
    "types",
    "mod",
    "lib",
    "programs",
    "models",
    "views",
    "generated",
];

/// Validate hand-authored extension module stems against the generated
/// module names. Entry-stem collisions are a hard error because the staged
/// file would shadow (or be shadowed by) a generated module.
fn validate_extension_modules(
    config: &RustStackConfig,
    extra_reserved: &[&str],
) -> Result<(), String> {
    if config.extension_modules.is_empty() && config.extension_entry.is_none() {
        return Ok(());
    }
    let Some(entry) = config.extension_entry.as_deref() else {
        return Err("extension modules were configured without an extension entry".to_string());
    };
    validate_extension_stems(&config.extension_modules, entry, extra_reserved)
}

fn validate_extension_stems(
    modules: &[String],
    entry: &str,
    extra_reserved: &[&str],
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for stem in modules {
        if RESERVED_EXTENSION_MODULES.contains(&stem.as_str())
            || extra_reserved.contains(&stem.as_str())
        {
            return Err(format!(
                "extension file '{stem}.rs' collides with the generated '{stem}' module; rename the extension file"
            ));
        }
        if !seen.insert(stem.as_str()) {
            return Err(format!(
                "extension file '{stem}.rs' resolves to the same module name as another staged extension file"
            ));
        }
    }
    if modules.last().map(String::as_str) != Some(entry) {
        return Err(format!(
            "extension entry module '{entry}' must be the last configured extension module"
        ));
    }
    Ok(())
}

/// Program package extensions embedded in a stack: one per program, each a
/// valid bundle that also compiles beside the program module's own `pdas`.
fn validate_program_extensions(extensions: &[RustProgramExtensionConfig]) -> Result<(), String> {
    let mut programs = HashSet::new();
    for extension in extensions {
        if !programs.insert(extension.program_id.as_str()) {
            return Err(format!(
                "program {} has more than one package extension",
                extension.program_id
            ));
        }
        validate_extension_stems(&extension.modules, &extension.entry, &["pdas"])
            .map_err(|error| format!("program {}: {error}", extension.program_id))?;
    }
    Ok(())
}

/// Every embedded program extension must name a program the stack
/// generates.
fn check_program_extensions_bound(
    extensions: &[RustProgramExtensionConfig],
    programs: Option<&ProgramsCodegen>,
) -> Result<(), String> {
    for extension in extensions {
        let bound = programs.is_some_and(|programs| {
            programs
                .modules
                .iter()
                .any(|module| module.program_id == extension.program_id)
        });
        if !bound {
            return Err(format!(
                "program package extension for {} names a program this stack does not generate",
                extension.program_id
            ));
        }
    }
    Ok(())
}

/// Entity names become Rust type names verbatim (and stay verbatim in view
/// IDs), so they must already be identifiers. Both authoring paths guarantee
/// this (Rust structs, and Stack Source entity names matching
/// `[A-Za-z][A-Za-z0-9_]*`); report anything else instead of emitting code
/// that does not compile.
fn validate_entity_identifiers(entity_names: &[String]) -> Result<(), String> {
    match entity_names
        .iter()
        .find(|name| !rust_ident::is_identifier(name))
    {
        Some(name) => Err(format!(
            "entity '{name}' cannot be used as a Rust type name; entity names must be identifiers such as `TokenAccount`"
        )),
        None => Ok(()),
    }
}

/// Report stack-level names that collide with entity or generated type names
/// (a glob `pub use types::*` would otherwise be silently shadowed).
fn check_stack_identifiers(
    stack_name: &str,
    stack_ident: &str,
    entity_names: &[String],
    types_rs: &str,
    has_programs: bool,
) -> Result<(), String> {
    let mut scope = IdentifierScope::new("Rust");
    for entity in entity_names {
        let owner = format!("entity '{entity}'");
        scope.claim(entity, &owner)?;
        scope.claim(&format!("{entity}EntityViews"), &owner)?;
    }
    claim_generated_types(&mut scope, entity_names, types_rs)?;
    let owner = format!("stack name '{stack_name}'");
    scope.claim(&format!("{stack_ident}Stack"), &owner)?;
    scope.claim(&format!("{stack_ident}StackViews"), &owner)?;
    if has_programs {
        scope.claim(&format!("{stack_ident}StackPrograms"), &owner)?;
    }
    Ok(())
}

/// Claim every type `types.rs` declares (entity structs belong to their
/// entity).
fn claim_generated_types(
    scope: &mut IdentifierScope,
    entity_names: &[String],
    types_rs: &str,
) -> Result<(), String> {
    for line in types_rs.lines() {
        let declared = ["pub struct ", "pub enum ", "pub type "]
            .iter()
            .find_map(|keyword| line.strip_prefix(keyword));
        if let Some(declared) = declared {
            let name: String = declared
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect();
            let owner = if entity_names.contains(&name) {
                format!("entity '{name}'")
            } else {
                format!("generated type `{name}`")
            };
            scope.claim(&name, &owner)?;
        }
    }
    Ok(())
}

fn generate_stack_lib_rs(
    stack_name: &str,
    entity_names: &[String],
    _module_mode: bool,
    has_programs: bool,
    extension_modules: &[String],
    extension_entry: Option<&str>,
) -> String {
    let entity_views_exports: Vec<String> = entity_names
        .iter()
        .map(|name| format!("{}EntityViews", name))
        .collect();

    let mut all_exports = format!(
        "{}Stack, {}StackViews, {}",
        stack_name,
        stack_name,
        entity_views_exports.join(", ")
    );
    if has_programs {
        all_exports.push_str(&format!(", {}StackPrograms", stack_name));
    }

    let programs_mod = if has_programs {
        "\npub mod programs;"
    } else {
        ""
    };

    let mut output = format!(
        r#"mod entity;
mod types;{programs_mod}

pub use entity::{{{all_exports}}};
pub use types::*;

pub use arete_sdk::{{ConnectionState, Arete, Stack, Update, Views}};
"#,
        programs_mod = programs_mod,
        all_exports = all_exports
    );

    // `generated` for the stack's bundle: the stack binding, the generated
    // types and the program modules.
    let mut generated = vec![
        "super::entity::*".to_string(),
        "super::types::*".to_string(),
    ];
    if has_programs {
        generated.push("super::programs".to_string());
    }
    append_rust_extension_exports(&mut output, extension_modules, extension_entry, &generated);

    output
}

/// Wire a staged extension bundle into the module that holds it: the
/// `generated` re-export module the bundle imports generated items through
/// (`super::generated::…`), one `pub mod` per staged file, and the entry
/// re-exported so its namespace modules resolve at the SDK root.
fn append_rust_extension_exports(
    output: &mut String,
    extension_modules: &[String],
    extension_entry: Option<&str>,
    generated: &[String],
) {
    if let Some(entry) = extension_entry {
        output.push_str(
            "\n// Hand-authored devex extensions (staged from extensions.json; not generated).\n",
        );
        output.push_str(&render_generated_reexport_module(generated, ""));
        for stem in extension_modules {
            output.push_str(&format!("pub mod {stem};\n"));
        }
        output.push_str(&format!("pub use {entry}::*;\n"));
    }
}

/// `generated` re-exports of a program's account models, and the models of
/// the IDL types they reach, under their stable names (the names its
/// standalone program SDK declares them under). They are explicit, so they
/// win over a same-named item another glob of `generated` brings in, such as
/// a stack entity type.
fn account_model_reexports(
    account_models: &AccountModels,
    program: Option<&str>,
    types_path: &str,
) -> Vec<String> {
    let Some(program) = program else {
        return Vec::new();
    };
    let names = account_models
        .program_model_names(program)
        .into_iter()
        .map(|(stable, model)| {
            if model == stable {
                model
            } else {
                format!("{model} as {stable}")
            }
        })
        .collect::<Vec<_>>();
    match names.as_slice() {
        [] => Vec::new(),
        [name] => vec![format!("{types_path}::{name}")],
        names => vec![format!("{types_path}::{{{}}}", names.join(", "))],
    }
}

/// The `generated` module an extension bundle imports generated items
/// through. It re-exports, and never defines, generated items.
fn render_generated_reexport_module(paths: &[String], indent: &str) -> String {
    let mut module = format!(
        "{indent}/// Generated items, as the extension bundle beside this module imports them\n{indent}/// (`super::generated::…`).\n{indent}#[allow(unused_imports)]\n{indent}mod generated {{\n"
    );
    for path in paths {
        module.push_str(&format!("{indent}    pub use {path};\n"));
    }
    module.push_str(&format!("{indent}}}\n"));
    module
}

/// The struct names `types.rs` declares for entities, their sections and the
/// runtime envelopes and builtin resolver outputs. A resolved type renamed
/// around a same-named, different type never takes one of them.
fn stack_entity_struct_names(
    entity_specs: &[SerializableStreamSpec],
    entity_names: &[String],
) -> Vec<String> {
    let mut names = vec!["EventWrapper".to_string(), "CaptureWrapper".to_string()];
    names.extend(
        BUILTIN_RESOLVER_STRUCTS
            .iter()
            .map(|(name, _)| name.to_string()),
    );
    for (spec, entity_name) in entity_specs.iter().zip(entity_names) {
        names.push(entity_name.clone());
        names.extend(
            spec.sections
                .iter()
                .filter(|section| !RustCompiler::is_root_section(&section.name))
                .map(|section| format!("{}{}", entity_name, to_pascal_case(&section.name))),
        );
    }
    names
}

/// The resolved types an entity's emitted fields reference, by emitted name.
fn emitted_resolved_types<'a>(
    spec: &'a SerializableStreamSpec,
    resolved_name_map: &HashMap<String, String>,
) -> Vec<(String, &'a ResolvedStructType)> {
    spec.sections
        .iter()
        .flat_map(|section| &section.fields)
        .filter(|field| field.emit)
        .filter_map(|field| field.resolved_type.as_ref())
        .map(|resolved| {
            let name = resolved_name_map
                .get(&resolved.type_name)
                .cloned()
                .unwrap_or_else(|| to_pascal_case(&resolved.type_name));
            (name, resolved)
        })
        .collect()
}

/// Generate types.rs containing structs for ALL entities in the stack.
///
/// Also returns the map of emitted raw account structs (IDL account type name
/// -> emitted Rust struct name) so the program SDK generator can attach typed
/// account readers for accounts that actually have a generated struct.
fn generate_stack_types_rs(
    entity_specs: &[SerializableStreamSpec],
    entity_names: &[String],
    idls: &[IdlSnapshot],
    reader_programs: &HashSet<String>,
) -> (String, AccountModels) {
    let mut output = String::new();
    output.push_str("use serde::{Deserialize, Serialize};\n");
    output.push_str("use arete_sdk::serde_utils;\n\n");

    let mut generated = HashSet::new();
    let mut account_structs = AccountModels::default();
    let mut used_builtins: BTreeSet<&'static str> = BTreeSet::new();
    let mut stack_types = StackResolvedTypes::default();
    stack_types.reserve(stack_entity_struct_names(entity_specs, entity_names));

    for (i, spec) in entity_specs.iter().enumerate() {
        let entity_name = &entity_names[i];
        let compiler = RustCompiler::new(spec.clone(), entity_name.clone(), RustConfig::default());
        let resolved_name_map = compiler
            .build_stack_resolved_type_name_map(&stack_types, entity_program_name(spec, idls));
        let program_name = entity_program_name(spec, idls);
        for (name, resolved) in emitted_resolved_types(spec, &resolved_name_map) {
            stack_types.declare(&name, resolved);
            account_structs.declare_model(&name, resolved);
            if resolved.is_account && !resolved.is_enum {
                account_structs.record(program_name, &resolved.type_name, &name);
            }
        }
        used_builtins.extend(compiler.used_builtin_resolver_types());

        // Generate section structs (e.g., OreRoundId, OreRoundState)
        for section in &spec.sections {
            if !RustCompiler::is_root_section(&section.name) {
                let struct_name = format!("{}{}", entity_name, to_pascal_case(&section.name));
                if generated.insert(struct_name) {
                    output.push_str(
                        &compiler.generate_struct_for_section(section, &resolved_name_map),
                    );
                    output.push_str("\n\n");
                }
            }
        }

        // Generate main entity struct (e.g., OreRound, OreTreasury)
        output.push_str(&compiler.generate_main_entity_struct(&resolved_name_map));
        output.push_str("\n\n");

        let resolved = compiler.generate_resolved_types(&resolved_name_map, &mut generated);
        output.push_str(&resolved);
        while !output.ends_with("\n\n") {
            output.push('\n');
        }
    }

    // Account models for the accounts no entity maps (or maps differently),
    // and the IDL types they reach: every account a program module reads
    // gets one.
    let reserved = rust_stable_reserved_names();
    let language = ModelLanguage {
        pascal: &to_pascal_case,
        render: &render_declared_model,
        extra_names: &|_, _| Vec::new(),
        reserved: &reserved,
    };
    let idl_models = bind_idl_models(
        idls,
        reader_programs,
        &mut account_structs,
        &language,
        &|name| stack_types.is_taken(name),
    );
    for (name, model) in &idl_models {
        output.push_str(&format!(
            "/// {} `{}` as program reads decode it.\n",
            if model.is_account {
                "Account"
            } else {
                "IDL type"
            },
            model.type_name
        ));
        output.push_str(&render_idl_model(model, name));
        output.push_str("\n\n");
    }

    // Generate the builtin resolver output structs (SlotHashBytes /
    // TokenMetadata) once, for the whole stack.
    output.push_str(&render_builtin_resolver_structs(&used_builtins));

    // Generate the runtime envelopes (EventWrapper / CaptureWrapper) once.
    output.push('\n');
    output.push_str(WRAPPER_TYPES);

    (output, account_structs)
}

/// Names a program's account and IDL type models never take in any Rust
/// SDK: the runtime envelopes and builtin resolver structs `types.rs` may
/// declare, and the prelude and derive names its declarations use (a model
/// named `Option` would shadow the `Option` every field is wrapped in).
fn rust_stable_reserved_names() -> Vec<&'static str> {
    let mut names = vec!["EventWrapper", "CaptureWrapper"];
    names.extend(BUILTIN_RESOLVER_STRUCTS.iter().map(|(name, _)| *name));
    names.extend([
        "Box",
        "Default",
        "Deserialize",
        "Option",
        "Result",
        "Serialize",
        "String",
        "Vec",
    ]);
    names
}

/// The programs whose module in `programs.rs` gets typed account readers.
fn rust_reader_programs(
    idls: &[IdlSnapshot],
    program_ids: &[String],
    instructions: &[InstructionDef],
    program_specs: &[arete_hash::ProgramSpecV1],
    reads: &[RustProgramReadConfig],
    include_idl_only_programs: bool,
) -> HashSet<String> {
    account_reader_programs(
        idls,
        program_ids,
        instructions,
        include_idl_only_programs,
        |program_id| {
            reads.iter().any(|read| read.program_id == program_id)
                || program_specs
                    .iter()
                    .any(|spec| spec.program_id == program_id)
        },
    )
}

/// Generate entity.rs with a single Stack impl and per-entity EntityViews.
fn generate_stack_entity_rs(
    stack_name: &str,
    stack_kebab: &str,
    entity_specs: &[SerializableStreamSpec],
    entity_names: &[String],
    config: &RustStackConfig,
    exact_views: bool,
    programs: Option<&ProgramsCodegen>,
) -> String {
    let types_import = if config.module_mode {
        "super::types"
    } else {
        "crate::types"
    };

    let selected_entities = entity_specs
        .iter()
        .zip(entity_names)
        .filter(|(spec, _)| !exact_views || !spec.views.is_empty())
        .collect::<Vec<_>>();
    let entity_type_imports = selected_entities
        .iter()
        .map(|(_, name)| (*name).to_string())
        .collect::<Vec<_>>();

    let url_impl = match &config.url {
        Some(url) => format!(
            r#"fn url() -> &'static str {{
        "{}"
    }}"#,
            url
        ),
        None => r#"fn url() -> &'static str {
        "" // TODO: Set URL after first deployment in arete.toml
    }"#
        .to_string(),
    };

    // Optional HTTP base URL override (account reads / queries / chain reads).
    let http_url_impl = match config.http_url.as_deref() {
        Some(http_url) if !http_url.is_empty() => format!(
            r#"

    fn http_url() -> &'static str {{
        "{}"
    }}"#,
            http_url
        ),
        _ => String::new(),
    };
    let release_impl = rust_release_impl(config.release.as_ref(), "    ");
    let gateway_impl = rust_gateway_impl(config.gateway.as_ref(), "    ");

    // StackViews struct fields
    let views_fields: Vec<String> = selected_entities
        .iter()
        .map(|(_, name)| {
            let snake = to_snake_case(name);
            format!("    pub {}: {}EntityViews,", snake, name)
        })
        .collect();

    // Views::from_builder body — clone builder for all but last entity
    let views_builder_fields: Vec<String> = selected_entities
        .iter()
        .enumerate()
        .map(|(i, (_, name))| {
            let snake = to_snake_case(name);
            if i < selected_entities.len() - 1 {
                format!(
                    "            {}: {}EntityViews {{ builder: builder.clone() }},",
                    snake, name
                )
            } else {
                format!("            {}: {}EntityViews {{ builder }},", snake, name)
            }
        })
        .collect();

    // Per-entity EntityViews structs
    let mut entity_views_structs = Vec::new();
    for (i, entity_name) in entity_names.iter().enumerate() {
        let spec = &entity_specs[i];
        if exact_views && spec.views.is_empty() {
            continue;
        }

        let derived: Vec<_> = spec
            .views
            .iter()
            .filter(|v| {
                !v.id.ends_with("/state")
                    && !v.id.ends_with("/list")
                    && v.id.starts_with(entity_name.as_str())
            })
            .collect();

        let mut methods = Vec::new();

        if !exact_views
            || spec
                .views
                .iter()
                .any(|view| view.id == format!("{entity_name}/state"))
        {
            methods.push(format!(
                r#"    pub fn state(&self) -> StateView<{entity}> {{
        StateView::new(
            self.builder.connection().clone(),
            self.builder.store().clone(),
            "{entity}/state".to_string(),
            self.builder.initial_data_timeout(),
        )
    }}"#,
                entity = entity_name
            ));
        }

        if !exact_views
            || spec
                .views
                .iter()
                .any(|view| view.id == format!("{entity_name}/list"))
        {
            methods.push(format!(
                r#"
    pub fn list(&self) -> ViewHandle<{entity}> {{
        self.builder.view("{entity}/list")
    }}"#,
                entity = entity_name
            ));
        }

        // Derived view methods
        for view in &derived {
            let view_name = view.id.split('/').nth(1).unwrap_or("unknown");
            let method_name = to_snake_case(view_name);
            methods.push(format!(
                r#"
    pub fn {method}(&self) -> ViewHandle<{entity}> {{
        self.builder.view("{view_id}")
    }}"#,
                method = method_name,
                entity = entity_name,
                view_id = view.id
            ));
        }

        entity_views_structs.push(format!(
            r#"
pub struct {entity}EntityViews {{
    builder: ViewBuilder,
}}

impl {entity}EntityViews {{
{methods}
}}"#,
            entity = entity_name,
            methods = methods.join("\n")
        ));
    }

    let types_use = if entity_type_imports.is_empty() {
        String::new()
    } else {
        format!(
            "use {types_import}::{{{}}};\n",
            entity_type_imports.join(", ")
        )
    };
    let empty_builder = if selected_entities.is_empty() {
        "        let _ = builder;\n"
    } else {
        ""
    };

    // Program SDK binding: stacks with generated programs bind a generated
    // accessor struct; program-less stacks bind `()`.
    let (programs_assoc, programs_struct) = match programs {
        Some(codegen) => {
            let programs_root = if config.module_mode { "super" } else { "crate" };
            let aggregate_name = format!("{}StackPrograms", stack_name);
            (
                format!("type Programs = {aggregate_name};"),
                format!(
                    "\n{}",
                    generate_programs_accessor_struct(
                        &aggregate_name,
                        codegen,
                        &format!("{programs_root}::programs"),
                    )
                ),
            )
        }
        None => ("type Programs = ();".to_string(), String::new()),
    };

    format!(
        r#"{types_use}use arete_sdk::{{Stack, StateView, ViewBuilder, ViewHandle, Views}};

pub struct {stack}Stack;

impl Stack for {stack}Stack {{
    type Views = {stack}StackViews;
    {programs_assoc}

    fn name() -> &'static str {{
        {stack_kebab}
    }}

    {url_impl}{http_url_impl}{release_impl}{gateway_impl}
}}

pub struct {stack}StackViews {{
{views_fields}
}}

impl Views for {stack}StackViews {{
    fn from_builder(builder: ViewBuilder) -> Self {{
{empty_builder}        Self {{
{views_builder}
        }}
    }}
}}
{entity_views}{programs_struct}"#,
        types_use = types_use,
        stack = stack_name,
        stack_kebab = rust_string_literal(stack_kebab),
        programs_assoc = programs_assoc,
        url_impl = url_impl,
        http_url_impl = http_url_impl,
        release_impl = release_impl,
        gateway_impl = gateway_impl,
        views_fields = views_fields.join("\n"),
        views_builder = views_builder_fields.join("\n"),
        entity_views = entity_views_structs.join("\n"),
        empty_builder = empty_builder,
        programs_struct = programs_struct,
    )
}

fn generate_programs_accessor_struct(
    aggregate_name: &str,
    codegen: &ProgramsCodegen,
    programs_root: &str,
) -> String {
    let fields = codegen
        .modules
        .iter()
        .map(|module| {
            format!(
                "    pub {}: {programs_root}::{}::{},",
                module.module_name, module.module_name, module.struct_name,
            )
        })
        .collect::<Vec<_>>();
    let inits = codegen
        .modules
        .iter()
        .enumerate()
        .map(|(index, module)| {
            let builder_expr = if index < codegen.modules.len() - 1 {
                "builder.clone()"
            } else {
                "builder"
            };
            format!(
                "            {}: {programs_root}::{}::{}::from_builder({builder_expr}),",
                module.module_name, module.module_name, module.struct_name,
            )
        })
        .collect::<Vec<_>>();

    format!(
        r#"pub struct {aggregate_name} {{
{fields}
}}

impl arete_sdk::Programs for {aggregate_name} {{
    fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {{
        Self {{
{inits}
        }}
    }}
}}"#,
        fields = fields.join("\n"),
        inits = inits.join("\n"),
    )
}

// ============================================================================
// Program SDK generation (programs.rs)
// ============================================================================

/// One generated program module and the accessor struct it exports.
#[derive(Debug, Clone)]
pub(crate) struct ProgramModule {
    program_id: String,
    module_name: String,
    struct_name: String,
}

/// Result of generating the `programs` module for a stack.
#[derive(Debug, Clone)]
pub(crate) struct ProgramsCodegen {
    code: String,
    modules: Vec<ProgramModule>,
}

impl ProgramsCodegen {
    fn program_modules(&self) -> Vec<RustProgramModule> {
        self.modules
            .iter()
            .map(|module| RustProgramModule {
                program_id: module.program_id.clone(),
                module_name: module.module_name.clone(),
            })
            .collect()
    }
}

/// Which `arete_sdk::instruction` items a generated program module references.
#[derive(Debug, Default)]
struct ProgramImports {
    account_meta: bool,
    arg_schema: bool,
    /// An inlined struct schema, or a struct enum variant, emits `ArgField`.
    arg_field: bool,
    /// An inlined enum schema emits `EnumVariantDef` and `EnumVariantKind`.
    enum_variant: bool,
    pda: bool,
    error_metadata: bool,
}

fn instruction_snapshot_matches(
    instruction: &InstructionDef,
    snapshot: &IdlInstructionSnapshot,
) -> bool {
    instruction.name == snapshot.name
        && instruction.discriminator == snapshot.discriminator
        && instruction.args.len() == snapshot.args.len()
        && instruction
            .args
            .iter()
            .zip(snapshot.args.iter())
            .all(|(arg, snapshot_arg)| arg.name == snapshot_arg.name)
}

fn find_instruction_snapshot<'a>(
    instruction: &InstructionDef,
    idl: Option<&'a IdlSnapshot>,
) -> Option<&'a IdlInstructionSnapshot> {
    idl?.instructions
        .iter()
        .find(|snapshot| instruction_snapshot_matches(instruction, snapshot))
}

/// A parsed instruction argument type.
#[derive(Debug, Clone)]
struct RustParsedArg {
    /// `ArgType::…` constructor expression for the handler schema.
    schema: String,
    /// Rust type for the typed params struct field.
    param_type: String,
    /// Whether the type is representable by the core serializer.
    supported: bool,
}

fn rust_unsupported() -> RustParsedArg {
    RustParsedArg {
        schema: "ArgType::U8".to_string(),
        param_type: "()".to_string(),
        supported: false,
    }
}

fn rust_prim(schema: &str, param_type: &str) -> RustParsedArg {
    RustParsedArg {
        schema: schema.to_string(),
        param_type: param_type.to_string(),
        supported: true,
    }
}

/// `Stack::stack_manifest_hash` / `Stack::live_alias` overrides naming the
/// served version; nothing when the stack has none.
fn rust_release_impl(
    release: Option<&crate::public_artifacts::StackRelease>,
    indent: &str,
) -> String {
    let Some(release) = release else {
        return String::new();
    };
    format!(
        "\n\n{indent}fn stack_manifest_hash() -> Option<&'static str> {{\n{indent}    Some({})\n{indent}}}\n\n{indent}fn live_alias() -> Option<&'static str> {{\n{indent}    Some({})\n{indent}}}",
        rust_string_literal(&release.stack_manifest_hash),
        rust_string_literal(&release.live_alias),
    )
}

/// Render a Rust string literal (quoted and escaped).
fn rust_gateway_impl(gateway: Option<&serde_json::Value>, indent: &str) -> String {
    let Some(gateway) = gateway else {
        return String::new();
    };
    let json = serde_json::to_string(gateway).expect("gateway descriptor must serialize");
    format!(
        "\n\n{indent}fn gateway() -> Option<arete_sdk::HostedSolanaGatewayBindings> {{\n{indent}    Some(serde_json::from_str({}).expect(\"generated hosted Solana gateway descriptor must be valid\"))\n{indent}}}",
        rust_string_literal(&json),
    )
}

fn rust_string_literal(value: &str) -> String {
    format!("{:?}", value)
}

/// Resolver for IDL-defined types (structs/enums) referenced by instruction
/// args. Resolved types are inlined into arg schemas as `ArgType::Struct` /
/// `ArgType::Enum` expressions; the typed params field for such args is
/// `serde_json::Value`. Mirrors the TypeScript `DefinedTypes` parsing rules.
struct RustDefinedTypes<'a> {
    /// IDL type definitions by name, looked up from the current program.
    types: ProgramTypeDefs<'a>,
    /// Memoized resolutions by IDL name (`None` = unsupported); a program's
    /// own definition of a name is keyed by `(program, name)`.
    resolved: BTreeMap<(Option<usize>, String), Option<RustParsedArg>>,
    /// Types currently being resolved (cycle guard).
    visiting: HashSet<(Option<usize>, String)>,
}

impl<'a> RustDefinedTypes<'a> {
    fn new(idls: &'a [IdlSnapshot]) -> Self {
        RustDefinedTypes {
            types: ProgramTypeDefs::new(idls),
            resolved: BTreeMap::new(),
            visiting: HashSet::new(),
        }
    }

    /// Look defined types up from `program`'s point of view (the program
    /// whose instructions are being generated).
    fn set_program(&mut self, program: Option<usize>) {
        self.types.set_scope(program);
    }

    /// Parse a stringified Rust-ish arg type (what `to_rust_type_string`
    /// produces), resolving bare names against the IDL type definitions.
    fn parse_arg_type(&mut self, raw: &str) -> RustParsedArg {
        let t = raw.trim().trim_start_matches('&').trim();

        if let Some((name, inner)) = split_generic(t) {
            match name {
                "Option" => {
                    let inner = self.parse_arg_type(inner);
                    return RustParsedArg {
                        schema: format!("ArgType::Option(Box::new({}))", inner.schema),
                        param_type: format!("Option<{}>", inner.param_type),
                        supported: inner.supported,
                    };
                }
                "Vec" | "VecU64Len" => {
                    let inner = self.parse_arg_type(inner);
                    return RustParsedArg {
                        schema: format!("ArgType::{}(Box::new({}))", name, inner.schema),
                        param_type: format!("Vec<{}>", inner.param_type),
                        supported: inner.supported,
                    };
                }
                _ => return rust_unsupported(),
            }
        }

        // Fixed-size array: [T; N].
        if let Some(stripped) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if let Some((ty, n)) = stripped.rsplit_once(';') {
                let inner = self.parse_arg_type(ty.trim());
                let n = n.trim();
                if n.parse::<usize>().is_ok() {
                    return RustParsedArg {
                        schema: format!("ArgType::Array(Box::new({}), {})", inner.schema, n),
                        param_type: format!("Vec<{}>", inner.param_type),
                        supported: inner.supported,
                    };
                }
            }
        }

        // Primitive (possibly path-qualified, e.g. solana_pubkey::Pubkey).
        let last = t.rsplit("::").next().unwrap_or(t);
        match last {
            "u8" => rust_prim("ArgType::U8", "u8"),
            "u16" => rust_prim("ArgType::U16", "u16"),
            "u32" => rust_prim("ArgType::U32", "u32"),
            "u64" => rust_prim("ArgType::U64", "u64"),
            // serde_json cannot carry 128-bit integers losslessly; the core
            // serializer accepts decimal strings for them.
            "u128" => rust_prim("ArgType::U128", "String"),
            "i8" => rust_prim("ArgType::I8", "i8"),
            "i16" => rust_prim("ArgType::I16", "i16"),
            "i32" => rust_prim("ArgType::I32", "i32"),
            "i64" => rust_prim("ArgType::I64", "i64"),
            "i128" => rust_prim("ArgType::I128", "String"),
            "f32" => rust_prim("ArgType::F32", "f32"),
            "f64" => rust_prim("ArgType::F64", "f64"),
            "bool" => rust_prim("ArgType::Bool", "bool"),
            "String" | "string" | "str" => rust_prim("ArgType::String", "String"),
            "Pubkey" | "pubkey" | "PublicKey" | "publicKey" => {
                rust_prim("ArgType::Pubkey", "String")
            }
            "bytes" => rust_prim("ArgType::Bytes", "Vec<u8>"),
            _ => self
                .resolve_defined(t)
                .or_else(|| (last != t).then(|| self.resolve_defined(last)).flatten())
                .unwrap_or_else(rust_unsupported),
        }
    }

    /// Parse an IDL snapshot type (used inside struct fields / enum variants).
    fn parse_snapshot_type(&mut self, t: &IdlTypeSnapshot) -> RustParsedArg {
        match t {
            IdlTypeSnapshot::Simple(s) => self.parse_arg_type(s),
            IdlTypeSnapshot::Option(o) => {
                let inner = self.parse_snapshot_type(&o.option);
                RustParsedArg {
                    schema: format!("ArgType::Option(Box::new({}))", inner.schema),
                    param_type: format!("Option<{}>", inner.param_type),
                    supported: inner.supported,
                }
            }
            IdlTypeSnapshot::Vec(v) => {
                let inner = self.parse_snapshot_type(&v.vec);
                let variant = match v.length_prefix {
                    Some(arete_idl::types::IdlLengthPrefix::U64) => "VecU64Len",
                    _ => "Vec",
                };
                RustParsedArg {
                    schema: format!("ArgType::{}(Box::new({}))", variant, inner.schema),
                    param_type: format!("Vec<{}>", inner.param_type),
                    supported: inner.supported,
                }
            }
            IdlTypeSnapshot::Array(arr) => {
                let mut element: Option<RustParsedArg> = None;
                let mut size: Option<u32> = None;
                for part in &arr.array {
                    match part {
                        IdlArrayElementSnapshot::Type(inner) => {
                            element = Some(self.parse_snapshot_type(inner))
                        }
                        IdlArrayElementSnapshot::TypeName(name) => {
                            element = Some(self.parse_arg_type(name))
                        }
                        IdlArrayElementSnapshot::Size(n) => size = Some(*n),
                    }
                }
                match (element, size) {
                    (Some(inner), Some(n)) => RustParsedArg {
                        schema: format!("ArgType::Array(Box::new({}), {})", inner.schema, n),
                        param_type: format!("Vec<{}>", inner.param_type),
                        supported: inner.supported,
                    },
                    _ => rust_unsupported(),
                }
            }
            IdlTypeSnapshot::HashMap(map) => {
                let key = self.parse_snapshot_type(&map.hash_map.0);
                let value = self.parse_snapshot_type(&map.hash_map.1);
                if !key.supported || key.schema != "ArgType::String" || !value.supported {
                    rust_unsupported()
                } else {
                    RustParsedArg {
                        schema: format!(
                            "ArgType::HashMap(Box::new({}), Box::new({}))",
                            key.schema, value.schema
                        ),
                        param_type: "serde_json::Value".to_string(),
                        supported: true,
                    }
                }
            }
            IdlTypeSnapshot::Tuple(tuple) => {
                let elements = tuple
                    .tuple
                    .iter()
                    .map(|element| self.parse_snapshot_type(element))
                    .collect::<Vec<_>>();
                if elements.iter().any(|element| !element.supported) {
                    rust_unsupported()
                } else {
                    RustParsedArg {
                        schema: format!(
                            "ArgType::Tuple(vec![{}])",
                            elements
                                .iter()
                                .map(|element| element.schema.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        param_type: "serde_json::Value".to_string(),
                        supported: true,
                    }
                }
            }
            IdlTypeSnapshot::Defined(d) => {
                let name = match &d.defined {
                    IdlDefinedInnerSnapshot::Named { name } => name.as_str(),
                    IdlDefinedInnerSnapshot::Simple(s) => s.as_str(),
                };
                self.resolve_defined(name).unwrap_or_else(rust_unsupported)
            }
        }
    }

    /// Resolve a bare type name against the IDL type definitions. Returns
    /// `None` when unsupported (unknown, recursive, tuple struct, …).
    fn resolve_defined(&mut self, name: &str) -> Option<RustParsedArg> {
        let found = self.types.lookup(name)?;
        let key = (found.program, found.name.to_string());
        if let Some(cached) = self.resolved.get(&key) {
            return cached.clone();
        }
        if !self.visiting.insert(key.clone()) {
            // Recursive types are not supported by instruction codegen.
            return None;
        }

        let result = match &found.def.type_def {
            IdlTypeDefKindSnapshot::Struct { fields, .. } => self.resolve_struct(fields),
            IdlTypeDefKindSnapshot::TupleStruct { .. } => None,
            IdlTypeDefKindSnapshot::Enum { variants, .. } => self.resolve_enum(variants),
        };
        self.visiting.remove(&key);
        self.resolved.insert(key, result.clone());
        result
    }

    fn resolve_struct(&mut self, fields: &[IdlFieldSnapshot]) -> Option<RustParsedArg> {
        let mut field_exprs: Vec<String> = Vec::new();
        for field in fields {
            let parsed = self.parse_snapshot_type(&field.type_);
            if !parsed.supported {
                return None;
            }
            field_exprs.push(format!(
                "ArgField {{ name: {}.to_string(), ty: {} }}",
                rust_string_literal(&field.name),
                parsed.schema
            ));
        }
        Some(RustParsedArg {
            schema: format!("ArgType::Struct(vec![{}])", field_exprs.join(", ")),
            param_type: "serde_json::Value".to_string(),
            supported: true,
        })
    }

    fn resolve_enum(&mut self, variants: &[IdlEnumVariantSnapshot]) -> Option<RustParsedArg> {
        let mut variant_exprs: Vec<String> = Vec::new();
        for variant in variants {
            let name_literal = rust_string_literal(&variant.name);
            if variant.fields.is_empty() {
                variant_exprs.push(format!(
                    "EnumVariantDef {{ name: {}.to_string(), kind: EnumVariantKind::Unit }}",
                    name_literal
                ));
                continue;
            }

            let named: Vec<_> = variant
                .fields
                .iter()
                .filter_map(|field| match field {
                    IdlEnumVariantFieldSnapshot::Named(field) => Some(field),
                    IdlEnumVariantFieldSnapshot::Tuple(_) => None,
                })
                .collect();

            if named.len() == variant.fields.len() {
                let mut field_exprs: Vec<String> = Vec::new();
                for field in named {
                    let parsed = self.parse_snapshot_type(&field.type_);
                    if !parsed.supported {
                        return None;
                    }
                    field_exprs.push(format!(
                        "ArgField {{ name: {}.to_string(), ty: {} }}",
                        rust_string_literal(&field.name),
                        parsed.schema
                    ));
                }
                variant_exprs.push(format!(
                    "EnumVariantDef {{ name: {}.to_string(), kind: EnumVariantKind::Struct(vec![{}]) }}",
                    name_literal,
                    field_exprs.join(", ")
                ));
            } else if named.is_empty() {
                let mut element_exprs: Vec<String> = Vec::new();
                for field in &variant.fields {
                    let IdlEnumVariantFieldSnapshot::Tuple(ty) = field else {
                        unreachable!("named.is_empty() guarantees tuple fields");
                    };
                    let parsed = self.parse_snapshot_type(ty);
                    if !parsed.supported {
                        return None;
                    }
                    element_exprs.push(parsed.schema);
                }
                variant_exprs.push(format!(
                    "EnumVariantDef {{ name: {}.to_string(), kind: EnumVariantKind::Tuple(vec![{}]) }}",
                    name_literal,
                    element_exprs.join(", ")
                ));
            } else {
                // Mixed named and tuple fields are not supported.
                return None;
            }
        }
        Some(RustParsedArg {
            schema: format!("ArgType::Enum(vec![{}])", variant_exprs.join(", ")),
            param_type: "serde_json::Value".to_string(),
            supported: true,
        })
    }
}

/// Record the schema items an emitted `ArgType` expression names, so the
/// program module imports exactly those: inlined struct types (and struct
/// enum variants) name `ArgField`, inlined enum types `EnumVariantDef` and
/// `EnumVariantKind`.
fn note_schema_imports(schema: &str, needs: &mut ProgramImports) {
    needs.arg_field |= schema.contains("ArgField {");
    needs.enum_variant |= schema.contains("EnumVariantDef {");
}

/// How a mapped account surfaces in the typed params struct.
#[derive(Debug, Clone, Copy, PartialEq)]
enum RustAccountFieldKind {
    /// Signer slot: optional address override (payer fallback applies).
    Signer,
    /// Required user-provided account address.
    Required,
    /// Optional user-provided account address.
    Optional,
}

/// Result of mapping a single instruction account.
struct MappedRustAccount {
    /// `AccountMeta { … },` literal, indented for the handler's accounts vec.
    literal: String,
    /// Params field for caller-supplied addresses.
    field: Option<(String, RustAccountFieldKind)>,
    /// Human-readable notes surfaced in the typed builder's doc comment.
    notes: Vec<String>,
    /// Whether the emitted resolution references `PdaConfig` / `PdaSeed`.
    uses_pda: bool,
}

fn rust_account_meta_literal(
    acc: &InstructionAccountDef,
    emitted_name: &str,
    resolution: &str,
    comment: Option<&str>,
) -> String {
    let mut out = String::new();
    if let Some(comment) = comment {
        out.push_str(&format!("                // [arete codegen] {}\n", comment));
    }
    out.push_str(&format!(
        "                AccountMeta {{\n                    name: {name}.to_string(),\n                    is_signer: {is_signer},\n                    is_writable: {is_writable},\n                    resolution: {resolution},\n                    is_optional: {is_optional},\n                }},",
        name = rust_string_literal(emitted_name),
        is_signer = acc.is_signer,
        is_writable = acc.is_writable,
        resolution = resolution,
        is_optional = acc.is_optional,
    ));
    out
}

fn map_rust_account(
    acc: &InstructionAccountDef,
    pda_lookup: &BTreeMap<&str, &PdaDefinition>,
    account_names: &HashSet<&str>,
    arg_types: &BTreeMap<&str, &str>,
    account_name_map: &BTreeMap<String, String>,
) -> MappedRustAccount {
    let emitted_name = account_name_map
        .get(&acc.name)
        .map(String::as_str)
        .unwrap_or(&acc.name);
    let user_field_kind = if acc.is_optional {
        RustAccountFieldKind::Optional
    } else {
        RustAccountFieldKind::Required
    };
    let degraded = |reason: String| -> MappedRustAccount {
        let note = format!(
            "account `{}` degraded to user-provided ({})",
            acc.name, reason
        );
        MappedRustAccount {
            literal: rust_account_meta_literal(
                acc,
                emitted_name,
                "AccountResolution::UserProvided",
                Some(&note),
            ),
            field: Some((emitted_name.to_string(), user_field_kind)),
            notes: vec![note],
            uses_pda: false,
        }
    };

    match &acc.resolution {
        AccountResolution::Signer => MappedRustAccount {
            literal: rust_account_meta_literal(
                acc,
                emitted_name,
                "AccountResolution::Signer",
                None,
            ),
            field: Some((emitted_name.to_string(), RustAccountFieldKind::Signer)),
            notes: Vec::new(),
            uses_pda: false,
        },
        AccountResolution::Known { address } => MappedRustAccount {
            literal: rust_account_meta_literal(
                acc,
                emitted_name,
                &format!(
                    "AccountResolution::Known({}.to_string())",
                    rust_string_literal(address)
                ),
                None,
            ),
            field: None,
            notes: Vec::new(),
            uses_pda: false,
        },
        AccountResolution::UserProvided => MappedRustAccount {
            literal: rust_account_meta_literal(
                acc,
                emitted_name,
                "AccountResolution::UserProvided",
                None,
            ),
            field: Some((emitted_name.to_string(), user_field_kind)),
            notes: Vec::new(),
            uses_pda: false,
        },
        AccountResolution::PdaInline {
            seeds,
            program_id,
            program,
        } => {
            if program.is_some() {
                return degraded(
                    "uses a dynamic PDA program selector not supported by the Rust low-level resolver"
                        .to_string(),
                );
            }
            match build_rust_pda_config(
                seeds,
                program_id.as_deref(),
                account_names,
                arg_types,
                account_name_map,
            ) {
                Ok((resolution, notes)) => MappedRustAccount {
                    literal: rust_account_meta_literal(acc, emitted_name, &resolution, None),
                    field: None,
                    notes,
                    uses_pda: true,
                },
                Err(reason) => degraded(reason),
            }
        }
        AccountResolution::PdaRef { pda_name } => match pda_lookup.get(pda_name.as_str()) {
            Some(def) => {
                if def.program.is_some() {
                    return degraded(format!(
                        "PDA '{}' uses a dynamic program selector not supported by the Rust low-level resolver",
                        pda_name
                    ));
                }
                match build_rust_pda_config(
                    &def.seeds,
                    def.program_id.as_deref(),
                    account_names,
                    arg_types,
                    account_name_map,
                ) {
                    Ok((resolution, notes)) => MappedRustAccount {
                        literal: rust_account_meta_literal(acc, emitted_name, &resolution, None),
                        field: None,
                        notes,
                        uses_pda: true,
                    },
                    Err(reason) => degraded(format!("PDA '{}': {}", pda_name, reason)),
                }
            }
            None => degraded(format!("references unknown PDA '{}'", pda_name)),
        },
    }
}

/// Build an `AccountResolution::Pda(PdaConfig { … })` expression from seed
/// definitions. Returns `Err(reason)` when the PDA cannot be represented by
/// the core resolver, so the caller can degrade to user-provided.
fn build_rust_pda_config(
    seeds: &[PdaSeedDef],
    program_id: Option<&str>,
    account_names: &HashSet<&str>,
    arg_types: &BTreeMap<&str, &str>,
    account_name_map: &BTreeMap<String, String>,
) -> Result<(String, Vec<String>), String> {
    let mut seed_exprs: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for seed in seeds {
        match seed {
            PdaSeedDef::Literal { value } => {
                seed_exprs.push(format!(
                    "PdaSeed::Literal({}.to_string())",
                    rust_string_literal(value)
                ));
            }
            PdaSeedDef::Bytes { value } => {
                let bytes: Vec<String> = value.iter().map(|b| b.to_string()).collect();
                seed_exprs.push(format!("PdaSeed::Bytes(vec![{}])", bytes.join(", ")));
            }
            PdaSeedDef::AccountRef { account_name } => {
                if account_name.contains('.') {
                    return Err(format!(
                        "seed references account field '{}' which is not supported for auto-resolution",
                        account_name
                    ));
                }
                if !account_names.contains(account_name.as_str()) {
                    return Err(format!(
                        "seed references account '{}' not present in this instruction",
                        account_name
                    ));
                }
                seed_exprs.push(format!(
                    "PdaSeed::AccountRef({}.to_string())",
                    rust_string_literal(account_name_map.get(account_name).unwrap_or(account_name))
                ));
            }
            PdaSeedDef::ArgRef { arg_name, arg_type } => {
                let arg_root = arg_name.split('.').next().unwrap_or(arg_name.as_str());
                let present =
                    arg_types.contains_key(arg_name.as_str()) || arg_types.contains_key(arg_root);
                // Prefer the seed's declared type; fall back to the
                // instruction arg's type (Anchor seeds carry no type info).
                let raw_type = arg_type
                    .as_deref()
                    .or_else(|| arg_types.get(arg_name.as_str()).copied())
                    .or_else(|| arg_types.get(arg_root).copied());
                let canonical = raw_type.and_then(normalize_seed_arg_type);
                if !present {
                    if canonical.is_none() {
                        return Err(format!(
                            "seed helper arg '{}' is not present in this instruction and has no primitive type information",
                            arg_name
                        ));
                    }
                    notes.push(format!(
                        "seed input `{}` must be supplied via the `resolve` key when building through the raw handler",
                        arg_name
                    ));
                }
                match canonical {
                    Some(canonical) => seed_exprs.push(format!(
                        "PdaSeed::ArgRef {{ arg: {}.to_string(), arg_type: Some({}.to_string()) }}",
                        rust_string_literal(arg_name),
                        rust_string_literal(&canonical)
                    )),
                    None => {
                        notes.push(format!(
                            "seed arg `{}` has non-primitive type '{}'; the runtime will use heuristic encoding",
                            arg_name,
                            raw_type.unwrap_or("<unknown>")
                        ));
                        seed_exprs.push(format!(
                            "PdaSeed::ArgRef {{ arg: {}.to_string(), arg_type: None }}",
                            rust_string_literal(arg_name)
                        ));
                    }
                }
            }
        }
    }

    let program_expr = match program_id {
        Some(pid) => format!("Some({}.to_string())", rust_string_literal(pid)),
        None => "None".to_string(),
    };
    Ok((
        format!(
            "AccountResolution::Pda(PdaConfig {{ program_id: {}, seeds: vec![{}] }})",
            program_expr,
            seed_exprs.join(", ")
        ),
        notes,
    ))
}

/// Generated code for one instruction: module items plus the accessor method.
struct RustInstructionBlock {
    code: String,
    method: String,
    /// The typed params struct's name, claimed once the block is emitted.
    params_name: String,
}

/// Names a program module's typed params structs: `<Ix>Params`, unless the
/// program's IDL declares a type or account of that name (meteora-dlmm's
/// `InitializeLbPair2Params` is the type of `initialize_lb_pair2`'s `params`
/// argument) or another instruction's params took it. Then the struct is
/// `<Ix>InstructionParams`, numbered from 2 if that is taken too, as the
/// TypeScript generator names it. Only the program's own IDL decides, so a
/// standalone program crate and a stack's program module name every params
/// struct alike, and an extension bundle that declares the IDL type under its
/// own name compiles in both.
struct RustParamsNames {
    /// Pascal-case names of the program's IDL types and accounts.
    declared: HashSet<String>,
    /// Params struct names already emitted in the module.
    taken: HashSet<String>,
}

impl RustParamsNames {
    fn new(idl: Option<&IdlSnapshot>) -> Self {
        let declared = idl
            .map(|idl| {
                idl.types
                    .iter()
                    .map(|def| to_pascal_case(&def.name))
                    .chain(
                        idl.accounts
                            .iter()
                            .map(|account| to_pascal_case(&account.name)),
                    )
                    .collect()
            })
            .unwrap_or_default();
        RustParamsNames {
            declared,
            taken: HashSet::new(),
        }
    }

    fn available(&self, name: &str) -> bool {
        !self.declared.contains(name) && !self.taken.contains(name)
    }

    /// The name the params struct of instruction `pascal` takes.
    fn candidate(&self, pascal: &str) -> String {
        let preferred = format!("{pascal}Params");
        if self.available(&preferred) {
            return preferred;
        }
        let fallback = format!("{pascal}InstructionParams");
        let mut candidate = fallback.clone();
        let mut counter = 2;
        while !self.available(&candidate) {
            candidate = format!("{fallback}{counter}");
            counter += 1;
        }
        candidate
    }

    fn claim(&mut self, name: String) {
        self.taken.insert(name);
    }
}

fn generate_rust_instruction_block(
    instr: &InstructionDef,
    instruction_snapshot: Option<&IdlInstructionSnapshot>,
    errors: &[IdlErrorSnapshot],
    pda_lookup: &BTreeMap<&str, &PdaDefinition>,
    parser: &mut RustDefinedTypes<'_>,
    params_names: &RustParamsNames,
    needs: &mut ProgramImports,
) -> Result<RustInstructionBlock, String> {
    // --- Parse args; skip the whole instruction on unsupported types. ---
    let mut parsed_args: Vec<(&InstructionArgDef, RustParsedArg)> = Vec::new();
    for (index, arg) in instr.args.iter().enumerate() {
        let parsed = instruction_snapshot
            .and_then(|snapshot| snapshot.args.get(index))
            .map(|snapshot_arg| parser.parse_snapshot_type(&snapshot_arg.type_))
            .unwrap_or_else(|| parser.parse_arg_type(&arg.arg_type));
        if !parsed.supported {
            return Err(format!(
                "arg '{}' has unsupported type '{}'",
                arg.name, arg.arg_type
            ));
        }
        parsed_args.push((arg, parsed));
    }

    // --- Map accounts. ---
    let account_names: HashSet<&str> = instr.accounts.iter().map(|a| a.name.as_str()).collect();
    let arg_types: BTreeMap<&str, &str> = instr
        .args
        .iter()
        .map(|a| (a.name.as_str(), a.arg_type.as_str()))
        .collect();

    let mut account_literals: Vec<String> = Vec::new();
    let mut account_fields: Vec<(String, RustAccountFieldKind)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let account_name_map = disambiguate_instruction_account_names(instr);
    for (source_name, emitted_name) in &account_name_map {
        if source_name != emitted_name {
            notes.push(format!(
                "account `{}` collides with an instruction arg and is exposed as `{}`",
                source_name, emitted_name
            ));
        }
    }
    for acc in &instr.accounts {
        let mapped = map_rust_account(
            acc,
            pda_lookup,
            &account_names,
            &arg_types,
            &account_name_map,
        );
        account_literals.push(mapped.literal);
        if let Some(field) = mapped.field {
            account_fields.push(field);
        }
        notes.extend(mapped.notes);
        if mapped.uses_pda {
            needs.pda = true;
        }
    }
    if !instr.accounts.is_empty() {
        needs.account_meta = true;
    }
    if !instr.args.is_empty() {
        needs.arg_schema = true;
    }
    if !errors.is_empty() {
        needs.error_metadata = true;
    }

    let fn_name = to_snake_case(&instr.name);
    // Suffixed from the unescaped stem: `use` builds with `use_` and its
    // handler is `use_handler`.
    let handler_name = format!("{}_handler", to_snake_stem(&instr.name));
    let pascal = to_pascal_case(&instr.name);
    let params_name = params_names.candidate(&pascal);
    if params_name != format!("{pascal}Params") {
        notes.push(format!(
            "params are `{params_name}`: the program declares `{pascal}Params`, or another instruction's params use the name"
        ));
    }

    // --- Typed params struct: args first, then caller-supplied accounts.
    // Account collisions have already been assigned explicit aliases above;
    // `used_field_names` remains a defensive check for Rust-normalization
    // collisions between otherwise-distinct source names. ---
    let mut used_field_names: HashSet<String> = HashSet::new();
    let mut param_fields: Vec<String> = Vec::new();
    for (arg, parsed) in &parsed_args {
        let field_name = to_snake_case(&arg.name);
        used_field_names.insert(field_name.clone());
        note_schema_imports(&parsed.schema, needs);
        let mut lines = Vec::new();
        if field_name != arg.name {
            lines.push(format!(
                "        #[serde(rename = {})]",
                rust_string_literal(&arg.name)
            ));
        }
        lines.push(format!(
            "        pub {}: {},",
            field_name, parsed.param_type
        ));
        param_fields.push(lines.join("\n"));
    }
    for (name, kind) in &account_fields {
        let field_name = to_snake_case(name);
        if !used_field_names.insert(field_name.clone()) {
            notes.push(format!(
                "account `{}` collides with another params field and has no typed override field",
                name
            ));
            continue;
        }
        let mut lines = Vec::new();
        match kind {
            RustAccountFieldKind::Signer => lines.push(format!(
                "        /// Optional address override for the `{}` signer (defaults to the payer).",
                name
            )),
            RustAccountFieldKind::Required => {
                lines.push(format!("        /// Address of the `{}` account.", name))
            }
            RustAccountFieldKind::Optional => lines.push(format!(
                "        /// Optional address of the `{}` account.",
                name
            )),
        }
        if field_name != *name {
            lines.push(format!(
                "        #[serde(rename = {})]",
                rust_string_literal(name)
            ));
        }
        match kind {
            RustAccountFieldKind::Required => {
                lines.push(format!("        pub {}: String,", field_name))
            }
            _ => {
                lines.push(
                    "        #[serde(skip_serializing_if = \"Option::is_none\")]".to_string(),
                );
                lines.push(format!("        pub {}: Option<String>,", field_name));
            }
        }
        param_fields.push(lines.join("\n"));
    }

    let params_struct = if param_fields.is_empty() {
        format!(
            "    /// Typed params for `{name}` (no args or caller-supplied accounts).\n    #[derive(Debug, Clone, Serialize, Default)]\n    pub struct {params_name} {{}}",
            name = instr.name,
            params_name = params_name
        )
    } else {
        format!(
            "    /// Typed params for `{name}`: instruction args plus overridable accounts.\n    #[derive(Debug, Clone, Serialize, Default)]\n    pub struct {params_name} {{\n{fields}\n    }}",
            name = instr.name,
            params_name = params_name,
            fields = param_fields.join("\n")
        )
    };

    // --- Typed builder fn. ---
    let mut doc_lines: Vec<String> = instr
        .docs
        .iter()
        .map(|line| line.trim().to_string())
        .collect();
    if doc_lines.is_empty() {
        doc_lines.push(format!("Builds the `{}` instruction.", instr.name));
    }
    if !notes.is_empty() {
        doc_lines.push(String::new());
        doc_lines.push("Codegen notes:".to_string());
        for note in &notes {
            doc_lines.push(format!("- {}", note));
        }
    }
    let docs = doc_lines
        .iter()
        .map(|line| {
            if line.is_empty() {
                "    ///".to_string()
            } else {
                format!("    /// {}", line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let typed_fn = format!(
        "{docs}\n    pub fn {fn_name}(params: {params_name}) -> Result<BuiltInstruction, InstructionError> {{\n        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {{\n            context: \"params\".to_string(),\n            message: error.to_string(),\n        }})?;\n        {handler_name}().build(params)\n    }}",
        docs = docs,
        fn_name = fn_name,
        handler_name = handler_name,
        params_name = params_name
    );

    // --- Handler literal. ---
    let discriminator = instr
        .discriminator
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let accounts_literal = if account_literals.is_empty() {
        "vec![]".to_string()
    } else {
        format!("vec![\n{}\n            ]", account_literals.join("\n"))
    };
    let args_literal = if parsed_args.is_empty() {
        "vec![]".to_string()
    } else {
        let entries: Vec<String> = parsed_args
            .iter()
            .map(|(arg, parsed)| {
                format!(
                    "                ArgSchema {{ name: {}.to_string(), ty: {} }},",
                    rust_string_literal(&arg.name),
                    parsed.schema
                )
            })
            .collect();
        format!("vec![\n{}\n            ]", entries.join("\n"))
    };
    let errors_literal = if errors.is_empty() {
        "vec![]".to_string()
    } else {
        let entries: Vec<String> = errors
            .iter()
            .map(|error| {
                format!(
                    "                ErrorMetadata {{ code: {}, name: {}.to_string(), msg: {}.to_string() }},",
                    error.code,
                    rust_string_literal(&error.name),
                    rust_string_literal(error.msg.as_deref().unwrap_or(""))
                )
            })
            .collect();
        format!("vec![\n{}\n            ]", entries.join("\n"))
    };

    let handler_fn = format!(
        "    /// Raw instruction handler for `{name}`.\n    pub fn {handler_name}() -> InstructionHandler {{\n        InstructionHandler {{\n            program_id: PROGRAM_ID.to_string(),\n            discriminator: vec![{discriminator}],\n            accounts: {accounts},\n            args: {args},\n            errors: {errors},\n        }}\n    }}",
        name = instr.name,
        handler_name = handler_name,
        discriminator = discriminator,
        accounts = accounts_literal,
        args = args_literal,
        errors = errors_literal
    );

    let method = format!(
        "        pub fn {fn_name}(&self, params: {params_name}) -> Result<BuiltInstruction, InstructionError> {{\n            {fn_name}(params)\n        }}",
        fn_name = fn_name,
        params_name = params_name
    );

    Ok(RustInstructionBlock {
        code: format!("{}\n\n{}\n\n{}", params_struct, typed_fn, handler_fn),
        method,
        params_name,
    })
}

/// Generate the `pdas` helper module for one program. Returns `None` when the
/// program declares no PDAs.
fn generate_rust_pdas_module(pdas: &BTreeMap<String, PdaDefinition>) -> Option<String> {
    if pdas.is_empty() {
        return None;
    }

    let mut fns: Vec<String> = Vec::new();
    let mut needs_serialize = false;
    let mut needs_program_id = false;
    for def in pdas.values() {
        let fn_name = to_snake_case(&def.name);
        let mut params: Vec<(String, String)> = Vec::new();
        let mut seed_exprs: Vec<String> = Vec::new();
        for seed in &def.seeds {
            match seed {
                PdaSeedDef::Literal { value } => {
                    seed_exprs.push(format!(
                        "{}.as_bytes().to_vec()",
                        rust_string_literal(value)
                    ));
                }
                PdaSeedDef::Bytes { value } => {
                    let bytes: Vec<String> = value.iter().map(|b| b.to_string()).collect();
                    seed_exprs.push(format!("vec![{}]", bytes.join(", ")));
                }
                PdaSeedDef::AccountRef { account_name } => {
                    let param = to_snake_case(account_name);
                    if !params.iter().any(|(name, _)| *name == param) {
                        params.push((param.clone(), "&str".to_string()));
                    }
                    needs_serialize = true;
                    seed_exprs.push(format!(
                        "serialize_seed_value(&serde_json::json!({}), Some(\"pubkey\"))?",
                        param
                    ));
                }
                PdaSeedDef::ArgRef { arg_name, arg_type } => {
                    let param = to_snake_case(arg_name);
                    let canonical = arg_type.as_deref().and_then(normalize_seed_arg_type);
                    let (param_type, hint) = match canonical.as_deref() {
                        Some("pubkey") => ("&str", "Some(\"pubkey\")".to_string()),
                        Some("string") => ("&str", "Some(\"string\")".to_string()),
                        Some(int) if int.starts_with('i') => {
                            ("i64", format!("Some({})", rust_string_literal(int)))
                        }
                        Some(int) => ("u64", format!("Some({})", rust_string_literal(int))),
                        None => ("&str", "None".to_string()),
                    };
                    if !params.iter().any(|(name, _)| *name == param) {
                        params.push((param.clone(), param_type.to_string()));
                    }
                    needs_serialize = true;
                    seed_exprs.push(format!(
                        "serialize_seed_value(&serde_json::json!({}), {})?",
                        param, hint
                    ));
                }
            }
        }

        let program_expr = match (&def.program_id, &def.program) {
            (Some(pid), _) => rust_string_literal(pid),
            (None, Some(PdaProgramDef::AccountRef { account_name })) => {
                let param = to_snake_case(account_name);
                if !params.iter().any(|(name, _)| *name == param) {
                    params.push((param.clone(), "&str".to_string()));
                }
                param
            }
            (None, Some(PdaProgramDef::ArgRef { arg_name })) => {
                let param = to_snake_case(arg_name);
                if !params.iter().any(|(name, _)| *name == param) {
                    params.push((param.clone(), "&str".to_string()));
                }
                param
            }
            (None, None) => {
                needs_program_id = true;
                "PROGRAM_ID".to_string()
            }
        };
        let param_list = params
            .iter()
            .map(|(name, ty)| format!("{}: {}", name, ty))
            .collect::<Vec<_>>()
            .join(", ");
        let seeds_body = if seed_exprs.is_empty() {
            "            let seeds: Vec<Vec<u8>> = vec![];".to_string()
        } else {
            format!(
                "            let seeds: Vec<Vec<u8>> = vec![\n{}\n            ];",
                seed_exprs
                    .iter()
                    .map(|expr| format!("                {},", expr))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };
        fns.push(format!(
            "        /// Derive the `{name}` PDA (returns the address and bump).\n        pub fn {fn_name}({params}) -> Result<(Pubkey, u8), InstructionError> {{\n{seeds}\n            derive_program_address(&seeds, {program})\n        }}",
            name = def.name,
            fn_name = fn_name,
            params = param_list,
            seeds = seeds_body,
            program = program_expr
        ));
    }

    let mut imports = vec!["derive_program_address"];
    if needs_serialize {
        imports.push("serialize_seed_value");
    }
    imports.extend(["InstructionError", "Pubkey"]);
    imports.sort_unstable();
    let mut use_lines = format!(
        "        use arete_sdk::instruction::{{{}}};",
        imports.join(", ")
    );
    if needs_program_id {
        use_lines.push_str("\n\n        use super::PROGRAM_ID;");
    }

    Some(format!(
        "    /// PDA derivation helpers for this program.\n    pub mod pdas {{\n{use_lines}\n\n{fns}\n    }}",
        use_lines = use_lines,
        fns = fns.join("\n\n")
    ))
}

/// Release identity computed at generation time for one program, or the
/// reason the program's read layer is omitted.
type ProgramReadLayer = Result<(String, String, Option<serde_json::Value>), String>;

/// Resolve the release identity (`PROGRAM_SPEC_HASH`, `PROGRAM_RELEASE_HASH`)
/// for one program from the stack's recorded program specs.
fn resolve_program_read_layer(
    program_specs: &[arete_hash::ProgramSpecV1],
    program_id: &str,
) -> ProgramReadLayer {
    let Some(spec) = program_specs
        .iter()
        .find(|spec| spec.program_id == program_id)
    else {
        return Err("no program specification was recorded for this program".to_string());
    };
    let spec_hash = spec
        .hash()
        .map_err(|error| format!("failed to compute the program spec hash ({error})"))?;
    let release_hash = spec
        .oss_release_hash()
        .map_err(|error| format!("failed to compute the release hash ({error})"))?;
    Ok((spec_hash.to_string(), release_hash.to_string(), None))
}

/// Generate `programs.rs`: one module per program with typed instruction
/// builders, raw handlers, PDA helpers, and (when the stack records a program
/// spec for the program) release identity consts plus typed account readers.
/// Returns `None` when the stack declares no instructions.
#[allow(clippy::too_many_arguments)]
fn generate_stack_programs_rs(
    stack_name: &str,
    instructions: &[InstructionDef],
    idls: &[IdlSnapshot],
    pdas: &BTreeMap<String, BTreeMap<String, PdaDefinition>>,
    program_ids: &[String],
    program_specs: &[arete_hash::ProgramSpecV1],
    account_structs: &AccountModels,
    module_mode: bool,
    reads: &[RustProgramReadConfig],
    include_idl_only_programs: bool,
    program_extensions: &[RustProgramExtensionConfig],
) -> Option<ProgramsCodegen> {
    if instructions.is_empty() && !include_idl_only_programs {
        return None;
    }

    // Path to the generated types module from inside a `pub mod <program>`
    // block within programs.rs, and from a `generated` module inside it.
    let (types_path, nested_types_path) = if module_mode {
        ("super::super::types", "super::super::super::types")
    } else {
        ("crate::types", "crate::types")
    };

    let default_program_id = program_ids.first().cloned().unwrap_or_default();

    // Group instructions by resolved program id, preserving first-seen order.
    let mut groups: Vec<(String, Vec<&InstructionDef>)> = Vec::new();
    if include_idl_only_programs {
        for (index, idl) in idls.iter().enumerate() {
            let program_id = idl
                .program_id
                .clone()
                .or_else(|| program_ids.get(index).cloned())
                .unwrap_or_default();
            if !groups.iter().any(|(existing, _)| *existing == program_id) {
                groups.push((program_id, Vec::new()));
            }
        }
    }
    for instr in instructions {
        let pid = instr
            .program_id
            .clone()
            .unwrap_or_else(|| default_program_id.clone());
        match groups.iter_mut().find(|(existing, _)| *existing == pid) {
            Some((_, list)) => list.push(instr),
            None => groups.push((pid, vec![instr])),
        }
    }

    let mut parser = RustDefinedTypes::new(idls);
    let mut used_module_names: HashSet<String> = HashSet::new();
    let mut module_blocks: Vec<String> = Vec::new();
    let mut modules: Vec<ProgramModule> = Vec::new();

    for (index, (program_id, group)) in groups.iter().enumerate() {
        let idl_index = idls
            .iter()
            .position(|idl| idl.program_id.as_deref() == Some(program_id.as_str()));
        let idl = idl_index.map(|idl_index| &idls[idl_index]);
        parser.set_program(idl_index);
        let raw_name = match idl {
            Some(idl) => idl.name.clone(),
            None if index == 0 => stack_name.to_string(),
            None => format!("program{}", index),
        };
        let mut module_name = rust_module_name(&raw_name);
        if module_name.is_empty() {
            module_name = format!("program{}", index);
        }
        while !used_module_names.insert(module_name.clone()) {
            module_name.push('_');
        }
        let struct_name = format!("{}Program", to_pascal_case(&module_name));

        // PDA registry lookup: this program's group first, then any group.
        let own_pdas = idl.and_then(|idl| pdas.get(idl.name.as_str()));
        let mut pda_lookup: BTreeMap<&str, &PdaDefinition> = BTreeMap::new();
        if let Some(own) = own_pdas {
            for (name, def) in own {
                pda_lookup.insert(name.as_str(), def);
            }
        }
        for group_pdas in pdas.values() {
            for (name, def) in group_pdas {
                pda_lookup.entry(name.as_str()).or_insert(def);
            }
        }

        let program_errors = idl
            .map(|idl| dedupe_errors_by_code(&idl.errors))
            .unwrap_or_default();

        let mut needs = ProgramImports::default();
        let mut blocks: Vec<String> = Vec::new();
        let mut methods: Vec<String> = Vec::new();
        let mut skipped: Vec<(String, String)> = Vec::new();
        let mut params_names = RustParamsNames::new(idl);
        for instr in group {
            let errors = if instr.errors.is_empty() {
                program_errors.clone()
            } else {
                dedupe_errors_by_code(&instr.errors)
            };
            match generate_rust_instruction_block(
                instr,
                find_instruction_snapshot(instr, idl),
                &errors,
                &pda_lookup,
                &mut parser,
                &params_names,
                &mut needs,
            ) {
                Ok(block) => {
                    params_names.claim(block.params_name);
                    blocks.push(block.code);
                    methods.push(block.method);
                }
                Err(reason) => skipped.push((instr.name.clone(), reason)),
            }
        }

        // --- Program read layer: release identity + typed account readers. ---
        let read_layer = match reads.iter().find(|r| r.program_id == *program_id) {
            Some(r) => Ok((
                r.program_spec_hash.clone(),
                r.program_release_hash.clone(),
                r.descriptor.clone(),
            )),
            None => resolve_program_read_layer(program_specs, program_id),
        };
        let mut reader_methods: Vec<String> = Vec::new();
        let mut reader_notes: Vec<String> = Vec::new();
        if read_layer.is_ok() {
            let mut used_method_names: HashSet<String> = group
                .iter()
                .map(|instr| to_snake_case(&instr.name))
                .collect();
            used_method_names.insert("from_builder".to_string());
            // Every IDL account has a bound model (`types.rs`), or reads as
            // raw JSON when its layout is an enum.
            let accounts = idl
                .map(|idl| account_structs.program_accounts(&idl.name))
                .unwrap_or_default();
            for account in accounts {
                let method_name = format!("{}_accounts", to_snake_stem(&account.account));
                if !used_method_names.insert(method_name.clone()) {
                    reader_notes.push(format!(
                        "account reader for `{}` skipped: method name `{}` collides with an instruction builder",
                        account.account, method_name
                    ));
                    continue;
                }
                let (value_type, doc) = match &account.model {
                    Some(model) => (
                        format!("{types_path}::{model}"),
                        format!(
                            "Typed reader for `{}` accounts (release-addressed HTTP reads).",
                            account.account
                        ),
                    ),
                    None => (
                        "serde_json::Value".to_string(),
                        format!(
                            "Reader for `{}` accounts (release-addressed HTTP reads); its enum layout decodes as raw JSON.",
                            account.account
                        ),
                    ),
                };
                reader_methods.push(format!(
                    "        /// {doc}\n        pub fn {method_name}(&self) -> Result<arete_sdk::AccountReader<{value_type}>, arete_sdk::AreteError> {{\n            Ok(arete_sdk::AccountReader::new(\n                {account_literal},\n                std::sync::Arc::new(self.builder.account_transport({program_literal}, &read_descriptor())?),\n            ))\n        }}",
                    account_literal = rust_string_literal(&account.account),
                    program_literal = rust_string_literal(&raw_name),
                ));
            }
        }

        let mut sections: Vec<String> = Vec::new();
        if !blocks.is_empty() {
            let mut imports = vec!["BuiltInstruction", "InstructionError", "InstructionHandler"];
            if needs.account_meta {
                imports.extend(["AccountMeta", "AccountResolution"]);
            }
            if needs.arg_schema {
                imports.extend(["ArgSchema", "ArgType"]);
            }
            if needs.arg_field {
                imports.push("ArgField");
            }
            if needs.enum_variant {
                imports.extend(["EnumVariantDef", "EnumVariantKind"]);
            }
            if needs.error_metadata {
                imports.push("ErrorMetadata");
            }
            if needs.pda {
                imports.extend(["PdaConfig", "PdaSeed"]);
            }
            imports.sort_unstable();
            sections.push(format!(
                "    use arete_sdk::instruction::{{{}}};\n    use serde::Serialize;",
                imports.join(", ")
            ));
        }
        sections.push(format!(
            "    pub const PROGRAM_ID: &str = {};",
            rust_string_literal(program_id)
        ));
        if let Ok((spec_hash, release_hash, descriptor)) = &read_layer {
            let descriptor_body = match descriptor {
                Some(descriptor) => {
                    let json = serde_json::to_string(descriptor)
                        .expect("program read descriptor must serialize");
                    format!(
                        "        serde_json::from_str({}).expect(\"generated hosted program read descriptor must be valid\")",
                        rust_string_literal(&json),
                    )
                }
                None => "        arete_sdk::ProgramReadDescriptor::LocalHttp {\n            release: arete_sdk::ProgramReleaseReference {\n                program_release_hash: PROGRAM_RELEASE_HASH.to_string(),\n                program_spec_hash: PROGRAM_SPEC_HASH.to_string(),\n            },\n        }".to_string(),
            };
            sections.push(format!(
                "    /// Content hash of the exact program specification captured at generation time.\n    pub const PROGRAM_SPEC_HASH: &str = {spec};\n\n    /// Release identity addressing hosted account reads for this program.\n    pub const PROGRAM_RELEASE_HASH: &str = {release};\n\n    /// Exact release-addressed read descriptor for this program.\n    pub fn read_descriptor() -> arete_sdk::ProgramReadDescriptor {{\n{descriptor_body}\n    }}",
                spec = rust_string_literal(spec_hash),
                release = rust_string_literal(release_hash),
            ));
            if let Some(package_release_hash) = reads
                .iter()
                .find(|r| r.program_id == *program_id)
                .and_then(|r| r.package_release_hash.as_deref())
            {
                sections.push(format!(
                    "    /// Program package release this program SDK was generated from.\n    pub const PACKAGE_RELEASE_HASH: &str = {};",
                    rust_string_literal(package_release_hash)
                ));
            }
        }
        sections.extend(blocks);
        // `pdas` carries every PDA the program's ProgramSpec declares, as the
        // standalone program SDK does, plus the stack's own (which win on a
        // name both declare).
        let mut module_pdas = own_pdas.cloned().unwrap_or_default();
        if let Some(spec) = program_specs
            .iter()
            .find(|spec| spec.program_id == *program_id)
        {
            for (name, pda) in crate::program_sdk::program_spec_pdas(spec) {
                module_pdas.entry(name).or_insert(pda);
            }
        }
        if let Some(pdas_module) = generate_rust_pdas_module(&module_pdas) {
            sections.push(pdas_module);
        }

        // Program accessor: carries the client's program runtime so account
        // readers can build release-addressed transports and program
        // extensions get a `ProgramContext`. Instruction builders stay pure
        // and are also available as free functions.
        let mut impl_methods: Vec<String> = vec![
            "        /// Construct from the connected client's program runtime.\n        pub fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {\n            Self { builder }\n        }"
                .to_string(),
        ];
        // An instruction named `context` keeps its builder method; the
        // context stays available as `arete_sdk::ProgramContext::new(&program)`.
        let context_taken = group
            .iter()
            .any(|instr| to_snake_case(&instr.name) == "context");
        if context_taken {
            reader_notes.push(
                "`context()` is not generated: an instruction builder uses the name; use `arete_sdk::ProgramContext::new(&program)`".to_string(),
            );
        } else {
            impl_methods.push(format!(
                "        /// The context program extension functions take: the client's chain\n        /// reader, its wallet, and this accessor.\n        pub fn context(&self) -> arete_sdk::ProgramContext<'_, {struct_name}> {{\n            arete_sdk::ProgramContext::new(self)\n        }}"
            ));
        }
        impl_methods.extend(methods);
        impl_methods.extend(reader_methods);
        let program_struct = format!(
            "    /// Program accessor exposed on the stack client's `programs` namespace.\n    #[derive(Clone)]\n    pub struct {struct_name} {{\n        builder: arete_sdk::ProgramBuilder,\n    }}\n\n    impl {struct_name} {{\n{impl_methods}\n    }}\n\n    impl arete_sdk::ProgramAccessor for {struct_name} {{\n        fn program_builder(&self) -> &arete_sdk::ProgramBuilder {{\n            &self.builder\n        }}\n    }}",
            struct_name = struct_name,
            impl_methods = impl_methods.join("\n\n")
        );
        sections.push(program_struct);

        // The program package's own extension, embedded: files staged under
        // `programs/<module>/`, bound here exactly as at a standalone program
        // crate's root.
        if let Some(extension) = program_extensions
            .iter()
            .find(|extension| extension.program_id == *program_id)
        {
            let mut wiring = format!(
                "    // Hand-authored program package extension (staged from programs/{module_name}/extensions.json; not generated).\n"
            );
            let mut generated = vec!["super::*".to_string(), format!("{nested_types_path}::*")];
            generated.extend(account_model_reexports(
                account_structs,
                idl.map(|idl| idl.name.as_str()),
                nested_types_path,
            ));
            wiring.push_str(&render_generated_reexport_module(&generated, "    "));
            for stem in &extension.modules {
                wiring.push_str(&format!("    pub mod {stem};\n"));
            }
            wiring.push_str(&format!("    pub use {}::*;", extension.entry));
            sections.push(wiring);
        }

        let mut doc = format!(
            "/// Program SDK for `{}` (program ID `{}`).\n",
            raw_name, program_id
        );
        if let Err(reason) = &read_layer {
            doc.push_str(&format!(
                "///\n/// Program read layer omitted: {}.\n",
                reason
            ));
        }
        if !reader_notes.is_empty() {
            doc.push_str("///\n");
            for note in &reader_notes {
                doc.push_str(&format!("/// {}\n", note));
            }
        }
        if !skipped.is_empty() {
            doc.push_str("///\n/// Skipped instructions (unsupported by instruction codegen):\n");
            for (name, reason) in &skipped {
                doc.push_str(&format!("/// - `{}`: {}\n", name, reason));
            }
        }
        module_blocks.push(format!(
            "{doc}pub mod {module_name} {{\n{body}\n}}",
            doc = doc,
            module_name = module_name,
            body = sections.join("\n\n")
        ));
        modules.push(ProgramModule {
            program_id: program_id.clone(),
            module_name,
            struct_name,
        });
    }

    let code = format!(
        "//! Generated program SDK: typed instruction builders grouped per program.\n//!\n//! Instruction building is pure (no network access). Each program module\n//! exposes `PROGRAM_ID`, typed `*Params` structs, `fn <instruction>(params)`\n//! builders returning `BuiltInstruction`, raw `*_handler()` accessors, and a\n//! `pdas` module with PDA derivation helpers. Programs with a recorded\n//! program spec additionally expose `PROGRAM_SPEC_HASH` /\n//! `PROGRAM_RELEASE_HASH`, a `read_descriptor()` for release-addressed HTTP\n//! reads, and typed `*_accounts()` readers on the program accessor. Standalone\n//! output also exports a `ProgramSdk` aggregate for direct/session composition.\n\n{}\n",
        module_blocks.join("\n\n")
    );

    Some(ProgramsCodegen { code, modules })
}

fn to_kebab_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('-');
            }
            result.push(c.to_lowercase().next().unwrap());
        } else {
            result.push(c);
        }
    }
    result
}

fn to_pascal_case(s: &str) -> String {
    s.split(['_', '-', '.', ':'])
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

fn to_snake_case(s: &str) -> String {
    let mut result = to_snake_stem(s);
    if is_rust_keyword(&result) {
        result.push('_');
    }
    result
}

/// `to_snake_case` without the keyword escape: the stem a suffixed name is
/// built from. `use` is the instruction `use_`, but its handler is
/// `use_handler`, not `use__handler` (which `non_snake_case` rejects).
fn to_snake_stem(s: &str) -> String {
    let mut result = String::new();
    let mut separator = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            if separator && !result.is_empty() {
                result.push('_');
            }
            separator = false;
            if ch.is_ascii_uppercase() {
                if !result.is_empty() && !result.ends_with('_') {
                    result.push('_');
                }
                result.push(ch.to_ascii_lowercase());
            } else {
                result.push(ch.to_ascii_lowercase());
            }
        } else {
            separator = true;
        }
    }
    if result
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        result.insert_str(0, "value_");
    }
    result
}

/// Derive a valid Rust module name from an arbitrary alias or file stem
/// (lowercased, non-alphanumerics collapsed to `_`, keywords and leading
/// digits escaped). Shared with the CLI so staged devex extension files wire
/// up under the same stems the composition generator would use.
pub fn rust_module_name(value: &str) -> String {
    let mut output = String::new();
    let mut separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !output.is_empty() {
                output.push('_');
            }
            separator = false;
            output.push(character.to_ascii_lowercase());
        } else {
            separator = true;
        }
    }
    if output
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        output.insert_str(0, "live_");
    }
    if is_rust_keyword(&output) {
        output.push_str("_live");
    }
    output
}

fn is_rust_keyword(value: &str) -> bool {
    rust_ident::is_keyword(value)
}
