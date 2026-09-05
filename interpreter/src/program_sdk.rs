use crate::ast::{
    IdlArrayElementSnapshot, IdlArrayTypeSnapshot, IdlDefinedInnerSnapshot, IdlDefinedTypeSnapshot,
    IdlHashMapTypeSnapshot, IdlOptionTypeSnapshot, IdlSnapshot, IdlTupleTypeSnapshot,
    IdlTypeSnapshot, IdlVecTypeSnapshot, InstructionDef, PdaDefinition, SerializableStackSpec,
    CURRENT_AST_VERSION,
};
use arete_idl as idl_parser;
use std::collections::BTreeMap;

pub fn build_program_only_stack_spec_from_idl(
    idl: &idl_parser::IdlSpec,
    stack_name: &str,
) -> SerializableStackSpec {
    let program_spec = build_program_spec_v1_from_idl(idl)
        .expect("IDL program identity must be valid for ProgramSpecV1");
    build_program_only_stack_spec_from_program_spec(program_spec, stack_name)
}

pub fn build_program_spec_v1_from_idl(
    idl: &idl_parser::IdlSpec,
) -> Result<arete_hash::ProgramSpecV1, arete_hash::HashError> {
    arete_hash::build_program_spec_v1_from_idl(idl, None)
}

pub fn build_program_spec_v1_from_idl_bytes(
    bytes: &[u8],
    explicit_program_id: Option<&str>,
) -> Result<arete_hash::ProgramSpecV1, arete_hash::HashError> {
    arete_hash::build_program_spec_v1_from_bytes(bytes, explicit_program_id)
}

pub fn build_oss_program_identity_v1_from_idl(
    idl: &idl_parser::IdlSpec,
) -> Result<arete_hash::OssProgramIdentityV1, arete_hash::HashError> {
    arete_hash::OssProgramIdentityV1::new(build_program_spec_v1_from_idl(idl)?)
}

pub fn build_oss_program_identity_v1_from_idl_bytes(
    bytes: &[u8],
    explicit_program_id: Option<&str>,
) -> Result<arete_hash::OssProgramIdentityV1, arete_hash::HashError> {
    arete_hash::build_oss_program_identity_v1_from_bytes(bytes, explicit_program_id)
}

pub fn build_program_only_stack_spec_from_idl_bytes(
    bytes: &[u8],
    explicit_program_id: Option<&str>,
    stack_name: &str,
) -> Result<SerializableStackSpec, arete_hash::HashError> {
    let program_spec = build_program_spec_v1_from_idl_bytes(bytes, explicit_program_id)?;
    Ok(build_program_only_stack_spec_from_program_spec(
        program_spec,
        stack_name,
    ))
}

pub fn build_program_only_stack_spec_from_program_spec(
    program_spec: arete_hash::ProgramSpecV1,
    stack_name: &str,
) -> SerializableStackSpec {
    build_program_only_stack_spec_from_program_spec_ref(&program_spec, stack_name)
}

pub fn build_program_only_stack_spec_from_identity(
    identity: &arete_hash::OssProgramIdentityV1,
    stack_name: &str,
) -> SerializableStackSpec {
    build_program_only_stack_spec_from_program_spec_ref(&identity.program_spec, stack_name)
}

fn build_program_only_stack_spec_from_program_spec_ref(
    program_spec: &arete_hash::ProgramSpecV1,
    stack_name: &str,
) -> SerializableStackSpec {
    let snapshot = program_spec.idl_snapshot.clone().into_legacy_snapshot();
    let program_id = Some(program_spec.program_id.clone());
    let pdas: BTreeMap<String, PdaDefinition> =
        transcode_program_projection(program_spec.pdas.clone());
    let instructions: Vec<InstructionDef> =
        transcode_program_projection(program_spec.instructions.clone());

    let mut grouped_pdas = BTreeMap::new();
    if !pdas.is_empty() {
        grouped_pdas.insert(snapshot.name.clone(), pdas);
    }

    SerializableStackSpec {
        ast_version: CURRENT_AST_VERSION.to_string(),
        stack_name: stack_name.to_string(),
        program_ids: program_id.into_iter().collect(),
        idls: vec![snapshot],
        program_specs: vec![program_spec.clone()],
        entities: vec![],
        pdas: grouped_pdas,
        instructions,
        content_hash: None,
    }
    .with_content_hash()
}

fn transcode_program_projection<T, U>(value: T) -> U
where
    T: serde::Serialize,
    U: serde::de::DeserializeOwned,
{
    serde_json::from_value(
        serde_json::to_value(value).expect("shared ProgramSpec projection must serialize"),
    )
    .expect("shared ProgramSpec projection must match the legacy AST adapter")
}

pub fn convert_idl_to_snapshot(idl: &idl_parser::IdlSpec) -> IdlSnapshot {
    arete_idl::normalize_idl_snapshot(idl)
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
            length_prefix: vec_type.length_prefix,
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
        idl_parser::IdlType::Tuple(tuple) => IdlTypeSnapshot::Tuple(IdlTupleTypeSnapshot {
            tuple: tuple.tuple.iter().map(convert_idl_type).collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{AmountDecimalsSource, InstructionAmountHint, PdaSeedDef};

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

    #[test]
    fn derives_the_checked_in_program_and_release_identities() {
        let corpus: serde_json::Value =
            serde_json::from_str(include_str!("../../test-vectors/hash-v1.json"))
                .expect("vector corpus");
        let vector = corpus["idlVectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|vector| vector["id"] == "idl-primary")
            .expect("primary IDL vector");
        let source = vector["input"]["data"].as_str().unwrap().as_bytes();

        let identity = build_oss_program_identity_v1_from_idl_bytes(source, None)
            .expect("interpreter identity");
        let stack_spec = build_program_only_stack_spec_from_identity(&identity, "Demo");

        assert_eq!(
            identity.program_spec_hash.to_string(),
            vector["expected"]["programSpecIdentity"]["hashId"]
        );
        assert_eq!(
            identity.release_hash.to_string(),
            vector["expected"]["ossReleaseIdentity"]["hashId"]
        );
        assert_eq!(stack_spec.content_hash.as_deref().unwrap().len(), 64);
        assert_eq!(stack_spec.program_specs.len(), 1);
        assert_eq!(
            stack_spec.program_specs[0].hash().unwrap(),
            identity.program_spec_hash
        );
        assert!(stack_spec
            .content_hash
            .as_deref()
            .unwrap()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
        assert_eq!(
            stack_spec.content_hash.as_deref(),
            Some(stack_spec.compute_content_hash().as_str())
        );
    }
}
