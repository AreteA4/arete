//! IDL parsing utilities

use crate::types::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub fn parse_idl_file<P: AsRef<Path>>(path: P) -> Result<IdlSpec, String> {
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read IDL file {:?}: {}", path.as_ref(), e))?;

    parse_idl_content(&content)
}

pub fn parse_idl_content(content: &str) -> Result<IdlSpec, String> {
    match serde_json::from_str(content) {
        Ok(idl) => Ok(idl),
        Err(parse_error) => {
            let codama_root: CodamaRoot = serde_json::from_str(content)
                .map_err(|_| format!("Failed to parse IDL JSON: {}", parse_error))?;
            codama_root_to_idl_spec(&codama_root)
        }
    }
}

fn codama_root_to_idl_spec(root: &CodamaRoot) -> Result<IdlSpec, String> {
    if root.kind != "rootNode" {
        return Err(format!(
            "unsupported Codama root kind '{}': expected rootNode",
            root.kind
        ));
    }

    let account_discriminators = root
        .program
        .defined_types
        .iter()
        .find(|defined_type| defined_type.name == "accountDiscriminator")
        .map(codama_account_discriminators)
        .transpose()?
        .unwrap_or_default();

    Ok(IdlSpec {
        version: root.program.version.clone(),
        name: Some(root.program.name.clone()),
        address: Some(root.program.public_key.clone()),
        instructions: root
            .program
            .instructions
            .iter()
            .map(codama_instruction_to_idl)
            .collect::<Result<Vec<_>, _>>()?,
        accounts: root
            .program
            .accounts
            .iter()
            .map(|account| codama_account_to_idl(account, &account_discriminators))
            .collect::<Result<Vec<_>, _>>()?,
        types: root
            .program
            .defined_types
            .iter()
            .map(codama_defined_type_to_idl)
            .collect::<Result<Vec<_>, _>>()?,
        events: Vec::new(),
        errors: root
            .program
            .errors
            .iter()
            .map(|error| IdlError {
                code: error.code,
                name: error.name.clone(),
                msg: Some(error.message.clone()),
            })
            .collect(),
        constants: Vec::new(),
        metadata: Some(IdlMetadata {
            name: Some(root.program.name.clone()),
            version: root.program.version.clone(),
            address: Some(root.program.public_key.clone()),
            spec: Some("codama".to_string()),
            description: None,
            origin: Some("codama-rootNode".to_string()),
        }),
    })
}

fn codama_account_discriminators(
    defined_type: &CodamaDefinedType,
) -> Result<HashMap<String, u8>, String> {
    let CodamaTypeNode::Enum { variants } = &defined_type.type_ else {
        return Err(format!(
            "Codama accountDiscriminator must be an enum, got {:?}",
            defined_type.type_
        ));
    };

    variants
        .iter()
        .map(|variant| {
            let discriminator = variant.discriminator.ok_or_else(|| {
                format!(
                    "Codama accountDiscriminator variant '{}' is missing a discriminator",
                    variant.name
                )
            })?;
            let discriminator = u8::try_from(discriminator).map_err(|_| {
                format!(
                    "Codama accountDiscriminator variant '{}' uses unsupported discriminator {}",
                    variant.name, discriminator
                )
            })?;
            Ok((variant.name.clone(), discriminator))
        })
        .collect()
}

fn codama_account_to_idl(
    account: &CodamaAccount,
    discriminators: &HashMap<String, u8>,
) -> Result<IdlAccount, String> {
    Ok(IdlAccount {
        name: account.name.clone(),
        discriminator: discriminators
            .get(&account.name)
            .copied()
            .map(|value| vec![value])
            .unwrap_or_default(),
        docs: Vec::new(),
        type_def: Some(codama_type_def_kind_to_idl(&account.data)?),
    })
}

fn codama_defined_type_to_idl(defined_type: &CodamaDefinedType) -> Result<IdlTypeDef, String> {
    Ok(IdlTypeDef {
        name: defined_type.name.clone(),
        docs: Vec::new(),
        serialization: None,
        repr: None,
        type_def: codama_type_def_kind_to_idl(&defined_type.type_)?,
    })
}

fn codama_instruction_to_idl(instruction: &CodamaInstruction) -> Result<IdlInstruction, String> {
    let discriminant = instruction
        .arguments
        .iter()
        .find(|argument| argument.name == "discriminator")
        .and_then(|argument| match &argument.default_value {
            Some(CodamaValueNode::Number { number }) => Some(SteelDiscriminant {
                type_: argument.number_format().unwrap_or("u8").to_string(),
                value: *number,
            }),
            _ => None,
        });

    Ok(IdlInstruction {
        name: instruction.name.clone(),
        discriminator: Vec::new(),
        discriminant,
        docs: Vec::new(),
        accounts: instruction
            .accounts
            .iter()
            .map(|account| IdlAccountArg {
                name: account.name.clone(),
                is_mut: account.is_writable,
                is_signer: account.is_signer,
                address: account.default_public_key(),
                optional: false,
                docs: account.docs.clone(),
                pda: None,
            })
            .collect(),
        args: instruction
            .arguments
            .iter()
            .filter(|argument| argument.name != "discriminator")
            .map(codama_field_to_idl)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn codama_type_def_kind_to_idl(type_node: &CodamaTypeNode) -> Result<IdlTypeDefKind, String> {
    match type_node {
        CodamaTypeNode::Struct { fields } => Ok(IdlTypeDefKind::Struct {
            kind: "struct".to_string(),
            fields: fields
                .iter()
                .map(codama_field_to_idl)
                .collect::<Result<Vec<_>, _>>()?,
        }),
        CodamaTypeNode::Enum { variants } => {
            if let Some(variant) = variants.iter().find(|variant| !variant.fields.is_empty()) {
                return Err(format!(
                    "Codama enum variant '{}' carries fields, which are not supported by arete-idl enums",
                    variant.name
                ));
            }

            Ok(IdlTypeDefKind::Enum {
                kind: "enum".to_string(),
                variants: variants
                    .iter()
                    .map(|variant| IdlEnumVariant {
                        name: variant.name.clone(),
                    })
                    .collect(),
            })
        }
        other => Err(format!(
            "Codama type node {:?} is not a type definition",
            other
        )),
    }
}

fn codama_field_to_idl(field: &CodamaFieldNode) -> Result<IdlField, String> {
    Ok(IdlField {
        name: field.name.clone(),
        type_: codama_type_to_idl(&field.type_)?,
    })
}

fn codama_type_to_idl(type_node: &CodamaTypeNode) -> Result<IdlType, String> {
    match type_node {
        CodamaTypeNode::Number { format } => Ok(IdlType::Simple(format.clone())),
        CodamaTypeNode::PublicKey {} => Ok(IdlType::Simple("publicKey".to_string())),
        CodamaTypeNode::String {} => Ok(IdlType::Simple("string".to_string())),
        CodamaTypeNode::DefinedTypeLink { name } => Ok(IdlType::Defined(IdlTypeDefined {
            defined: IdlTypeDefinedInner::Simple(name.clone()),
        })),
        CodamaTypeNode::Array { item, count } => Ok(IdlType::Array(IdlTypeArray {
            array: vec![
                IdlTypeArrayElement::Nested(codama_type_to_idl(item)?),
                IdlTypeArrayElement::Size(count.value()),
            ],
        })),
        CodamaTypeNode::FixedSize { size, type_ } => {
            let element_type = match type_.as_ref() {
                CodamaTypeNode::String {} => IdlType::Simple("u8".to_string()),
                other => codama_type_to_idl(other)?,
            };
            Ok(IdlType::Array(IdlTypeArray {
                array: vec![
                    IdlTypeArrayElement::Nested(element_type),
                    IdlTypeArrayElement::Size(*size),
                ],
            }))
        }
        other => Err(format!("unsupported Codama field type {:?}", other)),
    }
}

#[derive(Debug, Deserialize)]
struct CodamaRoot {
    kind: String,
    program: CodamaProgram,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodamaProgram {
    name: String,
    public_key: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    accounts: Vec<CodamaAccount>,
    #[serde(default)]
    defined_types: Vec<CodamaDefinedType>,
    #[serde(default)]
    errors: Vec<CodamaError>,
    #[serde(default)]
    instructions: Vec<CodamaInstruction>,
}

#[derive(Debug, Deserialize)]
struct CodamaAccount {
    name: String,
    data: CodamaTypeNode,
}

#[derive(Debug, Deserialize)]
struct CodamaDefinedType {
    name: String,
    #[serde(rename = "type")]
    type_: CodamaTypeNode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodamaInstruction {
    name: String,
    #[serde(default)]
    accounts: Vec<CodamaInstructionAccount>,
    #[serde(default)]
    arguments: Vec<CodamaFieldNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodamaInstructionAccount {
    name: String,
    #[serde(default)]
    is_signer: bool,
    #[serde(default)]
    is_writable: bool,
    #[serde(default)]
    docs: Vec<String>,
    #[serde(default)]
    default_value: Option<CodamaValueNode>,
}

impl CodamaInstructionAccount {
    fn default_public_key(&self) -> Option<String> {
        match &self.default_value {
            Some(CodamaValueNode::PublicKey { public_key }) => Some(public_key.clone()),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodamaFieldNode {
    name: String,
    #[serde(rename = "type")]
    type_: CodamaTypeNode,
    #[serde(default)]
    default_value: Option<CodamaValueNode>,
}

impl CodamaFieldNode {
    fn number_format(&self) -> Option<&str> {
        match &self.type_ {
            CodamaTypeNode::Number { format } => Some(format.as_str()),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind")]
enum CodamaTypeNode {
    #[serde(rename = "numberTypeNode")]
    Number { format: String },
    #[serde(rename = "publicKeyTypeNode")]
    PublicKey {},
    #[serde(rename = "stringTypeNode")]
    String {},
    #[serde(rename = "definedTypeLinkNode")]
    DefinedTypeLink { name: String },
    #[serde(rename = "arrayTypeNode")]
    Array {
        item: Box<CodamaTypeNode>,
        count: CodamaCountNode,
    },
    #[serde(rename = "fixedSizeTypeNode")]
    FixedSize {
        size: u32,
        #[serde(rename = "type")]
        type_: Box<CodamaTypeNode>,
    },
    #[serde(rename = "structTypeNode")]
    Struct { fields: Vec<CodamaFieldNode> },
    #[serde(rename = "enumTypeNode")]
    Enum { variants: Vec<CodamaEnumVariant> },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind")]
enum CodamaCountNode {
    #[serde(rename = "fixedCountNode")]
    Fixed { value: u32 },
}

impl CodamaCountNode {
    fn value(&self) -> u32 {
        match self {
            Self::Fixed { value } => *value,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CodamaEnumVariant {
    name: String,
    #[serde(default)]
    discriminator: Option<u64>,
    #[serde(default)]
    fields: Vec<CodamaFieldNode>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind")]
enum CodamaValueNode {
    #[serde(rename = "numberValueNode")]
    Number { number: u64 },
    #[serde(rename = "publicKeyValueNode")]
    PublicKey {
        #[serde(rename = "publicKey")]
        public_key: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct CodamaError {
    code: u32,
    name: String,
    message: String,
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

    #[test]
    fn test_codama_root_node_parses() {
        let json = r#"{
            "kind": "rootNode",
            "program": {
                "kind": "programNode",
                "name": "subscriptions",
                "publicKey": "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44",
                "version": "0.1.0",
                "accounts": [
                    {
                        "kind": "accountNode",
                        "name": "subscriptionAuthority",
                        "data": {
                            "kind": "structTypeNode",
                            "fields": [
                                {
                                    "kind": "structFieldTypeNode",
                                    "name": "owner",
                                    "type": { "kind": "publicKeyTypeNode" }
                                }
                            ]
                        }
                    }
                ],
                "definedTypes": [
                    {
                        "kind": "definedTypeNode",
                        "name": "accountDiscriminator",
                        "type": {
                            "kind": "enumTypeNode",
                            "variants": [
                                {
                                    "kind": "enumEmptyVariantTypeNode",
                                    "name": "subscriptionAuthority",
                                    "discriminator": 0
                                }
                            ]
                        }
                    },
                    {
                        "kind": "definedTypeNode",
                        "name": "transferData",
                        "type": {
                            "kind": "structTypeNode",
                            "fields": [
                                {
                                    "kind": "structFieldTypeNode",
                                    "name": "amount",
                                    "type": {
                                        "kind": "numberTypeNode",
                                        "format": "u64",
                                        "endian": "le"
                                    }
                                }
                            ]
                        }
                    }
                ],
                "errors": [
                    {
                        "kind": "errorNode",
                        "code": 100,
                        "name": "bad",
                        "message": "Bad"
                    }
                ],
                "instructions": [
                    {
                        "kind": "instructionNode",
                        "name": "transfer",
                        "accounts": [
                            {
                                "kind": "instructionAccountNode",
                                "name": "owner",
                                "isSigner": true,
                                "isWritable": true
                            }
                        ],
                        "arguments": [
                            {
                                "kind": "instructionArgumentNode",
                                "name": "discriminator",
                                "defaultValue": {
                                    "kind": "numberValueNode",
                                    "number": 4
                                },
                                "type": {
                                    "kind": "numberTypeNode",
                                    "format": "u8",
                                    "endian": "le"
                                }
                            },
                            {
                                "kind": "instructionArgumentNode",
                                "name": "transferData",
                                "type": {
                                    "kind": "definedTypeLinkNode",
                                    "name": "transferData"
                                }
                            }
                        ]
                    }
                ]
            }
        }"#;

        let idl = parse_idl_content(json).expect("Codama root-node IDL should parse");

        assert_eq!(idl.get_name(), "subscriptions");
        assert_eq!(
            idl.address.as_deref(),
            Some("De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44")
        );
        assert_eq!(idl.instructions.len(), 1);
        assert_eq!(idl.instructions[0].name, "transfer");
        assert_eq!(idl.instructions[0].get_discriminator(), vec![4]);
        assert_eq!(idl.instructions[0].args.len(), 1);
        assert_eq!(idl.accounts.len(), 1);
        assert_eq!(idl.accounts[0].name, "subscriptionAuthority");
        assert_eq!(idl.accounts[0].get_discriminator(), vec![0]);
        assert_eq!(idl.types.len(), 2);
        assert_eq!(idl.errors.len(), 1);
    }
}
