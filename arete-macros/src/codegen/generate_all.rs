//! Unified code generation from SerializableStreamSpec.
//!
//! This module provides code generation utilities. Note: The unified
//! `generate_all_from_spec` function is kept for reference but is currently
//! unused since `#[ast_spec]` was moved to the closed-source ast-compiler-macros crate.

#![allow(dead_code)]

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::ast::SerializableStreamSpec;
use crate::parse::idl::IdlSpec;

use super::{
    generate_bytecode_from_spec, generate_computed_evaluator, generate_handlers_from_specs,
    generate_parsers_from_idl, generate_resolver_registries, generate_sdk_from_idl,
    generate_spec_function, generate_vm_handler, RuntimeGenConfig,
};

/// Configuration for code generation.
///
/// This struct allows controlling which components are generated,
/// useful for partial generation in different contexts.
#[derive(Debug, Clone, Default)]
pub struct GenerateConfig {
    /// Generate SDK types (accounts, instructions)
    pub sdk_types: bool,
    /// Generate Vixen parsers
    pub parsers: bool,
    /// Generate VmHandler
    pub vm_handler: bool,
    /// Generate bytecode creation function
    pub bytecode: bool,
    /// Generate computed field evaluator
    pub computed: bool,
    /// Generate resolver registries
    pub resolvers: bool,
    /// Generate handler functions
    pub handlers: bool,
    /// Generate spec() function
    pub spec_fn: bool,
}

impl GenerateConfig {
    /// Create a config that generates all components.
    pub fn all() -> Self {
        Self {
            sdk_types: true,
            parsers: true,
            vm_handler: true,
            bytecode: true,
            computed: true,
            resolvers: true,
            handlers: true,
            spec_fn: true,
        }
    }
}

/// Context for code generation, including the IDL spec.
pub struct GenerateContext<'a> {
    /// The parsed IDL specification (required for SDK/parser generation)
    pub idl: &'a IdlSpec,
    /// The program ID string
    pub program_id: &'a str,
}

/// Generate all code from a SerializableStreamSpec.
///
/// This is the main entry point for unified code generation. Both `#[arete]`
/// and `#[ast_spec]` macros use this function to ensure identical output.
///
/// # Arguments
///
/// * `ast` - The serializable stream specification
/// * `module_name` - The name of the generated module
/// * `ctx` - Generation context including IDL spec
/// * `config` - Configuration controlling which components to generate
///
/// # Returns
///
/// A `TokenStream` containing all generated code wrapped in a module.
pub fn generate_all_from_spec(
    ast: &SerializableStreamSpec,
    module_name: &syn::Ident,
    ctx: &GenerateContext,
    config: &GenerateConfig,
) -> TokenStream {
    let entity_name = &ast.state_name;
    let state_name_ident = format_ident!("{}State", entity_name);

    // Get program name from IDL
    let program_name = ctx.idl.get_name();
    let state_enum_name = format!("{}State", crate::parse::idl::to_pascal_case(program_name));
    let instruction_enum_name = format!(
        "{}Instruction",
        crate::parse::idl::to_pascal_case(program_name)
    );

    // Generate each component based on config
    let sdk_module_name = format!("{}_sdk", program_name);
    let sdk_types = if config.sdk_types {
        generate_sdk_from_idl(ctx.idl, &sdk_module_name)
    } else {
        quote! {}
    };

    let parsers = if config.parsers {
        generate_parsers_from_idl(ctx.idl, ctx.program_id, &sdk_module_name)
    } else {
        quote! {}
    };

    let registries = if config.resolvers {
        generate_resolver_registries(&ast.resolver_hooks, &ast.instruction_hooks, Some(ctx.idl))
    } else {
        quote! {}
    };

    let vm_handler = if config.vm_handler {
        generate_vm_handler(&state_enum_name, &instruction_enum_name, entity_name)
    } else {
        quote! {}
    };

    let bytecode = if config.bytecode {
        generate_bytecode_from_spec(ast)
    } else {
        quote! {}
    };

    let computed = if config.computed {
        generate_computed_evaluator(&ast.computed_field_specs)
    } else {
        quote! {}
    };

    let spec_fn = if config.spec_fn {
        let runtime_config = RuntimeGenConfig::for_generate_all();
        generate_spec_function(
            &state_enum_name,
            &instruction_enum_name,
            program_name,
            &runtime_config,
        )
    } else {
        quote! {}
    };

    let (handler_fns, handler_calls) = if config.handlers {
        generate_handlers_from_specs(&ast.handlers, entity_name, &state_name_ident)
    } else {
        (Vec::new(), Vec::new())
    };

    // Combine all generated code into a module
    quote! {
        pub mod #module_name {
            #sdk_types
            #parsers
            #registries
            #vm_handler
            #bytecode
            #computed
            #spec_fn

            // Handler functions generated from AST
            #(#handler_fns)*

            /// Returns all handler calls for spec creation
            pub fn handler_calls() -> Vec<arete::runtime::arete_interpreter::ast::TypedHandlerSpec<#state_name_ident>> {
                vec![#(#handler_calls),*]
            }
        }
    }
}

/// Generate code from AST without wrapping in a module.
///
/// This variant is useful when you want to embed the generated code
/// into an existing module structure (like arete does).
///
/// # Arguments
///
/// * `ast` - The serializable stream specification
/// * `ctx` - Generation context including IDL spec
/// * `config` - Configuration controlling which components to generate
///
/// # Returns
///
/// A tuple of `TokenStream`s for each component category.
#[allow(dead_code)]
pub fn generate_components_from_spec(
    ast: &SerializableStreamSpec,
    ctx: &GenerateContext,
    config: &GenerateConfig,
) -> GeneratedComponents {
    let entity_name = &ast.state_name;
    let state_name_ident = format_ident!("{}State", entity_name);

    // Get program name from IDL
    let program_name = ctx.idl.get_name();
    let state_enum_name = format!("{}State", crate::parse::idl::to_pascal_case(program_name));
    let instruction_enum_name = format!(
        "{}Instruction",
        crate::parse::idl::to_pascal_case(program_name)
    );

    let sdk_module_name = format!("{}_sdk", program_name);
    GeneratedComponents {
        sdk_types: if config.sdk_types {
            generate_sdk_from_idl(ctx.idl, &sdk_module_name)
        } else {
            quote! {}
        },
        parsers: if config.parsers {
            generate_parsers_from_idl(ctx.idl, ctx.program_id, &sdk_module_name)
        } else {
            quote! {}
        },
        registries: if config.resolvers {
            generate_resolver_registries(&ast.resolver_hooks, &ast.instruction_hooks, Some(ctx.idl))
        } else {
            quote! {}
        },
        vm_handler: if config.vm_handler {
            generate_vm_handler(&state_enum_name, &instruction_enum_name, entity_name)
        } else {
            quote! {}
        },
        bytecode: if config.bytecode {
            generate_bytecode_from_spec(ast)
        } else {
            quote! {}
        },
        computed: if config.computed {
            generate_computed_evaluator(&ast.computed_field_specs)
        } else {
            quote! {}
        },
        spec_fn: if config.spec_fn {
            let runtime_config = RuntimeGenConfig::for_generate_all();
            generate_spec_function(
                &state_enum_name,
                &instruction_enum_name,
                program_name,
                &runtime_config,
            )
        } else {
            quote! {}
        },
        handler_fns: if config.handlers {
            let (fns, _) =
                generate_handlers_from_specs(&ast.handlers, entity_name, &state_name_ident);
            fns
        } else {
            Vec::new()
        },
        handler_calls: if config.handlers {
            let (_, calls) =
                generate_handlers_from_specs(&ast.handlers, entity_name, &state_name_ident);
            calls
        } else {
            Vec::new()
        },
        state_name_ident,
    }
}

/// Generated code components that can be assembled into a module.
#[allow(dead_code)]
pub struct GeneratedComponents {
    pub sdk_types: TokenStream,
    pub parsers: TokenStream,
    pub registries: TokenStream,
    pub vm_handler: TokenStream,
    pub bytecode: TokenStream,
    pub computed: TokenStream,
    pub spec_fn: TokenStream,
    pub handler_fns: Vec<TokenStream>,
    pub handler_calls: Vec<TokenStream>,
    pub state_name_ident: syn::Ident,
}
