//! Bytecode creation from AST specifications.

use proc_macro2::TokenStream;
use quote::quote;

use crate::ast::SerializableStreamSpec;

pub fn generate_bytecode_from_spec(spec: &SerializableStreamSpec) -> TokenStream {
    let spec_json = match serde_json::to_string(spec) {
        Ok(json) => json,
        Err(error) => {
            let error_message = format!("failed to serialize embedded AST JSON: {error}");
            return quote! {
                compile_error!(#error_message);
            };
        }
    };
    let entity_name = &spec.state_name;

    quote! {
        pub fn create_multi_entity_bytecode() -> hyperstack::runtime::hyperstack_interpreter::compiler::MultiEntityBytecode {
            let ast_json = #spec_json;
            let spec: hyperstack::runtime::hyperstack_interpreter::ast::SerializableStreamSpec = hyperstack::runtime::serde_json::from_str(ast_json)
                .expect("Failed to parse embedded AST JSON");

            let typed_spec = hyperstack::runtime::hyperstack_interpreter::ast::TypedStreamSpec::<hyperstack::runtime::serde_json::Value>::from_serializable(spec);

            hyperstack::runtime::hyperstack_interpreter::compiler::MultiEntityBytecode::from_single(
                #entity_name.to_string(),
                typed_spec,
                0,
            )
        }

        /// Extract view definitions from the embedded AST specification.
        pub fn get_view_definitions() -> Vec<hyperstack::runtime::hyperstack_interpreter::ast::ViewDef> {
            let ast_json = #spec_json;
            let spec: hyperstack::runtime::hyperstack_interpreter::ast::SerializableStreamSpec = hyperstack::runtime::serde_json::from_str(ast_json)
                .expect("Failed to parse embedded AST JSON");
            spec.views
        }
    }
}
