use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::ast::{
    ComputedExpr, EntitySection, FieldPath, Predicate, PredicateValue, ViewTransform,
};
use crate::diagnostic::{suggestion_or_available_suffix, ErrorCollector};
use crate::event_type_helpers::{find_idl_for_type, IdlLookup};
use crate::parse;
use crate::parse::idl as idl_parser;
use crate::parse::pda_validation::PdaValidationContext;
use crate::parse::pdas::PdasBlock;
use crate::utils::path_to_string;
use crate::validation::idl_refs::{
    resolve_instruction_lookup, resolve_instruction_lookup_from_path, validate_account_field,
    validate_instruction_field_spec,
};

use crate::diagnostic::idl_error_to_syn;
use crate::stream_spec::computed::{parse_computed_expression, qualify_field_refs};
use hyperstack_idl::error::IdlSearchError;
use hyperstack_idl::types::IdlSpec;

pub mod idl_refs;

pub struct ComputedFieldValidation {
    pub target_path: String,
    pub expression: proc_macro2::TokenStream,
    pub span: proc_macro2::Span,
}

pub struct ValidationInput<'a> {
    pub entity_name: &'a str,
    pub primary_keys: &'a [String],
    pub lookup_indexes: &'a [(String, Option<String>)],
    pub sources_by_type: &'a HashMap<String, Vec<parse::MapAttribute>>,
    pub events_by_instruction: &'a HashMap<String, Vec<(String, parse::EventAttribute, syn::Type)>>,
    pub derive_from_mappings: &'a HashMap<String, Vec<parse::DeriveFromAttribute>>,
    pub aggregate_conditions: &'a HashMap<String, crate::ast::ConditionExpr>,
    pub resolver_hooks: &'a [parse::ResolveKeyAttribute],
    pub computed_fields: &'a [ComputedFieldValidation],
    pub resolve_specs: &'a [parse::ResolveSpec],
    pub section_specs: &'a [EntitySection],
    pub view_specs: &'a [parse::ViewAttributeSpec],
    pub idls: IdlLookup<'a>,
}

pub struct KeyResolutionValidationInput<'a> {
    pub entity_name: &'a str,
    pub primary_keys: &'a [String],
    pub lookup_indexes: &'a [(String, Option<String>)],
    pub sources_by_type: &'a HashMap<String, Vec<parse::MapAttribute>>,
    pub events_by_instruction: &'a HashMap<String, Vec<(String, parse::EventAttribute, syn::Type)>>,
    pub derive_from_mappings: &'a HashMap<String, Vec<parse::DeriveFromAttribute>>,
    pub resolver_hooks: &'a [parse::ResolveKeyAttribute],
}

type GroupedEventMappings =
    HashMap<(String, Option<String>), Vec<(String, parse::EventAttribute, syn::Type)>>;

enum ResolvedMappingSource<'a> {
    Instruction {
        idl: &'a IdlSpec,
        instruction_name: String,
    },
    Account {
        idl: &'a IdlSpec,
        account_name: String,
    },
    Other,
}

pub fn validate_semantics(input: ValidationInput<'_>) -> syn::Result<()> {
    let known_fields = collect_known_field_paths(input.section_specs, input.computed_fields);
    let available_fields = sorted_field_paths(&known_fields);

    let mut errors = ErrorCollector::default();

    validate_key_resolution_paths(
        KeyResolutionValidationInput {
            entity_name: input.entity_name,
            primary_keys: input.primary_keys,
            lookup_indexes: input.lookup_indexes,
            sources_by_type: input.sources_by_type,
            events_by_instruction: input.events_by_instruction,
            derive_from_mappings: input.derive_from_mappings,
            resolver_hooks: input.resolver_hooks,
        },
        &mut errors,
    );

    validate_mapping_references(
        input.entity_name,
        input.sources_by_type,
        &known_fields,
        &available_fields,
        input.idls,
        &mut errors,
    );
    validate_event_references(
        input.entity_name,
        input.events_by_instruction,
        &known_fields,
        &available_fields,
        input.idls,
        &mut errors,
    );
    validate_derive_from_references(input.derive_from_mappings, input.idls, &mut errors);
    validate_aggregate_conditions(
        input.entity_name,
        input.aggregate_conditions,
        input.sources_by_type,
        &known_fields,
        input.idls,
        &mut errors,
    );
    validate_resolve_specs(
        input.entity_name,
        input.resolve_specs,
        &known_fields,
        &available_fields,
        &mut errors,
    );
    validate_views(
        input.entity_name,
        input.view_specs,
        &known_fields,
        &available_fields,
        &mut errors,
    );
    validate_computed_fields(
        input.entity_name,
        input.computed_fields,
        &known_fields,
        &available_fields,
        &mut errors,
    );

    errors.finish()
}

pub fn validate_key_resolution_paths(
    input: KeyResolutionValidationInput<'_>,
    errors: &mut ErrorCollector,
) {
    let primary_key_leafs = primary_key_leafs(input.primary_keys);
    let lookup_index_leafs = lookup_index_leafs(input.lookup_indexes);

    validate_source_handler_keys(
        input.entity_name,
        &primary_key_leafs,
        &lookup_index_leafs,
        input.lookup_indexes,
        input.sources_by_type,
        input.resolver_hooks,
        errors,
    );
    // Event-derived MapAttributes are validated here first (before being merged into
    // sources_by_type for codegen). They have is_event_source=true and are created
    // in handlers.rs, while other mappings in entity.rs have is_event_source=false.
    validate_event_handler_keys(
        input.entity_name,
        &primary_key_leafs,
        &lookup_index_leafs,
        input.events_by_instruction,
        errors,
    );
    validate_instruction_hook_keys(
        input.entity_name,
        &primary_key_leafs,
        &lookup_index_leafs,
        input.derive_from_mappings,
        errors,
    );
}

pub fn validate_pda_blocks(
    idls: &HashMap<String, idl_parser::IdlSpec>,
    blocks: &[PdasBlock],
) -> syn::Result<()> {
    let ctx = PdaValidationContext::new(idls);
    let mut errors = ErrorCollector::default();

    for block in blocks {
        if let Err(error) = ctx.validate(block) {
            errors.push(error);
        }
    }

    errors.finish()
}

fn collect_known_field_paths(
    section_specs: &[EntitySection],
    computed_fields: &[ComputedFieldValidation],
) -> HashSet<String> {
    let mut known = HashSet::new();

    for section in section_specs {
        for field in &section.fields {
            if section.name == "root" {
                known.insert(field.field_name.clone());
            } else {
                known.insert(format!("{}.{}", section.name, field.field_name));
            }
        }
    }

    for computed in computed_fields {
        known.insert(computed.target_path.clone());
    }

    known
}

fn sorted_field_paths(known_fields: &HashSet<String>) -> Vec<String> {
    let mut values: Vec<String> = known_fields.iter().cloned().collect();
    values.sort();
    values
}

fn entity_field_error(
    entity_name: &str,
    reference: &str,
    context: &str,
    span: proc_macro2::Span,
    available_fields: &[String],
) -> syn::Error {
    let mut message = format!(
        "unknown {} '{}' on entity '{}'",
        context, reference, entity_name
    );
    let suffix = suggestion_or_available_suffix(reference, available_fields, "Available fields");
    if !suffix.is_empty() {
        message.push_str(&suffix);
    }

    syn::Error::new(span, message)
}

type FieldSpecSortKey = (u8, String);
type RegisterFromSortKey = (String, FieldSpecSortKey, FieldSpecSortKey);

fn field_spec_sort_key(field_spec: Option<&parse::FieldSpec>) -> Option<FieldSpecSortKey> {
    field_spec.map(|field_spec| {
        let location = match field_spec.explicit_location {
            Some(parse::FieldLocation::Account) => 0,
            Some(parse::FieldLocation::InstructionArg) => 1,
            None => 2,
        };
        (location, field_spec.ident.to_string())
    })
}

fn field_specs_sort_key(field_specs: &[parse::FieldSpec]) -> Vec<FieldSpecSortKey> {
    field_specs
        .iter()
        .map(|field_spec| field_spec_sort_key(Some(field_spec)).expect("field spec key"))
        .collect()
}

fn path_sort_key(path: Option<&syn::Path>) -> Option<String> {
    path.map(path_to_string)
}

fn register_from_sort_key(register_from: &[parse::RegisterFromSpec]) -> Vec<RegisterFromSortKey> {
    register_from
        .iter()
        .map(|spec| {
            (
                path_to_string(&spec.instruction_path),
                field_spec_sort_key(Some(&spec.pda_field)).expect("pda field key"),
                field_spec_sort_key(Some(&spec.primary_key_field)).expect("primary key field key"),
            )
        })
        .collect()
}

fn condition_sort_key(condition: &Option<crate::ast::ConditionExpr>) -> Option<String> {
    condition.as_ref().map(|condition| format!("{condition:?}"))
}

fn resolver_transform_sort_key(
    transform: &Option<parse::ResolverTransformSpec>,
) -> Option<(String, String)> {
    transform
        .as_ref()
        .map(|transform| (transform.method.clone(), transform.args.to_string()))
}

fn event_transforms_sort_key<K, V>(transforms: &HashMap<K, V>) -> Vec<(String, String)>
where
    K: ToString,
    V: ToString,
{
    let mut entries = transforms
        .iter()
        .map(|(field, transform)| (field.to_string(), transform.to_string()))
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn stable_map_attribute_cmp(a: &parse::MapAttribute, b: &parse::MapAttribute) -> Ordering {
    a.target_field_name
        .cmp(&b.target_field_name)
        .then_with(|| a.source_field_name.cmp(&b.source_field_name))
        .then_with(|| path_to_string(&a.source_type_path).cmp(&path_to_string(&b.source_type_path)))
        .then_with(|| a.strategy.cmp(&b.strategy))
        .then_with(|| {
            field_spec_sort_key(a.join_on.as_ref()).cmp(&field_spec_sort_key(b.join_on.as_ref()))
        })
        .then_with(|| {
            field_spec_sort_key(a.lookup_by.as_ref())
                .cmp(&field_spec_sort_key(b.lookup_by.as_ref()))
        })
        .then_with(|| a.transform.cmp(&b.transform))
        .then_with(|| {
            resolver_transform_sort_key(&a.resolver_transform)
                .cmp(&resolver_transform_sort_key(&b.resolver_transform))
        })
        .then_with(|| {
            register_from_sort_key(&a.register_from).cmp(&register_from_sort_key(&b.register_from))
        })
        .then_with(|| a.temporal_field.cmp(&b.temporal_field))
        .then_with(|| condition_sort_key(&a.condition).cmp(&condition_sort_key(&b.condition)))
        .then_with(|| path_sort_key(a.when.as_ref()).cmp(&path_sort_key(b.when.as_ref())))
        .then_with(|| path_sort_key(a.stop.as_ref()).cmp(&path_sort_key(b.stop.as_ref())))
        .then_with(|| {
            field_spec_sort_key(a.stop_lookup_by.as_ref())
                .cmp(&field_spec_sort_key(b.stop_lookup_by.as_ref()))
        })
        .then_with(|| a.is_primary_key.cmp(&b.is_primary_key))
        .then_with(|| a.is_lookup_index.cmp(&b.is_lookup_index))
        .then_with(|| a.is_instruction.cmp(&b.is_instruction))
        .then_with(|| a.is_event_source.cmp(&b.is_event_source))
        .then_with(|| a.is_whole_source.cmp(&b.is_whole_source))
        .then_with(|| a.emit.cmp(&b.emit))
}

fn stable_event_mapping_cmp(
    a: &(String, parse::EventAttribute, syn::Type),
    b: &(String, parse::EventAttribute, syn::Type),
) -> Ordering {
    a.0.cmp(&b.0)
        .then_with(|| a.1.target_field_name.cmp(&b.1.target_field_name))
        .then_with(|| a.1.strategy.cmp(&b.1.strategy))
        .then_with(|| {
            path_sort_key(a.1.from_instruction.as_ref())
                .cmp(&path_sort_key(b.1.from_instruction.as_ref()))
        })
        .then_with(|| {
            path_sort_key(a.1.inferred_instruction.as_ref())
                .cmp(&path_sort_key(b.1.inferred_instruction.as_ref()))
        })
        .then_with(|| a.1.instruction.cmp(&b.1.instruction))
        .then_with(|| {
            field_specs_sort_key(&a.1.capture_fields)
                .cmp(&field_specs_sort_key(&b.1.capture_fields))
        })
        .then_with(|| a.1.capture_fields_legacy.cmp(&b.1.capture_fields_legacy))
        .then_with(|| {
            event_transforms_sort_key(&a.1.field_transforms)
                .cmp(&event_transforms_sort_key(&b.1.field_transforms))
        })
        .then_with(|| {
            event_transforms_sort_key(&a.1.field_transforms_legacy)
                .cmp(&event_transforms_sort_key(&b.1.field_transforms_legacy))
        })
        .then_with(|| {
            field_spec_sort_key(a.1.lookup_by.as_ref())
                .cmp(&field_spec_sort_key(b.1.lookup_by.as_ref()))
        })
        .then_with(|| {
            field_spec_sort_key(a.1.join_on.as_ref())
                .cmp(&field_spec_sort_key(b.1.join_on.as_ref()))
        })
}

fn primary_key_leafs(primary_keys: &[String]) -> HashSet<String> {
    primary_keys
        .iter()
        .map(|key| key.split('.').next_back().unwrap_or(key).to_string())
        .collect()
}

fn lookup_index_leafs(lookup_indexes: &[(String, Option<String>)]) -> HashSet<String> {
    let mut values = HashSet::new();

    for (field, _) in lookup_indexes {
        let leaf = field.split('.').next_back().unwrap_or(field).to_string();
        values.insert(leaf.clone());
        if let Some(stripped) = leaf.strip_suffix("_address") {
            values.insert(stripped.to_string());
        }
    }

    values
}

fn has_explicit_key_resolver(
    source_type: &str,
    resolver_hooks: &[parse::ResolveKeyAttribute],
) -> bool {
    resolver_hooks
        .iter()
        .any(|hook| crate::utils::path_to_string(&hook.account_path) == source_type)
}

fn source_field_can_resolve_key(
    field_name: &str,
    primary_key_leafs: &HashSet<String>,
    lookup_index_leafs: &HashSet<String>,
) -> bool {
    primary_key_leafs.contains(field_name) || lookup_index_leafs.contains(field_name)
}

fn has_account_address_lookup_path(
    mappings: &[parse::MapAttribute],
    lookup_indexes: &[(String, Option<String>)],
) -> bool {
    mappings.iter().any(|mapping| {
        mapping.source_field_name == "__account_address"
            && lookup_indexes
                .iter()
                .any(|(field_name, _)| field_name == &mapping.target_field_name)
    })
}

fn key_resolution_error(
    span: proc_macro2::Span,
    source_kind: &str,
    source_name: &str,
    entity_name: &str,
    detail: &str,
) -> syn::Error {
    syn::Error::new(
        span,
        format!(
            "{} '{}' cannot resolve the primary key for entity '{}'. {}",
            source_kind, source_name, entity_name, detail
        ),
    )
}

fn resolve_mapping_source_once<'a>(
    source_type: &str,
    mappings: &[parse::MapAttribute],
    idls: IdlLookup<'a>,
) -> Result<ResolvedMappingSource<'a>, IdlSearchError> {
    let is_instruction = mappings.iter().any(|mapping| mapping.is_instruction);
    let is_account_source = mappings.iter().any(|mapping| mapping.is_account_source);

    if is_instruction && is_account_source {
        #[cfg(debug_assertions)]
        eprintln!(
            "[hyperstack] warning: source type '{}' matches both instruction and account \
             classification — skipping IDL field validation. Ensure the source type path \
             contains exactly one of `::instructions::` or `::accounts::`.",
            source_type
        );
        return Ok(ResolvedMappingSource::Other);
    }

    if is_instruction {
        let path =
            syn::parse_str::<syn::Path>(source_type).map_err(|_| IdlSearchError::InvalidPath {
                path: source_type.to_string(),
            })?;
        let (idl, instruction_name) = resolve_instruction_lookup_from_path(&path, idls)?;
        Ok(ResolvedMappingSource::Instruction {
            idl,
            instruction_name,
        })
    } else if is_account_source {
        let path =
            syn::parse_str::<syn::Path>(source_type).map_err(|_| IdlSearchError::InvalidPath {
                path: source_type.to_string(),
            })?;
        let idl =
            find_idl_for_type(source_type, idls).ok_or_else(|| IdlSearchError::InvalidPath {
                path: source_type.to_string(),
            })?;
        let account_name = path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .ok_or_else(|| IdlSearchError::InvalidPath {
                path: source_type.to_string(),
            })?;
        Ok(ResolvedMappingSource::Account { idl, account_name })
    } else {
        Ok(ResolvedMappingSource::Other)
    }
}

fn validate_source_handler_keys(
    entity_name: &str,
    primary_key_leafs: &HashSet<String>,
    lookup_index_leafs: &HashSet<String>,
    lookup_indexes: &[(String, Option<String>)],
    sources_by_type: &HashMap<String, Vec<parse::MapAttribute>>,
    resolver_hooks: &[parse::ResolveKeyAttribute],
    errors: &mut ErrorCollector,
) {
    let mut grouped: HashMap<(String, Option<String>), Vec<parse::MapAttribute>> = HashMap::new();
    for (source_type, mappings) in sources_by_type {
        for mapping in mappings {
            grouped
                .entry((
                    source_type.clone(),
                    mapping
                        .join_on
                        .as_ref()
                        .map(|field_spec| field_spec.ident.to_string()),
                ))
                .or_default()
                .push(mapping.clone());
        }
    }

    let mut grouped_entries: Vec<_> = grouped.into_iter().collect();
    grouped_entries.sort_by(|(a_key, _), (b_key, _)| a_key.cmp(b_key));

    for ((source_type, join_key), mut mappings) in grouped_entries {
        mappings.sort_by(stable_map_attribute_cmp);
        let Some(first_mapping) = mappings.first() else {
            continue;
        };

        if mappings.iter().any(|mapping| mapping.is_primary_key) {
            continue;
        }

        let is_instruction = mappings.iter().any(|mapping| mapping.is_instruction);

        // Event-derived MapAttribute values are produced in handlers.rs via
        // convert_event_to_map_attributes(...), which sets is_event_source = true.
        // Those groups are validated in validate_event_handler_keys before they
        // are merged into sources_by_type for codegen.
        if mappings.iter().all(|mapping| mapping.is_event_source) {
            continue;
        }

        if !is_instruction && has_explicit_key_resolver(&source_type, resolver_hooks) {
            continue;
        }

        if mappings.iter().any(|mapping| {
            source_field_can_resolve_key(
                &mapping.source_field_name,
                primary_key_leafs,
                lookup_index_leafs,
            )
        }) {
            continue;
        }

        if !is_instruction && has_account_address_lookup_path(&mappings, lookup_indexes) {
            continue;
        }

        if let Some(join_field) = join_key.as_ref() {
            if source_field_can_resolve_key(join_field, primary_key_leafs, lookup_index_leafs) {
                continue;
            }
        }

        if let Some(lookup_by) = mappings
            .iter()
            .find_map(|mapping| mapping.lookup_by.as_ref())
        {
            let field_name = lookup_by.ident.to_string();
            if source_field_can_resolve_key(&field_name, primary_key_leafs, lookup_index_leafs) {
                continue;
            }

            errors.push(key_resolution_error(
                lookup_by.ident.span(),
                if is_instruction { "instruction source" } else { "account source" },
                &source_type,
                entity_name,
                &format!(
                    "The `lookup_by` field '{}' is neither a primary-key field nor a lookup-index-backed field.",
                    field_name
                ),
            ));
            continue;
        }

        if let Some(join_field) = join_key {
            errors.push(key_resolution_error(
                first_mapping.attr_span,
                if is_instruction { "instruction source" } else { "account source" },
                &source_type,
                entity_name,
                &format!(
                    "The `join_on` field '{}' does not provide a provable path back to the entity primary key. Use `primary_key`, `lookup_by`, a lookup-index-backed source field, or an explicit `#[resolve_key(...)]` hook.",
                    join_field
                ),
            ));
            continue;
        }

        errors.push(key_resolution_error(
            first_mapping.attr_span,
            if is_instruction { "instruction source" } else { "account source" },
            &source_type,
            entity_name,
            if is_instruction {
                "Add a `primary_key` mapping or `lookup_by = ...` that points to the primary key or to a lookup index field."
            } else {
                "Add a `primary_key` mapping, a lookup-index-backed field (commonly via `__account_address`), or an explicit `#[resolve_key(...)]` hook."
            },
        ));
    }
}

fn validate_event_handler_keys(
    entity_name: &str,
    primary_key_leafs: &HashSet<String>,
    lookup_index_leafs: &HashSet<String>,
    events_by_instruction: &HashMap<String, Vec<(String, parse::EventAttribute, syn::Type)>>,
    errors: &mut ErrorCollector,
) {
    let mut grouped: GroupedEventMappings = HashMap::new();
    for (instruction, event_mappings) in events_by_instruction {
        for event_mapping in event_mappings {
            let join_key = event_mapping
                .1
                .join_on
                .as_ref()
                .map(|field_spec| field_spec.ident.to_string());
            grouped
                .entry((instruction.clone(), join_key))
                .or_default()
                .push(event_mapping.clone());
        }
    }

    let mut grouped_entries: Vec<_> = grouped.into_iter().collect();
    grouped_entries.sort_by(|(a_key, _), (b_key, _)| a_key.cmp(b_key));

    for ((instruction, join_key), mut mappings) in grouped_entries {
        mappings.sort_by(stable_event_mapping_cmp);
        let Some((_, first_attr, _)) = mappings.first() else {
            continue;
        };

        let captured_field_resolves = mappings.iter().any(|(_, attr, _)| {
            attr.capture_fields.iter().any(|field_spec| {
                source_field_can_resolve_key(
                    &field_spec.ident.to_string(),
                    primary_key_leafs,
                    lookup_index_leafs,
                )
            }) || attr.capture_fields_legacy.iter().any(|field_name| {
                source_field_can_resolve_key(field_name, primary_key_leafs, lookup_index_leafs)
            })
        });
        if captured_field_resolves {
            continue;
        }

        let lookup_by = mappings
            .iter()
            .filter_map(|(_, attr, _)| attr.lookup_by.as_ref())
            .find(|lookup_by| {
                source_field_can_resolve_key(
                    &lookup_by.ident.to_string(),
                    primary_key_leafs,
                    lookup_index_leafs,
                )
            });
        if lookup_by.is_some() {
            continue;
        }

        if let Some(join_field) = join_key.as_ref() {
            if source_field_can_resolve_key(join_field, primary_key_leafs, lookup_index_leafs) {
                continue;
            }
        }

        if let Some(lookup_by) = mappings
            .iter()
            .filter_map(|(_, attr, _)| attr.lookup_by.as_ref())
            .next()
        {
            let field_name = lookup_by.ident.to_string();
            errors.push(key_resolution_error(
                lookup_by.ident.span(),
                "event source",
                &instruction,
                entity_name,
                &format!(
                    "The `lookup_by` field '{}' is neither a primary-key field nor a lookup-index-backed field.",
                    field_name
                ),
            ));
            continue;
        }

        if let Some(join_field) = join_key {
            let join_on = mappings
                .iter()
                .find_map(|(_, attr, _)| attr.join_on.as_ref());
            debug_assert!(
                join_on.is_some(),
                "group key has a join field but no mapping carries join_on"
            );
            let Some(join_on) = join_on else {
                continue;
            };
            let field_name = join_field;

            errors.push(key_resolution_error(
                join_on.ident.span(),
                "event source",
                &instruction,
                entity_name,
                &format!(
                    "The `join_on` field '{}' is neither a primary-key field nor a lookup-index-backed field.",
                    field_name
                ),
            ));
            continue;
        }

        errors.push(key_resolution_error(
            first_attr.attr_span,
            "event source",
            &instruction,
            entity_name,
            "Add `lookup_by = ...`, `join_on = ...`, or include the primary-key field in `fields = [...]` that points to the primary key or to a lookup index field.",
        ));
    }
}

fn stable_derive_from_cmp(
    a: &parse::DeriveFromAttribute,
    b: &parse::DeriveFromAttribute,
) -> Ordering {
    a.target_field_name
        .cmp(&b.target_field_name)
        .then_with(|| a.field.ident.to_string().cmp(&b.field.ident.to_string()))
        .then_with(|| a.strategy.cmp(&b.strategy))
        .then_with(|| {
            field_spec_sort_key(a.lookup_by.as_ref())
                .cmp(&field_spec_sort_key(b.lookup_by.as_ref()))
        })
}

fn validate_instruction_hook_keys(
    entity_name: &str,
    primary_key_leafs: &HashSet<String>,
    lookup_index_leafs: &HashSet<String>,
    derive_from_mappings: &HashMap<String, Vec<parse::DeriveFromAttribute>>,
    errors: &mut ErrorCollector,
) {
    let mut instruction_types: Vec<&String> = derive_from_mappings.keys().collect();
    instruction_types.sort();

    for instruction_type in instruction_types {
        let mut derive_attrs = derive_from_mappings[instruction_type].clone();
        derive_attrs.sort_by(stable_derive_from_cmp);

        let group_resolved = derive_attrs.iter().any(|derive_attr| {
            let field_name = derive_attr
                .lookup_by
                .as_ref()
                .map(|lookup_by| lookup_by.ident.to_string())
                .unwrap_or_else(|| derive_attr.field.ident.to_string());

            source_field_can_resolve_key(&field_name, primary_key_leafs, lookup_index_leafs)
        });

        if group_resolved {
            continue;
        }

        let Some(first_attr) = derive_attrs.first() else {
            continue;
        };

        // Emit a single group-level error: if any attribute has a bad lookup_by,
        // report on that field; otherwise report on the first attribute's span.
        // This mirrors the one-error-per-group pattern in validate_source_handler_keys.
        if let Some(lookup_by) = derive_attrs
            .iter()
            .find_map(|derive_attr| derive_attr.lookup_by.as_ref())
        {
            errors.push(key_resolution_error(
                lookup_by.ident.span(),
                "instruction hook",
                instruction_type,
                entity_name,
                &format!(
                    "The `lookup_by` field '{}' is neither a primary-key field nor a lookup-index-backed field.",
                    lookup_by.ident
                ),
            ));
        } else {
            errors.push(key_resolution_error(
                first_attr.attr_span,
                "instruction hook",
                instruction_type,
                entity_name,
                "Add `lookup_by = ...` that points to the primary key or to a lookup index field.",
            ));
        }
    }
}

fn validate_mapping_references(
    entity_name: &str,
    sources_by_type: &HashMap<String, Vec<parse::MapAttribute>>,
    known_fields: &HashSet<String>,
    available_fields: &[String],
    idls: IdlLookup,
    errors: &mut ErrorCollector,
) {
    let mut source_types: Vec<&String> = sources_by_type.keys().collect();
    source_types.sort();

    for source_type in source_types {
        let mut mappings = sources_by_type[source_type].clone();
        mappings.sort_by(stable_map_attribute_cmp);

        let Some(first_mapping) = mappings.first() else {
            continue;
        };

        let resolved_source = match resolve_mapping_source_once(source_type, &mappings, idls) {
            Ok(resolved_source) => resolved_source,
            Err(error) => {
                errors.push(idl_error_to_syn(first_mapping.source_type_span, error));
                continue;
            }
        };

        let mut reported_join_ons: HashSet<String> = HashSet::new();
        let mut reported_condition_leaves: HashSet<String> = HashSet::new();

        for mapping in &mappings {
            match &resolved_source {
                ResolvedMappingSource::Instruction {
                    idl,
                    instruction_name,
                } => {
                    if !mapping.source_field_name.is_empty()
                        && !mapping.source_field_name.starts_with("__")
                    {
                        let temp_field = parse::FieldSpec {
                            ident: syn::Ident::new(
                                &mapping.source_field_name,
                                mapping.source_field_span,
                            ),
                            explicit_location: None,
                        };

                        if let Err(error) =
                            validate_instruction_field_spec(idl, instruction_name, &temp_field)
                        {
                            errors.push(idl_error_to_syn(mapping.source_field_span, error));
                        }
                    }
                }
                ResolvedMappingSource::Account { idl, account_name } => {
                    if !mapping.source_field_name.is_empty()
                        && !mapping.source_field_name.starts_with("__")
                    {
                        if let Err(error) =
                            validate_account_field(idl, account_name, &mapping.source_field_name)
                        {
                            errors.push(idl_error_to_syn(mapping.source_field_span, error));
                        }
                    }
                }
                ResolvedMappingSource::Other => {}
            }

            if let Some(join_on) = &mapping.join_on {
                let reference = join_on.ident.to_string();
                if !known_fields.contains(&reference) && reported_join_ons.insert(reference.clone())
                {
                    errors.push(entity_field_error(
                        entity_name,
                        &reference,
                        "join_on field",
                        join_on.ident.span(),
                        available_fields,
                    ));
                }
            }

            if let Some(lookup_by) = &mapping.lookup_by {
                match &resolved_source {
                    ResolvedMappingSource::Instruction {
                        idl,
                        instruction_name,
                    } => {
                        if let Err(error) =
                            validate_instruction_field_spec(idl, instruction_name, lookup_by)
                        {
                            errors.push(idl_error_to_syn(lookup_by.ident.span(), error));
                        }
                    }
                    ResolvedMappingSource::Account { idl, account_name } => {
                        if let Err(error) =
                            validate_account_field(idl, account_name, &lookup_by.ident.to_string())
                        {
                            errors.push(idl_error_to_syn(lookup_by.ident.span(), error));
                        }
                    }
                    ResolvedMappingSource::Other => {}
                }
            }

            if let Some(stop_lookup_by) = &mapping.stop_lookup_by {
                match &resolved_source {
                    ResolvedMappingSource::Instruction {
                        idl,
                        instruction_name,
                    } => {
                        if let Err(error) =
                            validate_instruction_field_spec(idl, instruction_name, stop_lookup_by)
                        {
                            errors.push(idl_error_to_syn(stop_lookup_by.ident.span(), error));
                        }
                    }
                    ResolvedMappingSource::Account { idl, account_name } => {
                        if let Err(error) = validate_account_field(
                            idl,
                            account_name,
                            &stop_lookup_by.ident.to_string(),
                        ) {
                            errors.push(idl_error_to_syn(stop_lookup_by.ident.span(), error));
                        }
                    }
                    ResolvedMappingSource::Other => {}
                }
            }

            if let Some(condition) = &mapping.condition {
                if let Some(parsed) = &condition.parsed {
                    let field_leaves = collect_condition_field_leaves(parsed);
                    match &resolved_source {
                        ResolvedMappingSource::Instruction {
                            idl,
                            instruction_name,
                        } => {
                            for leaf in &field_leaves {
                                if let Some(temp_field) =
                                    try_field_spec_from_leaf(leaf, mapping.attr_span)
                                {
                                    if let Err(error) = validate_instruction_field_spec(
                                        idl,
                                        instruction_name,
                                        &temp_field,
                                    ) {
                                        let key = format!("{leaf}@{instruction_name}");
                                        if reported_condition_leaves.insert(key) {
                                            errors.push(idl_error_to_syn(mapping.attr_span, error));
                                        }
                                    }
                                }
                            }
                        }
                        ResolvedMappingSource::Account { idl, account_name } => {
                            for leaf in &field_leaves {
                                if let Err(error) = validate_account_field(idl, account_name, leaf)
                                {
                                    let key = format!("{leaf}@{account_name}");
                                    if reported_condition_leaves.insert(key) {
                                        errors.push(idl_error_to_syn(mapping.attr_span, error));
                                    }
                                }
                            }
                        }
                        ResolvedMappingSource::Other => {}
                    }
                }
            }
        }
    }
}

fn validate_aggregate_conditions(
    _entity_name: &str,
    aggregate_conditions: &HashMap<String, crate::ast::ConditionExpr>,
    sources_by_type: &HashMap<String, Vec<parse::MapAttribute>>,
    known_fields: &HashSet<String>,
    idls: IdlLookup,
    errors: &mut ErrorCollector,
) {
    let mut field_paths: Vec<(&String, Vec<String>)> = Vec::new();

    for (target_field, condition) in aggregate_conditions {
        if let Some(parsed) = &condition.parsed {
            let leaves = collect_condition_field_leaves(parsed);
            if !leaves.is_empty() {
                field_paths.push((target_field, leaves));
            }
        }
    }
    field_paths.sort_by_key(|(target, _)| *target);

    for (target_field, leaves) in &field_paths {
        // Collect all instruction-source mappings for this aggregate target,
        // sorted by source type for deterministic validation order.
        let mut instruction_mappings: Vec<&parse::MapAttribute> = sources_by_type
            .values()
            .flatten()
            .filter(|m| m.target_field_name == **target_field && m.is_instruction)
            .collect();
        instruction_mappings.sort_by(|a, b| stable_map_attribute_cmp(a, b));

        let mut reported: HashSet<(String, String)> = HashSet::new();
        for mapping in &instruction_mappings {
            let source_type = mapping.source_type_string();
            if let Ok(ResolvedMappingSource::Instruction {
                idl,
                instruction_name,
            }) = resolve_mapping_source_once(&source_type, std::slice::from_ref(mapping), idls)
            {
                for leaf in leaves {
                    if let Some(temp_field) = try_field_spec_from_leaf(leaf, mapping.attr_span) {
                        if let Err(error) =
                            validate_instruction_field_spec(idl, &instruction_name, &temp_field)
                        {
                            if reported.insert((leaf.clone(), instruction_name.clone())) {
                                errors.push(idl_error_to_syn(mapping.attr_span, error));
                            }
                        }
                    }
                }
            }
        }

        // Also validate against account-source mappings for the same target
        let mut account_mappings: Vec<&parse::MapAttribute> = sources_by_type
            .values()
            .flatten()
            .filter(|m| m.target_field_name == **target_field && m.is_account_source)
            .collect();
        account_mappings.sort_by(|a, b| stable_map_attribute_cmp(a, b));

        let mut reported_account: HashSet<(String, String)> = HashSet::new();
        for mapping in &account_mappings {
            let source_type = mapping.source_type_string();
            if let Ok(ResolvedMappingSource::Account { idl, account_name }) =
                resolve_mapping_source_once(&source_type, std::slice::from_ref(mapping), idls)
            {
                for leaf in leaves {
                    if let Err(error) = validate_account_field(idl, &account_name, leaf) {
                        if reported_account.insert((leaf.clone(), account_name.clone())) {
                            errors.push(idl_error_to_syn(mapping.attr_span, error));
                        }
                    }
                }
            }
        }

        // Fallback: validate against known entity fields when no IDL sources exist
        // (e.g., for event-backed aggregates)
        if instruction_mappings.is_empty() && account_mappings.is_empty() {
            let available = sorted_field_paths(known_fields);
            if let Some(cond_expr) = aggregate_conditions.get(*target_field) {
                for leaf in leaves {
                    if !known_fields.contains(leaf) {
                        errors.push(entity_field_error(
                            _entity_name,
                            leaf,
                            "condition field",
                            cond_expr.span,
                            &available,
                        ));
                    }
                }
            }
        }
    }
}

/// Try to construct a `FieldSpec` from a condition leaf string. Returns `None`
/// if the leaf is not a valid Rust identifier (e.g. starts with a digit),
/// preventing a `syn::Ident::new` panic inside the proc-macro process.
fn try_field_spec_from_leaf(leaf: &str, span: proc_macro2::Span) -> Option<parse::FieldSpec> {
    syn::parse_str::<syn::Ident>(leaf).ok().map(|mut ident| {
        ident.set_span(span);
        parse::FieldSpec {
            ident,
            explicit_location: None,
        }
    })
}

/// Recursively collect the leaf (last) segment of every field path referenced
/// in a parsed condition tree. Only leaf segments are collected because IDL
/// instruction fields are flat identifiers — dotted paths like `data.amount`
/// use the final segment `amount` for validation.
fn collect_condition_field_leaves(condition: &crate::ast::ParsedCondition) -> Vec<String> {
    let mut leaves = Vec::new();
    collect_condition_field_leaves_recursive(condition, &mut leaves);
    leaves.sort();
    leaves.dedup();
    leaves
}

fn collect_condition_field_leaves_recursive(
    condition: &crate::ast::ParsedCondition,
    leaves: &mut Vec<String>,
) {
    match condition {
        crate::ast::ParsedCondition::Comparison { field, .. } => {
            if let Some(leaf) = field.segments.last() {
                if !leaf.is_empty() {
                    leaves.push(leaf.clone());
                }
            }
        }
        crate::ast::ParsedCondition::Logical { conditions, .. } => {
            for sub in conditions {
                collect_condition_field_leaves_recursive(sub, leaves);
            }
        }
    }
}

fn validate_event_references(
    entity_name: &str,
    events_by_instruction: &HashMap<String, Vec<(String, parse::EventAttribute, syn::Type)>>,
    known_fields: &HashSet<String>,
    available_fields: &[String],
    idls: IdlLookup,
    errors: &mut ErrorCollector,
) {
    let mut instruction_keys: Vec<&String> = events_by_instruction.keys().collect();
    instruction_keys.sort();

    for instruction_key in instruction_keys {
        let mut event_mappings = events_by_instruction[instruction_key].clone();
        event_mappings.sort_by(stable_event_mapping_cmp);

        let mut reported_join_ons: HashSet<String> = HashSet::new();
        for (_target_field, event_attr, _field_type) in &event_mappings {
            if let Some(join_on) = &event_attr.join_on {
                let reference = join_on.ident.to_string();
                if !known_fields.contains(&reference) && reported_join_ons.insert(reference.clone())
                {
                    errors.push(entity_field_error(
                        entity_name,
                        &reference,
                        "join_on field",
                        join_on.ident.span(),
                        available_fields,
                    ));
                }
            }
        }

        let Some((_, first_attr, _)) = event_mappings.first() else {
            continue;
        };

        let (idl, instruction_name) =
            match resolve_instruction_lookup(first_attr, instruction_key, idls) {
                Ok(value) => value,
                Err(error) => {
                    errors.push(idl_error_to_syn(
                        first_attr.instruction_span.unwrap_or(first_attr.attr_span),
                        error,
                    ));
                    continue;
                }
            };

        for (_target_field, event_attr, _field_type) in &event_mappings {
            let (event_idl, event_instruction_name) = if event_attr.from_instruction.is_some()
                || event_attr.inferred_instruction.is_some()
                || !event_attr.instruction.is_empty()
            {
                match resolve_instruction_lookup(event_attr, instruction_key, idls) {
                    Ok(value) => value,
                    Err(error) => {
                        errors.push(idl_error_to_syn(
                            event_attr.instruction_span.unwrap_or(event_attr.attr_span),
                            error,
                        ));
                        continue;
                    }
                }
            } else {
                (idl, instruction_name.clone())
            };

            for field_spec in &event_attr.capture_fields {
                if let Err(error) =
                    validate_instruction_field_spec(event_idl, &event_instruction_name, field_spec)
                {
                    errors.push(idl_error_to_syn(field_spec.ident.span(), error));
                }
            }

            for field_name in &event_attr.capture_fields_legacy {
                if field_name.starts_with("__") {
                    continue;
                }
                if let Some(temp_field) = try_field_spec_from_leaf(field_name, event_attr.attr_span)
                {
                    if let Err(error) = validate_instruction_field_spec(
                        event_idl,
                        &event_instruction_name,
                        &temp_field,
                    ) {
                        errors.push(idl_error_to_syn(event_attr.attr_span, error));
                    }
                }
            }

            if let Some(field_spec) = &event_attr.lookup_by {
                if let Err(error) =
                    validate_instruction_field_spec(event_idl, &event_instruction_name, field_spec)
                {
                    errors.push(idl_error_to_syn(field_spec.ident.span(), error));
                }
            }
        }
    }
}

fn validate_derive_from_references(
    derive_from_mappings: &HashMap<String, Vec<parse::DeriveFromAttribute>>,
    idls: IdlLookup,
    errors: &mut ErrorCollector,
) {
    let mut instruction_types: Vec<&String> = derive_from_mappings.keys().collect();
    instruction_types.sort();

    for instruction_type in instruction_types {
        let mut derive_attrs = derive_from_mappings[instruction_type].clone();
        derive_attrs.sort_by(stable_derive_from_cmp);
        let path = match syn::parse_str::<syn::Path>(instruction_type) {
            Ok(path) => path,
            Err(_) => {
                if let Some(first_attr) = derive_attrs.first() {
                    errors.push(syn::Error::new(
                        first_attr.attr_span,
                        format!(
                            "internal error: could not re-parse instruction path '{}'",
                            instruction_type
                        ),
                    ));
                }
                continue;
            }
        };

        let instruction_lookup = idl_refs::resolve_instruction_lookup_from_path(&path, idls);
        let (idl, instruction_name) = match instruction_lookup {
            Ok(value) => value,
            Err(error) => {
                if let Some(first_attr) = derive_attrs.first() {
                    errors.push(idl_error_to_syn(first_attr.attr_span, error));
                }
                continue;
            }
        };

        for derive_attr in derive_attrs {
            if !derive_attr.field.ident.to_string().starts_with("__") {
                if let Err(error) =
                    validate_instruction_field_spec(idl, &instruction_name, &derive_attr.field)
                {
                    errors.push(idl_error_to_syn(derive_attr.field.ident.span(), error));
                }
            }

            if let Some(lookup_by) = &derive_attr.lookup_by {
                if let Err(error) =
                    validate_instruction_field_spec(idl, &instruction_name, lookup_by)
                {
                    errors.push(idl_error_to_syn(lookup_by.ident.span(), error));
                }
            }
        }
    }
}

fn validate_resolve_specs(
    entity_name: &str,
    resolve_specs: &[parse::ResolveSpec],
    known_fields: &HashSet<String>,
    available_fields: &[String],
    errors: &mut ErrorCollector,
) {
    for spec in resolve_specs {
        if let Some(from) = &spec.from {
            if !known_fields.contains(from) {
                errors.push(entity_field_error(
                    entity_name,
                    from,
                    "resolver input field",
                    spec.from_span.unwrap_or(spec.attr_span),
                    available_fields,
                ));
            }
        }

        if let Some(schedule_at) = &spec.schedule_at {
            if !known_fields.contains(&schedule_at.raw) {
                errors.push(entity_field_error(
                    entity_name,
                    &schedule_at.raw,
                    "resolver schedule_at field",
                    schedule_at.span,
                    available_fields,
                ));
            }
        }

        if let Some(condition) = &spec.condition {
            let field_path = &condition.parsed.field_path;
            if !field_path.is_empty() && !known_fields.contains(field_path) {
                errors.push(entity_field_error(
                    entity_name,
                    field_path,
                    "resolver condition field",
                    condition.span,
                    available_fields,
                ));
            }
        }
    }
}

fn validate_views(
    entity_name: &str,
    view_specs: &[parse::ViewAttributeSpec],
    known_fields: &HashSet<String>,
    available_fields: &[String],
    errors: &mut ErrorCollector,
) {
    let mut seen_ids = HashSet::new();

    for view_spec in view_specs {
        if !seen_ids.insert(view_spec.view.id.clone()) {
            errors.push(syn::Error::new(
                view_spec.attr_span,
                format!(
                    "duplicate view id '{}' on entity '{}'",
                    view_spec.view.id, entity_name
                ),
            ));
            continue;
        }

        for transform in &view_spec.view.pipeline {
            let maybe_field = match transform {
                ViewTransform::Sort { key, .. }
                | ViewTransform::MaxBy { key, .. }
                | ViewTransform::MinBy { key, .. } => Some(key),
                _ => None,
            };

            if let Some(field) = maybe_field {
                let raw = field_path_to_string(field);
                if !known_fields.contains(&raw) {
                    let span = match transform {
                        ViewTransform::Sort { key_span, .. } => {
                            key_span.unwrap_or(view_spec.attr_span)
                        }
                        ViewTransform::MaxBy { key_span, .. } => {
                            key_span.unwrap_or(view_spec.attr_span)
                        }
                        ViewTransform::MinBy { key_span, .. } => {
                            key_span.unwrap_or(view_spec.attr_span)
                        }
                        _ => view_spec.attr_span,
                    };
                    errors.push(entity_field_error(
                        entity_name,
                        &raw,
                        "view field",
                        span,
                        available_fields,
                    ));
                }
            }

            if let ViewTransform::Filter { predicate } = transform {
                let mut filter_refs: Vec<String> = collect_predicate_field_refs(predicate)
                    .into_iter()
                    .collect();
                filter_refs.sort();
                for field in filter_refs {
                    if !known_fields.contains(&field) {
                        errors.push(entity_field_error(
                            entity_name,
                            &field,
                            "view filter field",
                            view_spec.attr_span,
                            available_fields,
                        ));
                    }
                }
            }
        }
    }
}

fn validate_computed_fields(
    entity_name: &str,
    computed_fields: &[ComputedFieldValidation],
    known_fields: &HashSet<String>,
    available_fields: &[String],
    errors: &mut ErrorCollector,
) {
    let computed_targets: HashSet<String> = computed_fields
        .iter()
        .map(|field| field.target_path.clone())
        .collect();
    let mut dependencies: HashMap<String, HashSet<String>> = HashMap::new();
    let mut spans = HashMap::new();

    for computed in computed_fields {
        spans.insert(computed.target_path.clone(), computed.span);

        let parsed = parse_computed_expression(&computed.expression);
        let section = computed.target_path.split('.').next().unwrap_or("");
        let parsed = if computed.target_path.contains('.') {
            qualify_field_refs(parsed, section)
        } else {
            parsed
        };

        let refs = collect_field_refs(&parsed);
        let mut sorted_refs: Vec<&String> = refs.iter().collect();
        sorted_refs.sort();
        for reference in sorted_refs {
            if !known_fields.contains(reference) {
                errors.push(entity_field_error(
                    entity_name,
                    reference,
                    "computed field reference",
                    computed.span,
                    available_fields,
                ));
            }
        }

        dependencies.insert(
            computed.target_path.clone(),
            refs.into_iter()
                .filter(|reference| computed_targets.contains(reference))
                .collect(),
        );
    }

    for cycle in detect_cycles(&dependencies) {
        if let Some(first) = cycle.first() {
            errors.push(syn::Error::new(
                spans
                    .get(first)
                    .copied()
                    .unwrap_or(proc_macro2::Span::call_site()),
                format!(
                    "computed fields contain a dependency cycle: {}",
                    cycle.join(" -> ")
                ),
            ));
        }
    }
}

fn field_path_to_string(path: &FieldPath) -> String {
    path.segments.join(".")
}

fn collect_predicate_field_refs(predicate: &Predicate) -> HashSet<String> {
    let mut refs = HashSet::new();
    collect_predicate_field_refs_recursive(predicate, &mut refs);
    refs
}

fn collect_predicate_field_refs_recursive(predicate: &Predicate, refs: &mut HashSet<String>) {
    match predicate {
        Predicate::Compare { field, value, .. } => {
            refs.insert(field_path_to_string(field));
            if let PredicateValue::Field(field) = value {
                refs.insert(field_path_to_string(field));
            }
        }
        Predicate::And(predicates) | Predicate::Or(predicates) => {
            for predicate in predicates {
                collect_predicate_field_refs_recursive(predicate, refs);
            }
        }
        Predicate::Not(predicate) => collect_predicate_field_refs_recursive(predicate, refs),
        Predicate::Exists { field } => {
            refs.insert(field_path_to_string(field));
        }
    }
}

fn collect_field_refs(expr: &ComputedExpr) -> HashSet<String> {
    let mut refs = HashSet::new();
    collect_field_refs_recursive(expr, &mut refs);
    refs
}

fn collect_field_refs_recursive(expr: &ComputedExpr, refs: &mut HashSet<String>) {
    match expr {
        ComputedExpr::FieldRef { path } => {
            refs.insert(path.clone());
        }
        ComputedExpr::Binary { left, right, .. } => {
            collect_field_refs_recursive(left, refs);
            collect_field_refs_recursive(right, refs);
        }
        ComputedExpr::Unary { expr, .. }
        | ComputedExpr::Paren { expr }
        | ComputedExpr::Cast { expr, .. }
        | ComputedExpr::UnwrapOr { expr, .. }
        | ComputedExpr::Slice { expr, .. }
        | ComputedExpr::Index { expr, .. }
        | ComputedExpr::Keccak256 { expr }
        | ComputedExpr::JsonToBytes { expr }
        | ComputedExpr::U64FromLeBytes { bytes: expr }
        | ComputedExpr::U64FromBeBytes { bytes: expr } => {
            collect_field_refs_recursive(expr, refs);
        }
        ComputedExpr::MethodCall { expr, args, .. } => {
            collect_field_refs_recursive(expr, refs);
            for arg in args {
                collect_field_refs_recursive(arg, refs);
            }
        }
        ComputedExpr::ResolverComputed { args, .. } => {
            for arg in args {
                collect_field_refs_recursive(arg, refs);
            }
        }
        ComputedExpr::Let { value, body, .. } => {
            collect_field_refs_recursive(value, refs);
            collect_field_refs_recursive(body, refs);
        }
        ComputedExpr::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_field_refs_recursive(condition, refs);
            collect_field_refs_recursive(then_branch, refs);
            collect_field_refs_recursive(else_branch, refs);
        }
        ComputedExpr::Some { value } => collect_field_refs_recursive(value, refs),
        ComputedExpr::Closure { body, .. } => collect_field_refs_recursive(body, refs),
        ComputedExpr::Var { .. }
        | ComputedExpr::Literal { .. }
        | ComputedExpr::ByteArray { .. }
        | ComputedExpr::None
        | ComputedExpr::ContextSlot
        | ComputedExpr::ContextTimestamp => {}
    }
}

/// Three-color DFS cycle detection. White (0) = unvisited, gray (1) = on the
/// current path, black (2) = fully processed. Gray back-edges record cycles;
/// black nodes are safe to skip entirely.
fn detect_cycles(graph: &HashMap<String, HashSet<String>>) -> Vec<Vec<String>> {
    let mut color: HashMap<String, u8> = HashMap::new();
    let mut stack = Vec::new();
    let mut cycles = Vec::new();

    let mut nodes: Vec<&String> = graph.keys().collect();
    nodes.sort();

    for node in nodes {
        detect_cycles_from(node, graph, &mut color, &mut stack, &mut cycles);
    }

    cycles
}

fn detect_cycles_from(
    node: &str,
    graph: &HashMap<String, HashSet<String>>,
    color: &mut HashMap<String, u8>,
    stack: &mut Vec<String>,
    cycles: &mut Vec<Vec<String>>,
) {
    match color.get(node).copied().unwrap_or(0) {
        2 => return, // black: fully processed
        1 => {
            // gray: back-edge — cycle found
            if let Some(index) = stack.iter().position(|entry| entry == node) {
                let mut cycle = stack[index..].to_vec();
                cycle.push(node.to_string());
                if !cycles.iter().any(|existing| existing == &cycle) {
                    cycles.push(cycle);
                }
            }
            return;
        }
        _ => {} // white: first visit
    }

    color.insert(node.to_string(), 1); // gray
    stack.push(node.to_string());

    if let Some(edges) = graph.get(node) {
        let mut sorted_edges: Vec<&String> = edges.iter().collect();
        sorted_edges.sort();

        for edge in sorted_edges {
            detect_cycles_from(edge, graph, color, stack, cycles);
        }
    }

    stack.pop();
    color.insert(node.to_string(), 2); // black
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{
        CompareOp, Predicate, PredicateValue, ViewDef, ViewOutput, ViewSource, ViewTransform,
    };
    use crate::parse;

    #[test]
    fn filter_view_fields_are_validated() {
        let mut known_fields = HashSet::new();
        known_fields.insert("existing".to_string());
        let available_fields = vec!["existing".to_string()];
        let mut errors = ErrorCollector::default();

        validate_views(
            "Thing",
            &[parse::ViewAttributeSpec {
                view: ViewDef {
                    id: "latest".to_string(),
                    source: ViewSource::Entity {
                        name: "Thing".to_string(),
                    },
                    pipeline: vec![ViewTransform::Filter {
                        predicate: Predicate::Compare {
                            field: FieldPath::new(&["ghost"]),
                            op: CompareOp::Eq,
                            value: PredicateValue::Literal(serde_json::json!(true)),
                        },
                    }],
                    output: ViewOutput::Collection,
                },
                attr_span: proc_macro2::Span::call_site(),
                sort_key_span: None,
            }],
            &known_fields,
            &available_fields,
            &mut errors,
        );

        let error = errors
            .finish()
            .expect_err("filter field should be validated");
        assert!(error
            .to_string()
            .contains("unknown view filter field 'ghost' on entity 'Thing'"));
    }
}
