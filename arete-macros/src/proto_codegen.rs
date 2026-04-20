use crate::parse::proto::ProtoAnalysis;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

fn snake_case_to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

fn package_to_module_name(package: &str) -> String {
    let last_segment = package.split('.').next_back().unwrap_or(package);
    format!("{}_proto", last_segment)
}

pub fn generate_proto_module_declarations(
    proto_analyses: &[(String, ProtoAnalysis)],
) -> TokenStream {
    let mut module_declarations = Vec::new();

    for (_path, analysis) in proto_analyses {
        let module_name = format_ident!("{}", package_to_module_name(&analysis.package));
        let package_name = &analysis.package;

        module_declarations.push(quote! {
            pub mod #module_name {
                tonic::include_proto!(#package_name);
            }
        });
    }

    quote! {
        #(#module_declarations)*
    }
}

pub fn generate_proto_decoders(proto_analyses: &[(String, ProtoAnalysis)]) -> TokenStream {
    let mut decoder_functions = Vec::new();

    for (_path, analysis) in proto_analyses {
        let module_name = format_ident!("{}", package_to_module_name(&analysis.package));

        for oneof_type in &analysis.oneof_types {
            let message_name_ident = format_ident!("{}", oneof_type.message_name);
            let message_name_snake = to_snake_case(&oneof_type.message_name);
            let oneof_field_name_ident = format_ident!("{}", &oneof_type.oneof_field);
            let oneof_enum_name = snake_case_to_pascal_case(&oneof_type.oneof_field);
            let oneof_enum_ident = format_ident!("{}", oneof_enum_name);

            let mut match_arms = Vec::new();

            for variant in &oneof_type.variants {
                let variant_name_ident =
                    format_ident!("{}", snake_case_to_pascal_case(&variant.field_name));
                let type_name = format!("{}State", variant.type_name);

                match_arms.push(quote! {
                    Some(#oneof_enum_ident::#variant_name_ident(state)) => {
                        Ok((serde_json::to_value(&state)?, #type_name.to_string()))
                    }
                });
            }

            let decoder_fn_name = format_ident!(
                "decode_{}_{}",
                analysis.package.replace('.', "_"),
                to_snake_case(&oneof_type.message_name)
            );

            let message_name_snake_ident = format_ident!("{}", message_name_snake);

            decoder_functions.push(quote! {
                fn #decoder_fn_name(bytes: &[u8]) -> Result<(serde_json::Value, String), Box<dyn std::error::Error>> {
                    use #module_name::{#message_name_ident, #message_name_snake_ident::#oneof_enum_ident};
                    use prost::Message;

                    let msg = #message_name_ident::decode(bytes)?;

                    match msg.#oneof_field_name_ident {
                        #(#match_arms)*
                        _ => Err("Unsupported message type".into()),
                    }
                }
            });
        }
    }

    quote! {
        #(#decoder_functions)*
    }
}

fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    let mut prev_is_lower = false;

    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 && prev_is_lower {
                result.push('_');
            }
            result.push(ch.to_lowercase().next().unwrap());
            prev_is_lower = false;
        } else {
            result.push(ch);
            prev_is_lower = ch.is_lowercase();
        }
    }

    result
}

pub fn generate_proto_router_setup(proto_analyses: &[(String, ProtoAnalysis)]) -> TokenStream {
    let mut registrations = Vec::new();

    for (_path, analysis) in proto_analyses {
        for oneof_type in &analysis.oneof_types {
            let type_url = format!("/{}.{}", analysis.package, oneof_type.message_name);
            let decoder_fn_name = format_ident!(
                "decode_{}_{}",
                analysis.package.replace('.', "_"),
                to_snake_case(&oneof_type.message_name)
            );

            registrations.push(quote! {
                router.register(
                    #type_url.to_string(),
                    #decoder_fn_name as arete::runtime::arete_interpreter::proto_router::ProtoDecoder,
                );
            });
        }
    }

    quote! {
        fn setup_proto_router() -> arete::runtime::arete_interpreter::proto_router::ProtoRouter {
            let mut router = arete::runtime::arete_interpreter::proto_router::ProtoRouter::new();

            #(#registrations)*

            router
        }
    }
}
