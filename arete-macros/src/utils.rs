//! Utility functions shared across the transform-macros crate.
//!
//! This module contains small helper functions used by multiple modules
//! throughout the crate, including string case conversion and path manipulation.

#![allow(dead_code)]

use syn::{Path, Type};

/// Convert a string to snake_case.
///
/// Handles paths like "module::Type" by extracting the last segment first.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(to_snake_case("MyTypeName"), "my_type_name");
/// assert_eq!(to_snake_case("module::MyType"), "my_type");
/// ```
pub fn to_snake_case(s: &str) -> String {
    s.split("::")
        .last()
        .unwrap_or(s)
        .chars()
        .enumerate()
        .flat_map(|(i, c)| {
            if c.is_uppercase() && i > 0 {
                vec!['_', c.to_lowercase().next().unwrap()]
            } else {
                vec![c.to_lowercase().next().unwrap()]
            }
        })
        .collect()
}

/// Convert a string to PascalCase.
///
/// Splits on underscores and capitalizes the first letter of each word.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(to_pascal_case("my_type_name"), "MyTypeName");
/// ```
pub fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut c = word.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect()
}

/// Convert a syn::Path to a string representation.
///
/// Joins path segments with "::" separator.
///
/// # Examples
///
/// ```ignore
/// let path: syn::Path = syn::parse_str("foo::bar::Baz").unwrap();
/// assert_eq!(path_to_string(&path), "foo::bar::Baz");
/// ```
pub fn path_to_string(path: &Path) -> String {
    path.segments
        .iter()
        .map(|seg| seg.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

/// Check if a type is a primitive or common wrapper type.
///
/// Returns true for numeric types, bool, String, Option, and Vec.
/// These types don't represent nested structs that need special processing.
pub fn is_primitive_or_wrapper(ty: &Type) -> bool {
    match ty {
        Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                let type_name = segment.ident.to_string();
                matches!(
                    type_name.as_str(),
                    "u8" | "u16"
                        | "u32"
                        | "u64"
                        | "u128"
                        | "i8"
                        | "i16"
                        | "i32"
                        | "i64"
                        | "i128"
                        | "f32"
                        | "f64"
                        | "bool"
                        | "String"
                        | "Option"
                        | "Vec"
                )
            } else {
                false
            }
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("MyTypeName"), "my_type_name");
        assert_eq!(to_snake_case("HTTPServer"), "h_t_t_p_server");
        assert_eq!(to_snake_case("module::MyType"), "my_type");
    }

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("my_type_name"), "MyTypeName");
        assert_eq!(to_pascal_case("hello"), "Hello");
    }
}
