//! TypeScript codegen for instruction-construction handlers.
//!
//! This module consumes the `InstructionDef[]` serialized into a stack spec and
//! emits data-driven instruction handlers that target the core SDK's
//! [`createInstructionHandler`] factory. No imperative serialization code is
//! generated: the output is metadata (discriminator, ordered accounts, arg
//! schema, errors) plus typed `Params`/`Error` shapes the core runtime
//! interprets.
//!
//! The generated handlers are the codegen counterpart to the hand-written
//! golden fixture in `examples/subscriptions-instructions/src/handlers.ts`.

use crate::ast::{
    AccountResolution, IdlArrayElementSnapshot, IdlDefinedInnerSnapshot, IdlErrorSnapshot,
    IdlSnapshot, IdlTypeDefKindSnapshot, IdlTypeDefSnapshot, IdlTypeSnapshot,
    InstructionAccountDef, InstructionDef, PdaDefinition, PdaSeedDef,
};
use crate::typescript::{to_pascal_case, to_screaming_snake_case};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// One entry in the stack definition's `instructions` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackInstructionEntry {
    /// Program namespace key (camelCase IDL name) for multi-program stacks.
    /// `None` for single-program stacks, where the block stays flat.
    pub program_key: Option<String>,
    /// Key inside the (possibly nested) instructions block.
    pub instruction_name: String,
    /// Name of the generated handler const.
    pub handler_const: String,
}

/// Result of generating instruction handler code for a stack.
#[derive(Debug, Clone, Default)]
pub struct InstructionsCodegen {
    /// Generated TypeScript: program-error consts, per-instruction param/error
    /// types and handler consts. Empty when there are no emittable handlers.
    pub code: String,
    /// Entries to wire into the stack definition's `instructions` block.
    pub stack_entries: Vec<StackInstructionEntry>,
    /// Whether the generated code references the `@usearete/sdk` runtime
    /// (`createInstructionHandler` / `ErrorMetadata`).
    pub needs_runtime_import: bool,
    /// Human-readable warnings (skipped instructions, degraded PDAs).
    pub warnings: Vec<String>,
}

/// Per-program error const/type names plus the rendered declarations.
struct ProgramErrorScope {
    const_name: String,
    type_name: String,
    errors: Vec<IdlErrorSnapshot>,
    used: bool,
}

/// Generate instruction handler code for a stack.
///
/// `idls` are the stack's IDL snapshots; each handler's errors are scoped to
/// its own program (matched via `InstructionDef.program_id`). Single-program
/// stacks keep flat naming; multi-program stacks prefix handler/type names
/// with the program name and namespace the `instructions` block per program.
pub fn generate_instructions_code(
    stack_name: &str,
    instructions: &[InstructionDef],
    idls: &[IdlSnapshot],
    pdas: &BTreeMap<String, BTreeMap<String, PdaDefinition>>,
    program_ids: &[String],
    reserved_type_names: &HashSet<String>,
) -> InstructionsCodegen {
    if instructions.is_empty() {
        return InstructionsCodegen::default();
    }

    let multi_program = idls.len() > 1;
    let mut defined_types = DefinedTypes::new(idls, reserved_type_names);

    // Flatten the PDA registry into a name -> definition lookup. PDA names are
    // expected to be unique across programs within a stack; conflicting
    // definitions keep the first and warn.
    let mut warnings: Vec<String> = Vec::new();
    let mut pda_lookup: BTreeMap<&str, &PdaDefinition> = BTreeMap::new();
    for program_pdas in pdas.values() {
        for (name, def) in program_pdas {
            if let Some(existing) = pda_lookup.get(name.as_str()) {
                if format!("{:?}", existing) != format!("{:?}", def) {
                    warnings.push(format!(
                        "PDA '{}' is defined differently in multiple programs; using the first definition",
                        name
                    ));
                }
            } else {
                pda_lookup.insert(name.as_str(), def);
            }
        }
    }

    let default_program_id = program_ids.first().cloned().unwrap_or_default();

    let mut blocks: Vec<String> = Vec::new();
    let mut stack_entries: Vec<StackInstructionEntry> = Vec::new();

    let stack_screaming = to_screaming_snake_case(stack_name);
    let stack_pascal = to_pascal_case(stack_name);

    // Per-program error scopes. The fallback scope (stack-level naming, all
    // errors flattened) serves single-program stacks, stacks without IDL
    // snapshots, and instructions that cannot be matched to a program.
    let mut program_scopes: Vec<ProgramErrorScope> = idls
        .iter()
        .map(|idl| {
            let (const_name, type_name) = if multi_program {
                (
                    format!(
                        "{}_{}_PROGRAM_ERRORS",
                        stack_screaming,
                        to_screaming_snake_case(&to_pascal_case(&idl.name))
                    ),
                    format!("{}{}ProgramError", stack_pascal, to_pascal_case(&idl.name)),
                )
            } else {
                (
                    format!("{}_PROGRAM_ERRORS", stack_screaming),
                    format!("{}ProgramError", stack_pascal),
                )
            };
            ProgramErrorScope {
                const_name,
                type_name,
                errors: dedupe_errors_by_code(&idl.errors),
                used: false,
            }
        })
        .collect();
    let mut fallback_scope = ProgramErrorScope {
        const_name: format!("{}_PROGRAM_ERRORS", stack_screaming),
        type_name: format!("{}ProgramError", stack_pascal),
        errors: dedupe_errors_by_code(
            &idls
                .iter()
                .flat_map(|idl| idl.errors.iter().cloned())
                .collect::<Vec<_>>(),
        ),
        used: false,
    };

    for instr in instructions {
        // Match the instruction to its program for error scoping and naming.
        let program_index: Option<usize> = if multi_program {
            instr.program_id.as_deref().and_then(|pid| {
                idls.iter()
                    .position(|idl| idl.program_id.as_deref() == Some(pid))
            })
        } else if idls.len() == 1 {
            Some(0)
        } else {
            None
        };
        if multi_program && program_index.is_none() {
            warnings.push(format!(
                "instruction '{}' could not be matched to a program IDL; using stack-wide error metadata and unprefixed naming",
                instr.name
            ));
        }

        // Naming: multi-program handlers are prefixed with their program name
        // so duplicate instruction names across programs cannot collide.
        let program_name = program_index.map(|i| idls[i].name.as_str());
        let (pascal, handler_const, program_key) = match program_name {
            Some(name) if multi_program => {
                let program_pascal = to_pascal_case(name);
                let instr_pascal = to_pascal_case(&instr.name);
                (
                    format!("{}{}", program_pascal, instr_pascal),
                    format!("{}{}Instruction", to_camel_case(name), instr_pascal),
                    Some(to_camel_case(name)),
                )
            }
            _ => (
                to_pascal_case(&instr.name),
                format!("{}Instruction", instr.name),
                None,
            ),
        };
        let (program_errors_const, program_error_type) = match program_index {
            Some(i) => {
                program_scopes[i].used = true;
                (
                    program_scopes[i].const_name.clone(),
                    program_scopes[i].type_name.clone(),
                )
            }
            None => {
                fallback_scope.used = true;
                (
                    fallback_scope.const_name.clone(),
                    fallback_scope.type_name.clone(),
                )
            }
        };

        // --- Parse args; skip the whole instruction on unsupported types. ---
        let mut parsed_args: Vec<(String, ParsedArgType)> = Vec::new();
        let mut unsupported_arg: Option<(String, String)> = None;
        for arg in &instr.args {
            let parsed = defined_types.parse_arg_type(&arg.arg_type);
            if !parsed.supported {
                unsupported_arg = Some((arg.name.clone(), arg.arg_type.clone()));
                break;
            }
            parsed_args.push((arg.name.clone(), parsed));
        }

        if let Some((arg_name, arg_type)) = unsupported_arg {
            let warning = format!(
                "skipped instruction '{}': arg '{}' has unsupported type '{}'",
                instr.name, arg_name, arg_type
            );
            warnings.push(warning.clone());
            blocks.push(format!("// [arete codegen] {}", warning));
            continue;
        }

        // --- Map accounts. ---
        let instr_account_names: HashSet<&str> =
            instr.accounts.iter().map(|a| a.name.as_str()).collect();
        // name -> raw type string, used both for arg-existence checks and to
        // type PDA seeds that reference args.
        let instr_arg_types: BTreeMap<&str, &str> = instr
            .args
            .iter()
            .map(|a| (a.name.as_str(), a.arg_type.as_str()))
            .collect();

        let mut account_literals: Vec<String> = Vec::new();
        let mut user_params: Vec<UserParam> = Vec::new();
        for acc in &instr.accounts {
            let mapped = map_account(
                acc,
                &pda_lookup,
                &instr_account_names,
                &instr_arg_types,
                &instr.name,
                &mut warnings,
            );
            account_literals.push(mapped.literal);
            if let Some(param) = mapped.param {
                user_params.push(param);
            }
        }

        // --- Params interface. ---
        let mut param_lines: Vec<String> = Vec::new();
        for (name, parsed) in &parsed_args {
            param_lines.push(format!("  {}: {};", name, parsed.ts_type));
        }
        for param in &user_params {
            let optional = if param.optional { "?" } else { "" };
            param_lines.push(format!("  {}{}: string;", param.name, optional));
        }
        let params_body = if param_lines.is_empty() {
            "  // This instruction takes no arguments or user-provided accounts.".to_string()
        } else {
            param_lines.join("\n")
        };
        let params_type = format!("{}Params", pascal);
        let params_interface = format!(
            "export interface {} {{\n{}\n}}",
            params_type, params_body
        );

        // --- Error type. Program errors are stack-wide (IDLs do not scope
        // errors to instructions), so each handler's typed error is an alias of
        // the program-wide union. ---
        let error_type = format!("{}Error", pascal);
        let error_decl = format!("export type {} = {};", error_type, program_error_type);

        // --- Args schema literal. ---
        let args_literal = if parsed_args.is_empty() {
            "[]".to_string()
        } else {
            let entries: Vec<String> = parsed_args
                .iter()
                .map(|(name, parsed)| {
                    format!("    {{ name: '{}', type: {} }},", name, parsed.schema)
                })
                .collect();
            format!("[\n{}\n  ]", entries.join("\n"))
        };

        // --- Accounts literal. ---
        let accounts_literal = if account_literals.is_empty() {
            "[]".to_string()
        } else {
            format!("[\n{}\n  ]", account_literals.join("\n"))
        };

        let program_id = instr.program_id.clone().unwrap_or_else(|| default_program_id.clone());
        let discriminator = format!(
            "[{}]",
            instr
                .discriminator
                .iter()
                .map(|b| b.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );

        let docs = render_docs(&instr.docs);
        let handler = format!(
            "{docs}export const {handler_const} = createInstructionHandler<{params_type}, {error_type}>({{\n  programId: '{program_id}',\n  discriminator: {discriminator},\n  args: {args_literal},\n  accounts: {accounts_literal},\n  errors: {program_errors_const},\n}});",
            docs = docs,
            handler_const = handler_const,
            params_type = params_type,
            error_type = error_type,
            program_id = program_id,
            discriminator = discriminator,
            args_literal = args_literal,
            accounts_literal = accounts_literal,
            program_errors_const = program_errors_const,
        );

        blocks.push(format!(
            "{}\n\n{}\n\n{}",
            params_interface, error_decl, handler
        ));
        // Mark the error scope as referenced only when a handler is emitted,
        // so fully-skipped programs do not produce dangling consts.
        match program_index {
            Some(i) => program_scopes[i].used = true,
            None => fallback_scope.used = true,
        }
        stack_entries.push(StackInstructionEntry {
            program_key,
            instruction_name: instr.name.clone(),
            handler_const,
        });
    }

    warnings.append(&mut defined_types.warnings);

    if stack_entries.is_empty() {
        // Nothing emittable (all instructions skipped). Still surface warnings.
        return InstructionsCodegen {
            code: String::new(),
            stack_entries,
            needs_runtime_import: false,
            warnings,
        };
    }

    // Program-level error metadata blocks, one per referenced scope. Errors
    // live on the stack's IDL snapshots (not duplicated onto instructions).
    let mut error_blocks: Vec<String> = Vec::new();
    for scope in program_scopes.iter().chain(std::iter::once(&fallback_scope)) {
        if scope.used {
            error_blocks.push(render_program_errors(
                &scope.const_name,
                &scope.type_name,
                &scope.errors,
            ));
        }
    }
    if error_blocks.is_empty() {
        // Stacks without IDL snapshots still need the fallback scope that
        // every handler references.
        error_blocks.push(render_program_errors(
            &fallback_scope.const_name,
            &fallback_scope.type_name,
            &fallback_scope.errors,
        ));
    }

    let header = "// ============================================================================\n// Instruction Handlers\n// ============================================================================";

    // Defined-type declarations referenced by arg schemas, in dependency order.
    let type_decls = if defined_types.decls.is_empty() {
        String::new()
    } else {
        format!("{}\n\n", defined_types.decls.join("\n\n"))
    };

    let code = format!(
        "{header}\n\n{program_errors_block}\n\n{type_decls}{blocks}",
        header = header,
        program_errors_block = error_blocks.join("\n\n"),
        type_decls = type_decls,
        blocks = blocks.join("\n\n")
    );

    InstructionsCodegen {
        code,
        stack_entries,
        needs_runtime_import: true,
        warnings,
    }
}

/// Render the `instructions: { ... }` block for the stack definition const.
///
/// Entries without a `program_key` render flat; entries with one are grouped
/// under their program's key (multi-program stacks).
pub fn render_instructions_stack_block(entries: &[StackInstructionEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();
    // Flat entries first (single-program stacks, or unmatched instructions).
    for entry in entries.iter().filter(|e| e.program_key.is_none()) {
        lines.push(format!(
            "    {}: {},",
            entry.instruction_name, entry.handler_const
        ));
    }
    // Then one nested block per program, preserving first-seen program order.
    let mut program_order: Vec<&str> = Vec::new();
    for entry in entries {
        if let Some(key) = entry.program_key.as_deref() {
            if !program_order.contains(&key) {
                program_order.push(key);
            }
        }
    }
    for program in program_order {
        let nested: Vec<String> = entries
            .iter()
            .filter(|e| e.program_key.as_deref() == Some(program))
            .map(|e| format!("      {}: {},", e.instruction_name, e.handler_const))
            .collect();
        lines.push(format!(
            "    {}: {{\n{}\n    }},",
            program,
            nested.join("\n")
        ));
    }

    format!("\n  instructions: {{\n{}\n  }},", lines.join("\n"))
}

/// Convert a program name to camelCase for use as a namespace key / const
/// prefix (e.g. "ore_boost" -> "oreBoost").
fn to_camel_case(s: &str) -> String {
    let pascal = to_pascal_case(s);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => pascal,
    }
}

// ============================================================================
// Argument type parsing
// ============================================================================

/// A parsed instruction argument type.
#[derive(Debug, Clone)]
struct ParsedArgType {
    /// TypeScript literal for the core `ArgType` (e.g. `'u64'`, `{ option: 'u64' }`).
    schema: String,
    /// TypeScript parameter type (e.g. `bigint`, `string`, `number[]`).
    ts_type: String,
    /// Whether the type is representable by the core serializer.
    supported: bool,
}

fn unsupported() -> ParsedArgType {
    ParsedArgType {
        schema: "'u8'".to_string(),
        ts_type: "unknown".to_string(),
        supported: false,
    }
}

/// Parse an arg type without any defined-type lookup (primitives and wrappers
/// only). Defined types come back unsupported.
#[cfg(test)]
fn parse_arg_type(raw: &str) -> ParsedArgType {
    DefinedTypes::empty().parse_arg_type(raw)
}

/// Resolver for IDL-defined types (structs/enums) referenced by instruction
/// args. Resolved types are emitted as `export interface` / `export type`
/// declarations (collected in `decls`) and inlined into arg schemas as
/// `{ struct: [...] }` / `{ enum: [...] }` literals, so the runtime needs no
/// type registry.
struct DefinedTypes<'a> {
    /// IDL type definitions by name, first-wins across programs.
    defs: BTreeMap<String, &'a IdlTypeDefSnapshot>,
    /// lowercase name -> canonical key, for case-insensitive fallback lookup.
    lower: BTreeMap<String, String>,
    /// Emitted TS declarations, in dependency order.
    decls: Vec<String>,
    /// Memoized resolutions by original IDL name (None = unsupported).
    resolved: BTreeMap<String, Option<ParsedArgType>>,
    /// TS identifiers already in use (entity interfaces + emitted types).
    taken_names: HashSet<String>,
    /// Names currently being resolved (cycle guard).
    visiting: HashSet<String>,
    warnings: Vec<String>,
}

impl<'a> DefinedTypes<'a> {
    fn new(idls: &'a [IdlSnapshot], reserved_type_names: &HashSet<String>) -> Self {
        let mut defs: BTreeMap<String, &'a IdlTypeDefSnapshot> = BTreeMap::new();
        let mut lower: BTreeMap<String, String> = BTreeMap::new();
        let mut warnings: Vec<String> = Vec::new();
        for idl in idls {
            for def in &idl.types {
                if let Some(existing) = defs.get(def.name.as_str()) {
                    if format!("{:?}", existing.type_def) != format!("{:?}", def.type_def) {
                        warnings.push(format!(
                            "type '{}' is defined differently in multiple programs; using the first definition",
                            def.name
                        ));
                    }
                } else {
                    defs.insert(def.name.clone(), def);
                    lower.insert(def.name.to_lowercase(), def.name.clone());
                }
            }
        }
        DefinedTypes {
            defs,
            lower,
            decls: Vec::new(),
            resolved: BTreeMap::new(),
            taken_names: reserved_type_names.clone(),
            visiting: HashSet::new(),
            warnings,
        }
    }

    #[cfg(test)]
    fn empty() -> DefinedTypes<'static> {
        DefinedTypes::new(&[], &HashSet::new())
    }

    /// Parse a stringified Rust-ish arg type (what `to_rust_type_string`
    /// produces), resolving bare names against the IDL type definitions.
    fn parse_arg_type(&mut self, raw: &str) -> ParsedArgType {
        let t = raw.trim().trim_start_matches('&').trim();

        // Generic wrappers: Option<T>, Vec<T>.
        if let Some((name, inner)) = split_generic(t) {
            match name {
                "Option" => {
                    let inner = self.parse_arg_type(inner);
                    return ParsedArgType {
                        schema: format!("{{ option: {} }}", inner.schema),
                        ts_type: format!("{} | null", inner.ts_type),
                        supported: inner.supported,
                    };
                }
                "Vec" => {
                    let inner = self.parse_arg_type(inner);
                    return ParsedArgType {
                        schema: format!("{{ vec: {} }}", inner.schema),
                        ts_type: format!("{}[]", maybe_paren(&inner.ts_type)),
                        supported: inner.supported,
                    };
                }
                _ => return unsupported(),
            }
        }

        // Fixed-size array: [T; N].
        if let Some(stripped) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if let Some((ty, n)) = stripped.rsplit_once(';') {
                let inner = self.parse_arg_type(ty.trim());
                let n = n.trim();
                if n.parse::<usize>().is_ok() {
                    return ParsedArgType {
                        schema: format!("{{ array: [{}, {}] }}", inner.schema, n),
                        ts_type: format!("{}[]", maybe_paren(&inner.ts_type)),
                        supported: inner.supported,
                    };
                }
            }
        }

        // Primitive (possibly path-qualified, e.g. solana_pubkey::Pubkey).
        let last = t.rsplit("::").next().unwrap_or(t);
        match last {
            "u8" => prim("u8", "number"),
            "u16" => prim("u16", "number"),
            "u32" => prim("u32", "number"),
            "u64" => prim("u64", "bigint"),
            "u128" => prim("u128", "bigint"),
            "i8" => prim("i8", "number"),
            "i16" => prim("i16", "number"),
            "i32" => prim("i32", "number"),
            "i64" => prim("i64", "bigint"),
            "i128" => prim("i128", "bigint"),
            "f32" => prim("f32", "number"),
            "f64" => prim("f64", "number"),
            "bool" => prim("bool", "boolean"),
            "String" | "string" | "str" => prim("string", "string"),
            "Pubkey" | "pubkey" | "PublicKey" | "publicKey" => prim("pubkey", "string"),
            // IDL `bytes` (instruction args reach here as Vec<u8> instead and
            // keep the wire-identical `{ vec: 'u8' }` schema).
            "bytes" => ParsedArgType {
                schema: "'bytes'".to_string(),
                ts_type: "Uint8Array | number[]".to_string(),
                supported: true,
            },
            _ => self.resolve_defined(last).unwrap_or_else(unsupported),
        }
    }

    /// Parse an IDL snapshot type (used inside struct fields / enum variants).
    fn parse_snapshot_type(&mut self, t: &IdlTypeSnapshot) -> ParsedArgType {
        match t {
            IdlTypeSnapshot::Simple(s) => self.parse_arg_type(s),
            IdlTypeSnapshot::Option(o) => {
                let inner = self.parse_snapshot_type(&o.option);
                ParsedArgType {
                    schema: format!("{{ option: {} }}", inner.schema),
                    ts_type: format!("{} | null", inner.ts_type),
                    supported: inner.supported,
                }
            }
            IdlTypeSnapshot::Vec(v) => {
                let inner = self.parse_snapshot_type(&v.vec);
                ParsedArgType {
                    schema: format!("{{ vec: {} }}", inner.schema),
                    ts_type: format!("{}[]", maybe_paren(&inner.ts_type)),
                    supported: inner.supported,
                }
            }
            IdlTypeSnapshot::Array(arr) => {
                let mut element: Option<ParsedArgType> = None;
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
                    (Some(inner), Some(n)) => ParsedArgType {
                        schema: format!("{{ array: [{}, {}] }}", inner.schema, n),
                        ts_type: format!("{}[]", maybe_paren(&inner.ts_type)),
                        supported: inner.supported,
                    },
                    _ => unsupported(),
                }
            }
            IdlTypeSnapshot::HashMap(_) => unsupported(),
            IdlTypeSnapshot::Defined(d) => {
                let name = match &d.defined {
                    IdlDefinedInnerSnapshot::Named { name } => name.as_str(),
                    IdlDefinedInnerSnapshot::Simple(s) => s.as_str(),
                };
                self.resolve_defined(name).unwrap_or_else(unsupported)
            }
        }
    }

    /// Resolve a bare type name against the IDL type definitions, emitting a
    /// TS declaration on first use. Returns `None` when unsupported.
    fn resolve_defined(&mut self, name: &str) -> Option<ParsedArgType> {
        if let Some(cached) = self.resolved.get(name) {
            return cached.clone();
        }
        if self.visiting.contains(name) {
            self.warnings.push(format!(
                "type '{}' is recursive; recursive types are not supported by instruction codegen",
                name
            ));
            return None;
        }

        let key = if self.defs.contains_key(name) {
            name.to_string()
        } else {
            // `to_rust_type_string` passes IDL names through verbatim, but the
            // referencing spelling occasionally differs in case.
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
                self.resolve_struct(&key, &fields)
            }
            IdlTypeDefKindSnapshot::TupleStruct { .. } => {
                self.warnings.push(format!(
                    "type '{}' is a tuple struct, which instruction codegen does not support yet",
                    key
                ));
                None
            }
            IdlTypeDefKindSnapshot::Enum { variants, .. } => {
                let variants = variants.clone();
                self.resolve_enum(&key, &variants)
            }
        };
        self.visiting.remove(&key);
        self.resolved.insert(name.to_string(), result.clone());
        if name != key {
            self.resolved.insert(key, result.clone());
        }
        result
    }

    fn resolve_struct(
        &mut self,
        name: &str,
        fields: &[crate::ast::IdlFieldSnapshot],
    ) -> Option<ParsedArgType> {
        let mut schema_fields: Vec<String> = Vec::new();
        let mut ts_fields: Vec<String> = Vec::new();
        for field in fields {
            let parsed = self.parse_snapshot_type(&field.type_);
            if !parsed.supported {
                self.warnings.push(format!(
                    "type '{}': field '{}' has an unsupported type",
                    name, field.name
                ));
                return None;
            }
            schema_fields.push(format!("{{ name: '{}', type: {} }}", field.name, parsed.schema));
            ts_fields.push(format!("  {}: {};", field.name, parsed.ts_type));
        }

        let ts_name = self.claim_ts_name(name);
        self.decls.push(format!(
            "export interface {} {{\n{}\n}}",
            ts_name,
            ts_fields.join("\n")
        ));
        Some(ParsedArgType {
            schema: format!("{{ struct: [{}] }}", schema_fields.join(", ")),
            ts_type: ts_name,
            supported: true,
        })
    }

    fn resolve_enum(
        &mut self,
        name: &str,
        variants: &[crate::ast::IdlEnumVariantSnapshot],
    ) -> Option<ParsedArgType> {
        use crate::ast::IdlEnumVariantFieldSnapshot;

        let mut schema_variants: Vec<String> = Vec::new();
        let mut ts_variants: Vec<String> = Vec::new();
        for variant in variants {
            if variant.fields.is_empty() {
                schema_variants.push(format!("'{}'", variant.name));
                ts_variants.push(format!("'{}'", variant.name));
                continue;
            }

            let named: Vec<_> = variant
                .fields
                .iter()
                .filter_map(|f| match f {
                    IdlEnumVariantFieldSnapshot::Named(field) => Some(field),
                    IdlEnumVariantFieldSnapshot::Tuple(_) => None,
                })
                .collect();

            if named.len() == variant.fields.len() {
                // Struct variant: { name: 'x', fields: [...] }.
                let mut field_schemas: Vec<String> = Vec::new();
                let mut field_ts: Vec<String> = Vec::new();
                for field in named {
                    let parsed = self.parse_snapshot_type(&field.type_);
                    if !parsed.supported {
                        self.warnings.push(format!(
                            "enum '{}': variant '{}' field '{}' has an unsupported type",
                            name, variant.name, field.name
                        ));
                        return None;
                    }
                    field_schemas
                        .push(format!("{{ name: '{}', type: {} }}", field.name, parsed.schema));
                    field_ts.push(format!("{}: {}", field.name, parsed.ts_type));
                }
                schema_variants.push(format!(
                    "{{ name: '{}', fields: [{}] }}",
                    variant.name,
                    field_schemas.join(", ")
                ));
                ts_variants.push(format!(
                    "{{ {}: {{ {} }} }}",
                    variant.name,
                    field_ts.join("; ")
                ));
            } else if named.is_empty() {
                // Tuple variant: { name: 'x', tuple: [...] }.
                let mut element_schemas: Vec<String> = Vec::new();
                let mut element_ts: Vec<String> = Vec::new();
                for field in &variant.fields {
                    let IdlEnumVariantFieldSnapshot::Tuple(ty) = field else {
                        unreachable!("named.is_empty() guarantees tuple fields");
                    };
                    let parsed = self.parse_snapshot_type(ty);
                    if !parsed.supported {
                        self.warnings.push(format!(
                            "enum '{}': variant '{}' has an unsupported tuple element type",
                            name, variant.name
                        ));
                        return None;
                    }
                    element_schemas.push(parsed.schema);
                    element_ts.push(parsed.ts_type);
                }
                schema_variants.push(format!(
                    "{{ name: '{}', tuple: [{}] }}",
                    variant.name,
                    element_schemas.join(", ")
                ));
                ts_variants.push(format!(
                    "{{ {}: [{}] }}",
                    variant.name,
                    element_ts.join(", ")
                ));
            } else {
                self.warnings.push(format!(
                    "enum '{}': variant '{}' mixes named and tuple fields, which is not supported",
                    name, variant.name
                ));
                return None;
            }
        }

        let ts_name = self.claim_ts_name(name);
        self.decls.push(format!(
            "export type {} =\n  | {};",
            ts_name,
            ts_variants.join("\n  | ")
        ));
        Some(ParsedArgType {
            schema: format!("{{ enum: [{}] }}", schema_variants.join(", ")),
            ts_type: ts_name,
            supported: true,
        })
    }

    /// Pick a unique TS identifier for a defined type, suffixing `Input` (then
    /// a counter) when the pascal-cased name collides with an entity interface
    /// or another emitted type.
    fn claim_ts_name(&mut self, name: &str) -> String {
        let base = to_pascal_case(name);
        let mut candidate = base.clone();
        if self.taken_names.contains(&candidate) {
            candidate = format!("{}Input", base);
            let mut counter = 2;
            while self.taken_names.contains(&candidate) {
                candidate = format!("{}Input{}", base, counter);
                counter += 1;
            }
            self.warnings.push(format!(
                "type '{}' collides with an existing interface; emitted as '{}'",
                name, candidate
            ));
        }
        self.taken_names.insert(candidate.clone());
        candidate
    }
}

fn prim(schema: &str, ts: &str) -> ParsedArgType {
    ParsedArgType {
        schema: format!("'{}'", schema),
        ts_type: ts.to_string(),
        supported: true,
    }
}

/// Split `Name<inner>` into `(Name, inner)`, ignoring path qualifiers on `Name`.
fn split_generic(t: &str) -> Option<(&str, &str)> {
    let open = t.find('<')?;
    if !t.ends_with('>') {
        return None;
    }
    let name = t[..open].rsplit("::").next().unwrap_or(&t[..open]).trim();
    let inner = t[open + 1..t.len() - 1].trim();
    Some((name, inner))
}

/// Wrap union types in parentheses so `T | null` arrays read as `(T | null)[]`.
fn maybe_paren(ts: &str) -> String {
    if ts.contains('|') {
        format!("({})", ts)
    } else {
        ts.to_string()
    }
}

// ============================================================================
// Account mapping
// ============================================================================

/// A user-provided account that must surface as a `Params` field.
#[derive(Debug, Clone)]
struct UserParam {
    name: String,
    optional: bool,
}

/// Result of mapping a single instruction account.
struct MappedAccount {
    /// TypeScript `AccountMeta` object literal.
    literal: String,
    /// Set when the account is caller-supplied (`userProvided`).
    param: Option<UserParam>,
}

fn map_account(
    acc: &InstructionAccountDef,
    pda_lookup: &BTreeMap<&str, &PdaDefinition>,
    instr_account_names: &HashSet<&str>,
    instr_arg_types: &BTreeMap<&str, &str>,
    instr_name: &str,
    warnings: &mut Vec<String>,
) -> MappedAccount {
    let base = format!(
        "name: '{}', isSigner: {}, isWritable: {}",
        acc.name, acc.is_signer, acc.is_writable
    );
    let optional_suffix = if acc.is_optional {
        ", isOptional: true".to_string()
    } else {
        String::new()
    };

    let user_provided = |warn: Option<String>, warnings: &mut Vec<String>| -> MappedAccount {
        // Degradations are surfaced both to the compiler caller (warnings) and
        // in the generated code, so SDK readers can see why an account that
        // looks derivable must be passed in manually.
        let comment = match &warn {
            Some(w) => format!("    // [arete codegen] {}\n", w),
            None => String::new(),
        };
        if let Some(w) = warn {
            warnings.push(w);
        }
        MappedAccount {
            literal: format!(
                "{}    {{ {}, category: 'userProvided'{} }},",
                comment, base, optional_suffix
            ),
            param: Some(UserParam {
                name: acc.name.clone(),
                optional: acc.is_optional,
            }),
        }
    };

    match &acc.resolution {
        AccountResolution::Signer => MappedAccount {
            literal: format!("    {{ {}, category: 'signer'{} }},", base, optional_suffix),
            param: None,
        },
        AccountResolution::Known { address } => MappedAccount {
            literal: format!(
                "    {{ {}, category: 'known', knownAddress: '{}'{} }},",
                base, address, optional_suffix
            ),
            param: None,
        },
        AccountResolution::UserProvided => user_provided(None, warnings),
        AccountResolution::PdaInline { seeds, program_id } => {
            match build_pda_config(seeds, program_id.as_deref(), instr_account_names, instr_arg_types)
            {
                Ok((pda_config, seed_warnings)) => {
                    for w in seed_warnings {
                        warnings.push(format!(
                            "instruction '{}': account '{}': {}",
                            instr_name, acc.name, w
                        ));
                    }
                    MappedAccount {
                        literal: format!(
                            "    {{ {}, category: 'pda', pdaConfig: {}{} }},",
                            base, pda_config, optional_suffix
                        ),
                        param: None,
                    }
                }
                Err(reason) => user_provided(
                    Some(format!(
                        "instruction '{}': account '{}' inline PDA degraded to userProvided ({})",
                        instr_name, acc.name, reason
                    )),
                    warnings,
                ),
            }
        }
        AccountResolution::PdaRef { pda_name } => match pda_lookup.get(pda_name.as_str()) {
            Some(def) => match build_pda_config(
                &def.seeds,
                def.program_id.as_deref(),
                instr_account_names,
                instr_arg_types,
            ) {
                Ok((pda_config, seed_warnings)) => {
                    for w in seed_warnings {
                        warnings.push(format!(
                            "instruction '{}': account '{}': {}",
                            instr_name, acc.name, w
                        ));
                    }
                    MappedAccount {
                        literal: format!(
                            "    {{ {}, category: 'pda', pdaConfig: {}{} }},",
                            base, pda_config, optional_suffix
                        ),
                        param: None,
                    }
                }
                Err(reason) => user_provided(
                    Some(format!(
                        "instruction '{}': account '{}' PDA '{}' degraded to userProvided ({})",
                        instr_name, acc.name, pda_name, reason
                    )),
                    warnings,
                ),
            },
            None => user_provided(
                Some(format!(
                    "instruction '{}': account '{}' references unknown PDA '{}'; degraded to userProvided",
                    instr_name, acc.name, pda_name
                )),
                warnings,
            ),
        },
    }
}

/// Build a TypeScript `PdaConfig` literal from seed definitions.
///
/// Returns `Err(reason)` when the PDA cannot be represented by the core
/// resolver (e.g. seeds referencing accounts/args that do not exist in this
/// instruction), so the caller can degrade to `userProvided`. On success the
/// second tuple element carries soft warnings (e.g. an arg seed whose type
/// could not be determined, leaving the runtime to encode heuristically).
fn build_pda_config(
    seeds: &[PdaSeedDef],
    program_id: Option<&str>,
    instr_account_names: &HashSet<&str>,
    instr_arg_types: &BTreeMap<&str, &str>,
) -> Result<(String, Vec<String>), String> {
    let mut seed_literals: Vec<String> = Vec::new();
    let mut soft_warnings: Vec<String> = Vec::new();
    for seed in seeds {
        match seed {
            PdaSeedDef::Literal { value } => {
                seed_literals.push(format!(
                    "{{ type: 'literal', value: '{}' }}",
                    escape_single_quotes(value)
                ));
            }
            PdaSeedDef::AccountRef { account_name } => {
                if !instr_account_names.contains(account_name.as_str()) {
                    return Err(format!(
                        "seed references account '{}' not present in this instruction",
                        account_name
                    ));
                }
                seed_literals.push(format!(
                    "{{ type: 'accountRef', accountName: '{}' }}",
                    account_name
                ));
            }
            PdaSeedDef::ArgRef { arg_name, arg_type } => {
                if !instr_arg_types.contains_key(arg_name.as_str()) {
                    return Err(format!(
                        "seed references arg '{}' not present in this instruction",
                        arg_name
                    ));
                }
                // Prefer the seed's declared type; fall back to the
                // instruction arg's type (Anchor seeds carry no type info).
                let raw_type = arg_type
                    .as_deref()
                    .or_else(|| instr_arg_types.get(arg_name.as_str()).copied());
                match raw_type.and_then(normalize_seed_arg_type) {
                    Some(canonical) => seed_literals.push(format!(
                        "{{ type: 'argRef', argName: '{}', argType: '{}' }}",
                        arg_name, canonical
                    )),
                    None => {
                        soft_warnings.push(format!(
                            "seed arg '{}' has non-primitive type '{}'; runtime will use heuristic encoding",
                            arg_name,
                            raw_type.unwrap_or("<unknown>")
                        ));
                        seed_literals
                            .push(format!("{{ type: 'argRef', argName: '{}' }}", arg_name));
                    }
                }
            }
            PdaSeedDef::Bytes { value } => {
                let bytes: Vec<String> = value.iter().map(|b| b.to_string()).collect();
                seed_literals.push(format!(
                    "{{ type: 'bytes', value: [{}] }}",
                    bytes.join(", ")
                ));
            }
        }
    }

    let seeds_str = seed_literals.join(", ");
    let config = match program_id {
        Some(pid) => format!("{{ programId: '{}', seeds: [{}] }}", pid, seeds_str),
        None => format!("{{ seeds: [{}] }}", seeds_str),
    };
    Ok((config, soft_warnings))
}

/// Normalize a raw arg-type string (IDL or `pdas!` DSL spelling) to the
/// canonical seed type the runtime's `serializeSeedValue` understands.
/// Returns `None` for types that cannot be encoded as a seed.
fn normalize_seed_arg_type(raw: &str) -> Option<String> {
    let t = raw.rsplit("::").next().unwrap_or(raw).trim();
    if let Some(width) = t.strip_prefix('u').or_else(|| t.strip_prefix('i')) {
        if matches!(width, "8" | "16" | "32" | "64" | "128") {
            return Some(t.to_string());
        }
        return None;
    }
    match t {
        "Pubkey" | "pubkey" | "publicKey" | "PublicKey" => Some("pubkey".to_string()),
        "String" | "string" | "str" => Some("string".to_string()),
        _ => None,
    }
}

// ============================================================================
// Errors
// ============================================================================

/// Dedupe errors by code, preserving first-seen definitions, sorted ascending.
fn dedupe_errors_by_code(errors: &[IdlErrorSnapshot]) -> Vec<IdlErrorSnapshot> {
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    let mut by_code: BTreeMap<u32, IdlErrorSnapshot> = BTreeMap::new();
    for err in errors {
        if seen.insert(err.code) {
            by_code.insert(err.code, err.clone());
        }
    }
    by_code.into_values().collect()
}

fn render_program_errors(
    const_name: &str,
    type_name: &str,
    errors: &[IdlErrorSnapshot],
) -> String {
    if errors.is_empty() {
        return format!(
            "/** Program errors for this stack (none declared in the IDL). */\nexport type {} = never;\n\nconst {}: ErrorMetadata[] = [];",
            type_name, const_name
        );
    }

    let type_decl = format!(
        "/** Union of all program errors declared across this stack's instructions. */\nexport type {} =\n{};",
        type_name,
        error_union_variants(errors)
    );

    let entries: Vec<String> = errors
        .iter()
        .map(|err| {
            format!(
                "  {{ code: {}, name: '{}', msg: '{}' }},",
                err.code,
                err.name,
                escape_single_quotes(err.msg.as_deref().unwrap_or(""))
            )
        })
        .collect();
    let const_decl = format!(
        "const {}: ErrorMetadata[] = [\n{}\n];",
        const_name,
        entries.join("\n")
    );

    format!("{}\n\n{}", type_decl, const_decl)
}

/// Render the `| { code; name; msg } | ...` body of an error union type.
fn error_union_variants(errors: &[IdlErrorSnapshot]) -> String {
    errors
        .iter()
        .map(|err| {
            format!(
                "  | {{ code: {}; name: '{}'; msg: string }}",
                err.code, err.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ============================================================================
// Helpers
// ============================================================================

fn escape_single_quotes(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace(['\n', '\r'], " ")
}

fn render_docs(docs: &[String]) -> String {
    if docs.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = docs
        .iter()
        .map(|line| format!(" * {}", line.trim()))
        .collect();
    format!("/**\n{}\n */\n", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{InstructionAccountDef, InstructionArgDef};

    fn arg(name: &str, ty: &str) -> InstructionArgDef {
        InstructionArgDef {
            name: name.to_string(),
            arg_type: ty.to_string(),
            docs: vec![],
        }
    }

    fn idl(name: &str, program_id: &str, errors: Vec<IdlErrorSnapshot>) -> IdlSnapshot {
        IdlSnapshot {
            name: name.to_string(),
            program_id: Some(program_id.to_string()),
            version: "0.1.0".to_string(),
            accounts: vec![],
            instructions: vec![],
            types: vec![],
            events: vec![],
            errors,
            discriminant_size: 1,
        }
    }

    #[test]
    fn parses_primitive_and_wrapper_arg_types() {
        let u64 = parse_arg_type("u64");
        assert_eq!(u64.schema, "'u64'");
        assert_eq!(u64.ts_type, "bigint");
        assert!(u64.supported);

        let pk = parse_arg_type("solana_pubkey::Pubkey");
        assert_eq!(pk.schema, "'pubkey'");
        assert_eq!(pk.ts_type, "string");

        let opt = parse_arg_type("Option<u64>");
        assert_eq!(opt.schema, "{ option: 'u64' }");
        assert_eq!(opt.ts_type, "bigint | null");

        let vec = parse_arg_type("Vec<u8>");
        assert_eq!(vec.schema, "{ vec: 'u8' }");
        assert_eq!(vec.ts_type, "number[]");

        let arr = parse_arg_type("[u8; 32]");
        assert_eq!(arr.schema, "{ array: ['u8', 32] }");
        assert_eq!(arr.ts_type, "number[]");

        let opt_vec = parse_arg_type("Vec<Option<u64>>");
        assert_eq!(opt_vec.ts_type, "(bigint | null)[]");
    }

    #[test]
    fn defined_types_are_unsupported_without_a_lookup() {
        let defined = parse_arg_type("createFixedDelegationData");
        assert!(!defined.supported);
    }

    fn struct_def(name: &str, fields: Vec<(&str, IdlTypeSnapshot)>) -> IdlTypeDefSnapshot {
        IdlTypeDefSnapshot {
            name: name.to_string(),
            docs: vec![],
            serialization: None,
            type_def: IdlTypeDefKindSnapshot::Struct {
                kind: "struct".to_string(),
                fields: fields
                    .into_iter()
                    .map(|(n, t)| crate::ast::IdlFieldSnapshot {
                        name: n.to_string(),
                        type_: t,
                    })
                    .collect(),
            },
        }
    }

    fn simple(t: &str) -> IdlTypeSnapshot {
        IdlTypeSnapshot::Simple(t.to_string())
    }

    fn defined(name: &str) -> IdlTypeSnapshot {
        IdlTypeSnapshot::Defined(crate::ast::IdlDefinedTypeSnapshot {
            defined: IdlDefinedInnerSnapshot::Named {
                name: name.to_string(),
            },
        })
    }

    #[test]
    fn resolves_struct_args_with_nesting_and_enums() {
        let mut idl = idl("demo", "Prog111", vec![]);
        idl.types = vec![
            struct_def(
                "transferData",
                vec![
                    ("amount", simple("u64")),
                    ("terms", defined("planTerms")),
                    ("status", defined("planStatus")),
                ],
            ),
            struct_def("planTerms", vec![("periodHours", simple("u64"))]),
            IdlTypeDefSnapshot {
                name: "planStatus".to_string(),
                docs: vec![],
                serialization: None,
                type_def: IdlTypeDefKindSnapshot::Enum {
                    kind: "enum".to_string(),
                    variants: vec![
                        crate::ast::IdlEnumVariantSnapshot {
                            name: "Active".to_string(),
                            fields: vec![],
                        },
                        crate::ast::IdlEnumVariantSnapshot {
                            name: "Sunset".to_string(),
                            fields: vec![crate::ast::IdlEnumVariantFieldSnapshot::Named(
                                crate::ast::IdlFieldSnapshot {
                                    name: "endTs".to_string(),
                                    type_: simple("i64"),
                                },
                            )],
                        },
                    ],
                },
            },
        ];
        let idls = vec![idl];

        let instr = InstructionDef {
            name: "transfer".to_string(),
            discriminator: vec![4],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![arg("transferData", "transferData")],
            errors: vec![],
            program_id: Some("Prog111".to_string()),
            docs: vec![],
        };

        let out = generate_instructions_code(
            "Demo",
            std::slice::from_ref(&instr),
            &idls,
            &BTreeMap::new(),
            &["Prog111".to_string()],
            &HashSet::new(),
        );

        assert_eq!(out.stack_entries.len(), 1, "warnings: {:?}", out.warnings);
        let code = &out.code;
        // Emitted TS declarations for every referenced defined type.
        assert!(code.contains("export interface TransferData"));
        assert!(code.contains("export interface PlanTerms"));
        assert!(code.contains("export type PlanStatus"));
        assert!(code.contains("'Active'"));
        assert!(code.contains("{ Sunset: { endTs: bigint } }"));
        // Inlined schemas, including the nested struct and fielded enum.
        assert!(code.contains("{ name: 'periodHours', type: 'u64' }"));
        assert!(code.contains(
            "{ name: 'status', type: { enum: ['Active', { name: 'Sunset', fields: [{ name: 'endTs', type: 'i64' }] }] } }"
        ));
        // Params reference the generated interface type.
        assert!(code.contains("transferData: TransferData;"));
    }

    #[test]
    fn recursive_defined_types_skip_with_warning() {
        let mut idl_snap = idl("demo", "Prog111", vec![]);
        idl_snap.types = vec![struct_def("node", vec![("next", defined("node"))])];
        let idls = vec![idl_snap];

        let instr = InstructionDef {
            name: "insert".to_string(),
            discriminator: vec![1],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![arg("node", "node")],
            errors: vec![],
            program_id: Some("Prog111".to_string()),
            docs: vec![],
        };
        let out = generate_instructions_code(
            "Demo",
            std::slice::from_ref(&instr),
            &idls,
            &BTreeMap::new(),
            &["Prog111".to_string()],
            &HashSet::new(),
        );
        assert!(out.stack_entries.is_empty());
        assert!(out.warnings.iter().any(|w| w.contains("recursive")));
    }

    #[test]
    fn defined_type_name_collisions_get_input_suffix() {
        let mut idl_snap = idl("demo", "Prog111", vec![]);
        idl_snap.types = vec![struct_def("planTerms", vec![("amount", simple("u64"))])];
        let idls = vec![idl_snap];

        let instr = InstructionDef {
            name: "setTerms".to_string(),
            discriminator: vec![2],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![arg("terms", "planTerms")],
            errors: vec![],
            program_id: Some("Prog111".to_string()),
            docs: vec![],
        };

        // Simulate an entity interface already named PlanTerms.
        let reserved: HashSet<String> = ["PlanTerms".to_string()].into_iter().collect();
        let out = generate_instructions_code(
            "Demo",
            std::slice::from_ref(&instr),
            &idls,
            &BTreeMap::new(),
            &["Prog111".to_string()],
            &reserved,
        );
        assert!(out.code.contains("export interface PlanTermsInput"));
        assert!(out.code.contains("terms: PlanTermsInput;"));
        assert!(out.warnings.iter().any(|w| w.contains("collides")));
    }

    #[test]
    fn skips_instructions_with_unsupported_args() {
        let instr = InstructionDef {
            name: "subscribe".to_string(),
            discriminator: vec![3],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![arg("data", "subscribeData")],
            errors: vec![],
            program_id: None,
            docs: vec![],
        };
        let out = generate_instructions_code(
            "Subscriptions",
            std::slice::from_ref(&instr),
            &[],
            &BTreeMap::new(),
            &["Prog111".to_string()],
            &HashSet::new(),
        );
        assert!(out.stack_entries.is_empty());
        assert!(out.warnings.iter().any(|w| w.contains("subscribe")));
    }

    #[test]
    fn emits_handler_with_signer_known_and_user_provided_accounts() {
        let instr = InstructionDef {
            name: "closeSubscriptionAuthority".to_string(),
            discriminator: vec![6],
            discriminator_size: 1,
            accounts: vec![
                InstructionAccountDef {
                    name: "user".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                    docs: vec![],
                },
                InstructionAccountDef {
                    name: "subscriptionAuthority".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                    docs: vec![],
                },
            ],
            args: vec![],
            errors: vec![],
            program_id: Some("De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44".to_string()),
            docs: vec![],
        };

        let idls = vec![idl(
            "subscriptions",
            "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44",
            vec![IdlErrorSnapshot {
                code: 130,
                name: "unauthorized".to_string(),
                msg: Some("Caller not authorized".to_string()),
            }],
        )];
        let out = generate_instructions_code(
            "Subscriptions",
            std::slice::from_ref(&instr),
            &idls,
            &BTreeMap::new(),
            &["De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44".to_string()],
            &HashSet::new(),
        );

        assert_eq!(
            out.stack_entries,
            vec![StackInstructionEntry {
                program_key: None,
                instruction_name: "closeSubscriptionAuthority".to_string(),
                handler_const: "closeSubscriptionAuthorityInstruction".to_string(),
            }]
        );
        assert!(out.needs_runtime_import);
        let code = &out.code;
        assert!(code.contains("export interface CloseSubscriptionAuthorityParams"));
        assert!(code.contains("subscriptionAuthority: string;"));
        assert!(code.contains("category: 'signer'"));
        assert!(code.contains("category: 'userProvided'"));
        assert!(code.contains(
            "export const closeSubscriptionAuthorityInstruction = createInstructionHandler<CloseSubscriptionAuthorityParams, CloseSubscriptionAuthorityError>"
        ));
        assert!(code.contains("SUBSCRIPTIONS_PROGRAM_ERRORS: ErrorMetadata[]"));
        assert!(code.contains("code: 130, name: 'unauthorized'"));
    }

    #[test]
    fn inlines_pda_ref_seeds_including_raw_bytes() {
        let mut program_pdas: BTreeMap<String, PdaDefinition> = BTreeMap::new();
        program_pdas.insert(
            "subscriptionAuthority".to_string(),
            PdaDefinition {
                name: "subscriptionAuthority".to_string(),
                seeds: vec![
                    PdaSeedDef::Literal {
                        value: "SubscriptionAuthority".to_string(),
                    },
                    PdaSeedDef::Bytes {
                        value: vec![1, 2, 255],
                    },
                    PdaSeedDef::AccountRef {
                        account_name: "owner".to_string(),
                    },
                    PdaSeedDef::AccountRef {
                        account_name: "tokenMint".to_string(),
                    },
                ],
                program_id: None,
            },
        );
        let mut pdas = BTreeMap::new();
        pdas.insert("subscriptions".to_string(), program_pdas);

        let instr = InstructionDef {
            name: "initSubscriptionAuthority".to_string(),
            discriminator: vec![0],
            discriminator_size: 1,
            accounts: vec![
                InstructionAccountDef {
                    name: "owner".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                    docs: vec![],
                },
                InstructionAccountDef {
                    name: "subscriptionAuthority".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::PdaRef {
                        pda_name: "subscriptionAuthority".to_string(),
                    },
                    is_optional: false,
                    docs: vec![],
                },
                InstructionAccountDef {
                    name: "tokenMint".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                    docs: vec![],
                },
            ],
            args: vec![],
            errors: vec![],
            program_id: Some("De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44".to_string()),
            docs: vec![],
        };

        let out = generate_instructions_code(
            "Subscriptions",
            std::slice::from_ref(&instr),
            &[],
            &pdas,
            &["De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44".to_string()],
            &HashSet::new(),
        );
        let code = &out.code;
        assert!(code.contains("category: 'pda'"));
        assert!(code.contains("{ type: 'literal', value: 'SubscriptionAuthority' }"));
        assert!(code.contains("{ type: 'bytes', value: [1, 2, 255] }"));
        assert!(code.contains("{ type: 'accountRef', accountName: 'owner' }"));
        // PDA account is resolved internally, so it is NOT a param; tokenMint is.
        assert!(code.contains("tokenMint: string;"));
        assert!(!code.contains("subscriptionAuthority: string;"));
        assert!(out.warnings.is_empty(), "no degradation expected: {:?}", out.warnings);
    }

    #[test]
    fn emits_typed_arg_seeds_with_instr_args_fallback() {
        let mut program_pdas: BTreeMap<String, PdaDefinition> = BTreeMap::new();
        program_pdas.insert(
            "round".to_string(),
            PdaDefinition {
                name: "round".to_string(),
                seeds: vec![
                    PdaSeedDef::Literal {
                        value: "round".to_string(),
                    },
                    // Declared type on the seed itself (pdas! DSL style).
                    PdaSeedDef::ArgRef {
                        arg_name: "roundId".to_string(),
                        arg_type: Some("u32".to_string()),
                    },
                    // No declared type: must fall back to the instruction arg.
                    PdaSeedDef::ArgRef {
                        arg_name: "owner".to_string(),
                        arg_type: None,
                    },
                ],
                program_id: None,
            },
        );
        let mut pdas = BTreeMap::new();
        pdas.insert("demo".to_string(), program_pdas);

        let instr = InstructionDef {
            name: "commit".to_string(),
            discriminator: vec![1],
            discriminator_size: 1,
            accounts: vec![InstructionAccountDef {
                name: "round".to_string(),
                is_signer: false,
                is_writable: true,
                resolution: AccountResolution::PdaRef {
                    pda_name: "round".to_string(),
                },
                is_optional: false,
                docs: vec![],
            }],
            args: vec![
                arg("roundId", "u32"),
                arg("owner", "solana_pubkey::Pubkey"),
            ],
            errors: vec![],
            program_id: Some("De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44".to_string()),
            docs: vec![],
        };

        let out = generate_instructions_code(
            "Demo",
            std::slice::from_ref(&instr),
            &[],
            &pdas,
            &["De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44".to_string()],
            &HashSet::new(),
        );
        let code = &out.code;
        assert!(code.contains("{ type: 'argRef', argName: 'roundId', argType: 'u32' }"));
        assert!(
            code.contains("{ type: 'argRef', argName: 'owner', argType: 'pubkey' }"),
            "path-qualified Pubkey arg type should normalize via instr.args fallback: {}",
            code
        );
    }

    #[test]
    fn untypeable_arg_seed_emits_without_arg_type_and_warns() {
        let mut program_pdas: BTreeMap<String, PdaDefinition> = BTreeMap::new();
        program_pdas.insert(
            "vault".to_string(),
            PdaDefinition {
                name: "vault".to_string(),
                seeds: vec![PdaSeedDef::ArgRef {
                    arg_name: "data".to_string(),
                    arg_type: None,
                }],
                program_id: None,
            },
        );
        let mut pdas = BTreeMap::new();
        pdas.insert("demo".to_string(), program_pdas);

        let instr = InstructionDef {
            name: "store".to_string(),
            discriminator: vec![2],
            discriminator_size: 1,
            accounts: vec![InstructionAccountDef {
                name: "vault".to_string(),
                is_signer: false,
                is_writable: true,
                resolution: AccountResolution::PdaRef {
                    pda_name: "vault".to_string(),
                },
                is_optional: false,
                docs: vec![],
            }],
            args: vec![arg("data", "Vec<u8>")],
            errors: vec![],
            program_id: Some("De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44".to_string()),
            docs: vec![],
        };

        let out = generate_instructions_code(
            "Demo",
            std::slice::from_ref(&instr),
            &[],
            &pdas,
            &["De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44".to_string()],
            &HashSet::new(),
        );
        assert!(out.code.contains("{ type: 'argRef', argName: 'data' }"));
        assert!(
            out.warnings
                .iter()
                .any(|w| w.contains("heuristic encoding")),
            "expected soft warning, got {:?}",
            out.warnings
        );
    }

    /// Golden test: drive the codegen from the real, compiler-produced ore
    /// stack JSON and assert the expected handlers and PDA configs appear. This
    /// exercises the full `stack.json -> TypeScript` path against actual data
    /// (the Steel `pdas!` registry resolving instruction accounts to `PdaRef`).
    #[test]
    fn golden_ore_stack_json_emits_pda_handlers() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../stacks/ore/.arete/OreStream.stack.json"
        );
        let json = match std::fs::read_to_string(path) {
            Ok(c) => c,
            // Stack JSON is generated by the macro build; skip if not present.
            Err(_) => return,
        };
        let spec: crate::ast::SerializableStackSpec =
            serde_json::from_str(&json).expect("ore stack json should deserialize");

        let out = generate_instructions_code(
            &to_pascal_case(&spec.stack_name),
            &spec.instructions,
            &spec.idls,
            &spec.pdas,
            &spec.program_ids,
            &HashSet::new(),
        );

        assert!(
            !out.stack_entries.is_empty(),
            "expected at least one emitted ore handler"
        );
        let code = &out.code;
        assert!(code.contains("createInstructionHandler"));
        // Pure-literal PDA (treasury) and authority-keyed PDA (miner) both appear.
        assert!(
            code.contains("{ type: 'literal', value: 'treasury' }"),
            "treasury PDA seed should be inlined"
        );
        assert!(
            code.contains("{ type: 'literal', value: 'miner' }")
                && code.contains("{ type: 'accountRef', accountName: 'authority' }"),
            "miner PDA seeds should be inlined with an authority accountRef"
        );
        assert!(code.contains("category: 'pda'"));

        // The ore stack bundles two programs (ore + entropy) that BOTH define
        // a `close` instruction: handlers must be program-prefixed, the stack
        // block namespaced, and errors scoped per program.
        assert!(spec.idls.len() > 1, "ore stack should bundle two programs");
        assert!(code.contains("export const oreCloseInstruction"));
        assert!(code.contains("export const entropyCloseInstruction"));
        assert!(!code.contains("export const closeInstruction"));
        assert!(code.contains("ORE_STREAM_ORE_PROGRAM_ERRORS"));
        assert!(code.contains("ORE_STREAM_ENTROPY_PROGRAM_ERRORS"));
        let block = render_instructions_stack_block(&out.stack_entries);
        assert!(block.contains("    ore: {"));
        assert!(block.contains("    entropy: {"));
        assert!(block.contains("      close: oreCloseInstruction,"));
        assert!(block.contains("      close: entropyCloseInstruction,"));

        // Exact-string golden: the full oreClose block (params interface,
        // error alias, docs, handler) must match the checked-in fixture. This
        // catches naming/formatting churn before the CI regenerate-diff does.
        // To update intentionally: regenerate the examples, then copy the
        // block: awk '/^export interface OreCloseParams/,/^}\);$/' \
        //   examples/ore-typescript/src/generated/ore-stack.ts \
        //   > interpreter/tests/golden/ore-close-instruction.expected.ts
        let expected_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/golden/ore-close-instruction.expected.ts"
        );
        let expected = std::fs::read_to_string(expected_path)
            .expect("golden fixture should exist")
            .trim_end()
            .to_string();
        let start = code
            .find("export interface OreCloseParams")
            .expect("OreCloseParams block present");
        let end_marker = "});";
        let end = code[start..]
            .find(&format!(
                "export const oreCloseInstruction"
            ))
            .and_then(|handler_offset| {
                code[start + handler_offset..]
                    .find(end_marker)
                    .map(|e| start + handler_offset + e + end_marker.len())
            })
            .expect("oreCloseInstruction block terminates");
        let actual = code[start..end].trim_end();
        assert_eq!(
            actual, expected,
            "generated oreClose block diverged from the golden fixture"
        );
    }

    #[test]
    fn multi_program_scopes_errors_and_prefixes_names() {
        let idls = vec![
            idl(
                "ore",
                "Prog111111111111111111111111111111111111111",
                vec![IdlErrorSnapshot {
                    code: 0,
                    name: "OreBroke".to_string(),
                    msg: Some("ore broke".to_string()),
                }],
            ),
            idl(
                "entropy",
                "Prog222222222222222222222222222222222222222",
                vec![IdlErrorSnapshot {
                    code: 0,
                    name: "EntropyBroke".to_string(),
                    msg: Some("entropy broke".to_string()),
                }],
            ),
        ];

        let close = |program_id: &str| InstructionDef {
            name: "close".to_string(),
            discriminator: vec![9],
            discriminator_size: 1,
            accounts: vec![InstructionAccountDef {
                name: "signer".to_string(),
                is_signer: true,
                is_writable: true,
                resolution: AccountResolution::Signer,
                is_optional: false,
                docs: vec![],
            }],
            args: vec![],
            errors: vec![],
            program_id: Some(program_id.to_string()),
            docs: vec![],
        };
        let instructions = vec![
            close("Prog111111111111111111111111111111111111111"),
            close("Prog222222222222222222222222222222222222222"),
        ];

        let out = generate_instructions_code(
            "Demo",
            &instructions,
            &idls,
            &BTreeMap::new(),
            &[
                "Prog111111111111111111111111111111111111111".to_string(),
                "Prog222222222222222222222222222222222222222".to_string(),
            ],
            &HashSet::new(),
        );

        let code = &out.code;
        // Names prefixed per program; no collisions.
        assert!(code.contains("export const oreCloseInstruction"));
        assert!(code.contains("export const entropyCloseInstruction"));
        assert!(code.contains("export interface OreCloseParams"));
        assert!(code.contains("export interface EntropyCloseParams"));
        // Overlapping error code 0 is attributed per program, not deduped away.
        assert!(code.contains("DEMO_ORE_PROGRAM_ERRORS"));
        assert!(code.contains("DEMO_ENTROPY_PROGRAM_ERRORS"));
        assert!(code.contains("name: 'OreBroke'"));
        assert!(code.contains("name: 'EntropyBroke'"));
        // Each handler references its own program's errors.
        assert!(code.contains("errors: DEMO_ORE_PROGRAM_ERRORS"));
        assert!(code.contains("errors: DEMO_ENTROPY_PROGRAM_ERRORS"));

        assert_eq!(out.stack_entries.len(), 2);
        assert_eq!(out.stack_entries[0].program_key.as_deref(), Some("ore"));
        assert_eq!(out.stack_entries[1].program_key.as_deref(), Some("entropy"));

        let block = render_instructions_stack_block(&out.stack_entries);
        assert!(block.contains("    ore: {\n      close: oreCloseInstruction,\n    },"));
        assert!(block.contains("    entropy: {\n      close: entropyCloseInstruction,\n    },"));
    }

    #[test]
    fn multi_program_unmatched_instruction_falls_back_with_warning() {
        let idls = vec![
            idl("ore", "Prog111111111111111111111111111111111111111", vec![]),
            idl("entropy", "Prog222222222222222222222222222222222222222", vec![]),
        ];
        let instr = InstructionDef {
            name: "mystery".to_string(),
            discriminator: vec![1],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![],
            errors: vec![],
            program_id: None,
            docs: vec![],
        };

        let out = generate_instructions_code(
            "Demo",
            std::slice::from_ref(&instr),
            &idls,
            &BTreeMap::new(),
            &["Prog111111111111111111111111111111111111111".to_string()],
            &HashSet::new(),
        );

        assert!(out
            .warnings
            .iter()
            .any(|w| w.contains("could not be matched to a program IDL")));
        // Unmatched: unprefixed name, flat stack entry, stack-wide errors.
        assert!(out.code.contains("export const mysteryInstruction"));
        assert!(out.code.contains("errors: DEMO_PROGRAM_ERRORS"));
        assert_eq!(out.stack_entries[0].program_key, None);
    }
}
