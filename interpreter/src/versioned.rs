//! Versioned AST loader with automatic migration support.
//!
//! This module provides:
//! - Version detection from raw JSON
//! - Deserialization routing to the correct version
//! - Automatic migration to the latest AST format
//!
//! # Usage
//!
//! ```rust,ignore
//! use hyperstack_interpreter::versioned::{load_stack_spec, load_stream_spec};
//!
//! let stack = load_stack_spec(&json_string)?;
//! let stream = load_stream_spec(&json_string)?;
//! ```

use serde::Deserialize;
use serde_json::Value;
use std::fmt;

use crate::ast::{SerializableStackSpec, SerializableStreamSpec, CURRENT_AST_VERSION};

/// Error type for versioned AST loading failures.
#[derive(Debug, Clone)]
pub enum VersionedLoadError {
    /// The JSON could not be parsed
    InvalidJson(String),
    /// The AST version is not supported
    UnsupportedVersion(String),
    /// The AST structure is invalid for the detected version
    InvalidStructure(String),
}

impl fmt::Display for VersionedLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionedLoadError::InvalidJson(msg) => {
                write!(f, "Invalid JSON: {}", msg)
            }
            VersionedLoadError::UnsupportedVersion(version) => {
                write!(
                    f,
                    "Unsupported AST version: {}. Latest supported version: {}. \
                     Older versions are supported via automatic migration.",
                    version, CURRENT_AST_VERSION
                )
            }
            VersionedLoadError::InvalidStructure(msg) => {
                write!(f, "Invalid AST structure: {}", msg)
            }
        }
    }
}

impl std::error::Error for VersionedLoadError {}

/// Load a stack spec from JSON with automatic version detection and migration.
///
/// This function:
/// 1. Detects the AST version from the JSON
/// 2. Deserializes the appropriate version
/// 3. Migrates to the latest format if needed
///
/// # Arguments
///
/// * `json` - The JSON string containing the AST
///
/// # Returns
///
/// The deserialized and migrated `SerializableStackSpec`
///
/// # Example
///
/// ```rust,ignore
/// let json = std::fs::read_to_string("MyStack.stack.json")?;
/// let spec = load_stack_spec(&json)?;
/// ```
pub fn load_stack_spec(json: &str) -> Result<SerializableStackSpec, VersionedLoadError> {
    // Parse raw JSON to detect version
    let raw: Value =
        serde_json::from_str(json).map_err(|e| VersionedLoadError::InvalidJson(e.to_string()))?;

    // Extract version - default to "0.0.1" if not present (backwards compatibility)
    let version = raw
        .get("ast_version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.1");

    // Route to appropriate deserializer based on version
    match version {
        v if v == CURRENT_AST_VERSION => {
            // Current version - deserialize directly
            serde_json::from_value::<SerializableStackSpec>(raw)
                .map_err(|e| VersionedLoadError::InvalidStructure(e.to_string()))
        }
        // Add migration arms for old versions here, e.g.:
        // "0.0.1" => { migrate_v1_to_latest(raw) }
        _ => {
            // Unknown version
            Err(VersionedLoadError::UnsupportedVersion(version.to_string()))
        }
    }
}

/// Load a stream spec from JSON with automatic version detection and migration.
///
/// Similar to `load_stack_spec` but for entity/stream specs.
///
/// # Arguments
///
/// * `json` - The JSON string containing the AST
///
/// # Returns
///
/// The deserialized and migrated `SerializableStreamSpec`
pub fn load_stream_spec(json: &str) -> Result<SerializableStreamSpec, VersionedLoadError> {
    // Parse raw JSON to detect version
    let raw: Value =
        serde_json::from_str(json).map_err(|e| VersionedLoadError::InvalidJson(e.to_string()))?;

    // Extract version - default to "0.0.1" if not present (backwards compatibility)
    let version = raw
        .get("ast_version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.1");

    // Route to appropriate deserializer based on version
    match version {
        v if v == CURRENT_AST_VERSION => {
            // Current version - deserialize directly
            serde_json::from_value::<SerializableStreamSpec>(raw)
                .map_err(|e| VersionedLoadError::InvalidStructure(e.to_string()))
        }
        // Add migration arms for old versions here, e.g.:
        // "0.0.1" => { migrate_v1_to_latest(raw) }
        _ => {
            // Unknown version
            Err(VersionedLoadError::UnsupportedVersion(version.to_string()))
        }
    }
}

/// Versioned wrapper for SerializableStackSpec.
///
/// This enum allows deserializing multiple AST versions and then
/// converting them to the latest format via `into_latest()`.
/// Note: Only Deserialize is derived to avoid duplicate `ast_version` keys
/// (the inner struct already has this field, and we only use this for loading).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "ast_version")]
pub enum VersionedStackSpec {
    #[serde(rename = "0.0.1")]
    V1(SerializableStackSpec),
}

impl VersionedStackSpec {
    /// Convert the versioned spec to the latest format.
    pub fn into_latest(self) -> SerializableStackSpec {
        match self {
            VersionedStackSpec::V1(spec) => spec,
        }
    }
}

/// Versioned wrapper for SerializableStreamSpec.
///
/// This enum allows deserializing multiple AST versions and then
/// converting them to the latest format via `into_latest()`.
/// Note: Only Deserialize is derived to avoid duplicate `ast_version` keys
/// (the inner struct already has this field, and we only use this for loading).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "ast_version")]
pub enum VersionedStreamSpec {
    #[serde(rename = "0.0.1")]
    V1(SerializableStreamSpec),
}

impl VersionedStreamSpec {
    /// Convert the versioned spec to the latest format.
    pub fn into_latest(self) -> SerializableStreamSpec {
        match self {
            VersionedStreamSpec::V1(spec) => spec,
        }
    }
}

/// Detect the AST version from a JSON string without full deserialization.
///
/// This is useful for logging, debugging, or routing decisions.
///
/// # Arguments
///
/// * `json` - The JSON string containing the AST
///
/// # Returns
///
/// The detected version string, or "unknown" if it cannot be determined.
///
/// # Example
///
/// ```rust,ignore
/// let version = detect_ast_version(&json)?;
/// println!("AST version: {}", version);
/// ```
pub fn detect_ast_version(json: &str) -> Result<String, VersionedLoadError> {
    let raw: Value =
        serde_json::from_str(json).map_err(|e| VersionedLoadError::InvalidJson(e.to_string()))?;

    Ok(raw
        .get("ast_version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "0.0.1".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_stack_spec_v1() {
        let json = r#"
        {
            "ast_version": "0.0.1",
            "stack_name": "TestStack",
            "program_ids": [],
            "idls": [],
            "entities": [],
            "pdas": {},
            "instructions": []
        }
        "#;

        let result = load_stack_spec(json);
        assert!(result.is_ok());
        let spec = result.unwrap();
        assert_eq!(spec.stack_name, "TestStack");
        assert_eq!(spec.ast_version, "0.0.1");
    }

    #[test]
    fn test_load_stack_spec_no_version_defaults_to_v1() {
        // Test backwards compatibility - no ast_version field should default to 0.0.1
        let json = r#"
        {
            "stack_name": "TestStack",
            "program_ids": [],
            "idls": [],
            "entities": [],
            "pdas": {},
            "instructions": []
        }
        "#;

        let result = load_stack_spec(json);
        assert!(result.is_ok());
        let spec = result.unwrap();
        assert_eq!(spec.stack_name, "TestStack");
        assert_eq!(spec.ast_version, "0.0.1");
    }

    #[test]
    fn test_load_stack_spec_unsupported_version() {
        let json = r#"
        {
            "ast_version": "99.0.0",
            "stack_name": "TestStack",
            "program_ids": [],
            "idls": [],
            "entities": [],
            "pdas": {},
            "instructions": []
        }
        "#;

        let result = load_stack_spec(json);
        assert!(result.is_err());
        match result.unwrap_err() {
            VersionedLoadError::UnsupportedVersion(v) => assert_eq!(v, "99.0.0"),
            _ => panic!("Expected UnsupportedVersion error"),
        }
    }

    #[test]
    fn test_load_stream_spec_v1() {
        let json = r#"
        {
            "ast_version": "0.0.1",
            "state_name": "TestEntity",
            "identity": {"primary_keys": ["id"], "lookup_indexes": []},
            "handlers": [],
            "sections": [],
            "field_mappings": {},
            "resolver_hooks": [],
            "instruction_hooks": [],
            "resolver_specs": [],
            "computed_fields": [],
            "computed_field_specs": [],
            "views": []
        }
        "#;

        let result = load_stream_spec(json);
        assert!(result.is_ok());
        let spec = result.unwrap();
        assert_eq!(spec.state_name, "TestEntity");
        assert_eq!(spec.ast_version, "0.0.1");
    }

    #[test]
    fn test_load_stream_spec_no_version_defaults_to_v1() {
        // Test backwards compatibility - no ast_version field should default to 0.0.1
        let json = r#"
        {
            "state_name": "TestEntity",
            "identity": {"primary_keys": ["id"], "lookup_indexes": []},
            "handlers": [],
            "sections": [],
            "field_mappings": {},
            "resolver_hooks": [],
            "instruction_hooks": [],
            "resolver_specs": [],
            "computed_fields": [],
            "computed_field_specs": [],
            "views": []
        }
        "#;

        let result = load_stream_spec(json);
        assert!(result.is_ok());
        let spec = result.unwrap();
        assert_eq!(spec.state_name, "TestEntity");
        assert_eq!(spec.ast_version, "0.0.1");
    }

    #[test]
    fn test_load_stream_spec_unsupported_version() {
        let json = r#"
        {
            "ast_version": "99.0.0",
            "state_name": "TestEntity",
            "identity": {"primary_keys": ["id"], "lookup_indexes": []},
            "handlers": [],
            "sections": [],
            "field_mappings": {},
            "resolver_hooks": [],
            "instruction_hooks": [],
            "resolver_specs": [],
            "computed_fields": [],
            "computed_field_specs": [],
            "views": []
        }
        "#;

        let result = load_stream_spec(json);
        assert!(result.is_err());
        match result.unwrap_err() {
            VersionedLoadError::UnsupportedVersion(v) => assert_eq!(v, "99.0.0"),
            _ => panic!("Expected UnsupportedVersion error"),
        }
    }

    #[test]
    fn test_detect_ast_version() {
        let json = r#"{"ast_version": "0.0.1", "stack_name": "Test"}"#;
        assert_eq!(detect_ast_version(json).unwrap(), "0.0.1");

        let json_no_version = r#"{"stack_name": "Test"}"#;
        assert_eq!(detect_ast_version(json_no_version).unwrap(), "0.0.1");
    }

    /// Verifies that the AST version constant matches the hyperstack-macros crate.
    /// This test ensures both crates stay in sync.
    #[test]
    fn test_ast_version_sync_with_macros() {
        // Read the hyperstack-macros' types.rs file
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let macros_types_path = std::path::Path::new(&manifest_dir)
            .join("..") // Go up to workspace root
            .join("hyperstack-macros")
            .join("src")
            .join("ast")
            .join("types.rs");

        // Verify the file exists before attempting to read
        assert!(
            macros_types_path.exists(),
            "Cannot find hyperstack-macros source file at {:?}. \
             This test requires the source tree to be available.",
            macros_types_path
        );

        let content = std::fs::read_to_string(&macros_types_path)
            .expect("Failed to read hyperstack-macros/src/ast/types.rs");

        // Parse the CURRENT_AST_VERSION constant
        let version_line = content
            .lines()
            .find(|line| line.contains("pub const CURRENT_AST_VERSION"))
            .expect("CURRENT_AST_VERSION not found in hyperstack-macros");

        let version_str = version_line
            .split('"')
            .nth(1)
            .expect("Failed to parse version string");

        assert_eq!(
            version_str, CURRENT_AST_VERSION,
            "AST version mismatch! interpreter has '{}', hyperstack-macros has '{}'. \
             Both crates must have the same CURRENT_AST_VERSION. \
             Update both files when bumping the version.",
            CURRENT_AST_VERSION, version_str
        );
    }
}
