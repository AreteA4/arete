//! Resolver and instruction hook registry generation.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::core::{generate_hook_actions, to_snake_case};
use crate::ast::*;
use crate::parse::idl::IdlSpec;

pub fn generate_resolver_registries(
    resolver_hooks: &[ResolverHook],
    instruction_hooks: &[InstructionHook],
    idl: Option<&IdlSpec>,
) -> TokenStream {
    let resolver_registry = generate_resolver_registry(resolver_hooks);
    let instruction_hook_registry = generate_instruction_hook_registry(instruction_hooks, idl);

    quote! {
        #resolver_registry
        #instruction_hook_registry
    }
}

fn generate_resolver_registry(resolver_hooks: &[ResolverHook]) -> TokenStream {
    if resolver_hooks.is_empty() {
        return quote! {
            fn get_resolver_for_account_type(_account_type: &str) -> Option<fn(&str, &arete::runtime::serde_json::Value, &mut arete::runtime::arete_interpreter::resolvers::ResolveContext) -> arete::runtime::arete_interpreter::resolvers::KeyResolution> {
                None
            }
        };
    }

    let resolver_arms = resolver_hooks.iter().map(|hook| {
        let event_type = hook.account_type.clone();
        
        let account_type_base = crate::event_type_helpers::strip_event_type_suffix(&event_type);
        let _fn_name = format_ident!("resolve_{}_key", to_snake_case(account_type_base));

        match &hook.strategy {
            ResolverStrategy::PdaReverseLookup { lookup_name: _, queue_discriminators } => {
                let disc_bytes: Vec<u8> = queue_discriminators.iter().flatten().copied().collect();
                
                quote! {
                    #event_type => {
                        Some(|account_address: &str, _account_data: &arete::runtime::serde_json::Value, ctx: &mut arete::runtime::arete_interpreter::resolvers::ResolveContext| {
                            if let Some(key) = ctx.pda_reverse_lookup(account_address) {
                                return arete::runtime::arete_interpreter::resolvers::KeyResolution::Found(key);
                            }
                            arete::runtime::arete_interpreter::resolvers::KeyResolution::QueueUntil(&[#(#disc_bytes),*])
                        })
                    }
                }
            }
            ResolverStrategy::DirectField { field_path } => {
                let segments = &field_path.segments;
                quote! {
                    #event_type => {
                        Some(|_account_address: &str, account_data: &arete::runtime::serde_json::Value, _ctx: &mut arete::runtime::arete_interpreter::resolvers::ResolveContext| {
                            let mut current = account_data;
                            #(
                                current = current.get(#segments).unwrap_or(&arete::runtime::serde_json::Value::Null);
                            )*
                            if let Some(key) = current.as_str() {
                                arete::runtime::arete_interpreter::resolvers::KeyResolution::Found(key.to_string())
                            } else {
                                arete::runtime::arete_interpreter::resolvers::KeyResolution::Found(String::new())
                            }
                        })
                    }
                }
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
    instruction_hooks: &[InstructionHook],
    _idl: Option<&IdlSpec>,
) -> TokenStream {
    if instruction_hooks.is_empty() {
        return quote! {
            fn get_instruction_hooks(_instruction_type: &str) -> Vec<fn(&mut arete::runtime::arete_interpreter::resolvers::InstructionContext)> {
                Vec::new()
            }
        };
    }

    use std::collections::HashMap;
    let mut hooks_by_instruction: HashMap<String, Vec<&InstructionHook>> = HashMap::new();
    for hook in instruction_hooks {
        let event_type = hook.instruction_type.clone();
        hooks_by_instruction
            .entry(event_type)
            .or_default()
            .push(hook);
    }

    let hook_arms = hooks_by_instruction.iter().map(|(event_type, hooks)| {
        let (hook_fn_defs, hook_fn_names): (Vec<_>, Vec<_>) = hooks
            .iter()
            .enumerate()
            .map(|(idx, hook)| {
                let instruction_base =
                    crate::event_type_helpers::strip_event_type_suffix(&hook.instruction_type);
                let fn_name = format_ident!("hook_{}_{}", to_snake_case(instruction_base), idx);
                let actions = generate_hook_actions(&hook.actions, &hook.lookup_by);

                let fn_def = quote! {
                    fn #fn_name(ctx: &mut arete::runtime::arete_interpreter::resolvers::InstructionContext) {
                        #actions
                    }
                };

                (fn_def, fn_name)
            })
            .unzip();

        quote! {
            #event_type => {
                #(#hook_fn_defs)*
                vec![#(#hook_fn_names),*]
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
