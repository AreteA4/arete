//! Snapshot type definitions

use serde::{de::Error, Deserialize, Deserializer, Serialize};

use crate::types::SteelDiscriminant;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default = "default_discriminant_size")]
    pub discriminant_size: usize,
}

fn default_discriminant_size() -> usize {
    8
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlAccountSnapshot {
    pub name: String,
    pub discriminator: Vec<u8>,
    #[serde(default)]
    pub docs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serialization: Option<IdlSerializationSnapshot>,
    /// Account fields - populated from inline type definition
    #[serde(default)]
    pub fields: Vec<IdlFieldSnapshot>,
    /// Inline type definition (for Steel format with type.fields structure)
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub type_def: Option<IdlInlineTypeDef>,
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
            let value = disc.value as u8;
            return vec![value, 0, 0, 0, 0, 0, 0, 0];
        }

        crate::discriminator::anchor_discriminator(&format!("global:{}", self.name))
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlEventSnapshot {
    pub name: String,
    pub discriminator: Vec<u8>,
    #[serde(default)]
    pub docs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IdlErrorSnapshot {
    pub code: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    }],
                },
            }],
            events: vec![IdlEventSnapshot {
                name: "ExampleEvent".to_string(),
                discriminator: vec![0, 0, 0, 0, 0, 0, 0, 1],
                docs: vec![],
            }],
            errors: vec![IdlErrorSnapshot {
                code: 6000,
                name: "ExampleError".to_string(),
                msg: Some("example".to_string()),
            }],
            discriminant_size: default_discriminant_size(),
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
}
