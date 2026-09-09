use crate::ast::*;
use arete_idl::utils::to_snake_case as idl_to_snake_case;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct StateViewKeyDefinition {
    field_name: String,
    typescript_type: String,
}

impl StateViewKeyDefinition {
    fn object_type(&self) -> String {
        format!(
            "{{ {}: {} }}",
            render_ts_property_name_literal(&self.field_name),
            self.typescript_type
        )
    }

    fn fields_literal(&self) -> String {
        format!("['{}']", escape_ts_single_quotes(&self.field_name))
    }
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
        self.try_compile()
            .expect("TypeScript SDK generation failed")
    }

    pub fn try_compile(&self) -> Result<TypeScriptOutput, String> {
        let state_view_key = state_view_key_definition(
            &self.entity_name,
            &self.spec.identity,
            &self.spec.field_mappings,
            &self.spec.sections,
        )?;
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
        let stack_definition = self.generate_stack_definition(&state_view_key);

        Ok(TypeScriptOutput {
            imports,
            interfaces: combined_interfaces,
            stack_definition,
            schema_names: schema_output.names,
        })
    }

    fn generate_imports(&self) -> String {
        "import { z } from 'zod';".to_string()
    }

    fn generate_view_helpers(&self) -> String {
        generate_view_helpers_static()
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

        if self.should_emit_capture_wrapper() {
            interfaces.push(self.generate_capture_wrapper_interface());
        }

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

                let ts_field = TypeScriptField::patch(
                    field_name.to_string(),
                    self.mapping_to_typescript_type(mapping),
                    self.is_field_nullable(mapping),
                );

                sections
                    .entry(section_name.to_string())
                    .or_default()
                    .push(ts_field);
            } else {
                let ts_field = TypeScriptField::patch(
                    mapping.target_path.clone(),
                    self.mapping_to_typescript_type(mapping),
                    self.is_field_nullable(mapping),
                );

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
                        let (raw_name, canonical_name) = localize_section_field_names(
                            section.name.as_str(),
                            effective_field_info,
                        );

                        section_fields.push(TypeScriptField::from_names(
                            raw_name,
                            canonical_name,
                            self.field_type_info_to_typescript(effective_field_info),
                            effective_field_info.is_optional,
                            FieldPresence::Patch,
                        ));
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
                        section_fields.push(TypeScriptField::from_names(
                            field_name.to_string(),
                            field_type_info.canonical_field_name(),
                            self.base_type_to_typescript(
                                &field_type_info.base_type,
                                field_type_info.effective_integer_kind(),
                                field_type_info.is_array,
                            ),
                            field_type_info.is_optional,
                            FieldPresence::Patch,
                        ));
                    }
                }
            }
        }
    }

    fn generate_interface_from_fields(&self, name: &str, fields: &[TypeScriptField]) -> String {
        let interface_name = self.section_interface_name(name);
        render_interface_from_ts_fields(&interface_name, fields, true)
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

        let main_fields = self.collect_main_entity_fields();
        if main_fields.is_empty() {
            return format!(
                "export interface {} {{\n  // Generated interface - extend as needed\n}}",
                entity_name
            );
        }

        render_interface_from_ts_fields(&entity_name, &main_fields, true)
    }

    fn generate_schemas(&self) -> SchemaOutput {
        let patch_schema_types = self.patch_schema_type_names();
        let mut definitions = Vec::new();
        let mut names = Vec::new();
        let mut seen = HashSet::new();

        let mut push_schema = |schema_name: String, definition: String, export_name: bool| {
            if seen.insert(schema_name.clone()) {
                if export_name {
                    names.push(schema_name);
                }
                definitions.push(definition);
            }
        };

        for (schema_name, definition) in self.generate_builtin_resolver_schemas() {
            push_schema(schema_name, definition, true);
        }

        if self.has_event_types() {
            push_schema(
                "EventWrapperSchema".to_string(),
                self.generate_event_wrapper_schema(),
                true,
            );
        }

        if self.should_emit_capture_wrapper() {
            push_schema(
                "CaptureWrapperSchema".to_string(),
                self.generate_capture_wrapper_schema(),
                false,
            );
        }

        for (schema_name, definition) in self.generate_resolved_type_schemas(&patch_schema_types) {
            push_schema(schema_name, definition, true);
        }

        for (schema_name, definition) in
            self.generate_resolved_type_patch_schemas(&patch_schema_types)
        {
            push_schema(schema_name, definition, false);
        }

        for (schema_name, definition) in self.generate_event_schemas() {
            push_schema(schema_name, definition, true);
        }

        for (schema_name, definition) in self.generate_idl_enum_schemas() {
            push_schema(schema_name, definition, true);
        }

        let all_sections = self.collect_interface_sections();

        for (section_name, fields) in &all_sections {
            if is_root_section(section_name) {
                continue;
            }
            let deduplicated_fields = self.deduplicate_fields(fields.clone());
            let interface_name = self.section_interface_name(section_name);
            let schema_definition = self.generate_schema_for_fields(
                &interface_name,
                &deduplicated_fields,
                true,
                SchemaMode::Canonical,
                &patch_schema_types,
            );
            push_schema(format!("{}Schema", interface_name), schema_definition, true);

            let patch_schema_definition = self.generate_schema_for_fields(
                &interface_name,
                &deduplicated_fields,
                false,
                SchemaMode::Patch,
                &patch_schema_types,
            );
            push_schema(
                format!("{}PatchSchema", interface_name),
                patch_schema_definition,
                false,
            );
        }

        let entity_name = to_pascal_case(&self.entity_name);
        let main_fields = self.collect_main_entity_fields();
        let entity_schema = self.generate_schema_for_fields(
            &entity_name,
            &main_fields,
            true,
            SchemaMode::Canonical,
            &patch_schema_types,
        );
        push_schema(format!("{}Schema", entity_name), entity_schema, true);

        let patch_schema = self.generate_schema_for_fields(
            &entity_name,
            &main_fields,
            false,
            SchemaMode::Patch,
            &patch_schema_types,
        );
        push_schema(format!("{}PatchSchema", entity_name), patch_schema, false);

        let completed_schema =
            self.generate_completed_entity_schema(&entity_name, &patch_schema_types);
        push_schema(
            format!("{}CompletedSchema", entity_name),
            completed_schema,
            true,
        );

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

    fn generate_capture_wrapper_schema(&self) -> String {
        r#"export const CaptureWrapperSchema = <T extends z.ZodTypeAny>(data: T) => z.object({
  timestamp: z.number(),
  account_address: z.string(),
  data,
  slot: z.number().optional(),
  signature: z.string().optional(),
}).transform((value) => ({
  timestamp: value.timestamp,
  accountAddress: value.account_address,
  data: value.data,
  ...(value.slot !== undefined ? { slot: value.slot } : {}),
  ...(value.signature !== undefined ? { signature: value.signature } : {}),
}));"#
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
                fields.push(TypeScriptField::patch(
                    section.to_string(),
                    section_interface_name,
                    false,
                ));
            }
        }

        for section in &self.spec.sections {
            if is_root_section(&section.name) {
                for field in &section.fields {
                    if !field.emit {
                        continue;
                    }
                    fields.push(TypeScriptField::from_names(
                        field.raw_field_name().to_string(),
                        field.canonical_field_name(),
                        self.field_type_info_to_typescript(field),
                        field.is_optional,
                        FieldPresence::Patch,
                    ));
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
        mode: SchemaMode,
        patch_schema_types: &HashSet<String>,
    ) -> String {
        if fields.is_empty() {
            return format!(
                "export const {} = z.object({{}});",
                schema_constant_name(name, mode)
            );
        }

        let mut field_definitions = Vec::new();
        let mut transform_fields = Vec::new();

        for field in fields {
            let base_schema = field.zod_schema.clone().unwrap_or_else(|| {
                self.typescript_type_to_zod_for_schema(&field.ts_type, mode, patch_schema_types)
            });
            let with_nullable = if field.nullable {
                format!("{}.nullable()", base_schema)
            } else {
                base_schema
            };
            let nullable_keys_can_be_absent = matches!(mode, SchemaMode::Canonical);
            let schema = if required || matches!(field.presence, FieldPresence::Required) {
                if field.nullable && nullable_keys_can_be_absent {
                    // Nullable entity fields are projected with LastWrite semantics and may be
                    // absent until a value exists. Keep them nullable and key-optional in
                    // completed schemas so partial hydration never rejects the entity.
                    format!("{}.optional()", with_nullable)
                } else {
                    with_nullable
                }
            } else {
                format!("{}.optional()", with_nullable)
            };

            field_definitions.push(format!("  {}: {},", field.raw_name, schema));
            if mode == SchemaMode::Patch {
                transform_fields.push(format!(
                    "  ...(value.{raw_name} !== undefined ? {{ {field_name}: value.{raw_name} }} : {{}}),",
                    raw_name = field.raw_name,
                    field_name = field.name,
                ));
            } else {
                transform_fields.push(format!("  {}: value.{},", field.name, field.raw_name));
            }
        }

        format!(
            "export const {} = z.object({{\n{}\n}}).transform((value) => ({{\n{}\n}}));",
            schema_constant_name(name, mode),
            field_definitions.join("\n"),
            transform_fields.join("\n")
        )
    }

    fn generate_completed_entity_schema(
        &self,
        entity_name: &str,
        patch_schema_types: &HashSet<String>,
    ) -> String {
        let main_fields = self.collect_main_entity_fields();
        self.generate_schema_for_fields(
            &format!("{}Completed", entity_name),
            &main_fields,
            true,
            SchemaMode::Canonical,
            patch_schema_types,
        )
    }

    fn generate_resolved_type_schemas(
        &self,
        patch_schema_types: &HashSet<String>,
    ) -> Vec<(String, String)> {
        let mut schemas = Vec::new();
        let mut generated_types = HashSet::new();
        let resolved_name_map = self.build_resolved_type_name_map();

        for section in &self.spec.sections {
            for field_info in &section.fields {
                if let Some(resolved) = &field_info.resolved_type {
                    let type_name =
                        self.resolved_type_to_interface_name_with_map(resolved, &resolved_name_map);

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

                    let schema = self.generate_schema_for_fields(
                        &type_name,
                        &self.resolved_fields_to_typescript_fields(&resolved.fields),
                        true,
                        SchemaMode::Canonical,
                        patch_schema_types,
                    );
                    schemas.push((format!("{}Schema", type_name), schema));
                }
            }
        }

        schemas
    }

    fn generate_resolved_type_patch_schemas(
        &self,
        patch_schema_types: &HashSet<String>,
    ) -> Vec<(String, String)> {
        let mut schemas = Vec::new();
        let mut generated_types = HashSet::new();
        let resolved_name_map = self.build_resolved_type_name_map();

        for section in &self.spec.sections {
            for field_info in &section.fields {
                if let Some(resolved) = &field_info.resolved_type {
                    let type_name =
                        self.resolved_type_to_interface_name_with_map(resolved, &resolved_name_map);

                    if !generated_types.insert(type_name.clone()) || resolved.is_enum {
                        continue;
                    }

                    let schema = self.generate_schema_for_fields(
                        &type_name,
                        &self.resolved_fields_to_typescript_fields(&resolved.fields),
                        false,
                        SchemaMode::Patch,
                        patch_schema_types,
                    );
                    schemas.push((format!("{}PatchSchema", type_name), schema));
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
                            fields.push(TypeScriptField::patch(
                                field_name.to_string(),
                                ts_type,
                                false,
                            ));
                        }
                        break;
                    }
                }
            }
        }

        Some(render_schema_from_ts_fields(interface_name, &fields, true))
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
                    let interface_name = to_pascal_case(type_name);
                    if !generated_types.insert(interface_name.clone()) {
                        continue;
                    }
                    if let Some(variants) = type_obj.get("variants").and_then(|v| v.as_array()) {
                        let variant_names: Vec<String> = variants
                            .iter()
                            .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
                            .map(|s| format!("\"{}\"", to_pascal_case(s)))
                            .collect();

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

    fn typescript_type_to_zod_for_schema(
        &self,
        ts_type: &str,
        mode: SchemaMode,
        patch_schema_types: &HashSet<String>,
    ) -> String {
        typescript_type_to_zod_for_schema_static(ts_type, mode, patch_schema_types)
    }

    fn patch_schema_type_names(&self) -> HashSet<String> {
        let mut names = HashSet::new();
        let resolved_name_map = self.build_resolved_type_name_map();

        for section in &self.spec.sections {
            if !is_root_section(&section.name) && section.fields.iter().any(|field| field.emit) {
                names.insert(self.section_interface_name(&section.name));
            }

            for field in &section.fields {
                let Some(resolved) = &field.resolved_type else {
                    continue;
                };

                if resolved.is_enum {
                    continue;
                }

                names.insert(
                    self.resolved_type_to_interface_name_with_map(resolved, &resolved_name_map),
                );
            }
        }

        names.insert("TokenMetadata".to_string());
        names
    }

    fn generate_stack_definition(&self, state_view_key: &StateViewKeyDefinition) -> String {
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
                .filter(|name| name.ends_with("Schema") && !name.ends_with("PatchSchema"))
                .map(|name| format!("    {}: {},", name.trim_end_matches("Schema"), name))
                .collect();
            if schema_entries.is_empty() {
                String::new()
            } else {
                format!("\n  schemas: {{\n{}\n  }},", schema_entries.join("\n"))
            }
        };

        let patch_schemas_block = format!(
            "\n  patchSchemas: {{\n    {entity}: {entity}PatchSchema,\n  }},",
            entity = entity_pascal
        );

        // Generate URL line - either actual URL or placeholder comment
        let url_line = match &self.config.url {
            Some(url) => format!("  url: '{}',", url),
            None => "  url: '', // TODO: Set after first deployment or pass useArete(..., { url })"
                .to_string(),
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
      state: stateView<{}, {}>('{}/state', {}),
      list: listView<{}>('{}/list'),{}
    }},
  }},{}{}
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
            state_view_key.object_type(),
            self.entity_name,
            state_view_key.fields_literal(),
            entity_pascal,
            self.entity_name,
            derived_views,
            schemas_block,
            patch_schemas_block,
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
            } else if resolved.is_account && self.is_capture_field(field_info) {
                format!("CaptureWrapper<{}>", interface_name)
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

        if let Some(ts_type) = typescript_integer_type(
            field_info.effective_integer_kind(),
            field_info
                .inner_type
                .as_deref()
                .or(Some(field_info.rust_type_name.as_str())),
        ) {
            return if field_info.is_array {
                format!("{}[]", ts_type)
            } else {
                ts_type.to_string()
            };
        }

        // Arrays of scalar non-integer primitives (e.g. Vec<f64> display
        // fields) map to the corresponding primitive array instead of any[].
        if field_info.base_type == BaseType::Array && field_info.is_array {
            if let Some(element) = field_info
                .inner_type
                .as_deref()
                .and_then(typescript_scalar_array_element)
            {
                return format!("{}[]", element);
            }
        }

        if field_info.base_type == BaseType::Any
            || (field_info.base_type == BaseType::Array
                && field_info.inner_type.as_deref() == Some("Value"))
        {
            if let Some(event_type) = self.find_event_interface_for_field(&field_info.field_name) {
                return if field_info.is_array {
                    format!("{}[]", event_type)
                } else {
                    event_type
                };
            }
        }

        self.base_type_to_typescript(
            &field_info.base_type,
            field_info.effective_integer_kind(),
            field_info.is_array,
        )
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
        self.build_resolved_type_name_map()
            .get(&resolved.type_name)
            .cloned()
            .unwrap_or_else(|| to_pascal_case(&resolved.type_name))
    }

    /// Generate nested interfaces for all resolved types in the AST
    fn generate_nested_interfaces(&self) -> Vec<String> {
        let mut interfaces = Vec::new();
        let mut generated_types = self.already_emitted_types.clone();
        let resolved_name_map = self.build_resolved_type_name_map();

        // Collect all resolved types from all sections
        for section in &self.spec.sections {
            for field_info in &section.fields {
                if let Some(resolved) = &field_info.resolved_type {
                    let type_name =
                        self.resolved_type_to_interface_name_with_map(resolved, &resolved_name_map);

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
                            let interface_name = to_pascal_case(type_name);
                            if generated_types.insert(interface_name.clone()) {
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
                            fields.push(TypeScriptField::patch(
                                field_name.to_string(),
                                ts_type,
                                false,
                            ));
                        }
                        break;
                    }
                }
            }
        }

        if !fields.is_empty() {
            return Some(render_interface_from_ts_fields(
                interface_name,
                &fields,
                true,
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
                "u64" | "u128" | "i64" | "i128" => "bigint".to_string(),
                "u8" | "u16" | "u32" | "i8" | "i16" | "i32" => "number".to_string(),
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
        let interface_name = self.resolved_type_to_interface_name(resolved);

        // Handle enums as TypeScript union types
        if resolved.is_enum {
            let variants: Vec<String> = resolved
                .enum_variants
                .iter()
                .map(|v| format!("\"{}\"", to_pascal_case(v)))
                .collect();

            return format!("export type {} = {};", interface_name, variants.join(" | "));
        }

        render_interface_from_ts_fields(
            &interface_name,
            &self.resolved_fields_to_typescript_fields(&resolved.fields),
            true,
        )
    }

    /// Convert a resolved field to TypeScript type
    fn resolved_field_to_typescript(&self, field: &ResolvedField) -> String {
        if let Some(ts_type) =
            typescript_integer_type(field.effective_integer_kind(), Some(&field.field_type))
        {
            return if field.is_array {
                format!("{}[]", ts_type)
            } else {
                ts_type.to_string()
            };
        }
        let base_ts =
            self.base_type_to_typescript(&field.base_type, field.effective_integer_kind(), false);

        if field.is_array {
            format!("{}[]", base_ts)
        } else {
            base_ts
        }
    }

    fn resolved_fields_to_typescript_fields(
        &self,
        fields: &[ResolvedField],
    ) -> Vec<TypeScriptField> {
        fields
            .iter()
            .map(|field| {
                TypeScriptField::from_names(
                    field.raw_field_name().to_string(),
                    field.canonical_field_name(),
                    self.resolved_field_to_typescript(field),
                    field.is_optional,
                    FieldPresence::Patch,
                )
            })
            .collect()
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

    fn has_capture_types(&self) -> bool {
        self.spec
            .sections
            .iter()
            .flat_map(|section| &section.fields)
            .any(|field| self.is_capture_field(field))
    }

    fn should_emit_capture_wrapper(&self) -> bool {
        self.has_capture_types() && !self.already_emitted_types.contains("CaptureWrapper")
    }

    fn is_capture_field(&self, field_info: &FieldTypeInfo) -> bool {
        let raw_name = field_info.raw_field_name();
        self.spec.handlers.iter().any(|handler| {
            handler.mappings.iter().any(|mapping| {
                matches!(&mapping.source, MappingSource::AsCapture { .. })
                    && (mapping.target_path == field_info.field_name
                        || mapping.target_path == raw_name)
            })
        })
    }

    fn build_resolved_type_name_map(&self) -> HashMap<String, String> {
        let mut reserved_names = self.already_emitted_types.clone();
        reserved_names.insert(to_pascal_case(&self.entity_name));

        for section in &self.spec.sections {
            if !is_root_section(&section.name) && section.fields.iter().any(|field| field.emit) {
                reserved_names.insert(self.section_interface_name(&section.name));
            }
        }

        let mut resolved_name_map = HashMap::new();

        for section in &self.spec.sections {
            for field_info in &section.fields {
                if !field_info.emit {
                    continue;
                }

                let Some(resolved) = &field_info.resolved_type else {
                    continue;
                };

                if resolved_name_map.contains_key(&resolved.type_name) {
                    continue;
                }

                let emitted_name = unique_resolved_type_name_ts(resolved, &mut reserved_names);
                resolved_name_map.insert(resolved.type_name.clone(), emitted_name);
            }
        }

        resolved_name_map
    }

    fn resolved_type_to_interface_name_with_map(
        &self,
        resolved: &ResolvedStructType,
        resolved_name_map: &HashMap<String, String>,
    ) -> String {
        resolved_name_map
            .get(&resolved.type_name)
            .cloned()
            .unwrap_or_else(|| to_pascal_case(&resolved.type_name))
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

    fn generate_capture_wrapper_interface(&self) -> String {
        r#"/**
 * Wrapper for account data captured with context metadata.
 */
export interface CaptureWrapper<T> {
  /** Unix timestamp when the account was captured */
  timestamp: number;
  /** Base58 account address */
  accountAddress: string;
  /** Captured account data */
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

    fn is_field_nullable(&self, mapping: &TypedFieldMapping<S>) -> bool {
        // Stream mappings produce patch-shaped objects, so this bool only captures
        // whether the field can be explicitly null in the payload.
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
    fn base_type_to_typescript(
        &self,
        base_type: &BaseType,
        integer_kind: Option<IntegerKind>,
        is_array: bool,
    ) -> String {
        let base_ts_type = match base_type {
            BaseType::Integer => integer_kind
                .map(integer_kind_to_typescript)
                .unwrap_or("number"),
            BaseType::Float => "number",
            BaseType::String => "string",
            BaseType::Boolean => "boolean",
            BaseType::Timestamp => integer_kind
                .map(integer_kind_to_typescript)
                .unwrap_or("number"),
            BaseType::Binary => "string", // Base64 encoded strings
            BaseType::Pubkey => "string", // Solana public keys as Base58 strings
            BaseType::Array => "any[]",   // Default array type
            BaseType::Object => "Record<string, any>", // Generic object
            BaseType::Any => "any",
        };

        if is_array && !matches!(base_type, BaseType::Array) {
            format!("{}[]", base_ts_type)
        } else {
            base_ts_type.to_string()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldPresence {
    Patch,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemaMode {
    Canonical,
    Patch,
}

fn schema_constant_name(name: &str, mode: SchemaMode) -> String {
    match mode {
        SchemaMode::Canonical => format!("{}Schema", name),
        SchemaMode::Patch => format!("{}PatchSchema", name),
    }
}

fn localize_section_field_names(
    section_name: &str,
    field_info: &FieldTypeInfo,
) -> (String, String) {
    let raw_name = field_info.raw_field_name();
    if is_root_section(section_name) {
        return (raw_name.to_string(), field_info.canonical_field_name());
    }

    let prefix = format!("{}.", section_name);
    if let Some(local_raw_name) = raw_name.strip_prefix(&prefix) {
        return (local_raw_name.to_string(), to_camel_case(local_raw_name));
    }

    (raw_name.to_string(), field_info.canonical_field_name())
}

/// Represents a TypeScript field in an interface
#[derive(Debug, Clone)]
struct TypeScriptField {
    name: String,
    raw_name: String,
    ts_type: String,
    nullable: bool,
    presence: FieldPresence,
    zod_schema: Option<String>,
    #[allow(dead_code)]
    description: Option<String>,
}

impl TypeScriptField {
    fn patch(raw_name: String, ts_type: String, nullable: bool) -> Self {
        Self::from_names(
            raw_name.clone(),
            to_camel_case(&raw_name),
            ts_type,
            nullable,
            FieldPresence::Patch,
        )
    }

    fn required_with_schema(
        raw_name: String,
        canonical_name: String,
        ts_type: String,
        nullable: bool,
        zod_schema: String,
    ) -> Self {
        let mut field = Self::from_names(
            raw_name,
            canonical_name,
            ts_type,
            nullable,
            FieldPresence::Required,
        );
        field.zod_schema = Some(zod_schema);
        field
    }

    fn from_names(
        raw_name: String,
        canonical_name: String,
        ts_type: String,
        nullable: bool,
        presence: FieldPresence,
    ) -> Self {
        Self {
            name: canonical_name,
            raw_name,
            ts_type,
            nullable,
            presence,
            zod_schema: None,
            description: None,
        }
    }

    fn rendered_ts_type(&self) -> String {
        if self.nullable {
            format!("{} | null", self.ts_type)
        } else {
            self.ts_type.clone()
        }
    }
}

#[derive(Debug, Clone)]
struct SchemaOutput {
    definitions: String,
    names: Vec<String>,
}

#[derive(Debug, Default)]
struct IdlAccountArtifacts {
    code: String,
    schema_names: Vec<String>,
    type_names: HashSet<String>,
    account_type_names: BTreeMap<(String, String), String>,
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

fn generate_idl_account_artifacts(
    idls: &[IdlSnapshot],
    reserved_type_names: &HashSet<String>,
) -> IdlAccountArtifacts {
    let mut used_type_names = reserved_type_names.clone();
    let mut emitted_type_names = HashSet::new();
    let mut seen_schema_names = HashSet::new();
    let mut interface_blocks = Vec::new();
    let mut schema_blocks = Vec::new();
    let mut schema_names = Vec::new();
    let mut account_type_names = BTreeMap::new();

    for idl in idls {
        let program_key = to_camel_case(&idl.name);
        let program_prefix = to_pascal_case(&idl.name);
        let type_defs: BTreeMap<String, &IdlTypeDefSnapshot> = idl
            .types
            .iter()
            .map(|type_def| (type_def.name.clone(), type_def))
            .collect();
        let account_names: HashSet<String> = idl
            .accounts
            .iter()
            .map(|account| account.name.clone())
            .collect();
        let mut local_name_map = BTreeMap::new();

        for account in &idl.accounts {
            let unique_name =
                unique_idl_type_name(&account.name, &program_prefix, &mut used_type_names);
            emitted_type_names.insert(unique_name.clone());
            local_name_map.insert(account.name.clone(), unique_name.clone());
            account_type_names.insert((program_key.clone(), account.name.clone()), unique_name);
        }

        let mut required_defined_types = BTreeSet::new();
        for account in &idl.accounts {
            for field in resolve_idl_account_fields(account, &type_defs) {
                collect_required_defined_types(
                    &field.type_,
                    &type_defs,
                    &account_names,
                    &mut required_defined_types,
                );
            }
        }

        for type_name in &required_defined_types {
            if local_name_map.contains_key(type_name) {
                continue;
            }
            let unique_name =
                unique_idl_type_name(type_name, &program_prefix, &mut used_type_names);
            emitted_type_names.insert(unique_name.clone());
            local_name_map.insert(type_name.clone(), unique_name);
        }

        for type_name in &required_defined_types {
            if account_names.contains(type_name) {
                continue;
            }
            if let Some(type_def) = type_defs.get(type_name) {
                if let Some((interface_def, schema_name, schema_def)) =
                    generate_type_defs_from_idl_type(type_def, &local_name_map)
                {
                    interface_blocks.push(interface_def);
                    if seen_schema_names.insert(schema_name.clone()) {
                        schema_names.push(schema_name.clone());
                        schema_blocks.push(schema_def);
                    }
                }
            }
        }

        for account in &idl.accounts {
            let Some(type_name) = local_name_map.get(&account.name) else {
                continue;
            };
            let account_fields = resolve_idl_account_fields(account, &type_defs);
            interface_blocks.push(generate_interface_from_idl_fields(
                type_name,
                account_fields,
                &local_name_map,
            ));
            let schema_name = format!("{}Schema", type_name);
            if seen_schema_names.insert(schema_name.clone()) {
                schema_names.push(schema_name.clone());
                schema_blocks.push(generate_schema_from_idl_fields(
                    type_name,
                    account_fields,
                    &local_name_map,
                ));
            }
        }
    }

    let code = if interface_blocks.is_empty() && schema_blocks.is_empty() {
        String::new()
    } else if schema_blocks.is_empty() {
        interface_blocks.join("\n\n")
    } else if interface_blocks.is_empty() {
        schema_blocks.join("\n\n")
    } else {
        format!(
            "{}\n\n{}",
            interface_blocks.join("\n\n"),
            schema_blocks.join("\n\n")
        )
    };

    IdlAccountArtifacts {
        code,
        schema_names,
        type_names: emitted_type_names,
        account_type_names,
    }
}

fn resolve_idl_account_fields<'a>(
    account: &'a IdlAccountSnapshot,
    type_defs: &'a BTreeMap<String, &'a IdlTypeDefSnapshot>,
) -> &'a [IdlFieldSnapshot] {
    if !account.fields.is_empty() {
        return &account.fields;
    }

    let Some(type_def) = type_defs.get(&account.name) else {
        return &account.fields;
    };

    match &type_def.type_def {
        IdlTypeDefKindSnapshot::Struct { fields, .. } => fields,
        _ => &account.fields,
    }
}

fn unique_idl_type_name(
    raw_name: &str,
    program_prefix: &str,
    used_type_names: &mut HashSet<String>,
) -> String {
    let base_name = to_pascal_case(raw_name);
    if used_type_names.insert(base_name.clone()) {
        return base_name;
    }

    let prefixed = format!("{}{}", program_prefix, base_name);
    if used_type_names.insert(prefixed.clone()) {
        return prefixed;
    }

    let mut index = 2;
    loop {
        let candidate = format!("{}{}", prefixed, index);
        if used_type_names.insert(candidate.clone()) {
            return candidate;
        }
        index += 1;
    }
}

fn collect_required_defined_types(
    idl_type: &IdlTypeSnapshot,
    type_defs: &BTreeMap<String, &IdlTypeDefSnapshot>,
    account_names: &HashSet<String>,
    output: &mut BTreeSet<String>,
) {
    match idl_type {
        IdlTypeSnapshot::Simple(_) => {}
        IdlTypeSnapshot::Array(array_type) => {
            for element in &array_type.array {
                if let IdlArrayElementSnapshot::Type(inner) = element {
                    collect_required_defined_types(inner, type_defs, account_names, output);
                }
            }
        }
        IdlTypeSnapshot::Option(option_type) => {
            collect_required_defined_types(&option_type.option, type_defs, account_names, output);
        }
        IdlTypeSnapshot::Vec(vec_type) => {
            collect_required_defined_types(&vec_type.vec, type_defs, account_names, output);
        }
        IdlTypeSnapshot::HashMap(hash_map_type) => {
            collect_required_defined_types(
                &hash_map_type.hash_map.0,
                type_defs,
                account_names,
                output,
            );
            collect_required_defined_types(
                &hash_map_type.hash_map.1,
                type_defs,
                account_names,
                output,
            );
        }
        IdlTypeSnapshot::Tuple(tuple_type) => {
            for element in &tuple_type.tuple {
                collect_required_defined_types(element, type_defs, account_names, output);
            }
        }
        IdlTypeSnapshot::Defined(defined_type) => {
            let type_name = match &defined_type.defined {
                IdlDefinedInnerSnapshot::Named { name } => name,
                IdlDefinedInnerSnapshot::Simple(name) => name,
            };

            if !output.insert(type_name.clone()) {
                return;
            }

            if account_names.contains(type_name) {
                return;
            }

            let Some(type_def) = type_defs.get(type_name) else {
                return;
            };

            match &type_def.type_def {
                IdlTypeDefKindSnapshot::Struct { fields, .. } => {
                    for field in fields {
                        collect_required_defined_types(
                            &field.type_,
                            type_defs,
                            account_names,
                            output,
                        );
                    }
                }
                IdlTypeDefKindSnapshot::TupleStruct { fields, .. } => {
                    for field in fields {
                        collect_required_defined_types(field, type_defs, account_names, output);
                    }
                }
                IdlTypeDefKindSnapshot::Enum { variants, .. } => {
                    for variant in variants {
                        for field in &variant.fields {
                            match field {
                                IdlEnumVariantFieldSnapshot::Named(named) => {
                                    collect_required_defined_types(
                                        &named.type_,
                                        type_defs,
                                        account_names,
                                        output,
                                    );
                                }
                                IdlEnumVariantFieldSnapshot::Tuple(tuple) => {
                                    collect_required_defined_types(
                                        tuple,
                                        type_defs,
                                        account_names,
                                        output,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn generate_type_defs_from_idl_type(
    type_def: &IdlTypeDefSnapshot,
    local_name_map: &BTreeMap<String, String>,
) -> Option<(String, String, String)> {
    let type_name = local_name_map
        .get(&type_def.name)
        .cloned()
        .unwrap_or_else(|| to_pascal_case(&type_def.name));
    let schema_name = format!("{}Schema", type_name);

    match &type_def.type_def {
        IdlTypeDefKindSnapshot::Struct { fields, .. } => Some((
            generate_interface_from_idl_fields(&type_name, fields, local_name_map),
            schema_name,
            generate_schema_from_idl_fields(&type_name, fields, local_name_map),
        )),
        IdlTypeDefKindSnapshot::TupleStruct { fields, .. } => {
            let interface = format!(
                "export type {} = [{}];",
                type_name,
                fields
                    .iter()
                    .map(|field| idl_snapshot_type_to_typescript(field, local_name_map))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let schema = format!(
                "export const {} = z.tuple([{}]);",
                schema_name,
                fields
                    .iter()
                    .map(|field| idl_snapshot_type_to_zod(field, local_name_map))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            Some((interface, schema_name, schema))
        }
        IdlTypeDefKindSnapshot::Enum { variants, .. } => {
            let variant_names = variants
                .iter()
                .map(|variant| format!("\"{}\"", variant.name))
                .collect::<Vec<_>>();
            let interface = if variant_names.is_empty() {
                format!("export type {} = string;", type_name)
            } else {
                format!("export type {} = {};", type_name, variant_names.join(" | "))
            };
            let schema = if variant_names.is_empty() {
                format!("export const {} = z.string();", schema_name)
            } else {
                format!(
                    "export const {} = z.enum([{}]);",
                    schema_name,
                    variant_names.join(", ")
                )
            };
            Some((interface, schema_name, schema))
        }
    }
}

fn generate_interface_from_idl_fields(
    name: &str,
    fields: &[IdlFieldSnapshot],
    local_name_map: &BTreeMap<String, String>,
) -> String {
    render_interface_from_ts_fields(name, &normalize_idl_fields(fields, local_name_map), true)
}

fn generate_schema_from_idl_fields(
    name: &str,
    fields: &[IdlFieldSnapshot],
    local_name_map: &BTreeMap<String, String>,
) -> String {
    render_schema_from_ts_fields(name, &normalize_idl_fields(fields, local_name_map), true)
}

fn normalize_idl_fields(
    fields: &[IdlFieldSnapshot],
    local_name_map: &BTreeMap<String, String>,
) -> Vec<TypeScriptField> {
    let canonical_names = canonical_idl_field_names(fields);
    let mut normalized = Vec::with_capacity(fields.len());

    for (field, canonical_name) in fields.iter().zip(canonical_names) {
        let (normalized_type, nullable) = strip_nullable_idl_type(&field.type_);

        normalized.push(TypeScriptField::required_with_schema(
            idl_field_wire_name(&field.name),
            canonical_name,
            idl_snapshot_type_to_typescript(normalized_type, local_name_map),
            nullable,
            idl_snapshot_type_to_zod(normalized_type, local_name_map),
        ));
    }

    normalized
}

/// Produce stable, distinct TypeScript property names for IDL fields.
///
/// Most fields use the existing camel-case projection. When distinct wire
/// names collapse to the same projection, preserve leading underscores first
/// (`padding_0` -> `padding0`, `_padding_0` -> `_padding0`) and fall back to a
/// numeric suffix for less structured collisions. This keeps account codegen
/// total for valid binary layouts instead of panicking during generation.
fn canonical_idl_field_names(fields: &[IdlFieldSnapshot]) -> Vec<String> {
    let base_names = fields
        .iter()
        .map(|field| to_camel_case(&field.name))
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
            let leading_underscores = field.name.chars().take_while(|ch| *ch == '_').count();
            let preferred = if base_counts.get(base_name.as_str()).copied().unwrap_or(0) > 1
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
                let candidate = format!("{}{}", preferred, suffix);
                if used_names.insert(candidate.clone()) {
                    return candidate;
                }
                suffix += 1;
            }
        })
        .collect()
}

fn idl_field_wire_name(field_name: &str) -> String {
    idl_to_snake_case(field_name)
}

fn idl_snapshot_type_to_typescript(
    idl_type: &IdlTypeSnapshot,
    local_name_map: &BTreeMap<String, String>,
) -> String {
    match idl_type {
        IdlTypeSnapshot::Simple(type_name) => match type_name.as_str() {
            "u64" | "u128" | "i64" | "i128" => "bigint".to_string(),
            "u8" | "u16" | "u32" | "i8" | "i16" | "i32" | "f32" | "f64" => "number".to_string(),
            "bool" => "boolean".to_string(),
            "string" => "string".to_string(),
            "pubkey" | "publicKey" => "string".to_string(),
            "bytes" => "number[]".to_string(),
            _ => "any".to_string(),
        },
        IdlTypeSnapshot::Array(array_type) => {
            let (inner_ts, size) = match array_type.array.as_slice() {
                [IdlArrayElementSnapshot::Type(inner), IdlArrayElementSnapshot::Size(size)] => (
                    Some(idl_snapshot_type_to_typescript(inner, local_name_map)),
                    Some(*size),
                ),
                [IdlArrayElementSnapshot::TypeName(inner), IdlArrayElementSnapshot::Size(size)] => {
                    (
                        Some(idl_snapshot_type_to_typescript(
                            &IdlTypeSnapshot::Simple(inner.clone()),
                            local_name_map,
                        )),
                        Some(*size),
                    )
                }
                _ => (None, None),
            };
            let inner_ts = inner_ts.unwrap_or_else(|| "any".to_string());
            if let Some(size) = size {
                format!(
                    "{}[]",
                    if size == 0 {
                        "never".to_string()
                    } else {
                        inner_ts
                    }
                )
            } else {
                format!("{}[]", inner_ts)
            }
        }
        IdlTypeSnapshot::Option(option_type) => {
            format!(
                "{} | null",
                idl_snapshot_type_to_typescript(&option_type.option, local_name_map)
            )
        }
        IdlTypeSnapshot::Vec(vec_type) => {
            format!(
                "{}[]",
                idl_snapshot_type_to_typescript(&vec_type.vec, local_name_map)
            )
        }
        IdlTypeSnapshot::HashMap(hash_map_type) => {
            format!(
                "Record<string, {}>",
                idl_snapshot_type_to_typescript(&hash_map_type.hash_map.1, local_name_map)
            )
        }
        IdlTypeSnapshot::Tuple(tuple_type) => {
            format!(
                "[{}]",
                tuple_type
                    .tuple
                    .iter()
                    .map(|element| idl_snapshot_type_to_typescript(element, local_name_map))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        IdlTypeSnapshot::Defined(defined_type) => {
            let type_name = match &defined_type.defined {
                IdlDefinedInnerSnapshot::Named { name } => name,
                IdlDefinedInnerSnapshot::Simple(name) => name,
            };
            local_name_map
                .get(type_name)
                .cloned()
                .unwrap_or_else(|| to_pascal_case(type_name))
        }
    }
}

fn idl_snapshot_type_to_zod(
    idl_type: &IdlTypeSnapshot,
    local_name_map: &BTreeMap<String, String>,
) -> String {
    match idl_type {
        IdlTypeSnapshot::Simple(type_name) => match type_name.as_str() {
            "u64" | "u128" | "i64" | "i128" => bigint_zod(),
            "u8" | "u16" | "u32" | "i8" | "i16" | "i32" | "f32" | "f64" => "z.number()".to_string(),
            "bool" => "z.boolean()".to_string(),
            "string" => "z.string()".to_string(),
            "pubkey" | "publicKey" => "z.string()".to_string(),
            "bytes" => "z.array(z.number())".to_string(),
            _ => "z.any()".to_string(),
        },
        IdlTypeSnapshot::Array(array_type) => match array_type.array.as_slice() {
            [IdlArrayElementSnapshot::Type(inner), IdlArrayElementSnapshot::Size(size)] => {
                format!(
                    "z.array({}).length({})",
                    idl_snapshot_type_to_zod(inner, local_name_map),
                    size
                )
            }
            [IdlArrayElementSnapshot::TypeName(inner), IdlArrayElementSnapshot::Size(size)] => {
                format!(
                    "z.array({}).length({})",
                    idl_snapshot_type_to_zod(
                        &IdlTypeSnapshot::Simple(inner.clone()),
                        local_name_map
                    ),
                    size
                )
            }
            _ => "z.array(z.any())".to_string(),
        },
        IdlTypeSnapshot::Option(option_type) => {
            format!(
                "{}.nullable()",
                idl_snapshot_type_to_zod(&option_type.option, local_name_map)
            )
        }
        IdlTypeSnapshot::Vec(vec_type) => {
            format!(
                "z.array({})",
                idl_snapshot_type_to_zod(&vec_type.vec, local_name_map)
            )
        }
        IdlTypeSnapshot::HashMap(hash_map_type) => {
            format!(
                "z.record({})",
                idl_snapshot_type_to_zod(&hash_map_type.hash_map.1, local_name_map)
            )
        }
        IdlTypeSnapshot::Tuple(tuple_type) => {
            format!(
                "z.tuple([{}])",
                tuple_type
                    .tuple
                    .iter()
                    .map(|element| idl_snapshot_type_to_zod(element, local_name_map))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        IdlTypeSnapshot::Defined(defined_type) => {
            let type_name = match &defined_type.defined {
                IdlDefinedInnerSnapshot::Named { name } => name,
                IdlDefinedInnerSnapshot::Simple(name) => name,
            };
            let resolved_name = local_name_map
                .get(type_name)
                .cloned()
                .unwrap_or_else(|| to_pascal_case(type_name));
            format!("z.lazy(() => {}Schema)", resolved_name)
        }
    }
}

fn typescript_integer_type_from_rust(rust_type: &str) -> Option<&'static str> {
    IntegerKind::from_rust_type(rust_type).map(integer_kind_to_typescript)
}

fn typescript_integer_type(
    integer_kind: Option<IntegerKind>,
    rust_type: Option<&str>,
) -> Option<&'static str> {
    integer_kind
        .map(integer_kind_to_typescript)
        .or_else(|| rust_type.and_then(typescript_integer_type_from_rust))
}

/// Map the element of a `Vec<T>` scalar array to its TypeScript primitive.
/// Accepts both stored forms of the inner type (`"Vec < f64 >"` and the bare
/// `"f64"`), returning `None` for non-scalar or non-array elements.
fn typescript_scalar_array_element(inner_type: &str) -> Option<&'static str> {
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
        "f32" | "f64" => Some("number"),
        "bool" => Some("boolean"),
        "String" | "&str" | "str" => Some("string"),
        _ => None,
    }
}

fn integer_kind_to_typescript(integer_kind: IntegerKind) -> &'static str {
    if integer_kind.is_bigint() {
        "bigint"
    } else {
        "number"
    }
}

fn strip_nullable_idl_type(mut idl_type: &IdlTypeSnapshot) -> (&IdlTypeSnapshot, bool) {
    let mut nullable = false;
    while let IdlTypeSnapshot::Option(option_type) = idl_type {
        nullable = true;
        idl_type = &option_type.option;
    }
    (idl_type, nullable)
}

fn typescript_type_to_zod_static(ts_type: &str) -> String {
    let trimmed = ts_type.trim();

    if let Some(inner) = trimmed.strip_suffix("[]") {
        return format!("z.array({})", typescript_type_to_zod_static(inner));
    }

    if let Some(inner) = trimmed.strip_prefix("EventWrapper<") {
        if let Some(inner) = inner.strip_suffix('>') {
            return format!(
                "EventWrapperSchema({})",
                typescript_type_to_zod_static(inner)
            );
        }
    }

    if let Some(inner) = trimmed.strip_prefix("CaptureWrapper<") {
        if let Some(inner) = inner.strip_suffix('>') {
            return format!(
                "CaptureWrapperSchema({})",
                typescript_type_to_zod_static(inner)
            );
        }
    }

    match trimmed {
        "string" => "z.string()".to_string(),
        "number" => "z.number()".to_string(),
        "bigint" => bigint_zod(),
        "boolean" => "z.boolean()".to_string(),
        "any" => "z.any()".to_string(),
        "Record<string, any>" => "z.record(z.any())".to_string(),
        _ => format!("{}Schema", trimmed),
    }
}

fn typescript_type_to_zod_for_schema_static(
    ts_type: &str,
    mode: SchemaMode,
    patch_schema_types: &HashSet<String>,
) -> String {
    let trimmed = ts_type.trim();

    if let Some(inner) = trimmed.strip_suffix("[]") {
        return format!(
            "z.array({})",
            typescript_type_to_zod_for_schema_static(inner, mode, patch_schema_types)
        );
    }

    if let Some(inner) = trimmed.strip_prefix("EventWrapper<") {
        if let Some(inner) = inner.strip_suffix('>') {
            return format!(
                "EventWrapperSchema({})",
                typescript_type_to_zod_for_schema_static(inner, mode, patch_schema_types)
            );
        }
    }

    if let Some(inner) = trimmed.strip_prefix("CaptureWrapper<") {
        if let Some(inner) = inner.strip_suffix('>') {
            return format!(
                "CaptureWrapperSchema({})",
                typescript_type_to_zod_for_schema_static(
                    inner,
                    SchemaMode::Canonical,
                    patch_schema_types,
                )
            );
        }
    }

    match trimmed {
        "string" => "z.string()".to_string(),
        "number" => "z.number()".to_string(),
        "bigint" => bigint_zod(),
        "boolean" => "z.boolean()".to_string(),
        "any" => "z.any()".to_string(),
        "Record<string, any>" => "z.record(z.any())".to_string(),
        _ => {
            if mode == SchemaMode::Patch && patch_schema_types.contains(trimmed) {
                format!("{}PatchSchema", trimmed)
            } else {
                format!("{}Schema", trimmed)
            }
        }
    }
}

fn render_interface_from_ts_fields(
    name: &str,
    fields: &[TypeScriptField],
    force_required: bool,
) -> String {
    if fields.is_empty() {
        return format!("export interface {} {{\n}}", name);
    }

    let field_definitions = fields
        .iter()
        .map(|field| {
            let optional = if force_required || matches!(field.presence, FieldPresence::Required) {
                ""
            } else {
                "?"
            };
            format!(
                "  {}{}: {};",
                field.name,
                optional,
                field.rendered_ts_type()
            )
        })
        .collect::<Vec<_>>();

    format!(
        "export interface {} {{\n{}\n}}",
        name,
        field_definitions.join("\n")
    )
}

fn render_schema_from_ts_fields(
    name: &str,
    fields: &[TypeScriptField],
    force_required: bool,
) -> String {
    if fields.is_empty() {
        return format!("export const {}Schema = z.object({{}});", name);
    }

    let field_definitions = fields
        .iter()
        .map(|field| {
            let base_schema = field
                .zod_schema
                .clone()
                .unwrap_or_else(|| typescript_type_to_zod_static(&field.ts_type));
            let with_nullable = if field.nullable {
                format!("{}.nullable()", base_schema)
            } else {
                base_schema
            };
            let schema = if force_required || matches!(field.presence, FieldPresence::Required) {
                with_nullable
            } else {
                format!("{}.optional()", with_nullable)
            };
            format!("  {}: {},", field.raw_name, schema)
        })
        .collect::<Vec<_>>();

    let transform_fields = fields
        .iter()
        .map(|field| format!("  {}: value.{},", field.name, field.raw_name))
        .collect::<Vec<_>>();

    format!(
        "export const {}Schema = z.object({{\n{}\n}}).transform((value) => ({{\n{}\n}}));",
        name,
        field_definitions.join("\n"),
        transform_fields.join("\n")
    )
}

fn bigint_zod() -> String {
    "z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value))"
        .to_string()
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
                    names.insert(to_pascal_case(type_name));
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

fn unique_resolved_type_name_ts(
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

/// Convert snake_case to PascalCase
pub(crate) fn to_pascal_case(s: &str) -> String {
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

fn to_camel_case(s: &str) -> String {
    let pascal = to_pascal_case(s);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => pascal,
    }
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

fn state_view_key_definition(
    entity_name: &str,
    identity: &IdentitySpec,
    field_mappings: &BTreeMap<String, FieldTypeInfo>,
    sections: &[EntitySection],
) -> Result<StateViewKeyDefinition, String> {
    let mut seen = HashSet::new();
    let distinct_keys: Vec<&str> = identity
        .primary_keys
        .iter()
        .map(String::as_str)
        .filter(|key| seen.insert(*key))
        .collect();

    if distinct_keys.len() > 1 {
        return Err(format!(
            "TypeScript SDK generation does not support composite state keys for entity '{}': distinct identity.primary_keys are [{}]",
            entity_name,
            distinct_keys.join(", ")
        ));
    }

    let key_path = distinct_keys.first().copied().ok_or_else(|| {
        format!(
            "TypeScript SDK generation requires a primary key for entity '{}'",
            entity_name
        )
    })?;
    let key_leaf = key_path.rsplit('.').next().unwrap_or(key_path);
    let field_info = field_mappings.get(key_path).or_else(|| {
        sections.iter().find_map(|section| {
            section.fields.iter().find(|field| {
                field.raw_field_name() == key_path
                    || field.raw_field_name() == key_leaf
                    || field.field_name == key_path
                    || field.field_name == key_leaf
            })
        })
    });

    let field_name = field_info
        .map(FieldTypeInfo::canonical_field_name)
        .unwrap_or_else(|| to_camel_case(key_leaf));
    let typescript_type = match field_info {
        Some(field) if field.is_array => {
            return Err(format!(
                "TypeScript SDK generation does not support array state key '{}' for entity '{}'",
                key_path, entity_name
            ));
        }
        Some(field) => match field.base_type {
            BaseType::String | BaseType::Binary | BaseType::Pubkey => "string".to_string(),
            BaseType::Integer | BaseType::Timestamp => field
                .effective_integer_kind()
                .map(integer_kind_to_typescript)
                .unwrap_or("number")
                .to_string(),
            _ => {
                return Err(format!(
                    "TypeScript SDK generation does not support state key '{}' with type '{}' for entity '{}'",
                    key_path, field.rust_type_name, entity_name
                ));
            }
        },
        // Legacy ASTs may omit field metadata; retain their existing string wire key.
        None => "string".to_string(),
    };

    Ok(StateViewKeyDefinition {
        field_name,
        typescript_type,
    })
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

    compiler.try_compile()
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

    compiler.try_compile()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeScriptProgramDefinitionMetadata {
    pub program_id: String,
    /// Output metadata. Generation recomputes this from emitted content; input values are ignored.
    pub sdk_definition_hash: Option<String>,
    pub program_spec_hash: String,
    pub idl_content_hash: String,
    pub normalized_idl_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeScriptProgramReleaseReference {
    pub program_release_hash: String,
    pub program_spec_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeScriptProgramReadBinding {
    pub endpoint: String,
    pub program_read_binding_id: String,
    pub auth: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeScriptProgramReadTransport {
    LocalHttp,
    HostedBinding(TypeScriptProgramReadBinding),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeScriptProgramConfig {
    pub definition: TypeScriptProgramDefinitionMetadata,
    pub release: TypeScriptProgramReleaseReference,
    pub transport: TypeScriptProgramReadTransport,
    /// Managed-hosting transports carried by a standalone program cartridge.
    pub gateway: Option<serde_json::Value>,
}

impl From<&arete_hash::OssProgramIdentityV1> for TypeScriptProgramConfig {
    fn from(identity: &arete_hash::OssProgramIdentityV1) -> Self {
        Self {
            definition: TypeScriptProgramDefinitionMetadata {
                program_id: identity.program_spec.program_id.clone(),
                sdk_definition_hash: None,
                program_spec_hash: identity.program_spec_hash.to_string(),
                idl_content_hash: identity.program_spec.idl_content_hash.to_string(),
                normalized_idl_hash: identity.program_spec.normalized_idl_hash.to_string(),
            },
            release: TypeScriptProgramReleaseReference {
                program_release_hash: identity.release_hash.to_string(),
                program_spec_hash: identity.program_spec_hash.to_string(),
            },
            transport: TypeScriptProgramReadTransport::LocalHttp,
            gateway: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TypeScriptStackConfig {
    pub package_name: String,
    pub generate_helpers: bool,
    pub export_const_name: String,
    pub websocket_url: Option<String>,
    pub http_url: Option<String>,
    pub extension_import: Option<String>,
    /// Hosted metadata in exact AST program order. Local generation derives this from
    /// `SerializableStackSpec::program_specs` instead.
    pub programs: Option<Vec<TypeScriptProgramConfig>>,
    /// Managed-hosting transports. Local generation leaves this unset.
    pub gateway: Option<serde_json::Value>,
}

impl Default for TypeScriptStackConfig {
    fn default() -> Self {
        Self {
            package_name: "@usearete/react".to_string(),
            generate_helpers: true,
            export_const_name: "STACK".to_string(),
            websocket_url: None,
            http_url: None,
            extension_import: None,
            programs: None,
            gateway: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TypeScriptStackOutput {
    pub interfaces: String,
    pub stack_definition: String,
    pub imports: String,
    /// Non-fatal codegen warnings (skipped instructions, PDAs degraded to
    /// user-provided accounts). Callers should surface these to the user.
    pub warnings: Vec<String>,
    /// Structured PDA degradations for summary reporting.
    pub pda_degradations: Vec<crate::typescript_instructions::PdaDegradation>,
}

#[derive(Debug, Clone, Default)]
pub struct TypeScriptLiveEndpoints {
    pub websocket_url: Option<String>,
    pub http_url: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TypeScriptCompositionConfig {
    pub stack: TypeScriptStackConfig,
    pub live_endpoints: BTreeMap<String, TypeScriptLiveEndpoints>,
    pub live_module_imports: BTreeMap<String, String>,
    pub program_module_imports: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct TypeScriptAliasedStackOutput {
    pub alias: String,
    pub module_name: String,
    pub output: TypeScriptStackOutput,
}

#[derive(Debug, Clone)]
pub struct TypeScriptProgramCollectionOutput {
    pub module_name: String,
    pub output: TypeScriptStackOutput,
    pub members: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct TypeScriptCompositionOutput {
    pub name: String,
    pub live_stacks: Vec<TypeScriptAliasedStackOutput>,
    pub program_collection: Option<TypeScriptProgramCollectionOutput>,
    pub session_definition: String,
    pub warnings: Vec<String>,
    pub pda_degradations: Vec<crate::typescript_instructions::PdaDegradation>,
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

fn resolve_program_configs(
    stack_spec: &SerializableStackSpec,
    configured: Option<&[TypeScriptProgramConfig]>,
    allow_view_only: bool,
) -> Result<Vec<TypeScriptProgramConfig>, String> {
    if stack_spec.idls.is_empty() {
        if stack_spec.program_specs.is_empty()
            && configured.is_none_or(|programs| programs.is_empty())
        {
            return Ok(Vec::new());
        }
        return Err(format!(
            "Stack '{}' has program metadata but no ordered IDL list",
            stack_spec.stack_name
        ));
    }

    if stack_spec.program_specs.is_empty() {
        let view_only = stack_spec.program_ids.is_empty()
            && stack_spec.instructions.is_empty()
            && stack_spec.pdas.is_empty()
            && stack_spec
                .idls
                .iter()
                .all(|idl| idl.accounts.is_empty() && idl.instructions.is_empty());
        if allow_view_only && view_only && configured.is_none_or(|programs| programs.is_empty()) {
            return Ok(Vec::new());
        }
        return Err(format!(
            "Stack '{}' uses program SDK/account definitions but its compiled input has no exact public ProgramSpecV1 values. Rebuild the ProgramSpec and StackManifest artifact closure with the current arete-macros before generating an SDK.",
            stack_spec.stack_name
        ));
    }

    let expected = stack_spec.idls.len();
    if stack_spec.program_ids.len() != expected || stack_spec.program_specs.len() != expected {
        return Err(format!(
            "Stack '{}' program metadata is not aligned: program_ids={}, idls={}, program_specs={}. Rebuild the ProgramSpec and StackManifest artifact closure with the current arete-macros.",
            stack_spec.stack_name,
            stack_spec.program_ids.len(),
            stack_spec.idls.len(),
            stack_spec.program_specs.len(),
        ));
    }

    let mut exact_definitions = Vec::with_capacity(expected);
    let mut program_keys = BTreeSet::new();
    for (index, ((program_id, idl), program_spec)) in stack_spec
        .program_ids
        .iter()
        .zip(&stack_spec.idls)
        .zip(&stack_spec.program_specs)
        .enumerate()
    {
        program_spec.validate().map_err(|error| {
            format!(
                "Stack '{}' program_specs[{index}] is invalid: {error}",
                stack_spec.stack_name
            )
        })?;
        if program_id != &program_spec.program_id {
            return Err(format!(
                "Stack '{}' program ID mismatch at index {index}: program_ids has '{}', ProgramSpecV1 has '{}'",
                stack_spec.stack_name, program_id, program_spec.program_id
            ));
        }
        if idl.program_id.as_deref() != Some(program_spec.program_id.as_str()) {
            return Err(format!(
                "Stack '{}' IDL program ID mismatch at index {index}: expected '{}'",
                stack_spec.stack_name, program_spec.program_id
            ));
        }
        if idl.name != program_spec.idl_snapshot.snapshot.name {
            return Err(format!(
                "Stack '{}' IDL/ProgramSpec name mismatch at index {index}: '{}' != '{}'",
                stack_spec.stack_name, idl.name, program_spec.idl_snapshot.snapshot.name
            ));
        }
        let program_key = to_camel_case(&idl.name);
        if !program_keys.insert(program_key.clone()) {
            return Err(format!(
                "Stack '{}' has an ambiguous duplicate generated program key '{}'",
                stack_spec.stack_name, program_key
            ));
        }
        exact_definitions.push(TypeScriptProgramDefinitionMetadata {
            program_id: program_spec.program_id.clone(),
            sdk_definition_hash: None,
            program_spec_hash: program_spec
                .hash()
                .map_err(|error| {
                    format!(
                        "Stack '{}' could not hash ProgramSpecV1 at index {index}: {error}",
                        stack_spec.stack_name
                    )
                })?
                .to_string(),
            idl_content_hash: program_spec.idl_content_hash.to_string(),
            normalized_idl_hash: program_spec.normalized_idl_hash.to_string(),
        });
    }

    let Some(configured) = configured else {
        return stack_spec
            .program_specs
            .iter()
            .enumerate()
            .map(|(index, program_spec)| {
                arete_hash::OssProgramIdentityV1::new(program_spec.clone())
                    .map(|identity| TypeScriptProgramConfig::from(&identity))
                    .map_err(|error| {
                        format!(
                            "Stack '{}' could not derive the OSS release for program index {index}: {error}",
                            stack_spec.stack_name
                        )
                    })
            })
            .collect();
    };
    if configured.len() != expected {
        return Err(format!(
            "Stack '{}' hosted descriptor count mismatch: expected {expected}, received {}",
            stack_spec.stack_name,
            configured.len()
        ));
    }

    for (index, (hosted, exact)) in configured.iter().zip(&exact_definitions).enumerate() {
        if hosted.definition.program_id != exact.program_id {
            return Err(format!(
                "Stack '{}' hosted descriptor program ID mismatch at index {index}: expected '{}', received '{}'",
                stack_spec.stack_name,
                exact.program_id,
                hosted.definition.program_id
            ));
        }
        if hosted.definition.program_spec_hash != exact.program_spec_hash {
            return Err(format!(
                "Stack '{}' hosted descriptor programSpecHash mismatch at index {index}: expected '{}', received '{}'",
                stack_spec.stack_name,
                exact.program_spec_hash,
                hosted.definition.program_spec_hash
            ));
        }
        if hosted.definition.idl_content_hash != exact.idl_content_hash
            || hosted.definition.normalized_idl_hash != exact.normalized_idl_hash
        {
            return Err(format!(
                "Stack '{}' hosted descriptor definition hashes mismatch at index {index}",
                stack_spec.stack_name
            ));
        }
        if hosted.release.program_spec_hash != hosted.definition.program_spec_hash {
            return Err(format!(
                "Stack '{}' hosted descriptor release programSpecHash mismatch at index {index}",
                stack_spec.stack_name
            ));
        }
        if let TypeScriptProgramReadTransport::HostedBinding(binding) = &hosted.transport {
            let target_kind = binding
                .auth
                .get("targetKind")
                .and_then(serde_json::Value::as_str);
            let session_endpoint = binding
                .auth
                .get("sessionEndpoint")
                .and_then(serde_json::Value::as_str);
            if binding.endpoint.trim().is_empty()
                || binding.program_read_binding_id.trim().is_empty()
                || target_kind != Some("program-read-binding")
                || session_endpoint.is_none_or(|value| value.trim().is_empty())
            {
                return Err(format!(
                    "Stack '{}' hosted descriptor binding is incomplete at index {index}",
                    stack_spec.stack_name
                ));
            }
        }
    }

    Ok(configured.to_vec())
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
    compile_stack_spec_with_view_selection(stack_spec, config, false)
}

fn compile_stack_spec_with_view_selection(
    stack_spec: SerializableStackSpec,
    config: Option<TypeScriptStackConfig>,
    exact_views: bool,
) -> Result<TypeScriptStackOutput, String> {
    let config = config.unwrap_or_default();
    let program_configs = resolve_program_configs(&stack_spec, config.programs.as_deref(), true)?;
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
            url: config.websocket_url.clone(),
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
        if output
            .interfaces
            .contains("export interface CaptureWrapper<T>")
        {
            emitted_types.insert("CaptureWrapper".to_string());
        }

        // Only take the interfaces part (not the stack_definition — we generate our own)
        if !output.interfaces.is_empty() {
            all_interfaces.push(output.interfaces);
        }

        schema_names.extend(output.schema_names);
    }

    let mut interfaces = all_interfaces.join("\n\n");

    // 2. Generate instruction-construction handlers from the stack spec.
    // Program errors live once at the stack level (in the IDL snapshots) and
    // are scoped per program by the instruction codegen. Entity interface
    // names are reserved so defined-type interfaces cannot collide with them.
    let mut reserved_type_names: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for line in interfaces.lines() {
        for prefix in ["export interface ", "export type "] {
            if let Some(rest) = line.strip_prefix(prefix) {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    reserved_type_names.insert(name);
                }
            }
        }
    }
    let idl_account_artifacts =
        generate_idl_account_artifacts(&stack_spec.idls, &reserved_type_names);
    for type_name in &idl_account_artifacts.type_names {
        reserved_type_names.insert(type_name.clone());
    }
    if !idl_account_artifacts.code.is_empty() {
        if interfaces.is_empty() {
            interfaces = idl_account_artifacts.code.clone();
        } else {
            interfaces = format!("{}\n\n{}", interfaces, idl_account_artifacts.code);
        }
    }
    schema_names.extend(idl_account_artifacts.schema_names.clone());

    let instructions_codegen = crate::typescript_instructions::generate_instructions_code(
        stack_name,
        &stack_spec.instructions,
        &stack_spec.idls,
        &stack_spec.pdas,
        &stack_spec.program_ids,
        &reserved_type_names,
    );
    if !instructions_codegen.code.is_empty() {
        if interfaces.is_empty() {
            interfaces = instructions_codegen.code.clone();
        } else {
            interfaces = format!("{}\n\n{}", interfaces, instructions_codegen.code);
        }
    }

    let imports = assemble_sdk_imports(
        collect_emitted_pda_imports(&stack_spec.idls, &stack_spec.pdas),
        !idl_account_artifacts.account_type_names.is_empty(),
        &instructions_codegen,
    );
    let program_configs = program_content_identities(
        &ProgramGenerationContext {
            pdas: &stack_spec.pdas,
            program_ids: &stack_spec.program_ids,
            instruction_entries: &instructions_codegen.stack_entries,
            schema_names: &schema_names.iter().cloned().collect(),
            account_type_names: &idl_account_artifacts.account_type_names,
            programs: &program_configs,
        },
        &stack_spec.idls,
        &imports,
        &interfaces,
    )?;

    // 3. Generate unified stack definition with all entity views and attached program SDKs.
    let stack_definition = generate_stack_definition_multi(
        stack_name,
        &stack_kebab,
        &stack_spec.entities,
        &entity_names,
        &stack_spec.idls,
        &stack_spec.pdas,
        &stack_spec.program_ids,
        &schema_names,
        &idl_account_artifacts.account_type_names,
        &instructions_codegen.stack_entries,
        &program_configs,
        &config,
        exact_views,
    )?;

    Ok(TypeScriptStackOutput {
        imports,
        interfaces,
        stack_definition,
        warnings: instructions_codegen.warnings,
        pda_degradations: instructions_codegen.pda_degradations,
    })
}

/// Compile a stack model whose `views` have already been projected by a
/// StackManifest selected-view allowlist.
pub fn compile_stack_spec_with_exact_views(
    stack_spec: SerializableStackSpec,
    config: Option<TypeScriptStackConfig>,
) -> Result<TypeScriptStackOutput, String> {
    compile_stack_spec_with_view_selection(stack_spec, config, true)
}

/// Compile an explicit StackManifest and its exact public dependencies.
pub fn compile_public_artifacts(
    programs: &[arete_artifacts::ProgramSpecArtifact],
    live_spec: &arete_artifacts::LiveSpecArtifact,
    manifest: &arete_artifacts::StackManifestArtifact,
    config: Option<TypeScriptStackConfig>,
) -> Result<TypeScriptStackOutput, String> {
    let stack_spec =
        crate::public_artifacts::stack_spec_from_artifacts(programs, live_spec, manifest)?;
    compile_stack_spec(stack_spec, config)
}

/// Compile typed V2 public artifacts through the current single-live generator.
pub fn compile_public_artifacts_v2(
    programs: &[arete_artifacts::ProgramSpecArtifact],
    live_spec: &arete_artifacts::LiveSpecArtifactV2,
    manifest: &arete_artifacts::StackManifestArtifactV2,
    config: Option<TypeScriptStackConfig>,
) -> Result<TypeScriptStackOutput, String> {
    let stack_spec =
        crate::public_artifacts::stack_spec_from_artifacts_v2(programs, live_spec, manifest)?;
    compile_stack_spec_with_view_selection(stack_spec, config, true)
}

/// Compile each aliased LiveSpec into an independent module and generate a
/// manifest-level `createSession` definition that preserves exact alias keys.
pub fn compile_composed_public_artifacts_v2(
    programs: &[arete_artifacts::ProgramSpecArtifact],
    live_specs: &[(String, arete_artifacts::LiveSpecArtifactV2)],
    manifest: &arete_artifacts::StackManifestArtifactV2,
    config: Option<TypeScriptCompositionConfig>,
) -> Result<TypeScriptCompositionOutput, String> {
    let composed =
        crate::public_artifacts::stack_specs_from_artifacts_v2(programs, live_specs, manifest)?;
    if composed.live_specs.is_empty() {
        return Err(
            "TypeScript session generation requires at least one aliased LiveSpec".to_string(),
        );
    }
    let config = config.unwrap_or_default();
    let live_aliases = composed
        .live_specs
        .iter()
        .map(|live| live.alias.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(alias) = config
        .live_module_imports
        .keys()
        .find(|alias| !live_aliases.contains(alias.as_str()))
    {
        return Err(format!(
            "composition module import references unknown LiveSpec alias '{alias}'"
        ));
    }
    let mut outputs = Vec::with_capacity(composed.live_specs.len());
    let mut warnings = Vec::new();
    let mut pda_degradations = Vec::new();

    let live_program_hashes = live_specs
        .iter()
        .flat_map(|(_, live)| &live.payload.programs)
        .map(|requirement| requirement.program_spec_hash.to_string())
        .collect::<BTreeSet<_>>();
    let independent_programs = programs
        .iter()
        .filter(|program| !live_program_hashes.contains(&program.artifact_hash.to_string()))
        .cloned()
        .collect::<Vec<_>>();
    let independent_program_keys = independent_programs
        .iter()
        .map(|program| {
            let source = to_camel_case(&program.payload.idl_snapshot.snapshot.name);
            composition_program_key(program, &source)
        })
        .collect::<BTreeSet<_>>();
    if let Some(alias) = config
        .program_module_imports
        .keys()
        .find(|alias| !independent_program_keys.contains(alias.as_str()))
    {
        return Err(format!(
            "composition program module import references unknown independent program alias '{alias}'"
        ));
    }
    let program_collection = if independent_programs.is_empty() {
        None
    } else {
        let program_stack = crate::public_artifacts::stack_spec_from_program_artifacts(
            format!("{}Programs", composed.name),
            &independent_programs,
        )?;
        let mut program_config = config.stack.clone();
        program_config.websocket_url = None;
        program_config.http_url = None;
        program_config.programs =
            subset_program_configs(&program_stack, config.stack.programs.as_deref())?;
        let output =
            compile_stack_spec_with_view_selection(program_stack, Some(program_config), true)?;
        warnings.extend(output.warnings.iter().cloned());
        pda_degradations.extend(output.pda_degradations.iter().cloned());
        Some(TypeScriptProgramCollectionOutput {
            module_name: format!("{}-programs", to_kebab_case(&composed.name)),
            output,
            members: independent_programs
                .iter()
                .map(|program| {
                    let source = to_camel_case(&program.payload.idl_snapshot.snapshot.name);
                    let public = composition_program_key(program, &source);
                    (public, source)
                })
                .collect(),
        })
    };

    let mut promoted_programs = Vec::new();
    let mut promoted_hashes = BTreeMap::<String, String>::new();
    for live in composed.live_specs {
        let mut stack_config = config.stack.clone();
        if let Some(endpoints) = config.live_endpoints.get(&live.alias) {
            stack_config.websocket_url = endpoints.websocket_url.clone();
            stack_config.http_url = endpoints.http_url.clone();
        } else {
            stack_config.websocket_url = None;
            stack_config.http_url = None;
        }
        stack_config.programs =
            subset_program_configs(&live.stack_spec, config.stack.programs.as_deref())?;
        for program in &live.stack_spec.program_specs {
            let source = to_camel_case(&program.idl_snapshot.snapshot.name);
            let hash = program
                .hash()
                .map_err(|error| error.to_string())?
                .to_string();
            if let Some(existing_hash) = promoted_hashes.get(&source) {
                if existing_hash != &hash {
                    return Err(format!(
                        "composition programs use duplicate generated key '{source}' for different ProgramSpecs"
                    ));
                }
                continue;
            }
            promoted_hashes.insert(source.clone(), hash);
            promoted_programs.push((source.clone(), live.alias.clone(), source));
        }
        let module_name = typescript_module_name(&live.alias);
        let output =
            compile_stack_spec_with_view_selection(live.stack_spec, Some(stack_config), true)?;
        warnings.extend(output.warnings.iter().cloned());
        pda_degradations.extend(output.pda_degradations.iter().cloned());
        outputs.push(TypeScriptAliasedStackOutput {
            alias: live.alias,
            module_name,
            output,
        });
    }

    let session_definition = generate_session_definition(
        &composed.name,
        &outputs,
        &promoted_programs,
        program_collection.as_ref(),
        &config.live_module_imports,
        &config.program_module_imports,
        config.stack.gateway.as_ref(),
    );
    Ok(TypeScriptCompositionOutput {
        name: composed.name,
        live_stacks: outputs,
        program_collection,
        session_definition,
        warnings,
        pda_degradations,
    })
}

fn subset_program_configs(
    stack_spec: &SerializableStackSpec,
    configured: Option<&[TypeScriptProgramConfig]>,
) -> Result<Option<Vec<TypeScriptProgramConfig>>, String> {
    let Some(configured) = configured else {
        return Ok(None);
    };
    let by_hash = configured
        .iter()
        .map(|program| {
            (
                program.definition.program_spec_hash.as_str(),
                program.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    stack_spec
        .program_specs
        .iter()
        .map(|program| {
            let hash = program
                .hash()
                .map_err(|error| error.to_string())?
                .to_string();
            by_hash.get(hash.as_str()).cloned().ok_or_else(|| {
                format!("missing configured program descriptor for ProgramSpec {hash}")
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn generate_session_definition(
    manifest_name: &str,
    live_stacks: &[TypeScriptAliasedStackOutput],
    promoted_programs: &[(String, String, String)],
    program_collection: Option<&TypeScriptProgramCollectionOutput>,
    live_module_imports: &BTreeMap<String, String>,
    program_module_imports: &BTreeMap<String, String>,
    gateway: Option<&serde_json::Value>,
) -> String {
    let manifest_pascal = safe_pascal_identifier(manifest_name);
    let definition_name = format!(
        "{}_SESSION_DEFINITION",
        to_screaming_snake_case(&manifest_pascal)
    );
    let imports = live_stacks
        .iter()
        .map(|live| {
            let import = live_module_imports
                .get(&live.alias)
                .cloned()
                .unwrap_or_else(|| format!("./{}.js", live.module_name));
            format!(
                "import {}Stack from '{}';",
                safe_pascal_identifier(&live.alias),
                import
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let program_import = program_collection
        .map(|programs| {
            format!(
                "import {manifest_pascal}Programs from './{}.js';",
                programs.module_name
            )
        })
        .unwrap_or_default();
    let program_module_import_lines = program_module_imports
        .iter()
        .map(|(alias, import)| {
            format!(
                "import {}Program from '{}';",
                safe_pascal_identifier(alias),
                import
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let program_members = program_collection
        .map(|programs| {
            let definitions = programs
                .members
                .iter()
                .map(|(public, source)| {
                    let value = if program_module_imports.contains_key(public) {
                        format!("{}Program", safe_pascal_identifier(public))
                    } else {
                        format!(
                            "{manifest_pascal}Programs.programs.{}",
                            typescript_property_key(source)
                        )
                    };
                    format!("    {}: {value},", typescript_property_key(public))
                })
                .collect::<Vec<_>>()
                .join("\n");
            let reads = programs
                .members
                .iter()
                .map(|(public, source)| {
                    format!(
                        "    {}: {manifest_pascal}Programs.programReads.{},",
                        typescript_property_key(public),
                        typescript_property_key(source)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            (definitions, reads)
        })
        .unwrap_or_default();
    let promoted_definitions = promoted_programs
        .iter()
        .map(|(public, live_alias, source)| {
            format!(
                "    {}: {}Stack.programs.{},",
                typescript_property_key(public),
                safe_pascal_identifier(live_alias),
                typescript_property_key(source)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let promoted_reads = promoted_programs
        .iter()
        .map(|(public, live_alias, source)| {
            format!(
                "    {}: {}Stack.programReads.{},",
                typescript_property_key(public),
                safe_pascal_identifier(live_alias),
                typescript_property_key(source)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let definitions = [promoted_definitions, program_members.0]
        .into_iter()
        .filter(|members| !members.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let reads = [promoted_reads, program_members.1]
        .into_iter()
        .filter(|members| !members.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let program_members =
        format!("  programs: {{\n{definitions}\n  }},\n  programReads: {{\n{reads}\n  }},");
    let members = live_stacks
        .iter()
        .map(|live| {
            format!(
                "    {}: {}Stack,",
                typescript_property_key(&live.alias),
                safe_pascal_identifier(&live.alias)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let gateway_member = gateway
        .map(|gateway| format!("  gateway: {},", gateway))
        .unwrap_or_default();
    format!(
        r#"import {{ createSession, type CompositionSessionOptions }} from '@usearete/sdk';
{imports}
{program_import}
{program_module_import_lines}

export const {definition_name} = {{
  mode: 'composition',
{gateway_member}
  stacks: {{
{members}
  }},
{program_members}
}} as const;

export type {manifest_pascal}SessionDefinition = typeof {definition_name};
export const {manifest_screaming}_SDK = {definition_name};
export type {manifest_pascal}Sdk = {manifest_pascal}SessionDefinition;

export function create{manifest_pascal}Session(
  options: CompositionSessionOptions<{manifest_pascal}SessionDefinition>
) {{
  return createSession({definition_name}, options);
}}
"#,
        imports = imports,
        program_import = program_import,
        program_module_import_lines = program_module_import_lines,
        definition_name = definition_name,
        gateway_member = gateway_member,
        members = members,
        program_members = program_members,
        manifest_pascal = manifest_pascal,
        manifest_screaming = to_screaming_snake_case(&manifest_pascal),
    )
}

fn safe_pascal_identifier(value: &str) -> String {
    let mut output = value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .map(to_pascal_case)
        .collect::<String>();
    if output.is_empty() {
        output.push_str("Manifest");
    }
    if output
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        output.insert(0, 'A');
    }
    output
}

fn typescript_module_name(alias: &str) -> String {
    let module = alias
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("{module}-stack")
}

fn composition_program_key(
    program: &arete_artifacts::ProgramSpecArtifact,
    generated_key: &str,
) -> String {
    match program.payload.program_id.as_str() {
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" => "splToken".to_string(),
        "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL" => "splAta".to_string(),
        _ => generated_key.to_string(),
    }
}

/// Assemble the `zod` + `@usearete/sdk` import lines based on which runtime
/// helpers the emitted code references.
fn assemble_sdk_imports(
    pda_imports: PdaImportUsage,
    has_account_reads: bool,
    instructions_codegen: &crate::typescript_instructions::InstructionsCodegen,
) -> String {
    let mut sdk_named: Vec<String> = Vec::new();
    for (needed, helper) in [
        (pda_imports.pda, "pda"),
        (pda_imports.literal, "literal"),
        (pda_imports.account, "account"),
        (pda_imports.arg, "arg"),
        (pda_imports.bytes, "bytes"),
    ] {
        if needed {
            sdk_named.push(helper.to_string());
        }
    }
    if has_account_reads {
        sdk_named.push("programAccountRead".to_string());
    }
    if instructions_codegen.needs_runtime_import {
        sdk_named.push("createInstructionHandler".to_string());
        sdk_named.push("type ErrorMetadata".to_string());
    }
    if !instructions_codegen.stack_entries.is_empty() {
        sdk_named.push("buildInstruction".to_string());
    }
    if instructions_codegen.needs_build_options {
        sdk_named.push("type BuildOptions".to_string());
    }
    if instructions_codegen.needs_program_runtime_extensions {
        sdk_named.push("PROGRAM_OPERATION_EXTENSIONS".to_string());
        sdk_named.push("instructionOperation".to_string());
        sdk_named.push("createPreparedInstruction".to_string());
    }
    if instructions_codegen.needs_operation_context {
        sdk_named.push("type ProgramOperationContext".to_string());
    }
    if instructions_codegen.needs_amount_input {
        sdk_named.push("type AmountInput".to_string());
    }
    if instructions_codegen.needs_resolve_amount_to_raw {
        sdk_named.push("resolveAmountToRaw".to_string());
    }
    if instructions_codegen.needs_to_raw_amount {
        sdk_named.push("toRawAmount".to_string());
    }
    if sdk_named.is_empty() {
        "import { z } from 'zod';".to_string()
    } else {
        format!(
            "import {{ z }} from 'zod';\nimport {{ {} }} from '@usearete/sdk';",
            sdk_named.join(", ")
        )
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct PdaImportUsage {
    pda: bool,
    literal: bool,
    account: bool,
    arg: bool,
    bytes: bool,
}

fn collect_emitted_pda_imports(
    idls: &[IdlSnapshot],
    pdas: &BTreeMap<String, BTreeMap<String, PdaDefinition>>,
) -> PdaImportUsage {
    let mut usage = PdaImportUsage::default();

    for idl in idls {
        let Some(program_pdas) = pdas
            .get(&idl.name)
            .or_else(|| pdas.get(&to_camel_case(&idl.name)))
            .filter(|program_pdas| !program_pdas.is_empty())
        else {
            continue;
        };

        usage.pda = true;
        for seed in program_pdas.values().flat_map(|pda| &pda.seeds) {
            match seed {
                PdaSeedDef::Literal { .. } => usage.literal = true,
                PdaSeedDef::AccountRef { .. } => usage.account = true,
                PdaSeedDef::ArgRef { .. } => usage.arg = true,
                PdaSeedDef::Bytes { .. } => usage.bytes = true,
            }
        }
    }

    usage
}

/// Compile only the program-SDK surface of a stack spec — account types +
/// Zod schemas, instruction handlers, and standalone per-program consts.
/// No entities, views, or stack const are emitted, and the spec's `entities`
/// may be empty. Used by `a4 sdk create --ts --program-only`.
pub fn compile_program_modules(
    stack_spec: SerializableStackSpec,
    config: Option<TypeScriptStackConfig>,
) -> Result<TypeScriptStackOutput, String> {
    let config = config.unwrap_or_default();
    let program_configs = resolve_program_configs(&stack_spec, config.programs.as_deref(), false)?;
    let stack_name = &stack_spec.stack_name;

    if stack_spec.idls.is_empty() {
        return Err(format!(
            "Stack '{}' carries no IDLs; a program-only SDK has nothing to emit",
            stack_name
        ));
    }

    let mut reserved_type_names: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let idl_account_artifacts =
        generate_idl_account_artifacts(&stack_spec.idls, &reserved_type_names);
    for type_name in &idl_account_artifacts.type_names {
        reserved_type_names.insert(type_name.clone());
    }

    let mut interfaces = idl_account_artifacts.code.clone();

    let instructions_codegen = crate::typescript_instructions::generate_instructions_code(
        stack_name,
        &stack_spec.instructions,
        &stack_spec.idls,
        &stack_spec.pdas,
        &stack_spec.program_ids,
        &reserved_type_names,
    );
    if !instructions_codegen.code.is_empty() {
        if interfaces.is_empty() {
            interfaces = instructions_codegen.code.clone();
        } else {
            interfaces = format!("{}\n\n{}", interfaces, instructions_codegen.code);
        }
    }

    let unique_schemas: BTreeSet<String> =
        idl_account_artifacts.schema_names.iter().cloned().collect();

    let imports = assemble_sdk_imports(
        collect_emitted_pda_imports(&stack_spec.idls, &stack_spec.pdas),
        !idl_account_artifacts.account_type_names.is_empty(),
        &instructions_codegen,
    );

    let program_context = ProgramGenerationContext {
        pdas: &stack_spec.pdas,
        program_ids: &stack_spec.program_ids,
        instruction_entries: &instructions_codegen.stack_entries,
        schema_names: &unique_schemas,
        account_type_names: &idl_account_artifacts.account_type_names,
        programs: &program_configs,
    };
    let program_configs =
        program_content_identities(&program_context, &stack_spec.idls, &imports, &interfaces)?;
    let program_context = ProgramGenerationContext {
        programs: &program_configs,
        ..program_context
    };
    let stack_definition =
        generate_program_definitions(stack_name, &stack_spec.idls, &program_context);

    Ok(TypeScriptStackOutput {
        imports,
        interfaces,
        stack_definition,
        warnings: instructions_codegen.warnings,
        pda_degradations: instructions_codegen.pda_degradations,
    })
}

/// Compile standalone program SDK modules directly from ProgramSpec artifacts.
pub fn compile_program_artifacts(
    name: impl Into<String>,
    programs: &[arete_artifacts::ProgramSpecArtifact],
    config: Option<TypeScriptStackConfig>,
) -> Result<TypeScriptStackOutput, String> {
    let stack_spec = crate::public_artifacts::stack_spec_from_program_artifacts(name, programs)?;
    compile_program_modules(stack_spec, config)
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
    idls: &[IdlSnapshot],
    pdas: &BTreeMap<String, BTreeMap<String, PdaDefinition>>,
    program_ids: &[String],
    schema_names: &[String],
    account_type_names: &BTreeMap<(String, String), String>,
    instruction_entries: &[crate::typescript_instructions::StackInstructionEntry],
    program_configs: &[TypeScriptProgramConfig],
    config: &TypeScriptStackConfig,
    exact_views: bool,
) -> Result<String, String> {
    let export_name = format!(
        "{}_{}",
        to_screaming_snake_case(stack_name),
        config.export_const_name
    );
    let core_export_name = format!("{}_CORE", export_name);

    let view_helpers = generate_view_helpers_static();

    let websocket_endpoint = match &config.websocket_url {
        Some(url) => format!("    ws: '{}',", url),
        None => "    ws: '', // TODO: Set after first deployment or pass useArete(..., { url })"
            .to_string(),
    };
    let http_endpoint = match &config.http_url {
        Some(url) => format!("    http: '{}',", url),
        None => {
            "    http: '', // TODO: Set after first deployment or pass useArete(..., { httpUrl })"
                .to_string()
        }
    };
    let endpoints_block = format!(
        "  endpoints: {{\n{}\n{}\n  }},",
        websocket_endpoint, http_endpoint
    );
    let gateway_block = config
        .gateway
        .as_ref()
        .map(|gateway| {
            format!(
                "\n  gateway: {},",
                serde_json::to_string(gateway).expect("gateway descriptor must serialize")
            )
        })
        .unwrap_or_default();

    // Generate views block for each entity
    let mut entity_view_blocks = Vec::new();
    for (i, entity_spec) in entities.iter().enumerate() {
        let entity_name = &entity_names[i];
        let entity_pascal = to_pascal_case(entity_name);
        let mut view_entries = Vec::new();

        if !exact_views
            || entity_spec
                .views
                .iter()
                .any(|view| view.id == format!("{entity_name}/state"))
        {
            let state_view_key = state_view_key_definition(
                entity_name,
                &entity_spec.identity,
                &entity_spec.field_mappings,
                &entity_spec.sections,
            )?;
            view_entries.push(format!(
                "      state: stateView<{entity}, {key_type}>('{entity_name}/state', {key_fields}),",
                entity = entity_pascal,
                entity_name = entity_name,
                key_type = state_view_key.object_type(),
                key_fields = state_view_key.fields_literal(),
            ));
        }

        if !exact_views
            || entity_spec
                .views
                .iter()
                .any(|view| view.id == format!("{entity_name}/list"))
        {
            view_entries.push(format!(
                "      list: listView<{entity}>('{entity_name}/list'),",
                entity = entity_pascal,
                entity_name = entity_name
            ));
        }

        for view in &entity_spec.views {
            if !view.id.ends_with("/state")
                && !view.id.ends_with("/list")
                && view.id.starts_with(entity_name)
            {
                let view_name = view.id.split('/').nth(1).unwrap_or("unknown");
                view_entries.push(format!(
                    "      {}: listView<{entity}>('{}'),",
                    typescript_property_key(view_name),
                    view.id,
                    entity = entity_pascal
                ));
            }
        }

        if !view_entries.is_empty() {
            entity_view_blocks.push(format!(
                "    {}: {{\n{}\n    }},",
                typescript_property_key(entity_name),
                view_entries.join("\n")
            ));
        }
    }

    let views_body = entity_view_blocks.join("\n");

    let mut unique_schemas: BTreeSet<String> = BTreeSet::new();
    for name in schema_names {
        unique_schemas.insert(name.clone());
    }
    let schemas_block = if unique_schemas.is_empty() {
        String::new()
    } else {
        let schema_entries: Vec<String> = unique_schemas
            .iter()
            .filter(|name| name.ends_with("Schema") && !name.ends_with("PatchSchema"))
            .map(|name| format!("    {}: {},", name.trim_end_matches("Schema"), name))
            .collect();
        if schema_entries.is_empty() {
            String::new()
        } else {
            format!("\n  schemas: {{\n{}\n  }},", schema_entries.join("\n"))
        }
    };
    let patch_schema_entries: Vec<String> = entity_names
        .iter()
        .map(|entity_name| {
            let entity_pascal = to_pascal_case(entity_name);
            format!("    {}: {}PatchSchema,", entity_pascal, entity_pascal)
        })
        .collect();
    let patch_schemas_block = if patch_schema_entries.is_empty() {
        String::new()
    } else {
        format!(
            "\n  patchSchemas: {{\n{}\n  }},",
            patch_schema_entries.join("\n")
        )
    };

    let program_context = ProgramGenerationContext {
        pdas,
        program_ids,
        instruction_entries,
        schema_names: &unique_schemas,
        account_type_names,
        programs: program_configs,
    };
    let programs_block = generate_programs_block(idls, &program_context);
    let program_reads_block = generate_program_reads_block(idls, &program_context);
    let addresses_block = generate_stack_addresses_block(idls, pdas, program_ids);

    let entity_types: Vec<String> = entity_names.iter().map(|n| to_pascal_case(n)).collect();

    let stack_export = format!(
        r#"export const {core_export_name} = {{
  name: '{stack_kebab}',
{endpoints_block}{gateway_block}
  views: {{
{views_body}
  }},{schemas_section}{patch_schemas_section}{programs_section}{program_reads_section}{addresses_section}
}} as const;"#,
        core_export_name = core_export_name,
        stack_kebab = stack_kebab,
        endpoints_block = endpoints_block,
        gateway_block = gateway_block,
        views_body = views_body,
        schemas_section = schemas_block,
        patch_schemas_section = patch_schemas_block,
        programs_section = programs_block,
        program_reads_section = program_reads_block,
        addresses_section = addresses_block,
    );

    Ok(format!(
        r#"{view_helpers}

// ============================================================================
// Stack Definition
// ============================================================================

/** Stack definition for {stack_name} with {entity_count} entities */
{stack_export}

/** Type alias for the core stack */
export type {stack_name}CoreStack = typeof {core_export_name};

/** Entity types in this stack */
export type {stack_name}Entity = {entity_union};

/** Default export for convenience */
export default {core_export_name};"#,
        view_helpers = view_helpers,
        stack_name = stack_name,
        entity_count = entities.len(),
        core_export_name = core_export_name,
        stack_export = stack_export,
        entity_union = if entity_types.is_empty() {
            "never".to_string()
        } else {
            entity_types.join(" | ")
        },
    ))
}

fn typescript_property_key(value: &str) -> String {
    let mut characters = value.chars();
    let valid = characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || matches!(character, '_' | '$'))
        && characters
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '$'));
    if valid {
        value.to_string()
    } else {
        serde_json::to_string(value).expect("string property serialization cannot fail")
    }
}

struct ProgramGenerationContext<'a> {
    pdas: &'a BTreeMap<String, BTreeMap<String, PdaDefinition>>,
    program_ids: &'a [String],
    instruction_entries: &'a [crate::typescript_instructions::StackInstructionEntry],
    schema_names: &'a BTreeSet<String>,
    account_type_names: &'a BTreeMap<(String, String), String>,
    programs: &'a [TypeScriptProgramConfig],
}

/// Frozen content projection for the TypeScript program runtime contract.
/// Includes the emitted imports, shared declarations (schemas and instruction
/// implementations), and this program's literal body with sdkDefinitionHash
/// absent. Release/read bindings and stack wrappers are separate identities.
/// Shared declarations deliberately make this conservative within one module.
const TYPESCRIPT_PROGRAM_RUNTIME_CONTRACT: &str = "@usearete/sdk/program-definition-v1";

fn program_content_identities(
    context: &ProgramGenerationContext<'_>,
    idls: &[IdlSnapshot],
    imports: &str,
    declarations: &str,
) -> Result<Vec<TypeScriptProgramConfig>, String> {
    use arete_hash::{hash_artifact_tree, ArtifactTreeEntry, SdkDefinitionV2, SdkOutputTree};

    let mut unhashed = context.programs.to_vec();
    for program in &mut unhashed {
        program.definition.sdk_definition_hash = None;
    }
    let unhashed_context = ProgramGenerationContext {
        programs: &unhashed,
        ..*context
    };
    let mut identified = unhashed.clone();
    for (index, program) in identified.iter_mut().enumerate() {
        let (_, sections) =
            generate_single_program_sections(&idls[index], index, &unhashed_context);
        let definition = sections.join("\n");
        let output_tree_hash = hash_artifact_tree::<SdkOutputTree>(&[
            ArtifactTreeEntry::file("imports.ts", imports.as_bytes()),
            ArtifactTreeEntry::file("declarations.ts", declarations.as_bytes()),
            ArtifactTreeEntry::file("definition.ts", definition.as_bytes()),
        ])
        .map_err(|error| error.to_string())?;
        let input_hash = program
            .definition
            .program_spec_hash
            .parse()
            .map_err(|error: arete_hash::HashError| error.to_string())?;
        program.definition.sdk_definition_hash = Some(
            SdkDefinitionV2::new(
                input_hash,
                "typescript",
                TYPESCRIPT_PROGRAM_RUNTIME_CONTRACT,
                output_tree_hash,
            )
            .hash()
            .map_err(|error| error.to_string())?
            .to_string(),
        );
    }
    Ok(identified)
}

/// Build one program's `{ name, programId, pdas?, accounts?, instructions? }`
/// literal body. Sections are indented for nesting inside a stack const
/// (`programs: { <key>: { ... } }`); callers emitting top-level program
/// consts dedent them.
fn generate_single_program_sections(
    idl: &IdlSnapshot,
    index: usize,
    context: &ProgramGenerationContext<'_>,
) -> (String, Vec<String>) {
    let program_key = to_camel_case(&idl.name);
    let metadata = &context.programs[index];
    let multi_program = context.program_ids.len() > 1
        || context
            .instruction_entries
            .iter()
            .any(|entry| entry.program_key.is_some());
    let program_id = context
        .program_ids
        .get(index)
        .cloned()
        .or_else(|| idl.program_id.clone())
        .unwrap_or_default();

    let instruction_entries_for_program: Vec<
        &crate::typescript_instructions::StackInstructionEntry,
    > = context
        .instruction_entries
        .iter()
        .filter(|entry| {
            if multi_program {
                entry.program_key.as_deref() == Some(program_key.as_str())
            } else {
                true
            }
        })
        .collect();
    let instruction_entry_literals: Vec<String> = instruction_entries_for_program
        .iter()
        .map(|entry| {
            format!(
                "        {}: {},",
                entry.instruction_name, entry.handler_const
            )
        })
        .collect();

    let account_entries: Vec<String> = idl
        .accounts
        .iter()
        .filter_map(|account| {
            let type_name = context
                .account_type_names
                .get(&(program_key.clone(), account.name.clone()))?
                .clone();
            let schema_name = format!("{}Schema", type_name);
            if !context.schema_names.contains(&schema_name) {
                return None;
            }
            Some((account.name.clone(), type_name, schema_name))
        })
        .map(|account| {
            format!(
                "        {account_name}: programAccountRead<{type_name}>({{ account: '{account_name}', schema: {schema_name} }}),",
                account_name = account.0,
                type_name = account.1,
                schema_name = account.2,
            )
        })
        .collect();

    let program_pdas = context
        .pdas
        .get(&idl.name)
        .or_else(|| context.pdas.get(&program_key))
        .filter(|program_pdas| !program_pdas.is_empty());

    let mut sections: Vec<String> = vec![
        format!("      name: '{}',", idl.name),
        format!("      programId: '{}',", program_id),
    ];
    if let Some(definition_hash) = &metadata.definition.sdk_definition_hash {
        sections.push(format!("      sdkDefinitionHash: '{}',", definition_hash));
    }
    sections.extend([
        format!(
            "      programSpecHash: '{}',",
            metadata.definition.program_spec_hash
        ),
        format!(
            "      idlContentHash: '{}',",
            metadata.definition.idl_content_hash
        ),
        format!(
            "      normalizedIdlHash: '{}',",
            metadata.definition.normalized_idl_hash
        ),
    ]);

    if let Some(gateway) = &metadata.gateway {
        sections.push(format!(
            "      gateway: {},",
            serde_json::to_string(gateway).expect("gateway descriptor must serialize")
        ));
    }

    if let Some(program_pdas) = program_pdas {
        let pda_entries = generate_program_pda_entries(program_pdas, &program_id, "        ");
        if !pda_entries.is_empty() {
            sections.push(format!(
                "      pdas: {{\n{}\n      }},",
                pda_entries.join("\n")
            ));
            sections.push(format!(
                "      addresses: {{\n{}\n      }},",
                pda_entries.join("\n")
            ));
        }
    }

    if !account_entries.is_empty() {
        sections.push(format!(
            "      accounts: {{\n{}\n      }},",
            account_entries.join("\n")
        ));
    }

    if !instruction_entry_literals.is_empty() {
        sections.push(format!(
            "      rawInstructions: {{\n{}\n      }},",
            instruction_entry_literals.join("\n")
        ));
        if let Some(semantic_block) =
            generate_program_semantic_instructions_block(&instruction_entries_for_program, "      ")
        {
            sections.push(semantic_block);
        }
    }

    (program_key, sections)
}

fn generate_programs_block(idls: &[IdlSnapshot], context: &ProgramGenerationContext<'_>) -> String {
    if idls.is_empty() || context.programs.is_empty() {
        return String::new();
    }

    let mut program_blocks = Vec::new();

    for (index, idl) in idls.iter().enumerate() {
        let (program_key, sections) = generate_single_program_sections(idl, index, context);

        program_blocks.push(format!(
            "    {}: {{\n{}\n    }},",
            program_key,
            sections.join("\n")
        ));
    }

    if program_blocks.is_empty() {
        return String::new();
    }

    format!("\n  programs: {{\n{}\n  }},", program_blocks.join("\n"))
}

fn generate_program_read_sections(metadata: &TypeScriptProgramConfig, indent: &str) -> Vec<String> {
    let mut sections = vec![format!(
        "{indent}release: {{ programReleaseHash: {release_hash}, programSpecHash: {spec_hash} }},",
        release_hash = serde_json::to_string(&metadata.release.program_release_hash)
            .expect("program release hash must serialize"),
        spec_hash = serde_json::to_string(&metadata.release.program_spec_hash)
            .expect("program spec hash must serialize"),
    )];
    sections.push(match &metadata.transport {
        TypeScriptProgramReadTransport::LocalHttp => format!(
            "{indent}transport: {{ kind: 'local-http', endpointSource: 'connect-http-url' }},"
        ),
        TypeScriptProgramReadTransport::HostedBinding(binding) => format!(
            "{indent}transport: {{ kind: 'hosted-binding', binding: {{ endpoint: {endpoint}, programReadBindingId: {binding_id}, auth: {auth} }} }},",
            endpoint = serde_json::to_string(&binding.endpoint)
                .expect("program endpoint must serialize"),
            binding_id = serde_json::to_string(&binding.program_read_binding_id)
                .expect("program binding ID must serialize"),
            auth = serde_json::to_string(&binding.auth)
                .expect("program auth metadata must serialize"),
        ),
    });
    sections
}

fn generate_program_reads_block(
    idls: &[IdlSnapshot],
    context: &ProgramGenerationContext<'_>,
) -> String {
    if idls.is_empty() || context.programs.is_empty() {
        return String::new();
    }

    let entries = idls
        .iter()
        .zip(context.programs)
        .map(|(idl, metadata)| {
            format!(
                "    {}: {{\n{}\n    }},",
                to_camel_case(&idl.name),
                generate_program_read_sections(metadata, "      ").join("\n")
            )
        })
        .collect::<Vec<_>>();
    format!("\n  programReads: {{\n{}\n  }},", entries.join("\n"))
}

/// Strip up to `spaces` leading spaces from every line.
fn dedent_lines(text: &str, spaces: usize) -> String {
    text.lines()
        .map(|line| {
            let strip = line
                .char_indices()
                .take_while(|(i, c)| *i < spaces && *c == ' ')
                .count();
            &line[strip..]
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Emit standalone per-program consts plus a combined `<STACK>_PROGRAMS` map:
///
/// ```typescript
/// export const SQUADS_MULTISIG_PROGRAM = { name, programId, pdas, accounts, instructions } as const;
/// export const SQUADS_V4_PROGRAMS = { squadsMultisigProgram: SQUADS_MULTISIG_PROGRAM } as const;
/// ```
///
/// Each const structurally satisfies the runtime's `ProgramSdkDefinition`, so
/// the map can be dropped straight into `createSession({ programs: ... })`.
fn generate_program_definitions(
    stack_name: &str,
    idls: &[IdlSnapshot],
    context: &ProgramGenerationContext<'_>,
) -> String {
    let mut program_consts = Vec::new();
    let mut map_entries = Vec::new();
    let mut program_read_consts = Vec::new();
    let mut read_map_entries = Vec::new();

    for (index, idl) in idls.iter().enumerate() {
        let (program_key, sections) = generate_single_program_sections(idl, index, context);
        let const_name = to_screaming_snake_case(&idl.name);
        let body = dedent_lines(&sections.join("\n"), 4);
        program_consts.push(format!(
            "/** Standalone program SDK for '{name}' */\nexport const {const_name} = {{\n{body}\n}} as const;",
            name = idl.name,
            const_name = const_name,
            body = body,
        ));
        map_entries.push(format!("  {}: {},", program_key, const_name));
        let read_const_name = format!("{}_READ", const_name);
        let read_body = dedent_lines(
            &generate_program_read_sections(&context.programs[index], "    ").join("\n"),
            4,
        );
        program_read_consts.push(format!(
            "/** Release and explicit read transport for '{name}' */\nexport const {read_const_name} = {{\n{read_body}\n}} as const;",
            name = idl.name,
        ));
        read_map_entries.push(format!("  {}: {},", program_key, read_const_name));
    }

    let map_name = format!("{}_PROGRAMS", to_screaming_snake_case(stack_name));
    let reads_map_name = format!("{}_PROGRAM_READS", to_screaming_snake_case(stack_name));
    let type_name = format!("{}Programs", to_pascal_case(stack_name));

    format!(
        r#"// ============================================================================
// Program Definitions
// ============================================================================

{program_consts}

{program_read_consts}

/** All portable programs from the {stack_name} stack */
export const {map_name} = {{
{map_entries}
}} as const;

/** Parallel release/read metadata keyed identically to {map_name} */
export const {reads_map_name} = {{
{read_map_entries}
}} as const;

export type {type_name} = typeof {map_name};

export default {map_name};"#,
        program_consts = program_consts.join("\n\n"),
        program_read_consts = program_read_consts.join("\n\n"),
        stack_name = stack_name,
        map_name = map_name,
        map_entries = map_entries.join("\n"),
        reads_map_name = reads_map_name,
        read_map_entries = read_map_entries.join("\n"),
        type_name = type_name,
    )
}

fn generate_program_pda_entries(
    program_pdas: &BTreeMap<String, PdaDefinition>,
    default_program_id: &str,
    indent: &str,
) -> Vec<String> {
    if program_pdas.is_empty() {
        return Vec::new();
    }

    program_pdas
        .iter()
        .map(|(pda_name, pda_def)| {
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

            let program = match (&pda_def.program_id, &pda_def.program) {
                (Some(pid), _) => format!("'{}'", pid),
                (None, Some(PdaProgramDef::AccountRef { account_name })) => {
                    format!("{{ type: 'accountRef', accountName: '{}' }}", account_name)
                }
                (None, Some(PdaProgramDef::ArgRef { arg_name })) => {
                    format!("{{ type: 'argRef', argName: '{}' }}", arg_name)
                }
                (None, None) => format!("'{}'", default_program_id),
            };
            let rendered_seeds = if seeds_str.is_empty() {
                String::new()
            } else {
                format!(", {}", seeds_str)
            };
            format!(
                "{}{}: pda({}{}),",
                indent, pda_name, program, rendered_seeds
            )
        })
        .collect()
}

fn generate_stack_addresses_block(
    idls: &[IdlSnapshot],
    pdas: &BTreeMap<String, BTreeMap<String, PdaDefinition>>,
    program_ids: &[String],
) -> String {
    if idls.is_empty() {
        return String::new();
    }

    if idls.len() == 1 {
        let idl = &idls[0];
        let program_id = program_ids
            .first()
            .cloned()
            .or_else(|| idl.program_id.clone())
            .unwrap_or_default();
        let Some(program_pdas) = pdas
            .get(&idl.name)
            .or_else(|| pdas.get(&to_camel_case(&idl.name)))
        else {
            return String::new();
        };
        let entries = generate_program_pda_entries(program_pdas, &program_id, "    ");
        if entries.is_empty() {
            return String::new();
        }
        return format!("\n  addresses: {{\n{}\n  }},", entries.join("\n"));
    }

    let mut blocks = Vec::new();
    for (index, idl) in idls.iter().enumerate() {
        let program_id = program_ids
            .get(index)
            .cloned()
            .or_else(|| idl.program_id.clone())
            .unwrap_or_default();
        let Some(program_pdas) = pdas
            .get(&idl.name)
            .or_else(|| pdas.get(&to_camel_case(&idl.name)))
        else {
            continue;
        };
        let entries = generate_program_pda_entries(program_pdas, &program_id, "      ");
        if entries.is_empty() {
            continue;
        }
        blocks.push(format!(
            "    {}: {{\n{}\n    }},",
            to_camel_case(&idl.name),
            entries.join("\n")
        ));
    }

    if blocks.is_empty() {
        String::new()
    } else {
        format!("\n  addresses: {{\n{}\n  }},", blocks.join("\n"))
    }
}

fn is_valid_ts_identifier(name: &str) -> bool {
    name.chars()
        .next()
        .map(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        .unwrap_or(false)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

fn escape_ts_single_quotes(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace(['\n', '\r'], " ")
}

fn render_ts_property_name_literal(name: &str) -> String {
    if is_valid_ts_identifier(name) {
        name.to_string()
    } else {
        format!("'{}'", escape_ts_single_quotes(name))
    }
}

fn render_program_semantic_instruction_entry(
    entry: &crate::typescript_instructions::StackInstructionEntry,
    indent: &str,
) -> Option<String> {
    let semantic_params_type = entry.semantic_params_type.as_ref()?;
    entry.runtime_program_key.as_ref()?;

    if entry.semantic_amount_args.is_empty() {
        return Some(format!(
            "{indent}{instruction_name}: instructionOperation(async (params: {semantic_params_type}) => {{\n{indent}  const instruction = buildInstruction({handler_const}, params as unknown as Record<string, unknown>);\n{indent}  return createPreparedInstruction({{\n{indent}    name: '{instruction_name}',\n{indent}    instruction,\n{indent}    artifacts: {{ instruction }},\n{indent}    errors: {handler_const}.errors,\n{indent}  }});\n{indent}}}),",
            indent = indent,
            instruction_name = entry.instruction_name,
            semantic_params_type = semantic_params_type,
            handler_const = entry.handler_const,
        ));
    }

    let raw_params_setup = if entry.semantic_extra_params.is_empty() {
        format!(
            "{indent}    const {{ build, ...rawParams }} = params;",
            indent = indent
        )
    } else {
        format!(
            "{indent}    const {{ build, {extras}, ...rawParams }} = params;",
            indent = indent,
            extras = entry.semantic_extra_params.join(", "),
        )
    };
    let resolution_lines: Vec<String> = entry
        .semantic_amount_args
        .iter()
        .map(|amount_arg| {
            format!(
                "{indent}    const {binding_name} = {raw_expression};",
                indent = indent,
                binding_name = amount_arg.binding_name,
                raw_expression = amount_arg.raw_expression,
            )
        })
        .collect();
    let raw_assignments: Vec<String> = entry
        .semantic_amount_args
        .iter()
        .map(|amount_arg| {
            format!(
                "{indent}      {}: {},",
                render_ts_property_name_literal(&amount_arg.arg_name),
                amount_arg.binding_name,
                indent = indent,
            )
        })
        .collect();

    Some(format!(
        "{indent}{instruction_name}: instructionOperation(async (params: {semantic_params_type}) => {{\n{raw_params_setup}\n{resolutions}\n{indent}    const instruction = buildInstruction({handler_const}, {{\n{indent}      ...rawParams,\n{assignments}\n{indent}    }} as unknown as Record<string, unknown>, build);\n{indent}    return createPreparedInstruction({{\n{indent}      name: '{instruction_name}',\n{indent}      instruction,\n{indent}      artifacts: {{ instruction }},\n{indent}      errors: {handler_const}.errors,\n{indent}    }});\n{indent}  }}),",
        indent = indent,
        instruction_name = entry.instruction_name,
        semantic_params_type = semantic_params_type,
        raw_params_setup = raw_params_setup,
        resolutions = resolution_lines.join("\n"),
        handler_const = entry.handler_const,
        assignments = raw_assignments.join("\n"),
    ))
}

fn generate_program_semantic_instructions_block(
    instruction_entries: &[&crate::typescript_instructions::StackInstructionEntry],
    indent: &str,
) -> Option<String> {
    let entry_indent = format!("{}      ", indent);
    let entries: Vec<String> = instruction_entries
        .iter()
        .filter_map(|entry| render_program_semantic_instruction_entry(entry, &entry_indent))
        .collect();
    if entries.is_empty() {
        return None;
    }

    let context_param = if instruction_entries
        .iter()
        .any(|entry| entry.uses_operation_context)
    {
        "context: ProgramOperationContext"
    } else {
        ""
    };

    Some(format!(
        "{indent}[PROGRAM_OPERATION_EXTENSIONS]: {{\n{indent}  createOperations({context_param}) {{\n{indent}    return {{\n{indent}      instructions: {{\n{entries}\n{indent}      }},\n{indent}    }};\n{indent}  }},\n{indent}}},",
        indent = indent,
        context_param = context_param,
        entries = entries.join("\n"),
    ))
}

fn generate_view_helpers_static() -> String {
    r#"// ============================================================================
// View Definition Types (framework-agnostic)
// ============================================================================

export type ViewKeyFields<TKey> = unknown extends TKey
  ? readonly string[]
  : TKey extends object
    ? readonly Extract<keyof TKey, string>[]
    : readonly string[];

/** View definition with embedded entity and state-key types */
export interface ViewDef<T, TMode extends 'state' | 'list', TKey = unknown> {
  readonly mode: TMode;
  readonly view: string;
  readonly keyFields?: ViewKeyFields<TKey>;
  /** Phantom field for type inference - not present at runtime */
  readonly _entity?: T;
  readonly _key?: TKey;
}

/** Helper to create typed state view definitions (keyed lookups) */
function stateView<T, TKey = unknown>(
  view: string,
  keyFields: ViewKeyFields<TKey>
): ViewDef<T, 'state', TKey> {
  return { mode: 'state', view, keyFields } as const;
}

/** Helper to create typed list view definitions (collections) */
function listView<T>(view: string): ViewDef<T, 'list'> {
  return { mode: 'list', view } as const;
}"#
    .to_string()
}

/// Convert PascalCase to SCREAMING_SNAKE_CASE (e.g., "OreStream" -> "ORE_STREAM")
pub(crate) fn to_screaming_snake_case(s: &str) -> String {
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

    fn demo_program_spec() -> arete_hash::ProgramSpecV1 {
        arete_hash::build_program_spec_v1_from_bytes(
            br#"{
              "address":"Prog111",
              "version":"0.1.0",
              "name":"demo",
              "instructions":[],
              "accounts":[],
              "types":[],
              "events":[],
              "errors":[]
            }"#,
            None,
        )
        .expect("test ProgramSpecV1")
    }

    fn named_program_spec(name: &str, program_id: &str) -> arete_hash::ProgramSpecV1 {
        arete_hash::build_program_spec_v1_from_bytes(
            format!(
                r#"{{
                  "address":"{program_id}",
                  "version":"0.1.0",
                  "name":"{name}",
                  "instructions":[],
                  "accounts":[],
                  "types":[],
                  "events":[],
                  "errors":[]
                }}"#
            )
            .as_bytes(),
            None,
        )
        .expect("named test ProgramSpecV1")
    }

    fn two_program_test_spec() -> SerializableStackSpec {
        let specs = vec![
            named_program_spec("second_program", "Program222"),
            named_program_spec("first_program", "Program111"),
        ];
        SerializableStackSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            stack_name: "OrderedStream".to_string(),
            program_ids: specs.iter().map(|spec| spec.program_id.clone()).collect(),
            idls: specs
                .iter()
                .map(|spec| spec.idl_snapshot.snapshot.clone())
                .collect(),
            program_specs: specs,
            entities: vec![],
            pdas: BTreeMap::new(),
            instructions: vec![],
            content_hash: None,
        }
    }

    fn program_only_test_spec(
        pdas: BTreeMap<String, BTreeMap<String, PdaDefinition>>,
        instructions: Vec<InstructionDef>,
    ) -> SerializableStackSpec {
        let program_spec = demo_program_spec();
        SerializableStackSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            stack_name: "DemoStream".to_string(),
            program_ids: vec!["Prog111".to_string()],
            idls: vec![program_spec.idl_snapshot.snapshot.clone()],
            program_specs: vec![program_spec],
            entities: vec![],
            pdas,
            instructions,
            content_hash: None,
        }
    }

    fn test_instruction(amount_hint: Option<InstructionAmountHint>) -> InstructionDef {
        InstructionDef {
            name: "deposit".to_string(),
            discriminator: vec![9],
            discriminator_size: 1,
            accounts: vec![],
            args: amount_hint
                .map(|amount_hint| InstructionArgDef {
                    name: "amount".to_string(),
                    arg_type: "u64".to_string(),
                    docs: vec![],
                    amount_hint: Some(amount_hint),
                })
                .into_iter()
                .collect(),
            errors: vec![],
            program_id: Some("Prog111".to_string()),
            docs: vec![],
        }
    }

    fn state_key_test_spec(primary_keys: Vec<&str>) -> SerializableStreamSpec {
        let round_id = FieldTypeInfo::new("round_id".to_string(), "u64".to_string());
        SerializableStreamSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            state_name: "OreRound".to_string(),
            program_id: None,
            idl: None,
            identity: IdentitySpec {
                primary_keys: primary_keys.into_iter().map(str::to_string).collect(),
                lookup_indexes: vec![],
            },
            handlers: vec![],
            sections: vec![EntitySection {
                name: "id".to_string(),
                fields: vec![round_id.clone()],
                is_nested_struct: false,
                parent_field: None,
            }],
            field_mappings: BTreeMap::from([("id.round_id".to_string(), round_id)]),
            resolver_hooks: vec![],
            instruction_hooks: vec![],
            resolver_specs: vec![],
            computed_fields: vec![],
            computed_field_specs: vec![],
            content_hash: None,
            views: vec![],
        }
    }

    fn endpoint_test_spec() -> SerializableStackSpec {
        SerializableStackSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            stack_name: "EndpointStream".to_string(),
            program_ids: vec![],
            idls: vec![],
            program_specs: vec![],
            entities: vec![state_key_test_spec(vec!["id.round_id"])],
            pdas: BTreeMap::new(),
            instructions: vec![],
            content_hash: None,
        }
    }

    #[test]
    fn test_case_conversions() {
        assert_eq!(to_pascal_case("settlement_game"), "SettlementGame");
        assert_eq!(
            to_pascal_case("sb_on_demand::actions::Submission"),
            "SbOnDemandActionsSubmission"
        );
        assert_eq!(to_kebab_case("SettlementGame"), "settlement-game");
    }

    #[test]
    fn idl_field_normalization_disambiguates_leading_underscore_collisions() {
        let fields = vec![
            IdlFieldSnapshot {
                name: "_padding_0".to_string(),
                type_: IdlTypeSnapshot::Simple("u8".to_string()),
                amount_hint: None,
            },
            IdlFieldSnapshot {
                name: "padding_0".to_string(),
                type_: IdlTypeSnapshot::Simple("u8".to_string()),
                amount_hint: None,
            },
        ];

        let normalized = normalize_idl_fields(&fields, &BTreeMap::new());
        assert_eq!(
            normalized
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["_padding0", "padding0"]
        );

        let interface =
            generate_interface_from_idl_fields("CollisionLayout", &fields, &BTreeMap::new());
        assert!(interface.contains("_padding0: number;"));
        assert!(interface.contains("padding0: number;"));

        let schema = generate_schema_from_idl_fields("CollisionLayout", &fields, &BTreeMap::new());
        assert!(schema.contains("_padding_0: z.number(),"));
        assert!(schema.contains("padding_0: z.number(),"));
        assert!(schema.contains("_padding0: value._padding_0,"));
        assert!(schema.contains("padding0: value.padding_0,"));
    }

    #[test]
    fn local_stack_codegen_is_endpointless_by_default() {
        let output = compile_stack_spec(endpoint_test_spec(), None)
            .expect("local stack generation should succeed");

        assert!(output.stack_definition.contains(
            "  endpoints: {\n    ws: '', // TODO: Set after first deployment or pass useArete(..., { url })\n    http: '', // TODO: Set after first deployment or pass useArete(..., { httpUrl })\n  },"
        ));
    }

    #[test]
    fn stack_codegen_emits_independent_endpoints_exactly() {
        let websocket_url = "wss://stream.example.test/ws/v2?tenant=endpoint";
        let http_url = "https://reads.unrelated.test/api/arete/v3";
        let output = compile_stack_spec(
            endpoint_test_spec(),
            Some(TypeScriptStackConfig {
                websocket_url: Some(websocket_url.to_string()),
                http_url: Some(http_url.to_string()),
                ..TypeScriptStackConfig::default()
            }),
        )
        .expect("configured stack generation should succeed");

        assert!(output.stack_definition.contains(&format!(
            "  endpoints: {{\n    ws: '{}',\n    http: '{}',\n  }},",
            websocket_url, http_url
        )));
        assert!(!output
            .stack_definition
            .contains("https://stream.example.test/ws/v2"));
    }

    #[test]
    fn explicit_local_websocket_does_not_derive_http() {
        let output = compile_stack_spec(
            endpoint_test_spec(),
            Some(TypeScriptStackConfig {
                websocket_url: Some("ws://127.0.0.1:8878/socket".to_string()),
                ..TypeScriptStackConfig::default()
            }),
        )
        .expect("configured local stack generation should succeed");

        assert!(output
            .stack_definition
            .contains("    ws: 'ws://127.0.0.1:8878/socket',"));
        assert!(output.stack_definition.contains(
            "    http: '', // TODO: Set after first deployment or pass useArete(..., { httpUrl })"
        ));
        assert!(!output
            .stack_definition
            .contains("http://127.0.0.1:8878/socket"));
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
    fn test_typescript_scalar_array_element() {
        assert_eq!(
            typescript_scalar_array_element("Vec < f64 >"),
            Some("number")
        );
        assert_eq!(typescript_scalar_array_element("Vec<f32>"), Some("number"));
        assert_eq!(typescript_scalar_array_element("f64"), Some("number"));
        assert_eq!(
            typescript_scalar_array_element("Vec < bool >"),
            Some("boolean")
        );
        assert_eq!(
            typescript_scalar_array_element("Vec < String >"),
            Some("string")
        );
        assert_eq!(typescript_scalar_array_element("Vec < u64 >"), None);
        assert_eq!(typescript_scalar_array_element("Vec < Pubkey >"), None);
    }

    #[test]
    fn state_view_codegen_emits_exact_key_type_and_deduped_runtime_metadata() {
        let output = compile_serializable_spec(
            state_key_test_spec(vec!["id.round_id", "id.round_id"]),
            "OreRound".to_string(),
            None,
        )
        .expect("duplicate identical keys should compile");

        assert!(output.stack_definition.contains(
            "export interface ViewDef<T, TMode extends 'state' | 'list', TKey = unknown>"
        ));
        assert!(output.stack_definition.contains(
            "state: stateView<OreRound, { roundId: bigint }>('OreRound/state', ['roundId'])"
        ));
        assert_eq!(output.stack_definition.matches("['roundId']").count(), 1);
    }

    #[test]
    fn state_view_codegen_rejects_distinct_composite_keys() {
        let mut spec = state_key_test_spec(vec!["id.round_id", "id.authority"]);
        spec.field_mappings.insert(
            "id.authority".to_string(),
            FieldTypeInfo::new("authority".to_string(), "String".to_string()),
        );

        let error = compile_serializable_spec(spec, "OreRound".to_string(), None)
            .expect_err("distinct composite keys must fail generation");

        assert!(error.contains("does not support composite state keys"));
        assert!(error.contains("id.round_id, id.authority"));
    }

    #[test]
    fn pda_imports_follow_emitted_seed_variants() {
        let empty_seed_pdas = BTreeMap::from([(
            "demo".to_string(),
            BTreeMap::from([(
                "singleton".to_string(),
                PdaDefinition {
                    name: "singleton".to_string(),
                    seeds: vec![],
                    program_id: None,
                    program: None,
                },
            )]),
        )]);
        let output = compile_program_modules(program_only_test_spec(empty_seed_pdas, vec![]), None)
            .expect("empty-seed PDA generation should succeed");
        assert_eq!(
            output.imports,
            "import { z } from 'zod';\nimport { pda } from '@usearete/sdk';"
        );
        assert!(output
            .stack_definition
            .contains("singleton: pda('Prog111'),"));
        assert!(!output.stack_definition.contains("pda('Prog111', )"));

        let literal_account_pdas = BTreeMap::from([(
            "demo".to_string(),
            BTreeMap::from([(
                "vault".to_string(),
                PdaDefinition {
                    name: "vault".to_string(),
                    seeds: vec![
                        PdaSeedDef::Literal {
                            value: "vault".to_string(),
                        },
                        PdaSeedDef::AccountRef {
                            account_name: "authority".to_string(),
                        },
                    ],
                    program_id: None,
                    program: None,
                },
            )]),
        )]);
        let output =
            compile_program_modules(program_only_test_spec(literal_account_pdas, vec![]), None)
                .expect("literal/account PDA generation should succeed");
        assert_eq!(
            output.imports,
            "import { z } from 'zod';\nimport { pda, literal, account } from '@usearete/sdk';"
        );

        let arg_bytes_pdas = BTreeMap::from([(
            "demo".to_string(),
            BTreeMap::from([(
                "position".to_string(),
                PdaDefinition {
                    name: "position".to_string(),
                    seeds: vec![
                        PdaSeedDef::ArgRef {
                            arg_name: "roundId".to_string(),
                            arg_type: Some("u64".to_string()),
                        },
                        PdaSeedDef::Bytes {
                            value: vec![0, 255],
                        },
                    ],
                    program_id: None,
                    program: None,
                },
            )]),
        )]);
        let output = compile_program_modules(program_only_test_spec(arg_bytes_pdas, vec![]), None)
            .expect("arg/bytes PDA generation should succeed");
        assert_eq!(
            output.imports,
            "import { z } from 'zod';\nimport { pda, arg, bytes } from '@usearete/sdk';"
        );
    }

    fn emitted_definition_hash(output: &TypeScriptStackOutput) -> &str {
        output
            .stack_definition
            .lines()
            .find(|line| line.trim_start().starts_with("sdkDefinitionHash:"))
            .unwrap()
            .trim()
    }

    #[test]
    fn program_content_identity_tracks_generated_pda_behavior_with_the_same_input_hash() {
        let spec = program_only_test_spec(BTreeMap::new(), vec![]);
        let initial = compile_program_modules(spec.clone(), None).unwrap();
        let mut changed = spec.clone();
        changed.pdas.insert(
            "demo".into(),
            BTreeMap::from([(
                "position".into(),
                PdaDefinition {
                    name: "position".into(),
                    seeds: vec![PdaSeedDef::Bytes {
                        value: vec![1, 2, 3],
                    }],
                    program_id: None,
                    program: None,
                },
            )]),
        );
        assert_eq!(
            spec.program_specs[0].hash().unwrap(),
            changed.program_specs[0].hash().unwrap()
        );
        let changed = compile_program_modules(changed, None).unwrap();
        assert_ne!(
            emitted_definition_hash(&initial),
            emitted_definition_hash(&changed)
        );
        assert_eq!(
            emitted_definition_hash(&initial),
            emitted_definition_hash(&compile_program_modules(spec, None).unwrap())
        );
    }

    #[test]
    fn program_modules_recompute_content_identity_instead_of_trusting_input() {
        let stack_spec = program_only_test_spec(BTreeMap::new(), vec![]);
        let mut program = TypeScriptProgramConfig::from(
            &arete_hash::OssProgramIdentityV1::new(stack_spec.program_specs[0].clone()).unwrap(),
        );
        let initial = compile_program_modules(stack_spec.clone(), None).unwrap();
        program.definition.sdk_definition_hash = Some("definition-v1".to_string());
        program.release.program_release_hash =
            "arete:h1:program-release:sha256:different-release".into();
        let output = compile_program_modules(
            stack_spec,
            Some(TypeScriptStackConfig {
                programs: Some(vec![program]),
                ..TypeScriptStackConfig::default()
            }),
        )
        .expect("program definition generation should succeed");

        assert_eq!(
            emitted_definition_hash(&initial),
            emitted_definition_hash(&output)
        );
        assert_ne!(initial.full_file(), output.full_file());
        assert!(!output.stack_definition.contains("definition-v1"));
        assert!(output
            .stack_definition
            .contains("sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:"));
    }

    #[test]
    fn program_modules_separate_portable_definition_from_release_reference() {
        let mut stack_spec = program_only_test_spec(BTreeMap::new(), vec![]);
        let mut local = TypeScriptProgramConfig::from(
            &arete_hash::OssProgramIdentityV1::new(stack_spec.program_specs[0].clone()).unwrap(),
        );
        local.definition.sdk_definition_hash = Some("portable-definition".to_string());
        let hosted_release = "arete:h1:program-release:sha256:hosted";
        let output = compile_program_modules(
            stack_spec.clone(),
            Some(TypeScriptStackConfig {
                programs: Some(vec![TypeScriptProgramConfig {
                    release: TypeScriptProgramReleaseReference {
                        program_release_hash: hosted_release.to_string(),
                        program_spec_hash: local.definition.program_spec_hash.clone(),
                    },
                    ..local.clone()
                }]),
                ..TypeScriptStackConfig::default()
            }),
        )
        .expect("program definition generation should succeed");
        let definition = output.stack_definition;

        assert!(definition.contains("sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:"));
        assert!(definition.contains(&format!(
            "programSpecHash: '{}',",
            local.definition.program_spec_hash
        )));
        let programs = definition
            .split("/** Release and explicit read transport")
            .next()
            .unwrap();
        assert!(!programs.contains("programReleaseHash"));
        assert!(!programs.contains("decoderEngineId"));
        assert!(definition.contains(&format!("programReleaseHash: \"{hosted_release}\"")));
        assert!(!definition.contains("decoderEngineId"));
        stack_spec.program_specs.clear();
        assert!(compile_program_modules(stack_spec, None)
            .unwrap_err()
            .contains("Rebuild the ProgramSpec and StackManifest artifact closure"));
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
        let output =
            compile_program_modules(spec, None).expect("typescript program SDK generation");
        // Instruction handlers (discriminators, account metas, arg schemas) are
        // emitted with the interfaces; the stack definition only wires them up.
        let code = output.interfaces;

        for tag in 0u8..5 {
            assert!(
                code.contains(&format!("discriminator: [{tag}, 0, 0, 0],")),
                "instruction tag {tag} must be a u32-LE bincode discriminator:\n{code}"
            );
        }
        assert!(
            code.contains("vecU64Len"),
            "extend_lookup_table must keep its u64 vector length prefix:\n{code}"
        );
        assert!(
            output.pda_degradations.is_empty(),
            "no PDA should degrade: {:?}",
            output.warnings
        );

        let lookup_table_accounts: Vec<&str> = code
            .lines()
            .filter(|line| {
                line.contains("category: '")
                    && line
                        .to_ascii_lowercase()
                        .replace('_', "")
                        .contains("name: 'lookuptable'")
            })
            .collect();
        assert_eq!(lookup_table_accounts.len(), 5, "{code}");
        let derived = lookup_table_accounts
            .iter()
            .filter(|line| line.contains("category: 'pda', pdaConfig:"))
            .count();
        let caller_provided = lookup_table_accounts
            .iter()
            .filter(|line| line.contains("category: 'userProvided'"))
            .count();
        assert_eq!((derived, caller_provided), (1, 4), "{code}");
    }

    #[test]
    fn program_account_codegen_is_semantic_and_release_lives_in_program_reads() {
        let identity = crate::program_sdk::build_oss_program_identity_v1_from_idl_bytes(
            include_bytes!("../../arete-macros/tests/fixtures/nested-computed.idl.json"),
            None,
        )
        .expect("fixture identity");
        let stack_spec =
            crate::program_sdk::build_program_only_stack_spec_from_identity(&identity, "Presale");
        let output = compile_program_modules(stack_spec, None).expect("program SDK generation");

        assert!(output.stack_definition.contains(
            "programAccountRead<Presale>({ account: 'Presale', schema: PresaleSchema })"
        ));
        assert!(!output.stack_definition.contains("path:"));
        assert!(output.stack_definition.contains(&format!(
            "programReleaseHash: \"{}\"",
            identity.release_hash
        )));
        assert!(output
            .stack_definition
            .contains("transport: { kind: 'local-http', endpointSource: 'connect-http-url' }"));
        assert!(!output.stack_definition.contains("path:"));
    }

    #[test]
    fn legacy_account_reader_ast_without_exact_program_specs_fails_closed() {
        let identity = crate::program_sdk::build_oss_program_identity_v1_from_idl_bytes(
            include_bytes!("../../arete-macros/tests/fixtures/nested-computed.idl.json"),
            None,
        )
        .expect("fixture identity");
        let mut stack_spec =
            crate::program_sdk::build_program_only_stack_spec_from_identity(&identity, "Presale");
        assert!(!stack_spec.idls[0].accounts.is_empty());
        stack_spec.program_specs.clear();

        let error = compile_program_modules(stack_spec, None).unwrap_err();
        assert!(error.contains("no exact public ProgramSpecV1 values"));
        assert!(error.contains("Rebuild the ProgramSpec and StackManifest artifact closure"));
    }

    #[test]
    fn hosted_program_configs_preserve_order_releases_endpoints_and_auth() {
        let stack_spec = two_program_test_spec();
        let programs = stack_spec
            .program_specs
            .iter()
            .enumerate()
            .map(|(index, spec)| {
                let identity = arete_hash::OssProgramIdentityV1::new(spec.clone()).unwrap();
                let mut config = TypeScriptProgramConfig::from(&identity);
                config.release.program_release_hash = format!("hosted-release-{index}");
                let binding_id = format!("prb_{index:032}");
                config.transport =
                    TypeScriptProgramReadTransport::HostedBinding(TypeScriptProgramReadBinding {
                        endpoint: format!("https://reads.example.test/exact/{index}/"),
                        program_read_binding_id: binding_id.clone(),
                        auth: serde_json::json!({
                            "targetKind": "program-read-binding",
                            "targetId": binding_id,
                            "sessionEndpoint": format!("https://auth.example.test/{index}"),
                            "index": index
                        }),
                    });
                config
            })
            .collect::<Vec<_>>();

        let output = compile_stack_spec(
            stack_spec,
            Some(TypeScriptStackConfig {
                programs: Some(programs),
                ..Default::default()
            }),
        )
        .expect("ordered hosted descriptors should compile");
        let generated = output.stack_definition;
        let portable = generated
            .split("  programs: {")
            .nth(1)
            .expect("portable programs block")
            .split("  programReads: {")
            .next()
            .expect("portable programs block end");
        let reads = generated
            .split("  programReads: {")
            .nth(1)
            .expect("parallel program reads block");

        assert!(portable.find("secondProgram:").unwrap() < portable.find("firstProgram:").unwrap());
        assert!(!portable.contains("programReleaseHash"));
        assert!(!portable.contains("endpoint"));
        assert!(reads.find("secondProgram:").unwrap() < reads.find("firstProgram:").unwrap());
        assert!(reads.contains("programReleaseHash: \"hosted-release-0\""));
        assert!(reads.contains("programReleaseHash: \"hosted-release-1\""));
        assert!(reads.contains("kind: 'hosted-binding'"));
        assert!(reads.contains("endpoint: \"https://reads.example.test/exact/0/\""));
        assert!(reads.contains("programReadBindingId: \"prb_00000000000000000000000000000001\""));
        assert!(reads.contains(
            "auth: {\"index\":0,\"sessionEndpoint\":\"https://auth.example.test/0\",\"targetId\":\"prb_00000000000000000000000000000000\",\"targetKind\":\"program-read-binding\"}"
        ));
    }

    #[test]
    fn hosted_program_config_mismatches_fail_by_index_without_name_fallback() {
        let stack_spec = two_program_test_spec();
        let local = stack_spec
            .program_specs
            .iter()
            .map(|spec| {
                TypeScriptProgramConfig::from(
                    &arete_hash::OssProgramIdentityV1::new(spec.clone()).unwrap(),
                )
            })
            .collect::<Vec<_>>();

        let count_error = compile_program_modules(
            stack_spec.clone(),
            Some(TypeScriptStackConfig {
                programs: Some(vec![local[0].clone()]),
                ..Default::default()
            }),
        )
        .unwrap_err();
        assert!(count_error.contains("descriptor count mismatch"));

        let mut swapped = local.clone();
        swapped.swap(0, 1);
        let order_error = compile_program_modules(
            stack_spec.clone(),
            Some(TypeScriptStackConfig {
                programs: Some(swapped),
                ..Default::default()
            }),
        )
        .unwrap_err();
        assert!(order_error.contains("program ID mismatch at index 0"));

        let mut bad_hash = local;
        bad_hash[1].definition.program_spec_hash = "wrong-spec".to_string();
        let hash_error = compile_program_modules(
            stack_spec,
            Some(TypeScriptStackConfig {
                programs: Some(bad_hash),
                ..Default::default()
            }),
        )
        .unwrap_err();
        assert!(hash_error.contains("programSpecHash mismatch at index 1"));
    }

    #[test]
    fn raw_operations_omit_semantic_only_types_and_context() {
        let output = compile_program_modules(
            program_only_test_spec(BTreeMap::new(), vec![test_instruction(None)]),
            None,
        )
        .expect("raw operation generation should succeed");
        let file = output.full_file();

        assert!(!output.imports.contains("BuildOptions"));
        assert!(!output.imports.contains("ProgramOperationContext"));
        assert!(file.contains("createOperations()"));
        assert!(!file.contains("build?: BuildOptions;"));
    }

    #[test]
    fn context_free_amount_operations_import_build_options_only() {
        let output = compile_program_modules(
            program_only_test_spec(
                BTreeMap::new(),
                vec![test_instruction(Some(InstructionAmountHint {
                    decimals_source: AmountDecimalsSource::Constant { decimals: 9 },
                }))],
            ),
            None,
        )
        .expect("constant amount operation generation should succeed");
        let file = output.full_file();

        assert!(output.imports.contains("type BuildOptions"));
        assert!(output.imports.contains("toRawAmount"));
        assert!(!output.imports.contains("ProgramOperationContext"));
        assert!(file.contains("build?: BuildOptions;"));
        assert!(file.contains("createOperations()"));
        assert!(!file.contains("context.chain"));
    }

    #[test]
    fn streamed_entity_codegen_normalizes_canonical_names_and_schemas() {
        let spec = SerializableStreamSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            state_name: "TokenPosition".to_string(),
            program_id: None,
            idl: None,
            identity: IdentitySpec {
                primary_keys: vec!["id.address".to_string()],
                lookup_indexes: vec![],
            },
            handlers: vec![],
            sections: vec![
                EntitySection {
                    name: "root".to_string(),
                    fields: vec![FieldTypeInfo::new(
                        "total_deposit".to_string(),
                        "u64".to_string(),
                    )],
                    is_nested_struct: false,
                    parent_field: None,
                },
                EntitySection {
                    name: "metrics".to_string(),
                    fields: vec![FieldTypeInfo::new(
                        "last_updated_at".to_string(),
                        "i64".to_string(),
                    )],
                    is_nested_struct: false,
                    parent_field: None,
                },
            ],
            field_mappings: BTreeMap::new(),
            resolver_hooks: vec![],
            resolver_specs: vec![],
            instruction_hooks: vec![],
            computed_fields: vec![],
            computed_field_specs: vec![],
            content_hash: None,
            views: vec![],
        };

        let output = compile_serializable_spec(spec, "TokenPosition".to_string(), None)
            .expect("should compile");
        let file = output.full_file();

        assert!(
            file.contains("export interface TokenPosition {"),
            "missing main interface:\n{}",
            file
        );
        assert!(
            file.contains("totalDeposit: bigint;"),
            "missing root canonical field:\n{}",
            file
        );
        assert!(
            file.contains("metrics: TokenPositionMetrics;"),
            "missing nested canonical section field:\n{}",
            file
        );
        assert!(
            file.contains("export interface TokenPositionMetrics {"),
            "missing nested interface:\n{}",
            file
        );
        assert!(
            file.contains("lastUpdatedAt: bigint;"),
            "missing nested canonical field:\n{}",
            file
        );

        let bigint_schema = bigint_zod();
        assert!(
            file.contains("export const TokenPositionSchema = z.object({"),
            "missing main schema:\n{}",
            file
        );
        assert!(
            file.contains(&format!("total_deposit: {},", bigint_schema)),
            "missing raw root schema field:\n{}",
            file
        );
        assert!(
            file.contains("metrics: TokenPositionMetricsSchema,"),
            "missing nested schema ref:\n{}",
            file
        );
        assert!(
            file.contains("totalDeposit: value.total_deposit,"),
            "missing root transform:\n{}",
            file
        );
        assert!(
            file.contains("metrics: value.metrics,"),
            "missing nested transform:\n{}",
            file
        );

        assert!(
            file.contains("export const TokenPositionMetricsSchema = z.object({"),
            "missing nested schema:\n{}",
            file
        );
        assert!(
            file.contains(&format!("last_updated_at: {},", bigint_schema)),
            "missing raw nested schema field:\n{}",
            file
        );
        assert!(
            file.contains("lastUpdatedAt: value.last_updated_at,"),
            "missing nested transform:\n{}",
            file
        );

        assert!(
            file.contains("export const TokenPositionPatchSchema = z.object({"),
            "missing patch schema:\n{}",
            file
        );
        assert!(
            file.contains(&format!("total_deposit: {}.optional(),", bigint_schema)),
            "missing sparse patch field:\n{}",
            file
        );
        assert!(
            file.contains("metrics: TokenPositionMetricsPatchSchema.optional(),"),
            "missing nested patch schema ref:\n{}",
            file
        );
        assert!(
            file.contains("...(value.total_deposit !== undefined ? { totalDeposit: value.total_deposit } : {}),"),
            "missing sparse patch transform:\n{}",
            file
        );
        assert!(
            file.contains("export const TokenPositionMetricsPatchSchema = z.object({"),
            "missing nested patch schema:\n{}",
            file
        );
        assert!(
            file.contains("patchSchemas: {\n    TokenPosition: TokenPositionPatchSchema,"),
            "missing stack patch schema map:\n{}",
            file
        );
    }

    #[test]
    fn captured_accounts_keep_the_runtime_envelope_and_full_inner_schema() {
        let miner_snapshot = FieldTypeInfo {
            field_name: "miner_snapshot".to_string(),
            raw_name: Some("miner_snapshot".to_string()),
            canonical_name: Some("minerSnapshot".to_string()),
            rust_type_name: "Option<Miner>".to_string(),
            base_type: BaseType::Object,
            integer_kind: None,
            is_optional: true,
            is_array: false,
            inner_type: Some("Miner".to_string()),
            source_path: None,
            resolved_type: Some(ResolvedStructType {
                type_name: "Miner".to_string(),
                fields: vec![
                    ResolvedField {
                        field_name: "deployed".to_string(),
                        raw_name: Some("deployed".to_string()),
                        canonical_name: Some("deployed".to_string()),
                        field_type: "u64".to_string(),
                        base_type: BaseType::Integer,
                        integer_kind: Some(IntegerKind::U64),
                        is_optional: false,
                        is_array: true,
                    },
                    ResolvedField {
                        field_name: "round_id".to_string(),
                        raw_name: Some("round_id".to_string()),
                        canonical_name: Some("roundId".to_string()),
                        field_type: "u64".to_string(),
                        base_type: BaseType::Integer,
                        integer_kind: Some(IntegerKind::U64),
                        is_optional: false,
                        is_array: false,
                    },
                ],
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
            state_name: "OreMiner".to_string(),
            program_id: None,
            idl: None,
            identity: IdentitySpec {
                primary_keys: vec!["id.authority".to_string()],
                lookup_indexes: vec![],
            },
            handlers: vec![SerializableHandlerSpec {
                source: SourceSpec::Source {
                    program_id: None,
                    discriminator: None,
                    type_name: "Miner".to_string(),
                    serialization: None,
                    is_account: true,
                },
                key_resolution: KeyResolutionStrategy::Embedded {
                    primary_field: FieldPath::new(&["authority"]),
                },
                mappings: vec![SerializableFieldMapping {
                    target_path: "miner_snapshot".to_string(),
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
            }],
            sections: vec![
                EntitySection {
                    name: "id".to_string(),
                    fields: vec![FieldTypeInfo::new(
                        "authority".to_string(),
                        "String".to_string(),
                    )],
                    is_nested_struct: false,
                    parent_field: None,
                },
                EntitySection {
                    name: "root".to_string(),
                    fields: vec![miner_snapshot],
                    is_nested_struct: false,
                    parent_field: None,
                },
            ],
            field_mappings: BTreeMap::new(),
            resolver_hooks: vec![],
            resolver_specs: vec![],
            instruction_hooks: vec![],
            computed_fields: vec![],
            computed_field_specs: vec![],
            content_hash: None,
            views: vec![],
        };

        let output = compile_serializable_spec(spec, "OreMiner".to_string(), None)
            .expect("captured account generation should succeed");
        let file = output.full_file();

        assert!(file.contains("minerSnapshot: CaptureWrapper<Miner> | null;"));
        assert!(file.contains("export interface CaptureWrapper<T> {"));
        assert!(file.contains("accountAddress: string;"));
        assert!(file.contains("account_address: z.string(),"));
        assert!(file.contains("accountAddress: value.account_address,"));
        assert!(file
            .contains("miner_snapshot: CaptureWrapperSchema(MinerSchema).nullable().optional(),"));
        assert!(file
            .contains("miner_snapshot: CaptureWrapperSchema(MinerSchema).nullable().optional(),"));
        assert!(!file.contains("CaptureWrapperSchema(MinerPatchSchema)"));
    }

    #[test]
    fn streamed_builtin_token_metadata_codegen_is_canonical_and_sparse() {
        let spec = SerializableStreamSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            state_name: "TokenHolder".to_string(),
            program_id: None,
            idl: None,
            identity: IdentitySpec {
                primary_keys: vec!["id.address".to_string()],
                lookup_indexes: vec![],
            },
            handlers: vec![],
            sections: vec![EntitySection {
                name: "root".to_string(),
                fields: vec![FieldTypeInfo {
                    field_name: "base_token_metadata".to_string(),
                    raw_name: Some("base_token_metadata".to_string()),
                    canonical_name: Some("baseTokenMetadata".to_string()),
                    rust_type_name: "Option<TokenMetadata>".to_string(),
                    base_type: BaseType::Object,
                    integer_kind: None,
                    is_optional: true,
                    is_array: false,
                    inner_type: Some("TokenMetadata".to_string()),
                    source_path: None,
                    resolved_type: None,
                    emit: true,
                }],
                is_nested_struct: false,
                parent_field: None,
            }],
            field_mappings: BTreeMap::new(),
            resolver_hooks: vec![],
            resolver_specs: vec![],
            instruction_hooks: vec![],
            computed_fields: vec![],
            computed_field_specs: vec![],
            content_hash: None,
            views: vec![],
        };

        let output = compile_serializable_spec(spec, "TokenHolder".to_string(), None)
            .expect("should compile");
        let file = output.full_file();

        assert!(
            file.contains("export interface TokenMetadata {"),
            "missing builtin interface:\n{}",
            file
        );
        assert!(
            file.contains("logoUri?: string | null;"),
            "missing canonical builtin field:\n{}",
            file
        );
        assert!(
            file.contains("export const TokenMetadataSchema = z.object({"),
            "missing builtin schema:\n{}",
            file
        );
        assert!(
            file.contains("logo_uri: z.string().nullable().optional(),"),
            "missing raw builtin input field:\n{}",
            file
        );
        assert!(
            file.contains("...(value.logo_uri !== undefined ? { logoUri: value.logo_uri } : {}),"),
            "missing canonical builtin transform:\n{}",
            file
        );
        assert!(
            file.contains("export const TokenMetadataPatchSchema = z.object({"),
            "missing builtin patch schema:\n{}",
            file
        );
        assert!(
            file.contains("base_token_metadata: TokenMetadataPatchSchema.nullable().optional(),"),
            "missing patch schema usage for builtin field:\n{}",
            file
        );
    }

    #[test]
    fn streamed_section_codegen_localizes_prefixed_raw_field_names() {
        let spec = SerializableStreamSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            state_name: "OreRound".to_string(),
            program_id: None,
            idl: None,
            identity: IdentitySpec {
                primary_keys: vec!["id.round_id".to_string()],
                lookup_indexes: vec![],
            },
            handlers: vec![],
            sections: vec![EntitySection {
                name: "results".to_string(),
                fields: vec![FieldTypeInfo {
                    field_name: "results.expires_at_slot_hash".to_string(),
                    raw_name: Some("results.expires_at_slot_hash".to_string()),
                    canonical_name: Some("resultsExpiresAtSlotHash".to_string()),
                    rust_type_name: "Option<String>".to_string(),
                    base_type: BaseType::String,
                    integer_kind: None,
                    is_optional: true,
                    is_array: false,
                    inner_type: Some("String".to_string()),
                    source_path: None,
                    resolved_type: None,
                    emit: true,
                }],
                is_nested_struct: false,
                parent_field: None,
            }],
            field_mappings: BTreeMap::new(),
            resolver_hooks: vec![],
            resolver_specs: vec![],
            instruction_hooks: vec![],
            computed_fields: vec![],
            computed_field_specs: vec![],
            content_hash: None,
            views: vec![],
        };

        let output =
            compile_serializable_spec(spec, "OreRound".to_string(), None).expect("should compile");
        let file = output.full_file();

        assert!(
            file.contains("export interface OreRoundResults {"),
            "missing section interface:\n{}",
            file
        );
        assert!(
            file.contains("expiresAtSlotHash: string | null;"),
            "missing localized canonical field:\n{}",
            file
        );
        assert!(
            file.contains("expires_at_slot_hash: z.string().nullable().optional(),"),
            "missing localized canonical schema field:\n{}",
            file
        );
        assert!(
            file.contains("expiresAtSlotHash: value.expires_at_slot_hash,"),
            "missing localized transform:\n{}",
            file
        );
        assert!(
            file.contains("expires_at_slot_hash: z.string().nullable().optional(),"),
            "missing localized patch schema field:\n{}",
            file
        );
        assert!(
            file.contains("...(value.expires_at_slot_hash !== undefined ? { expiresAtSlotHash: value.expires_at_slot_hash } : {}),"),
            "missing localized sparse transform:\n{}",
            file
        );
    }

    #[test]
    fn test_streamed_completed_schema_allows_unwritten_nullable_fields() {
        let mut optional_count = FieldTypeInfo::new("count".to_string(), "u64".to_string());
        optional_count.is_optional = true;
        let spec = SerializableStreamSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            state_name: "OreRound".to_string(),
            program_id: None,
            idl: None,
            identity: IdentitySpec {
                primary_keys: vec!["id.round_id".to_string()],
                lookup_indexes: vec![],
            },
            handlers: vec![],
            sections: vec![EntitySection {
                name: "state".to_string(),
                fields: vec![optional_count],
                is_nested_struct: false,
                parent_field: None,
            }],
            field_mappings: BTreeMap::new(),
            resolver_hooks: vec![],
            resolver_specs: vec![],
            instruction_hooks: vec![],
            computed_fields: vec![],
            computed_field_specs: vec![],
            content_hash: None,
            views: vec![],
        };

        let output =
            compile_serializable_spec(spec, "OreRound".to_string(), None).expect("should compile");
        let file = output.full_file();
        assert!(
            file.contains("count: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),"),
            "completed schema should allow absent nullable fields:\n{}",
            file
        );
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

    #[test]
    fn test_account_type_collision_uses_account_suffix() {
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
            identity: IdentitySpec {
                primary_keys: vec!["id.address".to_string()],
                lookup_indexes: vec![],
            },
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
            .expect("typescript sdk generation should succeed");

        assert!(
            output.interfaces.contains("export interface PlanPlan {"),
            "expected PlanPlan section interface, got:\n{}",
            output.interfaces
        );
        assert!(
            output.interfaces.contains("plan: PlanAccount;"),
            "expected PlanAccount field reference, got:\n{}",
            output.interfaces
        );
        assert!(
            output.interfaces.contains("export interface PlanAccount {"),
            "expected PlanAccount interface, got:\n{}",
            output.interfaces
        );
    }

    #[test]
    fn test_multi_entity_enum_dedup_uses_pascal_case_name_matching() {
        let shared_idl = serde_json::json!({
            "name": "subscriptions",
            "version": "0.1.0",
            "accounts": [],
            "instructions": [],
            "types": [
                {
                    "name": "planStatus",
                    "type": {
                        "kind": "enum",
                        "variants": [{ "name": "sunset" }, { "name": "active" }]
                    }
                }
            ],
            "events": [],
            "errors": [],
            "discriminant_size": 8
        });

        let idl_snapshot: IdlSnapshot =
            serde_json::from_value(shared_idl).expect("idl snapshot should deserialize");

        let make_entity = |name: &str| SerializableStreamSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            state_name: name.to_string(),
            program_id: None,
            idl: None,
            identity: IdentitySpec {
                primary_keys: vec!["id.address".to_string()],
                lookup_indexes: vec![],
            },
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
        };

        let stack_spec = SerializableStackSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            stack_name: "Subscriptions".to_string(),
            program_ids: vec![],
            idls: vec![idl_snapshot],
            program_specs: vec![],
            entities: vec![make_entity("Plan"), make_entity("Subscription")],
            pdas: BTreeMap::new(),
            instructions: vec![],
            content_hash: None,
        };

        let output =
            compile_stack_spec(stack_spec, None).expect("stack compilation should succeed");
        let file = output.full_file();
        let count = output
            .interfaces
            .matches("export type PlanStatus =")
            .count();

        assert_eq!(
            count, 1,
            "expected shared enum type to be emitted once, got:\n{}",
            output.interfaces
        );
        assert!(
            file.contains("_STACK_CORE = {"),
            "core export missing:\n{}",
            file
        );
        assert!(
            !file.contains("extendStack"),
            "no extension wiring expected:\n{}",
            file
        );
    }

    #[test]
    fn golden_ore_stack_json_compiles_program_modules_without_entities() {
        let mut spec = crate::public_artifacts::ore_stack_spec_from_exact_artifacts();

        // Program-only emission must not depend on entities at all.
        spec.entities.clear();

        let output =
            compile_program_modules(spec, None).expect("program-module compilation should succeed");
        let file = output.full_file();

        // Standalone per-program consts plus the combined map.
        assert!(
            file.contains("export const ORE = {"),
            "ore const missing:\n{}",
            file
        );
        assert!(
            file.contains("export const ENTROPY = {"),
            "entropy const missing"
        );
        assert!(
            file.contains("export const ORE_STREAM_PROGRAMS = {"),
            "combined program map missing"
        );
        assert!(file.contains("  ore: ORE,"));
        assert!(file.contains("  entropy: ENTROPY,"));
        assert!(file.contains("export default ORE_STREAM_PROGRAMS;"));
        assert!(file.contains("All portable programs from the OreStream stack"));
        assert!(file.contains("export const ORE_STREAM_PROGRAM_READS = {"));

        // Program bodies keep the full SDK surface...
        assert!(file.contains("createInstructionHandler"));
        assert!(file.contains("pdas: {"));
        assert!(file.contains("addresses: {"));
        assert!(file.contains("instructions: {"));
        assert!(file.contains("createPreparedInstruction({"));
        assert!(file.contains("buildInstruction("));

        // ...but nothing stack- or view-shaped is emitted.
        assert!(!file.contains("stateView"), "no view helpers expected");
        assert!(!file.contains("listView"), "no view helpers expected");
        assert!(!file.contains("views:"), "no views block expected");
        assert!(!file.contains("endpoints:"), "no endpoints block expected");
        assert!(
            !file.contains("extendStack"),
            "no extension wiring expected"
        );
    }

    #[test]
    fn golden_ore_stack_json_emits_typed_state_view_keys() {
        let spec = crate::public_artifacts::ore_stack_spec_from_exact_artifacts();

        let output = compile_stack_spec(spec, None).expect("ore stack should compile");
        let stack = output.stack_definition;

        assert!(stack.contains(
            "state: stateView<OreRound, { roundId: bigint }>('OreRound/state', ['roundId'])"
        ));
        assert!(stack.contains(
            "state: stateView<OreBoard, { address: string }>('OreBoard/state', ['address'])"
        ));
        assert!(stack.contains(
            "state: stateView<OreMiner, { authority: string }>('OreMiner/state', ['authority'])"
        ));
        assert_eq!(
            stack.matches(
                "state: stateView<OreMiner, { authority: string }>('OreMiner/state', ['authority'])"
            )
            .count(),
            1
        );
    }

    #[test]
    fn compile_program_modules_emits_amount_aware_semantic_instruction_wrappers() {
        let stack_spec = SerializableStackSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            stack_name: "DemoStream".to_string(),
            program_ids: vec!["Prog111".to_string()],
            idls: vec![IdlSnapshot {
                name: "demo".to_string(),
                program_id: Some("Prog111".to_string()),
                version: "0.1.0".to_string(),
                accounts: vec![],
                instructions: vec![IdlInstructionSnapshot {
                    name: "deposit".to_string(),
                    discriminator: vec![9],
                    discriminant: None,
                    docs: vec![],
                    accounts: vec![],
                    args: vec![
                        IdlFieldSnapshot {
                            name: "amount".to_string(),
                            type_: IdlTypeSnapshot::Simple("u64".to_string()),
                            amount_hint: None,
                        },
                        IdlFieldSnapshot {
                            name: "mint".to_string(),
                            type_: IdlTypeSnapshot::Simple("publicKey".to_string()),
                            amount_hint: None,
                        },
                    ],
                }],
                types: vec![],
                events: vec![],
                errors: vec![],
                discriminant_size: 1,
            }],
            program_specs: vec![demo_program_spec()],
            entities: vec![],
            pdas: BTreeMap::new(),
            instructions: vec![InstructionDef {
                name: "deposit".to_string(),
                discriminator: vec![9],
                discriminator_size: 1,
                accounts: vec![],
                args: vec![
                    InstructionArgDef {
                        name: "amount".to_string(),
                        arg_type: "u64".to_string(),
                        docs: vec![],
                        amount_hint: Some(InstructionAmountHint {
                            decimals_source: AmountDecimalsSource::ArgMint {
                                arg_name: "mint".to_string(),
                            },
                        }),
                    },
                    InstructionArgDef {
                        name: "mint".to_string(),
                        arg_type: "solana_pubkey::Pubkey".to_string(),
                        docs: vec![],
                        amount_hint: None,
                    },
                ],
                errors: vec![],
                program_id: Some("Prog111".to_string()),
                docs: vec![],
            }],
            content_hash: None,
        };

        let output = compile_program_modules(stack_spec, None)
            .expect("program-module compilation should succeed");
        let file = output.full_file();

        assert!(
            file.contains("type AmountInput"),
            "amount import missing:\n{}",
            file
        );
        assert!(
            file.contains("PROGRAM_OPERATION_EXTENSIONS"),
            "runtime extension import missing"
        );
        assert!(
            output.imports.contains("type BuildOptions"),
            "build options import missing"
        );
        assert!(
            output.imports.contains("type ProgramOperationContext"),
            "operation context import missing"
        );
        assert!(
            file.contains("resolveAmountToRaw"),
            "amount resolver import missing"
        );
        assert!(file.contains("export interface DepositSemanticParams"));
        assert!(file.contains("build?: BuildOptions;"));
        assert!(file.contains("[PROGRAM_OPERATION_EXTENSIONS]: {"));
        assert!(file.contains("createOperations(context: ProgramOperationContext)"));
        assert!(file
            .contains("deposit: instructionOperation(async (params: DepositSemanticParams) => {"));
        assert!(file.contains("const { build, amountDecimals, ...rawParams } = params;"));
        assert!(file.contains("resolveAmountToRaw(context.chain"));
        assert!(file.contains("const instruction = buildInstruction(depositInstruction, {"));
        assert!(file.contains("createPreparedInstruction({"));
    }

    #[test]
    fn golden_ore_stack_json_compiles_stack_with_root_helper_namespaces() {
        let spec = crate::public_artifacts::ore_stack_spec_from_exact_artifacts();

        let output = compile_stack_spec(spec, None).expect("stack compilation should succeed");
        let file = output.full_file();

        assert!(
            file.contains("endpoints:"),
            "stack endpoints missing:\n{}",
            file
        );
        assert!(
            file.contains("programs: {"),
            "program block missing:\n{}",
            file
        );
        assert!(
            file.contains("addresses: {"),
            "root addresses missing:\n{}",
            file
        );
        assert!(
            file.contains("instructions: {"),
            "program instructions missing:\n{}",
            file
        );
        assert!(
            file.contains("buildInstruction("),
            "instruction builders missing:\n{}",
            file
        );
    }

    #[test]
    fn account_codegen_normalizes_raw_keys_and_nested_types() {
        let idl_snapshot = IdlSnapshot {
            name: "presale".to_string(),
            program_id: None,
            version: "0.1.0".to_string(),
            accounts: vec![IdlAccountSnapshot {
                name: "Presale".to_string(),
                discriminator: vec![1, 2, 3, 4, 5, 6, 7, 8],
                docs: vec![],
                serialization: None,
                fields: vec![
                    IdlFieldSnapshot {
                        name: "owner".to_string(),
                        type_: IdlTypeSnapshot::Simple("pubkey".to_string()),
                        amount_hint: None,
                    },
                    IdlFieldSnapshot {
                        name: "total_deposit".to_string(),
                        type_: IdlTypeSnapshot::Simple("u64".to_string()),
                        amount_hint: None,
                    },
                    IdlFieldSnapshot {
                        name: "optional_authority".to_string(),
                        type_: IdlTypeSnapshot::Option(IdlOptionTypeSnapshot {
                            option: Box::new(IdlTypeSnapshot::Simple("pubkey".to_string())),
                        }),
                        amount_hint: None,
                    },
                    IdlFieldSnapshot {
                        name: "createKey".to_string(),
                        type_: IdlTypeSnapshot::Simple("pubkey".to_string()),
                        amount_hint: None,
                    },
                    IdlFieldSnapshot {
                        name: "member".to_string(),
                        type_: IdlTypeSnapshot::Defined(IdlDefinedTypeSnapshot {
                            defined: IdlDefinedInnerSnapshot::Simple("MemberConfig".to_string()),
                        }),
                        amount_hint: None,
                    },
                ],
                type_def: None,
            }],
            instructions: vec![],
            types: vec![IdlTypeDefSnapshot {
                name: "MemberConfig".to_string(),
                docs: vec![],
                serialization: None,
                type_def: IdlTypeDefKindSnapshot::Struct {
                    kind: "struct".to_string(),
                    fields: vec![
                        IdlFieldSnapshot {
                            name: "last_updated_at".to_string(),
                            type_: IdlTypeSnapshot::Simple("i128".to_string()),
                            amount_hint: None,
                        },
                        IdlFieldSnapshot {
                            name: "authority_key".to_string(),
                            type_: IdlTypeSnapshot::Simple("pubkey".to_string()),
                            amount_hint: None,
                        },
                    ],
                },
            }],
            events: vec![],
            errors: vec![],
            discriminant_size: 8,
        };

        let artifacts = generate_idl_account_artifacts(&[idl_snapshot], &HashSet::new());
        let account_bigint = bigint_zod();

        assert!(artifacts.code.contains("export interface Presale {"));
        assert!(
            artifacts.code.contains("owner: string;"),
            "missing owner field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts.code.contains("totalDeposit: bigint;"),
            "missing totalDeposit field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts.code.contains("optionalAuthority: string | null;"),
            "missing optionalAuthority field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts.code.contains("createKey: string;"),
            "missing createKey field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts.code.contains("member: MemberConfig;"),
            "missing nested type field:\n{}",
            artifacts.code
        );
        assert!(artifacts.code.contains("export interface MemberConfig {"));
        assert!(
            artifacts.code.contains("lastUpdatedAt: bigint;"),
            "missing nested bigint field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts.code.contains("authorityKey: string;"),
            "missing nested camelCase field:\n{}",
            artifacts.code
        );

        assert!(artifacts
            .code
            .contains("export const PresaleSchema = z.object({"));
        assert!(
            artifacts.code.contains("owner: z.string(),"),
            "missing owner schema field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts
                .code
                .contains(&format!("total_deposit: {},", account_bigint)),
            "missing total_deposit schema field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts
                .code
                .contains("optional_authority: z.string().nullable(),"),
            "missing optional_authority schema field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts.code.contains("create_key: z.string(),"),
            "missing create_key schema field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts
                .code
                .contains("member: z.lazy(() => MemberConfigSchema),"),
            "missing nested schema field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts.code.contains("owner: value.owner,"),
            "missing owner transform:\n{}",
            artifacts.code
        );
        assert!(
            artifacts
                .code
                .contains("totalDeposit: value.total_deposit,"),
            "missing totalDeposit transform:\n{}",
            artifacts.code
        );
        assert!(
            artifacts
                .code
                .contains("optionalAuthority: value.optional_authority,"),
            "missing optionalAuthority transform:\n{}",
            artifacts.code
        );
        assert!(
            artifacts.code.contains("createKey: value.create_key,"),
            "missing createKey transform:\n{}",
            artifacts.code
        );
        assert!(
            artifacts.code.contains("member: value.member,"),
            "missing nested transform:\n{}",
            artifacts.code
        );

        assert!(artifacts
            .code
            .contains("export const MemberConfigSchema = z.object({"));
        assert!(
            artifacts
                .code
                .contains(&format!("last_updated_at: {},", bigint_zod())),
            "missing nested bigint schema field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts.code.contains("authority_key: z.string(),"),
            "missing nested schema field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts
                .code
                .contains("lastUpdatedAt: value.last_updated_at,"),
            "missing nested bigint transform:\n{}",
            artifacts.code
        );
        assert!(
            artifacts
                .code
                .contains("authorityKey: value.authority_key,"),
            "missing nested camelCase transform:\n{}",
            artifacts.code
        );
    }

    #[test]
    fn account_codegen_falls_back_to_same_named_type_def_when_account_fields_are_empty() {
        let idl_snapshot = IdlSnapshot {
            name: "presale".to_string(),
            program_id: None,
            version: "0.1.0".to_string(),
            accounts: vec![IdlAccountSnapshot {
                name: "Presale".to_string(),
                discriminator: vec![1, 2, 3, 4, 5, 6, 7, 8],
                docs: vec![],
                serialization: None,
                fields: vec![],
                type_def: None,
            }],
            instructions: vec![],
            types: vec![IdlTypeDefSnapshot {
                name: "Presale".to_string(),
                docs: vec![],
                serialization: None,
                type_def: IdlTypeDefKindSnapshot::Struct {
                    kind: "struct".to_string(),
                    fields: vec![
                        IdlFieldSnapshot {
                            name: "owner".to_string(),
                            type_: IdlTypeSnapshot::Simple("pubkey".to_string()),
                            amount_hint: None,
                        },
                        IdlFieldSnapshot {
                            name: "total_deposit".to_string(),
                            type_: IdlTypeSnapshot::Simple("u64".to_string()),
                            amount_hint: None,
                        },
                    ],
                },
            }],
            events: vec![],
            errors: vec![],
            discriminant_size: 8,
        };

        let artifacts = generate_idl_account_artifacts(&[idl_snapshot], &HashSet::new());

        assert!(artifacts.code.contains("export interface Presale {"));
        assert!(
            artifacts.code.contains("owner: string;"),
            "missing owner field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts.code.contains("totalDeposit: bigint;"),
            "missing totalDeposit field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts
                .code
                .contains("export const PresaleSchema = z.object({"),
            "missing schema:\n{}",
            artifacts.code
        );
        assert!(
            artifacts.code.contains("owner: z.string(),"),
            "missing owner schema field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts
                .code
                .contains(&format!("total_deposit: {},", bigint_zod())),
            "missing total_deposit schema field:\n{}",
            artifacts.code
        );
        assert!(
            artifacts.code.contains("owner: value.owner,"),
            "missing owner transform:\n{}",
            artifacts.code
        );
        assert!(
            artifacts
                .code
                .contains("totalDeposit: value.total_deposit,"),
            "missing totalDeposit transform:\n{}",
            artifacts.code
        );
    }

    #[test]
    fn compile_program_modules_rejects_specs_without_idls() {
        let stack_spec = SerializableStackSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            stack_name: "Empty".to_string(),
            program_ids: vec![],
            idls: vec![],
            program_specs: vec![],
            entities: vec![],
            pdas: BTreeMap::new(),
            instructions: vec![],
            content_hash: None,
        };

        let error =
            compile_program_modules(stack_spec, None).expect_err("no IDLs should be an error");
        assert!(error.contains("no IDLs"), "unexpected error: {}", error);
    }
}
