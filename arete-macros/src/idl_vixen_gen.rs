//! IDL-based Vixen runtime generation.
//!
//! This module provides resolver registry generation for IDL-based streams.
//! The VmHandler and spec function generation is delegated to the unified
//! `codegen::vixen_runtime` module.

#![allow(dead_code)]

use crate::codegen::vixen_runtime::{self, RuntimeGenConfig};
use crate::parse::idl::*;
use crate::parse::{ResolverHookKind, ResolverHookSpec};
use proc_macro2::TokenStream;
use quote::quote;
use syn::Path;

fn path_to_event_type(path: &Path, is_instruction: bool, default_program_name: &str) -> String {
    let type_name = path
        .segments
        .last()
        .map(|seg| seg.ident.to_string())
        .unwrap_or_default();
    let suffix = if is_instruction { "IxState" } else { "State" };

    // Always use the program name from the IDL spec, not the generated module path.
    // The parser's event_type() uses the IDL program name (e.g., "pump"),
    // so hook match arms must use the same prefix to match at runtime.
    format!("{}::{}{}", default_program_name, type_name, suffix)
}

pub fn generate_resolver_registries(
    resolver_hooks: &[ResolverHookSpec],
    program_name: &str,
) -> TokenStream {
    let resolver_registry = generate_resolver_registry(resolver_hooks, program_name);
    let instruction_hook_registry =
        generate_instruction_hook_registry(resolver_hooks, program_name);

    quote! {
        #resolver_registry
        #instruction_hook_registry
    }
}

fn generate_resolver_registry(
    resolver_hooks: &[ResolverHookSpec],
    program_name: &str,
) -> TokenStream {
    let key_resolvers: Vec<_> = resolver_hooks
        .iter()
        .filter(|hook| matches!(hook.kind, ResolverHookKind::KeyResolver))
        .collect();

    if key_resolvers.is_empty() {
        return quote! {
            fn get_resolver_for_account_type(_account_type: &str) -> Option<fn(&str, &arete::runtime::serde_json::Value, &mut arete::runtime::arete_interpreter::resolvers::ResolveContext) -> arete::runtime::arete_interpreter::resolvers::KeyResolution> {
                None
            }
        };
    }

    let resolver_arms = key_resolvers.iter().map(|hook| {
        let event_type = path_to_event_type(&hook.account_type_path, false, program_name);
        let fn_name = &hook.fn_name;

        quote! {
            #event_type => {
                Some(#fn_name)
            }
        }
    });

    quote! {
        fn get_resolver_for_account_type(account_type: &str) -> Option<fn(&str, &arete::runtime::serde_json::Value, &mut arete::runtime::arete_interpreter::resolvers::ResolveContext) -> arete::runtime::arete_interpreter::resolvers::KeyResolution> {
            match account_type {
                #(#resolver_arms)*
                _ => None
            }
        }
    }
}

fn generate_instruction_hook_registry(
    resolver_hooks: &[ResolverHookSpec],
    program_name: &str,
) -> TokenStream {
    let instruction_hooks: Vec<_> = resolver_hooks
        .iter()
        .filter(|hook| matches!(hook.kind, ResolverHookKind::AfterInstruction))
        .collect();

    if instruction_hooks.is_empty() {
        return quote! {
            fn get_instruction_hooks(_instruction_type: &str) -> Vec<fn(&mut arete::runtime::arete_interpreter::resolvers::InstructionContext)> {
                Vec::new()
            }
        };
    }

    use std::collections::HashMap as StdHashMap;
    let mut hooks_by_instruction: StdHashMap<String, Vec<&syn::Ident>> = StdHashMap::new();

    for hook in &instruction_hooks {
        let event_type = path_to_event_type(&hook.account_type_path, true, program_name);
        hooks_by_instruction
            .entry(event_type)
            .or_default()
            .push(&hook.fn_name);
    }

    let hook_arms = hooks_by_instruction.iter().map(|(event_type, hook_fns)| {
        quote! {
            #event_type => {
                vec![#(#hook_fns),*]
            }
        }
    });

    quote! {
        fn get_instruction_hooks(instruction_type: &str) -> Vec<fn(&mut arete::runtime::arete_interpreter::resolvers::InstructionContext)> {
            match instruction_type {
                #(#hook_arms)*
                _ => Vec::new()
            }
        }
    }
}

pub fn generate_spec_function(
    idl: &IdlSpec,
    program_id: &str,
    resolver_hooks: &[ResolverHookSpec],
) -> TokenStream {
    let program_name = idl.get_name();
    let registries = generate_resolver_registries(resolver_hooks, program_name);
    let spec_fn = generate_spec_function_without_registries(idl, program_id);

    quote! {
        #registries
        #spec_fn
    }
}

pub fn generate_spec_function_without_registries(idl: &IdlSpec, _program_id: &str) -> TokenStream {
    let program_name = idl.get_name();
    let state_enum_name = format!("{}State", to_pascal_case(program_name));
    let instruction_enum_name = format!("{}Instruction", to_pascal_case(program_name));

    let config = RuntimeGenConfig::for_idl();

    let vm_handler =
        vixen_runtime::generate_vm_handler(&state_enum_name, &instruction_enum_name, program_name);

    let spec_fn = vixen_runtime::generate_spec_function(
        &state_enum_name,
        &instruction_enum_name,
        program_name,
        &config,
    );

    quote! {
        #vm_handler
        #spec_fn
    }
}

pub fn generate_multi_idl_spec_function(idls: &[(&IdlSpec, &str, &str)]) -> TokenStream {
    let config = RuntimeGenConfig::for_idl();

    let vm_handler_struct = vixen_runtime::generate_vm_handler_struct();

    let handler_impls: Vec<TokenStream> = idls
        .iter()
        .map(|(idl, _program_id, parser_module_name)| {
            let program_name = idl.get_name();
            let state_enum_name = format!("{}State", to_pascal_case(program_name));
            let instruction_enum_name = format!("{}Instruction", to_pascal_case(program_name));

            let account_impl =
                vixen_runtime::generate_account_handler_impl(parser_module_name, &state_enum_name);
            let instruction_impl = vixen_runtime::generate_instruction_handler_impl(
                parser_module_name,
                &instruction_enum_name,
                program_name,
            );

            quote! {
                #account_impl
                #instruction_impl
            }
        })
        .collect();

    let pipeline_infos: Vec<vixen_runtime::PipelineInfo> = idls
        .iter()
        .map(|(idl, program_id, parser_module_name)| {
            let program_name = idl.get_name();
            vixen_runtime::PipelineInfo {
                parser_module_name: parser_module_name.to_string(),
                program_name: program_name.to_string(),
                program_id: program_id.to_string(),
                state_enum_name: format!("{}State", to_pascal_case(program_name)),
                instruction_enum_name: format!("{}Instruction", to_pascal_case(program_name)),
            }
        })
        .collect();

    let spec_fn = vixen_runtime::generate_multi_pipeline_spec_function(&pipeline_infos, &config);

    quote! {
        #vm_handler_struct
        #(#handler_impls)*
        #spec_fn
    }
}
