//! IDL-based stream processing.
//!
//! This module handles processing of `#[hyperstack(idl = "...")]` modules,
//! which generate SDK types, parsers, and entity processing from Anchor IDL files.
//! Supports multiple IDLs for multi-program stacks.

use std::collections::{BTreeMap, HashMap, HashSet};

use quote::quote;
use syn::spanned::Spanned;
use syn::{Item, ItemMod};

use crate::ast::writer::{extract_instructions_from_idl, extract_pdas_from_idl};
use crate::ast::SerializableStackSpec;
use crate::codegen::generate_multi_entity_builder;
use crate::diagnostic::{idl_error_to_syn, internal_codegen_error, parse_generated_items};
use crate::idl_codegen;
use crate::idl_parser_gen;
use crate::idl_vixen_gen;
use crate::parse;
use crate::parse::idl as idl_parser;
use crate::parse::pdas::PdasBlock;
use crate::utils::{to_pascal_case, to_snake_case};
use crate::validation::validate_pda_blocks;

use super::entity::process_entity_struct_with_idl;
use super::handlers::{
    generate_auto_resolver_functions, generate_pda_registration_functions,
    generate_resolver_functions,
};

struct IdlInfo {
    idl: idl_parser::IdlSpec,
    program_id: String,
    program_name: String,
    sdk_module_name: String,
    parser_module_name: String,
}

pub fn process_idl_spec(
    mut module: ItemMod,
    idl_paths: &[String],
) -> syn::Result<proc_macro2::TokenStream> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());

    let mut idl_infos: Vec<IdlInfo> = Vec::new();

    for idl_path in idl_paths {
        let full_path = std::path::Path::new(&manifest_dir).join(idl_path);

        let idl = match idl_parser::parse_idl_file(&full_path) {
            Ok(idl) => idl,
            Err(e) => {
                return Err(idl_error_to_syn(
                    module.ident.span(),
                    hyperstack_idl::error::IdlSearchError::ParseError {
                        path: idl_path.clone(),
                        source: e,
                    },
                ));
            }
        };

        let program_id = idl
            .address
            .as_ref()
            .or_else(|| idl.metadata.as_ref().and_then(|m| m.address.as_ref()))
            .map(|s| s.to_string())
            .unwrap_or_else(|| "11111111111111111111111111111111".to_string());

        let program_name = idl.get_name().to_string();
        let sdk_module_name = format!("{}_sdk", program_name);

        let parser_module_name = if idl_paths.len() == 1 {
            "parsers".to_string()
        } else {
            format!("{}_parsers", program_name)
        };

        idl_infos.push(IdlInfo {
            idl,
            program_id,
            program_name,
            sdk_module_name,
            parser_module_name,
        });
    }

    let primary = &idl_infos[0];

    let mut all_sdk_tokens: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut all_parser_tokens: Vec<proc_macro2::TokenStream> = Vec::new();

    for info in &idl_infos {
        let sdk_types = idl_codegen::generate_sdk_types(&info.idl, &info.sdk_module_name);
        all_sdk_tokens.push(sdk_types);

        let parsers = idl_parser_gen::generate_named_parsers(
            &info.idl,
            &info.program_id,
            &info.sdk_module_name,
            &info.parser_module_name,
        );
        all_parser_tokens.push(parsers);
    }

    let stack_name = to_pascal_case(&module.ident.to_string());

    let mut section_structs = HashMap::new();
    let mut entity_structs = Vec::new();
    let mut impl_blocks = Vec::new();
    let mut has_game_event = false;
    let mut manual_pdas_blocks: Vec<PdasBlock> = Vec::new();

    if let Some((_, items)) = &module.content {
        for item in items {
            if let Item::Struct(item_struct) = item {
                if item_struct.ident == "GameEvent" {
                    has_game_event = true;
                }

                let has_stream_section = item_struct.attrs.iter().any(|attr| {
                    if attr.path().is_ident("derive") {
                        if let syn::Meta::List(meta_list) = &attr.meta {
                            return meta_list.tokens.to_string().contains("Stream");
                        }
                    }
                    false
                });

                let has_entity = parse::has_entity_attribute(&item_struct.attrs);

                if has_entity {
                    entity_structs.push(item_struct.clone());
                } else if has_stream_section {
                    section_structs.insert(item_struct.ident.to_string(), item_struct.clone());
                }
            } else if let Item::Impl(impl_item) = item {
                impl_blocks.push(impl_item.clone());
            } else if let Item::Macro(item_macro) = item {
                if item_macro.mac.path.is_ident("pdas") {
                    match syn::parse2::<PdasBlock>(item_macro.mac.tokens.clone()) {
                        Ok(block) => manual_pdas_blocks.push(block),
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }

    let mut all_resolver_hooks = Vec::new();
    for impl_block in &impl_blocks {
        let hooks = parse::extract_resolver_hooks(impl_block)?;
        all_resolver_hooks.extend(hooks);
    }

    if let Some((_, items)) = &module.content {
        for item in items {
            if let Item::Fn(item_fn) = item {
                let hooks = parse::extract_resolver_hooks_from_fn(item_fn)?;
                all_resolver_hooks.extend(hooks);
            }
        }
    }

    let mut resolver_hooks: Vec<parse::ResolveKeyAttribute> = Vec::new();
    let mut pda_registrations: Vec<parse::RegisterPdaAttribute> = Vec::new();

    // Collect per-entity PDA registrations to avoid cross-entity contamination
    let per_entity_pda_regs =
        collect_pda_registrations_per_entity(&entity_structs, &section_structs)?;

    if let Some((_, items)) = &module.content {
        for item in items {
            if let Item::Struct(item_struct) = item {
                for attr in &item_struct.attrs {
                    if let Some(resolve_attr) = parse::parse_resolve_key_attribute(attr)? {
                        resolver_hooks.push(resolve_attr);
                    }

                    if let Some(register_attr) = parse::parse_register_pda_attribute(attr)? {
                        pda_registrations.push(register_attr);
                    }
                }
            }
        }
    }

    // Keep collect_register_from_specs for backwards compatibility with resolver_hooks
    collect_register_from_specs(
        &entity_structs,
        &section_structs,
        &mut resolver_hooks,
        &mut pda_registrations,
    )?;

    let mut seen_resolver_fns: HashSet<String> = HashSet::new();
    resolver_hooks.retain(|hook| {
        let account_name = hook
            .account_path
            .segments
            .last()
            .map(|seg| seg.ident.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let fn_name = format!("resolve_{}_key", to_snake_case(&account_name));
        seen_resolver_fns.insert(fn_name)
    });

    for resolve_attr in &resolver_hooks {
        let account_name = resolve_attr
            .account_path
            .segments
            .last()
            .map(|seg| seg.ident.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let fn_name = syn::Ident::new(
            &format!("resolve_{}_key", to_snake_case(&account_name)),
            resolve_attr.account_path.span(),
        );

        let fn_sig: syn::Signature = syn::parse_quote! {
            fn #fn_name(
                account_address: &str,
                _account_data: &serde_json::Value,
                ctx: &mut hyperstack_interpreter::resolvers::ResolveContext
            ) -> hyperstack_interpreter::resolvers::KeyResolution
        };

        all_resolver_hooks.push(parse::ResolverHookSpec {
            kind: parse::ResolverHookKind::KeyResolver,
            account_type_path: resolve_attr.account_path.clone(),
            fn_name,
            fn_sig,
        });
    }

    for (i, pda_attr) in pda_registrations.iter().enumerate() {
        let fn_name = syn::Ident::new(
            &format!("register_pda_{}", i),
            pda_attr.instruction_path.span(),
        );

        let fn_sig: syn::Signature = syn::parse_quote! {
            fn #fn_name(ctx: &mut hyperstack_interpreter::resolvers::InstructionContext)
        };

        all_resolver_hooks.push(parse::ResolverHookSpec {
            kind: parse::ResolverHookKind::AfterInstruction,
            account_type_path: pda_attr.instruction_path.clone(),
            fn_name,
            fn_sig,
        });
    }

    if !entity_structs.is_empty() {
        let mut all_outputs = Vec::new();
        let mut entity_names = Vec::new();

        let idl_lookup: Vec<(String, &idl_parser::IdlSpec)> = idl_infos
            .iter()
            .map(|info| (info.sdk_module_name.clone(), &info.idl))
            .collect();

        for entity_struct in &entity_structs {
            let entity_name = parse::parse_entity_name(&entity_struct.attrs)
                .unwrap_or_else(|| entity_struct.ident.to_string());
            entity_names.push(entity_name.clone());

            let result = process_entity_struct_with_idl(
                entity_struct.clone(),
                entity_name.clone(),
                section_structs.clone(),
                has_game_event,
                &stack_name,
                &idl_lookup,
                resolver_hooks.clone(),
                per_entity_pda_regs
                    .get(&entity_name)
                    .cloned()
                    .unwrap_or_default(),
            )?;

            for hook in &result.auto_resolver_hooks {
                let account_name =
                    crate::event_type_helpers::strip_event_type_suffix(&hook.account_type);
                let fn_name = syn::Ident::new(
                    &format!("resolve_{}_key", to_snake_case(account_name)),
                    proc_macro2::Span::call_site(),
                );
                let fn_sig: syn::Signature = syn::parse_quote! {
                    fn #fn_name(
                        account_address: &str,
                        _account_data: &serde_json::Value,
                        ctx: &mut hyperstack_interpreter::resolvers::ResolveContext
                    ) -> hyperstack_interpreter::resolvers::KeyResolution
                };
                let account_type_path: syn::Path =
                    syn::parse_str(account_name).unwrap_or_else(|_| syn::parse_quote!(#fn_name));
                all_resolver_hooks.push(parse::ResolverHookSpec {
                    kind: parse::ResolverHookKind::KeyResolver,
                    account_type_path,
                    fn_name,
                    fn_sig,
                });
            }

            all_outputs.push(result);
        }

        if let Some((_brace, items)) = &mut module.content {
            items.retain(|item| {
                if let Item::Struct(s) = item {
                    if parse::has_entity_attribute(&s.attrs) {
                        return false;
                    }
                    let has_declarative_attr = s.attrs.iter().any(|attr| {
                        attr.path().is_ident("resolve_key") || attr.path().is_ident("register_pda")
                    });
                    !has_declarative_attr
                } else {
                    true
                }
            });

            for item in items.iter_mut() {
                if let Item::Impl(impl_item) = item {
                    for impl_item_inner in &mut impl_item.items {
                        if let syn::ImplItem::Fn(method) = impl_item_inner {
                            method.attrs.retain(|attr| {
                                !attr.path().is_ident("resolve_key_for")
                                    && !attr.path().is_ident("after_instruction")
                            });
                        }
                    }
                } else if let Item::Fn(item_fn) = item {
                    item_fn.attrs.retain(|attr| {
                        !attr.path().is_ident("resolve_key_for")
                            && !attr.path().is_ident("after_instruction")
                    });
                }
            }

            for sdk_tokens in &all_sdk_tokens {
                for gen_item in parse_generated_items(
                    sdk_tokens.clone(),
                    module.ident.span(),
                    "IDL SDK module",
                )? {
                    items.push(gen_item);
                }
            }

            for parser_tokens in &all_parser_tokens {
                for gen_item in parse_generated_items(
                    parser_tokens.clone(),
                    module.ident.span(),
                    "IDL parser module",
                )? {
                    items.push(gen_item);
                }
            }

            for result in &all_outputs {
                for gen_item in parse_generated_items(
                    result.token_stream.clone(),
                    module.ident.span(),
                    "entity expansion",
                )? {
                    items.push(gen_item);
                }
            }

            let mut seen_auto_resolver_fns = seen_resolver_fns.clone();
            let mut deduped_auto_hooks = Vec::new();
            for result in &all_outputs {
                for hook in &result.auto_resolver_hooks {
                    let account_name =
                        crate::event_type_helpers::strip_event_type_suffix(&hook.account_type);
                    let fn_name = format!("resolve_{}_key", to_snake_case(account_name));
                    if seen_auto_resolver_fns.insert(fn_name) {
                        deduped_auto_hooks.push(hook.clone());
                    }
                }
            }
            if !deduped_auto_hooks.is_empty() {
                let auto_fns = generate_auto_resolver_functions(&deduped_auto_hooks);
                for gen_item in
                    parse_generated_items(auto_fns, module.ident.span(), "auto resolver functions")?
                {
                    items.push(gen_item);
                }
            }

            let primary_idl = idl_infos.first().map(|info| &info.idl);
            let resolver_fns = generate_resolver_functions(&resolver_hooks, primary_idl);
            let pda_registration_fns = generate_pda_registration_functions(&pda_registrations);
            let combined_hook_fns: proc_macro2::TokenStream = quote! {
                #resolver_fns
                #pda_registration_fns
            };
            for gen_item in parse_generated_items(
                combined_hook_fns,
                module.ident.span(),
                "resolver hook functions",
            )? {
                items.push(gen_item);
            }

            let entity_asts: Vec<crate::ast::SerializableStreamSpec> = all_outputs
                .iter()
                .filter_map(|result| result.ast_spec.clone())
                .collect();

            let all_program_ids: Vec<String> = idl_infos
                .iter()
                .map(|info| info.program_id.clone())
                .collect();

            let all_idl_snapshots: Vec<_> = idl_infos
                .iter()
                .map(|info| {
                    let mut snapshot = crate::ast::writer::convert_idl_to_snapshot(&info.idl);
                    snapshot.program_id = Some(info.program_id.clone());
                    snapshot
                })
                .collect();

            let mut all_pdas: BTreeMap<String, BTreeMap<String, crate::ast::PdaDefinition>> =
                BTreeMap::new();
            let mut all_instructions: Vec<crate::ast::InstructionDef> = Vec::new();
            for info in &idl_infos {
                let pdas = extract_pdas_from_idl(&info.idl);
                let flat_pdas: BTreeMap<String, crate::ast::PdaDefinition> =
                    pdas.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                let instructions = extract_instructions_from_idl(&info.idl, &flat_pdas);
                all_pdas.insert(info.program_name.clone(), pdas);
                all_instructions.extend(instructions);
            }

            let idl_map: HashMap<String, idl_parser::IdlSpec> = idl_infos
                .iter()
                .map(|info| (info.program_name.clone(), info.idl.clone()))
                .collect();
            validate_pda_blocks(&idl_map, &manual_pdas_blocks)?;

            for manual_block in &manual_pdas_blocks {
                for program_pdas in &manual_block.programs {
                    let program_entry = all_pdas
                        .entry(program_pdas.program_name.clone())
                        .or_default();

                    for pda in &program_pdas.pdas {
                        program_entry.insert(pda.name.clone(), pda.to_pda_definition());
                    }
                }
            }

            let stack_spec = SerializableStackSpec {
                ast_version: crate::ast::CURRENT_AST_VERSION.to_string(),
                stack_name: stack_name.clone(),
                program_ids: all_program_ids,
                idls: all_idl_snapshots,
                entities: entity_asts
                    .into_iter()
                    .map(|mut e| {
                        e.idl = None;
                        e
                    })
                    .collect(),
                pdas: all_pdas,
                instructions: all_instructions,
                content_hash: None,
            }
            .try_with_content_hash()
            .map_err(|error| {
                internal_codegen_error(
                    module.ident.span(),
                    format!("failed to serialize stack spec for hashing: {error}"),
                )
            })?;

            let stack_spec_json = serde_json::to_string(&stack_spec).map_err(|error| {
                internal_codegen_error(
                    module.ident.span(),
                    format!("failed to serialize embedded stack spec: {error}"),
                )
            })?;

            crate::ast::writer::write_stack_to_file(&stack_spec, &stack_name).map_err(|e| {
                syn::Error::new(
                    module.ident.span(),
                    format!("Failed to write stack AST: {e}"),
                )
            })?;

            let multi_entity_builder = generate_multi_entity_builder(
                &entity_names,
                &[],
                false,
                &stack_name,
                &stack_spec_json,
            );
            for gen_item in parse_generated_items(
                multi_entity_builder,
                module.ident.span(),
                "multi-entity builder",
            )? {
                items.push(gen_item);
            }

            let resolver_registries = idl_vixen_gen::generate_resolver_registries(
                &all_resolver_hooks,
                &primary.program_name,
            );
            for gen_item in parse_generated_items(
                resolver_registries,
                module.ident.span(),
                "resolver registries",
            )? {
                items.push(gen_item);
            }

            let spec_function = idl_vixen_gen::generate_multi_idl_spec_function(
                &idl_infos
                    .iter()
                    .map(|info| {
                        (
                            &info.idl,
                            info.program_id.as_str(),
                            info.parser_module_name.as_str(),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
            for gen_item in
                parse_generated_items(spec_function, module.ident.span(), "IDL spec function")?
            {
                items.push(gen_item);
            }
        }
    } else if let Some((_brace, items)) = &mut module.content {
        for sdk_tokens in &all_sdk_tokens {
            for gen_item in
                parse_generated_items(sdk_tokens.clone(), module.ident.span(), "IDL SDK module")?
            {
                items.push(gen_item);
            }
        }

        for parser_tokens in &all_parser_tokens {
            for gen_item in parse_generated_items(
                parser_tokens.clone(),
                module.ident.span(),
                "IDL parser module",
            )? {
                items.push(gen_item);
            }
        }
    }

    Ok(quote! {
        #module
    })
}

fn collect_register_from_specs(
    entity_structs: &[syn::ItemStruct],
    section_structs: &HashMap<String, syn::ItemStruct>,
    resolver_hooks: &mut Vec<parse::ResolveKeyAttribute>,
    pda_registrations: &mut Vec<parse::RegisterPdaAttribute>,
) -> syn::Result<()> {
    let mut all_structs_to_scan: Vec<&syn::ItemStruct> = entity_structs.iter().collect();
    all_structs_to_scan.extend(section_structs.values());

    for item_struct in all_structs_to_scan {
        if let syn::Fields::Named(fields) = &item_struct.fields {
            for field in &fields.named {
                let field_name = field
                    .ident
                    .as_ref()
                    .map(|i| i.to_string())
                    .unwrap_or_default();
                for attr in &field.attrs {
                    if let Some(parse::RecognizedFieldAttribute::Map(map_attrs)) =
                        parse::parse_recognized_field_attribute(attr, &field_name)?
                    {
                        for map_attr in &map_attrs {
                            if !map_attr.register_from.is_empty() {
                                let account_path = map_attr.source_type_path.clone();
                                let instruction_paths: Vec<syn::Path> = map_attr
                                    .register_from
                                    .iter()
                                    .map(|rf| rf.instruction_path.clone())
                                    .collect();

                                resolver_hooks.push(parse::ResolveKeyAttribute {
                                    attr_span: map_attr.attr_span,
                                    account_path,
                                    strategy: "pda_reverse_lookup".to_string(),
                                    lookup_name: None,
                                    queue_until: instruction_paths,
                                });

                                for rf in &map_attr.register_from {
                                    pda_registrations.push(parse::RegisterPdaAttribute {
                                        attr_span: map_attr.attr_span,
                                        instruction_path: rf.instruction_path.clone(),
                                        pda_field: rf.pda_field.clone(),
                                        primary_key_field: rf.primary_key_field.clone(),
                                        lookup_name: "default_pda_lookup".to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Collect PDA registrations per-entity to prevent cross-entity contamination.
///
/// This function returns a HashMap where the key is the entity name and the value
/// is the list of PDA registrations for that specific entity only.
///
/// Scans both entity struct fields and section struct fields, since `register_from`
/// attributes are typically defined inside section structs (e.g., `PoolId`, `PositionId`).
fn collect_pda_registrations_per_entity(
    entity_structs: &[syn::ItemStruct],
    section_structs: &HashMap<String, syn::ItemStruct>,
) -> syn::Result<HashMap<String, Vec<parse::RegisterPdaAttribute>>> {
    let mut per_entity_regs: HashMap<String, Vec<parse::RegisterPdaAttribute>> = HashMap::new();

    for entity_struct in entity_structs {
        let entity_name = parse::parse_entity_name(&entity_struct.attrs)
            .unwrap_or_else(|| entity_struct.ident.to_string());
        let mut entity_regs: Vec<parse::RegisterPdaAttribute> = Vec::new();

        // Collect all structs to scan: the entity struct itself plus any section structs it references
        let mut structs_to_scan: Vec<&syn::ItemStruct> = vec![entity_struct];
        if let syn::Fields::Named(fields) = &entity_struct.fields {
            for field in &fields.named {
                if let syn::Type::Path(type_path) = &field.ty {
                    if let Some(type_ident) = type_path.path.segments.last() {
                        let type_name = type_ident.ident.to_string();
                        if let Some(section_struct) = section_structs.get(&type_name) {
                            structs_to_scan.push(section_struct);
                        }
                    }
                }
            }
        }

        for scan_struct in &structs_to_scan {
            if let syn::Fields::Named(fields) = &scan_struct.fields {
                for field in &fields.named {
                    let field_name = field
                        .ident
                        .as_ref()
                        .map(|i| i.to_string())
                        .unwrap_or_default();
                    for attr in &field.attrs {
                        if let Some(parse::RecognizedFieldAttribute::Map(map_attrs)) =
                            parse::parse_recognized_field_attribute(attr, &field_name)?
                        {
                            for map_attr in &map_attrs {
                                if !map_attr.register_from.is_empty() {
                                    for rf in &map_attr.register_from {
                                        entity_regs.push(parse::RegisterPdaAttribute {
                                            attr_span: map_attr.attr_span,
                                            instruction_path: rf.instruction_path.clone(),
                                            pda_field: rf.pda_field.clone(),
                                            primary_key_field: rf.primary_key_field.clone(),
                                            lookup_name: "default_pda_lookup".to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if !entity_regs.is_empty() {
            per_entity_regs.insert(entity_name, entity_regs);
        }
    }

    Ok(per_entity_regs)
}
