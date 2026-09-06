//! IDL-based stream processing.
//!
//! This module handles processing of `#[arete(idl = "...")]` modules,
//! which generate SDK types, parsers, and entity processing from Anchor IDL files.
//! Supports multiple IDLs for multi-program stacks.

use std::collections::{BTreeMap, HashMap, HashSet};

use quote::quote;
use syn::spanned::Spanned;
use syn::{Item, ItemMod};

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
    identity: arete_hash::OssProgramIdentityV1,
}

fn build_idl_info(idl_bytes: &[u8], multiple_idls: bool) -> Result<IdlInfo, arete_hash::HashError> {
    let document = arete_hash::CanonicalIdlDocument::parse(idl_bytes, None)?;
    let idl = document.parsed_idl().clone();
    let program_id = document.program_id().to_string();
    let identity = arete_hash::OssProgramIdentityV1::from_document(&document)?;
    let program_name = idl.get_name().to_string();
    let sdk_module_name = format!("{}_sdk", program_name);
    let parser_module_name = if multiple_idls {
        format!("{}_parsers", program_name)
    } else {
        "parsers".to_string()
    };

    Ok(IdlInfo {
        idl,
        program_id,
        program_name,
        sdk_module_name,
        parser_module_name,
        identity,
    })
}

pub fn process_idl_spec(
    mut module: ItemMod,
    idl_paths: &[String],
) -> syn::Result<proc_macro2::TokenStream> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());

    let mut idl_infos: Vec<IdlInfo> = Vec::new();

    for idl_path in idl_paths {
        let full_path = std::path::Path::new(&manifest_dir).join(idl_path);

        let idl_bytes = match std::fs::read(&full_path) {
            Ok(bytes) => bytes,
            Err(e) => {
                return Err(idl_error_to_syn(
                    module.ident.span(),
                    arete_idl::error::IdlSearchError::ParseError {
                        path: idl_path.clone(),
                        source: e.to_string(),
                    },
                ));
            }
        };
        let info = match build_idl_info(&idl_bytes, idl_paths.len() > 1) {
            Ok(info) => info,
            Err(error) => {
                return Err(idl_error_to_syn(
                    module.ident.span(),
                    arete_idl::error::IdlSearchError::ParseError {
                        path: idl_path.clone(),
                        source: error.to_string(),
                    },
                ));
            }
        };
        idl_infos.push(info);
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

    let program_specs_json: Vec<syn::LitStr> = idl_infos
        .iter()
        .map(|info| {
            serde_json::to_string(&info.identity.program_spec)
                .map(|json| syn::LitStr::new(&json, module.ident.span()))
                .map_err(|error| {
                    internal_codegen_error(
                        module.ident.span(),
                        format!("failed to serialize ProgramSpecV1: {error}"),
                    )
                })
        })
        .collect::<syn::Result<_>>()?;
    let program_spec_hashes: Vec<syn::LitStr> = idl_infos
        .iter()
        .map(|info| {
            syn::LitStr::new(
                &info.identity.program_spec_hash.to_string(),
                module.ident.span(),
            )
        })
        .collect();
    let release_hashes: Vec<syn::LitStr> = idl_infos
        .iter()
        .map(|info| syn::LitStr::new(&info.identity.release_hash.to_string(), module.ident.span()))
        .collect();
    let identity_constants = quote! {
        #[doc(hidden)]
        pub const __ARETE_PROGRAM_SPECS_V1_JSON: &[&str] = &[#(#program_specs_json),*];
        #[doc(hidden)]
        pub const __ARETE_PROGRAM_SPEC_HASHES_V1: &[&str] = &[#(#program_spec_hashes),*];
        #[doc(hidden)]
        pub const __ARETE_OSS_PROGRAM_RELEASE_HASHES_V1: &[&str] = &[#(#release_hashes),*];
    };
    if let Some((_brace, items)) = &mut module.content {
        for item in parse_generated_items(
            identity_constants,
            module.ident.span(),
            "authoritative program identities",
        )? {
            items.push(item);
        }
    }

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
                    manual_pdas_blocks
                        .push(syn::parse2::<PdasBlock>(item_macro.mac.tokens.clone())?);
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
                ctx: &mut arete_interpreter::resolvers::ResolveContext
            ) -> arete_interpreter::resolvers::KeyResolution
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
            fn #fn_name(ctx: &mut arete_interpreter::resolvers::InstructionContext)
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
                        ctx: &mut arete_interpreter::resolvers::ResolveContext
                    ) -> arete_interpreter::resolvers::KeyResolution
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
                } else if let Item::Macro(item_macro) = item {
                    // `pdas! { ... }` is consumed during spec construction; it is
                    // not a real macro, so it must not survive into the emitted
                    // module or compilation fails with "cannot find macro `pdas`".
                    !item_macro.mac.path.is_ident("pdas")
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

            let mut entity_asts: Vec<crate::ast::SerializableStreamSpec> = all_outputs
                .iter()
                .filter_map(|result| result.ast_spec.clone())
                .collect();
            for entity in &mut entity_asts {
                entity.idl = None;
                entity.normalize_event_names();
            }

            let all_program_ids: Vec<String> = idl_infos
                .iter()
                .map(|info| info.program_id.clone())
                .collect();

            let all_idl_snapshots: Vec<_> = idl_infos
                .iter()
                .map(|info| {
                    let mut snapshot = info.identity.program_spec.idl_snapshot.snapshot.clone();
                    // The macro's legacy stack AST stored Steel's value only in
                    // `discriminator`. Keep that wire projection, and therefore
                    // its bare content_hash, unchanged.
                    for instruction in &mut snapshot.instructions {
                        instruction.discriminant = None;
                    }
                    snapshot
                })
                .collect();
            let all_program_specs: Vec<arete_hash::ProgramSpecV1> = idl_infos
                .iter()
                .map(|info| info.identity.program_spec.clone())
                .collect();

            let idl_map: HashMap<String, idl_parser::IdlSpec> = idl_infos
                .iter()
                .map(|info| (info.program_name.clone(), info.idl.clone()))
                .collect();
            validate_pda_blocks(&idl_map, &manual_pdas_blocks)?;

            // Build per-program manual PDA definitions from `pdas!` blocks so they
            // can participate in instruction-account resolution below. This matters
            // for Steel-style IDLs whose accounts carry no embedded PDA metadata:
            // the registry is matched against account names in `convert_account_to_def`.
            let mut manual_pdas_by_program: HashMap<
                String,
                BTreeMap<String, crate::ast::PdaDefinition>,
            > = HashMap::new();
            for manual_block in &manual_pdas_blocks {
                for program_pdas in &manual_block.programs {
                    let entry = manual_pdas_by_program
                        .entry(program_pdas.program_name.clone())
                        .or_default();
                    for pda in &program_pdas.pdas {
                        entry.insert(pda.name.clone(), pda.to_pda_definition());
                    }
                }
            }

            let mut all_pdas: BTreeMap<String, BTreeMap<String, crate::ast::PdaDefinition>> =
                BTreeMap::new();
            let mut all_instructions: Vec<crate::ast::InstructionDef> = Vec::new();
            for info in &idl_infos {
                let mut pdas: BTreeMap<String, crate::ast::PdaDefinition> =
                    transcode_program_projection(info.identity.program_spec.pdas.clone())?;
                if let Some(manual) = manual_pdas_by_program.get(&info.program_name) {
                    for (k, v) in manual {
                        pdas.insert(k.clone(), v.clone());
                    }
                }
                let mut instructions: Vec<crate::ast::InstructionDef> =
                    transcode_program_projection(info.identity.program_spec.instructions.clone())?;
                if let Some(manual) = manual_pdas_by_program.get(&info.program_name) {
                    apply_manual_pda_overlay(&mut instructions, manual);
                }
                all_pdas.insert(info.program_name.clone(), pdas);
                all_instructions.extend(instructions);
            }

            let mut stack_spec = SerializableStackSpec {
                ast_version: crate::ast::CURRENT_AST_VERSION.to_string(),
                stack_name: stack_name.clone(),
                program_ids: all_program_ids,
                idls: all_idl_snapshots,
                program_specs: all_program_specs.clone(),
                entities: entity_asts.clone(),
                pdas: all_pdas.clone(),
                instructions: all_instructions.clone(),
                content_hash: None,
            };
            stack_spec.normalize_event_names();

            let stack_spec = stack_spec.try_with_content_hash().map_err(|error| {
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

            crate::ast::writer::write_public_artifacts(
                &stack_name,
                &all_program_specs,
                &entity_asts,
                &all_pdas,
                &all_instructions,
            )
            .map_err(|e| {
                syn::Error::new(
                    module.ident.span(),
                    format!("Failed to write public stack artifacts: {e}"),
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
                            &info.identity,
                        )
                    })
                    .collect::<Vec<_>>(),
                true,
            );
            for gen_item in
                parse_generated_items(spec_function, module.ident.span(), "IDL spec function")?
            {
                items.push(gen_item);
            }
        }
    } else if let Some((_brace, items)) = &mut module.content {
        items.retain(
            |item| !matches!(item, Item::Macro(item_macro) if item_macro.mac.path.is_ident("pdas")),
        );
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

        let all_program_specs: Vec<arete_hash::ProgramSpecV1> = idl_infos
            .iter()
            .map(|info| info.identity.program_spec.clone())
            .collect();
        let all_pdas = idl_infos
            .iter()
            .map(|info| {
                transcode_program_projection(info.identity.program_spec.pdas.clone())
                    .map(|pdas| (info.program_name.clone(), pdas))
            })
            .collect::<syn::Result<BTreeMap<_, _>>>()?;
        let all_instructions: Vec<crate::ast::InstructionDef> = idl_infos
            .iter()
            .map(|info| {
                transcode_program_projection::<_, Vec<crate::ast::InstructionDef>>(
                    info.identity.program_spec.instructions.clone(),
                )
            })
            .collect::<syn::Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();
        crate::ast::writer::write_public_artifacts(
            &stack_name,
            &all_program_specs,
            &[],
            &all_pdas,
            &all_instructions,
        )
        .map_err(|error| {
            syn::Error::new(
                module.ident.span(),
                format!("Failed to write authoritative program artifacts: {error}"),
            )
        })?;

        let spec_function = idl_vixen_gen::generate_multi_idl_spec_function(
            &idl_infos
                .iter()
                .map(|info| {
                    (
                        &info.idl,
                        info.program_id.as_str(),
                        info.parser_module_name.as_str(),
                        &info.identity,
                    )
                })
                .collect::<Vec<_>>(),
            false,
        );
        for gen_item in parse_generated_items(
            spec_function,
            module.ident.span(),
            "program-only spec function",
        )? {
            items.push(gen_item);
        }
    }

    Ok(quote! {
        #module
    })
}

fn transcode_program_projection<T, U>(value: T) -> syn::Result<U>
where
    T: serde::Serialize,
    U: serde::de::DeserializeOwned,
{
    let value = serde_json::to_value(value).map_err(|error| {
        internal_codegen_error(
            proc_macro2::Span::call_site(),
            format!("failed to serialize shared ProgramSpec projection: {error}"),
        )
    })?;
    serde_json::from_value(value).map_err(|error| {
        internal_codegen_error(
            proc_macro2::Span::call_site(),
            format!("shared ProgramSpec projection does not match legacy AST: {error}"),
        )
    })
}

/// Bind canonical user-provided instruction accounts to PDAs the stack author
/// declared explicitly in a `pdas!` overlay.
///
/// The canonical `ProgramSpec` already resolved every account the IDL itself
/// can justify (explicit top-level PDAs, and instruction-local PDAs kept per
/// instruction). Only the manual overlay may reinterpret a remaining
/// `UserProvided` account, and only by matching the author's declared name.
/// Canonical PDAs discovered on other instructions never participate: a PDA
/// that one instruction derives is not evidence that a same-named account in
/// another instruction is the same address.
fn apply_manual_pda_overlay(
    instructions: &mut [crate::ast::InstructionDef],
    manual_pdas: &BTreeMap<String, crate::ast::PdaDefinition>,
) {
    for account in instructions
        .iter_mut()
        .flat_map(|instruction| &mut instruction.accounts)
    {
        if matches!(
            account.resolution,
            crate::ast::AccountResolution::UserProvided
        ) && manual_pdas.contains_key(&account.name)
        {
            account.resolution = crate::ast::AccountResolution::PdaRef {
                pda_name: account.name.clone(),
            };
        }
    }
}

#[cfg(test)]
mod manual_pda_overlay_tests {
    use super::*;
    use crate::ast::{
        AccountResolution, InstructionAccountDef, InstructionDef, PdaDefinition, PdaSeedDef,
    };

    fn account(name: &str, resolution: AccountResolution) -> InstructionAccountDef {
        InstructionAccountDef {
            name: name.to_string(),
            is_signer: false,
            is_writable: true,
            resolution,
            is_optional: false,
            docs: vec![],
        }
    }

    fn instruction(name: &str, accounts: Vec<InstructionAccountDef>) -> InstructionDef {
        InstructionDef {
            name: name.to_string(),
            discriminator: vec![1],
            discriminator_size: 1,
            accounts,
            args: vec![],
            errors: vec![],
            program_id: None,
            docs: vec![],
        }
    }

    fn treasury_pda() -> PdaDefinition {
        PdaDefinition {
            name: "treasury".to_string(),
            seeds: vec![PdaSeedDef::Literal {
                value: "treasury".to_string(),
            }],
            program_id: None,
        }
    }

    fn canonical_instructions() -> Vec<InstructionDef> {
        vec![
            instruction(
                "create",
                vec![account(
                    "treasury",
                    AccountResolution::PdaInline {
                        seeds: vec![PdaSeedDef::Literal {
                            value: "treasury".to_string(),
                        }],
                        program_id: None,
                    },
                )],
            ),
            instruction(
                "claim",
                vec![
                    account("treasury", AccountResolution::UserProvided),
                    account("signer", AccountResolution::Signer),
                    account("recipient", AccountResolution::UserProvided),
                ],
            ),
        ]
    }

    fn resolution<'a>(
        instructions: &'a [InstructionDef],
        instruction: &str,
        account: &str,
    ) -> &'a AccountResolution {
        &instructions
            .iter()
            .find(|candidate| candidate.name == instruction)
            .unwrap()
            .accounts
            .iter()
            .find(|candidate| candidate.name == account)
            .unwrap()
            .resolution
    }

    #[test]
    fn canonical_user_provided_accounts_stay_user_provided_without_an_overlay() {
        let mut instructions = canonical_instructions();
        let before = instructions.clone();
        // No `pdas!` block: nothing may reinterpret a canonical account, even
        // though `create.treasury` derives a same-named PDA.
        apply_manual_pda_overlay(&mut instructions, &BTreeMap::new());
        assert_eq!(instructions, before);
        assert_eq!(
            resolution(&instructions, "claim", "treasury"),
            &AccountResolution::UserProvided
        );
    }

    #[test]
    fn explicit_overlay_binds_only_the_named_user_provided_accounts() {
        let mut instructions = canonical_instructions();
        let manual = BTreeMap::from([("treasury".to_string(), treasury_pda())]);
        apply_manual_pda_overlay(&mut instructions, &manual);

        assert!(matches!(
            resolution(&instructions, "claim", "treasury"),
            AccountResolution::PdaRef { pda_name } if pda_name == "treasury"
        ));
        // Canonical inline, signer, and unrelated user-provided accounts are untouched.
        assert!(matches!(
            resolution(&instructions, "create", "treasury"),
            AccountResolution::PdaInline { .. }
        ));
        assert_eq!(
            resolution(&instructions, "claim", "signer"),
            &AccountResolution::Signer
        );
        assert_eq!(
            resolution(&instructions, "claim", "recipient"),
            &AccountResolution::UserProvided
        );
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn macro_identity_derivation_matches_checked_in_vector() {
        let corpus: serde_json::Value =
            serde_json::from_str(include_str!("../../../test-vectors/hash-v1.json"))
                .expect("vector corpus");
        let vector = corpus["idlVectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|vector| vector["id"] == "idl-primary")
            .expect("primary IDL vector");
        let info = build_idl_info(vector["input"]["data"].as_str().unwrap().as_bytes(), false)
            .expect("macro IDL identity");

        assert_eq!(
            info.identity.program_spec_hash.to_string(),
            vector["expected"]["programSpecIdentity"]["hashId"]
        );
        assert_eq!(
            info.identity.release_hash.to_string(),
            vector["expected"]["ossReleaseIdentity"]["hashId"]
        );
        let legacy_snapshot = crate::ast::writer::convert_idl_to_snapshot(&info.idl);
        let mut shared_legacy_projection = info.identity.program_spec.idl_snapshot.snapshot.clone();
        for instruction in &mut shared_legacy_projection.instructions {
            instruction.discriminant = None;
        }
        assert_eq!(
            serde_json::to_value(legacy_snapshot).unwrap(),
            serde_json::to_value(shared_legacy_projection).unwrap()
        );
        assert_eq!(info.parser_module_name, "parsers");
    }
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
