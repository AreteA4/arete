use crate::ast::*;
use crate::typescript_instructions::{
    dedupe_errors_by_code, normalize_seed_arg_type, split_generic,
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
            sdk_version: "0.4".to_string(),
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
/// `signature`) alongside the decoded `data`.
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
}

impl<T: Default> Default for EventWrapper<T> {
    fn default() -> Self {
        Self {
            timestamp: 0,
            data: T::default(),
            slot: None,
            signature: None,
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
        output.push_str(&self.generate_resolved_types(&resolved_name_map, &mut generated, None));

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
        mut account_structs: Option<&mut BTreeMap<String, String>>,
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
                        if resolved.is_account && !resolved.is_enum {
                            if let Some(map) = account_structs.as_deref_mut() {
                                map.entry(resolved.type_name.clone())
                                    .or_insert_with(|| emitted_name.clone());
                            }
                        }
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
        if resolved.is_enum {
            let variants: Vec<String> = resolved
                .enum_variants
                .iter()
                .map(|v| format!("    {},", to_pascal_case(v)))
                .collect();

            format!(
                "#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]\npub enum {} {{\n{}\n}}",
                emitted_name,
                variants.join("\n")
            )
        } else {
            let fields: Vec<String> = resolved
                .fields
                .iter()
                .map(|f| {
                    let rust_type = self.resolved_field_to_rust(f);
                    let serde_attr = self.serde_attr_for_resolved_field(f);
                    format!(
                        "    {}\n    pub {}: {},",
                        serde_attr,
                        to_snake_case(&f.field_name),
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

    fn scalar_shape_for_resolved_field(&self, field: &ResolvedField) -> RustScalarShape {
        rust_scalar_field_shape(
            &field.base_type,
            field.effective_integer_kind(),
            field.is_array,
            Some(field.field_type.as_str()),
            &field.field_type,
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

    /// Same as `serde_attr_for_field` but for resolved struct fields.
    fn serde_attr_for_resolved_field(&self, field: &ResolvedField) -> String {
        let shape = self.scalar_shape_for_resolved_field(field);
        match deserialize_with_for_shape(&shape, field.is_optional) {
            Some(deser_fn) => format!("#[serde(default, deserialize_with = \"{}\")]", deser_fn),
            None => "#[serde(default)]".to_string(),
        }
    }

    fn resolved_field_to_rust(&self, field: &ResolvedField) -> String {
        let typed = self.scalar_shape_for_resolved_field(field).rust_type;

        if field.is_optional {
            format!("Option<Option<{}>>", typed)
        } else {
            format!("Option<{}>", typed)
        }
    }

    fn build_resolved_type_name_map(&self) -> HashMap<String, String> {
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
/// `u64`/`i64`/`u32`/`i32` have `serde_utils` deserializers, so unsigned small
/// ints widen to `u64` and everything else widens to `i64`.
fn normalized_integer_kind_of(kind: IntegerKind) -> &'static str {
    match kind {
        IntegerKind::U64 => "u64",
        IntegerKind::U32 => "u32",
        IntegerKind::I32 => "i32",
        IntegerKind::U8 | IntegerKind::U16 | IntegerKind::Usize => "u64",
        // Signed small ints (i16/i8/isize) and the 128-bit kinds widen to i64.
        _ => "i64",
    }
}

fn normalized_integer_kind(rust_type_name: &str) -> &'static str {
    if rust_type_name.contains("u64") {
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

        assert!(manifest.contains("arete-sdk = { package = \"arete-a4-sdk\", version = \"0.4\" }"));
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
        let output = compile_program_modules(
            programs_stack_spec(),
            Some(RustStackConfig {
                crate_name: "demo-program".to_string(),
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
                    fields: vec![],
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
        for reserved in ["entity", "types", "programs"] {
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

    /// Regeneration helper for the checked-in ore example. Run with:
    /// `cargo test -p arete-interpreter regenerate_ore_example -- --ignored`
    ///
    /// Rewrites `examples/ore-rust/src/generated/ore/{mod,types,entity,programs}.rs`
    /// from `stacks/ore/.arete/OreStream.stack.json`.
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
        let spec_json =
            std::fs::read_to_string(repo_root.join("stacks/ore/.arete/OreStream.stack.json"))
                .expect("ore stack spec should exist");
        let spec = crate::versioned::load_stack_spec(&spec_json)
            .expect("ore stack spec should deserialize");

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
            sdk_version: "0.4".to_string(),
            module_mode: true,
            url: Some("wss://ore.stack.arete.run".to_string()),
            http_url: Some("https://ore.stack.arete.run".to_string()),
            extension_modules,
            extension_entry,
            program_reads: Vec::new(),
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
    /// `mod.rs`/`lib.rs` re-exports the entry at the stack module root
    /// (`pub use <entry>::*;`) so extension traits come into scope with the
    /// stack's own glob import.
    pub extension_entry: Option<String>,
    /// Published-platform program read overrides keyed by program ID. When an
    /// entry matches a program, its exact `program_spec_hash` /
    /// `program_release_hash` (from a hosted platform release) are used for
    /// the program read layer instead of the OSS-derived release identity.
    /// Absent programs keep the default OSS/local-HTTP read layer. Published
    /// standalone programs may additionally carry their exact hosted-binding
    /// descriptor so they do not inherit a stack HTTP endpoint.
    pub program_reads: Vec<RustProgramReadConfig>,
}

#[derive(Debug, Clone)]
pub struct RustProgramReadConfig {
    pub program_id: String,
    pub program_spec_hash: String,
    pub program_release_hash: String,
    /// Exact wire descriptor for a published hosted binding. `None` keeps
    /// the local-HTTP descriptor used by locally generated stack SDKs.
    pub descriptor: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default)]
pub struct RustCompositionConfig {
    pub stack: RustStackConfig,
    pub live_urls: BTreeMap<String, String>,
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
            sdk_version: "0.4".to_string(),
            module_mode: false,
            url: None,
            http_url: None,
            extension_modules: Vec::new(),
            extension_entry: None,
            program_reads: Vec::new(),
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

    let mut entity_names: Vec<String> = Vec::new();
    let mut entity_specs: Vec<SerializableStreamSpec> = Vec::new();

    for mut spec in stack_spec.entities {
        if spec.idl.is_none() {
            spec.idl = stack_spec.idls.first().cloned();
        }
        entity_names.push(spec.state_name.clone());
        entity_specs.push(spec);
    }

    let view_entity_names = entity_specs
        .iter()
        .zip(&entity_names)
        .filter(|(spec, _)| !exact_views || !spec.views.is_empty())
        .map(|(_, name)| name.clone())
        .collect::<Vec<_>>();

    let (types_rs, account_structs) = generate_stack_types_rs(&entity_specs, &entity_names);

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
    );
    let entity_rs = generate_stack_entity_rs(
        stack_name,
        &stack_kebab,
        &entity_specs,
        &entity_names,
        &config,
        exact_views,
        programs.as_ref(),
    );
    validate_extension_modules(&config, programs.is_some())?;
    let lib_rs = generate_stack_lib_rs(
        stack_name,
        &view_entity_names,
        config.module_mode,
        programs.is_some(),
        &config.extension_modules,
        config.extension_entry.as_deref(),
    );
    let cargo_toml = generate_stack_cargo_toml(&config);

    Ok(RustOutput {
        cargo_toml,
        lib_rs,
        types_rs,
        entity_rs,
        programs_rs: programs.map(|codegen| codegen.code),
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
    let (types_rs, account_structs) = generate_stack_types_rs(&stack_spec.entities, &entity_names);
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
    )
    .ok_or_else(|| {
        format!(
            "Stack '{}' contains no programs to emit",
            stack_spec.stack_name
        )
    })?;

    validate_extension_modules(&config, true)?;
    let aggregate_name = format!("{}Programs", to_pascal_case(&stack_spec.stack_name));
    programs.code.push_str("\n");
    programs.code.push_str(&generate_programs_accessor_struct(
        &aggregate_name,
        &programs,
        "self",
    ));
    programs.code.push_str(&format!(
        "\n\nimpl arete_sdk::ProgramSdk for {aggregate_name} {{\n    fn name() -> &'static str {{\n        {}\n    }}\n}}\n",
        rust_string_literal(&to_kebab_case(&stack_spec.stack_name)),
    ));

    let mut lib_rs = format!(
        "mod types;\npub mod programs;\n\npub use programs::{aggregate_name};\npub use types::*;\n\npub use arete_sdk::{{ProgramSdk, Programs}};\n"
    );
    append_rust_extension_exports(
        &mut lib_rs,
        &config.extension_modules,
        config.extension_entry.as_deref(),
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
    compile_stack_spec_with_view_selection(stack_spec, config, true)
}

/// Generate one namespaced Rust stack module per live alias plus a manifest
/// module that preserves alias boundaries instead of flattening views/adapters.
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
    if !config.stack.extension_modules.is_empty() || config.stack.extension_entry.is_some() {
        return Err(
            "Rust composition SDKs do not support stack extensions; extensions attach to a single-live stack module".to_string(),
        );
    }
    let mut live_stacks = Vec::with_capacity(composed.live_specs.len());
    for live in composed.live_specs {
        let module_name = rust_module_name(&live.alias);
        let mut live_config = config.stack.clone();
        live_config.module_mode = true;
        live_config.url = config.live_urls.get(&live.alias).cloned();
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

/// Validate hand-authored extension module stems against the generated
/// module names. Entry-stem collisions are a hard error because the staged
/// file would shadow (or be shadowed by) a generated file.
fn validate_extension_modules(config: &RustStackConfig, has_programs: bool) -> Result<(), String> {
    if config.extension_modules.is_empty() && config.extension_entry.is_none() {
        return Ok(());
    }
    if config.extension_entry.is_none() {
        return Err("extension modules were configured without an extension entry".to_string());
    }
    let mut seen = HashSet::new();
    for stem in &config.extension_modules {
        let reserved = matches!(stem.as_str(), "entity" | "types" | "mod" | "lib")
            || (stem == "programs" && has_programs);
        if reserved {
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
    match &config.extension_entry {
        Some(entry) if config.extension_modules.last() == Some(entry) => Ok(()),
        Some(entry) => Err(format!(
            "extension entry module '{entry}' must be the last configured extension module"
        )),
        None => unreachable!("checked above"),
    }
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

    append_rust_extension_exports(&mut output, extension_modules, extension_entry);

    output
}

fn append_rust_extension_exports(
    output: &mut String,
    extension_modules: &[String],
    extension_entry: Option<&str>,
) {
    if let Some(entry) = extension_entry {
        output.push_str(
            "\n// Hand-authored devex extensions (staged from extensions.json; not generated).\n",
        );
        for stem in extension_modules {
            output.push_str(&format!("pub mod {stem};\n"));
        }
        output.push_str(&format!("pub use {entry}::*;\n"));
    }
}

/// Generate types.rs containing structs for ALL entities in the stack.
///
/// Also returns the map of emitted raw account structs (IDL account type name
/// -> emitted Rust struct name) so the program SDK generator can attach typed
/// account readers for accounts that actually have a generated struct.
fn generate_stack_types_rs(
    entity_specs: &[SerializableStreamSpec],
    entity_names: &[String],
) -> (String, BTreeMap<String, String>) {
    let mut output = String::new();
    output.push_str("use serde::{Deserialize, Serialize};\n");
    output.push_str("use arete_sdk::serde_utils;\n\n");

    let mut generated = HashSet::new();
    let mut account_structs: BTreeMap<String, String> = BTreeMap::new();
    let mut used_builtins: BTreeSet<&'static str> = BTreeSet::new();

    for (i, spec) in entity_specs.iter().enumerate() {
        let entity_name = &entity_names[i];
        let compiler = RustCompiler::new(spec.clone(), entity_name.clone(), RustConfig::default());
        let resolved_name_map = compiler.build_resolved_type_name_map();
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

        let resolved = compiler.generate_resolved_types(
            &resolved_name_map,
            &mut generated,
            Some(&mut account_structs),
        );
        output.push_str(&resolved);
        while !output.ends_with("\n\n") {
            output.push('\n');
        }
    }

    // Generate the builtin resolver output structs (SlotHashBytes /
    // TokenMetadata) once, for the whole stack.
    output.push_str(&render_builtin_resolver_structs(&used_builtins));

    // Generate the runtime envelopes (EventWrapper / CaptureWrapper) once.
    output.push('\n');
    output.push_str(WRAPPER_TYPES);

    (output, account_structs)
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
        "{stack_kebab}"
    }}

    {url_impl}{http_url_impl}
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
        stack_kebab = stack_kebab,
        programs_assoc = programs_assoc,
        url_impl = url_impl,
        http_url_impl = http_url_impl,
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
    module_name: String,
    struct_name: String,
}

/// Result of generating the `programs` module for a stack.
#[derive(Debug, Clone)]
pub(crate) struct ProgramsCodegen {
    code: String,
    modules: Vec<ProgramModule>,
}

/// Which `arete_sdk::instruction` items a generated program module references.
#[derive(Debug, Default)]
struct ProgramImports {
    account_meta: bool,
    arg_schema: bool,
    pda: bool,
    error_metadata: bool,
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

/// Render a Rust string literal (quoted and escaped).
fn rust_string_literal(value: &str) -> String {
    format!("{:?}", value)
}

/// Resolver for IDL-defined types (structs/enums) referenced by instruction
/// args. Resolved types are inlined into arg schemas as `ArgType::Struct` /
/// `ArgType::Enum` expressions; the typed params field for such args is
/// `serde_json::Value`. Mirrors the TypeScript `DefinedTypes` parsing rules.
struct RustDefinedTypes<'a> {
    /// IDL type definitions by name, first-wins across programs.
    defs: BTreeMap<String, &'a IdlTypeDefSnapshot>,
    /// lowercase name -> canonical key, for case-insensitive fallback lookup.
    lower: BTreeMap<String, String>,
    /// Memoized resolutions by original IDL name (`None` = unsupported).
    resolved: BTreeMap<String, Option<RustParsedArg>>,
    /// Names currently being resolved (cycle guard).
    visiting: HashSet<String>,
}

impl<'a> RustDefinedTypes<'a> {
    fn new(idls: &'a [IdlSnapshot]) -> Self {
        let mut defs: BTreeMap<String, &'a IdlTypeDefSnapshot> = BTreeMap::new();
        let mut lower: BTreeMap<String, String> = BTreeMap::new();
        for idl in idls {
            for def in &idl.types {
                if !defs.contains_key(def.name.as_str()) {
                    defs.insert(def.name.clone(), def);
                    lower.insert(def.name.to_lowercase(), def.name.clone());
                }
            }
        }
        RustDefinedTypes {
            defs,
            lower,
            resolved: BTreeMap::new(),
            visiting: HashSet::new(),
        }
    }

    /// Parse a stringified Rust-ish arg type (what `to_rust_type_string`
    /// produces), resolving bare names against the IDL type definitions.
    fn parse_arg_type(&mut self, raw: &str) -> RustParsedArg {
        let t = raw.trim().trim_start_matches('&').trim();

        // Generic wrappers: Option<T>, Vec<T>.
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
                "Vec" => {
                    let inner = self.parse_arg_type(inner);
                    return RustParsedArg {
                        schema: format!("ArgType::Vec(Box::new({}))", inner.schema),
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
            _ => self.resolve_defined(last).unwrap_or_else(rust_unsupported),
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
                RustParsedArg {
                    schema: format!("ArgType::Vec(Box::new({}))", inner.schema),
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
        if let Some(cached) = self.resolved.get(name) {
            return cached.clone();
        }
        if self.visiting.contains(name) {
            // Recursive types are not supported by instruction codegen.
            return None;
        }

        let key = if self.defs.contains_key(name) {
            name.to_string()
        } else {
            match self.lower.get(&name.to_lowercase()) {
                Some(canonical) => canonical.clone(),
                None => {
                    self.resolved.insert(name.to_string(), None);
                    return None;
                }
            }
        };

        self.visiting.insert(key.clone());
        let def = self.defs[&key];
        let result = match &def.type_def {
            IdlTypeDefKindSnapshot::Struct { fields, .. } => {
                let fields = fields.clone();
                self.resolve_struct(&fields)
            }
            IdlTypeDefKindSnapshot::TupleStruct { .. } => None,
            IdlTypeDefKindSnapshot::Enum { variants, .. } => {
                let variants = variants.clone();
                self.resolve_enum(&variants)
            }
        };
        self.visiting.remove(&key);
        self.resolved.insert(name.to_string(), result.clone());
        if name != key {
            self.resolved.insert(key, result.clone());
        }
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

/// Whether any emitted schema expression references `ArgField` /
/// `EnumVariantDef` (defined struct/enum types were inlined).
fn schema_uses_defined_types(schema: &str) -> bool {
    schema.contains("ArgField") || schema.contains("EnumVariantDef")
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
    resolution: &str,
    comment: Option<&str>,
) -> String {
    let mut out = String::new();
    if let Some(comment) = comment {
        out.push_str(&format!("                // [arete codegen] {}\n", comment));
    }
    out.push_str(&format!(
        "                AccountMeta {{\n                    name: {name}.to_string(),\n                    is_signer: {is_signer},\n                    is_writable: {is_writable},\n                    resolution: {resolution},\n                    is_optional: {is_optional},\n                }},",
        name = rust_string_literal(&acc.name),
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
) -> MappedRustAccount {
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
            literal: rust_account_meta_literal(acc, "AccountResolution::UserProvided", Some(&note)),
            field: Some((acc.name.clone(), user_field_kind)),
            notes: vec![note],
            uses_pda: false,
        }
    };

    match &acc.resolution {
        AccountResolution::Signer => MappedRustAccount {
            literal: rust_account_meta_literal(acc, "AccountResolution::Signer", None),
            field: Some((acc.name.clone(), RustAccountFieldKind::Signer)),
            notes: Vec::new(),
            uses_pda: false,
        },
        AccountResolution::Known { address } => MappedRustAccount {
            literal: rust_account_meta_literal(
                acc,
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
            literal: rust_account_meta_literal(acc, "AccountResolution::UserProvided", None),
            field: Some((acc.name.clone(), user_field_kind)),
            notes: Vec::new(),
            uses_pda: false,
        },
        AccountResolution::PdaInline { seeds, program_id } => {
            match build_rust_pda_config(seeds, program_id.as_deref(), account_names, arg_types) {
                Ok((resolution, notes)) => MappedRustAccount {
                    literal: rust_account_meta_literal(acc, &resolution, None),
                    field: None,
                    notes,
                    uses_pda: true,
                },
                Err(reason) => degraded(reason),
            }
        }
        AccountResolution::PdaRef { pda_name } => match pda_lookup.get(pda_name.as_str()) {
            Some(def) => {
                match build_rust_pda_config(
                    &def.seeds,
                    def.program_id.as_deref(),
                    account_names,
                    arg_types,
                ) {
                    Ok((resolution, notes)) => MappedRustAccount {
                        literal: rust_account_meta_literal(acc, &resolution, None),
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
                    rust_string_literal(account_name)
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
    uses_defined_types: bool,
}

fn generate_rust_instruction_block(
    instr: &InstructionDef,
    errors: &[IdlErrorSnapshot],
    pda_lookup: &BTreeMap<&str, &PdaDefinition>,
    parser: &mut RustDefinedTypes<'_>,
    needs: &mut ProgramImports,
) -> Result<RustInstructionBlock, String> {
    // --- Parse args; skip the whole instruction on unsupported types. ---
    let mut parsed_args: Vec<(&InstructionArgDef, RustParsedArg)> = Vec::new();
    for arg in &instr.args {
        let parsed = parser.parse_arg_type(&arg.arg_type);
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
    for acc in &instr.accounts {
        let mapped = map_rust_account(acc, pda_lookup, &account_names, &arg_types);
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
    let pascal = to_pascal_case(&instr.name);
    let params_name = format!("{}Params", pascal);

    // --- Typed params struct: args first, then caller-supplied accounts.
    // Instruction args win name collisions (mirrors the TS SDK's params
    // precedence in `splitParams`). ---
    let arg_name_set: HashSet<&str> = instr.args.iter().map(|a| a.name.as_str()).collect();
    let mut used_field_names: HashSet<String> = HashSet::new();
    let mut param_fields: Vec<String> = Vec::new();
    let mut uses_defined_types = false;
    for (arg, parsed) in &parsed_args {
        let field_name = to_snake_case(&arg.name);
        used_field_names.insert(field_name.clone());
        uses_defined_types |= schema_uses_defined_types(&parsed.schema);
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
        if arg_name_set.contains(name.as_str()) {
            notes.push(format!(
                "account `{}` shares its name with an instruction arg and has no typed override field",
                name
            ));
            continue;
        }
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
        "{docs}\n    pub fn {fn_name}(params: {params_name}) -> Result<BuiltInstruction, InstructionError> {{\n        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {{\n            context: \"params\".to_string(),\n            message: error.to_string(),\n        }})?;\n        {fn_name}_handler().build(params)\n    }}",
        docs = docs,
        fn_name = fn_name,
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
        "    /// Raw instruction handler for `{name}`.\n    pub fn {fn_name}_handler() -> InstructionHandler {{\n        InstructionHandler {{\n            program_id: PROGRAM_ID.to_string(),\n            discriminator: vec![{discriminator}],\n            accounts: {accounts},\n            args: {args},\n            errors: {errors},\n        }}\n    }}",
        name = instr.name,
        fn_name = fn_name,
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
        uses_defined_types,
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

        let program_expr = match &def.program_id {
            Some(pid) => rust_string_literal(pid),
            None => {
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
    account_structs: &BTreeMap<String, String>,
    module_mode: bool,
    reads: &[RustProgramReadConfig],
    include_idl_only_programs: bool,
) -> Option<ProgramsCodegen> {
    if instructions.is_empty() && !include_idl_only_programs {
        return None;
    }

    // Path to the generated types module from inside a `pub mod <program>`
    // block within programs.rs.
    let types_path = if module_mode {
        "super::super::types"
    } else {
        "crate::types"
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
        let idl = idls
            .iter()
            .find(|idl| idl.program_id.as_deref() == Some(program_id.as_str()));
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
        let mut uses_defined_types = false;
        for instr in group {
            let errors = if instr.errors.is_empty() {
                program_errors.clone()
            } else {
                dedupe_errors_by_code(&instr.errors)
            };
            match generate_rust_instruction_block(
                instr,
                &errors,
                &pda_lookup,
                &mut parser,
                &mut needs,
            ) {
                Ok(block) => {
                    blocks.push(block.code);
                    methods.push(block.method);
                    uses_defined_types |= block.uses_defined_types;
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
            let accounts = idl.map(|idl| idl.accounts.as_slice()).unwrap_or_default();
            for account in accounts {
                let Some(struct_name) = account_structs.get(&account.name).or_else(|| {
                    account_structs
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(&account.name))
                        .map(|(_, emitted)| emitted)
                }) else {
                    // No generated struct for this account type; no reader.
                    continue;
                };
                let method_name = format!("{}_accounts", to_snake_case(&account.name));
                if !used_method_names.insert(method_name.clone()) {
                    reader_notes.push(format!(
                        "account reader for `{}` skipped: method name `{}` collides with an instruction builder",
                        account.name, method_name
                    ));
                    continue;
                }
                reader_methods.push(format!(
                    "        /// Typed reader for `{account}` accounts (release-addressed HTTP reads).\n        pub fn {method_name}(&self) -> Result<arete_sdk::AccountReader<{types_path}::{struct_name}>, arete_sdk::AreteError> {{\n            Ok(arete_sdk::AccountReader::new(\n                {account_literal},\n                std::sync::Arc::new(self.builder.account_transport({program_literal}, &read_descriptor())?),\n            ))\n        }}",
                    account = account.name,
                    method_name = method_name,
                    types_path = types_path,
                    struct_name = struct_name,
                    account_literal = rust_string_literal(&account.name),
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
            if uses_defined_types {
                imports.extend(["ArgField", "EnumVariantDef", "EnumVariantKind"]);
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
        }
        sections.extend(blocks);
        if let Some(pdas_module) = own_pdas.and_then(generate_rust_pdas_module) {
            sections.push(pdas_module);
        }

        // Program accessor: carries the client's program runtime so account
        // readers can build release-addressed transports. Instruction
        // builders stay pure and are also available as free functions.
        let builder_field = if reader_methods.is_empty() {
            // No generated reader uses the runtime (yet); silence dead_code.
            "        #[allow(dead_code)]\n        builder: arete_sdk::ProgramBuilder,"
        } else {
            "        builder: arete_sdk::ProgramBuilder,"
        };
        let mut impl_methods: Vec<String> = vec![
            "        /// Construct from the connected client's program runtime.\n        pub fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {\n            Self { builder }\n        }"
                .to_string(),
        ];
        impl_methods.extend(methods);
        impl_methods.extend(reader_methods);
        let program_struct = format!(
            "    /// Program accessor exposed on the stack client's `programs` namespace.\n    #[derive(Clone)]\n    pub struct {struct_name} {{\n{builder_field}\n    }}\n\n    impl {struct_name} {{\n{impl_methods}\n    }}",
            struct_name = struct_name,
            builder_field = builder_field,
            impl_methods = impl_methods.join("\n\n")
        );
        sections.push(program_struct);

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
    s.split(['_', '-', '.'])
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
    if is_rust_keyword(&result) {
        result.push('_');
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
    matches!(
        value,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "union"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
    )
}
