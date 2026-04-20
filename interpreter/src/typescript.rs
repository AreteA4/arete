use crate::ast::*;
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// Output structure for TypeScript generation
#[derive(Debug, Clone)]
pub struct TypeScriptOutput {
    pub interfaces: String,
    pub stack_definition: String,
    pub imports: String,
    pub schema_names: Vec<String>,
}

impl TypeScriptOutput {
    pub fn full_file(&self) -> String {
        format!(
            "{}\n\n{}\n\n{}",
            self.imports, self.interfaces, self.stack_definition
        )
    }
}

/// Configuration for TypeScript generation
#[derive(Debug, Clone)]
pub struct TypeScriptConfig {
    pub package_name: String,
    pub generate_helpers: bool,
    pub interface_prefix: String,
    pub export_const_name: String,
    /// WebSocket URL for the stack. If None, generates a placeholder comment.
    pub url: Option<String>,
}

impl Default for TypeScriptConfig {
    fn default() -> Self {
        Self {
            package_name: "@usearete/react".to_string(),
            generate_helpers: true,
            interface_prefix: "".to_string(),
            export_const_name: "STACK".to_string(),
            url: None,
        }
    }
}

/// Trait for generating TypeScript code from AST components
pub trait TypeScriptGenerator {
    fn generate_typescript(&self, config: &TypeScriptConfig) -> String;
}

/// Trait for generating TypeScript interfaces
pub trait TypeScriptInterfaceGenerator {
    fn generate_interface(&self, name: &str, config: &TypeScriptConfig) -> String;
}

/// Trait for generating TypeScript type mappings
pub trait TypeScriptTypeMapper {
    fn to_typescript_type(&self) -> String;
}

/// Main TypeScript compiler for stream specs
pub struct TypeScriptCompiler<S> {
    spec: TypedStreamSpec<S>,
    entity_name: String,
    config: TypeScriptConfig,
    idl: Option<serde_json::Value>, // IDL for enum type generation
    handlers_json: Option<serde_json::Value>, // Raw handlers for event interface generation
    views: Vec<ViewDef>,            // View definitions for derived views
    already_emitted_types: HashSet<String>,
}

impl<S> TypeScriptCompiler<S> {
    pub fn new(spec: TypedStreamSpec<S>, entity_name: String) -> Self {
        Self {
            spec,
            entity_name,
            config: TypeScriptConfig::default(),
            idl: None,
            handlers_json: None,
            views: Vec::new(),
            already_emitted_types: HashSet::new(),
        }
    }

    pub fn with_config(mut self, config: TypeScriptConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_idl(mut self, idl: Option<serde_json::Value>) -> Self {
        self.idl = idl;
        self
    }

    pub fn with_handlers_json(mut self, handlers: Option<serde_json::Value>) -> Self {
        self.handlers_json = handlers;
        self
    }

    pub fn with_views(mut self, views: Vec<ViewDef>) -> Self {
        self.views = views;
        self
    }

    pub fn with_already_emitted_types(mut self, types: HashSet<String>) -> Self {
        self.already_emitted_types = types;
        self
    }

    pub fn compile(&self) -> TypeScriptOutput {
        let imports = self.generate_imports();
        let interfaces = self.generate_interfaces();
        let schema_output = self.generate_schemas();
        let combined_interfaces = if schema_output.definitions.is_empty() {
            interfaces
        } else if interfaces.is_empty() {
            schema_output.definitions.clone()
        } else {
            format!("{}\n\n{}", interfaces, schema_output.definitions)
        };
        let stack_definition = self.generate_stack_definition();

        TypeScriptOutput {
            imports,
            interfaces: combined_interfaces,
            stack_definition,
            schema_names: schema_output.names,
        }
    }

    fn generate_imports(&self) -> String {
        "import { z } from 'zod';".to_string()
    }

    fn generate_view_helpers(&self) -> String {
        r#"// ============================================================================
// View Definition Types (framework-agnostic)
// ============================================================================

/** View definition with embedded entity type */
export interface ViewDef<T, TMode extends 'state' | 'list'> {
  readonly mode: TMode;
  readonly view: string;
  /** Phantom field for type inference - not present at runtime */
  readonly _entity?: T;
}

/** Helper to create typed state view definitions (keyed lookups) */
function stateView<T>(view: string): ViewDef<T, 'state'> {
  return { mode: 'state', view } as const;
}

/** Helper to create typed list view definitions (collections) */
function listView<T>(view: string): ViewDef<T, 'list'> {
  return { mode: 'list', view } as const;
}"#
        .to_string()
    }

    fn generate_interfaces(&self) -> String {
        let mut interfaces = Vec::new();
        let mut processed_types = HashSet::new();
        let all_sections = self.collect_interface_sections();

        // Deduplicate fields within each section and generate interfaces
        // Skip root section - its fields will be flattened into main entity interface
        for (section_name, fields) in all_sections {
            if !is_root_section(&section_name) && processed_types.insert(section_name.clone()) {
                let deduplicated_fields = self.deduplicate_fields(fields);
                let interface =
                    self.generate_interface_from_fields(&section_name, &deduplicated_fields);
                interfaces.push(interface);
            }
        }

        // Generate main entity interface
        let main_interface = self.generate_main_entity_interface();
        interfaces.push(main_interface);

        let nested_interfaces = self.generate_nested_interfaces();
        interfaces.extend(nested_interfaces);

        let builtin_interfaces = self.generate_builtin_resolver_interfaces();
        interfaces.extend(builtin_interfaces);

        if self.has_event_types() {
            interfaces.push(self.generate_event_wrapper_interface());
        }

        interfaces.join("\n\n")
    }

    fn collect_interface_sections(&self) -> BTreeMap<String, Vec<TypeScriptField>> {
        let mut all_sections: BTreeMap<String, Vec<TypeScriptField>> = BTreeMap::new();

        // Collect all interface sections from all handlers
        for handler in &self.spec.handlers {
            let interface_sections = self.extract_interface_sections_from_handler(handler);

            for (section_name, mut fields) in interface_sections {
                all_sections
                    .entry(section_name)
                    .or_default()
                    .append(&mut fields);
            }
        }

        // Add unmapped fields from spec.sections ONCE (not per handler)
        // These are fields without #[map] or #[event] attributes
        self.add_unmapped_fields(&mut all_sections);

        all_sections
    }

    fn deduplicate_fields(&self, mut fields: Vec<TypeScriptField>) -> Vec<TypeScriptField> {
        let mut seen = HashSet::new();
        let mut unique_fields = Vec::new();

        // Sort fields by name for consistent output
        fields.sort_by(|a, b| a.name.cmp(&b.name));

        for field in fields {
            if seen.insert(field.name.clone()) {
                unique_fields.push(field);
            }
        }

        unique_fields
    }

    fn extract_interface_sections_from_handler(
        &self,
        handler: &TypedHandlerSpec<S>,
    ) -> BTreeMap<String, Vec<TypeScriptField>> {
        let mut sections: BTreeMap<String, Vec<TypeScriptField>> = BTreeMap::new();

        for mapping in &handler.mappings {
            if !mapping.emit {
                continue;
            }
            let parts: Vec<&str> = mapping.target_path.split('.').collect();

            if parts.len() > 1 {
                let section_name = parts[0];
                let field_name = parts[1];

                let ts_field = TypeScriptField {
                    name: field_name.to_string(),
                    ts_type: self.mapping_to_typescript_type(mapping),
                    optional: self.is_field_optional(mapping),
                    description: None,
                };

                sections
                    .entry(section_name.to_string())
                    .or_default()
                    .push(ts_field);
            } else {
                let ts_field = TypeScriptField {
                    name: mapping.target_path.clone(),
                    ts_type: self.mapping_to_typescript_type(mapping),
                    optional: self.is_field_optional(mapping),
                    description: None,
                };

                sections
                    .entry("Root".to_string())
                    .or_default()
                    .push(ts_field);
            }
        }

        sections
    }

    fn add_unmapped_fields(&self, sections: &mut BTreeMap<String, Vec<TypeScriptField>>) {
        // NEW: Enhanced approach using AST type information if available
        if !self.spec.sections.is_empty() {
            // Use type information from the enhanced AST
            for section in &self.spec.sections {
                let section_fields = sections.entry(section.name.clone()).or_default();

                for field_info in &section.fields {
                    if !field_info.emit {
                        continue;
                    }
                    // Check if field is already mapped
                    let already_exists = section_fields
                        .iter()
                        .any(|f| f.name == field_info.field_name);

                    if !already_exists {
                        // For computed fields, check field_mappings for resolver type info
                        let field_path = format!("{}.{}", section.name, field_info.field_name);
                        let effective_field_info =
                            if let Some(mapping) = self.spec.field_mappings.get(&field_path) {
                                // Use mapping's inner_type if it's a resolver output type
                                if mapping
                                    .inner_type
                                    .as_ref()
                                    .is_some_and(|t| is_builtin_resolver_type(t))
                                {
                                    mapping
                                } else {
                                    field_info
                                }
                            } else {
                                field_info
                            };

                        section_fields.push(TypeScriptField {
                            name: field_info.field_name.clone(),
                            ts_type: self.field_type_info_to_typescript(effective_field_info),
                            optional: field_info.is_optional,
                            description: None,
                        });
                    }
                }
            }
        } else {
            // FALLBACK: Use field mappings from spec if sections aren't available yet
            for (field_path, field_type_info) in &self.spec.field_mappings {
                if !field_type_info.emit {
                    continue;
                }
                let parts: Vec<&str> = field_path.split('.').collect();
                if parts.len() > 1 {
                    let section_name = parts[0];
                    let field_name = parts[1];

                    let section_fields = sections.entry(section_name.to_string()).or_default();

                    let already_exists = section_fields.iter().any(|f| f.name == field_name);

                    if !already_exists {
                        section_fields.push(TypeScriptField {
                            name: field_name.to_string(),
                            ts_type: self.base_type_to_typescript(
                                &field_type_info.base_type,
                                field_type_info.is_array,
                            ),
                            optional: field_type_info.is_optional,
                            description: None,
                        });
                    }
                }
            }
        }
    }

    fn generate_interface_from_fields(&self, name: &str, fields: &[TypeScriptField]) -> String {
        let interface_name = self.section_interface_name(name);

        // All fields are optional (?) since we receive patches - field may not yet exist
        // For spec-optional fields, we use `T | null` to distinguish "explicitly null" from "not received"
        let field_definitions: Vec<String> = fields
            .iter()
            .map(|field| {
                let ts_type = if field.optional {
                    // Spec-optional: can be explicitly null
                    format!("{} | null", field.ts_type)
                } else {
                    field.ts_type.clone()
                };
                format!("  {}?: {};", field.name, ts_type)
            })
            .collect();

        format!(
            "export interface {} {{\n{}\n}}",
            interface_name,
            field_definitions.join("\n")
        )
    }

    fn section_interface_name(&self, name: &str) -> String {
        if name == "Root" {
            format!(
                "{}{}",
                self.config.interface_prefix,
                to_pascal_case(&self.entity_name)
            )
        } else {
            // Create compound names like GameEvents, GameStatus, etc.
            // Extract the base name (e.g., "Game" from "TestGame" or "SettlementGame")
            let base_name = if self.entity_name.contains("Game") {
                "Game"
            } else {
                &self.entity_name
            };
            format!(
                "{}{}{}",
                self.config.interface_prefix,
                base_name,
                to_pascal_case(name)
            )
        }
    }

    fn generate_main_entity_interface(&self) -> String {
        let entity_name = to_pascal_case(&self.entity_name);

        // Extract all top-level sections from the handlers
        let mut sections = BTreeMap::new();

        for handler in &self.spec.handlers {
            for mapping in &handler.mappings {
                if !mapping.emit {
                    continue;
                }
                let parts: Vec<&str> = mapping.target_path.split('.').collect();
                if parts.len() > 1 {
                    sections.insert(parts[0], true);
                }
            }
        }

        if !self.spec.sections.is_empty() {
            for section in &self.spec.sections {
                if section.fields.iter().any(|field| field.emit) {
                    sections.insert(&section.name, true);
                }
            }
        } else {
            for mapping in &self.spec.handlers {
                for field_mapping in &mapping.mappings {
                    if !field_mapping.emit {
                        continue;
                    }
                    let parts: Vec<&str> = field_mapping.target_path.split('.').collect();
                    if parts.len() > 1 {
                        sections.insert(parts[0], true);
                    }
                }
            }
        }

        let mut fields = Vec::new();

        // Add non-root sections as nested interface references
        // All fields are optional since we receive patches
        for section in sections.keys() {
            if !is_root_section(section) {
                let base_name = if self.entity_name.contains("Game") {
                    "Game"
                } else {
                    &self.entity_name
                };
                let section_interface_name = format!("{}{}", base_name, to_pascal_case(section));
                // Keep section field names as-is (snake_case from AST)
                fields.push(format!("  {}?: {};", section, section_interface_name));
            }
        }

        // Flatten root section fields directly into main interface
        // All fields are optional (?) since we receive patches
        for section in &self.spec.sections {
            if is_root_section(&section.name) {
                for field in &section.fields {
                    if !field.emit {
                        continue;
                    }
                    let base_ts_type = self.field_type_info_to_typescript(field);
                    let ts_type = if field.is_optional {
                        format!("{} | null", base_ts_type)
                    } else {
                        base_ts_type
                    };
                    fields.push(format!("  {}?: {};", field.field_name, ts_type));
                }
            }
        }

        if fields.is_empty() {
            fields.push("  // Generated interface - extend as needed".to_string());
        }

        format!(
            "export interface {} {{\n{}\n}}",
            entity_name,
            fields.join("\n")
        )
    }

    fn generate_schemas(&self) -> SchemaOutput {
        let mut definitions = Vec::new();
        let mut names = Vec::new();
        let mut seen = HashSet::new();

        let mut push_schema = |schema_name: String, definition: String| {
            if seen.insert(schema_name.clone()) {
                names.push(schema_name);
                definitions.push(definition);
            }
        };

        for (schema_name, definition) in self.generate_builtin_resolver_schemas() {
            push_schema(schema_name, definition);
        }

        if self.has_event_types() {
            push_schema(
                "EventWrapperSchema".to_string(),
                self.generate_event_wrapper_schema(),
            );
        }

        for (schema_name, definition) in self.generate_resolved_type_schemas() {
            push_schema(schema_name, definition);
        }

        for (schema_name, definition) in self.generate_event_schemas() {
            push_schema(schema_name, definition);
        }

        for (schema_name, definition) in self.generate_idl_enum_schemas() {
            push_schema(schema_name, definition);
        }

        let all_sections = self.collect_interface_sections();

        for (section_name, fields) in &all_sections {
            if is_root_section(section_name) {
                continue;
            }
            let deduplicated_fields = self.deduplicate_fields(fields.clone());
            let interface_name = self.section_interface_name(section_name);
            let schema_definition =
                self.generate_schema_for_fields(&interface_name, &deduplicated_fields, false);
            push_schema(format!("{}Schema", interface_name), schema_definition);
        }

        let entity_name = to_pascal_case(&self.entity_name);
        let main_fields = self.collect_main_entity_fields();
        let entity_schema = self.generate_schema_for_fields(&entity_name, &main_fields, false);
        push_schema(format!("{}Schema", entity_name), entity_schema);

        let completed_schema = self.generate_completed_entity_schema(&entity_name);
        push_schema(format!("{}CompletedSchema", entity_name), completed_schema);

        SchemaOutput {
            definitions: definitions.join("\n\n"),
            names,
        }
    }

    fn generate_event_wrapper_schema(&self) -> String {
        r#"export const EventWrapperSchema = <T extends z.ZodTypeAny>(data: T) => z.object({
  timestamp: z.number(),
  data,
  slot: z.number().optional(),
  signature: z.string().optional(),
});"#
            .to_string()
    }

    fn generate_builtin_resolver_schemas(&self) -> Vec<(String, String)> {
        let mut schemas = Vec::new();
        let registry = crate::resolvers::builtin_resolver_registry();

        for resolver in registry.definitions() {
            let output_type = resolver.output_type();
            let should_emit = self.uses_builtin_type(output_type)
                && !self.already_emitted_types.contains(output_type);

            // Also check if any types from the resolver's typescript_schema are used
            let extra_types_used = if let Some(ts_schema) = resolver.typescript_schema() {
                // Extract type names from export statements (simple string parsing)
                ts_schema.definition.lines().any(|line| {
                    let line = line.trim();
                    // Match "export const TypeNameSchema"
                    if let Some(rest) = line.strip_prefix("export const ") {
                        let parts: Vec<&str> = rest.split_whitespace().collect();
                        if parts.len() >= 2 && parts[1] == "=" {
                            // Extract the base type name from "TypeNameSchema"
                            let schema_name = parts[0];
                            if let Some(type_name) = schema_name.strip_suffix("Schema") {
                                return self.uses_builtin_type(type_name)
                                    && !self.already_emitted_types.contains(type_name);
                            }
                        }
                    }
                    false
                })
            } else {
                false
            };

            if (should_emit || extra_types_used)
                && !self.already_emitted_types.contains(output_type)
            {
                if let Some(schema) = resolver.typescript_schema() {
                    schemas.push((schema.name.to_string(), schema.definition.to_string()));
                }
            }
        }

        schemas
    }

    fn uses_builtin_type(&self, type_name: &str) -> bool {
        // Check section fields
        for section in &self.spec.sections {
            for field in &section.fields {
                if field.inner_type.as_deref() == Some(type_name) {
                    return true;
                }
            }
        }
        // Check field_mappings for computed fields (they may have resolver types not in sections)
        for field_info in self.spec.field_mappings.values() {
            if field_info.inner_type.as_deref() == Some(type_name) {
                return true;
            }
        }
        false
    }

    fn generate_builtin_resolver_interfaces(&self) -> Vec<String> {
        let mut interfaces = Vec::new();
        let registry = crate::resolvers::builtin_resolver_registry();

        for resolver in registry.definitions() {
            let output_type = resolver.output_type();
            let should_emit = self.uses_builtin_type(output_type)
                && !self.already_emitted_types.contains(output_type);

            // Also check if any types from the resolver's typescript_interface are used
            let extra_types_used = if let Some(ts_interface) = resolver.typescript_interface() {
                // Extract type names from export statements (simple string parsing)
                ts_interface.lines().any(|line| {
                    let line = line.trim();
                    // Match "export type TypeName" or "export interface TypeName"
                    if let Some(rest) = line.strip_prefix("export type ") {
                        if let Some(type_name) = rest.split_whitespace().next() {
                            return self.uses_builtin_type(type_name)
                                && !self.already_emitted_types.contains(type_name);
                        }
                    } else if let Some(rest) = line.strip_prefix("export interface ") {
                        if let Some(type_name) = rest.split_whitespace().next() {
                            return self.uses_builtin_type(type_name)
                                && !self.already_emitted_types.contains(type_name);
                        }
                    }
                    false
                })
            } else {
                false
            };

            if should_emit || extra_types_used {
                if let Some(interface) = resolver.typescript_interface() {
                    interfaces.push(interface.to_string());
                }
            }
        }

        interfaces
    }

    fn collect_main_entity_fields(&self) -> Vec<TypeScriptField> {
        let mut sections = BTreeMap::new();

        for handler in &self.spec.handlers {
            for mapping in &handler.mappings {
                if !mapping.emit {
                    continue;
                }
                let parts: Vec<&str> = mapping.target_path.split('.').collect();
                if parts.len() > 1 {
                    sections.insert(parts[0], true);
                }
            }
        }

        if !self.spec.sections.is_empty() {
            for section in &self.spec.sections {
                if section.fields.iter().any(|field| field.emit) {
                    sections.insert(&section.name, true);
                }
            }
        } else {
            for mapping in &self.spec.handlers {
                for field_mapping in &mapping.mappings {
                    if !field_mapping.emit {
                        continue;
                    }
                    let parts: Vec<&str> = field_mapping.target_path.split('.').collect();
                    if parts.len() > 1 {
                        sections.insert(parts[0], true);
                    }
                }
            }
        }

        let mut fields = Vec::new();

        for section in sections.keys() {
            if !is_root_section(section) {
                let base_name = if self.entity_name.contains("Game") {
                    "Game"
                } else {
                    &self.entity_name
                };
                let section_interface_name = format!("{}{}", base_name, to_pascal_case(section));
                fields.push(TypeScriptField {
                    name: section.to_string(),
                    ts_type: section_interface_name,
                    optional: false,
                    description: None,
                });
            }
        }

        for section in &self.spec.sections {
            if is_root_section(&section.name) {
                for field in &section.fields {
                    if !field.emit {
                        continue;
                    }
                    fields.push(TypeScriptField {
                        name: field.field_name.clone(),
                        ts_type: self.field_type_info_to_typescript(field),
                        optional: field.is_optional,
                        description: None,
                    });
                }
            }
        }

        fields
    }

    fn generate_schema_for_fields(
        &self,
        name: &str,
        fields: &[TypeScriptField],
        required: bool,
    ) -> String {
        let mut field_definitions = Vec::new();

        for field in fields {
            let base_schema = self.typescript_type_to_zod(&field.ts_type);
            let schema = if required {
                base_schema
            } else {
                let with_nullable = if field.optional {
                    format!("{}.nullable()", base_schema)
                } else {
                    base_schema
                };
                format!("{}.optional()", with_nullable)
            };

            field_definitions.push(format!("  {}: {},", field.name, schema));
        }

        format!(
            "export const {}Schema = z.object({{\n{}\n}});",
            name,
            field_definitions.join("\n")
        )
    }

    fn generate_completed_entity_schema(&self, entity_name: &str) -> String {
        let main_fields = self.collect_main_entity_fields();
        self.generate_schema_for_fields(&format!("{}Completed", entity_name), &main_fields, true)
    }

    fn generate_resolved_type_schemas(&self) -> Vec<(String, String)> {
        let mut schemas = Vec::new();
        let mut generated_types = HashSet::new();

        for section in &self.spec.sections {
            for field_info in &section.fields {
                if let Some(resolved) = &field_info.resolved_type {
                    let type_name = to_pascal_case(&resolved.type_name);

                    if !generated_types.insert(type_name.clone()) {
                        continue;
                    }

                    if resolved.is_enum {
                        let variants: Vec<String> = resolved
                            .enum_variants
                            .iter()
                            .map(|v| format!("\"{}\"", to_pascal_case(v)))
                            .collect();
                        let schema = if variants.is_empty() {
                            format!("export const {}Schema = z.string();", type_name)
                        } else {
                            format!(
                                "export const {}Schema = z.enum([{}]);",
                                type_name,
                                variants.join(", ")
                            )
                        };
                        schemas.push((format!("{}Schema", type_name), schema));
                        continue;
                    }

                    let mut field_definitions = Vec::new();
                    for field in &resolved.fields {
                        let base = self.resolved_field_to_zod(field);
                        let schema = if field.is_optional {
                            format!("{}.nullable().optional()", base)
                        } else {
                            format!("{}.optional()", base)
                        };
                        field_definitions.push(format!("  {}: {},", field.field_name, schema));
                    }

                    let schema = format!(
                        "export const {}Schema = z.object({{\n{}\n}});",
                        type_name,
                        field_definitions.join("\n")
                    );
                    schemas.push((format!("{}Schema", type_name), schema));
                }
            }
        }

        schemas
    }

    fn generate_event_schemas(&self) -> Vec<(String, String)> {
        let mut schemas = Vec::new();
        let mut generated_types = HashSet::new();

        let handlers = match &self.handlers_json {
            Some(h) => h.as_array(),
            None => return schemas,
        };

        let handlers_array = match handlers {
            Some(arr) => arr,
            None => return schemas,
        };

        for handler in handlers_array {
            if let Some(mappings) = handler.get("mappings").and_then(|m| m.as_array()) {
                for mapping in mappings {
                    if let Some(target_path) = mapping.get("target_path").and_then(|t| t.as_str()) {
                        if target_path.contains(".events.") || target_path.starts_with("events.") {
                            if let Some(source) = mapping.get("source") {
                                if let Some(event_data) = self.extract_event_data(source) {
                                    if let Some(handler_source) = handler.get("source") {
                                        if let Some(instruction_name) =
                                            self.extract_instruction_name(handler_source)
                                        {
                                            let event_field_name =
                                                target_path.split('.').next_back().unwrap_or("");
                                            let interface_name = format!(
                                                "{}Event",
                                                to_pascal_case(event_field_name)
                                            );

                                            if generated_types.insert(interface_name.clone()) {
                                                if let Some(schema) = self
                                                    .generate_event_schema_from_idl(
                                                        &interface_name,
                                                        &instruction_name,
                                                        &event_data,
                                                    )
                                                {
                                                    schemas.push((
                                                        format!("{}Schema", interface_name),
                                                        schema,
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        schemas
    }

    fn generate_event_schema_from_idl(
        &self,
        interface_name: &str,
        rust_instruction_name: &str,
        captured_fields: &[(String, Option<String>)],
    ) -> Option<String> {
        if captured_fields.is_empty() {
            return Some(format!(
                "export const {}Schema = z.object({{}});",
                interface_name
            ));
        }

        let idl_value = self.idl.as_ref()?;
        let instructions = idl_value.get("instructions")?.as_array()?;

        let instruction = self.find_instruction_in_idl(instructions, rust_instruction_name)?;
        let args = instruction.get("args")?.as_array()?;

        let mut fields = Vec::new();
        for (field_name, transform) in captured_fields {
            for arg in args {
                if let Some(arg_name) = arg.get("name").and_then(|n| n.as_str()) {
                    if arg_name == field_name {
                        if let Some(arg_type) = arg.get("type") {
                            let ts_type =
                                self.idl_type_to_typescript(arg_type, transform.as_deref());
                            let schema = self.typescript_type_to_zod(&ts_type);
                            fields.push(format!("  {}: {},", field_name, schema));
                        }
                        break;
                    }
                }
            }
        }

        Some(format!(
            "export const {}Schema = z.object({{\n{}\n}});",
            interface_name,
            fields.join("\n")
        ))
    }

    fn generate_idl_enum_schemas(&self) -> Vec<(String, String)> {
        let mut schemas = Vec::new();
        let mut generated_types = self.already_emitted_types.clone();

        let idl_value = match &self.idl {
            Some(idl) => idl,
            None => return schemas,
        };

        let types_array = match idl_value.get("types").and_then(|v| v.as_array()) {
            Some(types) => types,
            None => return schemas,
        };

        for type_def in types_array {
            if let (Some(type_name), Some(type_obj)) = (
                type_def.get("name").and_then(|v| v.as_str()),
                type_def.get("type").and_then(|v| v.as_object()),
            ) {
                if type_obj.get("kind").and_then(|v| v.as_str()) == Some("enum") {
                    if !generated_types.insert(type_name.to_string()) {
                        continue;
                    }
                    if let Some(variants) = type_obj.get("variants").and_then(|v| v.as_array()) {
                        let variant_names: Vec<String> = variants
                            .iter()
                            .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
                            .map(|s| format!("\"{}\"", to_pascal_case(s)))
                            .collect();

                        let interface_name = to_pascal_case(type_name);
                        let schema = if variant_names.is_empty() {
                            format!("export const {}Schema = z.string();", interface_name)
                        } else {
                            format!(
                                "export const {}Schema = z.enum([{}]);",
                                interface_name,
                                variant_names.join(", ")
                            )
                        };
                        schemas.push((format!("{}Schema", interface_name), schema));
                    }
                }
            }
        }

        schemas
    }

    fn typescript_type_to_zod(&self, ts_type: &str) -> String {
        let trimmed = ts_type.trim();

        if let Some(inner) = trimmed.strip_suffix("[]") {
            return format!("z.array({})", self.typescript_type_to_zod(inner));
        }

        if let Some(inner) = trimmed.strip_prefix("EventWrapper<") {
            if let Some(inner) = inner.strip_suffix('>') {
                return format!("EventWrapperSchema({})", self.typescript_type_to_zod(inner));
            }
        }

        match trimmed {
            "string" => "z.string()".to_string(),
            "number" => "z.number()".to_string(),
            "boolean" => "z.boolean()".to_string(),
            "any" => "z.any()".to_string(),
            "Record<string, any>" => "z.record(z.any())".to_string(),
            _ => format!("{}Schema", trimmed),
        }
    }

    fn resolved_field_to_zod(&self, field: &ResolvedField) -> String {
        let base = self.base_type_to_zod(&field.base_type);
        if field.is_array {
            format!("z.array({})", base)
        } else {
            base
        }
    }

    fn generate_stack_definition(&self) -> String {
        let stack_name = to_kebab_case(&self.entity_name);
        let entity_pascal = to_pascal_case(&self.entity_name);
        let export_name = format!(
            "{}_{}",
            self.entity_name.to_uppercase(),
            self.config.export_const_name
        );

        let view_helpers = self.generate_view_helpers();
        let derived_views = self.generate_derived_view_entries();
        let schema_names = self.generate_schemas().names;
        let mut unique_schemas: BTreeSet<String> = BTreeSet::new();
        for name in schema_names {
            unique_schemas.insert(name);
        }
        let schemas_block = if unique_schemas.is_empty() {
            String::new()
        } else {
            let schema_entries: Vec<String> = unique_schemas
                .iter()
                .filter(|name| name.ends_with("Schema"))
                .map(|name| format!("    {}: {},", name.trim_end_matches("Schema"), name))
                .collect();
            if schema_entries.is_empty() {
                String::new()
            } else {
                format!("\n  schemas: {{\n{}\n  }},", schema_entries.join("\n"))
            }
        };

        // Generate URL line - either actual URL or placeholder comment
        let url_line = match &self.config.url {
            Some(url) => format!("  url: '{}',", url),
            None => "  // url: 'wss://your-stack-url.stack.arete.run', // TODO: Set after first deployment".to_string(),
        };

        format!(
            r#"{}

// ============================================================================
// Stack Definition
// ============================================================================

/** Stack definition for {} */
export const {} = {{
  name: '{}',
{}
  views: {{
    {}: {{
      state: stateView<{}>('{}/state'),
      list: listView<{}>('{}/list'),{}
    }},
  }},{}
}} as const;

/** Type alias for the stack */
export type {}Stack = typeof {};

/** Default export for convenience */
export default {};"#,
            view_helpers,
            entity_pascal,
            export_name,
            stack_name,
            url_line,
            self.entity_name,
            entity_pascal,
            self.entity_name,
            entity_pascal,
            self.entity_name,
            derived_views,
            schemas_block,
            entity_pascal,
            export_name,
            export_name
        )
    }

    fn generate_derived_view_entries(&self) -> String {
        let derived_views: Vec<&ViewDef> = self
            .views
            .iter()
            .filter(|v| {
                !v.id.ends_with("/state")
                    && !v.id.ends_with("/list")
                    && v.id.starts_with(&self.entity_name)
            })
            .collect();

        if derived_views.is_empty() {
            return String::new();
        }

        let entity_pascal = to_pascal_case(&self.entity_name);
        let mut entries = Vec::new();

        for view in derived_views {
            let view_name = view.id.split('/').nth(1).unwrap_or("unknown");

            entries.push(format!(
                "\n      {}: listView<{}>('{}'),",
                view_name, entity_pascal, view.id
            ));
        }

        entries.join("")
    }

    fn mapping_to_typescript_type(&self, mapping: &TypedFieldMapping<S>) -> String {
        // First, try to resolve from AST field mappings
        if let Some(field_info) = self.spec.field_mappings.get(&mapping.target_path) {
            let ts_type = self.field_type_info_to_typescript(field_info);

            // If it's an Append strategy, wrap in array
            if matches!(mapping.population, PopulationStrategy::Append) {
                return if ts_type.ends_with("[]") {
                    ts_type
                } else {
                    format!("{}[]", ts_type)
                };
            }

            return ts_type;
        }

        // Fallback to legacy inference
        match &mapping.population {
            PopulationStrategy::Append => {
                // For arrays, try to infer the element type
                match &mapping.source {
                    MappingSource::AsEvent { .. } => "any[]".to_string(),
                    _ => "any[]".to_string(),
                }
            }
            _ => {
                // Infer type from source and field name
                let base_type = match &mapping.source {
                    MappingSource::FromSource { .. } => {
                        self.infer_type_from_field_name(&mapping.target_path)
                    }
                    MappingSource::Constant(value) => value_to_typescript_type(value),
                    MappingSource::AsEvent { .. } => "any".to_string(),
                    _ => "any".to_string(),
                };

                // Apply transformations to type
                if let Some(transform) = &mapping.transform {
                    match transform {
                        Transformation::HexEncode | Transformation::HexDecode => {
                            "string".to_string()
                        }
                        Transformation::Base58Encode | Transformation::Base58Decode => {
                            "string".to_string()
                        }
                        Transformation::ToString => "string".to_string(),
                        Transformation::ToNumber => "number".to_string(),
                    }
                } else {
                    base_type
                }
            }
        }
    }

    fn field_type_info_to_typescript(&self, field_info: &FieldTypeInfo) -> String {
        if let Some(resolved) = &field_info.resolved_type {
            let interface_name = self.resolved_type_to_interface_name(resolved);

            let base_type = if resolved.is_event || (resolved.is_instruction && field_info.is_array)
            {
                format!("EventWrapper<{}>", interface_name)
            } else {
                interface_name
            };

            let with_array = if field_info.is_array {
                format!("{}[]", base_type)
            } else {
                base_type
            };

            return with_array;
        }

        if let Some(inner_type) = &field_info.inner_type {
            if is_builtin_resolver_type(inner_type) {
                return inner_type.clone();
            }
        }

        if field_info.base_type == BaseType::Any
            || (field_info.base_type == BaseType::Array
                && field_info.inner_type.as_deref() == Some("Value"))
        {
            if let Some(event_type) = self.find_event_interface_for_field(&field_info.field_name) {
                return if field_info.is_array {
                    format!("{}[]", event_type)
                } else if field_info.is_optional {
                    format!("{} | null", event_type)
                } else {
                    event_type
                };
            }
        }

        self.base_type_to_typescript(&field_info.base_type, field_info.is_array)
    }

    /// Find the generated event interface name for a given field
    fn find_event_interface_for_field(&self, field_name: &str) -> Option<String> {
        // Use the raw JSON handlers if available
        let handlers = self.handlers_json.as_ref()?.as_array()?;

        // Look through handlers to find event mappings for this field
        for handler in handlers {
            if let Some(mappings) = handler.get("mappings").and_then(|m| m.as_array()) {
                for mapping in mappings {
                    if let Some(target_path) = mapping.get("target_path").and_then(|t| t.as_str()) {
                        // Check if this mapping targets our field (e.g., "events.created")
                        let target_parts: Vec<&str> = target_path.split('.').collect();
                        if let Some(target_field) = target_parts.last() {
                            if *target_field == field_name {
                                // Check if this is an event mapping
                                if let Some(source) = mapping.get("source") {
                                    if self.extract_event_data(source).is_some() {
                                        // Generate the interface name (e.g., "created" -> "CreatedEvent")
                                        return Some(format!(
                                            "{}Event",
                                            to_pascal_case(field_name)
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Generate TypeScript interface name from resolved type
    fn resolved_type_to_interface_name(&self, resolved: &ResolvedStructType) -> String {
        to_pascal_case(&resolved.type_name)
    }

    /// Generate nested interfaces for all resolved types in the AST
    fn generate_nested_interfaces(&self) -> Vec<String> {
        let mut interfaces = Vec::new();
        let mut generated_types = self.already_emitted_types.clone();

        // Collect all resolved types from all sections
        for section in &self.spec.sections {
            for field_info in &section.fields {
                if let Some(resolved) = &field_info.resolved_type {
                    let type_name = resolved.type_name.clone();

                    // Only generate each type once
                    if generated_types.insert(type_name) {
                        let interface = self.generate_interface_for_resolved_type(resolved);
                        interfaces.push(interface);
                    }
                }
            }
        }

        // Generate event interfaces from instruction handlers
        interfaces.extend(self.generate_event_interfaces(&mut generated_types));

        // Also generate all enum types from the IDL (even if not directly referenced)
        if let Some(idl_value) = &self.idl {
            if let Some(types_array) = idl_value.get("types").and_then(|v| v.as_array()) {
                for type_def in types_array {
                    if let (Some(type_name), Some(type_obj)) = (
                        type_def.get("name").and_then(|v| v.as_str()),
                        type_def.get("type").and_then(|v| v.as_object()),
                    ) {
                        if type_obj.get("kind").and_then(|v| v.as_str()) == Some("enum") {
                            // Only generate if not already generated
                            if generated_types.insert(type_name.to_string()) {
                                if let Some(variants) =
                                    type_obj.get("variants").and_then(|v| v.as_array())
                                {
                                    let variant_names: Vec<String> = variants
                                        .iter()
                                        .filter_map(|v| {
                                            v.get("name")
                                                .and_then(|n| n.as_str())
                                                .map(|s| s.to_string())
                                        })
                                        .collect();

                                    if !variant_names.is_empty() {
                                        let interface_name = to_pascal_case(type_name);
                                        let variant_strings: Vec<String> = variant_names
                                            .iter()
                                            .map(|v| format!("\"{}\"", to_pascal_case(v)))
                                            .collect();

                                        let enum_type = format!(
                                            "export type {} = {};",
                                            interface_name,
                                            variant_strings.join(" | ")
                                        );
                                        interfaces.push(enum_type);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        interfaces
    }

    /// Generate TypeScript interfaces for event types from instruction handlers
    fn generate_event_interfaces(&self, generated_types: &mut HashSet<String>) -> Vec<String> {
        let mut interfaces = Vec::new();

        // Use the raw JSON handlers if available
        let handlers = match &self.handlers_json {
            Some(h) => h.as_array(),
            None => return interfaces,
        };

        let handlers_array = match handlers {
            Some(arr) => arr,
            None => return interfaces,
        };

        // Look through handlers to find instruction-based event mappings
        for handler in handlers_array {
            // Check if this handler has event mappings
            if let Some(mappings) = handler.get("mappings").and_then(|m| m.as_array()) {
                for mapping in mappings {
                    if let Some(target_path) = mapping.get("target_path").and_then(|t| t.as_str()) {
                        // Check if the target is an event field (contains ".events." or starts with "events.")
                        if target_path.contains(".events.") || target_path.starts_with("events.") {
                            // Check if the source is AsEvent
                            if let Some(source) = mapping.get("source") {
                                if let Some(event_data) = self.extract_event_data(source) {
                                    // Extract instruction name from handler source
                                    if let Some(handler_source) = handler.get("source") {
                                        if let Some(instruction_name) =
                                            self.extract_instruction_name(handler_source)
                                        {
                                            // Generate interface name from target path (e.g., "events.created" -> "CreatedEvent")
                                            let event_field_name =
                                                target_path.split('.').next_back().unwrap_or("");
                                            let interface_name = format!(
                                                "{}Event",
                                                to_pascal_case(event_field_name)
                                            );

                                            // Only generate once
                                            if generated_types.insert(interface_name.clone()) {
                                                if let Some(interface) = self
                                                    .generate_event_interface_from_idl(
                                                        &interface_name,
                                                        &instruction_name,
                                                        &event_data,
                                                    )
                                                {
                                                    interfaces.push(interface);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        interfaces
    }

    /// Extract event field data from a mapping source
    fn extract_event_data(
        &self,
        source: &serde_json::Value,
    ) -> Option<Vec<(String, Option<String>)>> {
        if let Some(as_event) = source.get("AsEvent") {
            if let Some(fields) = as_event.get("fields").and_then(|f| f.as_array()) {
                let mut event_fields = Vec::new();
                for field in fields {
                    if let Some(from_source) = field.get("FromSource") {
                        if let Some(path) = from_source
                            .get("path")
                            .and_then(|p| p.get("segments"))
                            .and_then(|s| s.as_array())
                        {
                            // Get the last segment as the field name (e.g., ["data", "game_id"] -> "game_id")
                            if let Some(field_name) = path.last().and_then(|v| v.as_str()) {
                                let transform = from_source
                                    .get("transform")
                                    .and_then(|t| t.as_str())
                                    .map(|s| s.to_string());
                                event_fields.push((field_name.to_string(), transform));
                            }
                        }
                    }
                }
                return Some(event_fields);
            }
        }
        None
    }

    /// Extract instruction name from handler source, returning the raw PascalCase name
    fn extract_instruction_name(&self, source: &serde_json::Value) -> Option<String> {
        if let Some(source_obj) = source.get("Source") {
            if let Some(type_name) = source_obj.get("type_name").and_then(|t| t.as_str()) {
                let instruction_part =
                    crate::event_type_helpers::strip_event_type_suffix(type_name);
                return Some(instruction_part.to_string());
            }
        }
        None
    }

    /// Find an instruction in the IDL by name, handling different naming conventions.
    /// IDLs may use snake_case (pumpfun: "admin_set_creator") or camelCase (ore: "claimSol").
    /// The input name comes from Rust types which are PascalCase ("AdminSetCreator", "ClaimSol").
    fn find_instruction_in_idl<'a>(
        &self,
        instructions: &'a [serde_json::Value],
        rust_name: &str,
    ) -> Option<&'a serde_json::Value> {
        let normalized_search = normalize_for_comparison(rust_name);

        for instruction in instructions {
            if let Some(idl_name) = instruction.get("name").and_then(|n| n.as_str()) {
                if normalize_for_comparison(idl_name) == normalized_search {
                    return Some(instruction);
                }
            }
        }
        None
    }

    /// Generate a TypeScript interface for an event from IDL instruction data
    fn generate_event_interface_from_idl(
        &self,
        interface_name: &str,
        rust_instruction_name: &str,
        captured_fields: &[(String, Option<String>)],
    ) -> Option<String> {
        if captured_fields.is_empty() {
            return Some(format!("export interface {} {{}}", interface_name));
        }

        let idl_value = self.idl.as_ref()?;
        let instructions = idl_value.get("instructions")?.as_array()?;

        let instruction = self.find_instruction_in_idl(instructions, rust_instruction_name)?;
        let args = instruction.get("args")?.as_array()?;

        let mut fields = Vec::new();
        for (field_name, transform) in captured_fields {
            for arg in args {
                if let Some(arg_name) = arg.get("name").and_then(|n| n.as_str()) {
                    if arg_name == field_name {
                        if let Some(arg_type) = arg.get("type") {
                            let ts_type =
                                self.idl_type_to_typescript(arg_type, transform.as_deref());
                            fields.push(format!("  {}: {};", field_name, ts_type));
                        }
                        break;
                    }
                }
            }
        }

        if !fields.is_empty() {
            return Some(format!(
                "export interface {} {{\n{}\n}}",
                interface_name,
                fields.join("\n")
            ));
        }

        None
    }

    /// Convert an IDL type (from JSON) to TypeScript, considering transforms
    fn idl_type_to_typescript(
        &self,
        idl_type: &serde_json::Value,
        transform: Option<&str>,
    ) -> String {
        #![allow(clippy::only_used_in_recursion)]
        // If there's a HexEncode transform, the result is always a string
        if transform == Some("HexEncode") {
            return "string".to_string();
        }

        // Handle different IDL type formats
        if let Some(type_str) = idl_type.as_str() {
            return match type_str {
                "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32" | "i64" | "i128" => {
                    "number".to_string()
                }
                "f32" | "f64" => "number".to_string(),
                "bool" => "boolean".to_string(),
                "string" => "string".to_string(),
                "pubkey" | "publicKey" => "string".to_string(),
                "bytes" => "string".to_string(),
                _ => "any".to_string(),
            };
        }

        // Handle complex types (option, vec, etc.)
        if let Some(type_obj) = idl_type.as_object() {
            if let Some(option_type) = type_obj.get("option") {
                let inner = self.idl_type_to_typescript(option_type, None);
                return format!("{} | null", inner);
            }
            if let Some(vec_type) = type_obj.get("vec") {
                let inner = self.idl_type_to_typescript(vec_type, None);
                return format!("{}[]", inner);
            }
        }

        "any".to_string()
    }

    /// Generate a TypeScript interface from a resolved struct type
    fn generate_interface_for_resolved_type(&self, resolved: &ResolvedStructType) -> String {
        let interface_name = to_pascal_case(&resolved.type_name);

        // Handle enums as TypeScript union types
        if resolved.is_enum {
            let variants: Vec<String> = resolved
                .enum_variants
                .iter()
                .map(|v| format!("\"{}\"", to_pascal_case(v)))
                .collect();

            return format!("export type {} = {};", interface_name, variants.join(" | "));
        }

        // Handle structs as interfaces
        // All fields are optional since we receive patches
        let fields: Vec<String> = resolved
            .fields
            .iter()
            .map(|field| {
                let base_ts_type = self.resolved_field_to_typescript(field);
                let ts_type = if field.is_optional {
                    format!("{} | null", base_ts_type)
                } else {
                    base_ts_type
                };
                format!("  {}?: {};", field.field_name, ts_type)
            })
            .collect();

        format!(
            "export interface {} {{\n{}\n}}",
            interface_name,
            fields.join("\n")
        )
    }

    /// Convert a resolved field to TypeScript type
    fn resolved_field_to_typescript(&self, field: &ResolvedField) -> String {
        let base_ts = self.base_type_to_typescript(&field.base_type, false);

        if field.is_array {
            format!("{}[]", base_ts)
        } else {
            base_ts
        }
    }

    /// Check if the spec has any event types
    fn has_event_types(&self) -> bool {
        for section in &self.spec.sections {
            for field_info in &section.fields {
                if let Some(resolved) = &field_info.resolved_type {
                    if resolved.is_event || (resolved.is_instruction && field_info.is_array) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Generate the EventWrapper interface
    fn generate_event_wrapper_interface(&self) -> String {
        r#"/**
 * Wrapper for event data that includes context metadata.
 * Events are automatically wrapped in this structure at runtime.
 */
export interface EventWrapper<T> {
  /** Unix timestamp when the event was processed */
  timestamp: number;
  /** The event-specific data */
  data: T;
  /** Optional blockchain slot number */
  slot?: number;
  /** Optional transaction signature */
  signature?: string;
}"#
        .to_string()
    }

    fn infer_type_from_field_name(&self, field_name: &str) -> String {
        let lower_name = field_name.to_lowercase();

        // Special case for event fields - these are typically Option<Value> and should be 'any'
        if lower_name.contains("events.") {
            // For fields in the events section, default to 'any' since they're typically Option<Value>
            return "any".to_string();
        }

        // Common patterns for type inference
        if lower_name.contains("id")
            || lower_name.contains("count")
            || lower_name.contains("number")
            || lower_name.contains("timestamp")
            || lower_name.contains("time")
            || lower_name.contains("at")
            || lower_name.contains("volume")
            || lower_name.contains("amount")
            || lower_name.contains("ev")
            || lower_name.contains("fee")
            || lower_name.contains("payout")
            || lower_name.contains("distributed")
            || lower_name.contains("claimable")
            || lower_name.contains("total")
            || lower_name.contains("rate")
            || lower_name.contains("ratio")
            || lower_name.contains("current")
            || lower_name.contains("state")
        {
            "number".to_string()
        } else if lower_name.contains("status")
            || lower_name.contains("hash")
            || lower_name.contains("address")
            || lower_name.contains("key")
        {
            "string".to_string()
        } else {
            "any".to_string()
        }
    }

    fn is_field_optional(&self, mapping: &TypedFieldMapping<S>) -> bool {
        // Most fields should be optional by default since we're dealing with Option<T> types
        match &mapping.source {
            // Constants are typically non-optional
            MappingSource::Constant(_) => false,
            // Events are typically optional (Option<Value>)
            MappingSource::AsEvent { .. } => true,
            // For source fields, default to optional since most Rust fields are Option<T>
            MappingSource::FromSource { .. } => true,
            // Other cases default to optional
            _ => true,
        }
    }

    /// Convert language-agnostic base types to TypeScript types
    fn base_type_to_typescript(&self, base_type: &BaseType, is_array: bool) -> String {
        let base_ts_type = match base_type {
            BaseType::Integer => "number",
            BaseType::Float => "number",
            BaseType::String => "string",
            BaseType::Boolean => "boolean",
            BaseType::Timestamp => "number", // Unix timestamps as numbers
            BaseType::Binary => "string",    // Base64 encoded strings
            BaseType::Pubkey => "string",    // Solana public keys as Base58 strings
            BaseType::Array => "any[]",      // Default array type
            BaseType::Object => "Record<string, any>", // Generic object
            BaseType::Any => "any",
        };

        if is_array && !matches!(base_type, BaseType::Array) {
            format!("{}[]", base_ts_type)
        } else {
            base_ts_type.to_string()
        }
    }

    /// Convert language-agnostic base types to Zod schema expressions
    fn base_type_to_zod(&self, base_type: &BaseType) -> String {
        match base_type {
            BaseType::Integer | BaseType::Float | BaseType::Timestamp => "z.number()".to_string(),
            BaseType::String | BaseType::Pubkey | BaseType::Binary => "z.string()".to_string(),
            BaseType::Boolean => "z.boolean()".to_string(),
            BaseType::Array => "z.array(z.any())".to_string(),
            BaseType::Object => "z.record(z.any())".to_string(),
            BaseType::Any => "z.any()".to_string(),
        }
    }
}

/// Represents a TypeScript field in an interface
#[derive(Debug, Clone)]
struct TypeScriptField {
    name: String,
    ts_type: String,
    optional: bool,
    #[allow(dead_code)]
    description: Option<String>,
}

#[derive(Debug, Clone)]
struct SchemaOutput {
    definitions: String,
    names: Vec<String>,
}

/// Convert serde_json::Value to TypeScript type string
fn value_to_typescript_type(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Number(_) => "number".to_string(),
        serde_json::Value::String(_) => "string".to_string(),
        serde_json::Value::Bool(_) => "boolean".to_string(),
        serde_json::Value::Array(_) => "any[]".to_string(),
        serde_json::Value::Object(_) => "Record<string, any>".to_string(),
        serde_json::Value::Null => "null".to_string(),
    }
}

fn extract_builtin_resolver_type_names(spec: &SerializableStreamSpec) -> HashSet<String> {
    let mut names = HashSet::new();
    let registry = crate::resolvers::builtin_resolver_registry();
    for resolver in registry.definitions() {
        let output_type = resolver.output_type();
        for section in &spec.sections {
            for field in &section.fields {
                if field.inner_type.as_deref() == Some(output_type) {
                    names.insert(output_type.to_string());
                }
            }
        }
    }
    names
}

fn extract_idl_enum_type_names(idl: &serde_json::Value) -> HashSet<String> {
    let mut names = HashSet::new();
    if let Some(types_array) = idl.get("types").and_then(|v| v.as_array()) {
        for type_def in types_array {
            if let (Some(type_name), Some(type_obj)) = (
                type_def.get("name").and_then(|v| v.as_str()),
                type_def.get("type").and_then(|v| v.as_object()),
            ) {
                if type_obj.get("kind").and_then(|v| v.as_str()) == Some("enum") {
                    names.insert(type_name.to_string());
                }
            }
        }
    }
    names
}

/// Extract enum type names that were actually emitted in the generated interfaces.
/// Looks for patterns like `export const DirectionKindSchema = z.enum([...])`
fn extract_emitted_enum_type_names(interfaces: &str, idl: Option<&IdlSnapshot>) -> HashSet<String> {
    let mut names = HashSet::new();

    // Get all enum type names from the IDL
    let idl_enum_names: HashSet<String> = idl
        .and_then(|idl| serde_json::to_value(idl).ok())
        .map(|v| extract_idl_enum_type_names(&v))
        .unwrap_or_default();

    // Look for emitted enum schemas in the interfaces
    // Pattern: export const DirectionKindSchema = z.enum([...]) or z.string() for empty variants
    for line in interfaces.lines() {
        if let Some(start) = line.find("export const ") {
            let end = line
                .find("Schema = z.enum")
                .or_else(|| line.find("Schema = z.string()"));
            if let Some(end) = end {
                let schema_name = line[start + 13..end].trim();
                // Check if this schema name corresponds to an IDL enum type
                if idl_enum_names.contains(schema_name) {
                    names.insert(schema_name.to_string());
                }
            }
        }
    }

    names
}

/// Convert snake_case to PascalCase
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

/// Normalize a name for case-insensitive comparison across naming conventions.
/// Removes underscores and converts to lowercase: "claim_sol", "claimSol", "ClaimSol" all become "claimsol"
fn normalize_for_comparison(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '_')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn is_root_section(name: &str) -> bool {
    name.eq_ignore_ascii_case("root")
}

fn is_builtin_resolver_type(type_name: &str) -> bool {
    crate::resolvers::is_resolver_output_type(type_name)
}

/// Convert PascalCase/camelCase to kebab-case
fn to_kebab_case(s: &str) -> String {
    let mut result = String::new();

    for ch in s.chars() {
        if ch.is_uppercase() && !result.is_empty() {
            result.push('-');
        }
        result.push(ch.to_lowercase().next().unwrap());
    }

    result
}

/// CLI-friendly function to generate TypeScript from a spec function
/// This will be used by the CLI tool to generate TypeScript from discovered specs
pub fn generate_typescript_from_spec_fn<F, S>(
    spec_fn: F,
    entity_name: String,
    config: Option<TypeScriptConfig>,
) -> Result<TypeScriptOutput, String>
where
    F: Fn() -> TypedStreamSpec<S>,
{
    let spec = spec_fn();
    let compiler =
        TypeScriptCompiler::new(spec, entity_name).with_config(config.unwrap_or_default());

    Ok(compiler.compile())
}

/// Write TypeScript output to a file
pub fn write_typescript_to_file(
    output: &TypeScriptOutput,
    path: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::fs::write(path, output.full_file())
}

/// Generate TypeScript from a SerializableStreamSpec (for CLI use)
/// This allows the CLI to compile TypeScript without needing the typed spec
pub fn compile_serializable_spec(
    spec: SerializableStreamSpec,
    entity_name: String,
    config: Option<TypeScriptConfig>,
) -> Result<TypeScriptOutput, String> {
    compile_serializable_spec_with_emitted(spec, entity_name, config, HashSet::new())
}

fn compile_serializable_spec_with_emitted(
    spec: SerializableStreamSpec,
    entity_name: String,
    config: Option<TypeScriptConfig>,
    already_emitted_types: HashSet<String>,
) -> Result<TypeScriptOutput, String> {
    let idl = spec
        .idl
        .as_ref()
        .and_then(|idl_snapshot| serde_json::to_value(idl_snapshot).ok());

    let handlers = serde_json::to_value(&spec.handlers).ok();
    let views = spec.views.clone();

    let typed_spec: TypedStreamSpec<()> = TypedStreamSpec::from_serializable(spec);

    let compiler = TypeScriptCompiler::new(typed_spec, entity_name)
        .with_idl(idl)
        .with_handlers_json(handlers)
        .with_views(views)
        .with_config(config.unwrap_or_default())
        .with_already_emitted_types(already_emitted_types);

    Ok(compiler.compile())
}

#[derive(Debug, Clone)]
pub struct TypeScriptStackConfig {
    pub package_name: String,
    pub generate_helpers: bool,
    pub export_const_name: String,
    pub url: Option<String>,
}

impl Default for TypeScriptStackConfig {
    fn default() -> Self {
        Self {
            package_name: "@usearete/react".to_string(),
            generate_helpers: true,
            export_const_name: "STACK".to_string(),
            url: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TypeScriptStackOutput {
    pub interfaces: String,
    pub stack_definition: String,
    pub imports: String,
}

impl TypeScriptStackOutput {
    pub fn full_file(&self) -> String {
        let mut parts = Vec::new();
        if !self.imports.is_empty() {
            parts.push(self.imports.as_str());
        }
        if !self.interfaces.is_empty() {
            parts.push(self.interfaces.as_str());
        }
        if !self.stack_definition.is_empty() {
            parts.push(self.stack_definition.as_str());
        }
        parts.join("\n\n")
    }
}

/// Compile a full SerializableStackSpec (multi-entity) into a single TypeScript file.
///
/// Generates:
/// - Interfaces for ALL entities (OreRound, OreTreasury, OreMiner, etc.)
/// - A single unified stack definition with nested views per entity
/// - View helpers (stateView, listView)
pub fn compile_stack_spec(
    stack_spec: SerializableStackSpec,
    config: Option<TypeScriptStackConfig>,
) -> Result<TypeScriptStackOutput, String> {
    let config = config.unwrap_or_default();
    let stack_name = &stack_spec.stack_name;
    let stack_kebab = to_kebab_case(stack_name);

    // 1. Compile each entity's interfaces using existing per-entity compiler
    let mut all_interfaces = Vec::new();
    let mut entity_names = Vec::new();
    let mut schema_names: Vec<String> = Vec::new();
    let mut emitted_types: HashSet<String> = HashSet::new();

    for entity_spec in &stack_spec.entities {
        let mut spec = entity_spec.clone();
        // Inject stack-level IDL if entity doesn't have its own
        if spec.idl.is_none() {
            spec.idl = stack_spec.idls.first().cloned();
        }
        let entity_name = spec.state_name.clone();
        entity_names.push(entity_name.clone());

        let per_entity_config = TypeScriptConfig {
            package_name: config.package_name.clone(),
            generate_helpers: false,
            interface_prefix: String::new(),
            export_const_name: config.export_const_name.clone(),
            url: config.url.clone(),
        };

        // Collect builtin type names before spec is consumed
        let builtin_type_names = extract_builtin_resolver_type_names(&spec);
        // Clone IDL before spec is moved so we can check which enums were emitted
        let idl_for_check = spec.idl.clone();

        let output = compile_serializable_spec_with_emitted(
            spec,
            entity_name,
            Some(per_entity_config),
            emitted_types.clone(),
        )?;

        // Track shared types for cross-entity dedup
        // Only track enum types that were actually emitted (found in output.interfaces)
        let emitted_enum_names =
            extract_emitted_enum_type_names(&output.interfaces, idl_for_check.as_ref());
        emitted_types.extend(emitted_enum_names);
        emitted_types.extend(builtin_type_names);

        // Only take the interfaces part (not the stack_definition — we generate our own)
        if !output.interfaces.is_empty() {
            all_interfaces.push(output.interfaces);
        }

        schema_names.extend(output.schema_names);
    }

    let interfaces = all_interfaces.join("\n\n");

    // 2. Generate unified stack definition with all entity views
    let stack_definition = generate_stack_definition_multi(
        stack_name,
        &stack_kebab,
        &stack_spec.entities,
        &entity_names,
        &stack_spec.pdas,
        &stack_spec.program_ids,
        &schema_names,
        &config,
    );

    let imports = if stack_spec.pdas.values().any(|p| !p.is_empty()) {
        "import { z } from 'zod';\nimport { pda, literal, account, arg, bytes } from '@usearete/sdk';".to_string()
    } else {
        "import { z } from 'zod';".to_string()
    };

    Ok(TypeScriptStackOutput {
        imports,
        interfaces,
        stack_definition,
    })
}

/// Write stack-level TypeScript output to a file
pub fn write_stack_typescript_to_file(
    output: &TypeScriptStackOutput,
    path: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::fs::write(path, output.full_file())
}

/// Generate a unified stack definition for multiple entities.
///
/// Produces something like:
/// ```typescript
/// export const ORE_STACK = {
///   name: 'ore',
///   url: 'wss://ore.stack.arete.run',
///   views: {
///     OreRound: {
///       state: stateView<OreRound>('OreRound/state'),
///       list: listView<OreRound>('OreRound/list'),
///       latest: listView<OreRound>('OreRound/latest'),
///     },
///     OreTreasury: {
///       state: stateView<OreTreasury>('OreTreasury/state'),
///     },
///     OreMiner: {
///       state: stateView<OreMiner>('OreMiner/state'),
///       list: listView<OreMiner>('OreMiner/list'),
///     },
///   },
/// } as const;
/// ```
#[allow(clippy::too_many_arguments)]
fn generate_stack_definition_multi(
    stack_name: &str,
    stack_kebab: &str,
    entities: &[SerializableStreamSpec],
    entity_names: &[String],
    pdas: &BTreeMap<String, BTreeMap<String, PdaDefinition>>,
    program_ids: &[String],
    schema_names: &[String],
    config: &TypeScriptStackConfig,
) -> String {
    let export_name = format!(
        "{}_{}",
        to_screaming_snake_case(stack_name),
        config.export_const_name
    );

    let view_helpers = generate_view_helpers_static();

    let url_line = match &config.url {
        Some(url) => format!("  url: '{}',", url),
        None => "  // url: 'wss://your-stack-url.stack.arete.run', // TODO: Set after first deployment".to_string(),
    };

    // Generate views block for each entity
    let mut entity_view_blocks = Vec::new();
    for (i, entity_spec) in entities.iter().enumerate() {
        let entity_name = &entity_names[i];
        let entity_pascal = to_pascal_case(entity_name);

        let mut view_entries = Vec::new();

        view_entries.push(format!(
            "      state: stateView<{entity}>('{entity_name}/state'),",
            entity = entity_pascal,
            entity_name = entity_name
        ));

        view_entries.push(format!(
            "      list: listView<{entity}>('{entity_name}/list'),",
            entity = entity_pascal,
            entity_name = entity_name
        ));

        for view in &entity_spec.views {
            if !view.id.ends_with("/state")
                && !view.id.ends_with("/list")
                && view.id.starts_with(entity_name)
            {
                let view_name = view.id.split('/').nth(1).unwrap_or("unknown");
                view_entries.push(format!(
                    "      {}: listView<{entity}>('{}'),",
                    view_name,
                    view.id,
                    entity = entity_pascal
                ));
            }
        }

        entity_view_blocks.push(format!(
            "    {}: {{\n{}\n    }},",
            entity_name,
            view_entries.join("\n")
        ));
    }

    let views_body = entity_view_blocks.join("\n");

    let pdas_block = generate_pdas_block(pdas, program_ids);

    let mut unique_schemas: BTreeSet<String> = BTreeSet::new();
    for name in schema_names {
        unique_schemas.insert(name.clone());
    }
    let schemas_block = if unique_schemas.is_empty() {
        String::new()
    } else {
        let schema_entries: Vec<String> = unique_schemas
            .iter()
            .filter(|name| name.ends_with("Schema"))
            .map(|name| format!("    {}: {},", name.trim_end_matches("Schema"), name))
            .collect();
        if schema_entries.is_empty() {
            String::new()
        } else {
            format!("\n  schemas: {{\n{}\n  }},", schema_entries.join("\n"))
        }
    };

    let entity_types: Vec<String> = entity_names.iter().map(|n| to_pascal_case(n)).collect();

    format!(
        r#"{view_helpers}

// ============================================================================
// Stack Definition
// ============================================================================

/** Stack definition for {stack_name} with {entity_count} entities */
export const {export_name} = {{
  name: '{stack_kebab}',
{url_line}
  views: {{
{views_body}
  }},{schemas_section}{pdas_section}
}} as const;

/** Type alias for the stack */
export type {stack_name}Stack = typeof {export_name};

/** Entity types in this stack */
export type {stack_name}Entity = {entity_union};

/** Default export for convenience */
export default {export_name};"#,
        view_helpers = view_helpers,
        stack_name = stack_name,
        entity_count = entities.len(),
        export_name = export_name,
        stack_kebab = stack_kebab,
        url_line = url_line,
        views_body = views_body,
        schemas_section = schemas_block,
        pdas_section = pdas_block,
        entity_union = entity_types.join(" | "),
    )
}

fn generate_pdas_block(
    pdas: &BTreeMap<String, BTreeMap<String, PdaDefinition>>,
    program_ids: &[String],
) -> String {
    if pdas.is_empty() {
        return String::new();
    }

    let mut program_blocks = Vec::new();

    for (program_name, program_pdas) in pdas {
        if program_pdas.is_empty() {
            continue;
        }

        let program_id = program_ids.first().cloned().unwrap_or_default();

        let mut pda_entries = Vec::new();
        for (pda_name, pda_def) in program_pdas {
            let seeds_str = pda_def
                .seeds
                .iter()
                .map(|seed| match seed {
                    PdaSeedDef::Literal { value } => format!("literal('{}')", value),
                    PdaSeedDef::AccountRef { account_name } => {
                        format!("account('{}')", account_name)
                    }
                    PdaSeedDef::ArgRef { arg_name, arg_type } => {
                        if let Some(t) = arg_type {
                            format!("arg('{}', '{}')", arg_name, t)
                        } else {
                            format!("arg('{}')", arg_name)
                        }
                    }
                    PdaSeedDef::Bytes { value } => {
                        let bytes_arr: Vec<String> = value.iter().map(|b| b.to_string()).collect();
                        format!("bytes(new Uint8Array([{}]))", bytes_arr.join(", "))
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");

            let pid = pda_def.program_id.as_ref().unwrap_or(&program_id);
            pda_entries.push(format!(
                "      {}: pda('{}', {}),",
                pda_name, pid, seeds_str
            ));
        }

        program_blocks.push(format!(
            "    {}: {{\n{}\n    }},",
            program_name,
            pda_entries.join("\n")
        ));
    }

    if program_blocks.is_empty() {
        return String::new();
    }

    format!("\n  pdas: {{\n{}\n  }},", program_blocks.join("\n"))
}

fn generate_view_helpers_static() -> String {
    r#"// ============================================================================
// View Definition Types (framework-agnostic)
// ============================================================================

/** View definition with embedded entity type */
export interface ViewDef<T, TMode extends 'state' | 'list'> {
  readonly mode: TMode;
  readonly view: string;
  /** Phantom field for type inference - not present at runtime */
  readonly _entity?: T;
}

/** Helper to create typed state view definitions (keyed lookups) */
function stateView<T>(view: string): ViewDef<T, 'state'> {
  return { mode: 'state', view } as const;
}

/** Helper to create typed list view definitions (collections) */
function listView<T>(view: string): ViewDef<T, 'list'> {
  return { mode: 'list', view } as const;
}"#
    .to_string()
}

/// Convert PascalCase to SCREAMING_SNAKE_CASE (e.g., "OreStream" -> "ORE_STREAM")
fn to_screaming_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(ch.to_uppercase().next().unwrap());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_case_conversions() {
        assert_eq!(to_pascal_case("settlement_game"), "SettlementGame");
        assert_eq!(to_kebab_case("SettlementGame"), "settlement-game");
    }

    #[test]
    fn test_normalize_for_comparison() {
        assert_eq!(normalize_for_comparison("claim_sol"), "claimsol");
        assert_eq!(normalize_for_comparison("claimSol"), "claimsol");
        assert_eq!(normalize_for_comparison("ClaimSol"), "claimsol");
        assert_eq!(
            normalize_for_comparison("admin_set_creator"),
            "adminsetcreator"
        );
        assert_eq!(
            normalize_for_comparison("AdminSetCreator"),
            "adminsetcreator"
        );
    }

    #[test]
    fn test_value_to_typescript_type() {
        assert_eq!(value_to_typescript_type(&serde_json::json!(42)), "number");
        assert_eq!(
            value_to_typescript_type(&serde_json::json!("hello")),
            "string"
        );
        assert_eq!(
            value_to_typescript_type(&serde_json::json!(true)),
            "boolean"
        );
        assert_eq!(value_to_typescript_type(&serde_json::json!([])), "any[]");
    }

    #[test]
    fn test_derived_view_codegen() {
        let spec = SerializableStreamSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            state_name: "OreRound".to_string(),
            program_id: None,
            idl: None,
            identity: IdentitySpec {
                primary_keys: vec!["id".to_string()],
                lookup_indexes: vec![],
            },
            handlers: vec![],
            sections: vec![],
            field_mappings: BTreeMap::new(),
            resolver_hooks: vec![],
            resolver_specs: vec![],
            instruction_hooks: vec![],
            computed_fields: vec![],
            computed_field_specs: vec![],
            content_hash: None,
            views: vec![
                ViewDef {
                    id: "OreRound/latest".to_string(),
                    source: ViewSource::Entity {
                        name: "OreRound".to_string(),
                    },
                    pipeline: vec![ViewTransform::Last],
                    output: ViewOutput::Single,
                },
                ViewDef {
                    id: "OreRound/top10".to_string(),
                    source: ViewSource::Entity {
                        name: "OreRound".to_string(),
                    },
                    pipeline: vec![ViewTransform::Take { count: 10 }],
                    output: ViewOutput::Collection,
                },
            ],
        };

        let output =
            compile_serializable_spec(spec, "OreRound".to_string(), None).expect("should compile");

        let stack_def = &output.stack_definition;

        assert!(
            stack_def.contains("listView<OreRound>('OreRound/latest')"),
            "Expected 'latest' derived view using listView, got:\n{}",
            stack_def
        );
        assert!(
            stack_def.contains("listView<OreRound>('OreRound/top10')"),
            "Expected 'top10' derived view using listView, got:\n{}",
            stack_def
        );
        assert!(
            stack_def.contains("latest:"),
            "Expected 'latest' key, got:\n{}",
            stack_def
        );
        assert!(
            stack_def.contains("top10:"),
            "Expected 'top10' key, got:\n{}",
            stack_def
        );
        assert!(
            stack_def.contains("function listView<T>(view: string): ViewDef<T, 'list'>"),
            "Expected listView helper function, got:\n{}",
            stack_def
        );
    }
}
