//! Snapshot type definitions

use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize};

use crate::types::{IdlAmountHint, SteelDiscriminant};

/// Version of the semantic IDL normalization performed by this crate.
///
/// Incrementing this value is a protocol change: it changes the canonical
/// `IdlSnapshotV1` input and all identities that contain it.
pub const IDL_NORMALIZATION_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize)]
pub struct IdlSnapshot {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "address")]
    pub program_id: Option<String>,
    pub version: String,
    pub accounts: Vec<IdlAccountSnapshot>,
    pub instructions: Vec<IdlInstructionSnapshot>,
    #[serde(default)]
    pub types: Vec<IdlTypeDefSnapshot>,
    #[serde(default)]
    pub events: Vec<IdlEventSnapshot>,
    #[serde(default)]
    pub errors: Vec<IdlErrorSnapshot>,
    pub discriminant_size: usize,
}

/// Canonical, versioned input for normalized IDL identities.
///
/// The legacy `IdlSnapshot` wire shape remains unchanged for existing ASTs.
/// This wrapper adds the normalization version only to new protocol inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "IdlSnapshotV1Wire")]
pub struct IdlSnapshotV1 {
    pub normalization_version: u32,
    #[serde(flatten)]
    pub snapshot: IdlSnapshot,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdlSnapshotV1Wire {
    normalization_version: u32,
    #[serde(flatten)]
    snapshot: IdlSnapshot,
}

impl TryFrom<IdlSnapshotV1Wire> for IdlSnapshotV1 {
    type Error = String;

    fn try_from(value: IdlSnapshotV1Wire) -> Result<Self, Self::Error> {
        if value.normalization_version != IDL_NORMALIZATION_VERSION {
            return Err(format!(
                "unsupported IDL normalization version {}; expected {}",
                value.normalization_version, IDL_NORMALIZATION_VERSION
            ));
        }
        Ok(Self {
            normalization_version: value.normalization_version,
            snapshot: value.snapshot,
        })
    }
}

impl IdlSnapshotV1 {
    pub fn new(snapshot: IdlSnapshot) -> Self {
        Self {
            normalization_version: IDL_NORMALIZATION_VERSION,
            snapshot,
        }
    }

    pub fn into_legacy_snapshot(self) -> IdlSnapshot {
        self.snapshot
    }
}

impl<'de> Deserialize<'de> for IdlSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // First deserialize to a generic Value to inspect instructions
        let value = serde_json::Value::deserialize(deserializer)?;

        // Check if any instruction has discriminant (Steel-style) vs discriminator (Anchor-style)
        let discriminant_size = value
            .get("instructions")
            .and_then(|instrs| instrs.as_array())
            .map(|instrs| {
                if instrs.is_empty() {
                    return false;
                }
                instrs.iter().all(|ix| {
                    let discriminator = ix.get("discriminator");
                    let disc_len = discriminator
                        .and_then(|d| d.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);

                    // Treat discriminant as present only if the value is non-null.
                    // ix.get("discriminant").is_some() returns true even for `null`,
                    // which causes misclassification when the AST serializer writes
                    // `discriminant: null` explicitly (as the ore AST does).
                    let has_discriminant = ix
                        .get("discriminant")
                        .map(|v| !v.is_null())
                        .unwrap_or(false);
                    let has_discriminator = discriminator
                        .map(|d| {
                            !d.is_null() && d.as_array().map(|a| !a.is_empty()).unwrap_or(true)
                        })
                        .unwrap_or(false);

                    // Steel-style variant 1: explicit discriminant object, no discriminator array
                    let is_steel_discriminant = has_discriminant && !has_discriminator;

                    // Steel-style variant 2: discriminator is stored as a 1-byte array with no
                    // discriminant value. This happens when the AST serializer flattens the
                    // Steel u8 discriminant directly into the discriminator field.
                    let is_steel_short_discriminator = !has_discriminant && disc_len == 1;

                    is_steel_discriminant || is_steel_short_discriminator
                })
            })
            .map(|is_steel| if is_steel { 1 } else { 8 })
            .unwrap_or(8); // Default to 8 if no instructions

        // Now deserialize the full struct
        let mut intermediate: IdlSnapshotIntermediate = serde_json::from_value(value)
            .map_err(|e| DeError::custom(format!("Failed to deserialize IDL: {}", e)))?;
        // Only use the heuristic if discriminant_size wasn't already present in the JSON
        // (discriminant_size = 0 means it was absent / defaulted).
        if intermediate.discriminant_size == 0 {
            intermediate.discriminant_size = discriminant_size;
        }

        Ok(IdlSnapshot {
            name: intermediate.name,
            program_id: intermediate.program_id,
            version: intermediate.version,
            accounts: intermediate.accounts,
            instructions: intermediate.instructions,
            types: intermediate.types,
            events: intermediate.events,
            errors: intermediate.errors,
            discriminant_size: intermediate.discriminant_size,
        })
    }
}

// Intermediate struct for deserialization
#[derive(Debug, Clone, Deserialize)]
struct IdlSnapshotIntermediate {
    pub name: String,
    #[serde(default, alias = "address")]
    pub program_id: Option<String>,
    pub version: String,
    pub accounts: Vec<IdlAccountSnapshot>,
    pub instructions: Vec<IdlInstructionSnapshot>,
    #[serde(default)]
    pub types: Vec<IdlTypeDefSnapshot>,
    #[serde(default)]
    pub events: Vec<IdlEventSnapshot>,
    #[serde(default)]
    pub errors: Vec<IdlErrorSnapshot>,
    #[serde(default)]
    pub discriminant_size: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct IdlAccountSnapshot {
    pub name: String,
    pub discriminator: Vec<u8>,
    pub docs: Vec<String>,
    pub serialization: Option<IdlSerializationSnapshot>,
    /// Account fields - populated from inline type definition
    pub fields: Vec<IdlFieldSnapshot>,
    /// Inline type definition (for Steel format with type.fields structure)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_def: Option<IdlInlineTypeDef>,
}

// Intermediate struct for deserialization
#[derive(Deserialize)]
struct IdlAccountSnapshotIntermediate {
    pub name: String,
    pub discriminator: Vec<u8>,
    #[serde(default)]
    pub docs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serialization: Option<IdlSerializationSnapshot>,
    #[serde(default)]
    pub fields: Vec<IdlFieldSnapshot>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub type_def: Option<IdlInlineTypeDef>,
}

impl<'de> Deserialize<'de> for IdlAccountSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let intermediate = IdlAccountSnapshotIntermediate::deserialize(deserializer)?;

        // Normalize fields: if empty but type_def has fields, use those
        let fields = if intermediate.fields.is_empty() {
            if let Some(type_def) = intermediate.type_def.as_ref() {
                type_def.fields.clone()
            } else {
                intermediate.fields
            }
        } else {
            intermediate.fields
        };

        Ok(IdlAccountSnapshot {
            name: intermediate.name,
            discriminator: intermediate.discriminator,
            docs: intermediate.docs,
            serialization: intermediate.serialization,
            fields,
            type_def: intermediate.type_def,
        })
    }
}

/// Inline type definition for account fields (Steel format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlInlineTypeDef {
    pub kind: String,
    pub fields: Vec<IdlFieldSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlInstructionSnapshot {
    pub name: String,
    #[serde(default)]
    pub discriminator: Vec<u8>,
    #[serde(default)]
    pub discriminant: Option<SteelDiscriminant>,
    #[serde(default)]
    pub docs: Vec<String>,
    pub accounts: Vec<IdlInstructionAccountSnapshot>,
    pub args: Vec<IdlFieldSnapshot>,
}

impl IdlInstructionSnapshot {
    /// Get the computed 8-byte discriminator.
    /// Returns the explicit discriminator if present, otherwise computes from discriminant.
    pub fn get_discriminator(&self) -> Vec<u8> {
        if !self.discriminator.is_empty() {
            return self.discriminator.clone();
        }

        if let Some(disc) = &self.discriminant {
            match u8::try_from(disc.value) {
                Ok(value) => return vec![value],
                Err(_) => {
                    tracing::warn!(
                        instruction = %self.name,
                        value = disc.value,
                        "Steel discriminant exceeds u8::MAX; falling back to Anchor hash"
                    );
                }
            }
        }

        crate::discriminator::anchor_discriminator(&format!(
            "global:{}",
            crate::utils::to_snake_case(&self.name)
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlInstructionAccountSnapshot {
    pub name: String,
    #[serde(default)]
    pub writable: bool,
    #[serde(default)]
    pub signer: bool,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub docs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlFieldSnapshot {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: IdlTypeSnapshot,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "amountHint"
    )]
    pub amount_hint: Option<IdlAmountHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IdlTypeSnapshot {
    Simple(String),
    Array(IdlArrayTypeSnapshot),
    Option(IdlOptionTypeSnapshot),
    Vec(IdlVecTypeSnapshot),
    HashMap(IdlHashMapTypeSnapshot),
    Defined(IdlDefinedTypeSnapshot),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlHashMapTypeSnapshot {
    #[serde(rename = "hashMap", deserialize_with = "deserialize_hash_map")]
    pub hash_map: (Box<IdlTypeSnapshot>, Box<IdlTypeSnapshot>),
}

fn deserialize_hash_map<'de, D>(
    deserializer: D,
) -> Result<(Box<IdlTypeSnapshot>, Box<IdlTypeSnapshot>), D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    let values: Vec<IdlTypeSnapshot> = Vec::deserialize(deserializer)?;
    if values.len() != 2 {
        return Err(D::Error::custom("hashMap must have exactly 2 elements"));
    }
    let mut iter = values.into_iter();
    Ok((
        Box::new(iter.next().expect("length checked")),
        Box::new(iter.next().expect("length checked")),
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlArrayTypeSnapshot {
    pub array: Vec<IdlArrayElementSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IdlArrayElementSnapshot {
    Type(IdlTypeSnapshot),
    TypeName(String),
    Size(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlOptionTypeSnapshot {
    pub option: Box<IdlTypeSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlVecTypeSnapshot {
    pub vec: Box<IdlTypeSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlDefinedTypeSnapshot {
    pub defined: IdlDefinedInnerSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IdlDefinedInnerSnapshot {
    Named { name: String },
    Simple(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdlSerializationSnapshot {
    Borsh,
    Bytemuck,
    #[serde(alias = "bytemuckunsafe")]
    BytemuckUnsafe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlTypeDefSnapshot {
    pub name: String,
    #[serde(default)]
    pub docs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serialization: Option<IdlSerializationSnapshot>,
    #[serde(rename = "type")]
    pub type_def: IdlTypeDefKindSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IdlTypeDefKindSnapshot {
    Struct {
        kind: String,
        fields: Vec<IdlFieldSnapshot>,
    },
    TupleStruct {
        kind: String,
        fields: Vec<IdlTypeSnapshot>,
    },
    Enum {
        kind: String,
        variants: Vec<IdlEnumVariantSnapshot>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlEnumVariantSnapshot {
    pub name: String,
    /// Variant payload, when the variant carries data. Skipped when empty so
    /// fieldless variants serialize byte-identically to older snapshots.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<IdlEnumVariantFieldSnapshot>,
}

/// One field of a data-carrying enum variant (named struct field or bare
/// tuple element).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IdlEnumVariantFieldSnapshot {
    Named(IdlFieldSnapshot),
    Tuple(IdlTypeSnapshot),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlEventSnapshot {
    pub name: String,
    pub discriminator: Vec<u8>,
    #[serde(default)]
    pub docs: Vec<String>,
    #[serde(default)]
    pub fields: Vec<IdlFieldSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IdlErrorSnapshot {
    pub code: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

/// Normalize a parsed IDL into the stable snapshot used by existing ASTs.
pub fn normalize_idl_snapshot(idl: &crate::types::IdlSpec) -> IdlSnapshot {
    use crate::types::IdlTypeDefKind;

    let mut types: Vec<IdlTypeDefSnapshot> = idl
        .types
        .iter()
        .map(|typedef| IdlTypeDefSnapshot {
            name: typedef.name.clone(),
            docs: typedef.docs.clone(),
            serialization: typedef.serialization.as_ref().map(snapshot_serialization),
            type_def: snapshot_type_def(&typedef.type_def),
        })
        .collect();

    for account in &idl.accounts {
        if types.iter().any(|typedef| typedef.name == account.name) {
            continue;
        }
        if let Some(type_def) = &account.type_def {
            types.push(IdlTypeDefSnapshot {
                name: account.name.clone(),
                docs: account.docs.clone(),
                serialization: None,
                type_def: snapshot_type_def(type_def),
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
        program_id,
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
                    .map(snapshot_serialization);
                let fields =
                    account
                        .type_def
                        .as_ref()
                        .map_or_else(Vec::new, |type_def| match type_def {
                            IdlTypeDefKind::Struct { fields, .. } => {
                                fields.iter().map(snapshot_field).collect()
                            }
                            _ => Vec::new(),
                        });

                IdlAccountSnapshot {
                    name: account.name.clone(),
                    discriminator: account.get_discriminator(),
                    docs: account.docs.clone(),
                    serialization,
                    fields,
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
                args: instruction.args.iter().map(snapshot_field).collect(),
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
                fields: event.fields.iter().map(snapshot_field).collect(),
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

/// Normalize a parsed IDL into the versioned input used by authoritative hashes.
pub fn normalize_idl_snapshot_v1(idl: &crate::types::IdlSpec) -> IdlSnapshotV1 {
    IdlSnapshotV1::new(normalize_idl_snapshot(idl))
}

fn snapshot_serialization(
    serialization: &crate::types::IdlSerialization,
) -> IdlSerializationSnapshot {
    match serialization {
        crate::types::IdlSerialization::Borsh => IdlSerializationSnapshot::Borsh,
        crate::types::IdlSerialization::Bytemuck => IdlSerializationSnapshot::Bytemuck,
        crate::types::IdlSerialization::BytemuckUnsafe => IdlSerializationSnapshot::BytemuckUnsafe,
    }
}

fn snapshot_field(field: &crate::types::IdlField) -> IdlFieldSnapshot {
    IdlFieldSnapshot {
        name: field.name.clone(),
        type_: snapshot_type(&field.type_),
        amount_hint: field.amount_hint.clone(),
    }
}

fn snapshot_type_def(type_def: &crate::types::IdlTypeDefKind) -> IdlTypeDefKindSnapshot {
    match type_def {
        crate::types::IdlTypeDefKind::Struct { kind, fields } => IdlTypeDefKindSnapshot::Struct {
            kind: kind.clone(),
            fields: fields.iter().map(snapshot_field).collect(),
        },
        crate::types::IdlTypeDefKind::TupleStruct { kind, fields } => {
            IdlTypeDefKindSnapshot::TupleStruct {
                kind: kind.clone(),
                fields: fields.iter().map(snapshot_type).collect(),
            }
        }
        crate::types::IdlTypeDefKind::Enum { kind, variants } => IdlTypeDefKindSnapshot::Enum {
            kind: kind.clone(),
            variants: variants.iter().map(snapshot_enum_variant).collect(),
        },
    }
}

fn snapshot_enum_variant(variant: &crate::types::IdlEnumVariant) -> IdlEnumVariantSnapshot {
    IdlEnumVariantSnapshot {
        name: variant.name.clone(),
        fields: variant
            .fields
            .iter()
            .map(|field| match field {
                crate::types::IdlEnumVariantField::Named(named) => {
                    IdlEnumVariantFieldSnapshot::Named(snapshot_field(named))
                }
                crate::types::IdlEnumVariantField::Tuple(tuple) => {
                    IdlEnumVariantFieldSnapshot::Tuple(snapshot_type(tuple))
                }
            })
            .collect(),
    }
}

fn snapshot_type(idl_type: &crate::types::IdlType) -> IdlTypeSnapshot {
    match idl_type {
        crate::types::IdlType::Simple(simple) => IdlTypeSnapshot::Simple(simple.clone()),
        crate::types::IdlType::Array(array) => IdlTypeSnapshot::Array(IdlArrayTypeSnapshot {
            array: array
                .array
                .iter()
                .map(|element| match element {
                    crate::types::IdlTypeArrayElement::Nested(ty) => {
                        IdlArrayElementSnapshot::Type(snapshot_type(ty))
                    }
                    crate::types::IdlTypeArrayElement::Type(type_name) => {
                        IdlArrayElementSnapshot::TypeName(type_name.clone())
                    }
                    crate::types::IdlTypeArrayElement::Size(size) => {
                        IdlArrayElementSnapshot::Size(*size)
                    }
                })
                .collect(),
        }),
        crate::types::IdlType::Option(option) => IdlTypeSnapshot::Option(IdlOptionTypeSnapshot {
            option: Box::new(snapshot_type(&option.option)),
        }),
        crate::types::IdlType::Vec(vec_type) => IdlTypeSnapshot::Vec(IdlVecTypeSnapshot {
            vec: Box::new(snapshot_type(&vec_type.vec)),
        }),
        crate::types::IdlType::Defined(defined) => {
            IdlTypeSnapshot::Defined(IdlDefinedTypeSnapshot {
                defined: match &defined.defined {
                    crate::types::IdlTypeDefinedInner::Named { name } => {
                        IdlDefinedInnerSnapshot::Named { name: name.clone() }
                    }
                    crate::types::IdlTypeDefinedInner::Simple(simple) => {
                        IdlDefinedInnerSnapshot::Simple(simple.clone())
                    }
                },
            })
        }
        crate::types::IdlType::HashMap(hash_map) => {
            IdlTypeSnapshot::HashMap(IdlHashMapTypeSnapshot {
                hash_map: (
                    Box::new(snapshot_type(&hash_map.hash_map.0)),
                    Box::new(snapshot_type(&hash_map.hash_map.1)),
                ),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fieldless_enum_variant_wire_format_unchanged() {
        // Fieldless variants must serialize WITHOUT a `fields` key so existing
        // stack.json files (and the CI regenerate-diff) are unaffected by the
        // fielded-enum extension.
        let fieldless = IdlEnumVariantSnapshot {
            name: "Active".to_string(),
            fields: vec![],
        };
        assert_eq!(
            serde_json::to_string(&fieldless).unwrap(),
            r#"{"name":"Active"}"#
        );

        // Old-format JSON (no `fields`) still deserializes.
        let parsed: IdlEnumVariantSnapshot = serde_json::from_str(r#"{"name":"Active"}"#).unwrap();
        assert!(parsed.fields.is_empty());

        // Fielded variants round-trip through both named and tuple shapes,
        // and the untagged IdlTypeDefKindSnapshot still classifies as Enum.
        let fielded = IdlTypeDefKindSnapshot::Enum {
            kind: "enum".to_string(),
            variants: vec![IdlEnumVariantSnapshot {
                name: "Sunset".to_string(),
                fields: vec![
                    IdlEnumVariantFieldSnapshot::Named(IdlFieldSnapshot {
                        name: "endTs".to_string(),
                        type_: IdlTypeSnapshot::Simple("i64".to_string()),
                        amount_hint: None,
                    }),
                    IdlEnumVariantFieldSnapshot::Tuple(IdlTypeSnapshot::Simple("u8".to_string())),
                ],
            }],
        };
        let json = serde_json::to_string(&fielded).unwrap();
        let parsed: IdlTypeDefKindSnapshot = serde_json::from_str(&json).unwrap();
        match parsed {
            IdlTypeDefKindSnapshot::Enum { variants, .. } => {
                assert_eq!(variants[0].fields.len(), 2);
                assert!(matches!(
                    variants[0].fields[0],
                    IdlEnumVariantFieldSnapshot::Named(_)
                ));
                assert!(matches!(
                    variants[0].fields[1],
                    IdlEnumVariantFieldSnapshot::Tuple(_)
                ));
            }
            other => panic!("untagged round-trip misclassified enum as {:?}", other),
        }
    }

    #[test]
    fn test_snapshot_serde() {
        let snapshot = IdlSnapshot {
            name: "test_program".to_string(),
            program_id: Some("11111111111111111111111111111111".to_string()),
            version: "0.1.0".to_string(),
            accounts: vec![IdlAccountSnapshot {
                name: "ExampleAccount".to_string(),
                discriminator: vec![1, 2, 3, 4, 5, 6, 7, 8],
                docs: vec!["Example account".to_string()],
                serialization: Some(IdlSerializationSnapshot::Borsh),
                fields: vec![],
                type_def: None,
            }],
            instructions: vec![IdlInstructionSnapshot {
                name: "example_instruction".to_string(),
                discriminator: vec![8, 7, 6, 5, 4, 3, 2, 1],
                discriminant: None,
                docs: vec!["Example instruction".to_string()],
                accounts: vec![IdlInstructionAccountSnapshot {
                    name: "payer".to_string(),
                    writable: true,
                    signer: true,
                    optional: false,
                    address: None,
                    docs: vec![],
                }],
                args: vec![IdlFieldSnapshot {
                    name: "amount".to_string(),
                    type_: IdlTypeSnapshot::HashMap(IdlHashMapTypeSnapshot {
                        hash_map: (
                            Box::new(IdlTypeSnapshot::Simple("u64".to_string())),
                            Box::new(IdlTypeSnapshot::Simple("string".to_string())),
                        ),
                    }),
                    amount_hint: None,
                }],
            }],
            types: vec![IdlTypeDefSnapshot {
                name: "ExampleType".to_string(),
                docs: vec![],
                serialization: None,
                type_def: IdlTypeDefKindSnapshot::Struct {
                    kind: "struct".to_string(),
                    fields: vec![IdlFieldSnapshot {
                        name: "value".to_string(),
                        type_: IdlTypeSnapshot::Simple("u64".to_string()),
                        amount_hint: None,
                    }],
                },
            }],
            events: vec![IdlEventSnapshot {
                name: "ExampleEvent".to_string(),
                discriminator: vec![0, 0, 0, 0, 0, 0, 0, 1],
                docs: vec![],
                fields: vec![],
            }],
            errors: vec![IdlErrorSnapshot {
                code: 6000,
                name: "ExampleError".to_string(),
                msg: Some("example".to_string()),
            }],
            discriminant_size: 8,
        };

        let serialized = serde_json::to_value(&snapshot).expect("serialize snapshot");
        let deserialized: IdlSnapshot =
            serde_json::from_value(serialized.clone()).expect("deserialize snapshot");
        let round_trip = serde_json::to_value(&deserialized).expect("re-serialize snapshot");

        assert_eq!(serialized, round_trip);
        assert_eq!(deserialized.name, "test_program");
    }

    #[test]
    fn test_hashmap_compat() {
        let json = r#"{"hashMap":["u64","string"]}"#;
        let parsed: IdlHashMapTypeSnapshot =
            serde_json::from_str(json).expect("deserialize hashMap");

        assert!(matches!(
            parsed.hash_map.0.as_ref(),
            IdlTypeSnapshot::Simple(value) if value == "u64"
        ));
        assert!(matches!(
            parsed.hash_map.1.as_ref(),
            IdlTypeSnapshot::Simple(value) if value == "string"
        ));
    }

    #[test]
    fn idl_snapshot_v1_rejects_unknown_normalization_versions() {
        let value = serde_json::json!({
            "normalizationVersion": 2,
            "name": "demo",
            "program_id": "11111111111111111111111111111111",
            "version": "1.0.0",
            "accounts": [],
            "instructions": [],
            "types": [],
            "events": [],
            "errors": [],
            "discriminant_size": 8
        });

        let error = serde_json::from_value::<IdlSnapshotV1>(value)
            .expect_err("unknown normalization version must fail");
        assert!(error
            .to_string()
            .contains("unsupported IDL normalization version 2"));
    }
}
