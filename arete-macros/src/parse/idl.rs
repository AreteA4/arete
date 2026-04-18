//! IDL file parsing.
//!
//! Re-exports IDL types from the `arete-idl` library and provides
//! macros-specific code generation helpers.

#![allow(dead_code)]
#![allow(unused_imports)]

// Re-export all IDL types from the arete-idl library
pub use arete_idl::discriminator::*;
pub use arete_idl::parse::*;
pub use arete_idl::types::*;

/// Convert an IDL type to a Rust type string for Borsh-serialized accounts.
pub fn to_rust_type_string(idl_type: &IdlType) -> String {
    match idl_type {
        IdlType::Simple(s) => map_simple_type(s),
        IdlType::Array(arr) => {
            if arr.array.len() == 2 {
                match (&arr.array[0], &arr.array[1]) {
                    (IdlTypeArrayElement::Type(t), IdlTypeArrayElement::Size(size)) => {
                        format!("[{}; {}]", map_simple_type(t), size)
                    }
                    (IdlTypeArrayElement::Nested(nested), IdlTypeArrayElement::Size(size)) => {
                        let inner = to_rust_type_string(nested);
                        format!("[{}; {}]", inner, size)
                    }
                    _ => "Vec<u8>".to_string(),
                }
            } else {
                "Vec<u8>".to_string()
            }
        }
        IdlType::Defined(def) => match &def.defined {
            IdlTypeDefinedInner::Named { name } => name.clone(),
            IdlTypeDefinedInner::Simple(s) => s.clone(),
        },
        IdlType::Option(opt) => {
            let inner_type = to_rust_type_string(&opt.option);
            format!("Option<{}>", inner_type)
        }
        IdlType::Vec(vec) => {
            let inner_type = to_rust_type_string(&vec.vec);
            format!("Vec<{}>", inner_type)
        }
        IdlType::HashMap(hm) => {
            let key_type = to_rust_type_string(&hm.hash_map.0);
            let val_type = to_rust_type_string(&hm.hash_map.1);
            format!("std::collections::HashMap<{}, {}>", key_type, val_type)
        }
    }
}

/// Convert an IDL type to a Rust type string for bytemuck (zero-copy) accounts.
pub fn to_rust_type_string_bytemuck(idl_type: &IdlType) -> String {
    match idl_type {
        IdlType::Simple(s) => map_simple_type_bytemuck(s),
        IdlType::Array(arr) => {
            if arr.array.len() == 2 {
                match (&arr.array[0], &arr.array[1]) {
                    (IdlTypeArrayElement::Type(t), IdlTypeArrayElement::Size(size)) => {
                        format!("[{}; {}]", map_simple_type_bytemuck(t), size)
                    }
                    (IdlTypeArrayElement::Nested(nested), IdlTypeArrayElement::Size(size)) => {
                        let inner = to_rust_type_string_bytemuck(nested);
                        format!("[{}; {}]", inner, size)
                    }
                    _ => "Vec<u8>".to_string(),
                }
            } else {
                "Vec<u8>".to_string()
            }
        }
        IdlType::Defined(def) => match &def.defined {
            IdlTypeDefinedInner::Named { name } => name.clone(),
            IdlTypeDefinedInner::Simple(s) => s.clone(),
        },
        IdlType::Option(opt) => {
            let inner_type = to_rust_type_string_bytemuck(&opt.option);
            format!("Option<{}>", inner_type)
        }
        IdlType::Vec(vec) => {
            let inner_type = to_rust_type_string_bytemuck(&vec.vec);
            format!("Vec<{}>", inner_type)
        }
        IdlType::HashMap(hm) => {
            let key_type = to_rust_type_string_bytemuck(&hm.hash_map.0);
            let val_type = to_rust_type_string_bytemuck(&hm.hash_map.1);
            format!("std::collections::HashMap<{}, {}>", key_type, val_type)
        }
    }
}

fn map_simple_type(idl_type: &str) -> String {
    match idl_type {
        "u8" => "u8".to_string(),
        "u16" => "u16".to_string(),
        "u32" => "u32".to_string(),
        "u64" => "u64".to_string(),
        "u128" => "u128".to_string(),
        "i8" => "i8".to_string(),
        "i16" => "i16".to_string(),
        "i32" => "i32".to_string(),
        "i64" => "i64".to_string(),
        "i128" => "i128".to_string(),
        "bool" => "bool".to_string(),
        "string" => "String".to_string(),
        "publicKey" | "pubkey" => "solana_pubkey::Pubkey".to_string(),
        "bytes" => "Vec<u8>".to_string(),
        _ => idl_type.to_string(),
    }
}

fn map_simple_type_bytemuck(idl_type: &str) -> String {
    match idl_type {
        "u8" => "u8".to_string(),
        "u16" => "u16".to_string(),
        "u32" => "u32".to_string(),
        "u64" => "u64".to_string(),
        "u128" => "u128".to_string(),
        "i8" => "i8".to_string(),
        "i16" => "i16".to_string(),
        "i32" => "i32".to_string(),
        "i64" => "i64".to_string(),
        "i128" => "i128".to_string(),
        // bool is NOT Pod-safe in bytemuck (not all bit patterns are valid).
        // Map to u8 instead: 0 = false, non-zero = true.
        "bool" => "u8".to_string(),
        "string" => "String".to_string(),
        "publicKey" | "pubkey" => "[u8; 32]".to_string(),
        "bytes" => "Vec<u8>".to_string(),
        _ => idl_type.to_string(),
    }
}
