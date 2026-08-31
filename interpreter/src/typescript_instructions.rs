//! TypeScript codegen for instruction-construction handlers.
//!
//! This module consumes the `InstructionDef[]` serialized into a stack spec and
//! emits data-driven instruction handlers that target the core SDK's
//! [`createInstructionHandler`] factory. No imperative serialization code is
//! generated: the output is metadata (discriminator, ordered accounts, arg
//! schema, errors) plus typed `Params`/`Error` shapes the core runtime
//! interprets.

use crate::ast::{
    AccountResolution, AmountDecimalsSource, IdlArrayElementSnapshot, IdlDefinedInnerSnapshot,
    IdlErrorSnapshot, IdlInstructionSnapshot, IdlSnapshot, IdlTypeDefKindSnapshot,
    IdlTypeDefSnapshot, IdlTypeSnapshot, InstructionAccountDef, InstructionDef, PdaDefinition,
    PdaProgramDef, PdaSeedDef,
};
use crate::typescript::{to_pascal_case, to_screaming_snake_case};
use arete_idl::{IdlAmountDecimalsSource, IdlAmountHint, IdlLengthPrefix};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// One entry in the stack definition's `instructions` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackInstructionEntry {
    /// Program namespace key (camelCase IDL name) for multi-program stacks.
    /// `None` for single-program stacks, where the block stays flat.
    pub program_key: Option<String>,
    /// Key inside the (possibly nested) instructions block.
    pub instruction_name: String,
    /// Program namespace key used at runtime on `client.programs.<key>`.
    pub runtime_program_key: Option<String>,
    /// Name of the generated handler const.
    pub handler_const: String,
    /// Name of the generated params interface for the handler.
    pub params_type: String,
    /// Name of the generated semantic params interface when amount hints are present.
    pub semantic_params_type: Option<String>,
    /// Extra params added only for semantic wrappers (for example decimals overrides).
    pub semantic_extra_params: Vec<String>,
    /// Root-arg conversions applied before delegating to the raw connected plan.
    pub semantic_amount_args: Vec<SemanticAmountArgEntry>,
    /// Whether the emitted semantic conversions reference the operation context.
    pub uses_operation_context: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticAmountArgEntry {
    pub arg_name: String,
    pub binding_name: String,
    pub raw_expression: String,
    pub uses_operation_context: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SemanticFieldSpec {
    root_arg_name: String,
    relative_path: Vec<String>,
    resolution: SemanticAmountResolution,
    decimals_override_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticAmountResolution {
    ArgMint {
        arg_name: String,
    },
    ArgDecimals {
        arg_name: String,
    },
    KnownAccount {
        account_name: String,
        optional: bool,
    },
    KnownAddress {
        address: String,
    },
    Constant {
        decimals: u8,
    },
}

impl SemanticAmountResolution {
    fn uses_operation_context(&self) -> bool {
        matches!(
            self,
            Self::ArgMint { .. } | Self::KnownAccount { .. } | Self::KnownAddress { .. }
        )
    }
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
    /// Whether generated program definitions need runtime semantic wrappers.
    pub needs_program_runtime_extensions: bool,
    /// Whether generated semantic wrappers reference `AmountInput`.
    pub needs_amount_input: bool,
    /// Whether generated semantic wrappers reference `resolveAmountToRaw`.
    pub needs_resolve_amount_to_raw: bool,
    /// Whether generated semantic wrappers reference `toRawAmount`.
    pub needs_to_raw_amount: bool,
    /// Whether an emitted semantic parameter interface references `BuildOptions`.
    pub needs_build_options: bool,
    /// Whether an emitted operation factory references `ProgramOperationContext`.
    pub needs_operation_context: bool,
    /// Human-readable warnings (skipped instructions, degraded PDAs).
    pub warnings: Vec<String>,
    /// Structured PDA degradation metadata for CLI summaries.
    pub pda_degradations: Vec<PdaDegradation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdaDegradationSource {
    Inline,
    Registry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdaDegradation {
    pub instruction_name: String,
    pub account_name: String,
    pub pda_name: Option<String>,
    pub source: PdaDegradationSource,
    pub reason: String,
}

impl PdaDegradation {
    pub fn warning_message(&self) -> String {
        match (&self.source, &self.pda_name) {
            (PdaDegradationSource::Inline, _) => format!(
                "instruction '{}': account '{}' inline PDA degraded to userProvided ({})",
                self.instruction_name, self.account_name, self.reason
            ),
            (_, Some(pda_name)) => format!(
                "instruction '{}': account '{}' PDA '{}' degraded to userProvided ({})",
                self.instruction_name, self.account_name, pda_name, self.reason
            ),
            (_, None) => format!(
                "instruction '{}': account '{}' degraded to userProvided ({})",
                self.instruction_name, self.account_name, self.reason
            ),
        }
    }
}

/// Per-program error const/type names plus the rendered declarations.
struct ProgramErrorScope {
    const_name: String,
    type_name: String,
    errors: Vec<IdlErrorSnapshot>,
    used: bool,
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
    idls: &'a [IdlSnapshot],
    program_index: Option<usize>,
) -> Option<&'a IdlInstructionSnapshot> {
    if let Some(index) = program_index {
        return idls.get(index).and_then(|idl| {
            idl.instructions
                .iter()
                .find(|candidate| instruction_snapshot_matches(instruction, candidate))
        });
    }

    let mut matches = idls
        .iter()
        .filter(|idl| {
            instruction
                .program_id
                .as_deref()
                .is_none_or(|program_id| idl.program_id.as_deref() == Some(program_id))
        })
        .flat_map(|idl| idl.instructions.iter())
        .filter(|candidate| instruction_snapshot_matches(instruction, candidate));
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first)
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
    let mut pda_degradations: Vec<PdaDegradation> = Vec::new();
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
    let mut needs_program_runtime_extensions = false;
    let mut needs_amount_input = false;
    let mut needs_resolve_amount_to_raw = false;
    let mut needs_to_raw_amount = false;
    let mut needs_build_options = false;
    let mut needs_operation_context = false;

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
            Some(name) => (
                to_pascal_case(&instr.name),
                format!("{}Instruction", instr.name),
                Some(to_camel_case(name)),
            ),
            _ => (
                to_pascal_case(&instr.name),
                format!("{}Instruction", instr.name),
                None,
            ),
        };
        let runtime_program_key = program_name.map(to_camel_case);
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

        let instruction_snapshot = find_instruction_snapshot(instr, idls, program_index);

        // --- Parse args; skip the whole instruction on unsupported types. ---
        let mut parsed_args: Vec<(String, ParsedArgType)> = Vec::new();
        let mut unsupported_arg: Option<(String, String)> = None;
        for (index, arg) in instr.args.iter().enumerate() {
            let parsed = instruction_snapshot
                .and_then(|snapshot| snapshot.args.get(index))
                .map(|snapshot_arg| defined_types.parse_snapshot_type(&snapshot_arg.type_))
                .unwrap_or_else(|| defined_types.parse_arg_type(&arg.arg_type));
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
        let account_name_map = disambiguate_instruction_account_names(instr);
        for (source_name, emitted_name) in &account_name_map {
            if source_name != emitted_name {
                warnings.push(format!(
                    "instruction '{}': account '{}' collides with an argument; exposed as '{}'",
                    instr.name, source_name, emitted_name
                ));
            }
        }

        let mut account_literals: Vec<String> = Vec::new();
        let mut user_params: Vec<UserParam> = Vec::new();
        let mut resolve_params: BTreeMap<String, String> = BTreeMap::new();
        for acc in &instr.accounts {
            let mut diagnostics = AccountMappingDiagnostics {
                warnings: &mut warnings,
                degradations: &mut pda_degradations,
            };
            let mapped = map_account(
                acc,
                &pda_lookup,
                &instr_account_names,
                &instr_arg_types,
                &account_name_map,
                &instr.name,
                &mut diagnostics,
            );
            account_literals.push(mapped.literal);
            if let Some(param) = mapped.param {
                user_params.push(param);
            }
            for resolve_param in mapped.resolve_params {
                resolve_params
                    .entry(resolve_param.name)
                    .or_insert(resolve_param.ts_type);
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
        if !resolve_params.is_empty() {
            let resolve_lines: Vec<String> = resolve_params
                .iter()
                .map(|(name, ts_type)| {
                    format!("    {}?: {};", render_ts_property_name(name), ts_type)
                })
                .collect();
            param_lines.push(format!(
                "  resolve?: {{\n{}\n  }};",
                resolve_lines.join("\n")
            ));
        }
        let params_body = if param_lines.is_empty() {
            "  // This instruction takes no arguments or user-provided accounts.".to_string()
        } else {
            param_lines.join("\n")
        };
        let params_type = defined_types.claim_generated_ts_name(
            &format!("{}Params", pascal),
            &format!("{}InstructionParams", pascal),
        );
        let params_interface = format!("export interface {} {{\n{}\n}}", params_type, params_body);

        let mut semantic_specs = collect_semantic_amount_args(
            instr,
            instruction_snapshot,
            &mut defined_types,
            &mut warnings,
        );
        semantic_specs.retain_mut(|spec| match spec.resolution.clone() {
            SemanticAmountResolution::KnownAccount { account_name, .. } => {
                let field_path = if spec.relative_path.is_empty() {
                    spec.root_arg_name.clone()
                } else {
                    format!("{}.{}", spec.root_arg_name, spec.relative_path.join("."))
                };
                let Some(account) = instr
                    .accounts
                    .iter()
                    .find(|account| account.name == account_name)
                else {
                    warnings.push(format!(
                        "instruction '{}': skipped amount-aware semantic wrapper for field '{}' because account '{}' was not found",
                        instr.name,
                        field_path,
                        account_name
                    ));
                    return false;
                };
                let emitted_account_name = account_name_map
                    .get(&account_name)
                    .cloned()
                    .unwrap_or_else(|| account_name.clone());
                if let Some(param) = user_params
                    .iter()
                    .find(|param| param.name == emitted_account_name)
                {
                    spec.resolution = SemanticAmountResolution::KnownAccount {
                        account_name: emitted_account_name,
                        optional: param.optional,
                    };
                    return true;
                }
                let AccountResolution::Known { address: known } = &account.resolution else {
                    warnings.push(format!(
                        "instruction '{}': skipped amount-aware semantic wrapper for field '{}' because account '{}' is neither caller-provided nor a known address",
                        instr.name,
                        field_path,
                        account_name
                    ));
                    return false;
                };
                spec.resolution = SemanticAmountResolution::KnownAddress {
                    address: known.clone(),
                };
                true
            }
            _ => true,
        });
        if runtime_program_key.is_some() {
            needs_program_runtime_extensions = true;
        }
        if !semantic_specs.is_empty() && runtime_program_key.is_none() {
            warnings.push(format!(
                "instruction '{}': skipped amount-aware semantic wrapper because the runtime program namespace could not be determined",
                instr.name
            ));
            semantic_specs.clear();
        }

        let semantic_extra_params: Vec<String> = semantic_specs
            .iter()
            .filter_map(|spec| spec.decimals_override_name.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        let semantic_amount_args = if semantic_specs.is_empty() {
            Vec::new()
        } else {
            needs_amount_input = true;
            if semantic_specs
                .iter()
                .any(|spec| spec.resolution.uses_operation_context())
            {
                needs_resolve_amount_to_raw = true;
            }
            if semantic_specs.iter().any(|spec| {
                matches!(
                    spec.resolution,
                    SemanticAmountResolution::ArgDecimals { .. }
                        | SemanticAmountResolution::Constant { .. }
                )
            }) {
                needs_to_raw_amount = true;
            }

            let mut conversions = Vec::new();
            for (arg_name, _) in &parsed_args {
                let arg_specs: Vec<&SemanticFieldSpec> = semantic_specs
                    .iter()
                    .filter(|spec| spec.root_arg_name == *arg_name)
                    .collect();
                if arg_specs.is_empty() {
                    continue;
                }

                let arg_access = render_ts_property_access("params", arg_name);
                let raw_expression = if let Some(snapshot_arg) = instruction_snapshot
                    .and_then(|snapshot| snapshot.args.iter().find(|arg| arg.name == *arg_name))
                {
                    render_semantic_raw_expression(
                        &snapshot_arg.type_,
                        &arg_access,
                        &mut defined_types,
                        &arg_specs,
                        0,
                    )
                    .unwrap_or_else(|| arg_access.clone())
                } else {
                    render_amount_resolution_expression(&arg_access, arg_specs[0])
                };

                conversions.push(SemanticAmountArgEntry {
                    arg_name: arg_name.clone(),
                    binding_name: semantic_raw_binding_name(arg_name),
                    raw_expression,
                    uses_operation_context: arg_specs
                        .iter()
                        .any(|spec| spec.resolution.uses_operation_context()),
                });
            }
            conversions
        };

        let semantic_params = if runtime_program_key.is_none() {
            None
        } else if semantic_specs.is_empty() {
            Some((params_type.clone(), None))
        } else {
            needs_build_options = true;
            let type_name = defined_types.claim_generated_ts_name(
                &format!("{}SemanticParams", pascal),
                &format!("{}OperationParams", pascal),
            );
            let interface = render_semantic_params_interface(
                &type_name,
                &parsed_args,
                instruction_snapshot,
                &user_params,
                &resolve_params,
                &semantic_specs,
                &mut defined_types,
            );
            Some((type_name, Some(interface)))
        };
        let uses_operation_context = semantic_amount_args
            .iter()
            .any(|amount_arg| amount_arg.uses_operation_context);
        needs_operation_context |= uses_operation_context;

        // --- Error type. Program errors are stack-wide (IDLs do not scope
        // errors to instructions), so each handler's typed error is an alias of
        // the program-wide union. ---
        let error_type = defined_types.claim_generated_ts_name(
            &format!("{}Error", pascal),
            &format!("{}InstructionError", pascal),
        );
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

        let program_id = instr
            .program_id
            .clone()
            .unwrap_or_else(|| default_program_id.clone());
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

        let mut block_parts = vec![params_interface];
        if let Some((_, Some(semantic_interface))) = &semantic_params {
            block_parts.push(semantic_interface.clone());
        }
        block_parts.push(error_decl);
        block_parts.push(handler);
        blocks.push(block_parts.join("\n\n"));
        // Mark the error scope as referenced only when a handler is emitted,
        // so fully-skipped programs do not produce dangling consts.
        match program_index {
            Some(i) => program_scopes[i].used = true,
            None => fallback_scope.used = true,
        }
        stack_entries.push(StackInstructionEntry {
            program_key,
            instruction_name: instr.name.clone(),
            runtime_program_key,
            handler_const,
            params_type: params_type.clone(),
            semantic_params_type: semantic_params.as_ref().map(|(name, _)| name.clone()),
            semantic_extra_params,
            semantic_amount_args,
            uses_operation_context,
        });
    }

    warnings.append(&mut defined_types.warnings);

    if stack_entries.is_empty() {
        // Nothing emittable (all instructions skipped). Still surface warnings.
        return InstructionsCodegen {
            code: String::new(),
            stack_entries,
            needs_runtime_import: false,
            needs_program_runtime_extensions,
            needs_amount_input,
            needs_resolve_amount_to_raw,
            needs_to_raw_amount,
            needs_build_options,
            needs_operation_context,
            warnings,
            pda_degradations,
        };
    }

    // Program-level error metadata blocks, one per referenced scope. Errors
    // live on the stack's IDL snapshots (not duplicated onto instructions).
    let mut error_blocks: Vec<String> = Vec::new();
    for scope in program_scopes
        .iter()
        .chain(std::iter::once(&fallback_scope))
    {
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
        needs_program_runtime_extensions,
        needs_amount_input,
        needs_resolve_amount_to_raw,
        needs_to_raw_amount,
        needs_build_options,
        needs_operation_context,
        warnings,
        pda_degradations,
    }
}

/// Render the `rawInstructions: { ... }` block for the stack definition const.
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

    format!("\n  rawInstructions: {{\n{}\n  }},", lines.join("\n"))
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

    fn claim_generated_ts_name(&mut self, preferred: &str, fallback: &str) -> String {
        let declared_names = self
            .defs
            .keys()
            .map(|name| to_pascal_case(name))
            .collect::<HashSet<_>>();
        let available = |candidate: &str, taken: &HashSet<String>| {
            !taken.contains(candidate) && !declared_names.contains(candidate)
        };

        let mut candidate = if available(preferred, &self.taken_names) {
            preferred.to_string()
        } else {
            fallback.to_string()
        };
        let mut counter = 2;
        while !available(&candidate, &self.taken_names) {
            candidate = format!("{}{}", fallback, counter);
            counter += 1;
        }
        if candidate != preferred {
            self.warnings.push(format!(
                "generated identifier '{}' collides with another export; emitted as '{}'",
                preferred, candidate
            ));
        }
        self.taken_names.insert(candidate.clone());
        candidate
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
                "VecU64Len" => {
                    let inner = self.parse_arg_type(inner);
                    return ParsedArgType {
                        schema: format!("{{ vecU64Len: {} }}", inner.schema),
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
                let key = match v.length_prefix {
                    Some(IdlLengthPrefix::U64) => "vecU64Len",
                    _ => "vec",
                };
                ParsedArgType {
                    schema: format!("{{ {}: {} }}", key, inner.schema),
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
            IdlTypeSnapshot::HashMap(map) => {
                let key = self.parse_snapshot_type(&map.hash_map.0);
                let value = self.parse_snapshot_type(&map.hash_map.1);
                if !key.supported || key.schema != "'string'" || !value.supported {
                    unsupported()
                } else {
                    ParsedArgType {
                        schema: format!("{{ hashMap: [{}, {}] }}", key.schema, value.schema),
                        ts_type: format!("Record<string, {}>", value.ts_type),
                        supported: true,
                    }
                }
            }
            // The TS instruction-args schema runtime has no tuple encoding
            // yet, so tuple args degrade to the explicit unsupported marker
            // (same as exotic hashMap keys) instead of emitting a schema the
            // runtime cannot serialize.
            IdlTypeSnapshot::Tuple(_) => unsupported(),
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

    fn find_definition(&self, name: &str) -> Option<&'a IdlTypeDefSnapshot> {
        if let Some(def) = self.defs.get(name) {
            return Some(*def);
        }
        self.lower
            .get(&name.to_lowercase())
            .and_then(|canonical| self.defs.get(canonical).copied())
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
            schema_fields.push(format!(
                "{{ name: '{}', type: {} }}",
                field.name, parsed.schema
            ));
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
                    field_schemas.push(format!(
                        "{{ name: '{}', type: {} }}",
                        field.name, parsed.schema
                    ));
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
pub(crate) fn split_generic(t: &str) -> Option<(&str, &str)> {
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

/// A helper-only PDA seed input exposed under `resolve`.
#[derive(Debug, Clone)]
struct ResolveParam {
    name: String,
    ts_type: String,
}

/// Result of mapping a single instruction account.
struct MappedAccount {
    /// TypeScript `AccountMeta` object literal.
    literal: String,
    /// Set when the account is caller-supplied (`userProvided`).
    param: Option<UserParam>,
    /// Helper-only PDA seed inputs needed for derivation.
    resolve_params: Vec<ResolveParam>,
}

struct AccountMappingDiagnostics<'a> {
    warnings: &'a mut Vec<String>,
    degradations: &'a mut Vec<PdaDegradation>,
}

/// Allocate the merged-parameter name used for each instruction account.
///
/// Instruction arguments and caller-provided accounts share one object at
/// runtime. Keep every non-conflicting account name unchanged, then suffix an
/// account that collides with an argument (`metadata` -> `metadataAccount`).
/// Reserving the unchanged names first prevents the suffix from stealing a
/// real account's name.
pub(crate) fn disambiguate_instruction_account_names(
    instruction: &InstructionDef,
) -> BTreeMap<String, String> {
    let argument_names = instruction
        .args
        .iter()
        .map(|arg| arg.name.as_str())
        .collect::<HashSet<_>>();
    let mut used_names = argument_names
        .iter()
        .map(|name| (*name).to_string())
        .collect::<HashSet<_>>();
    let mut names = BTreeMap::new();

    for account in &instruction.accounts {
        if !argument_names.contains(account.name.as_str())
            && used_names.insert(account.name.clone())
        {
            names.insert(account.name.clone(), account.name.clone());
        }
    }

    for account in &instruction.accounts {
        if names.contains_key(&account.name) {
            continue;
        }

        let base_name = format!("{}Account", account.name);
        let mut emitted_name = base_name.clone();
        let mut suffix = 2;
        while !used_names.insert(emitted_name.clone()) {
            emitted_name = format!("{}{}", base_name, suffix);
            suffix += 1;
        }
        names.insert(account.name.clone(), emitted_name);
    }

    names
}

fn map_account(
    acc: &InstructionAccountDef,
    pda_lookup: &BTreeMap<&str, &PdaDefinition>,
    instr_account_names: &HashSet<&str>,
    instr_arg_types: &BTreeMap<&str, &str>,
    account_name_map: &BTreeMap<String, String>,
    instr_name: &str,
    diagnostics: &mut AccountMappingDiagnostics<'_>,
) -> MappedAccount {
    let emitted_name = account_name_map
        .get(&acc.name)
        .map(String::as_str)
        .unwrap_or(&acc.name);
    let base = format!(
        "name: '{}', isSigner: {}, isWritable: {}",
        emitted_name, acc.is_signer, acc.is_writable
    );
    let optional_suffix = if acc.is_optional {
        ", isOptional: true".to_string()
    } else {
        String::new()
    };

    let user_provided = |degradation: Option<PdaDegradation>,
                         warnings: &mut Vec<String>,
                         degradations: &mut Vec<PdaDegradation>|
     -> MappedAccount {
        // Degradations are surfaced both to the compiler caller (warnings) and
        // in the generated code, so SDK readers can see why an account that
        // looks derivable must be passed in manually.
        let comment = match &degradation {
            Some(degradation) => {
                format!("    // [arete codegen] {}\n", degradation.warning_message())
            }
            None => String::new(),
        };
        if let Some(degradation) = degradation {
            warnings.push(degradation.warning_message());
            degradations.push(degradation);
        }
        MappedAccount {
            literal: format!(
                "{}    {{ {}, category: 'userProvided'{} }},",
                comment, base, optional_suffix
            ),
            param: Some(UserParam {
                name: emitted_name.to_string(),
                optional: acc.is_optional,
            }),
            resolve_params: Vec::new(),
        }
    };

    match &acc.resolution {
        AccountResolution::Signer => MappedAccount {
            literal: format!(
                "    {{ {}, category: 'signer', signerKind: 'provided'{} }},",
                base, optional_suffix
            ),
            param: Some(UserParam {
                name: emitted_name.to_string(),
                optional: acc.is_optional,
            }),
            resolve_params: Vec::new(),
        },
        AccountResolution::Known { address } => MappedAccount {
            literal: format!(
                "    {{ {}, category: 'known', knownAddress: '{}'{} }},",
                base, address, optional_suffix
            ),
            param: None,
            resolve_params: Vec::new(),
        },
        AccountResolution::UserProvided => user_provided(
            None,
            &mut *diagnostics.warnings,
            &mut *diagnostics.degradations,
        ),
        AccountResolution::PdaInline {
            seeds,
            program_id,
            program,
        } => {
            match build_pda_config(
                seeds,
                program_id.as_deref(),
                program.as_ref(),
                instr_account_names,
                instr_arg_types,
                account_name_map,
            ) {
                Ok((pda_config, seed_warnings, resolve_params)) => {
                    for w in seed_warnings {
                        diagnostics.warnings.push(format!(
                            "instruction '{}': account '{}': {}",
                            instr_name, acc.name, w
                        ));
                    }
                    MappedAccount {
                        literal: format!(
                            "    {{ {}, category: 'pda', pdaConfig: {}{} }},",
                            base, pda_config, optional_suffix
                        ),
                        param: Some(UserParam {
                            name: emitted_name.to_string(),
                            optional: true,
                        }),
                        resolve_params,
                    }
                }
                Err(reason) => user_provided(
                    Some(PdaDegradation {
                        instruction_name: instr_name.to_string(),
                        account_name: acc.name.clone(),
                        pda_name: None,
                        source: PdaDegradationSource::Inline,
                        reason,
                    }),
                    &mut *diagnostics.warnings,
                    &mut *diagnostics.degradations,
                ),
            }
        }
        AccountResolution::PdaRef { pda_name } => match pda_lookup.get(pda_name.as_str()) {
            Some(def) => match build_pda_config(
                &def.seeds,
                def.program_id.as_deref(),
                def.program.as_ref(),
                instr_account_names,
                instr_arg_types,
                account_name_map,
            ) {
                Ok((pda_config, seed_warnings, resolve_params)) => {
                    for w in seed_warnings {
                        diagnostics.warnings.push(format!(
                            "instruction '{}': account '{}': {}",
                            instr_name, acc.name, w
                        ));
                    }
                    MappedAccount {
                        literal: format!(
                            "    {{ {}, category: 'pda', pdaConfig: {}{} }},",
                            base, pda_config, optional_suffix
                        ),
                        param: Some(UserParam {
                            name: emitted_name.to_string(),
                            optional: true,
                        }),
                        resolve_params,
                    }
                }
                Err(reason) => user_provided(
                    Some(PdaDegradation {
                        instruction_name: instr_name.to_string(),
                        account_name: acc.name.clone(),
                        pda_name: Some(pda_name.clone()),
                        source: PdaDegradationSource::Registry,
                        reason,
                    }),
                    &mut *diagnostics.warnings,
                    &mut *diagnostics.degradations,
                ),
            },
            None => user_provided(
                Some(PdaDegradation {
                    instruction_name: instr_name.to_string(),
                    account_name: acc.name.clone(),
                    pda_name: Some(pda_name.clone()),
                    source: PdaDegradationSource::Registry,
                    reason: format!("references unknown PDA '{}'", pda_name),
                }),
                &mut *diagnostics.warnings,
                &mut *diagnostics.degradations,
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
    program: Option<&PdaProgramDef>,
    instr_account_names: &HashSet<&str>,
    instr_arg_types: &BTreeMap<&str, &str>,
    account_name_map: &BTreeMap<String, String>,
) -> Result<(String, Vec<String>, Vec<ResolveParam>), String> {
    let mut seed_literals: Vec<String> = Vec::new();
    let mut soft_warnings: Vec<String> = Vec::new();
    let mut resolve_params: Vec<ResolveParam> = Vec::new();
    for seed in seeds {
        match seed {
            PdaSeedDef::Literal { value } => {
                seed_literals.push(format!(
                    "{{ type: 'literal', value: '{}' }}",
                    escape_single_quotes(value)
                ));
            }
            PdaSeedDef::AccountRef { account_name } => {
                if account_name.contains('.') {
                    return Err(format!(
                        "seed references account field '{}' which is not supported for low-level auto-resolution; encode it as a typed helper arg instead",
                        account_name
                    ));
                }
                if !instr_account_names.contains(account_name.as_str()) {
                    return Err(format!(
                        "seed references account '{}' not present in this instruction",
                        account_name
                    ));
                }
                seed_literals.push(format!(
                    "{{ type: 'accountRef', accountName: '{}' }}",
                    account_name_map.get(account_name).unwrap_or(account_name)
                ));
            }
            PdaSeedDef::ArgRef { arg_name, arg_type } => {
                let arg_root = arg_name.split('.').next().unwrap_or(arg_name.as_str());
                let present_in_args = instr_arg_types.contains_key(arg_name.as_str())
                    || instr_arg_types.contains_key(arg_root);
                // Prefer the seed's declared type; fall back to the
                // instruction arg's type (Anchor seeds carry no type info).
                let raw_type = arg_type
                    .as_deref()
                    .or_else(|| instr_arg_types.get(arg_name.as_str()).copied())
                    .or_else(|| instr_arg_types.get(arg_root).copied());
                if !present_in_args {
                    let helper_type = match raw_type {
                        Some(raw) => {
                            let Some(ts_type) = seed_arg_ts_type(raw) else {
                                return Err(format!(
                                    "seed helper arg '{}' needs an explicit primitive type; found unsupported type '{}'",
                                    arg_name, raw
                                ));
                            };
                            ts_type
                        }
                        None => {
                            return Err(format!(
                                "seed helper arg '{}' is not present in this instruction and has no type information",
                                arg_name
                            ))
                        }
                    };
                    resolve_params.push(ResolveParam {
                        name: arg_name.clone(),
                        ts_type: helper_type,
                    });
                }
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
    let program_field = match program {
        Some(PdaProgramDef::AccountRef { account_name }) => {
            if !instr_account_names.contains(account_name.as_str()) {
                return Err(format!(
                    "PDA program references account '{}' not present in this instruction",
                    account_name
                ));
            }
            format!(
                "program: {{ type: 'accountRef', accountName: '{}' }}, ",
                account_name_map.get(account_name).unwrap_or(account_name)
            )
        }
        Some(PdaProgramDef::ArgRef { arg_name }) => {
            let arg_root = arg_name.split('.').next().unwrap_or(arg_name.as_str());
            if !instr_arg_types.contains_key(arg_name.as_str())
                && !instr_arg_types.contains_key(arg_root)
            {
                return Err(format!(
                    "PDA program references argument '{}' not present in this instruction",
                    arg_name
                ));
            }
            format!("program: {{ type: 'argRef', argName: '{}' }}, ", arg_name)
        }
        None => String::new(),
    };
    let static_program_field = program_id
        .map(|pid| format!("programId: '{}', ", pid))
        .unwrap_or_default();
    let config = format!(
        "{{ {}{}seeds: [{}] }}",
        static_program_field, program_field, seeds_str
    );
    Ok((config, soft_warnings, resolve_params))
}

fn render_ts_property_name(name: &str) -> String {
    if name
        .chars()
        .next()
        .map(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        .unwrap_or(false)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    {
        name.to_string()
    } else {
        format!("'{}'", escape_single_quotes(name))
    }
}

fn lower_first(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn semantic_path_identifier(path: &[String], suffix: &str) -> String {
    let joined = path
        .iter()
        .map(|segment| to_pascal_case(segment))
        .collect::<String>();
    format!("{}{}", lower_first(&joined), suffix)
}

fn semantic_decimals_override_name(path: &[String]) -> String {
    semantic_path_identifier(path, "Decimals")
}

fn semantic_raw_binding_name(arg_name: &str) -> String {
    semantic_path_identifier(&[arg_name.to_string()], "Raw")
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

fn render_ts_property_access(target: &str, property: &str) -> String {
    if is_valid_ts_identifier(property) {
        format!("{}.{}", target, property)
    } else {
        format!("{}['{}']", target, escape_single_quotes(property))
    }
}

fn render_ts_path_access(target: &str, path: &str) -> String {
    path.split('.')
        .filter(|segment| !segment.is_empty())
        .fold(target.to_string(), |acc, segment| {
            render_ts_property_access(&acc, segment)
        })
}

fn semantic_resolution_from_amount_source(
    source: &AmountDecimalsSource,
) -> SemanticAmountResolution {
    match source {
        AmountDecimalsSource::ArgMint { arg_name } => SemanticAmountResolution::ArgMint {
            arg_name: arg_name.clone(),
        },
        AmountDecimalsSource::ArgDecimals { arg_name } => SemanticAmountResolution::ArgDecimals {
            arg_name: arg_name.clone(),
        },
        AmountDecimalsSource::KnownAccount { account_name } => {
            SemanticAmountResolution::KnownAccount {
                account_name: account_name.clone(),
                optional: false,
            }
        }
        AmountDecimalsSource::Constant { decimals } => SemanticAmountResolution::Constant {
            decimals: *decimals,
        },
    }
}

fn semantic_resolution_from_idl_hint(hint: &IdlAmountHint) -> SemanticAmountResolution {
    match &hint.decimals_source {
        IdlAmountDecimalsSource::ArgMint { arg_name } => SemanticAmountResolution::ArgMint {
            arg_name: arg_name.clone(),
        },
        IdlAmountDecimalsSource::ArgDecimals { arg_name } => {
            SemanticAmountResolution::ArgDecimals {
                arg_name: arg_name.clone(),
            }
        }
        IdlAmountDecimalsSource::KnownAccount { account_name } => {
            SemanticAmountResolution::KnownAccount {
                account_name: account_name.clone(),
                optional: false,
            }
        }
        IdlAmountDecimalsSource::Constant { decimals } => SemanticAmountResolution::Constant {
            decimals: *decimals,
        },
    }
}

fn semantic_needs_decimals_override(resolution: &SemanticAmountResolution) -> bool {
    matches!(
        resolution,
        SemanticAmountResolution::ArgMint { .. }
            | SemanticAmountResolution::KnownAccount { .. }
            | SemanticAmountResolution::KnownAddress { .. }
    )
}

fn collect_semantic_specs_from_type(
    root_arg_name: &str,
    relative_path: &[String],
    ty: &IdlTypeSnapshot,
    defined_types: &mut DefinedTypes<'_>,
    warnings: &mut Vec<String>,
    specs: &mut Vec<SemanticFieldSpec>,
) {
    match ty {
        IdlTypeSnapshot::Vec(vec_type) => {
            collect_semantic_specs_from_type(
                root_arg_name,
                relative_path,
                &vec_type.vec,
                defined_types,
                warnings,
                specs,
            );
            return;
        }
        IdlTypeSnapshot::Array(array_type) => {
            for part in &array_type.array {
                match part {
                    IdlArrayElementSnapshot::Type(element) => collect_semantic_specs_from_type(
                        root_arg_name,
                        relative_path,
                        element,
                        defined_types,
                        warnings,
                        specs,
                    ),
                    IdlArrayElementSnapshot::TypeName(name) => {
                        collect_semantic_specs_from_type(
                            root_arg_name,
                            relative_path,
                            &IdlTypeSnapshot::Simple(name.clone()),
                            defined_types,
                            warnings,
                            specs,
                        );
                    }
                    IdlArrayElementSnapshot::Size(_) => {}
                }
            }
            return;
        }
        _ => {}
    }

    let IdlTypeSnapshot::Defined(defined) = ty else {
        return;
    };
    let type_name = match &defined.defined {
        IdlDefinedInnerSnapshot::Named { name } => name.as_str(),
        IdlDefinedInnerSnapshot::Simple(simple) => simple.as_str(),
    };
    let Some(def) = defined_types.find_definition(type_name) else {
        return;
    };

    match &def.type_def {
        IdlTypeDefKindSnapshot::Struct { fields, .. } => {
            let fields = fields.clone();
            for field in &fields {
                let mut child_path = relative_path.to_vec();
                child_path.push(field.name.clone());
                collect_semantic_specs_from_field(
                    root_arg_name,
                    &child_path,
                    field,
                    defined_types,
                    warnings,
                    specs,
                );
            }
        }
        IdlTypeDefKindSnapshot::Enum { variants, .. } => {
            let variants = variants.clone();
            for variant in &variants {
                for field in &variant.fields {
                    let crate::ast::IdlEnumVariantFieldSnapshot::Named(named) = field else {
                        continue;
                    };
                    let mut child_path = relative_path.to_vec();
                    child_path.push(variant.name.clone());
                    child_path.push(named.name.clone());
                    collect_semantic_specs_from_field(
                        root_arg_name,
                        &child_path,
                        named,
                        defined_types,
                        warnings,
                        specs,
                    );
                }
            }
        }
        IdlTypeDefKindSnapshot::TupleStruct { .. } => {}
    }
}

fn collect_semantic_specs_from_field(
    root_arg_name: &str,
    relative_path: &[String],
    field: &crate::ast::IdlFieldSnapshot,
    defined_types: &mut DefinedTypes<'_>,
    warnings: &mut Vec<String>,
    specs: &mut Vec<SemanticFieldSpec>,
) {
    if let Some(hint) = &field.amount_hint {
        let parsed = defined_types.parse_snapshot_type(&field.type_);
        if parsed.ts_type != "bigint" {
            let full_path = std::iter::once(root_arg_name)
                .chain(relative_path.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(".");
            warnings.push(format!(
                "instruction semantic wrapper: skipped amount-aware field '{}' because only bigint-backed raw fields are currently supported",
                full_path
            ));
            return;
        }

        let resolution = semantic_resolution_from_idl_hint(hint);
        let mut full_path = vec![root_arg_name.to_string()];
        full_path.extend(relative_path.iter().cloned());
        let decimals_override_name = if semantic_needs_decimals_override(&resolution) {
            Some(semantic_decimals_override_name(&full_path))
        } else {
            None
        };
        specs.push(SemanticFieldSpec {
            root_arg_name: root_arg_name.to_string(),
            relative_path: relative_path.to_vec(),
            resolution,
            decimals_override_name,
        });
        return;
    }

    collect_semantic_specs_from_type(
        root_arg_name,
        relative_path,
        &field.type_,
        defined_types,
        warnings,
        specs,
    );
}

fn collect_semantic_amount_args(
    instr: &InstructionDef,
    instruction_snapshot: Option<&IdlInstructionSnapshot>,
    defined_types: &mut DefinedTypes<'_>,
    warnings: &mut Vec<String>,
) -> Vec<SemanticFieldSpec> {
    if let Some(snapshot) = instruction_snapshot {
        let mut specs = Vec::new();
        for arg in &snapshot.args {
            collect_semantic_specs_from_field(
                &arg.name,
                &[],
                arg,
                defined_types,
                warnings,
                &mut specs,
            );
        }
        if !specs.is_empty() {
            return specs;
        }
    }

    let mut specs = Vec::new();
    for arg in &instr.args {
        let Some(hint) = &arg.amount_hint else {
            continue;
        };
        let parsed = defined_types.parse_arg_type(&arg.arg_type);
        if parsed.ts_type != "bigint" {
            warnings.push(format!(
                "instruction '{}': skipped amount-aware semantic wrapper for arg '{}' because only bigint-backed raw args are currently supported",
                instr.name, arg.name
            ));
            continue;
        }
        let resolution = semantic_resolution_from_amount_source(&hint.decimals_source);
        let path = vec![arg.name.clone()];
        specs.push(SemanticFieldSpec {
            root_arg_name: arg.name.clone(),
            relative_path: Vec::new(),
            decimals_override_name: semantic_needs_decimals_override(&resolution)
                .then(|| semantic_decimals_override_name(&path)),
            resolution,
        });
    }

    specs
}

fn render_amount_resolution_expression(input_expr: &str, spec: &SemanticFieldSpec) -> String {
    let render_source_path = |path: &str| {
        if let Some((_, element_path)) = path.rsplit_once("[]") {
            let element_path = element_path.trim_start_matches('.');
            if element_path.is_empty() {
                "entry".to_string()
            } else {
                render_ts_path_access("entry", element_path)
            }
        } else {
            render_ts_path_access("params", path)
        }
    };
    match &spec.resolution {
        SemanticAmountResolution::ArgMint { arg_name } => format!(
            "await resolveAmountToRaw(context.chain, {{ mint: {}, amount: {}, decimals: {} }})",
            render_source_path(arg_name),
            input_expr,
            render_ts_property_access(
                "params",
                spec.decimals_override_name
                    .as_deref()
                    .expect("mint-based amount hints should declare an override field")
            )
        ),
        SemanticAmountResolution::ArgDecimals { arg_name } => format!(
            "toRawAmount({}, {})",
            input_expr,
            render_source_path(arg_name)
        ),
        SemanticAmountResolution::KnownAccount {
            account_name,
            optional,
        } => format!(
            "await resolveAmountToRaw(context.chain, {{ mint: {}, amount: {}, decimals: {} }})",
            if *optional {
                format!(
                    "({} ?? '')",
                    render_ts_property_access("params", account_name)
                )
            } else {
                render_ts_property_access("params", account_name)
            },
            input_expr,
            render_ts_property_access(
                "params",
                spec.decimals_override_name
                    .as_deref()
                    .expect("known-account amount hints should declare an override field")
            )
        ),
        SemanticAmountResolution::KnownAddress { address } => format!(
            "await resolveAmountToRaw(context.chain, {{ mint: '{}', amount: {}, decimals: {} }})",
            escape_single_quotes(address),
            input_expr,
            render_ts_property_access(
                "params",
                spec.decimals_override_name
                    .as_deref()
                    .expect("known-account amount hints should declare an override field")
            )
        ),
        SemanticAmountResolution::Constant { decimals } => {
            format!("toRawAmount({}, {})", input_expr, decimals)
        }
    }
}

fn render_semantic_ts_type(
    ty: &IdlTypeSnapshot,
    defined_types: &mut DefinedTypes<'_>,
    specs: &[&SemanticFieldSpec],
    depth: usize,
) -> Option<String> {
    if specs.iter().any(|spec| spec.relative_path.len() == depth) {
        return Some("AmountInput".to_string());
    }
    if !specs.iter().any(|spec| spec.relative_path.len() > depth) {
        return Some(defined_types.parse_snapshot_type(ty).ts_type);
    }

    match ty {
        IdlTypeSnapshot::Vec(vec_type) => {
            let inner = render_semantic_ts_type(&vec_type.vec, defined_types, specs, depth)?;
            return Some(format!("{}[]", maybe_paren(&inner)));
        }
        IdlTypeSnapshot::Array(array_type) => {
            for part in &array_type.array {
                let inner = match part {
                    IdlArrayElementSnapshot::Type(element) => {
                        render_semantic_ts_type(element, defined_types, specs, depth)?
                    }
                    IdlArrayElementSnapshot::TypeName(name) => render_semantic_ts_type(
                        &IdlTypeSnapshot::Simple(name.clone()),
                        defined_types,
                        specs,
                        depth,
                    )?,
                    IdlArrayElementSnapshot::Size(_) => continue,
                };
                return Some(format!("{}[]", maybe_paren(&inner)));
            }
            return Some(defined_types.parse_snapshot_type(ty).ts_type);
        }
        _ => {}
    }

    let IdlTypeSnapshot::Defined(defined) = ty else {
        return Some(defined_types.parse_snapshot_type(ty).ts_type);
    };
    let type_name = match &defined.defined {
        IdlDefinedInnerSnapshot::Named { name } => name.as_str(),
        IdlDefinedInnerSnapshot::Simple(simple) => simple.as_str(),
    };
    let Some(def) = defined_types.find_definition(type_name) else {
        return Some(defined_types.parse_snapshot_type(ty).ts_type);
    };

    match &def.type_def {
        IdlTypeDefKindSnapshot::Struct { fields, .. } => {
            render_semantic_struct_type(fields, defined_types, specs, depth)
        }
        IdlTypeDefKindSnapshot::Enum { variants, .. } => {
            render_semantic_enum_type(variants, defined_types, specs, depth)
        }
        IdlTypeDefKindSnapshot::TupleStruct { .. } => {
            Some(defined_types.parse_snapshot_type(ty).ts_type)
        }
    }
}

fn render_semantic_struct_type(
    fields: &[crate::ast::IdlFieldSnapshot],
    defined_types: &mut DefinedTypes<'_>,
    specs: &[&SemanticFieldSpec],
    depth: usize,
) -> Option<String> {
    let mut field_entries: Vec<String> = Vec::new();
    for field in fields {
        let child_specs: Vec<&SemanticFieldSpec> = specs
            .iter()
            .copied()
            .filter(|spec| spec.relative_path.get(depth) == Some(&field.name))
            .collect();
        let field_ts = if child_specs.is_empty() {
            defined_types.parse_snapshot_type(&field.type_).ts_type
        } else {
            render_semantic_ts_type(&field.type_, defined_types, &child_specs, depth + 1)?
        };
        field_entries.push(format!(
            "{}: {};",
            render_ts_property_name(&field.name),
            field_ts
        ));
    }
    Some(format!("{{ {} }}", field_entries.join(" ")))
}

fn render_semantic_enum_type(
    variants: &[crate::ast::IdlEnumVariantSnapshot],
    defined_types: &mut DefinedTypes<'_>,
    specs: &[&SemanticFieldSpec],
    depth: usize,
) -> Option<String> {
    use crate::ast::IdlEnumVariantFieldSnapshot;

    let mut rendered_variants: Vec<String> = Vec::new();
    for variant in variants {
        if variant.fields.is_empty() {
            rendered_variants.push(format!("'{}'", variant.name));
            continue;
        }

        let named: Vec<_> = variant
            .fields
            .iter()
            .filter_map(|field| match field {
                IdlEnumVariantFieldSnapshot::Named(named) => Some(named),
                IdlEnumVariantFieldSnapshot::Tuple(_) => None,
            })
            .collect();
        if named.len() == variant.fields.len() {
            let mut field_entries: Vec<String> = Vec::new();
            for field in named {
                let child_specs: Vec<&SemanticFieldSpec> = specs
                    .iter()
                    .copied()
                    .filter(|spec| {
                        spec.relative_path.get(depth) == Some(&variant.name)
                            && spec.relative_path.get(depth + 1) == Some(&field.name)
                    })
                    .collect();
                let field_ts = if child_specs.is_empty() {
                    defined_types.parse_snapshot_type(&field.type_).ts_type
                } else {
                    render_semantic_ts_type(&field.type_, defined_types, &child_specs, depth + 2)?
                };
                field_entries.push(format!(
                    "{}: {};",
                    render_ts_property_name(&field.name),
                    field_ts
                ));
            }
            rendered_variants.push(format!(
                "{{ {}: {{ {} }} }}",
                render_ts_property_name(&variant.name),
                field_entries.join(" ")
            ));
            continue;
        }

        let tuple: Vec<_> = variant
            .fields
            .iter()
            .filter_map(|field| match field {
                IdlEnumVariantFieldSnapshot::Tuple(ty) => {
                    Some(defined_types.parse_snapshot_type(ty).ts_type)
                }
                IdlEnumVariantFieldSnapshot::Named(_) => None,
            })
            .collect();
        rendered_variants.push(format!(
            "{{ {}: [{}] }}",
            render_ts_property_name(&variant.name),
            tuple.join(", ")
        ));
    }
    Some(rendered_variants.join(" | "))
}

fn render_semantic_raw_expression(
    ty: &IdlTypeSnapshot,
    value_expr: &str,
    defined_types: &mut DefinedTypes<'_>,
    specs: &[&SemanticFieldSpec],
    depth: usize,
) -> Option<String> {
    if let Some(spec) = specs.iter().find(|spec| spec.relative_path.len() == depth) {
        return Some(render_amount_resolution_expression(value_expr, spec));
    }
    if !specs.iter().any(|spec| spec.relative_path.len() > depth) {
        return Some(value_expr.to_string());
    }

    match ty {
        IdlTypeSnapshot::Vec(vec_type) => {
            let element_expr = render_semantic_raw_expression(
                &vec_type.vec,
                "entry",
                defined_types,
                specs,
                depth,
            )?;
            return Some(format!(
                "await Promise.all({}.map(async (entry) => ({})))",
                value_expr, element_expr
            ));
        }
        IdlTypeSnapshot::Array(array_type) => {
            for part in &array_type.array {
                let element_expr = match part {
                    IdlArrayElementSnapshot::Type(element) => render_semantic_raw_expression(
                        element,
                        "entry",
                        defined_types,
                        specs,
                        depth,
                    )?,
                    IdlArrayElementSnapshot::TypeName(name) => render_semantic_raw_expression(
                        &IdlTypeSnapshot::Simple(name.clone()),
                        "entry",
                        defined_types,
                        specs,
                        depth,
                    )?,
                    IdlArrayElementSnapshot::Size(_) => continue,
                };
                return Some(format!(
                    "await Promise.all({}.map(async (entry) => ({})))",
                    value_expr, element_expr
                ));
            }
            return Some(value_expr.to_string());
        }
        _ => {}
    }

    let IdlTypeSnapshot::Defined(defined) = ty else {
        return Some(value_expr.to_string());
    };
    let type_name = match &defined.defined {
        IdlDefinedInnerSnapshot::Named { name } => name.as_str(),
        IdlDefinedInnerSnapshot::Simple(simple) => simple.as_str(),
    };
    let Some(def) = defined_types.find_definition(type_name) else {
        return Some(value_expr.to_string());
    };

    match &def.type_def {
        IdlTypeDefKindSnapshot::Struct { fields, .. } => {
            render_semantic_raw_struct_expression(fields, value_expr, defined_types, specs, depth)
        }
        IdlTypeDefKindSnapshot::Enum { variants, .. } => {
            render_semantic_raw_enum_expression(variants, value_expr, defined_types, specs, depth)
        }
        IdlTypeDefKindSnapshot::TupleStruct { .. } => Some(value_expr.to_string()),
    }
}

fn render_semantic_raw_struct_expression(
    fields: &[crate::ast::IdlFieldSnapshot],
    value_expr: &str,
    defined_types: &mut DefinedTypes<'_>,
    specs: &[&SemanticFieldSpec],
    depth: usize,
) -> Option<String> {
    let mut overrides: Vec<String> = Vec::new();
    for field in fields {
        let child_specs: Vec<&SemanticFieldSpec> = specs
            .iter()
            .copied()
            .filter(|spec| spec.relative_path.get(depth) == Some(&field.name))
            .collect();
        if child_specs.is_empty() {
            continue;
        }
        let child_expr = render_semantic_raw_expression(
            &field.type_,
            &render_ts_property_access(value_expr, &field.name),
            defined_types,
            &child_specs,
            depth + 1,
        )?;
        overrides.push(format!(
            "{}: {}",
            render_ts_property_name(&field.name),
            child_expr
        ));
    }
    if overrides.is_empty() {
        Some(value_expr.to_string())
    } else {
        Some(format!("{{ ...{}, {} }}", value_expr, overrides.join(", ")))
    }
}

fn render_semantic_raw_enum_expression(
    variants: &[crate::ast::IdlEnumVariantSnapshot],
    value_expr: &str,
    defined_types: &mut DefinedTypes<'_>,
    specs: &[&SemanticFieldSpec],
    depth: usize,
) -> Option<String> {
    use crate::ast::IdlEnumVariantFieldSnapshot;

    let mut branches: Vec<String> = Vec::new();
    for variant in variants {
        let variant_specs: Vec<&SemanticFieldSpec> = specs
            .iter()
            .copied()
            .filter(|spec| spec.relative_path.get(depth) == Some(&variant.name))
            .collect();
        if variant_specs.is_empty() {
            continue;
        }

        let named: Vec<_> = variant
            .fields
            .iter()
            .filter_map(|field| match field {
                IdlEnumVariantFieldSnapshot::Named(named) => Some(named),
                IdlEnumVariantFieldSnapshot::Tuple(_) => None,
            })
            .collect();
        if named.len() != variant.fields.len() {
            return Some(value_expr.to_string());
        }

        let variant_value_expr = render_ts_property_access(value_expr, &variant.name);
        let mut overrides: Vec<String> = Vec::new();
        for field in named {
            let child_specs: Vec<&SemanticFieldSpec> = variant_specs
                .iter()
                .copied()
                .filter(|spec| spec.relative_path.get(depth + 1) == Some(&field.name))
                .collect();
            if child_specs.is_empty() {
                continue;
            }
            let child_expr = render_semantic_raw_expression(
                &field.type_,
                &render_ts_property_access(&variant_value_expr, &field.name),
                defined_types,
                &child_specs,
                depth + 2,
            )?;
            overrides.push(format!(
                "{}: {}",
                render_ts_property_name(&field.name),
                child_expr
            ));
        }
        let payload_expr = if overrides.is_empty() {
            variant_value_expr.clone()
        } else {
            format!("{{ ...{}, {} }}", variant_value_expr, overrides.join(", "))
        };
        branches.push(format!(
            "('{}' in {value_expr} ? {{ {}: {} }} : __ELSE__)",
            escape_single_quotes(&variant.name),
            render_ts_property_name(&variant.name),
            payload_expr,
            value_expr = value_expr,
        ));
    }

    let mut rendered = value_expr.to_string();
    for branch in branches.into_iter().rev() {
        rendered = branch.replace("__ELSE__", &rendered);
    }
    Some(rendered)
}

fn render_semantic_params_interface(
    name: &str,
    parsed_args: &[(String, ParsedArgType)],
    instruction_snapshot: Option<&IdlInstructionSnapshot>,
    user_params: &[UserParam],
    resolve_params: &BTreeMap<String, String>,
    semantic_specs: &[SemanticFieldSpec],
    defined_types: &mut DefinedTypes<'_>,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    for (arg_name, parsed) in parsed_args {
        let arg_specs: Vec<&SemanticFieldSpec> = semantic_specs
            .iter()
            .filter(|spec| spec.root_arg_name == *arg_name)
            .collect();
        let ts_type = if arg_specs.is_empty() {
            parsed.ts_type.clone()
        } else if let Some(snapshot_arg) = instruction_snapshot
            .and_then(|snapshot| snapshot.args.iter().find(|arg| arg.name == *arg_name))
        {
            render_semantic_ts_type(&snapshot_arg.type_, defined_types, &arg_specs, 0)
                .unwrap_or_else(|| parsed.ts_type.clone())
        } else {
            "AmountInput".to_string()
        };
        lines.push(format!(
            "  {}: {};",
            render_ts_property_name(arg_name),
            ts_type
        ));
    }

    let mut extra_params: BTreeSet<String> = BTreeSet::new();
    for spec in semantic_specs {
        if let Some(extra) = &spec.decimals_override_name {
            extra_params.insert(extra.clone());
        }
    }
    for extra_param in extra_params {
        lines.push(format!(
            "  {}?: number;",
            render_ts_property_name(&extra_param)
        ));
    }

    for param in user_params {
        let optional = if param.optional { "?" } else { "" };
        lines.push(format!(
            "  {}{}: string;",
            render_ts_property_name(&param.name),
            optional
        ));
    }

    if !resolve_params.is_empty() {
        let resolve_lines: Vec<String> = resolve_params
            .iter()
            .map(|(name, ts_type)| format!("    {}?: {};", render_ts_property_name(name), ts_type))
            .collect();
        lines.push(format!(
            "  resolve?: {{\n{}\n  }};",
            resolve_lines.join("\n")
        ));
    }

    lines.push("  build?: BuildOptions;".to_string());

    let body = if lines.is_empty() {
        "  // This instruction takes no arguments or user-provided accounts.".to_string()
    } else {
        lines.join("\n")
    };

    format!("export interface {} {{\n{}\n}}", name, body)
}

/// Normalize a raw arg-type string (IDL or `pdas!` DSL spelling) to the
/// canonical seed type the runtime's `serializeSeedValue` understands.
/// Returns `None` for types that cannot be encoded as a seed.
pub(crate) fn normalize_seed_arg_type(raw: &str) -> Option<String> {
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

fn seed_arg_ts_type(raw: &str) -> Option<String> {
    let canonical = normalize_seed_arg_type(raw)?;
    let ts_type = match canonical.as_str() {
        "u8" | "u16" | "u32" | "i8" | "i16" | "i32" => "number",
        "u64" | "u128" | "i64" | "i128" => "bigint",
        "pubkey" => "string",
        "string" => "string",
        _ => return None,
    };
    Some(ts_type.to_string())
}

// ============================================================================
// Errors
// ============================================================================

/// Dedupe errors by code, preserving first-seen definitions, sorted ascending.
pub(crate) fn dedupe_errors_by_code(errors: &[IdlErrorSnapshot]) -> Vec<IdlErrorSnapshot> {
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    let mut by_code: BTreeMap<u32, IdlErrorSnapshot> = BTreeMap::new();
    for err in errors {
        if seen.insert(err.code) {
            by_code.insert(err.code, err.clone());
        }
    }
    by_code.into_values().collect()
}

fn render_program_errors(const_name: &str, type_name: &str, errors: &[IdlErrorSnapshot]) -> String {
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
    use crate::ast::{
        AmountDecimalsSource, InstructionAccountDef, InstructionAmountHint, InstructionArgDef,
    };

    fn arg(name: &str, ty: &str) -> InstructionArgDef {
        InstructionArgDef {
            name: name.to_string(),
            arg_type: ty.to_string(),
            docs: vec![],
            amount_hint: None,
        }
    }

    fn amount_arg(
        name: &str,
        ty: &str,
        decimals_source: AmountDecimalsSource,
    ) -> InstructionArgDef {
        InstructionArgDef {
            name: name.to_string(),
            arg_type: ty.to_string(),
            docs: vec![],
            amount_hint: Some(InstructionAmountHint { decimals_source }),
        }
    }

    fn user_account(name: &str) -> InstructionAccountDef {
        InstructionAccountDef {
            name: name.to_string(),
            is_signer: false,
            is_writable: false,
            resolution: AccountResolution::UserProvided,
            is_optional: false,
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

        let vec_u64 = parse_arg_type("VecU64Len<publicKey>");
        assert_eq!(vec_u64.schema, "{ vecU64Len: 'pubkey' }");
        assert_eq!(vec_u64.ts_type, "string[]");
        assert!(vec_u64.supported);

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
                        amount_hint: None,
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

    fn field(name: &str, type_: IdlTypeSnapshot) -> crate::ast::IdlFieldSnapshot {
        crate::ast::IdlFieldSnapshot {
            name: name.to_string(),
            type_,
            amount_hint: None,
        }
    }

    fn hinted_field(
        name: &str,
        type_: IdlTypeSnapshot,
        amount_hint: arete_idl::IdlAmountHint,
    ) -> crate::ast::IdlFieldSnapshot {
        crate::ast::IdlFieldSnapshot {
            name: name.to_string(),
            type_,
            amount_hint: Some(amount_hint),
        }
    }

    fn option(type_: IdlTypeSnapshot) -> IdlTypeSnapshot {
        IdlTypeSnapshot::Option(crate::ast::IdlOptionTypeSnapshot {
            option: Box::new(type_),
        })
    }

    fn vec_type(type_: IdlTypeSnapshot) -> IdlTypeSnapshot {
        IdlTypeSnapshot::Vec(crate::ast::IdlVecTypeSnapshot {
            vec: Box::new(type_),
            length_prefix: None,
        })
    }

    fn vec_u64_len_type(type_: IdlTypeSnapshot) -> IdlTypeSnapshot {
        IdlTypeSnapshot::Vec(crate::ast::IdlVecTypeSnapshot {
            vec: Box::new(type_),
            length_prefix: Some(IdlLengthPrefix::U64),
        })
    }

    fn array_type(type_: IdlTypeSnapshot, size: u32) -> IdlTypeSnapshot {
        IdlTypeSnapshot::Array(crate::ast::IdlArrayTypeSnapshot {
            array: vec![
                IdlArrayElementSnapshot::Type(type_),
                IdlArrayElementSnapshot::Size(size),
            ],
        })
    }

    fn hash_map(key: IdlTypeSnapshot, value: IdlTypeSnapshot) -> IdlTypeSnapshot {
        IdlTypeSnapshot::HashMap(crate::ast::IdlHashMapTypeSnapshot {
            hash_map: (Box::new(key), Box::new(value)),
        })
    }

    fn instruction_snapshot(
        name: &str,
        discriminator: Vec<u8>,
        args: Vec<crate::ast::IdlFieldSnapshot>,
    ) -> IdlInstructionSnapshot {
        IdlInstructionSnapshot {
            name: name.to_string(),
            discriminator,
            discriminant: None,
            docs: vec![],
            accounts: vec![],
            args,
        }
    }

    #[test]
    fn parses_top_level_args_from_idl_snapshots_before_lossy_strings() {
        let mut idl = idl("demo", "Prog111", vec![]);
        idl.instructions = vec![instruction_snapshot(
            "deposit",
            vec![9],
            vec![field("amount", simple("u64"))],
        )];
        let idls = vec![idl];

        let instr = InstructionDef {
            name: "deposit".to_string(),
            discriminator: vec![9],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![arg("amount", "DefinitelyUnsupported")],
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
        assert!(out.code.contains("amount: bigint;"));
        assert!(out.code.contains("{ name: 'amount', type: 'u64' }"));
    }

    #[test]
    fn emits_u64_length_prefixed_vec_schema() {
        let mut idl = idl("demo", "Prog111", vec![]);
        idl.instructions = vec![instruction_snapshot(
            "extendLookupTable",
            vec![2],
            vec![
                field("newAddresses", vec_u64_len_type(simple("publicKey"))),
                field("borshAddresses", vec_type(simple("publicKey"))),
            ],
        )];
        let idls = vec![idl];

        let instr = InstructionDef {
            name: "extendLookupTable".to_string(),
            discriminator: vec![2],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![
                arg("newAddresses", "Vec<publicKey>"),
                arg("borshAddresses", "Vec<publicKey>"),
            ],
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
        assert!(
            out.code
                .contains("{ name: 'newAddresses', type: { vecU64Len: 'pubkey' } }"),
            "{}",
            out.code
        );
        // An absent length prefix must keep emitting today's Borsh u32 schema.
        assert!(
            out.code
                .contains("{ name: 'borshAddresses', type: { vec: 'pubkey' } }"),
            "{}",
            out.code
        );
        assert!(out.code.contains("newAddresses: string[];"));
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
                                    amount_hint: None,
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
    fn path_qualified_idl_type_names_emit_valid_typescript_identifiers() {
        let qualified_name =
            "sb_on_demand::actions::pull_feed::pull_feed_submit_response_action::Submission";
        let mut idl = idl("switchboard", "Prog111", vec![]);
        idl.types = vec![struct_def(qualified_name, vec![("value", simple("u64"))])];
        idl.instructions = vec![instruction_snapshot(
            "submit",
            vec![7],
            vec![field("submission", defined(qualified_name))],
        )];
        let idls = vec![idl];

        let instr = InstructionDef {
            name: "submit".to_string(),
            discriminator: vec![7],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![arg("submission", qualified_name)],
            errors: vec![],
            program_id: Some("Prog111".to_string()),
            docs: vec![],
        };

        let out = generate_instructions_code(
            "Switchboard",
            std::slice::from_ref(&instr),
            &idls,
            &BTreeMap::new(),
            &["Prog111".to_string()],
            &HashSet::new(),
        );

        assert_eq!(out.stack_entries.len(), 1, "warnings: {:?}", out.warnings);
        assert!(out.code.contains(
            "export interface SbOnDemandActionsPullFeedPullFeedSubmitResponseActionSubmission"
        ));
        assert!(out.code.contains(
            "submission: SbOnDemandActionsPullFeedPullFeedSubmitResponseActionSubmission;"
        ));
        assert!(!out.code.contains("::"), "invalid identifier: {}", out.code);
    }

    #[test]
    fn resolves_string_key_maps_inside_instruction_arg_types() {
        let mut idl = idl("demo", "Prog111", vec![]);
        idl.types = vec![
            struct_def("authorizationData", vec![("payload", defined("payload"))]),
            struct_def(
                "payload",
                vec![("map", hash_map(simple("string"), defined("payloadType")))],
            ),
            IdlTypeDefSnapshot {
                name: "payloadType".to_string(),
                docs: vec![],
                serialization: None,
                type_def: IdlTypeDefKindSnapshot::Enum {
                    kind: "enum".to_string(),
                    variants: vec![
                        crate::ast::IdlEnumVariantSnapshot {
                            name: "Pubkey".to_string(),
                            fields: vec![crate::ast::IdlEnumVariantFieldSnapshot::Tuple(simple(
                                "publicKey",
                            ))],
                        },
                        crate::ast::IdlEnumVariantSnapshot {
                            name: "Number".to_string(),
                            fields: vec![crate::ast::IdlEnumVariantFieldSnapshot::Tuple(simple(
                                "u64",
                            ))],
                        },
                    ],
                },
            },
            IdlTypeDefSnapshot {
                name: "mintArgs".to_string(),
                docs: vec![],
                serialization: None,
                type_def: IdlTypeDefKindSnapshot::Enum {
                    kind: "enum".to_string(),
                    variants: vec![crate::ast::IdlEnumVariantSnapshot {
                        name: "V1".to_string(),
                        fields: vec![
                            crate::ast::IdlEnumVariantFieldSnapshot::Named(field(
                                "amount",
                                simple("u64"),
                            )),
                            crate::ast::IdlEnumVariantFieldSnapshot::Named(field(
                                "authorization_data",
                                option(defined("authorizationData")),
                            )),
                        ],
                    }],
                },
            },
        ];
        idl.instructions = vec![instruction_snapshot(
            "Mint",
            vec![4],
            vec![field("mintArgs", defined("mintArgs"))],
        )];
        let idls = vec![idl];

        let instr = InstructionDef {
            name: "Mint".to_string(),
            discriminator: vec![4],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![arg("mintArgs", "mintArgs")],
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
        assert!(
            out.warnings.is_empty(),
            "map-backed args should emit cleanly: {:?}",
            out.warnings
        );
        let code = &out.code;
        assert!(code.contains("export interface AuthorizationData"));
        assert!(code.contains("export interface Payload"));
        assert!(code.contains("export type PayloadType"));
        assert!(code.contains("map: Record<string, PayloadType>;"));
        assert!(code.contains(
            "{ name: 'map', type: { hashMap: ['string', { enum: [{ name: 'Pubkey', tuple: ['pubkey'] }, { name: 'Number', tuple: ['u64'] }] }] } }"
        ));
        assert!(code.contains("mintArgs: MintArgs;"));
    }

    #[test]
    fn resolves_string_to_string_maps() {
        let mut idl = idl("demo", "Prog111", vec![]);
        idl.types = vec![struct_def(
            "tokenMetadata",
            vec![(
                "additionalMetadata",
                hash_map(simple("string"), simple("string")),
            )],
        )];
        idl.instructions = vec![instruction_snapshot(
            "updateTokenMetadata",
            vec![5],
            vec![field("metadata", defined("tokenMetadata"))],
        )];
        let idls = vec![idl];

        let instr = InstructionDef {
            name: "updateTokenMetadata".to_string(),
            discriminator: vec![5],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![arg("metadata", "tokenMetadata")],
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
        assert!(out
            .code
            .contains("additionalMetadata: Record<string, string>;"));
        assert!(out.code.contains("{ hashMap: ['string', 'string'] }"));
    }

    #[test]
    fn non_string_key_maps_remain_unsupported() {
        let mut idl = idl("demo", "Prog111", vec![]);
        idl.types = vec![struct_def(
            "tokenMetadata",
            vec![(
                "additionalMetadata",
                hash_map(simple("u64"), simple("string")),
            )],
        )];
        idl.instructions = vec![instruction_snapshot(
            "updateTokenMetadata",
            vec![6],
            vec![field("metadata", defined("tokenMetadata"))],
        )];
        let idls = vec![idl];

        let instr = InstructionDef {
            name: "updateTokenMetadata".to_string(),
            discriminator: vec![6],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![arg("metadata", "tokenMetadata")],
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
        assert!(out
            .warnings
            .iter()
            .any(|warning| warning.contains("unsupported type")));
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
                program_key: Some("subscriptions".to_string()),
                instruction_name: "closeSubscriptionAuthority".to_string(),
                runtime_program_key: Some("subscriptions".to_string()),
                handler_const: "closeSubscriptionAuthorityInstruction".to_string(),
                params_type: "CloseSubscriptionAuthorityParams".to_string(),
                semantic_params_type: Some("CloseSubscriptionAuthorityParams".to_string()),
                semantic_extra_params: vec![],
                semantic_amount_args: vec![],
                uses_operation_context: false,
            }]
        );
        assert!(out.needs_runtime_import);
        let code = &out.code;
        assert!(code.contains("export interface CloseSubscriptionAuthorityParams"));
        assert!(code.contains("subscriptionAuthority: string;"));
        assert!(code.contains("category: 'signer'"));
        assert!(code.contains("signerKind: 'provided'"));
        assert!(code.contains("category: 'userProvided'"));
        assert!(code.contains(
            "export const closeSubscriptionAuthorityInstruction = createInstructionHandler<CloseSubscriptionAuthorityParams, CloseSubscriptionAuthorityError>"
        ));
        assert!(code.contains("SUBSCRIPTIONS_PROGRAM_ERRORS: ErrorMetadata[]"));
        assert!(code.contains("code: 130, name: 'unauthorized'"));
    }

    #[test]
    fn arg_account_name_collisions_suffix_the_account_and_update_pda_refs() {
        let instr = InstructionDef {
            name: "decompressV1".to_string(),
            discriminator: vec![54],
            discriminator_size: 1,
            accounts: vec![
                user_account("metadata"),
                InstructionAccountDef {
                    name: "receipt".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::PdaInline {
                        seeds: vec![PdaSeedDef::AccountRef {
                            account_name: "metadata".to_string(),
                        }],
                        program_id: None,
                        program: None,
                    },
                    is_optional: false,
                    docs: vec![],
                },
            ],
            args: vec![arg("metadata", "string")],
            errors: vec![],
            program_id: Some("Prog111".to_string()),
            docs: vec![],
        };

        let out = generate_instructions_code(
            "Bubblegum",
            std::slice::from_ref(&instr),
            &[],
            &BTreeMap::new(),
            &["Prog111".to_string()],
            &HashSet::new(),
        );

        assert!(out.code.contains("export interface DecompressV1Params {"));
        assert_eq!(out.code.matches("  metadata: string;").count(), 1);
        assert!(out.code.contains("  metadataAccount: string;"));
        assert!(out.code.contains("name: 'metadataAccount'"));
        assert!(out
            .code
            .contains("{ type: 'accountRef', accountName: 'metadataAccount' }"));
        assert!(out.warnings.iter().any(|warning| {
            warning.contains("account 'metadata' collides with an argument")
                && warning.contains("'metadataAccount'")
        }));
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
                program: None,
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
        // PDA accounts remain derivable, but callers can still pass explicit overrides.
        assert!(code.contains("tokenMint: string;"));
        assert!(code.contains("subscriptionAuthority?: string;"));
        assert!(
            out.warnings.is_empty(),
            "no degradation expected: {:?}",
            out.warnings
        );
    }

    #[test]
    fn emits_dynamic_pda_program_account_selector() {
        let mut program_pdas: BTreeMap<String, PdaDefinition> = BTreeMap::new();
        program_pdas.insert(
            "metadata".to_string(),
            PdaDefinition {
                name: "metadata".to_string(),
                seeds: vec![PdaSeedDef::Literal {
                    value: "metadata".to_string(),
                }],
                program_id: None,
                program: Some(PdaProgramDef::AccountRef {
                    account_name: "metadataProgram".to_string(),
                }),
            },
        );
        let pdas = BTreeMap::from([("demo".to_string(), program_pdas)]);
        let instr = InstructionDef {
            name: "createMetadata".to_string(),
            discriminator: vec![7],
            discriminator_size: 1,
            accounts: vec![
                InstructionAccountDef {
                    name: "metadataProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                    docs: vec![],
                },
                InstructionAccountDef {
                    name: "metadata".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::PdaRef {
                        pda_name: "metadata".to_string(),
                    },
                    is_optional: false,
                    docs: vec![],
                },
            ],
            args: vec![],
            errors: vec![],
            program_id: Some("Prog111".to_string()),
            docs: vec![],
        };

        let out = generate_instructions_code(
            "Demo",
            &[instr],
            &[],
            &pdas,
            &["Prog111".to_string()],
            &HashSet::new(),
        );

        assert!(out
            .code
            .contains("program: { type: 'accountRef', accountName: 'metadataProgram' }"));
        assert!(out.code.contains("metadata?: string;"));
        assert!(
            out.warnings.is_empty(),
            "unexpected warnings: {:?}",
            out.warnings
        );
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
                program: None,
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
            args: vec![arg("roundId", "u32"), arg("owner", "solana_pubkey::Pubkey")],
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
                program: None,
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

    #[test]
    fn nested_arg_seed_uses_struct_root_without_degrading() {
        let mut program_pdas: BTreeMap<String, PdaDefinition> = BTreeMap::new();
        program_pdas.insert(
            "proposal".to_string(),
            PdaDefinition {
                name: "proposal".to_string(),
                seeds: vec![PdaSeedDef::ArgRef {
                    arg_name: "args.transactionIndex".to_string(),
                    arg_type: Some("u64".to_string()),
                }],
                program_id: None,
                program: None,
            },
        );
        let mut pdas = BTreeMap::new();
        pdas.insert("demo".to_string(), program_pdas);

        let instr = InstructionDef {
            name: "proposalCreate".to_string(),
            discriminator: vec![3],
            discriminator_size: 1,
            accounts: vec![InstructionAccountDef {
                name: "proposal".to_string(),
                is_signer: false,
                is_writable: true,
                resolution: AccountResolution::PdaRef {
                    pda_name: "proposal".to_string(),
                },
                is_optional: false,
                docs: vec![],
            }],
            args: vec![arg("args", "u64")],
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

        assert!(out
            .code
            .contains("{ type: 'argRef', argName: 'args.transactionIndex', argType: 'u64' }"));
        assert!(out.code.contains("proposal?: string;"));
        assert!(
            out.warnings.is_empty(),
            "unexpected warnings: {:?}",
            out.warnings
        );
    }

    #[test]
    fn helper_only_arg_seed_emits_resolve_namespace() {
        let mut program_pdas: BTreeMap<String, PdaDefinition> = BTreeMap::new();
        program_pdas.insert(
            "proposal".to_string(),
            PdaDefinition {
                name: "proposal".to_string(),
                seeds: vec![PdaSeedDef::ArgRef {
                    arg_name: "transactionIndex".to_string(),
                    arg_type: Some("u64".to_string()),
                }],
                program_id: None,
                program: None,
            },
        );
        let mut pdas = BTreeMap::new();
        pdas.insert("demo".to_string(), program_pdas);

        let instr = InstructionDef {
            name: "proposalActivate".to_string(),
            discriminator: vec![4],
            discriminator_size: 1,
            accounts: vec![InstructionAccountDef {
                name: "proposal".to_string(),
                is_signer: false,
                is_writable: true,
                resolution: AccountResolution::PdaRef {
                    pda_name: "proposal".to_string(),
                },
                is_optional: false,
                docs: vec![],
            }],
            args: vec![],
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

        assert!(
            out.code.contains("resolve?: {"),
            "code missing resolve namespace: {}",
            out.code
        );
        assert!(
            out.code.contains("transactionIndex?: bigint;"),
            "code missing helper input: {}",
            out.code
        );
        assert!(out.code.contains("proposal?: string;"));
        assert!(
            out.warnings.is_empty(),
            "unexpected warnings: {:?}",
            out.warnings
        );
    }

    #[test]
    fn account_field_seed_still_degrades_with_actionable_warning() {
        let mut program_pdas: BTreeMap<String, PdaDefinition> = BTreeMap::new();
        program_pdas.insert(
            "proposal".to_string(),
            PdaDefinition {
                name: "proposal".to_string(),
                seeds: vec![PdaSeedDef::AccountRef {
                    account_name: "transaction.index".to_string(),
                }],
                program_id: None,
                program: None,
            },
        );
        let mut pdas = BTreeMap::new();
        pdas.insert("demo".to_string(), program_pdas);

        let instr = InstructionDef {
            name: "configTransactionExecute".to_string(),
            discriminator: vec![5],
            discriminator_size: 1,
            accounts: vec![
                InstructionAccountDef {
                    name: "transaction".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                    docs: vec![],
                },
                InstructionAccountDef {
                    name: "proposal".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::PdaRef {
                        pda_name: "proposal".to_string(),
                    },
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
            "Demo",
            std::slice::from_ref(&instr),
            &[],
            &pdas,
            &["De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44".to_string()],
            &HashSet::new(),
        );

        assert!(out.code.contains("proposal: string;"));
        assert!(out.warnings.iter().any(|w| w.contains("typed helper arg")));
        assert_eq!(out.pda_degradations.len(), 1);
        assert_eq!(
            out.pda_degradations[0],
            PdaDegradation {
                instruction_name: "configTransactionExecute".to_string(),
                account_name: "proposal".to_string(),
                pda_name: Some("proposal".to_string()),
                source: PdaDegradationSource::Registry,
                reason: "seed references account field 'transaction.index' which is not supported for low-level auto-resolution; encode it as a typed helper arg instead".to_string(),
            }
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
            .find(&format!("export const oreCloseInstruction"))
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
    fn emits_amount_aware_semantic_params_without_changing_raw_params() {
        let out = generate_instructions_code(
            "Demo",
            &[InstructionDef {
                name: "deposit".to_string(),
                discriminator: vec![9],
                discriminator_size: 1,
                accounts: vec![],
                args: vec![
                    amount_arg(
                        "amount",
                        "u64",
                        AmountDecimalsSource::ArgMint {
                            arg_name: "mint".to_string(),
                        },
                    ),
                    arg("mint", "solana_pubkey::Pubkey"),
                ],
                errors: vec![],
                program_id: Some("Prog111".to_string()),
                docs: vec![],
            }],
            &[idl("demo", "Prog111", vec![])],
            &BTreeMap::new(),
            &["Prog111".to_string()],
            &HashSet::new(),
        );

        assert!(out.code.contains("export interface DepositParams"));
        assert!(out.code.contains("amount: bigint;"));
        assert!(out.code.contains("export interface DepositSemanticParams"));
        assert!(out.code.contains("amount: AmountInput;"));
        assert!(out.code.contains("amountDecimals?: number;"));
        assert!(out.code.contains("build?: BuildOptions;"));
        assert!(out.needs_amount_input);
        assert!(out.needs_program_runtime_extensions);
        assert!(out.needs_resolve_amount_to_raw);
        assert!(out.needs_operation_context);
        assert!(out.stack_entries[0].uses_operation_context);
        assert!(out.stack_entries[0].semantic_amount_args[0].uses_operation_context);
        assert_eq!(
            out.stack_entries[0].semantic_params_type.as_deref(),
            Some("DepositSemanticParams")
        );
        assert_eq!(
            out.stack_entries[0].runtime_program_key.as_deref(),
            Some("demo")
        );
    }

    #[test]
    fn emits_nested_amount_aware_semantic_params_and_root_conversions() {
        let mut idl = idl("demo", "Prog111", vec![]);
        idl.types = vec![IdlTypeDefSnapshot {
            name: "depositParams".to_string(),
            docs: vec![],
            serialization: None,
            type_def: IdlTypeDefKindSnapshot::Struct {
                kind: "struct".to_string(),
                fields: vec![
                    hinted_field(
                        "maxAmount",
                        simple("u64"),
                        arete_idl::IdlAmountHint {
                            decimals_source: arete_idl::IdlAmountDecimalsSource::ArgMint {
                                arg_name: "params.quoteMint".to_string(),
                            },
                        },
                    ),
                    field("quoteMint", simple("publicKey")),
                    field("memo", simple("string")),
                ],
            },
        }];
        idl.instructions = vec![instruction_snapshot(
            "deposit",
            vec![9],
            vec![field("params", defined("depositParams"))],
        )];

        let out = generate_instructions_code(
            "Demo",
            &[InstructionDef {
                name: "deposit".to_string(),
                discriminator: vec![9],
                discriminator_size: 1,
                accounts: vec![],
                args: vec![arg("params", "depositParams")],
                errors: vec![],
                program_id: Some("Prog111".to_string()),
                docs: vec![],
            }],
            &[idl],
            &BTreeMap::new(),
            &["Prog111".to_string()],
            &HashSet::new(),
        );

        assert!(out.code.contains("params: DepositParams;"));
        assert!(out.code.contains("export interface DepositSemanticParams"));
        assert!(out
            .code
            .contains("params: { maxAmount: AmountInput; quoteMint: string; memo: string; };"));
        assert!(out.code.contains("paramsMaxAmountDecimals?: number;"));
        assert!(out.code.contains("build?: BuildOptions;"));
        assert_eq!(
            out.stack_entries[0].semantic_params_type.as_deref(),
            Some("DepositSemanticParams")
        );
        assert_eq!(
            out.stack_entries[0].semantic_extra_params,
            vec!["paramsMaxAmountDecimals".to_string()]
        );
        assert_eq!(out.stack_entries[0].semantic_amount_args.len(), 1);
        assert_eq!(
            out.stack_entries[0].semantic_amount_args[0].arg_name,
            "params"
        );
        assert!(out.stack_entries[0].semantic_amount_args[0]
            .raw_expression
            .contains("params.params.maxAmount"));
        assert!(out.stack_entries[0].semantic_amount_args[0]
            .raw_expression
            .contains("params.params.quoteMint"));
    }

    #[test]
    fn renames_generated_params_when_an_idl_type_owns_the_preferred_name() {
        let mut snapshot = idl("demo", "Prog111", vec![]);
        snapshot.types = vec![IdlTypeDefSnapshot {
            name: "initializeLaunchParams".to_string(),
            docs: vec![],
            serialization: None,
            type_def: IdlTypeDefKindSnapshot::Struct {
                kind: "struct".to_string(),
                fields: vec![field("launchId", simple("u64"))],
            },
        }];
        snapshot.instructions = vec![instruction_snapshot(
            "initializeLaunch",
            vec![8],
            vec![field("params", defined("initializeLaunchParams"))],
        )];

        let out = generate_instructions_code(
            "Demo",
            &[InstructionDef {
                name: "initializeLaunch".to_string(),
                discriminator: vec![8],
                discriminator_size: 1,
                accounts: vec![],
                args: vec![arg("params", "initializeLaunchParams")],
                errors: vec![],
                program_id: Some("Prog111".to_string()),
                docs: vec![],
            }],
            &[snapshot],
            &BTreeMap::new(),
            &["Prog111".to_string()],
            &HashSet::new(),
        );

        assert!(out.code.contains("export interface InitializeLaunchParams"));
        assert!(out
            .code
            .contains("export interface InitializeLaunchInstructionParams"));
        assert!(out.code.contains("params: InitializeLaunchParams;"));
        assert_eq!(
            out.stack_entries[0].params_type,
            "InitializeLaunchInstructionParams"
        );
    }

    #[test]
    fn emits_amount_aware_semantic_params_inside_vectors_and_arrays() {
        for (raw_type, snapshot_type) in [
            ("Vec<registry>", vec_type(defined("registry"))),
            ("[registry; 2]", array_type(defined("registry"), 2)),
        ] {
            let mut idl = idl("demo", "Prog111", vec![]);
            idl.types = vec![IdlTypeDefSnapshot {
                name: "registry".to_string(),
                docs: vec![],
                serialization: None,
                type_def: IdlTypeDefKindSnapshot::Struct {
                    kind: "struct".to_string(),
                    fields: vec![
                        hinted_field(
                            "buyerCap",
                            simple("u64"),
                            arete_idl::IdlAmountHint {
                                decimals_source: arete_idl::IdlAmountDecimalsSource::ArgMint {
                                    arg_name: "registries[].quoteMint".to_string(),
                                },
                            },
                        ),
                        field("quoteMint", simple("publicKey")),
                        hinted_field(
                            "supply",
                            simple("u64"),
                            arete_idl::IdlAmountHint {
                                decimals_source: arete_idl::IdlAmountDecimalsSource::ArgMint {
                                    arg_name: "baseMint".to_string(),
                                },
                            },
                        ),
                        field("memo", simple("string")),
                    ],
                },
            }];
            idl.instructions = vec![instruction_snapshot(
                "initialize",
                vec![9],
                vec![
                    field("registries", snapshot_type),
                    field("quoteMint", simple("publicKey")),
                    field("baseMint", simple("publicKey")),
                ],
            )];

            let out = generate_instructions_code(
                "Demo",
                &[InstructionDef {
                    name: "initialize".to_string(),
                    discriminator: vec![9],
                    discriminator_size: 1,
                    accounts: vec![],
                    args: vec![
                        arg("registries", raw_type),
                        arg("quoteMint", "publicKey"),
                        arg("baseMint", "publicKey"),
                    ],
                    errors: vec![],
                    program_id: Some("Prog111".to_string()),
                    docs: vec![],
                }],
                &[idl],
                &BTreeMap::new(),
                &["Prog111".to_string()],
                &HashSet::new(),
            );

            assert!(out.code.contains(
                "registries: { buyerCap: AmountInput; quoteMint: string; supply: AmountInput; memo: string; }[];"
            ));
            let raw_expression = &out.stack_entries[0].semantic_amount_args[0].raw_expression;
            assert!(raw_expression
                .contains("await Promise.all(params.registries.map(async (entry) => ("));
            assert!(
                raw_expression.contains("mint: entry.quoteMint"),
                "unexpected raw expression: {raw_expression}"
            );
            assert!(raw_expression.contains("mint: params.baseMint"));
            assert!(raw_expression.contains("...entry"));
            assert!(raw_expression.contains("memo") == false);
        }
    }

    #[test]
    fn emits_known_account_semantic_params_for_user_provided_accounts() {
        let out = generate_instructions_code(
            "Demo",
            &[InstructionDef {
                name: "mintTo".to_string(),
                discriminator: vec![7],
                discriminator_size: 1,
                accounts: vec![user_account("mint"), user_account("destination")],
                args: vec![amount_arg(
                    "amount",
                    "u64",
                    AmountDecimalsSource::KnownAccount {
                        account_name: "mint".to_string(),
                    },
                )],
                errors: vec![],
                program_id: Some("Prog111".to_string()),
                docs: vec![],
            }],
            &[idl("demo", "Prog111", vec![])],
            &BTreeMap::new(),
            &["Prog111".to_string()],
            &HashSet::new(),
        );

        assert!(out.code.contains("export interface MintToSemanticParams"));
        assert!(out.code.contains("amount: AmountInput;"));
        assert!(out.code.contains("amountDecimals?: number;"));
        assert!(out.code.contains("build?: BuildOptions;"));
        assert_eq!(out.stack_entries[0].semantic_amount_args.len(), 1);
        assert!(out.stack_entries[0].semantic_amount_args[0]
            .raw_expression
            .contains("mint: params.mint"));
        assert!(out.stack_entries[0].semantic_amount_args[0]
            .raw_expression
            .contains("amount: params.amount"));
    }

    #[test]
    fn emits_nested_known_account_semantic_params_for_user_provided_accounts() {
        let mut idl = idl("demo", "Prog111", vec![]);
        idl.types = vec![IdlTypeDefSnapshot {
            name: "depositParams".to_string(),
            docs: vec![],
            serialization: None,
            type_def: IdlTypeDefKindSnapshot::Struct {
                kind: "struct".to_string(),
                fields: vec![
                    hinted_field(
                        "maxAmount",
                        simple("u64"),
                        arete_idl::IdlAmountHint {
                            decimals_source: arete_idl::IdlAmountDecimalsSource::KnownAccount {
                                account_name: "quoteMint".to_string(),
                            },
                        },
                    ),
                    field("memo", simple("string")),
                ],
            },
        }];
        idl.instructions = vec![instruction_snapshot(
            "deposit",
            vec![9],
            vec![field("params", defined("depositParams"))],
        )];

        let out = generate_instructions_code(
            "Demo",
            &[InstructionDef {
                name: "deposit".to_string(),
                discriminator: vec![9],
                discriminator_size: 1,
                accounts: vec![user_account("quoteMint")],
                args: vec![arg("params", "depositParams")],
                errors: vec![],
                program_id: Some("Prog111".to_string()),
                docs: vec![],
            }],
            &[idl],
            &BTreeMap::new(),
            &["Prog111".to_string()],
            &HashSet::new(),
        );

        assert!(out.code.contains("export interface DepositSemanticParams"));
        assert!(out
            .code
            .contains("params: { maxAmount: AmountInput; memo: string; };"));
        assert!(out.code.contains("paramsMaxAmountDecimals?: number;"));
        assert!(out.code.contains("build?: BuildOptions;"));
        assert_eq!(out.stack_entries[0].semantic_amount_args.len(), 1);
        assert!(out.stack_entries[0].semantic_amount_args[0]
            .raw_expression
            .contains("params.params.maxAmount"));
        assert!(out.stack_entries[0].semantic_amount_args[0]
            .raw_expression
            .contains("mint: params.quoteMint"));
    }

    #[test]
    fn multi_program_unmatched_instruction_falls_back_with_warning() {
        let idls = vec![
            idl("ore", "Prog111111111111111111111111111111111111111", vec![]),
            idl(
                "entropy",
                "Prog222222222222222222222222222222222222222",
                vec![],
            ),
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
