use crate::ast::*;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct RustOutput {
    pub cargo_toml: String,
    pub lib_rs: String,
    pub types_rs: String,
    pub entity_rs: String,
}

impl RustOutput {
    pub fn full_lib(&self) -> String {
        format!(
            "{}\n\n// types.rs\n{}\n\n// entity.rs\n{}",
            self.lib_rs, self.types_rs, self.entity_rs
        )
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
            sdk_version: "0.2".to_string(),
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
    Ok(())
}

pub(crate) struct RustCompiler {
    spec: SerializableStreamSpec,
    entity_name: String,
    config: RustConfig,
}

impl RustCompiler {
    pub(crate) fn new(
        spec: SerializableStreamSpec,
        entity_name: String,
        config: RustConfig,
    ) -> Self {
        Self {
            spec,
            entity_name,
            config,
        }
    }

    fn compile(&self) -> RustOutput {
        RustOutput {
            cargo_toml: self.generate_cargo_toml(),
            lib_rs: self.generate_lib_rs(),
            types_rs: self.generate_types_rs(),
            entity_rs: self.generate_entity_rs(),
        }
    }

    fn generate_cargo_toml(&self) -> String {
        format!(
            r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
hyperstack-sdk = "{}"
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

pub use hyperstack_sdk::{{ConnectionState, HyperStack, Stack, Update, Views}};
"#,
            stack_name = stack_name,
            entity_name = entity_name
        )
    }

    fn generate_types_rs(&self) -> String {
        let mut output = String::new();
        output.push_str("use serde::{Deserialize, Serialize};\n");
        output.push_str("use hyperstack_sdk::serde_utils;\n\n");

        let mut generated = HashSet::new();

        for section in &self.spec.sections {
            if !Self::is_root_section(&section.name)
                && section.fields.iter().any(|field| field.emit)
                && generated.insert(section.name.clone())
            {
                output.push_str(&self.generate_struct_for_section(section));
                output.push_str("\n\n");
            }
        }

        output.push_str(&self.generate_main_entity_struct());
        output.push_str(&self.generate_resolved_types(&mut generated));
        output.push_str(&self.generate_event_wrapper());

        output
    }

    pub(crate) fn generate_struct_for_section(&self, section: &EntitySection) -> String {
        let struct_name = format!("{}{}", self.entity_name, to_pascal_case(&section.name));
        let mut fields = Vec::new();

        for field in &section.fields {
            if !field.emit {
                continue;
            }
            let field_name = to_snake_case(&field.field_name);
            let rust_type = self.field_type_to_rust(field);
            let serde_attr = self.serde_attr_for_field(field);

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

    pub(crate) fn generate_main_entity_struct(&self) -> String {
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
                    let rust_type = self.field_type_to_rust(field);
                    let serde_attr = self.serde_attr_for_field(field);
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

    pub(crate) fn generate_resolved_types(&self, generated: &mut HashSet<String>) -> String {
        let mut output = String::new();

        for section in &self.spec.sections {
            for field in &section.fields {
                if !field.emit {
                    continue;
                }
                if let Some(resolved) = &field.resolved_type {
                    if generated.insert(resolved.type_name.clone()) {
                        output.push_str("\n\n");
                        output.push_str(&self.generate_resolved_struct(resolved));
                    }
                }
            }
        }

        output
    }

    fn generate_resolved_struct(&self, resolved: &ResolvedStructType) -> String {
        if resolved.is_enum {
            let variants: Vec<String> = resolved
                .enum_variants
                .iter()
                .map(|v| format!("    {},", to_pascal_case(v)))
                .collect();

            format!(
                "#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]\npub enum {} {{\n{}\n}}",
                to_pascal_case(&resolved.type_name),
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
                to_pascal_case(&resolved.type_name),
                fields.join("\n")
            )
        }
    }

    fn generate_event_wrapper(&self) -> String {
        r#"

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventWrapper<T> {
    #[serde(default, deserialize_with = "serde_utils::deserialize_i64")]
    pub timestamp: i64,
    pub data: T,
    #[serde(default)]
    pub slot: Option<f64>,
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
"#
        .to_string()
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
        "" // TODO: Set URL after first deployment in hyperstack.toml
    }"#
            .to_string(),
        };

        let entity_views = self.generate_entity_views_struct();

        format!(
            r#"use {types_import}::{entity_name};
use hyperstack_sdk::{{Stack, StateView, ViewBuilder, ViewHandle, Views}};

pub struct {stack_name}Stack;

impl Stack for {stack_name}Stack {{
    type Views = {stack_name}StackViews;

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
    fn field_type_to_rust(&self, field: &FieldTypeInfo) -> String {
        let base = self.base_type_to_rust(&field.base_type, &field.rust_type_name);

        let typed = if field.is_array && !matches!(field.base_type, BaseType::Array) {
            format!("Vec<{}>", base)
        } else {
            base
        };

        // All fields wrapped in Option since we receive patches
        // Optional spec fields get Option<Option<T>> to distinguish "not received" from "explicitly null"
        if field.is_optional {
            format!("Option<Option<{}>>", typed)
        } else {
            format!("Option<{}>", typed)
        }
    }

    fn base_type_to_rust(&self, base_type: &BaseType, rust_type_name: &str) -> String {
        match base_type {
            BaseType::Integer => {
                if rust_type_name.contains("u64") {
                    "u64".to_string()
                } else if rust_type_name.contains("i64") {
                    "i64".to_string()
                } else if rust_type_name.contains("u32") {
                    "u32".to_string()
                } else if rust_type_name.contains("i32") {
                    "i32".to_string()
                } else {
                    "i64".to_string()
                }
            }
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

    /// Return the `#[serde(...)]` attribute for a field.
    /// Integer fields get a `deserialize_with` pointing to the appropriate
    /// `serde_utils` function so that string-encoded big integers are handled.
    fn serde_attr_for_field(&self, field: &FieldTypeInfo) -> String {
        if let Some(deser_fn) = self.deserialize_with_for_type(
            &field.base_type,
            field.is_optional,
            field.is_array && !matches!(field.base_type, BaseType::Array),
            &field.rust_type_name,
        ) {
            format!("#[serde(default, deserialize_with = \"{}\")]", deser_fn)
        } else {
            "#[serde(default)]".to_string()
        }
    }

    /// Same as `serde_attr_for_field` but for resolved struct fields.
    fn serde_attr_for_resolved_field(&self, field: &ResolvedField) -> String {
        if let Some(deser_fn) = self.deserialize_with_for_type(
            &field.base_type,
            field.is_optional,
            field.is_array,
            &field.field_type,
        ) {
            format!("#[serde(default, deserialize_with = \"{}\")]", deser_fn)
        } else {
            "#[serde(default)]".to_string()
        }
    }

    /// Determine the appropriate `serde_utils::deserialize_*` function for a
    /// given type combination, or `None` if no custom deserializer is needed.
    fn deserialize_with_for_type(
        &self,
        base_type: &BaseType,
        is_optional: bool,
        is_array: bool,
        rust_type_name: &str,
    ) -> Option<String> {
        // Only integer and timestamp types need the string-or-number treatment
        let int_kind = match base_type {
            BaseType::Integer => {
                if rust_type_name.contains("i64") {
                    "i64"
                } else if rust_type_name.contains("i32") {
                    "i32"
                } else if rust_type_name.contains("u32") {
                    "u32"
                } else {
                    "u64"
                }
            }
            BaseType::Timestamp => "i64",
            _ => return None,
        };

        let fn_name = match (is_optional, is_array) {
            (false, false) => format!("serde_utils::deserialize_option_{}", int_kind),
            (true, false) => format!("serde_utils::deserialize_option_option_{}", int_kind),
            (false, true) => format!("serde_utils::deserialize_option_vec_{}", int_kind),
            (true, true) => format!("serde_utils::deserialize_option_option_vec_{}", int_kind),
        };

        Some(fn_name)
    }

    fn resolved_field_to_rust(&self, field: &ResolvedField) -> String {
        let base = self.base_type_to_rust(&field.base_type, &field.field_type);

        let typed = if field.is_array {
            format!("Vec<{}>", base)
        } else {
            base
        };

        if field.is_optional {
            format!("Option<Option<{}>>", typed)
        } else {
            format!("Option<{}>", typed)
        }
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
}

impl Default for RustStackConfig {
    fn default() -> Self {
        Self {
            crate_name: "generated-stack".to_string(),
            sdk_version: "0.2".to_string(),
            module_mode: false,
            url: None,
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

    let types_rs = generate_stack_types_rs(&entity_specs, &entity_names);
    let entity_rs = generate_stack_entity_rs(
        stack_name,
        &stack_kebab,
        &entity_specs,
        &entity_names,
        &config,
    );
    let lib_rs = generate_stack_lib_rs(stack_name, &entity_names, config.module_mode);
    let cargo_toml = generate_stack_cargo_toml(&config);

    Ok(RustOutput {
        cargo_toml,
        lib_rs,
        types_rs,
        entity_rs,
    })
}

fn generate_stack_cargo_toml(config: &RustStackConfig) -> String {
    format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
hyperstack-sdk = "{}"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#,
        config.crate_name, config.sdk_version
    )
}

fn generate_stack_lib_rs(stack_name: &str, entity_names: &[String], _module_mode: bool) -> String {
    let entity_views_exports: Vec<String> = entity_names
        .iter()
        .map(|name| format!("{}EntityViews", name))
        .collect();

    let all_exports = format!(
        "{}Stack, {}StackViews, {}",
        stack_name,
        stack_name,
        entity_views_exports.join(", ")
    );

    format!(
        r#"mod entity;
mod types;

pub use entity::{{{all_exports}}};
pub use types::*;

pub use hyperstack_sdk::{{ConnectionState, HyperStack, Stack, Update, Views}};
"#,
        all_exports = all_exports
    )
}

/// Generate types.rs containing structs for ALL entities in the stack.
fn generate_stack_types_rs(
    entity_specs: &[SerializableStreamSpec],
    entity_names: &[String],
) -> String {
    let mut output = String::new();
    output.push_str("use serde::{Deserialize, Serialize};\n");
    output.push_str("use hyperstack_sdk::serde_utils;\n\n");

    let mut generated = HashSet::new();

    for (i, spec) in entity_specs.iter().enumerate() {
        let entity_name = &entity_names[i];
        let compiler = RustCompiler::new(spec.clone(), entity_name.clone(), RustConfig::default());

        // Generate section structs (e.g., OreRoundId, OreRoundState)
        for section in &spec.sections {
            if !RustCompiler::is_root_section(&section.name) {
                let struct_name = format!("{}{}", entity_name, to_pascal_case(&section.name));
                if generated.insert(struct_name) {
                    output.push_str(&compiler.generate_struct_for_section(section));
                    output.push_str("\n\n");
                }
            }
        }

        // Generate main entity struct (e.g., OreRound, OreTreasury)
        output.push_str(&compiler.generate_main_entity_struct());
        output.push_str("\n\n");

        let resolved = compiler.generate_resolved_types(&mut generated);
        output.push_str(&resolved);
        while !output.ends_with("\n\n") {
            output.push('\n');
        }
    }

    // Generate EventWrapper once
    output.push_str(
        r#"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventWrapper<T> {
    #[serde(default, deserialize_with = "serde_utils::deserialize_i64")]
    pub timestamp: i64,
    pub data: T,
    #[serde(default)]
    pub slot: Option<f64>,
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
"#,
    );

    output
}

/// Generate entity.rs with a single Stack impl and per-entity EntityViews.
fn generate_stack_entity_rs(
    stack_name: &str,
    stack_kebab: &str,
    entity_specs: &[SerializableStreamSpec],
    entity_names: &[String],
    config: &RustStackConfig,
) -> String {
    let types_import = if config.module_mode {
        "super::types"
    } else {
        "crate::types"
    };

    let entity_type_imports: Vec<String> =
        entity_names.iter().map(|name| name.to_string()).collect();

    let url_impl = match &config.url {
        Some(url) => format!(
            r#"fn url() -> &'static str {{
        "{}"
    }}"#,
            url
        ),
        None => r#"fn url() -> &'static str {
        "" // TODO: Set URL after first deployment in hyperstack.toml
    }"#
        .to_string(),
    };

    // StackViews struct fields
    let views_fields: Vec<String> = entity_names
        .iter()
        .map(|name| {
            let snake = to_snake_case(name);
            format!("    pub {}: {}EntityViews,", snake, name)
        })
        .collect();

    // Views::from_builder body — clone builder for all but last entity
    let views_builder_fields: Vec<String> = entity_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let snake = to_snake_case(name);
            if i < entity_names.len() - 1 {
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

        // state() method — always present
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

        // Always include list view (built-in view, like state)
        methods.push(format!(
            r#"
    pub fn list(&self) -> ViewHandle<{entity}> {{
        self.builder.view("{entity}/list")
    }}"#,
            entity = entity_name
        ));

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

    format!(
        r#"use {types_import}::{{{entity_imports}}};
use hyperstack_sdk::{{Stack, StateView, ViewBuilder, ViewHandle, Views}};

pub struct {stack}Stack;

impl Stack for {stack}Stack {{
    type Views = {stack}StackViews;

    fn name() -> &'static str {{
        "{stack_kebab}"
    }}

    {url_impl}
}}

pub struct {stack}StackViews {{
{views_fields}
}}

impl Views for {stack}StackViews {{
    fn from_builder(builder: ViewBuilder) -> Self {{
        Self {{
{views_builder}
        }}
    }}
}}
{entity_views}"#,
        types_import = types_import,
        entity_imports = entity_type_imports.join(", "),
        stack = stack_name,
        stack_kebab = stack_kebab,
        url_impl = url_impl,
        views_fields = views_fields.join("\n"),
        views_builder = views_builder_fields.join("\n"),
        entity_views = entity_views_structs.join("\n"),
    )
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
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_lowercase().next().unwrap());
        } else {
            result.push(ch);
        }
    }
    result
}
