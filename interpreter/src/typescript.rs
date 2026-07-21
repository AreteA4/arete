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
            let schema = if required || matches!(field.presence, FieldPresence::Required) {
                with_nullable
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
    let mut canonical_names = BTreeMap::new();
    let mut normalized = Vec::with_capacity(fields.len());

    for field in fields {
        let canonical_name = to_camel_case(&field.name);
        if let Some(existing_source) =
            canonical_names.insert(canonical_name.clone(), field.name.clone())
        {
            assert_eq!(
                existing_source, field.name,
                "IDL field normalization collision: '{}' and '{}' both normalize to '{}'",
                existing_source, field.name, canonical_name
            );
        }

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

#[derive(Debug, Clone)]
pub struct TypeScriptStackConfig {
    pub package_name: String,
    pub generate_helpers: bool,
    pub export_const_name: String,
    pub url: Option<String>,
    pub extension_import: Option<String>,
    pub definition_hash: Option<String>,
}

impl Default for TypeScriptStackConfig {
    fn default() -> Self {
        Self {
            package_name: "@usearete/react".to_string(),
            generate_helpers: true,
            export_const_name: "STACK".to_string(),
            url: None,
            extension_import: None,
            definition_hash: None,
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
        &config,
    )?;

    // 4. Assemble `@usearete/sdk` imports based on what was actually emitted.
    let imports = assemble_sdk_imports(
        collect_emitted_pda_imports(&stack_spec.idls, &stack_spec.pdas),
        !idl_account_artifacts.account_type_names.is_empty(),
        &instructions_codegen,
    );

    Ok(TypeScriptStackOutput {
        imports,
        interfaces,
        stack_definition,
        warnings: instructions_codegen.warnings,
        pda_degradations: instructions_codegen.pda_degradations,
    })
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

    let program_context = ProgramGenerationContext {
        pdas: &stack_spec.pdas,
        program_ids: &stack_spec.program_ids,
        instruction_entries: &instructions_codegen.stack_entries,
        schema_names: &unique_schemas,
        account_type_names: &idl_account_artifacts.account_type_names,
        definition_hash: config.definition_hash.as_deref(),
    };
    let stack_definition =
        generate_program_definitions(stack_name, &stack_spec.idls, &program_context);

    let imports = assemble_sdk_imports(
        collect_emitted_pda_imports(&stack_spec.idls, &stack_spec.pdas),
        !idl_account_artifacts.account_type_names.is_empty(),
        &instructions_codegen,
    );

    Ok(TypeScriptStackOutput {
        imports,
        interfaces,
        stack_definition,
        warnings: instructions_codegen.warnings,
        pda_degradations: instructions_codegen.pda_degradations,
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
    idls: &[IdlSnapshot],
    pdas: &BTreeMap<String, BTreeMap<String, PdaDefinition>>,
    program_ids: &[String],
    schema_names: &[String],
    account_type_names: &BTreeMap<(String, String), String>,
    instruction_entries: &[crate::typescript_instructions::StackInstructionEntry],
    config: &TypeScriptStackConfig,
) -> Result<String, String> {
    let export_name = format!(
        "{}_{}",
        to_screaming_snake_case(stack_name),
        config.export_const_name
    );
    let core_export_name = format!("{}_CORE", export_name);

    let view_helpers = generate_view_helpers_static();

    let endpoints_block = match &config.url {
        Some(url) => format!(
            "  endpoints: {{\n    ws: '{}',\n    http: '{}',\n  }},",
            url,
            derive_http_url(url)
        ),
        None => "  endpoints: {\n    ws: '', // TODO: Set after first deployment or pass useArete(..., { url })\n    http: '', // TODO: Set after first deployment or pass useArete(..., { httpUrl })\n  },"
            .to_string(),
    };

    // Generate views block for each entity
    let mut entity_view_blocks = Vec::new();
    for (i, entity_spec) in entities.iter().enumerate() {
        let entity_name = &entity_names[i];
        let entity_pascal = to_pascal_case(entity_name);
        let state_view_key = state_view_key_definition(
            entity_name,
            &entity_spec.identity,
            &entity_spec.field_mappings,
            &entity_spec.sections,
        )?;

        let mut view_entries = Vec::new();

        view_entries.push(format!(
            "      state: stateView<{entity}, {key_type}>('{entity_name}/state', {key_fields}),",
            entity = entity_pascal,
            entity_name = entity_name,
            key_type = state_view_key.object_type(),
            key_fields = state_view_key.fields_literal(),
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
        definition_hash: config.definition_hash.as_deref(),
    };
    let programs_block = generate_programs_block(idls, &program_context);
    let addresses_block = generate_stack_addresses_block(idls, pdas, program_ids);

    let entity_types: Vec<String> = entity_names.iter().map(|n| to_pascal_case(n)).collect();

    let stack_export = format!(
        r#"export const {core_export_name} = {{
  name: '{stack_kebab}',
{endpoints_block}
  views: {{
{views_body}
  }},{schemas_section}{patch_schemas_section}{programs_section}{addresses_section}
}} as const;"#,
        core_export_name = core_export_name,
        stack_kebab = stack_kebab,
        endpoints_block = endpoints_block,
        views_body = views_body,
        schemas_section = schemas_block,
        patch_schemas_section = patch_schemas_block,
        programs_section = programs_block,
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
        entity_union = entity_types.join(" | "),
    ))
}

struct ProgramGenerationContext<'a> {
    pdas: &'a BTreeMap<String, BTreeMap<String, PdaDefinition>>,
    program_ids: &'a [String],
    instruction_entries: &'a [crate::typescript_instructions::StackInstructionEntry],
    schema_names: &'a BTreeSet<String>,
    account_type_names: &'a BTreeMap<(String, String), String>,
    definition_hash: Option<&'a str>,
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
                "        {account_name}: programAccountRead<{type_name}>({{ account: '{account_name}', path: '/programs/{program}/accounts/{account_name}', schema: {schema_name} }}),",
                account_name = account.0,
                type_name = account.1,
                schema_name = account.2,
                program = program_key,
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
    if let Some(definition_hash) = context.definition_hash {
        sections.push(format!("      definitionHash: '{}',", definition_hash));
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
    if idls.is_empty() {
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
    }

    let map_name = format!("{}_PROGRAMS", to_screaming_snake_case(stack_name));
    let type_name = format!("{}Programs", to_pascal_case(stack_name));

    format!(
        r#"// ============================================================================
// Program Definitions
// ============================================================================

{program_consts}

    /** All programs from the {stack_name} stack, ready for createSession({{ programs: ... }}) */
export const {map_name} = {{
{map_entries}
}} as const;

export type {type_name} = typeof {map_name};

export default {map_name};"#,
        program_consts = program_consts.join("\n\n"),
        stack_name = stack_name,
        map_name = map_name,
        map_entries = map_entries.join("\n"),
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

            let pid = pda_def.program_id.as_deref().unwrap_or(default_program_id);
            let rendered_seeds = if seeds_str.is_empty() {
                String::new()
            } else {
                format!(", {}", seeds_str)
            };
            format!("{}{}: pda('{}'{}),", indent, pda_name, pid, rendered_seeds)
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

fn derive_http_url(ws_url: &str) -> String {
    if ws_url.starts_with("wss://") {
        ws_url.replacen("wss://", "https://", 1)
    } else if ws_url.starts_with("ws://") {
        ws_url.replacen("ws://", "http://", 1)
    } else {
        ws_url.to_string()
    }
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

    fn program_only_test_spec(
        pdas: BTreeMap<String, BTreeMap<String, PdaDefinition>>,
        instructions: Vec<InstructionDef>,
    ) -> SerializableStackSpec {
        SerializableStackSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            stack_name: "DemoStream".to_string(),
            program_ids: vec!["Prog111".to_string()],
            idls: vec![IdlSnapshot {
                name: "demo".to_string(),
                program_id: Some("Prog111".to_string()),
                version: "0.1.0".to_string(),
                accounts: vec![],
                instructions: vec![],
                types: vec![],
                events: vec![],
                errors: vec![],
                discriminant_size: 1,
            }],
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
    fn test_typescript_scalar_array_element() {
        assert_eq!(typescript_scalar_array_element("Vec < f64 >"), Some("number"));
        assert_eq!(typescript_scalar_array_element("Vec<f32>"), Some("number"));
        assert_eq!(typescript_scalar_array_element("f64"), Some("number"));
        assert_eq!(typescript_scalar_array_element("Vec < bool >"), Some("boolean"));
        assert_eq!(typescript_scalar_array_element("Vec < String >"), Some("string"));
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

    #[test]
    fn program_modules_emit_configured_definition_hash() {
        let output = compile_program_modules(
            program_only_test_spec(BTreeMap::new(), vec![]),
            Some(TypeScriptStackConfig {
                definition_hash: Some("definition-v1".to_string()),
                ..TypeScriptStackConfig::default()
            }),
        )
        .expect("program definition generation should succeed");

        assert!(output
            .stack_definition
            .contains("definitionHash: 'definition-v1',"));
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
        assert!(file.contains("miner_snapshot: CaptureWrapperSchema(MinerSchema).nullable(),"));
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
            file.contains("expires_at_slot_hash: z.string().nullable(),"),
            "missing localized raw schema field:\n{}",
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
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../stacks/ore/.arete/OreStream.stack.json"
        );
        let json = match std::fs::read_to_string(path) {
            Ok(c) => c,
            // Stack JSON is generated by the macro build; skip if not present.
            Err(_) => return,
        };
        let mut spec: SerializableStackSpec =
            serde_json::from_str(&json).expect("ore stack json should deserialize");

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
        assert!(file.contains("ready for createSession({ programs: ... })"));

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
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../stacks/ore/.arete/OreStream.stack.json"
        );
        let json = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            // Stack JSON is generated by the macro build; skip if not present.
            Err(_) => return,
        };
        let spec: SerializableStackSpec =
            serde_json::from_str(&json).expect("ore stack json should deserialize");

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
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../stacks/ore/.arete/OreStream.stack.json"
        );
        let json = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let spec: SerializableStackSpec =
            serde_json::from_str(&json).expect("ore stack json should deserialize");

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
