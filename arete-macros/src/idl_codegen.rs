use crate::parse::idl::*;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::HashSet;

fn collect_account_names(accounts: &[IdlAccount]) -> HashSet<String> {
    accounts.iter().map(|a| a.name.clone()).collect()
}

fn qualify_defined_name(
    name: &str,
    account_names: &HashSet<String>,
    in_accounts_module: bool,
) -> String {
    if account_names.contains(name) && !in_accounts_module {
        format!("accounts::{}", name)
    } else {
        name.to_string()
    }
}

fn is_bytemuck_serialization(serialization: &Option<IdlSerialization>) -> bool {
    matches!(
        serialization,
        Some(IdlSerialization::Bytemuck) | Some(IdlSerialization::BytemuckUnsafe)
    )
}

fn is_bytemuck_unsafe(serialization: &Option<IdlSerialization>) -> bool {
    matches!(serialization, Some(IdlSerialization::BytemuckUnsafe))
}

fn lookup_account_serialization<'a>(
    account_name: &str,
    types: &'a [IdlTypeDef],
) -> &'a Option<IdlSerialization> {
    types
        .iter()
        .find(|t| t.name == account_name)
        .map(|t| &t.serialization)
        .unwrap_or(&None)
}

fn resolve_type_string(
    idl_type: &IdlType,
    bytemuck: bool,
    account_names: &HashSet<String>,
    in_accounts_module: bool,
) -> String {
    let base = if bytemuck {
        crate::parse::idl::to_rust_type_string_bytemuck(idl_type)
    } else {
        crate::parse::idl::to_rust_type_string(idl_type)
    };
    match idl_type {
        IdlType::Defined(def) => {
            let name = match &def.defined {
                IdlTypeDefinedInner::Named { name } => name.as_str(),
                IdlTypeDefinedInner::Simple(s) => s.as_str(),
            };
            qualify_defined_name(name, account_names, in_accounts_module)
        }
        IdlType::Option(opt) => {
            let inner =
                resolve_type_string(&opt.option, bytemuck, account_names, in_accounts_module);
            format!("Option<{}>", inner)
        }
        IdlType::Vec(vec) => {
            let inner = resolve_type_string(&vec.vec, bytemuck, account_names, in_accounts_module);
            format!("Vec<{}>", inner)
        }
        IdlType::HashMap(hm) => {
            let key =
                resolve_type_string(&hm.hash_map.0, bytemuck, account_names, in_accounts_module);
            let val =
                resolve_type_string(&hm.hash_map.1, bytemuck, account_names, in_accounts_module);
            format!("std::collections::HashMap<{}, {}>", key, val)
        }
        _ => base,
    }
}

fn type_to_token_stream_bytemuck(idl_type: &IdlType) -> TokenStream {
    type_to_token_stream_resolved(idl_type, true, &HashSet::new(), true)
}

fn type_to_token_stream_resolved(
    idl_type: &IdlType,
    bytemuck: bool,
    account_names: &HashSet<String>,
    in_accounts_module: bool,
) -> TokenStream {
    let type_str = resolve_type_string(idl_type, bytemuck, account_names, in_accounts_module);
    type_str.parse().unwrap_or_else(|_| {
        quote! { arete::runtime::serde_json::Value }
    })
}

fn generate_bytemuck_field(field: &IdlField) -> TokenStream {
    let field_name = format_ident!("{}", to_snake_case(&field.name));
    let field_type = type_to_token_stream_bytemuck(&field.type_);
    quote! { pub #field_name: #field_type }
}

pub fn generate_sdk_types(idl: &IdlSpec, module_name: &str) -> TokenStream {
    let account_names = collect_account_names(&idl.accounts);
    let account_types = generate_account_types(&idl.accounts, &idl.types, &account_names);
    let instruction_types =
        generate_instruction_types(&idl.instructions, &idl.types, &account_names);
    let event_types = generate_event_types(&idl.events, &idl.types, &account_names);
    let custom_types = generate_custom_types(&idl.types, &account_names);
    let module_ident = format_ident!("{}", module_name);

    quote! {
        pub mod #module_ident {
            use borsh::{BorshDeserialize, BorshSerialize};

            pub mod types {
                use super::*;
                #custom_types
            }

            pub mod accounts {
                use super::*;
                use super::types::*;
                #account_types
            }

            pub mod instructions {
                use super::*;
                use super::types::*;
                #instruction_types
            }

            pub mod events {
                use super::*;
                use super::types::*;
                #event_types
            }
        }
    }
}

/// Maximum array size that serde's derive macro supports natively.
/// Arrays larger than this need a custom serde helper.
const SERDE_MAX_ARRAY_SIZE: u32 = 32;

/// Check if the outermost array dimension exceeds SERDE_MAX_ARRAY_SIZE.
/// Nested arrays are not inspected; only the outer length matters for the
/// `#[serde(with = "big_array")]` attribute.
fn is_large_array(idl_type: &IdlType) -> bool {
    if let IdlType::Array(arr) = idl_type {
        if arr.array.len() == 2 {
            if let IdlTypeArrayElement::Size(size) = &arr.array[1] {
                return *size > SERDE_MAX_ARRAY_SIZE;
            }
        }
    }
    false
}

fn generate_struct_fields(
    fields: &[IdlField],
    use_bytemuck: bool,
    account_names: &HashSet<String>,
    in_accounts_module: bool,
) -> Vec<TokenStream> {
    fields
        .iter()
        .map(|field| {
            if use_bytemuck {
                generate_bytemuck_field(field)
            } else {
                let field_name = format_ident!("{}", to_snake_case(&field.name));
                let field_type =
                    type_to_token_stream_in_module(&field.type_, account_names, in_accounts_module);

                // Add serde helper attribute for large arrays
                if is_large_array(&field.type_) {
                    quote! {
                        #[serde(with = "arete::runtime::serde_helpers::big_array")]
                        pub #field_name: #field_type
                    }
                } else {
                    quote! { pub #field_name: #field_type }
                }
            }
        })
        .collect()
}

fn array_inner_type(idl_type: &IdlTypeArray) -> Option<IdlType> {
    if idl_type.array.len() != 2 {
        return None;
    }

    match &idl_type.array[0] {
        IdlTypeArrayElement::Type(type_name) => Some(IdlType::Simple(type_name.clone())),
        IdlTypeArrayElement::Nested(nested) => Some(nested.clone()),
        IdlTypeArrayElement::Size(_) => None,
    }
}

fn generate_json_value_for_type(
    idl_type: &IdlType,
    value_expr: TokenStream,
    use_bytemuck: bool,
) -> TokenStream {
    match idl_type {
        IdlType::Simple(type_name) => match type_name.as_str() {
            "u128" | "i128" => {
                quote! { arete::runtime::serde_json::Value::String((#value_expr).to_string()) }
            }
            "pubkey" | "publicKey" => {
                if use_bytemuck {
                    quote! {
                        arete::runtime::serde_json::Value::String(
                            arete::runtime::bs58::encode(#value_expr).into_string()
                        )
                    }
                } else {
                    quote! { arete::runtime::serde_json::Value::String((#value_expr).to_string()) }
                }
            }
            "u8" | "u16" | "u32" | "u64" | "i8" | "i16" | "i32" | "i64" | "f32" | "f64"
            | "bool" | "string" | "bytes" => {
                quote! { arete::runtime::serde_json::json!(#value_expr) }
            }
            _ => quote! { (#value_expr).to_json_value() },
        },
        IdlType::Defined(_) => quote! { (#value_expr).to_json_value() },
        IdlType::Option(option_type) => {
            let inner_value =
                generate_json_value_for_type(&option_type.option, quote! { value }, use_bytemuck);
            quote! {
                match (#value_expr).as_ref() {
                    Some(value) => #inner_value,
                    None => arete::runtime::serde_json::Value::Null,
                }
            }
        }
        IdlType::Vec(vec_type) => {
            let inner_value =
                generate_json_value_for_type(&vec_type.vec, quote! { item }, use_bytemuck);
            quote! {
                arete::runtime::serde_json::Value::Array(
                    (#value_expr)
                        .iter()
                        .map(|item| #inner_value)
                        .collect::<Vec<_>>()
                )
            }
        }
        IdlType::Array(array_type) => {
            if let Some(inner_type) = array_inner_type(array_type) {
                let inner_value =
                    generate_json_value_for_type(&inner_type, quote! { item }, use_bytemuck);
                quote! {
                    arete::runtime::serde_json::Value::Array(
                        (#value_expr)
                            .iter()
                            .map(|item| #inner_value)
                            .collect::<Vec<_>>()
                    )
                }
            } else {
                quote! { arete::runtime::serde_json::json!(#value_expr) }
            }
        }
        IdlType::HashMap(hm) => {
            let val_value =
                generate_json_value_for_type(&hm.hash_map.1, quote! { v }, use_bytemuck);
            quote! {
                arete::runtime::serde_json::Value::Object(
                    (#value_expr)
                        .iter()
                        .map(|(k, v)| (k.to_string(), #val_value))
                        .collect::<arete::runtime::serde_json::Map<String, arete::runtime::serde_json::Value>>()
                )
            }
        }
    }
}

fn generate_struct_to_json_method(fields: &[IdlField], use_bytemuck: bool) -> TokenStream {
    generate_struct_to_json_method_inner(fields, use_bytemuck, false)
}

fn generate_struct_to_json_method_packed(fields: &[IdlField], use_bytemuck: bool) -> TokenStream {
    generate_struct_to_json_method_inner(fields, use_bytemuck, true)
}

fn generate_struct_to_json_method_inner(
    fields: &[IdlField],
    use_bytemuck: bool,
    is_packed: bool,
) -> TokenStream {
    let field_inserts = fields.iter().map(|field| {
        let field_ident = format_ident!("{}", to_snake_case(&field.name));
        let field_name = field_ident.to_string();

        if is_packed {
            // For packed structs, copy field to a local variable to avoid
            // creating unaligned references (which is UB in Rust).
            let local_var = format_ident!("_packed_{}", to_snake_case(&field.name));
            let field_value =
                generate_json_value_for_type(&field.type_, quote! { #local_var }, use_bytemuck);
            quote! {
                let #local_var = { self.#field_ident };
                object.insert(#field_name.to_string(), #field_value);
            }
        } else {
            let field_value = generate_json_value_for_type(
                &field.type_,
                quote! { self.#field_ident },
                use_bytemuck,
            );
            quote! {
                object.insert(#field_name.to_string(), #field_value);
            }
        }
    });

    quote! {
        pub fn to_json_value(&self) -> arete::runtime::serde_json::Value {
            let mut object = arete::runtime::serde_json::Map::new();
            #(#field_inserts)*
            arete::runtime::serde_json::Value::Object(object)
        }
    }
}

fn generate_tuple_struct_to_json_method(fields: &[IdlType], use_bytemuck: bool) -> TokenStream {
    let field_values = fields.iter().enumerate().map(|(index, field_type)| {
        let field_access: TokenStream = format!("self.{}", index)
            .parse()
            .expect("tuple field access must be valid");
        generate_json_value_for_type(field_type, field_access, use_bytemuck)
    });

    quote! {
        pub fn to_json_value(&self) -> arete::runtime::serde_json::Value {
            arete::runtime::serde_json::Value::Array(vec![#(#field_values),*])
        }
    }
}

fn generate_account_types(
    accounts: &[IdlAccount],
    types: &[IdlTypeDef],
    account_names: &HashSet<String>,
) -> TokenStream {
    let account_structs = accounts
        .iter()
        .map(|account| generate_account_type(account, types, account_names));

    quote! {
        #(#account_structs)*
    }
}

fn generate_account_type(
    account: &IdlAccount,
    types: &[IdlTypeDef],
    account_names: &HashSet<String>,
) -> TokenStream {
    let name = format_ident!("{}", account.name);
    let serialization = lookup_account_serialization(&account.name, types);
    let use_bytemuck = is_bytemuck_serialization(serialization);
    let use_unsafe = is_bytemuck_unsafe(serialization);

    // Check if the type definition specifies packed repr
    let is_packed = types
        .iter()
        .find(|t| t.name == account.name)
        .and_then(|t| t.repr.as_ref())
        .and_then(|r| r.packed)
        .unwrap_or(false);

    let idl_fields = if let Some(type_def) = &account.type_def {
        match type_def {
            IdlTypeDefKind::Struct { fields, .. } => fields.clone(),
            IdlTypeDefKind::TupleStruct { .. } | IdlTypeDefKind::Enum { .. } => vec![],
        }
    } else if let Some(type_def) = types.iter().find(|t| t.name == account.name) {
        match &type_def.type_def {
            IdlTypeDefKind::Struct { fields, .. } => fields.clone(),
            IdlTypeDefKind::TupleStruct { .. } | IdlTypeDefKind::Enum { .. } => vec![],
        }
    } else {
        vec![]
    };

    let fields = generate_struct_fields(&idl_fields, use_bytemuck, account_names, true);
    let to_json_method = if is_packed {
        generate_struct_to_json_method_packed(&idl_fields, use_bytemuck)
    } else {
        generate_struct_to_json_method(&idl_fields, use_bytemuck)
    };

    let discriminator = account.get_discriminator();
    let disc_array = quote! { [#(#discriminator),*] };

    if use_bytemuck {
        let bytemuck_try_from = quote! {
            impl #name {
                pub const DISCRIMINATOR: &'static [u8] = &#disc_array;

                pub fn try_from_bytes(data: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
                    if data.len() < Self::DISCRIMINATOR.len() {
                        return Err("Data too short for discriminator".into());
                    }
                    let body = &data[Self::DISCRIMINATOR.len()..];
                    let struct_size = std::mem::size_of::<Self>();
                    if body.len() < struct_size {
                        return Err(format!(
                            "Data too short for {}: need {} bytes, got {}",
                            stringify!(#name), struct_size, body.len()
                        ).into());
                    }
                    Ok(arete::runtime::bytemuck::pod_read_unaligned::<Self>(&body[..struct_size]))
                }

                #to_json_method
            }
        };

        if use_unsafe {
            // BytemuckUnsafe: use unsafe impl to bypass padding checks,
            // matching how the on-chain program was compiled.
            if is_packed {
                quote! {
                    #[derive(Debug, Copy, Clone)]
                    #[repr(C, packed)]
                    pub struct #name {
                        #(#fields),*
                    }

                    unsafe impl arete::runtime::bytemuck::Zeroable for #name {}
                    unsafe impl arete::runtime::bytemuck::Pod for #name {}

                    #bytemuck_try_from
                }
            } else {
                quote! {
                    #[derive(Debug, Copy, Clone)]
                    #[repr(C)]
                    pub struct #name {
                        #(#fields),*
                    }

                    unsafe impl arete::runtime::bytemuck::Zeroable for #name {}
                    unsafe impl arete::runtime::bytemuck::Pod for #name {}

                    #bytemuck_try_from
                }
            }
        } else {
            // Bytemuck (safe): use derive macros which validate no padding at compile time.
            // Note: packed structs cannot use derive Pod, so they must use unsafe impl above.
            quote! {
                #[derive(Debug, Copy, Clone, arete::runtime::bytemuck::Pod, arete::runtime::bytemuck::Zeroable)]
                #[bytemuck(crate = "arete::runtime::bytemuck")]
                #[repr(C)]
                pub struct #name {
                    #(#fields),*
                }

                #bytemuck_try_from
            }
        }
    } else {
        quote! {
            #[derive(Debug, Clone, BorshSerialize, BorshDeserialize, arete::runtime::serde::Serialize, arete::runtime::serde::Deserialize)]
            #[serde(crate = "arete::runtime::serde")]
            pub struct #name {
                #(#fields),*
            }

            impl #name {
                pub const DISCRIMINATOR: &'static [u8] = &#disc_array;

                pub fn try_from_bytes(data: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
                    if data.len() < Self::DISCRIMINATOR.len() {
                        return Err("Data too short for discriminator".into());
                    }
                    let mut reader = &data[Self::DISCRIMINATOR.len()..];
                    borsh::BorshDeserialize::deserialize_reader(&mut reader)
                        .map_err(|e| e.into())
                }

                #to_json_method
            }
        }
    }
}

fn generate_instruction_types(
    instructions: &[IdlInstruction],
    _types: &[IdlTypeDef],
    account_names: &HashSet<String>,
) -> TokenStream {
    let instruction_structs = instructions
        .iter()
        .map(|ix| generate_instruction_type(ix, account_names));

    quote! {
        #(#instruction_structs)*
    }
}

fn generate_instruction_type(
    instruction: &IdlInstruction,
    account_names: &HashSet<String>,
) -> TokenStream {
    let name = format_ident!("{}", to_pascal_case(&instruction.name));

    let discriminator = instruction.get_discriminator();
    let disc_array = quote! { [#(#discriminator),*] };

    let args_fields = instruction.args.iter().map(|arg| {
        let arg_name = format_ident!("{}", to_snake_case(&arg.name));
        let arg_type = type_to_token_stream_in_module(&arg.type_, account_names, false);
        quote! { pub #arg_name: #arg_type }
    });

    let to_json_method = generate_struct_to_json_method(&instruction.args, false);
    let derives = quote! { #[derive(Debug, Clone, BorshSerialize, BorshDeserialize)] };

    quote! {
        #derives
        pub struct #name {
            #(#args_fields),*
        }

        impl #name {
            pub const DISCRIMINATOR: &'static [u8] = &#disc_array;

            pub fn try_from_bytes(data: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
                let mut reader = data;
                match borsh::BorshDeserialize::deserialize_reader(&mut reader) {
                    Ok(v) => Ok(v),
                    Err(_) if !data.is_empty() => {
                        let mut padded = data.to_vec();
                        padded.resize(256, 0);
                        let mut reader = padded.as_slice();
                        borsh::BorshDeserialize::deserialize_reader(&mut reader)
                            .map_err(|e| e.into())
                    }
                    Err(e) => Err(e.into()),
                }
            }

            #to_json_method
        }
    }
}

fn generate_event_types(
    events: &[IdlEvent],
    types: &[IdlTypeDef],
    account_names: &HashSet<String>,
) -> TokenStream {
    let event_structs = events
        .iter()
        .map(|event| generate_event_type(event, types, account_names));

    quote! {
        #(#event_structs)*
    }
}

fn generate_event_type(
    event: &IdlEvent,
    types: &[IdlTypeDef],
    account_names: &HashSet<String>,
) -> TokenStream {
    let name = format_ident!("{}", event.name);

    let discriminator = event.get_discriminator();
    let disc_array = quote! { [#(#discriminator),*] };

    // Event fields come from the matching type definition in `types`
    let idl_fields: Vec<IdlField> = types
        .iter()
        .find(|t| t.name == event.name)
        .and_then(|t| match &t.type_def {
            IdlTypeDefKind::Struct { fields, .. } => Some(fields.clone()),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "Event '{}' has no matching type definition in the IDL `types` array",
                event.name
            )
        });

    let fields = generate_struct_fields(&idl_fields, false, account_names, false);
    let to_json_method = generate_struct_to_json_method(&idl_fields, false);

    quote! {
        #[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
        pub struct #name {
            #(#fields),*
        }

        impl #name {
            pub const DISCRIMINATOR: &'static [u8] = &#disc_array;

            /// Decode a CPI event from raw instruction data.
            /// Anchor CPI events: discriminator followed by Borsh-encoded payload.
            pub fn try_from_bytes(data: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
                if data.len() < Self::DISCRIMINATOR.len() {
                    return Err("Data too short for event discriminator".into());
                }
                if &data[..Self::DISCRIMINATOR.len()] != Self::DISCRIMINATOR {
                    return Err(format!(
                        "Discriminator mismatch: expected {:?}, got {:?}",
                        Self::DISCRIMINATOR, &data[..Self::DISCRIMINATOR.len()]
                    ).into());
                }
                let mut reader = &data[Self::DISCRIMINATOR.len()..];
                borsh::BorshDeserialize::deserialize_reader(&mut reader).map_err(|e| e.into())
            }

            #to_json_method
        }
    }
}

fn generate_custom_types(types: &[IdlTypeDef], account_names: &HashSet<String>) -> TokenStream {
    let type_defs = types.iter().map(|t| generate_custom_type(t, account_names));

    quote! {
        #(#type_defs)*
    }
}

fn generate_custom_type(type_def: &IdlTypeDef, account_names: &HashSet<String>) -> TokenStream {
    let name = format_ident!("{}", type_def.name);
    let use_bytemuck = is_bytemuck_serialization(&type_def.serialization);
    let use_unsafe = is_bytemuck_unsafe(&type_def.serialization);
    let is_packed = type_def
        .repr
        .as_ref()
        .and_then(|r| r.packed)
        .unwrap_or(false);

    match &type_def.type_def {
        IdlTypeDefKind::Struct { kind: _, fields } => {
            let struct_fields = generate_struct_fields(fields, use_bytemuck, account_names, false);
            let to_json_method = if is_packed {
                generate_struct_to_json_method_packed(fields, use_bytemuck)
            } else {
                generate_struct_to_json_method(fields, use_bytemuck)
            };

            if use_bytemuck {
                if use_unsafe {
                    let repr_attr = if is_packed {
                        quote! { #[repr(C, packed)] }
                    } else {
                        quote! { #[repr(C)] }
                    };
                    quote! {
                        #[derive(Debug, Copy, Clone)]
                        #repr_attr
                        pub struct #name {
                            #(#struct_fields),*
                        }

                        unsafe impl arete::runtime::bytemuck::Zeroable for #name {}
                        unsafe impl arete::runtime::bytemuck::Pod for #name {}

                        impl #name {
                            #to_json_method
                        }
                    }
                } else {
                    quote! {
                        #[derive(Debug, Copy, Clone, arete::runtime::bytemuck::Pod, arete::runtime::bytemuck::Zeroable)]
                        #[bytemuck(crate = "arete::runtime::bytemuck")]
                        #[repr(C)]
                        pub struct #name {
                            #(#struct_fields),*
                        }

                        impl #name {
                            #to_json_method
                        }
                    }
                }
            } else {
                quote! {
                    #[derive(Debug, Clone, BorshSerialize, BorshDeserialize, arete::runtime::serde::Serialize, arete::runtime::serde::Deserialize)]
                    #[serde(crate = "arete::runtime::serde")]
                    pub struct #name {
                        #(#struct_fields),*
                    }

                    impl #name {
                        #to_json_method
                    }
                }
            }
        }
        IdlTypeDefKind::TupleStruct { kind: _, fields } => {
            let tuple_fields = fields
                .iter()
                .map(|f| type_to_token_stream_in_module(f, account_names, false));
            let to_json_method = generate_tuple_struct_to_json_method(fields, use_bytemuck);

            quote! {
                #[derive(Debug, Clone, BorshSerialize, BorshDeserialize, arete::runtime::serde::Serialize, arete::runtime::serde::Deserialize)]
                #[serde(crate = "arete::runtime::serde")]
                pub struct #name(#(pub #tuple_fields),*);

                impl #name {
                    #to_json_method
                }
            }
        }
        IdlTypeDefKind::Enum { kind: _, variants } => {
            let enum_variants = variants.iter().enumerate().map(|(i, variant)| {
                let variant_name = format_ident!("{}", variant.name);
                if i == 0 {
                    quote! { #[default] #variant_name }
                } else {
                    quote! { #variant_name }
                }
            });

            let enum_to_json_arms = variants.iter().map(|variant| {
                let variant_name = format_ident!("{}", variant.name);
                let variant_value = variant.name.clone();

                quote! {
                    Self::#variant_name => arete::runtime::serde_json::Value::String(#variant_value.to_string())
                }
            });

            quote! {
                #[derive(Debug, Clone, Default, BorshSerialize, BorshDeserialize, arete::runtime::serde::Serialize, arete::runtime::serde::Deserialize)]
                #[serde(crate = "arete::runtime::serde")]
                pub enum #name {
                    #(#enum_variants),*
                }

                impl #name {
                    pub fn to_json_value(&self) -> arete::runtime::serde_json::Value {
                        match self {
                            #(#enum_to_json_arms),*
                        }
                    }
                }
            }
        }
    }
}

#[allow(dead_code)]
fn type_to_token_stream(idl_type: &IdlType) -> TokenStream {
    type_to_token_stream_resolved(idl_type, false, &HashSet::new(), true)
}

fn type_to_token_stream_in_module(
    idl_type: &IdlType,
    account_names: &HashSet<String>,
    in_accounts_module: bool,
) -> TokenStream {
    type_to_token_stream_resolved(idl_type, false, account_names, in_accounts_module)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_bytemuck_idl() -> IdlSpec {
        let json = r#"{
            "address": "TestBytemuckProgram111111111111111111111",
            "metadata": {
                "name": "test_bytemuck",
                "version": "0.1.0",
                "spec": "0.1.0"
            },
            "instructions": [],
            "accounts": [
                {
                    "name": "MyHeader",
                    "discriminator": [1, 2, 3, 4, 5, 6, 7, 8]
                },
                {
                    "name": "RegularAccount",
                    "discriminator": [10, 20, 30, 40, 50, 60, 70, 80]
                }
            ],
            "types": [
                {
                    "name": "MyHeader",
                    "serialization": "bytemuck",
                    "repr": { "kind": "c" },
                    "type": {
                        "kind": "struct",
                        "fields": [
                            { "name": "authority", "type": "pubkey" },
                            { "name": "total_fees", "type": "u128" },
                            { "name": "bump", "type": "u8" },
                            { "name": "is_active", "type": "bool" },
                            { "name": "_padding", "type": { "array": ["u8", 14] } }
                        ]
                    }
                },
                {
                    "name": "RegularAccount",
                    "type": {
                        "kind": "struct",
                        "fields": [
                            { "name": "owner", "type": "pubkey" },
                            { "name": "balance", "type": "u64" },
                            { "name": "is_initialized", "type": "bool" }
                        ]
                    }
                }
            ],
            "events": [],
            "errors": []
        }"#;
        parse_idl_content(json).expect("test IDL should parse")
    }

    #[test]
    fn test_bytemuck_idl_parses_serialization_field() {
        let idl = minimal_bytemuck_idl();
        let header = idl.types.iter().find(|t| t.name == "MyHeader").unwrap();
        assert_eq!(header.serialization, Some(IdlSerialization::Bytemuck));
        assert!(header.repr.is_some());
        assert_eq!(header.repr.as_ref().unwrap().kind, "c");

        let regular = idl
            .types
            .iter()
            .find(|t| t.name == "RegularAccount")
            .unwrap();
        assert_eq!(regular.serialization, None);
    }

    #[test]
    fn test_bytemuck_codegen_produces_pod_derives() {
        let idl = minimal_bytemuck_idl();
        let output = generate_sdk_types(&idl, "generated_sdk");
        let code = output.to_string();

        assert!(code.contains("Pod"), "bytemuck account should derive Pod");
        assert!(
            code.contains("Zeroable"),
            "bytemuck account should derive Zeroable"
        );
        assert!(
            code.contains("repr (C)"),
            "bytemuck account should have #[repr(C)]"
        );
    }

    #[test]
    fn test_bytemuck_maps_pubkey_to_byte_array() {
        let idl = minimal_bytemuck_idl();
        let output = generate_sdk_types(&idl, "generated_sdk");
        let code = output.to_string();

        assert!(
            code.contains("[u8 ; 32]"),
            "bytemuck pubkey should map to [u8; 32], got: {}",
            code
        );
    }

    #[test]
    fn test_bytemuck_maps_bool_to_u8() {
        let idl = minimal_bytemuck_idl();
        let output = generate_sdk_types(&idl, "generated_sdk");
        let code = output.to_string();

        assert!(
            !code.contains("pub is_active : bool"),
            "bytemuck bool should NOT remain bool"
        );
        assert!(
            code.contains("pub is_active : u8"),
            "bytemuck bool should map to u8, got: {}",
            code
        );
    }

    #[test]
    fn test_bytemuck_pubkey_to_json_uses_bs58() {
        let idl = minimal_bytemuck_idl();
        let output = generate_sdk_types(&idl, "generated_sdk");
        let code = output.to_string();

        assert!(
            code.contains("bs58 :: encode"),
            "bytemuck pubkey to_json_value should encode with bs58, got: {}",
            code
        );
    }

    #[test]
    fn test_regular_account_uses_borsh() {
        let idl = minimal_bytemuck_idl();
        let output = generate_sdk_types(&idl, "generated_sdk");
        let code = output.to_string();

        assert!(
            code.contains("BorshDeserialize"),
            "regular account should derive BorshDeserialize"
        );
    }

    #[test]
    fn test_regular_account_keeps_bool_as_bool() {
        let idl = minimal_bytemuck_idl();
        let output = generate_sdk_types(&idl, "generated_sdk");
        let code = output.to_string();

        assert!(
            code.contains("pub is_initialized : bool"),
            "regular account bool should stay bool, got: {}",
            code
        );
    }

    #[test]
    fn test_bytemuck_try_from_bytes_uses_bytemuck() {
        let idl = minimal_bytemuck_idl();
        let output = generate_sdk_types(&idl, "generated_sdk");
        let code = output.to_string();

        assert!(
            code.contains("pod_read_unaligned"),
            "bytemuck try_from_bytes should use bytemuck::pod_read_unaligned"
        );
    }

    #[test]
    fn test_regular_try_from_bytes_uses_borsh() {
        let idl = minimal_bytemuck_idl();
        let output = generate_sdk_types(&idl, "generated_sdk");
        let code = output.to_string();

        assert!(
            code.contains("deserialize_reader"),
            "regular try_from_bytes should use borsh::deserialize_reader"
        );
    }

    #[test]
    fn test_bytemuck_unsafe_variant_parses() {
        let json = r#"{
            "metadata": { "name": "test", "version": "0.1.0" },
            "instructions": [],
            "accounts": [],
            "types": [
                {
                    "name": "UnsafeHeader",
                    "serialization": "bytemuckunsafe",
                    "repr": { "kind": "c" },
                    "type": {
                        "kind": "struct",
                        "fields": [
                            { "name": "data", "type": "u64" }
                        ]
                    }
                }
            ],
            "events": [],
            "errors": []
        }"#;
        let idl = parse_idl_content(json).expect("bytemuckunsafe should parse");
        let t = &idl.types[0];
        assert_eq!(t.serialization, Some(IdlSerialization::BytemuckUnsafe));
    }

    fn bytemuck_unsafe_idl() -> IdlSpec {
        let json = r#"{
            "address": "TestUnsafeProgram111111111111111111111111",
            "metadata": {
                "name": "test_unsafe",
                "version": "0.1.0",
                "spec": "0.1.0"
            },
            "instructions": [],
            "accounts": [
                {
                    "name": "PaddedAccount",
                    "discriminator": [1, 2, 3, 4, 5, 6, 7, 8]
                }
            ],
            "types": [
                {
                    "name": "PaddedAccount",
                    "serialization": "bytemuckunsafe",
                    "repr": { "kind": "c" },
                    "type": {
                        "kind": "struct",
                        "fields": [
                            { "name": "authority", "type": "pubkey" },
                            { "name": "value", "type": "u64" },
                            { "name": "flag", "type": "u8" }
                        ]
                    }
                },
                {
                    "name": "SafeType",
                    "serialization": "bytemuck",
                    "repr": { "kind": "c" },
                    "type": {
                        "kind": "struct",
                        "fields": [
                            { "name": "data", "type": "u64" },
                            { "name": "padding", "type": { "array": ["u8", 8] } }
                        ]
                    }
                }
            ],
            "events": [],
            "errors": []
        }"#;
        parse_idl_content(json).expect("test IDL should parse")
    }

    #[test]
    fn test_bytemuck_unsafe_uses_unsafe_impl_not_derive() {
        let idl = bytemuck_unsafe_idl();
        let output = generate_sdk_types(&idl, "generated_sdk");
        let code = output.to_string();

        assert!(
            code.contains("unsafe impl arete :: runtime :: bytemuck :: Pod for PaddedAccount"),
            "bytemuckunsafe should emit unsafe impl Pod, got: {}",
            code
        );
        assert!(
            code.contains(
                "unsafe impl arete :: runtime :: bytemuck :: Zeroable for PaddedAccount"
            ),
            "bytemuckunsafe should emit unsafe impl Zeroable, got: {}",
            code
        );
    }

    #[test]
    fn test_bytemuck_unsafe_does_not_derive_pod() {
        let idl = bytemuck_unsafe_idl();
        let output = generate_sdk_types(&idl, "generated_sdk");
        let code = output.to_string();

        let parts: Vec<&str> = code.split("pub struct PaddedAccount").collect();
        assert!(parts.len() > 1, "PaddedAccount should exist in output");
        let before = parts[0];
        if let Some(pos) = before.rfind("derive") {
            let end = (pos + 200).min(before.len());
            let derive_block = &before[pos..end];
            assert!(
                !derive_block.contains("Pod"),
                "bytemuckunsafe should NOT derive Pod on the struct, got: {}",
                derive_block
            );
        }
    }

    #[test]
    fn test_bytemuck_safe_still_uses_derive() {
        let idl = bytemuck_unsafe_idl();
        let output = generate_sdk_types(&idl, "generated_sdk");
        let code = output.to_string();

        let safe_section = code.split("pub struct SafeType").collect::<Vec<_>>();
        assert!(safe_section.len() > 1, "SafeType should exist in output");
        let before_safe = safe_section[0];
        let last_chunk = &before_safe[before_safe.len().saturating_sub(300)..];
        assert!(
            last_chunk.contains("Pod"),
            "bytemuck (safe) should still derive Pod, got: {}",
            last_chunk
        );
    }

    #[test]
    fn test_bytemuck_unsafe_still_has_try_from_bytes() {
        let idl = bytemuck_unsafe_idl();
        let output = generate_sdk_types(&idl, "generated_sdk");
        let code = output.to_string();

        assert!(
            code.contains("pod_read_unaligned"),
            "bytemuckunsafe should still use pod_read_unaligned for deserialization"
        );
    }

    fn large_array_borsh_idl() -> IdlSpec {
        let json = r#"{
            "address": "TestLargeArrayProgram111111111111111111111",
            "metadata": {
                "name": "test_large_array",
                "version": "0.1.0",
                "spec": "0.1.0"
            },
            "instructions": [],
            "accounts": [
                {
                    "name": "LargeArrayAccount",
                    "discriminator": [1, 2, 3, 4, 5, 6, 7, 8]
                },
                {
                    "name": "SmallArrayAccount",
                    "discriminator": [10, 20, 30, 40, 50, 60, 70, 80]
                }
            ],
            "types": [
                {
                    "name": "LargeArrayAccount",
                    "type": {
                        "kind": "struct",
                        "fields": [
                            { "name": "owner", "type": "pubkey" },
                            { "name": "large_data", "type": { "array": ["u64", 70] } },
                            { "name": "small_data", "type": { "array": ["u8", 32] } }
                        ]
                    }
                },
                {
                    "name": "SmallArrayAccount",
                    "type": {
                        "kind": "struct",
                        "fields": [
                            { "name": "owner", "type": "pubkey" },
                            { "name": "data", "type": { "array": ["u8", 32] } }
                        ]
                    }
                }
            ],
            "events": [],
            "errors": []
        }"#;
        parse_idl_content(json).expect("test IDL should parse")
    }

    #[test]
    fn test_large_array_gets_serde_with_attribute() {
        let idl = large_array_borsh_idl();
        let output = generate_sdk_types(&idl, "generated_sdk");
        let code = output.to_string();

        // Large array (70 elements) should have serde(with = ...) attribute
        // The generated code: serde_helpers::big_array (no spaces around ::)
        assert!(
            code.contains("serde_helpers::big_array"),
            "large array should have serde(with) attribute for big_array helper, got: {}",
            code
        );

        // Verify the attribute is before large_data field
        let parts: Vec<&str> = code.split("pub large_data").collect();
        assert!(parts.len() > 1, "large_data field should exist");
        let before = &parts[0][parts[0].len().saturating_sub(150)..];
        assert!(
            before.contains("big_array"),
            "serde(with = big_array) should appear before large_data field, got context: {}",
            before
        );
    }

    #[test]
    fn test_small_array_no_serde_with_attribute() {
        let idl = large_array_borsh_idl();
        let output = generate_sdk_types(&idl, "generated_sdk");
        let code = output.to_string();

        // Small array (32 elements) should NOT have serde(with = ...) immediately before it
        // We need to check the immediate predecessor of small_data, not the whole preceding code
        // The pattern should be: pub large_data : [u64 ; 70] , pub small_data
        // NOT: # [serde (...)] pub small_data

        // Check that small_data field exists
        assert!(
            code.contains("pub small_data"),
            "small_data field should exist"
        );
        assert!(
            code.contains("pub data : [u8 ; 32]"),
            "SmallArrayAccount.data field should exist"
        );

        // Verify the SmallArrayAccount doesn't have big_array in its struct definition
        // Find just the SmallArrayAccount struct (second occurrence in types and accounts)
        let small_account_section = code
            .split("pub struct SmallArrayAccount")
            .nth(1)
            .expect("SmallArrayAccount should exist")
            .split("impl SmallArrayAccount")
            .next()
            .expect("impl block should exist");

        assert!(
            !small_account_section.contains("big_array"),
            "SmallArrayAccount should NOT have big_array attribute, got: {}",
            small_account_section
        );
    }

    #[test]
    fn test_is_large_array_detection() {
        // Test array > 32
        let large = IdlType::Array(IdlTypeArray {
            array: vec![
                IdlTypeArrayElement::Type("u64".to_string()),
                IdlTypeArrayElement::Size(70),
            ],
        });
        assert!(is_large_array(&large), "70-element array should be large");

        // Test array == 32 (boundary)
        let boundary = IdlType::Array(IdlTypeArray {
            array: vec![
                IdlTypeArrayElement::Type("u8".to_string()),
                IdlTypeArrayElement::Size(32),
            ],
        });
        assert!(
            !is_large_array(&boundary),
            "32-element array should NOT be large"
        );

        // Test array < 32
        let small = IdlType::Array(IdlTypeArray {
            array: vec![
                IdlTypeArrayElement::Type("u8".to_string()),
                IdlTypeArrayElement::Size(16),
            ],
        });
        assert!(
            !is_large_array(&small),
            "16-element array should NOT be large"
        );

        // Test non-array type
        let not_array = IdlType::Simple("u64".to_string());
        assert!(
            !is_large_array(&not_array),
            "non-array type should NOT be large"
        );
    }
}
