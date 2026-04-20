//! AST JSON file loading and deserialization.
//!
//! This module provides functions to load pre-serialized AST JSON files
//! for the `#[ast_spec]` macro.
//!
//! Note: These functions are currently unused but are kept for future use
//! when runtime AST loading is needed.

#![allow(dead_code)]

use std::path::Path;

use super::types::SerializableStreamSpec;

/// Error type for AST loading failures.
#[derive(Debug)]
pub enum AstLoadError {
    EnvVarNotSet,
    FileReadError { path: String, error: String },
    ParseError(String),
}

impl std::fmt::Display for AstLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AstLoadError::EnvVarNotSet => write!(f, "CARGO_MANIFEST_DIR not set"),
            AstLoadError::FileReadError { path, error } => {
                write!(f, "Failed to read AST file {:?}: {}", path, error)
            }
            AstLoadError::ParseError(e) => write!(f, "Failed to parse AST JSON: {}", e),
        }
    }
}

impl std::error::Error for AstLoadError {}

/// Load and parse an AST JSON file.
///
/// The path is resolved relative to `CARGO_MANIFEST_DIR`.
///
/// # Arguments
///
/// * `ast_path` - Relative path to the AST JSON file from the crate root.
///
/// # Returns
///
/// The deserialized `SerializableStreamSpec` or an error.
///
/// # Example
///
/// ```ignore
/// let spec = load_ast_from_file("spec.ast.json")?;
/// ```
pub fn load_ast_from_file(ast_path: &str) -> Result<SerializableStreamSpec, AstLoadError> {
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").map_err(|_| AstLoadError::EnvVarNotSet)?;

    let full_path = Path::new(&manifest_dir).join(ast_path);

    let content = std::fs::read_to_string(&full_path).map_err(|e| AstLoadError::FileReadError {
        path: full_path.display().to_string(),
        error: e.to_string(),
    })?;

    serde_json::from_str(&content).map_err(|e| AstLoadError::ParseError(e.to_string()))
}

/// Load an AST from the `.arete` directory by entity name.
///
/// This is a convenience function that constructs the standard path
/// `.arete/{entity_name}.ast.json`.
///
/// # Arguments
///
/// * `entity_name` - Name of the entity (e.g., "PumpfunToken")
///
/// # Returns
///
/// The deserialized `SerializableStreamSpec` or an error.
pub fn load_ast_by_entity_name(entity_name: &str) -> Result<SerializableStreamSpec, AstLoadError> {
    let ast_path = format!(".arete/{}.ast.json", entity_name);
    load_ast_from_file(&ast_path)
}
