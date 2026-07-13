use crate::ast::{
    idl_type_snapshot_to_rust_string, AccountResolution, AmountDecimalsSource, IdlAccountSnapshot,
    IdlArrayElementSnapshot, IdlArrayTypeSnapshot, IdlDefinedInnerSnapshot, IdlDefinedTypeSnapshot,
    IdlEnumVariantFieldSnapshot, IdlEnumVariantSnapshot, IdlErrorSnapshot, IdlEventSnapshot,
    IdlFieldSnapshot, IdlHashMapTypeSnapshot, IdlInstructionAccountSnapshot,
    IdlInstructionSnapshot, IdlOptionTypeSnapshot, IdlSerializationSnapshot, IdlSnapshot,
    IdlTypeDefKindSnapshot, IdlTypeDefSnapshot, IdlTypeSnapshot, IdlVecTypeSnapshot,
    InstructionAccountDef, InstructionAmountHint, InstructionArgDef, InstructionDef, PdaDefinition,
    PdaSeedDef, SerializableStackSpec, CURRENT_AST_VERSION,
};
use arete_idl as idl_parser;
use std::collections::BTreeMap;

fn sanitize_identifier_segment(name: &str) -> String {
    let mut sanitized = String::new();

    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            sanitized.push(ch);
        } else if !sanitized.ends_with('_') {
            sanitized.push('_');
        }
    }

    let sanitized = sanitized.trim_matches('_').to_string();
    if sanitized.is_empty() {
        return "value".to_string();
    }
    if sanitized
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
    {
        return format!("_{}", sanitized);
    }
    sanitized
}

fn sanitize_identifier(name: &str) -> String {
    sanitize_identifier_segment(name)
}

fn sanitize_seed_path(path: &str) -> String {
    path.split('.')
        .map(sanitize_identifier_segment)
        .collect::<Vec<_>>()
        .join(".")
}

pub fn build_program_only_stack_spec_from_idl(
    idl: &idl_parser::IdlSpec,
    stack_name: &str,
) -> SerializableStackSpec {
    let snapshot = convert_idl_to_snapshot(idl);
    let program_id = snapshot.program_id.clone();
    let pdas = extract_pdas_from_idl(idl);
    let instructions = extract_instructions_from_idl(idl, &pdas);

    let mut grouped_pdas = BTreeMap::new();
    if !pdas.is_empty() {
        grouped_pdas.insert(snapshot.name.clone(), pdas);
    }

    SerializableStackSpec {
        ast_version: CURRENT_AST_VERSION.to_string(),
        stack_name: stack_name.to_string(),
        program_ids: program_id.into_iter().collect(),
        idls: vec![snapshot],
        entities: vec![],
        pdas: grouped_pdas,
        instructions,
        content_hash: None,
    }
    .with_content_hash()
}

pub fn convert_idl_to_snapshot(idl: &idl_parser::IdlSpec) -> IdlSnapshot {
    let mut types: Vec<IdlTypeDefSnapshot> = idl
        .types
        .iter()
        .map(|typedef| IdlTypeDefSnapshot {
            name: typedef.name.clone(),
            docs: typedef.docs.clone(),
            serialization: typedef.serialization.as_ref().map(|s| match s {
                idl_parser::IdlSerialization::Borsh => IdlSerializationSnapshot::Borsh,
                idl_parser::IdlSerialization::Bytemuck => IdlSerializationSnapshot::Bytemuck,
                idl_parser::IdlSerialization::BytemuckUnsafe => {
                    IdlSerializationSnapshot::BytemuckUnsafe
                }
            }),
            type_def: match &typedef.type_def {
                idl_parser::IdlTypeDefKind::Struct { kind, fields } => {
                    IdlTypeDefKindSnapshot::Struct {
                        kind: kind.clone(),
                        fields: fields
                            .iter()
                            .map(|field| IdlFieldSnapshot {
                                name: field.name.clone(),
                                type_: convert_idl_type(&field.type_),
                                amount_hint: field.amount_hint.clone(),
                            })
                            .collect(),
                    }
                }
                idl_parser::IdlTypeDefKind::TupleStruct { kind, fields } => {
                    IdlTypeDefKindSnapshot::TupleStruct {
                        kind: kind.clone(),
                        fields: fields.iter().map(convert_idl_type).collect(),
                    }
                }
                idl_parser::IdlTypeDefKind::Enum { kind, variants } => {
                    IdlTypeDefKindSnapshot::Enum {
                        kind: kind.clone(),
                        variants: variants.iter().map(convert_enum_variant).collect(),
                    }
                }
            },
        })
        .collect();

    for account in &idl.accounts {
        if types.iter().any(|typedef| typedef.name == account.name) {
            continue;
        }

        if let Some(type_def) = &account.type_def {
            let snapshot_type = match type_def {
                idl_parser::IdlTypeDefKind::Struct { kind, fields } => {
                    IdlTypeDefKindSnapshot::Struct {
                        kind: kind.clone(),
                        fields: fields
                            .iter()
                            .map(|field| IdlFieldSnapshot {
                                name: field.name.clone(),
                                type_: convert_idl_type(&field.type_),
                                amount_hint: field.amount_hint.clone(),
                            })
                            .collect(),
                    }
                }
                idl_parser::IdlTypeDefKind::TupleStruct { kind, fields } => {
                    IdlTypeDefKindSnapshot::TupleStruct {
                        kind: kind.clone(),
                        fields: fields.iter().map(convert_idl_type).collect(),
                    }
                }
                idl_parser::IdlTypeDefKind::Enum { kind, variants } => {
                    IdlTypeDefKindSnapshot::Enum {
                        kind: kind.clone(),
                        variants: variants.iter().map(convert_enum_variant).collect(),
                    }
                }
            };
            types.push(IdlTypeDefSnapshot {
                name: account.name.clone(),
                docs: account.docs.clone(),
                serialization: None,
                type_def: snapshot_type,
            });
        }
    }

    let uses_steel_discriminant = idl.instructions.iter().any(|instruction| {
        instruction.discriminant.is_some() && instruction.discriminator.is_empty()
    });
    let discriminant_size = if uses_steel_discriminant { 1 } else { 8 };
    let program_id = idl.address.clone().or_else(|| {
        idl.metadata
            .as_ref()
            .and_then(|metadata| metadata.address.clone())
    });

    IdlSnapshot {
        name: idl.get_name().to_string(),
        program_id: program_id.clone(),
        version: idl.get_version().to_string(),
        accounts: idl
            .accounts
            .iter()
            .map(|account| {
                let serialization = idl
                    .types
                    .iter()
                    .find(|typedef| typedef.name == account.name)
                    .and_then(|typedef| typedef.serialization.as_ref())
                    .map(|serialization| match serialization {
                        idl_parser::IdlSerialization::Borsh => IdlSerializationSnapshot::Borsh,
                        idl_parser::IdlSerialization::Bytemuck => {
                            IdlSerializationSnapshot::Bytemuck
                        }
                        idl_parser::IdlSerialization::BytemuckUnsafe => {
                            IdlSerializationSnapshot::BytemuckUnsafe
                        }
                    });

                IdlAccountSnapshot {
                    name: account.name.clone(),
                    discriminator: account.get_discriminator(),
                    docs: account.docs.clone(),
                    serialization,
                    fields: account.type_def.as_ref().map_or_else(Vec::new, |type_def| {
                        match type_def {
                            idl_parser::IdlTypeDefKind::Struct { fields, .. } => fields
                                .iter()
                                .map(|field| IdlFieldSnapshot {
                                    name: field.name.clone(),
                                    type_: convert_idl_type(&field.type_),
                                    amount_hint: field.amount_hint.clone(),
                                })
                                .collect(),
                            _ => Vec::new(),
                        }
                    }),
                    type_def: None,
                }
            })
            .collect(),
        instructions: idl
            .instructions
            .iter()
            .map(|instruction| IdlInstructionSnapshot {
                name: instruction.name.clone(),
                discriminator: instruction.get_discriminator(),
                discriminant: instruction.discriminant.clone(),
                docs: instruction.docs.clone(),
                accounts: instruction
                    .flattened_accounts()
                    .iter()
                    .map(|account| IdlInstructionAccountSnapshot {
                        name: account.name.clone(),
                        writable: account.is_mut,
                        signer: account.is_signer,
                        optional: account.optional,
                        address: account.address.clone(),
                        docs: account.docs.clone(),
                    })
                    .collect(),
                args: instruction
                    .args
                    .iter()
                    .map(|arg| IdlFieldSnapshot {
                        name: arg.name.clone(),
                        type_: convert_idl_type(&arg.type_),
                        amount_hint: arg.amount_hint.clone(),
                    })
                    .collect(),
            })
            .collect(),
        types,
        events: idl
            .events
            .iter()
            .map(|event| IdlEventSnapshot {
                name: event.name.clone(),
                discriminator: event.get_discriminator(),
                docs: event.docs.clone(),
                fields: event
                    .fields
                    .iter()
                    .map(|field| IdlFieldSnapshot {
                        name: field.name.clone(),
                        type_: convert_idl_type(&field.type_),
                        amount_hint: field.amount_hint.clone(),
                    })
                    .collect(),
            })
            .collect(),
        errors: idl
            .errors
            .iter()
            .map(|error| IdlErrorSnapshot {
                code: error.code,
                name: error.name.clone(),
                msg: error.msg.clone(),
            })
            .collect(),
        discriminant_size,
    }
}

pub fn extract_pdas_from_idl(idl: &idl_parser::IdlSpec) -> BTreeMap<String, PdaDefinition> {
    let mut pdas = BTreeMap::new();

    for pda in &idl.pdas {
        let pda_name = sanitize_identifier(&pda.name);
        let pda_def = convert_idl_pda_to_def(&pda_name, &pda.seeds, pda.program.as_ref());
        pdas.insert(pda_name, pda_def);
    }

    for instruction in &idl.instructions {
        for account in instruction.flattened_accounts() {
            if let Some(pda_info) = &account.pda {
                let pda_name =
                    sanitize_identifier(pda_info.name.as_deref().unwrap_or(&account.name));
                let pda_def =
                    convert_idl_pda_to_def(&pda_name, &pda_info.seeds, pda_info.program.as_ref());
                pdas.entry(pda_name).or_insert(pda_def);
            }
        }
    }

    pdas
}

pub fn extract_instructions_from_idl(
    idl: &idl_parser::IdlSpec,
    pdas: &BTreeMap<String, PdaDefinition>,
) -> Vec<InstructionDef> {
    let program_id = idl.address.clone().or_else(|| {
        idl.metadata
            .as_ref()
            .and_then(|metadata| metadata.address.clone())
    });

    let uses_steel = idl.instructions.iter().any(|instruction| {
        instruction.discriminant.is_some() && instruction.discriminator.is_empty()
    });
    let discriminator_size = if uses_steel { 1 } else { 8 };

    idl.instructions
        .iter()
        .map(|instruction| {
            let accounts = instruction
                .flattened_accounts()
                .iter()
                .map(|account| convert_account_to_def(account, pdas))
                .collect();

            let args = instruction
                .args
                .iter()
                .map(|arg| InstructionArgDef {
                    name: arg.name.clone(),
                    arg_type: idl_type_snapshot_to_rust_string(&convert_idl_type(&arg.type_)),
                    docs: vec![],
                    amount_hint: arg.amount_hint.as_ref().map(convert_amount_hint),
                })
                .collect();

            InstructionDef {
                name: instruction.name.clone(),
                discriminator: instruction.get_discriminator(),
                discriminator_size,
                accounts,
                args,
                errors: Vec::new(),
                program_id: program_id.clone(),
                docs: instruction.docs.clone(),
            }
        })
        .collect()
}

pub fn convert_idl_type(idl_type: &idl_parser::IdlType) -> IdlTypeSnapshot {
    match idl_type {
        idl_parser::IdlType::Simple(simple) => IdlTypeSnapshot::Simple(simple.clone()),
        idl_parser::IdlType::Array(array) => IdlTypeSnapshot::Array(IdlArrayTypeSnapshot {
            array: array
                .array
                .iter()
                .map(|element| match element {
                    idl_parser::IdlTypeArrayElement::Nested(ty) => {
                        IdlArrayElementSnapshot::Type(convert_idl_type(ty))
                    }
                    idl_parser::IdlTypeArrayElement::Type(type_name) => {
                        IdlArrayElementSnapshot::TypeName(type_name.clone())
                    }
                    idl_parser::IdlTypeArrayElement::Size(size) => {
                        IdlArrayElementSnapshot::Size(*size)
                    }
                })
                .collect(),
        }),
        idl_parser::IdlType::Option(option) => IdlTypeSnapshot::Option(IdlOptionTypeSnapshot {
            option: Box::new(convert_idl_type(&option.option)),
        }),
        idl_parser::IdlType::Vec(vec_type) => IdlTypeSnapshot::Vec(IdlVecTypeSnapshot {
            vec: Box::new(convert_idl_type(&vec_type.vec)),
        }),
        idl_parser::IdlType::Defined(defined) => IdlTypeSnapshot::Defined(IdlDefinedTypeSnapshot {
            defined: match &defined.defined {
                idl_parser::IdlTypeDefinedInner::Named { name } => {
                    IdlDefinedInnerSnapshot::Named { name: name.clone() }
                }
                idl_parser::IdlTypeDefinedInner::Simple(simple) => {
                    IdlDefinedInnerSnapshot::Simple(simple.clone())
                }
            },
        }),
        idl_parser::IdlType::HashMap(hash_map) => {
            IdlTypeSnapshot::HashMap(IdlHashMapTypeSnapshot {
                hash_map: (
                    Box::new(convert_idl_type(&hash_map.hash_map.0)),
                    Box::new(convert_idl_type(&hash_map.hash_map.1)),
                ),
            })
        }
    }
}

fn convert_amount_hint(hint: &idl_parser::IdlAmountHint) -> InstructionAmountHint {
    InstructionAmountHint {
        decimals_source: match &hint.decimals_source {
            idl_parser::IdlAmountDecimalsSource::ArgMint { arg_name } => {
                AmountDecimalsSource::ArgMint {
                    arg_name: arg_name.clone(),
                }
            }
            idl_parser::IdlAmountDecimalsSource::ArgDecimals { arg_name } => {
                AmountDecimalsSource::ArgDecimals {
                    arg_name: arg_name.clone(),
                }
            }
            idl_parser::IdlAmountDecimalsSource::KnownAccount { account_name } => {
                AmountDecimalsSource::KnownAccount {
                    account_name: account_name.clone(),
                }
            }
            idl_parser::IdlAmountDecimalsSource::Constant { decimals } => {
                AmountDecimalsSource::Constant {
                    decimals: *decimals,
                }
            }
        },
    }
}

fn convert_enum_variant(variant: &idl_parser::IdlEnumVariant) -> IdlEnumVariantSnapshot {
    IdlEnumVariantSnapshot {
        name: variant.name.clone(),
        fields: variant
            .fields
            .iter()
            .map(|field| match field {
                idl_parser::IdlEnumVariantField::Named(named) => {
                    IdlEnumVariantFieldSnapshot::Named(IdlFieldSnapshot {
                        name: named.name.clone(),
                        type_: convert_idl_type(&named.type_),
                        amount_hint: named.amount_hint.clone(),
                    })
                }
                idl_parser::IdlEnumVariantField::Tuple(tuple) => {
                    IdlEnumVariantFieldSnapshot::Tuple(convert_idl_type(tuple))
                }
            })
            .collect(),
    }
}

fn convert_idl_pda_to_def(
    name: &str,
    pda_seeds: &[idl_parser::IdlPdaSeed],
    pda_program: Option<&idl_parser::IdlPdaProgram>,
) -> PdaDefinition {
    let seeds = pda_seeds
        .iter()
        .map(|seed| match seed {
            idl_parser::IdlPdaSeed::Const { value } => {
                if let Ok(string) = String::from_utf8(value.clone()) {
                    PdaSeedDef::Literal { value: string }
                } else {
                    PdaSeedDef::Bytes {
                        value: value.clone(),
                    }
                }
            }
            idl_parser::IdlPdaSeed::Account { path, .. } => PdaSeedDef::AccountRef {
                account_name: sanitize_seed_path(path),
            },
            idl_parser::IdlPdaSeed::Arg { path, arg_type } => PdaSeedDef::ArgRef {
                arg_name: sanitize_seed_path(path),
                arg_type: arg_type.clone(),
            },
        })
        .collect();

    let program_id = pda_program.and_then(|program| match program {
        idl_parser::IdlPdaProgram::Literal { value, .. } => Some(value.clone()),
        idl_parser::IdlPdaProgram::Const { value, .. } => Some(bs58::encode(value).into_string()),
        idl_parser::IdlPdaProgram::Account { .. } => None,
    });

    PdaDefinition {
        name: name.to_string(),
        seeds,
        program_id,
    }
}

fn convert_account_to_def(
    account: &idl_parser::IdlAccountArg,
    pdas: &BTreeMap<String, PdaDefinition>,
) -> InstructionAccountDef {
    let resolution = if account.is_signer && account.address.is_none() && account.pda.is_none() {
        AccountResolution::Signer
    } else if let Some(address) = &account.address {
        AccountResolution::Known {
            address: address.clone(),
        }
    } else if account.pda.is_some() {
        let pda_name = sanitize_identifier(
            account
                .pda
                .as_ref()
                .and_then(|pda| pda.name.as_deref())
                .unwrap_or(&account.name),
        );
        if pdas.contains_key(&pda_name) {
            AccountResolution::PdaRef {
                pda_name: pda_name.to_string(),
            }
        } else if let Some(pda_info) = &account.pda {
            let pda_def =
                convert_idl_pda_to_def(&pda_name, &pda_info.seeds, pda_info.program.as_ref());
            AccountResolution::PdaInline {
                seeds: pda_def.seeds,
                program_id: pda_def.program_id,
            }
        } else {
            AccountResolution::UserProvided
        }
    } else if pdas.contains_key(&sanitize_identifier(&account.name)) {
        AccountResolution::PdaRef {
            pda_name: sanitize_identifier(&account.name),
        }
    } else {
        AccountResolution::UserProvided
    };

    InstructionAccountDef {
        name: sanitize_identifier(&account.name),
        is_signer: account.is_signer,
        is_writable: account.is_mut,
        resolution,
        is_optional: account.optional,
        docs: account.docs.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_program_only_stack_spec_from_raw_idl() {
        let idl = arete_idl::parse::parse_idl_content(
            r#"{
              "address": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
              "version": "0.0.0",
              "name": "token",
              "instructions": [
                {
                  "name": "InitializeMint2",
                  "accounts": [
                    { "name": "mint", "isMut": true, "isSigner": false }
                  ],
                  "args": [
                    { "name": "decimals", "type": "u8" },
                    { "name": "mintAuthority", "type": "publicKey" }
                  ],
                  "discriminant": { "type": "u8", "value": 20 }
                }
              ],
              "accounts": [],
              "types": [],
              "events": [],
              "errors": []
            }"#,
        )
        .expect("IDL should parse");

        let spec = build_program_only_stack_spec_from_idl(&idl, "SplToken");
        assert_eq!(spec.stack_name, "SplToken");
        assert!(spec.entities.is_empty());
        assert_eq!(spec.idls.len(), 1);
        assert_eq!(spec.instructions.len(), 1);
        assert_eq!(spec.instructions[0].name, "InitializeMint2");
        assert_eq!(spec.instructions[0].discriminator, vec![20]);
        assert_eq!(spec.instructions[0].discriminator_size, 1);
        assert_eq!(
            spec.instructions[0].program_id.as_deref(),
            Some("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
        );
        assert_eq!(
            spec.instructions[0].args[1].arg_type,
            "solana_pubkey::Pubkey"
        );
        assert!(spec.content_hash.is_some());
    }

    #[test]
    fn preserves_nested_seed_paths_when_building_program_only_specs() {
        let idl = arete_idl::parse::parse_idl_content(
            r#"{
              "address": "Prog111111111111111111111111111111111111111",
              "version": "0.0.0",
              "name": "demo",
              "instructions": [
                {
                  "name": "proposalCreate",
                  "accounts": [
                    {
                      "name": "proposal",
                      "isMut": true,
                      "isSigner": false,
                      "pda": {
                        "name": "proposal",
                        "seeds": [
                          {
                            "kind": "arg",
                            "path": "args.transactionIndex",
                            "type": "u64"
                          }
                        ]
                      }
                    }
                  ],
                  "args": [
                    {
                      "name": "args",
                      "type": {
                        "defined": {
                          "name": "ProposalArgs"
                        }
                      }
                    }
                  ],
                  "discriminant": { "type": "u8", "value": 3 }
                }
              ],
              "accounts": [],
              "types": [
                {
                  "name": "ProposalArgs",
                  "type": {
                    "kind": "struct",
                    "fields": [
                      { "name": "transactionIndex", "type": "u64" }
                    ]
                  }
                }
              ],
              "events": [],
              "errors": []
            }"#,
        )
        .expect("IDL should parse");

        let spec = build_program_only_stack_spec_from_idl(&idl, "Demo");
        let pda = spec
            .pdas
            .get("demo")
            .and_then(|program| program.get("proposal"))
            .expect("proposal PDA should be present");
        assert_eq!(
            pda.seeds,
            vec![PdaSeedDef::ArgRef {
                arg_name: "args.transactionIndex".to_string(),
                arg_type: Some("u64".to_string()),
            }]
        );
    }

    #[test]
    fn preserves_amount_hints_from_idl_args() {
        let idl = arete_idl::parse::parse_idl_content(
            r#"{
              "address": "Prog111111111111111111111111111111111111111",
              "version": "0.0.0",
              "name": "demo",
              "instructions": [
                {
                  "name": "deposit",
                  "accounts": [],
                  "args": [
                    {
                      "name": "amount",
                      "type": "u64",
                      "amountHint": {
                        "decimalsSource": {
                          "kind": "argMint",
                          "argName": "mint"
                        }
                      }
                    },
                    {
                      "name": "mint",
                      "type": "publicKey"
                    }
                  ],
                  "discriminant": { "type": "u8", "value": 7 }
                }
              ],
              "accounts": [],
              "types": [],
              "events": [],
              "errors": []
            }"#,
        )
        .expect("IDL should parse");

        let spec = build_program_only_stack_spec_from_idl(&idl, "Demo");
        assert_eq!(
            spec.instructions[0].args[0].amount_hint,
            Some(InstructionAmountHint {
                decimals_source: AmountDecimalsSource::ArgMint {
                    arg_name: "mint".to_string(),
                },
            })
        );
        assert!(spec.idls[0].instructions[0].args[0].amount_hint.is_some());
    }
}
