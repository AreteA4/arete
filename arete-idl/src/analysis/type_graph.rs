//! Type graph analysis — extracts pubkey field references from IDL type definitions.

use crate::types::{IdlSpec, IdlType, IdlTypeDefKind};
use crate::utils::to_pascal_case;

/// A reference to a pubkey field within a type definition.
#[derive(Debug, Clone)]
pub struct PubkeyFieldRef {
    /// The field name (e.g. "lb_pair", "owner").
    pub field_name: String,
    /// Inferred target account type, matched by converting the field name
    /// (or field name stripped of `_id`/`_key` suffix) to PascalCase and
    /// comparing against account type names.
    pub likely_target: Option<String>,
}

/// A node in the type graph representing a type that contains pubkey fields.
#[derive(Debug, Clone)]
pub struct TypeNode {
    /// The type name (e.g. "Position", "LbPair").
    pub type_name: String,
    /// All pubkey fields found in this type.
    pub pubkey_fields: Vec<PubkeyFieldRef>,
}

/// Extract a type graph from an IDL spec.
///
/// Scans all type definitions (`idl.types`) for struct fields of type `pubkey`.
/// For each pubkey field, attempts to infer the target account type by matching
/// the field name (or field name with `_id`/`_key` suffix stripped) against
/// account type names (from `idl.accounts`).
///
/// Only types with at least one pubkey field are included in the result.
pub fn extract_type_graph(idl: &IdlSpec) -> Vec<TypeNode> {
    let account_names: Vec<&str> = idl.accounts.iter().map(|a| a.name.as_str()).collect();

    let mut nodes = Vec::new();

    for type_def in &idl.types {
        let fields = match &type_def.type_def {
            IdlTypeDefKind::Struct { fields, .. } => fields,
            _ => continue,
        };

        let pubkey_fields: Vec<PubkeyFieldRef> = fields
            .iter()
            .filter(|f| is_pubkey_type(&f.type_))
            .map(|f| {
                let likely_target = infer_target(&f.name, &account_names);
                PubkeyFieldRef {
                    field_name: f.name.clone(),
                    likely_target,
                }
            })
            .collect();

        if !pubkey_fields.is_empty() {
            nodes.push(TypeNode {
                type_name: type_def.name.clone(),
                pubkey_fields,
            });
        }
    }

    nodes
}

/// Check if an `IdlType` represents a public key.
fn is_pubkey_type(ty: &IdlType) -> bool {
    matches!(ty, IdlType::Simple(s) if s == "pubkey" || s == "publicKey")
}

/// Attempt to match a field name to an account type name.
///
/// Strategy: convert the field name (and variants with `_id`/`_key` stripped)
/// to PascalCase and compare against known account type names (case-insensitive).
fn infer_target(field_name: &str, account_names: &[&str]) -> Option<String> {
    let candidates = stripped_candidates(field_name);

    for candidate in &candidates {
        let pascal = to_pascal_case(candidate);
        for &acct in account_names {
            if acct.eq_ignore_ascii_case(&pascal) {
                return Some(acct.to_string());
            }
        }
    }

    None
}

/// Generate candidate base names by stripping common suffixes.
fn stripped_candidates(field_name: &str) -> Vec<&str> {
    let mut candidates = vec![field_name];

    for suffix in &["_id", "_key"] {
        if let Some(stripped) = field_name.strip_suffix(suffix) {
            if !stripped.is_empty() {
                candidates.push(stripped);
            }
        }
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_idl_file;
    use std::path::PathBuf;

    fn meteora_fixture() -> IdlSpec {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meteora_dlmm.json");
        parse_idl_file(&path).expect("should parse meteora_dlmm.json")
    }

    #[test]
    fn test_type_graph() {
        let idl = meteora_fixture();
        let graph = extract_type_graph(&idl);

        // Should have at least one TypeNode
        assert!(
            !graph.is_empty(),
            "should extract type nodes with pubkey fields"
        );

        // Find the Position type
        let position = graph.iter().find(|n| n.type_name == "Position");
        assert!(
            position.is_some(),
            "Position type should be in the type graph"
        );
        let position = position.unwrap();

        // Position should have lb_pair as a pubkey field
        let lb_pair_field = position
            .pubkey_fields
            .iter()
            .find(|f| f.field_name == "lb_pair");
        assert!(
            lb_pair_field.is_some(),
            "Position should have lb_pair pubkey field"
        );
        assert_eq!(
            lb_pair_field.unwrap().likely_target.as_deref(),
            Some("LbPair"),
            "lb_pair should resolve to LbPair account type"
        );

        // Position should also have owner as a pubkey field
        let owner_field = position
            .pubkey_fields
            .iter()
            .find(|f| f.field_name == "owner");
        assert!(
            owner_field.is_some(),
            "Position should have owner pubkey field"
        );

        // Print summary for evidence
        println!("Type graph nodes: {}", graph.len());
        for node in &graph {
            println!(
                "  {} — pubkey fields: {:?}",
                node.type_name,
                node.pubkey_fields
                    .iter()
                    .map(|f| format!(
                        "{} -> {}",
                        f.field_name,
                        f.likely_target.as_deref().unwrap_or("?")
                    ))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn test_is_pubkey_type() {
        assert!(is_pubkey_type(&IdlType::Simple("pubkey".to_string())));
        assert!(is_pubkey_type(&IdlType::Simple("publicKey".to_string())));
        assert!(!is_pubkey_type(&IdlType::Simple("u64".to_string())));
        assert!(!is_pubkey_type(&IdlType::Simple("bool".to_string())));
    }

    #[test]
    fn test_stripped_candidates() {
        assert_eq!(stripped_candidates("lb_pair"), vec!["lb_pair"]);
        assert_eq!(stripped_candidates("pool_id"), vec!["pool_id", "pool"]);
        assert_eq!(stripped_candidates("mint_key"), vec!["mint_key", "mint"]);
        assert_eq!(stripped_candidates("_id"), vec!["_id"]);
    }

    #[test]
    fn test_infer_target() {
        let accounts = vec!["LbPair", "Position", "BinArray"];

        assert_eq!(
            infer_target("lb_pair", &accounts),
            Some("LbPair".to_string())
        );
        assert_eq!(
            infer_target("position", &accounts),
            Some("Position".to_string())
        );
        assert_eq!(
            infer_target("bin_array_id", &accounts),
            Some("BinArray".to_string()),
            "should match after stripping _id suffix"
        );
        assert_eq!(
            infer_target("unknown_field", &accounts),
            None,
            "should return None for non-matching fields"
        );
    }
}
