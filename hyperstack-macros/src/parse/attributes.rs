//! Attribute parsing for hyperstack macros.
//!
//! This module parses macro attributes like #[map], #[event], #[snapshot], etc.

#![allow(dead_code)]

use proc_macro2::Span;
use std::collections::HashMap;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Attribute, Path, Token};

use crate::ast::{ConditionExpr, FieldPath, ResolverCondition, ResolverType};
use crate::diagnostic::{invalid_choice_message, ErrorCollector};
use crate::parse::conditions as condition_parser;

#[derive(Debug, Clone)]
pub struct RegisterFromSpec {
    pub instruction_path: Path,
    pub pda_field: FieldSpec,
    pub primary_key_field: FieldSpec,
}

#[derive(Debug, Clone)]
pub struct MapAttribute {
    pub attr_span: Span,
    pub source_type_span: Span,
    pub source_field_span: Span,
    // Set when #[event(...)] handlers are normalized into MapAttribute values.
    pub is_event_source: bool,
    pub is_account_source: bool,
    pub source_type_path: Path,
    pub source_field_name: String,
    pub target_field_name: String,
    pub is_primary_key: bool,
    pub is_lookup_index: bool,
    pub register_from: Vec<RegisterFromSpec>,
    pub temporal_field: Option<String>,
    pub strategy: String,
    pub join_on: Option<FieldSpec>,
    pub transform: Option<String>,
    /// Resolver transform: a parameterized transform like `ui_amount(ore_metadata.decimals)`
    /// that expands into a hidden raw field + synthesized computed field.
    pub resolver_transform: Option<ResolverTransformSpec>,
    pub is_instruction: bool,
    pub is_whole_source: bool,
    pub lookup_by: Option<FieldSpec>,
    pub condition: Option<ConditionExpr>,
    pub when: Option<Path>,
    pub stop: Option<Path>,
    pub stop_lookup_by: Option<FieldSpec>,
    pub emit: bool,
}

/// A parameterized resolver transform like `ui_amount(ore_metadata.decimals)`.
/// The method name is resolved via `resolver_for_method()` to a resolver name.
/// The args are token streams that will be parsed as computed expressions.
#[derive(Debug, Clone)]
pub struct ResolverTransformSpec {
    pub method: String,
    pub args: proc_macro2::TokenStream,
}

#[derive(Debug, Clone)]
pub struct EventAttribute {
    pub attr_span: Span,
    pub instruction_span: Option<Span>,
    // New type-safe fields
    pub from_instruction: Option<Path>, // Explicit source via `from = ...`
    pub inferred_instruction: Option<Path>, // Inferred from field type
    pub capture_fields: Vec<FieldSpec>, // Field identifiers with location info
    pub field_transforms: HashMap<String, syn::Ident>, // Identifier-based transforms

    // Backward compatibility - old string-based syntax
    pub instruction: String, // Legacy: "program::Instruction" format
    pub capture_fields_legacy: Vec<String>, // Legacy: string-based fields
    pub field_transforms_legacy: HashMap<String, String>, // Legacy: string-based transforms

    // Common fields
    pub strategy: String,
    pub target_field_name: String,
    pub join_on: Option<FieldSpec>,
    pub lookup_by: Option<FieldSpec>,
}

#[derive(Debug, Clone)]
pub struct CaptureAttribute {
    pub attr_span: Span,
    // Type-safe fields for account capture
    pub from_account: Option<Path>, // Explicit source via `from = ...`
    pub inferred_account: Option<Path>, // Inferred from field type

    // Single field extraction from account (e.g., field = token_mint_0)
    pub field: Option<syn::Ident>,

    // Field transformations
    pub field_transforms: HashMap<String, syn::Ident>, // Map field name to transformation

    // Common fields
    pub strategy: String, // Only SetOnce or LastWrite allowed
    pub target_field_name: String,
    pub join_on: Option<FieldSpec>,
    pub lookup_by: Option<FieldSpec>,
    pub when: Option<Path>,
}

#[derive(Debug, Clone)]
pub struct ValidatedFieldPath {
    pub raw: String,
    pub parsed: FieldPath,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ValidatedResolverCondition {
    pub expression: String,
    pub parsed: ResolverCondition,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FieldSpec {
    pub ident: syn::Ident,
    pub explicit_location: Option<FieldLocation>, // For accounts::mint syntax
}

#[derive(Debug, Clone)]
pub enum FieldLocation {
    InstructionArg,
    Account,
}

fn validate_strategy<T: quote::ToTokens>(
    attr_name: &str,
    strategy: String,
    tokens: &T,
    allowed: &[&str],
) -> syn::Result<String> {
    if allowed.contains(&strategy.as_str()) {
        Ok(strategy)
    } else {
        Err(syn::Error::new_spanned(
            tokens,
            invalid_choice_message("strategy", &strategy, attr_name, allowed),
        ))
    }
}

fn parse_condition_literal(literal: &syn::LitStr) -> syn::Result<ConditionExpr> {
    let expression = literal.value();
    let parsed = condition_parser::parse_condition_expression_strict(&expression)
        .map_err(|error| syn::Error::new_spanned(literal, error))?;

    Ok(ConditionExpr {
        expression,
        parsed: Some(parsed),
    })
}

fn parse_resolver_condition_literal(
    literal: &syn::LitStr,
) -> syn::Result<ValidatedResolverCondition> {
    let expression = literal.value();
    let parsed = condition_parser::parse_resolver_condition_expression(&expression)
        .map_err(|error| syn::Error::new_spanned(literal, error))?;

    Ok(ValidatedResolverCondition {
        expression,
        parsed,
        span: literal.span(),
    })
}

fn field_path_to_string(path: &FieldPath) -> String {
    path.segments.join(".")
}

fn parse_validated_field_path(input: ParseStream) -> syn::Result<ValidatedFieldPath> {
    let mut segments = Vec::new();
    let first: syn::Ident = input.parse()?;
    let mut last_span = first.span();
    segments.push(first.to_string());

    while input.peek(Token![.]) {
        input.parse::<Token![.]>()?;
        let next: syn::Ident = input.parse()?;
        last_span = next.span();
        segments.push(next.to_string());
    }

    let span = first.span().join(last_span).unwrap_or(first.span());

    let refs: Vec<&str> = segments.iter().map(String::as_str).collect();
    let parsed = FieldPath::new(&refs);
    Ok(ValidatedFieldPath {
        raw: field_path_to_string(&parsed),
        parsed,
        span,
    })
}

impl MapAttribute {
    pub fn source_type_string(&self) -> String {
        self.source_type_path
            .segments
            .iter()
            .map(|seg| seg.ident.to_string())
            .collect::<Vec<_>>()
            .join("::")
    }
}

fn classify_source_type_path(path: &Path) -> (bool, bool) {
    let has_instructions_segment = path
        .segments
        .iter()
        .any(|segment| segment.ident == "instructions");
    let has_accounts_segment = path
        .segments
        .iter()
        .any(|segment| segment.ident == "accounts");

    (has_instructions_segment, has_accounts_segment)
}

struct MapAttributeArgs {
    source_paths: Vec<Path>,
    is_primary_key: bool,
    is_lookup_index: bool,
    register_from: Vec<RegisterFromSpec>,
    temporal_field: Option<String>,
    strategy: Option<String>,
    rename: Option<String>,
    join_on: Option<FieldSpec>,
    transform: Option<String>,
    resolver_transform: Option<ResolverTransformSpec>,
    condition: Option<syn::LitStr>,
    when: Option<Path>,
    stop: Option<Path>,
    stop_lookup_by: Option<FieldSpec>,
    emit: Option<bool>,
}

impl Parse for MapAttributeArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // Parse single path or array of paths (like AggregateAttributeArgs::from)
        let mut source_paths = Vec::new();
        if input.peek(syn::token::Bracket) {
            let content;
            syn::bracketed!(content in input);
            while !content.is_empty() {
                source_paths.push(content.parse()?);
                if !content.is_empty() {
                    content.parse::<Token![,]>()?;
                }
            }
        } else {
            source_paths.push(input.parse()?);
        }

        let mut is_primary_key = false;
        let mut is_lookup_index = false;
        let mut register_from = Vec::new();
        let mut temporal_field = None;
        let mut strategy = None;
        let mut rename = None;
        let mut join_on = None;
        let mut transform = None;
        let mut resolver_transform = None;
        let mut condition = None;
        let mut when = None;
        let mut stop = None;
        let mut stop_lookup_by = None;
        let mut emit = None;

        while !input.is_empty() {
            input.parse::<Token![,]>()?;

            if input.is_empty() {
                break;
            }

            if input.peek(syn::Ident) {
                let ident: syn::Ident = input.parse()?;
                let ident_str = ident.to_string();

                if ident_str == "primary_key" {
                    is_primary_key = true;
                } else if ident_str == "lookup_index" {
                    is_lookup_index = true;
                    if input.peek(syn::token::Paren) {
                        let content;
                        syn::parenthesized!(content in input);
                        register_from = parse_register_from_list(&content)?;
                    }
                } else if ident_str == "temporal_field" {
                    input.parse::<Token![=]>()?;
                    let temporal_lit: syn::LitStr = input.parse()?;
                    temporal_field = Some(temporal_lit.value());
                } else if ident_str == "strategy" {
                    input.parse::<Token![=]>()?;
                    let strategy_ident: syn::Ident = input.parse()?;
                    strategy = Some(strategy_ident.to_string());
                } else if ident_str == "rename" {
                    input.parse::<Token![=]>()?;
                    let rename_lit: syn::LitStr = input.parse()?;
                    rename = Some(rename_lit.value());
                } else if ident_str == "join_on" {
                    input.parse::<Token![=]>()?;
                    if input.peek(syn::LitStr) {
                        let join_on_lit: syn::LitStr = input.parse()?;
                        join_on = Some(FieldSpec {
                            ident: syn::Ident::new(&join_on_lit.value(), join_on_lit.span()),
                            explicit_location: None,
                        });
                    } else {
                        join_on = Some(parse_field_spec(input)?);
                    }
                } else if ident_str == "transform" {
                    input.parse::<Token![=]>()?;
                    let transform_ident: syn::Ident = input.parse()?;
                    if input.peek(syn::token::Paren) {
                        let content;
                        syn::parenthesized!(content in input);
                        let args: proc_macro2::TokenStream = content.parse()?;
                        resolver_transform = Some(ResolverTransformSpec {
                            method: transform_ident.to_string(),
                            args,
                        });
                    } else {
                        transform = Some(transform_ident.to_string());
                    }
                } else if ident_str == "condition" {
                    input.parse::<Token![=]>()?;
                    let condition_lit: syn::LitStr = input.parse()?;
                    condition = Some(condition_lit);
                } else if ident_str == "when" {
                    input.parse::<Token![=]>()?;
                    let when_path: Path = input.parse()?;
                    when = Some(when_path);
                } else if ident_str == "stop" {
                    input.parse::<Token![=]>()?;
                    let stop_path: Path = input.parse()?;
                    stop = Some(stop_path);
                } else if ident_str == "stop_lookup_by" {
                    input.parse::<Token![=]>()?;
                    stop_lookup_by = Some(parse_field_spec(input)?);
                } else if ident_str == "emit" {
                    input.parse::<Token![=]>()?;
                    let emit_lit: syn::LitBool = input.parse()?;
                    emit = Some(emit_lit.value);
                } else {
                    return Err(syn::Error::new(
                        ident.span(),
                        format!("Unknown attribute argument: {}", ident_str),
                    ));
                }
            } else {
                return Err(input.error("Expected identifier"));
            }
        }

        Ok(MapAttributeArgs {
            source_paths,
            is_primary_key,
            is_lookup_index,
            register_from,
            temporal_field,
            strategy,
            rename,
            join_on,
            transform,
            resolver_transform,
            condition,
            when,
            stop,
            stop_lookup_by,
            emit,
        })
    }
}

pub fn parse_map_attribute(
    attr: &Attribute,
    target_field_name: &str,
) -> syn::Result<Option<Vec<MapAttribute>>> {
    if !attr.path().is_ident("map") {
        return Ok(None);
    }

    let args: MapAttributeArgs = attr.parse_args()?;

    if args.source_paths.is_empty() {
        return Err(syn::Error::new_spanned(
            attr,
            "#[map] requires at least one source path",
        ));
    }

    let strategy = validate_strategy(
        "#[map]",
        args.strategy.unwrap_or_else(|| "SetOnce".to_string()),
        attr,
        &["SetOnce", "LastWrite"],
    )?;
    let target_name = args.rename.unwrap_or_else(|| target_field_name.to_string());
    let emit = args.emit.unwrap_or(true);

    let mut results = Vec::new();
    for source_path in args.source_paths {
        let split = split_source_path(&source_path)?;
        let (is_instruction, is_account_source) =
            classify_source_type_path(&split.source_type_path);

        results.push(MapAttribute {
            attr_span: attr.span(),
            source_type_span: split.source_type_span,
            source_field_span: split.source_field_span,
            is_event_source: false,
            is_account_source,
            source_type_path: split.source_type_path,
            source_field_name: split.source_field_name,
            target_field_name: target_name.clone(),
            is_primary_key: args.is_primary_key,
            is_lookup_index: args.is_lookup_index,
            register_from: args.register_from.clone(),
            temporal_field: args.temporal_field.clone(),
            strategy: strategy.clone(),
            join_on: args.join_on.clone(),
            transform: args.transform.clone(),
            resolver_transform: args.resolver_transform.clone(),
            is_instruction,
            is_whole_source: false,
            lookup_by: None,
            condition: args
                .condition
                .as_ref()
                .map(parse_condition_literal)
                .transpose()?,
            when: args.when.clone(),
            stop: args.stop.clone(),
            stop_lookup_by: args.stop_lookup_by.clone(),
            emit,
        });
    }

    Ok(Some(results))
}

pub fn parse_from_instruction_attribute(
    attr: &Attribute,
    target_field_name: &str,
) -> syn::Result<Option<Vec<MapAttribute>>> {
    if !attr.path().is_ident("from_instruction") {
        return Ok(None);
    }

    let args: MapAttributeArgs = attr.parse_args()?;

    if args.source_paths.is_empty() {
        return Err(syn::Error::new_spanned(
            attr,
            "#[from_instruction] requires at least one source path",
        ));
    }

    let strategy = validate_strategy(
        "#[from_instruction]",
        args.strategy.unwrap_or_else(|| "SetOnce".to_string()),
        attr,
        &["SetOnce", "LastWrite"],
    )?;
    let target_name = args.rename.unwrap_or_else(|| target_field_name.to_string());
    let emit = args.emit.unwrap_or(true);

    let mut results = Vec::new();
    for source_path in args.source_paths {
        let split = split_source_path(&source_path)?;

        results.push(MapAttribute {
            attr_span: attr.span(),
            source_type_span: split.source_type_span,
            source_field_span: split.source_field_span,
            is_event_source: false,
            is_account_source: false,
            source_type_path: split.source_type_path,
            source_field_name: split.source_field_name,
            target_field_name: target_name.clone(),
            is_primary_key: args.is_primary_key,
            is_lookup_index: args.is_lookup_index,
            register_from: args.register_from.clone(),
            temporal_field: args.temporal_field.clone(),
            strategy: strategy.clone(),
            join_on: args.join_on.clone(),
            transform: args.transform.clone(),
            resolver_transform: args.resolver_transform.clone(),
            is_instruction: true,
            is_whole_source: false,
            lookup_by: None,
            condition: args
                .condition
                .as_ref()
                .map(parse_condition_literal)
                .transpose()?,
            when: args.when.clone(),
            stop: args.stop.clone(),
            stop_lookup_by: args.stop_lookup_by.clone(),
            emit,
        });
    }

    Ok(Some(results))
}

struct SplitSourcePath {
    source_type_path: Path,
    source_type_span: Span,
    source_field_name: String,
    source_field_span: Span,
}

fn split_source_path(path: &Path) -> syn::Result<SplitSourcePath> {
    if path.segments.len() < 2 {
        return Err(syn::Error::new_spanned(
            path,
            "Source path must be in format ModulePath::TypeName::field_name",
        ));
    }

    let field_segment = path.segments.last().unwrap();
    let field_name = field_segment.ident.to_string();
    let field_span = field_segment.ident.span();

    let mut type_path = path.clone();
    type_path.segments.pop();

    let type_span = type_path
        .segments
        .last()
        .map(|segment| segment.ident.span())
        .unwrap_or_else(Span::call_site);

    Ok(SplitSourcePath {
        source_type_path: type_path,
        source_type_span: type_span,
        source_field_name: field_name,
        source_field_span: field_span,
    })
}

struct EventAttributeArgs {
    // New type-safe syntax
    from: Option<Path>,
    from_span: Option<Span>,
    fields: Option<Vec<FieldSpec>>,
    transforms: Option<Vec<FieldTransform>>,

    // Backward compatibility
    instruction: Option<String>,
    instruction_span: Option<Span>,
    capture: Option<Vec<String>>,
    transforms_legacy: Option<HashMap<String, String>>,

    // Common fields
    strategy: Option<syn::Ident>,
    rename: Option<String>,
    join_on: Option<FieldSpec>,
    lookup_by: Option<FieldSpec>,
}

struct FieldTransform {
    field: syn::Ident,
    transform: syn::Ident,
}

impl Parse for EventAttributeArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut from = None;
        let mut from_span = None;
        let mut fields = None;
        let mut transforms = None;
        let mut transforms_legacy = None;
        let mut instruction = None;
        let mut instruction_span = None;
        let mut capture = None;
        let mut strategy = None;
        let mut rename = None;
        let mut join_on = None;
        let mut lookup_by = None;

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            let ident_str = ident.to_string();

            input.parse::<Token![=]>()?;

            if ident_str == "from" {
                // New: from = InstructionType
                let parsed: Path = input.parse()?;
                from_span = Some(parsed.span());
                from = Some(parsed);
            } else if ident_str == "fields" {
                // New: fields = [ident1, ident2] or fields = [accounts::ident1, args::ident2]
                let content;
                syn::bracketed!(content in input);

                let mut field_specs = Vec::new();
                while !content.is_empty() {
                    field_specs.push(parse_field_spec(&content)?);

                    if !content.is_empty() {
                        content.parse::<Token![,]>()?;
                    }
                }
                fields = Some(field_specs);
            } else if ident_str == "instruction" {
                // Legacy: instruction = "string"
                let lit: syn::LitStr = input.parse()?;
                instruction_span = Some(lit.span());
                instruction = Some(lit.value());
            } else if ident_str == "capture" {
                // Legacy/Alias for fields: capture = ["string1", "string2"]
                let content;
                syn::bracketed!(content in input);

                // Try to parse as identifiers first, fall back to strings
                let mut field_specs = Vec::new();
                let mut string_fields = Vec::new();
                let mut is_string_mode = false;

                while !content.is_empty() {
                    if content.peek(syn::LitStr) {
                        // Legacy string mode
                        is_string_mode = true;
                        let field_lit: syn::LitStr = content.parse()?;
                        string_fields.push(field_lit.value());
                    } else {
                        // New identifier mode
                        field_specs.push(parse_field_spec(&content)?);
                    }

                    if !content.is_empty() {
                        content.parse::<Token![,]>()?;
                    }
                }

                if is_string_mode {
                    capture = Some(string_fields);
                } else {
                    fields = Some(field_specs);
                }
            } else if ident_str == "transforms" {
                // Try to parse as new syntax first: [(ident1, Transform1), (ident2, Transform2)]
                let content;
                syn::bracketed!(content in input);

                let mut new_transforms = Vec::new();
                let mut legacy_transforms = HashMap::new();
                let mut is_legacy = false;

                while !content.is_empty() {
                    let tuple_content;
                    syn::parenthesized!(tuple_content in content);

                    if tuple_content.peek(syn::LitStr) {
                        // Legacy: ("string", Transform)
                        is_legacy = true;
                        let field_name: syn::LitStr = tuple_content.parse()?;
                        tuple_content.parse::<Token![,]>()?;
                        let transform_type: syn::Ident = tuple_content.parse()?;
                        legacy_transforms.insert(field_name.value(), transform_type.to_string());
                    } else {
                        // New: (ident, Transform)
                        let field: syn::Ident = tuple_content.parse()?;
                        tuple_content.parse::<Token![,]>()?;
                        let transform_type: syn::Ident = tuple_content.parse()?;
                        new_transforms.push(FieldTransform {
                            field,
                            transform: transform_type,
                        });
                    }

                    if !content.is_empty() {
                        content.parse::<Token![,]>()?;
                    }
                }

                if is_legacy {
                    transforms_legacy = Some(legacy_transforms);
                } else {
                    transforms = Some(new_transforms);
                }
            } else if ident_str == "strategy" {
                strategy = Some(input.parse()?);
            } else if ident_str == "rename" {
                let rename_lit: syn::LitStr = input.parse()?;
                rename = Some(rename_lit.value());
            } else if ident_str == "join_on" {
                // Try identifier first, fall back to string
                if input.peek(syn::LitStr) {
                    let join_on_lit: syn::LitStr = input.parse()?;
                    let ident = syn::Ident::new(&join_on_lit.value(), join_on_lit.span());
                    join_on = Some(FieldSpec {
                        ident,
                        explicit_location: None,
                    });
                } else {
                    join_on = Some(parse_field_spec(input)?);
                }
            } else if ident_str == "lookup_by" {
                // Try identifier first, fall back to string
                if input.peek(syn::LitStr) {
                    let lookup_by_lit: syn::LitStr = input.parse()?;
                    let ident = syn::Ident::new(&lookup_by_lit.value(), lookup_by_lit.span());
                    lookup_by = Some(FieldSpec {
                        ident,
                        explicit_location: None,
                    });
                } else {
                    lookup_by = Some(parse_field_spec(input)?);
                }
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("Unknown event attribute argument: {}", ident_str),
                ));
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(EventAttributeArgs {
            from,
            from_span,
            fields,
            transforms,
            instruction,
            instruction_span,
            capture,
            transforms_legacy,
            strategy,
            rename,
            join_on,
            lookup_by,
        })
    }
}

// Helper function to parse a field spec (ident or location::ident)
fn parse_field_spec(input: ParseStream) -> syn::Result<FieldSpec> {
    let lookahead = input.lookahead1();

    if lookahead.peek(syn::Ident) {
        let first_ident: syn::Ident = input.parse()?;

        // Check if there's a :: following
        if input.peek(Token![::]) {
            input.parse::<Token![::]>()?;
            let second_ident: syn::Ident = input.parse()?;

            // first_ident is the location (accounts, args, data)
            let location = match first_ident.to_string().as_str() {
                "accounts" => Some(FieldLocation::Account),
                "args" | "data" => Some(FieldLocation::InstructionArg),
                _ => {
                    return Err(syn::Error::new(
                        first_ident.span(),
                        format!(
                            "Invalid field location '{}'. Use 'accounts', 'args', or 'data'",
                            first_ident
                        ),
                    ));
                }
            };

            Ok(FieldSpec {
                ident: second_ident,
                explicit_location: location,
            })
        } else {
            // Just an identifier without location
            Ok(FieldSpec {
                ident: first_ident,
                explicit_location: None,
            })
        }
    } else {
        Err(lookahead.error())
    }
}

pub fn parse_event_attribute(
    attr: &Attribute,
    target_field_name: &str,
) -> syn::Result<Option<EventAttribute>> {
    if !attr.path().is_ident("event") {
        return Ok(None);
    }

    let args: EventAttributeArgs = attr.parse_args()?;

    let target_name = args.rename.unwrap_or_else(|| target_field_name.to_string());

    // Convert new-style transforms to HashMap
    let field_transforms = if let Some(transforms) = args.transforms {
        transforms
            .into_iter()
            .map(|ft| (ft.field.to_string(), ft.transform))
            .collect()
    } else {
        HashMap::new()
    };

    // For backward compatibility, convert legacy transforms
    let field_transforms_legacy = args.transforms_legacy.unwrap_or_default();

    // Determine strategy
    let strategy = validate_strategy(
        "#[event]",
        args.strategy
            .map(|s| s.to_string())
            .unwrap_or_else(|| "SetOnce".to_string()),
        attr,
        &["SetOnce", "LastWrite"],
    )?;

    // Handle legacy instruction string
    let instruction_str = args.instruction.unwrap_or_default();

    Ok(Some(EventAttribute {
        attr_span: attr.span(),
        instruction_span: args.from_span.or(args.instruction_span),
        from_instruction: args.from,
        inferred_instruction: None, // Will be filled in later from field type
        capture_fields: args.fields.unwrap_or_default(),
        field_transforms,
        instruction: instruction_str,
        capture_fields_legacy: args.capture.unwrap_or_default(),
        field_transforms_legacy,
        strategy,
        target_field_name: target_name,
        join_on: args.join_on,
        lookup_by: args.lookup_by,
    }))
}

// Parse args for #[snapshot] attribute
struct SnapshotAttributeArgs {
    from: Option<Path>,
    field: Option<syn::Ident>,
    strategy: Option<syn::Ident>,
    rename: Option<String>,
    join_on: Option<FieldSpec>,
    lookup_by: Option<FieldSpec>,
    transforms: Vec<(String, syn::Ident)>, // Field transformations: (field_name, transform)
    when: Option<Path>,
}

impl Parse for SnapshotAttributeArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut from = None;
        let mut field = None;
        let mut strategy = None;
        let mut rename = None;
        let mut join_on = None;
        let mut lookup_by = None;
        let mut transforms = Vec::new();
        let mut when = None;

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            let ident_str = ident.to_string();

            input.parse::<Token![=]>()?;

            if ident_str == "from" {
                from = Some(input.parse()?);
            } else if ident_str == "field" {
                field = Some(input.parse()?);
            } else if ident_str == "strategy" {
                strategy = Some(input.parse()?);
            } else if ident_str == "rename" {
                let rename_lit: syn::LitStr = input.parse()?;
                rename = Some(rename_lit.value());
            } else if ident_str == "join_on" {
                if input.peek(syn::LitStr) {
                    let join_on_lit: syn::LitStr = input.parse()?;
                    let ident = syn::Ident::new(&join_on_lit.value(), join_on_lit.span());
                    join_on = Some(FieldSpec {
                        ident,
                        explicit_location: None,
                    });
                } else {
                    join_on = Some(parse_field_spec(input)?);
                }
            } else if ident_str == "lookup_by" {
                if input.peek(syn::LitStr) {
                    let lookup_by_lit: syn::LitStr = input.parse()?;
                    let ident = syn::Ident::new(&lookup_by_lit.value(), lookup_by_lit.span());
                    lookup_by = Some(FieldSpec {
                        ident,
                        explicit_location: None,
                    });
                } else {
                    lookup_by = Some(parse_field_spec(input)?);
                }
            } else if ident_str == "transforms" {
                // Parse transforms = [(field1, Transform1), (field2, Transform2)]
                let content;
                syn::bracketed!(content in input);

                while !content.is_empty() {
                    let tuple_content;
                    syn::parenthesized!(tuple_content in content);

                    let field_name: syn::Ident = tuple_content.parse()?;
                    tuple_content.parse::<Token![,]>()?;
                    let transform: syn::Ident = tuple_content.parse()?;

                    transforms.push((field_name.to_string(), transform));

                    if !content.is_empty() {
                        content.parse::<Token![,]>()?;
                    }
                }
            } else if ident_str == "when" {
                when = Some(input.parse()?);
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("Unknown snapshot attribute argument: {}", ident_str),
                ));
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(SnapshotAttributeArgs {
            from,
            field,
            strategy,
            rename,
            join_on,
            lookup_by,
            transforms,
            when,
        })
    }
}

pub fn parse_snapshot_attribute(
    attr: &Attribute,
    target_field_name: &str,
) -> syn::Result<Option<CaptureAttribute>> {
    if !attr.path().is_ident("snapshot") {
        return Ok(None);
    }

    let args: SnapshotAttributeArgs = attr.parse_args()?;

    let target_name = args.rename.unwrap_or_else(|| target_field_name.to_string());

    // Determine strategy - only SetOnce or LastWrite allowed
    let strategy = validate_strategy(
        "#[snapshot]",
        args.strategy
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "SetOnce".to_string()),
        attr,
        &["SetOnce", "LastWrite"],
    )?;

    Ok(Some(CaptureAttribute {
        attr_span: attr.span(),
        from_account: args.from,
        inferred_account: None, // Will be filled in later from field type
        field: args.field,
        field_transforms: args.transforms.into_iter().collect(),
        strategy,
        target_field_name: target_name,
        join_on: args.join_on,
        lookup_by: args.lookup_by,
        when: args.when,
    }))
}

// ============================================================================
// Aggregate Macro - Declarative Aggregations
// ============================================================================

#[derive(Debug, Clone)]
pub struct AggregateAttribute {
    pub attr_span: Span,
    /// Instruction type(s) to aggregate from
    pub from_instructions: Vec<Path>,
    /// Field to aggregate (optional - if omitted, just count occurrences)
    pub field: Option<FieldSpec>,
    /// Aggregation strategy
    pub strategy: String,
    /// Transform to apply to the field before aggregating
    pub transform: Option<syn::Ident>,
    /// Target field name (defaults to struct field name)
    pub target_field_name: String,
    /// Join condition for multi-entity scenarios
    pub join_on: Option<FieldSpec>,
    /// Lookup field for key resolution
    pub lookup_by: Option<FieldSpec>,
    /// Condition expression for conditional aggregation (Level 1)
    pub condition: Option<ConditionExpr>,
}

struct AggregateAttributeArgs {
    from: Vec<Path>,
    field: Option<FieldSpec>,
    strategy: Option<syn::Ident>,
    transform: Option<syn::Ident>,
    rename: Option<String>,
    join_on: Option<FieldSpec>,
    lookup_by: Option<FieldSpec>,
    condition: Option<syn::LitStr>,
}

impl Parse for AggregateAttributeArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut from = Vec::new();
        let mut field = None;
        let mut strategy = None;
        let mut transform = None;
        let mut rename = None;
        let mut join_on = None;
        let mut lookup_by = None;
        let mut condition = None;

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            let ident_str = ident.to_string();

            input.parse::<Token![=]>()?;

            if ident_str == "from" {
                // Parse single instruction or array of instructions
                if input.peek(syn::token::Bracket) {
                    let content;
                    syn::bracketed!(content in input);
                    while !content.is_empty() {
                        from.push(content.parse()?);
                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                } else {
                    from.push(input.parse()?);
                }
            } else if ident_str == "field" {
                field = Some(parse_field_spec(input)?);
            } else if ident_str == "strategy" {
                strategy = Some(input.parse()?);
            } else if ident_str == "transform" {
                transform = Some(input.parse()?);
            } else if ident_str == "rename" {
                let rename_lit: syn::LitStr = input.parse()?;
                rename = Some(rename_lit.value());
            } else if ident_str == "join_on" {
                if input.peek(syn::LitStr) {
                    let join_on_lit: syn::LitStr = input.parse()?;
                    let ident = syn::Ident::new(&join_on_lit.value(), join_on_lit.span());
                    join_on = Some(FieldSpec {
                        ident,
                        explicit_location: None,
                    });
                } else {
                    join_on = Some(parse_field_spec(input)?);
                }
            } else if ident_str == "lookup_by" {
                if input.peek(syn::LitStr) {
                    let lookup_by_lit: syn::LitStr = input.parse()?;
                    let ident = syn::Ident::new(&lookup_by_lit.value(), lookup_by_lit.span());
                    lookup_by = Some(FieldSpec {
                        ident,
                        explicit_location: None,
                    });
                } else {
                    lookup_by = Some(parse_field_spec(input)?);
                }
            } else if ident_str == "condition" {
                let condition_lit: syn::LitStr = input.parse()?;
                condition = Some(condition_lit);
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("Unknown aggregate attribute argument: {}", ident_str),
                ));
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(AggregateAttributeArgs {
            from,
            field,
            strategy,
            transform,
            rename,
            join_on,
            lookup_by,
            condition,
        })
    }
}

pub fn parse_aggregate_attribute(
    attr: &Attribute,
    target_field_name: &str,
) -> syn::Result<Option<AggregateAttribute>> {
    if !attr.path().is_ident("aggregate") {
        return Ok(None);
    }

    let args: AggregateAttributeArgs = attr.parse_args()?;

    if args.from.is_empty() {
        return Err(syn::Error::new_spanned(
            attr,
            "#[aggregate] requires 'from' parameter specifying instruction type(s)",
        ));
    }

    let target_name = args.rename.unwrap_or_else(|| target_field_name.to_string());

    // Determine strategy - default to Count if no field specified, Sum otherwise
    let strategy = if let Some(ref strategy_ident) = args.strategy {
        validate_strategy(
            "#[aggregate]",
            strategy_ident.to_string(),
            strategy_ident,
            &["Sum", "Count", "Min", "Max", "UniqueCount"],
        )?
    } else {
        // Default strategy based on whether field is specified
        if args.field.is_none() {
            "Count".to_string()
        } else {
            "Sum".to_string()
        }
    };

    Ok(Some(AggregateAttribute {
        attr_span: attr.span(),
        from_instructions: args.from,
        field: args.field,
        strategy,
        transform: args.transform,
        target_field_name: target_name,
        join_on: args.join_on,
        lookup_by: args.lookup_by,
        condition: args
            .condition
            .as_ref()
            .map(parse_condition_literal)
            .transpose()?,
    }))
}

// ============================================================================
// Computed Macro - Declarative Computed Fields
// ============================================================================

#[derive(Debug, Clone)]
pub struct ResolveAttribute {
    pub attr_span: Span,
    pub from_span: Option<Span>,
    pub from: Option<String>,
    pub address: Option<String>,
    pub url: Option<String>,
    pub url_is_template: bool,
    pub method: Option<String>,
    pub extract: Option<String>,
    pub target_field_name: String,
    pub resolver: Option<String>,
    pub strategy: String,
    pub condition: Option<ValidatedResolverCondition>,
    pub schedule_at: Option<ValidatedFieldPath>,
}

#[derive(Debug, Clone)]
pub struct ResolveSpec {
    pub attr_span: Span,
    pub from_span: Option<Span>,
    pub resolver: ResolverType,
    pub from: Option<String>,
    pub address: Option<String>,
    pub extract: Option<String>,
    pub target_field_name: String,
    pub strategy: String,
    pub condition: Option<ValidatedResolverCondition>,
    pub schedule_at: Option<ValidatedFieldPath>,
}

struct ResolveAttributeArgs {
    from: Option<String>,
    from_span: Option<Span>,
    address: Option<String>,
    url: Option<String>,
    url_is_template: bool,
    method: Option<syn::Ident>,
    extract: Option<String>,
    resolver: Option<String>,
    strategy: Option<String>,
    condition: Option<syn::LitStr>,
    schedule_at: Option<ValidatedFieldPath>,
}

impl Parse for ResolveAttributeArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut from = None;
        let mut from_span = None;
        let mut address = None;
        let mut url = None;
        let mut url_is_template = false;
        let mut method = None;
        let mut extract = None;
        let mut resolver = None;
        let mut strategy = None;
        let mut condition = None;
        let mut schedule_at = None;

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            let ident_str = ident.to_string();

            input.parse::<Token![=]>()?;

            if ident_str == "from" {
                let lit: syn::LitStr = input.parse()?;
                from_span = Some(lit.span());
                from = Some(lit.value());
            } else if ident_str == "address" {
                let lit: syn::LitStr = input.parse()?;
                address = Some(lit.value());
            } else if ident_str == "url" {
                if input.peek(syn::LitStr) {
                    let lit: syn::LitStr = input.parse()?;
                    url = Some(lit.value());
                    url_is_template = true;
                } else {
                    let mut parts = Vec::new();
                    let first: syn::Ident = input.parse()?;
                    parts.push(first.to_string());

                    while input.peek(Token![.]) {
                        input.parse::<Token![.]>()?;
                        let next: syn::Ident = input.parse()?;
                        parts.push(next.to_string());
                    }

                    url = Some(parts.join("."));
                }
            } else if ident_str == "method" {
                let method_ident: syn::Ident = input.parse()?;
                match method_ident.to_string().to_lowercase().as_str() {
                    "get" | "post" => method = Some(method_ident),
                    _ => {
                        return Err(syn::Error::new(
                            method_ident.span(),
                            "Invalid HTTP method. Only 'GET' or 'POST' are supported.",
                        ))
                    }
                }
            } else if ident_str == "extract" {
                let lit: syn::LitStr = input.parse()?;
                extract = Some(lit.value());
            } else if ident_str == "resolver" {
                if input.peek(syn::LitStr) {
                    let lit: syn::LitStr = input.parse()?;
                    resolver = Some(lit.value());
                } else {
                    let ident: syn::Ident = input.parse()?;
                    resolver = Some(ident.to_string());
                }
            } else if ident_str == "strategy" {
                let ident: syn::Ident = input.parse()?;
                strategy = Some(ident.to_string());
            } else if ident_str == "condition" {
                let lit: syn::LitStr = input.parse()?;
                condition = Some(lit);
            } else if ident_str == "schedule_at" {
                schedule_at = Some(parse_validated_field_path(input)?);
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("Unknown resolve attribute argument: {}", ident_str),
                ));
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(ResolveAttributeArgs {
            from,
            from_span,
            address,
            url,
            url_is_template,
            method,
            extract,
            resolver,
            strategy,
            condition,
            schedule_at,
        })
    }
}

pub fn parse_resolve_attribute(
    attr: &Attribute,
    target_field_name: &str,
) -> syn::Result<Option<ResolveAttribute>> {
    if !attr.path().is_ident("resolve") {
        return Ok(None);
    }

    let args: ResolveAttributeArgs = attr.parse_args()?;

    // Check for mutually exclusive parameters: url vs (from/address)
    let has_url = args.url.is_some();
    let has_token_source = args.from.is_some() || args.address.is_some();

    if has_url && has_token_source {
        return Err(syn::Error::new_spanned(
            attr,
            "#[resolve] cannot specify 'url' together with 'from' or 'address'",
        ));
    }

    if !has_url && !has_token_source {
        return Err(syn::Error::new_spanned(
            attr,
            "#[resolve] requires either 'url' or 'from'/'address' parameter",
        ));
    }

    // Token resolvers: cannot have both from and address
    if args.from.is_some() && args.address.is_some() {
        return Err(syn::Error::new_spanned(
            attr,
            "#[resolve] cannot specify both 'from' and 'address'",
        ));
    }

    // URL resolvers require extract parameter
    if has_url && args.extract.is_none() {
        return Err(syn::Error::new_spanned(
            attr,
            "#[resolve] with 'url' requires 'extract' parameter",
        ));
    }

    let strategy = validate_strategy(
        "#[resolve]",
        args.strategy.unwrap_or_else(|| "SetOnce".to_string()),
        attr,
        &["SetOnce", "LastWrite"],
    )?;

    Ok(Some(ResolveAttribute {
        attr_span: attr.span(),
        from_span: args.from_span,
        from: args.from,
        address: args.address,
        url: args.url,
        url_is_template: args.url_is_template,
        method: args.method.map(|m| m.to_string()),
        extract: args.extract,
        target_field_name: target_field_name.to_string(),
        resolver: args.resolver,
        strategy,
        condition: args
            .condition
            .as_ref()
            .map(parse_resolver_condition_literal)
            .transpose()?,
        schedule_at: args.schedule_at,
    }))
}

#[derive(Debug, Clone)]
pub struct ComputedAttribute {
    pub attr_span: Span,
    /// The expression to evaluate (stored as TokenStream for code generation)
    pub expression: proc_macro2::TokenStream,
    /// Target field name (defaults to struct field name)
    pub target_field_name: String,
}

/// Parse #[computed(expression)] attribute
pub fn parse_computed_attribute(
    attr: &Attribute,
    target_field_name: &str,
) -> syn::Result<Option<ComputedAttribute>> {
    if !attr.path().is_ident("computed") {
        return Ok(None);
    }

    // Parse the expression inside the attribute
    // e.g., #[computed(total_buy_volume.unwrap_or(0) + total_sell_volume.unwrap_or(0))]
    let expression: proc_macro2::TokenStream = attr.parse_args()?;

    Ok(Some(ComputedAttribute {
        attr_span: attr.span(),
        expression,
        target_field_name: target_field_name.to_string(),
    }))
}

#[derive(Debug, Clone)]
pub enum RecognizedFieldAttribute {
    Map(Vec<MapAttribute>),
    FromInstruction(Vec<MapAttribute>),
    Event(EventAttribute),
    Snapshot(CaptureAttribute),
    Aggregate(AggregateAttribute),
    DeriveFrom(DeriveFromAttribute),
    Resolve(ResolveAttribute),
    Computed(ComputedAttribute),
}

pub fn parse_recognized_field_attribute(
    attr: &Attribute,
    target_field_name: &str,
) -> syn::Result<Option<RecognizedFieldAttribute>> {
    if let Some(map_attrs) = parse_map_attribute(attr, target_field_name)? {
        return Ok(Some(RecognizedFieldAttribute::Map(map_attrs)));
    }

    if let Some(map_attrs) = parse_from_instruction_attribute(attr, target_field_name)? {
        return Ok(Some(RecognizedFieldAttribute::FromInstruction(map_attrs)));
    }

    if let Some(event_attr) = parse_event_attribute(attr, target_field_name)? {
        return Ok(Some(RecognizedFieldAttribute::Event(event_attr)));
    }

    if let Some(snapshot_attr) = parse_snapshot_attribute(attr, target_field_name)? {
        return Ok(Some(RecognizedFieldAttribute::Snapshot(snapshot_attr)));
    }

    if let Some(aggregate_attr) = parse_aggregate_attribute(attr, target_field_name)? {
        return Ok(Some(RecognizedFieldAttribute::Aggregate(aggregate_attr)));
    }

    if let Some(derive_attr) = parse_derive_from_attribute(attr, target_field_name)? {
        return Ok(Some(RecognizedFieldAttribute::DeriveFrom(derive_attr)));
    }

    if let Some(resolve_attr) = parse_resolve_attribute(attr, target_field_name)? {
        return Ok(Some(RecognizedFieldAttribute::Resolve(resolve_attr)));
    }

    if let Some(computed_attr) = parse_computed_attribute(attr, target_field_name)? {
        return Ok(Some(RecognizedFieldAttribute::Computed(computed_attr)));
    }

    Ok(None)
}

pub fn has_entity_attribute(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("entity"))
}

pub fn parse_entity_name(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("entity") {
            if let syn::Meta::List(meta_list) = &attr.meta {
                let tokens_str = meta_list.tokens.to_string();
                if tokens_str.contains("name") {
                    if let Ok(parsed) = syn::parse_str::<syn::ExprAssign>(&tokens_str) {
                        if let syn::Expr::Lit(expr_lit) = &*parsed.right {
                            if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                                return Some(lit_str.value());
                            }
                        }
                    }
                }
            }
            return None;
        }
    }
    None
}

#[derive(Debug, Clone)]
pub struct StreamSpecAttribute {
    pub proto_files: Vec<String>,
    pub idl_files: Vec<String>,
    pub skip_decoders: bool,
}

struct StreamSpecAttributeArgs {
    proto_files: Vec<String>,
    idl_files: Vec<String>,
    skip_decoders: bool,
}

impl Parse for StreamSpecAttributeArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut proto_files = Vec::new();
        let mut idl_files = Vec::new();
        let mut skip_decoders = false;

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            let ident_str = ident.to_string();

            if ident_str == "proto" {
                input.parse::<Token![=]>()?;

                if input.peek(syn::LitStr) {
                    let lit: syn::LitStr = input.parse()?;
                    proto_files.push(lit.value());
                } else if input.peek(syn::token::Bracket) {
                    let content;
                    syn::bracketed!(content in input);

                    while !content.is_empty() {
                        let file_lit: syn::LitStr = content.parse()?;
                        proto_files.push(file_lit.value());

                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                } else {
                    return Err(input.error("Expected string literal or array of string literals"));
                }
            } else if ident_str == "idl" {
                input.parse::<Token![=]>()?;

                if input.peek(syn::LitStr) {
                    let lit: syn::LitStr = input.parse()?;
                    idl_files.push(lit.value());
                } else if input.peek(syn::token::Bracket) {
                    let content;
                    syn::bracketed!(content in input);

                    while !content.is_empty() {
                        let file_lit: syn::LitStr = content.parse()?;
                        idl_files.push(file_lit.value());

                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                } else {
                    return Err(
                        input.error("Expected string literal or array of string literals for idl")
                    );
                }
            } else if ident_str == "skip_decoders" {
                skip_decoders = true;
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("Unknown stream_spec attribute argument: {}", ident_str),
                ));
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(StreamSpecAttributeArgs {
            proto_files,
            idl_files,
            skip_decoders,
        })
    }
}

pub fn parse_stream_spec_attribute(
    attr: proc_macro::TokenStream,
) -> Result<StreamSpecAttribute, syn::Error> {
    if attr.is_empty() {
        return Ok(StreamSpecAttribute {
            proto_files: Vec::new(),
            idl_files: Vec::new(),
            skip_decoders: false,
        });
    }

    let args: StreamSpecAttributeArgs = syn::parse(attr)?;

    for file in &args.proto_files {
        if file.trim().is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "proto file references cannot be empty",
            ));
        }
    }

    for file in &args.idl_files {
        if file.trim().is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "idl file references cannot be empty",
            ));
        }
    }

    Ok(StreamSpecAttribute {
        proto_files: args.proto_files,
        idl_files: args.idl_files,
        skip_decoders: args.skip_decoders,
    })
}

/// Parse #[resolve_key_for(AccountType)] attribute
pub fn parse_resolve_key_for_attribute(
    attr: &Attribute,
) -> syn::Result<Option<ResolverKeyForAttr>> {
    if !attr.path().is_ident("resolve_key_for") {
        return Ok(None);
    }

    // Parse the account type path inside the attribute
    let account_type_path: Path = attr.parse_args()?;

    Ok(Some(ResolverKeyForAttr { account_type_path }))
}

#[derive(Debug, Clone)]
pub struct ResolverKeyForAttr {
    pub account_type_path: Path,
}

/// Parse #[after_instruction(InstructionType)] attribute
///
/// NOTE: This attribute is no longer supported for direct use by users.
/// All instruction hooks must be defined using declarative macros:
/// - Use #[register_pda] to register PDA mappings
/// - Use #[derive_from] to derive fields from instructions
/// - Use #[aggregate] for aggregations with optional conditions
pub fn parse_after_instruction_attribute(
    attr: &Attribute,
) -> syn::Result<Option<AfterInstructionAttr>> {
    if !attr.path().is_ident("after_instruction") {
        return Ok(None);
    }

    // Reject all user-written #[after_instruction] attributes
    Err(syn::Error::new_spanned(
        attr,
        "Direct use of #[after_instruction] is not allowed.\n\
         \n\
         Use declarative macros instead:\n\
         \n\
         • Use #[register_pda(instruction = ..., pda_field = ..., primary_key = ...)] \
         to register PDA mappings\n\
         \n\
         • Use #[derive_from(from = [...], field = ..., strategy = ...)] \
         to derive fields from instructions\n\
         \n\
         • Use #[aggregate(from = [...], field = ..., strategy = ..., condition = \"...\")] \
         for conditional aggregations\n\
         \n\
          These declarative macros are captured in the AST and enable secure cloud deployment.\n\
         See documentation for examples.",
    ))
}

#[derive(Debug, Clone)]
pub struct AfterInstructionAttr {
    pub instruction_type_path: Path,
}

/// Extract resolver hooks from an impl block
pub fn extract_resolver_hooks(item_impl: &syn::ItemImpl) -> syn::Result<Vec<ResolverHookSpec>> {
    let mut hooks = Vec::new();
    let mut errors = ErrorCollector::default();

    for item in &item_impl.items {
        if let syn::ImplItem::Fn(method) = item {
            for attr in &method.attrs {
                match parse_resolve_key_for_attribute(attr) {
                    Ok(Some(resolver_attr)) => {
                        hooks.push(ResolverHookSpec {
                            kind: ResolverHookKind::KeyResolver,
                            account_type_path: resolver_attr.account_type_path,
                            fn_name: method.sig.ident.clone(),
                            fn_sig: method.sig.clone(),
                        });
                    }
                    Ok(None) => {}
                    Err(error) => errors.push(error),
                }

                match parse_after_instruction_attribute(attr) {
                    Ok(Some(instruction_attr)) => {
                        hooks.push(ResolverHookSpec {
                            kind: ResolverHookKind::AfterInstruction,
                            account_type_path: instruction_attr.instruction_type_path,
                            fn_name: method.sig.ident.clone(),
                            fn_sig: method.sig.clone(),
                        });
                    }
                    Ok(None) => {}
                    Err(error) => errors.push(error),
                }
            }
        }
    }

    errors.finish()?;
    Ok(hooks)
}

/// Extract resolver hooks from a standalone function
pub fn extract_resolver_hooks_from_fn(item_fn: &syn::ItemFn) -> syn::Result<Vec<ResolverHookSpec>> {
    let mut hooks = Vec::new();
    let mut errors = ErrorCollector::default();

    for attr in &item_fn.attrs {
        match parse_resolve_key_for_attribute(attr) {
            Ok(Some(resolver_attr)) => {
                hooks.push(ResolverHookSpec {
                    kind: ResolverHookKind::KeyResolver,
                    account_type_path: resolver_attr.account_type_path,
                    fn_name: item_fn.sig.ident.clone(),
                    fn_sig: item_fn.sig.clone(),
                });
            }
            Ok(None) => {}
            Err(error) => errors.push(error),
        }

        match parse_after_instruction_attribute(attr) {
            Ok(Some(instruction_attr)) => {
                hooks.push(ResolverHookSpec {
                    kind: ResolverHookKind::AfterInstruction,
                    account_type_path: instruction_attr.instruction_type_path,
                    fn_name: item_fn.sig.ident.clone(),
                    fn_sig: item_fn.sig.clone(),
                });
            }
            Ok(None) => {}
            Err(error) => errors.push(error),
        }
    }

    errors.finish()?;
    Ok(hooks)
}

#[derive(Debug, Clone)]
pub struct ResolverHookSpec {
    pub kind: ResolverHookKind,
    pub account_type_path: Path,
    pub fn_name: syn::Ident,
    pub fn_sig: syn::Signature,
}

#[derive(Debug, Clone)]
pub enum ResolverHookKind {
    KeyResolver,
    AfterInstruction,
}

// ============================================================================
// Level 1: New Declarative Macros
// ============================================================================

// #[derive_from] Attribute Parser
#[derive(Debug, Clone)]
pub struct DeriveFromAttribute {
    pub attr_span: Span,
    pub from_instructions: Vec<Path>,
    pub field: FieldSpec, // Can be special: __timestamp, __slot, __signature
    pub strategy: String, // LastWrite, SetOnce
    pub lookup_by: Option<FieldSpec>,
    pub condition: Option<ConditionExpr>,
    pub transform: Option<syn::Ident>,
    pub target_field_name: String,
}

struct DeriveFromAttributeArgs {
    from: Vec<Path>,
    field: Option<FieldSpec>,
    strategy: Option<syn::Ident>,
    lookup_by: Option<FieldSpec>,
    condition: Option<syn::LitStr>,
    transform: Option<syn::Ident>,
}

impl Parse for DeriveFromAttributeArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut from = Vec::new();
        let mut field = None;
        let mut strategy = None;
        let mut lookup_by = None;
        let mut condition = None;
        let mut transform = None;

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            let ident_str = ident.to_string();

            input.parse::<Token![=]>()?;

            if ident_str == "from" {
                // Parse single instruction or array of instructions
                if input.peek(syn::token::Bracket) {
                    let content;
                    syn::bracketed!(content in input);
                    while !content.is_empty() {
                        from.push(content.parse()?);
                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                } else {
                    from.push(input.parse()?);
                }
            } else if ident_str == "field" {
                field = Some(parse_field_spec(input)?);
            } else if ident_str == "strategy" {
                strategy = Some(input.parse()?);
            } else if ident_str == "lookup_by" {
                if input.peek(syn::LitStr) {
                    let lookup_by_lit: syn::LitStr = input.parse()?;
                    let ident = syn::Ident::new(&lookup_by_lit.value(), lookup_by_lit.span());
                    lookup_by = Some(FieldSpec {
                        ident,
                        explicit_location: None,
                    });
                } else {
                    lookup_by = Some(parse_field_spec(input)?);
                }
            } else if ident_str == "condition" {
                let condition_lit: syn::LitStr = input.parse()?;
                condition = Some(condition_lit);
            } else if ident_str == "transform" {
                transform = Some(input.parse()?);
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("Unknown derive_from attribute argument: {}", ident_str),
                ));
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(DeriveFromAttributeArgs {
            from,
            field,
            strategy,
            lookup_by,
            condition,
            transform,
        })
    }
}

pub fn parse_derive_from_attribute(
    attr: &Attribute,
    target_field_name: &str,
) -> syn::Result<Option<DeriveFromAttribute>> {
    if !attr.path().is_ident("derive_from") {
        return Ok(None);
    }

    let args: DeriveFromAttributeArgs = attr.parse_args()?;

    if args.from.is_empty() {
        return Err(syn::Error::new_spanned(
            attr,
            "#[derive_from] requires 'from' parameter specifying instruction type(s)",
        ));
    }

    let field = args.field.ok_or_else(|| {
        syn::Error::new_spanned(attr, "#[derive_from] requires 'field' parameter")
    })?;

    let strategy = validate_strategy(
        "#[derive_from]",
        args.strategy
            .map(|s| s.to_string())
            .unwrap_or_else(|| "LastWrite".to_string()),
        attr,
        &["SetOnce", "LastWrite"],
    )?;

    Ok(Some(DeriveFromAttribute {
        attr_span: attr.span(),
        from_instructions: args.from,
        field,
        strategy,
        lookup_by: args.lookup_by,
        condition: args
            .condition
            .as_ref()
            .map(parse_condition_literal)
            .transpose()?,
        transform: args.transform,
        target_field_name: target_field_name.to_string(),
    }))
}

fn parse_register_from_list(input: ParseStream) -> syn::Result<Vec<RegisterFromSpec>> {
    let ident: syn::Ident = input.parse()?;
    if ident != "register_from" {
        return Err(syn::Error::new(
            ident.span(),
            format!("Expected 'register_from', found '{}'", ident),
        ));
    }
    input.parse::<Token![=]>()?;

    let content;
    syn::bracketed!(content in input);

    let mut specs = Vec::new();
    while !content.is_empty() {
        let tuple_content;
        syn::parenthesized!(tuple_content in content);

        let instruction_path: Path = tuple_content.parse()?;
        tuple_content.parse::<Token![,]>()?;
        let pda_field = parse_field_spec(&tuple_content)?;
        tuple_content.parse::<Token![,]>()?;
        let primary_key_field = parse_field_spec(&tuple_content)?;

        specs.push(RegisterFromSpec {
            instruction_path,
            pda_field,
            primary_key_field,
        });

        if !content.is_empty() {
            content.parse::<Token![,]>()?;
        }
    }

    Ok(specs)
}

// #[resolve_key] Attribute Parser
#[derive(Debug, Clone)]
pub struct ResolveKeyAttribute {
    pub attr_span: Span,
    pub account_path: Path,
    pub strategy: String, // "pda_reverse_lookup", "direct_field"
    pub lookup_name: Option<String>,
    pub queue_until: Vec<Path>, // Instruction paths
}

struct ResolveKeyAttributeArgs {
    account: Option<Path>,
    strategy: Option<syn::Ident>,
    lookup_name: Option<String>,
    queue_until: Vec<Path>,
}

impl Parse for ResolveKeyAttributeArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut account = None;
        let mut strategy = None;
        let mut lookup_name = None;
        let mut queue_until = Vec::new();

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            let ident_str = ident.to_string();

            input.parse::<Token![=]>()?;

            if ident_str == "account" {
                account = Some(input.parse()?);
            } else if ident_str == "strategy" {
                let strategy_lit: syn::LitStr = input.parse()?;
                strategy = Some(syn::Ident::new(&strategy_lit.value(), strategy_lit.span()));
            } else if ident_str == "lookup_name" {
                let lookup_name_lit: syn::LitStr = input.parse()?;
                lookup_name = Some(lookup_name_lit.value());
            } else if ident_str == "queue_until" {
                if input.peek(syn::token::Bracket) {
                    let content;
                    syn::bracketed!(content in input);
                    while !content.is_empty() {
                        queue_until.push(content.parse()?);
                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                } else {
                    queue_until.push(input.parse()?);
                }
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("Unknown resolve_key attribute argument: {}", ident_str),
                ));
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(ResolveKeyAttributeArgs {
            account,
            strategy,
            lookup_name,
            queue_until,
        })
    }
}

pub fn parse_resolve_key_attribute(attr: &Attribute) -> syn::Result<Option<ResolveKeyAttribute>> {
    if !attr.path().is_ident("resolve_key") {
        return Ok(None);
    }

    let args: ResolveKeyAttributeArgs = attr.parse_args()?;

    let account_path = args.account.ok_or_else(|| {
        syn::Error::new_spanned(attr, "#[resolve_key] requires 'account' parameter")
    })?;

    let strategy = validate_strategy(
        "#[resolve_key]",
        args.strategy
            .map(|s| s.to_string())
            .unwrap_or_else(|| "pda_reverse_lookup".to_string()),
        attr,
        &["pda_reverse_lookup", "direct_field"],
    )?;

    Ok(Some(ResolveKeyAttribute {
        attr_span: attr.span(),
        account_path,
        strategy,
        lookup_name: args.lookup_name,
        queue_until: args.queue_until,
    }))
}

// #[register_pda] Attribute Parser
#[derive(Debug, Clone)]
pub struct RegisterPdaAttribute {
    pub attr_span: Span,
    pub instruction_path: Path,
    pub pda_field: FieldSpec,
    pub primary_key_field: FieldSpec, // The primary key to associate with the PDA
    pub lookup_name: String,
}

struct RegisterPdaAttributeArgs {
    instruction: Option<Path>,
    pda_field: Option<FieldSpec>,
    primary_key: Option<FieldSpec>,
    lookup_name: Option<String>,
}

impl Parse for RegisterPdaAttributeArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut instruction = None;
        let mut pda_field = None;
        let mut primary_key = None;
        let mut lookup_name = None;

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            let ident_str = ident.to_string();

            input.parse::<Token![=]>()?;

            if ident_str == "instruction" {
                instruction = Some(input.parse()?);
            } else if ident_str == "pda_field" {
                pda_field = Some(parse_field_spec(input)?);
            } else if ident_str == "primary_key" {
                primary_key = Some(parse_field_spec(input)?);
            } else if ident_str == "lookup_name" {
                let lookup_name_lit: syn::LitStr = input.parse()?;
                lookup_name = Some(lookup_name_lit.value());
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("Unknown register_pda attribute argument: {}", ident_str),
                ));
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(RegisterPdaAttributeArgs {
            instruction,
            pda_field,
            primary_key,
            lookup_name,
        })
    }
}

pub fn parse_register_pda_attribute(attr: &Attribute) -> syn::Result<Option<RegisterPdaAttribute>> {
    if !attr.path().is_ident("register_pda") {
        return Ok(None);
    }

    let args: RegisterPdaAttributeArgs = attr.parse_args()?;

    let instruction_path = args.instruction.ok_or_else(|| {
        syn::Error::new_spanned(attr, "#[register_pda] requires 'instruction' parameter")
    })?;

    let pda_field = args.pda_field.ok_or_else(|| {
        syn::Error::new_spanned(attr, "#[register_pda] requires 'pda_field' parameter")
    })?;

    let primary_key_field = args.primary_key.ok_or_else(|| {
        syn::Error::new_spanned(attr, "#[register_pda] requires 'primary_key' parameter")
    })?;

    let lookup_name = args
        .lookup_name
        .unwrap_or_else(|| "default_pda_lookup".to_string());

    Ok(Some(RegisterPdaAttribute {
        attr_span: attr.span(),
        instruction_path,
        pda_field,
        primary_key_field,
        lookup_name,
    }))
}

#[derive(Debug, Clone)]
pub struct ViewAttributeSpec {
    pub view: crate::ast::ViewDef,
    pub attr_span: Span,
    pub sort_key_span: Option<Span>,
}

/// Parse #[view(name = "latest", sort_by = "id.round_id", order = "desc")] attributes
pub fn parse_view_attribute_specs(attrs: &[Attribute]) -> syn::Result<Vec<ViewAttributeSpec>> {
    use crate::ast::{FieldPath, SortOrder, ViewDef, ViewOutput, ViewSource, ViewTransform};

    let mut views = Vec::new();

    for attr in attrs {
        if !attr.path().is_ident("view") {
            continue;
        }

        let mut name: Option<String> = None;
        let mut sort_by: Option<String> = None;
        let mut sort_key_span = None;
        let mut order = SortOrder::Desc;
        let mut take: Option<usize> = None;
        let output = ViewOutput::Collection;

        if let syn::Meta::List(meta_list) = &attr.meta {
            meta_list.parse_nested_meta(|meta| {
                if meta.path.is_ident("name") {
                    let value: syn::LitStr = meta.value()?.parse()?;
                    name = Some(value.value());
                } else if meta.path.is_ident("sort_by") {
                    let value: syn::LitStr = meta.value()?.parse()?;
                    sort_by = Some(value.value());
                    sort_key_span = Some(value.span());
                } else if meta.path.is_ident("order") {
                    let value: syn::LitStr = meta.value()?.parse()?;
                    order = match value.value().to_lowercase().as_str() {
                        "asc" => SortOrder::Asc,
                        _ => SortOrder::Desc,
                    };
                } else if meta.path.is_ident("take") {
                    let value: syn::LitInt = meta.value()?.parse()?;
                    take = Some(value.base10_parse::<usize>()?);
                }
                Ok(())
            })?;
        }

        if let (Some(view_name), Some(sort_field)) = (name, sort_by) {
            // Keep segments in snake_case to match AST field paths
            let segments: Vec<String> = sort_field.split('.').map(String::from).collect();
            let mut pipeline = vec![ViewTransform::Sort {
                key: FieldPath {
                    segments,
                    offsets: None,
                },
                order,
            }];

            // Only add Take transform if explicitly specified in the view definition.
            // Views return all matching entities by default - users can limit results
            // at query time using take() on the SDK side.
            if let Some(n) = take {
                pipeline.push(ViewTransform::Take { count: n });
            }

            views.push(ViewAttributeSpec {
                view: ViewDef {
                    id: view_name,
                    source: ViewSource::Entity {
                        name: String::new(),
                    },
                    pipeline,
                    output,
                },
                attr_span: attr.span(),
                sort_key_span,
            });
        }
    }

    Ok(views)
}

pub fn parse_view_attributes(attrs: &[Attribute]) -> syn::Result<Vec<crate::ast::ViewDef>> {
    Ok(parse_view_attribute_specs(attrs)?
        .into_iter()
        .map(|spec| spec.view)
        .collect())
}
