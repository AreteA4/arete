//! IDL parsing utilities

use crate::types::IdlSpec;
use std::fs;
use std::path::Path;

pub fn parse_idl_file<P: AsRef<Path>>(path: P) -> Result<IdlSpec, String> {
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read IDL file {:?}: {}", path.as_ref(), e))?;

    parse_idl_content(&content)
}

pub fn parse_idl_content(content: &str) -> Result<IdlSpec, String> {
    serde_json::from_str(content).map_err(|e| format!("Failed to parse IDL JSON: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discriminator::anchor_discriminator;
    use sha2::{Digest, Sha256};

    #[test]
    fn test_anchor_discriminator_known_values() {
        let disc = anchor_discriminator("global:initialize");
        assert_eq!(disc.len(), 8);
        assert_eq!(disc, &Sha256::digest(b"global:initialize")[..8]);
    }

    #[test]
    fn test_anchor_account_discriminator() {
        let disc = anchor_discriminator("account:LendingMarket");
        assert_eq!(disc.len(), 8);
        assert_eq!(disc, &Sha256::digest(b"account:LendingMarket")[..8]);
    }

    #[test]
    fn test_legacy_idl_parses_without_discriminator() {
        let json = r#"{
            "address": "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8",
            "version": "0.3.0",
            "name": "raydium_amm",
            "instructions": [
                {
                    "name": "initialize",
                    "accounts": [
                        { "name": "tokenProgram", "isMut": false, "isSigner": false }
                    ],
                    "args": [
                        { "name": "nonce", "type": "u8" }
                    ]
                }
            ],
            "accounts": [
                {
                    "name": "TargetOrders",
                    "type": {
                        "kind": "struct",
                        "fields": [
                            { "name": "owner", "type": { "array": ["u64", 4] } }
                        ]
                    }
                }
            ],
            "types": [],
            "errors": []
        }"#;
        let idl = parse_idl_content(json).expect("legacy IDL should parse");

        assert_eq!(idl.instructions.len(), 1);
        assert_eq!(idl.accounts.len(), 1);
        assert!(idl.accounts[0].discriminator.is_empty());
        assert!(idl.instructions[0].discriminator.is_empty());
        assert!(idl.instructions[0].discriminant.is_none());
    }

    #[test]
    fn test_legacy_instruction_computes_discriminator() {
        let json = r#"{
            "name": "raydium_amm",
            "instructions": [
                {
                    "name": "initialize",
                    "accounts": [],
                    "args": []
                }
            ],
            "accounts": [],
            "types": [],
            "errors": []
        }"#;
        let idl = parse_idl_content(json).unwrap();
        let disc = idl.instructions[0].get_discriminator();

        assert_eq!(disc.len(), 8);
        let expected = anchor_discriminator("global:initialize");
        assert_eq!(disc, expected);
    }

    #[test]
    fn test_legacy_account_computes_discriminator() {
        let json = r#"{
            "name": "test",
            "instructions": [],
            "accounts": [
                {
                    "name": "LendingMarket",
                    "type": { "kind": "struct", "fields": [] }
                }
            ],
            "types": [],
            "errors": []
        }"#;
        let idl = parse_idl_content(json).unwrap();
        let disc = idl.accounts[0].get_discriminator();

        assert_eq!(disc.len(), 8);
        let expected = anchor_discriminator("account:LendingMarket");
        assert_eq!(disc, expected);
    }

    #[test]
    fn test_explicit_discriminator_not_overridden() {
        let json = r#"{
            "name": "test",
            "instructions": [
                {
                    "name": "transfer",
                    "discriminator": [1, 2, 3, 4, 5, 6, 7, 8],
                    "accounts": [],
                    "args": []
                }
            ],
            "accounts": [
                {
                    "name": "TokenAccount",
                    "discriminator": [10, 20, 30, 40, 50, 60, 70, 80]
                }
            ],
            "types": [],
            "errors": []
        }"#;
        let idl = parse_idl_content(json).unwrap();

        assert_eq!(
            idl.instructions[0].get_discriminator(),
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert_eq!(
            idl.accounts[0].get_discriminator(),
            vec![10, 20, 30, 40, 50, 60, 70, 80]
        );
    }

    #[test]
    fn test_steel_discriminant_still_works() {
        let json = r#"{
            "name": "test",
            "instructions": [
                {
                    "name": "CreateMetadataAccount",
                    "accounts": [],
                    "args": [],
                    "discriminant": { "type": "u8", "value": 0 }
                },
                {
                    "name": "UpdateMetadataAccount",
                    "accounts": [],
                    "args": [],
                    "discriminant": { "type": "u8", "value": 1 }
                }
            ],
            "accounts": [],
            "types": [],
            "errors": []
        }"#;
        let idl = parse_idl_content(json).unwrap();

        assert_eq!(idl.instructions[0].get_discriminator(), vec![0]);
        assert_eq!(idl.instructions[1].get_discriminator(), vec![1]);
    }

    #[test]
    fn test_legacy_event_computes_discriminator() {
        let json = r#"{
            "name": "test",
            "instructions": [],
            "accounts": [],
            "types": [],
            "events": [
                { "name": "TransferEvent" }
            ],
            "errors": []
        }"#;
        let idl = parse_idl_content(json).unwrap();
        let disc = idl.events[0].get_discriminator();

        assert_eq!(disc.len(), 8);
        let expected = anchor_discriminator("event:TransferEvent");
        assert_eq!(disc, expected);
    }

    #[test]
    fn test_is_mut_is_signer_aliases() {
        let json = r#"{
            "name": "test",
            "instructions": [
                {
                    "name": "do_thing",
                    "accounts": [
                        { "name": "payer", "isMut": true, "isSigner": true },
                        { "name": "dest", "writable": true, "signer": false }
                    ],
                    "args": []
                }
            ],
            "accounts": [],
            "types": [],
            "errors": []
        }"#;
        let idl = parse_idl_content(json).unwrap();
        let accounts = &idl.instructions[0].accounts;

        assert!(accounts[0].is_mut);
        assert!(accounts[0].is_signer);
        assert!(accounts[1].is_mut);
        assert!(!accounts[1].is_signer);
    }

    #[test]
    fn test_constants() {
        let json = r#"{
            "address": "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo",
            "metadata": {
                "name": "lb_clmm",
                "version": "0.11.0",
                "spec": "0.1.0",
                "description": "Created with Anchor"
            },
            "instructions": [],
            "accounts": [],
            "types": [],
            "events": [],
            "errors": [],
            "constants": [
                {
                    "name": "BASIS_POINT_MAX",
                    "type": "i32",
                    "value": "10000"
                },
                {
                    "name": "MAX_BIN_PER_ARRAY",
                    "type": "u64",
                    "value": "70"
                }
            ]
        }"#;
        let idl = parse_idl_content(json).expect("IDL with constants should parse");

        assert_eq!(idl.constants.len(), 2);
        assert_eq!(idl.constants[0].name, "BASIS_POINT_MAX");
        assert_eq!(idl.constants[0].value, "10000");
        assert_eq!(idl.constants[1].name, "MAX_BIN_PER_ARRAY");
        assert_eq!(idl.constants[1].value, "70");
    }
}
