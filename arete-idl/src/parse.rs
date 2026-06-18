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
            let codama_root: CodamaRoot =
                serde_json::from_str(content).map_err(|codama_error| {
                    format!(
                        "Failed to parse IDL JSON as neither IdlSpec nor Codama root. \
                     IdlSpec error: {}. Codama error: {}",
                        parse_error, codama_error
                    )
                })?;
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

    // Index the program's top-level PDA definitions by name so instruction
    // accounts referencing them via `pdaLinkNode` can inline their seeds.
    let pda_defs: HashMap<&str, &CodamaPdaNode> = root
        .program
        .pdas
        .iter()
        .map(|pda| (pda.name.as_str(), pda))
        .collect();

    Ok(IdlSpec {
        version: root.program.version.clone(),
        name: Some(root.program.name.clone()),
        address: Some(root.program.public_key.clone()),
        instructions: root
            .program
            .instructions
            .iter()
            .map(|instruction| codama_instruction_to_idl(instruction, &pda_defs))
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
    let discriminator = match discriminators.get(&account.name).copied() {
        Some(value) => vec![value],
        None if !discriminators.is_empty() => {
            return Err(format!(
                "Codama account '{}' has no entry in accountDiscriminator; \
                 add a variant or remove the account from the IDL",
                account.name
            ));
        }
        None => Vec::new(),
    };

    Ok(IdlAccount {
        name: account.name.clone(),
        discriminator,
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

fn codama_instruction_to_idl(
    instruction: &CodamaInstruction,
    pda_defs: &HashMap<&str, &CodamaPdaNode>,
) -> Result<IdlInstruction, String> {
    let discriminant = instruction
        .arguments
        .iter()
        .find(|argument| argument.name == "discriminator")
        .map(|argument| match &argument.default_value {
            Some(CodamaValueNode::Number { number }) => Ok(SteelDiscriminant {
                type_: argument.number_format().unwrap_or("u8").to_string(),
                value: u64::try_from(*number).map_err(|_| {
                    format!(
                        "Codama instruction '{}' has a negative discriminator value {}; \
                         discriminants must be non-negative",
                        instruction.name, number
                    )
                })?,
            }),
            _ => Err(format!(
                "Codama instruction '{}' has a 'discriminator' argument without a numeric \
                 default_value; cannot determine its Steel discriminant",
                instruction.name
            )),
        })
        .transpose()?;

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
                is_signer: account.is_signer.as_bool(),
                address: account.default_public_key(),
                optional: account.is_optional,
                docs: {
                    let mut docs = account.docs.clone();
                    if account.is_signer.is_either() {
                        docs.push(
                            "signer: either (may or may not sign; treated as non-signer)"
                                .to_string(),
                        );
                    }
                    docs
                },
                pda: codama_account_pda(account, pda_defs),
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
            // Borsh encodes the variant index, so explicit discriminators are
            // only representable when they match the positional index.
            if let Some((index, variant)) = variants
                .iter()
                .enumerate()
                .find(|(i, v)| v.discriminator.is_some_and(|d| d != *i as u64))
            {
                return Err(format!(
                    "Codama enum variant '{}' has explicit discriminator {:?} != index {}, which Borsh enums cannot represent",
                    variant.name, variant.discriminator, index
                ));
            }

            Ok(IdlTypeDefKind::Enum {
                kind: "enum".to_string(),
                variants: variants
                    .iter()
                    .map(codama_enum_variant_to_idl)
                    .collect::<Result<Vec<_>, _>>()?,
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

/// Convert a Codama enum variant (empty, struct, or tuple) to an arete-idl
/// variant. Codama struct variants nest their fields under a `struct` type
/// node; tuple variants under `tuple.items`; the simplified flattened shape
/// puts fields directly on the variant.
fn codama_enum_variant_to_idl(variant: &CodamaEnumVariant) -> Result<IdlEnumVariant, String> {
    let mut fields: Vec<IdlEnumVariantField> = Vec::new();

    if !variant.fields.is_empty() {
        for field in &variant.fields {
            fields.push(IdlEnumVariantField::Named(codama_field_to_idl(field)?));
        }
    } else if let Some(struct_node) = &variant.struct_ {
        match struct_node.as_ref() {
            CodamaTypeNode::Struct {
                fields: struct_fields,
            } => {
                for field in struct_fields {
                    fields.push(IdlEnumVariantField::Named(codama_field_to_idl(field)?));
                }
            }
            other => {
                return Err(format!(
                    "Codama enum variant '{}' struct payload is not a structTypeNode: {:?}",
                    variant.name, other
                ))
            }
        }
    } else if let Some(tuple_node) = &variant.tuple {
        match tuple_node.as_ref() {
            CodamaTypeNode::Tuple { items } => {
                for item in items {
                    fields.push(IdlEnumVariantField::Tuple(codama_type_to_idl(item)?));
                }
            }
            other => {
                return Err(format!(
                    "Codama enum variant '{}' tuple payload is not a tupleTypeNode: {:?}",
                    variant.name, other
                ))
            }
        }
    }

    Ok(IdlEnumVariant {
        name: variant.name.clone(),
        fields,
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
        CodamaTypeNode::Array { item, count } => {
            let size = match count {
                CodamaCountNode::Fixed { value } => *value,
                CodamaCountNode::Unknown => {
                    return Err(
                        "unsupported Codama array count kind (only fixedCountNode is supported)"
                            .to_string(),
                    );
                }
            };
            Ok(IdlType::Array(IdlTypeArray {
                array: vec![
                    IdlTypeArrayElement::Nested(codama_type_to_idl(item)?),
                    IdlTypeArrayElement::Size(size),
                ],
            }))
        }
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
        CodamaTypeNode::Unknown => {
            Err("unsupported Codama type node kind (not modelled by arete-idl)".to_string())
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
    #[serde(default)]
    pdas: Vec<CodamaPdaNode>,
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
    is_signer: CodamaSignerFlag,
    #[serde(default)]
    is_writable: bool,
    #[serde(default)]
    is_optional: bool,
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

/// Codama's `isSigner` is `true`, `false`, or the string `"either"` (the
/// account may or may not sign depending on context). The runtime has no
/// maybe-signer concept, so `"either"` maps to non-signer and the fact is
/// surfaced via the account docs.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CodamaSignerFlag {
    Bool(bool),
    Tag(String),
}

impl Default for CodamaSignerFlag {
    fn default() -> Self {
        CodamaSignerFlag::Bool(false)
    }
}

impl CodamaSignerFlag {
    fn as_bool(&self) -> bool {
        matches!(self, CodamaSignerFlag::Bool(true))
    }

    fn is_either(&self) -> bool {
        matches!(self, CodamaSignerFlag::Tag(tag) if tag == "either")
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
    #[serde(rename = "tupleTypeNode")]
    Tuple { items: Vec<CodamaTypeNode> },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind")]
enum CodamaCountNode {
    #[serde(rename = "fixedCountNode")]
    Fixed { value: u32 },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct CodamaEnumVariant {
    name: String,
    #[serde(default)]
    discriminator: Option<u64>,
    /// Simplified/flattened variant shape: fields directly on the variant.
    #[serde(default)]
    fields: Vec<CodamaFieldNode>,
    /// `enumStructVariantTypeNode`: payload nested under a structTypeNode.
    #[serde(default, rename = "struct")]
    struct_: Option<Box<CodamaTypeNode>>,
    /// `enumTupleVariantTypeNode`: payload nested under a tupleTypeNode.
    #[serde(default)]
    tuple: Option<Box<CodamaTypeNode>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind")]
enum CodamaValueNode {
    #[serde(rename = "numberValueNode")]
    Number { number: i64 },
    #[serde(rename = "publicKeyValueNode")]
    PublicKey {
        #[serde(rename = "publicKey")]
        public_key: String,
    },
    #[serde(rename = "stringValueNode")]
    StringValue { string: String },
    #[serde(rename = "bytesValueNode")]
    BytesValue {
        data: String,
        #[serde(default)]
        encoding: Option<String>,
    },
    /// PDA-derived address: references a PDA definition and binds its variable
    /// seeds to instruction accounts/arguments.
    #[serde(rename = "pdaValueNode")]
    Pda {
        pda: CodamaPdaSource,
        #[serde(default)]
        seeds: Vec<CodamaPdaSeedValue>,
    },
    /// Reference to another instruction account by name (a seed binding).
    #[serde(rename = "accountValueNode")]
    Account { name: String },
    /// Reference to an instruction argument by name (a seed binding).
    #[serde(rename = "argumentValueNode")]
    Argument { name: String },
    #[serde(other)]
    Other,
}

/// A program-level Codama PDA definition (`pdaNode`).
#[derive(Debug, Deserialize)]
struct CodamaPdaNode {
    #[allow(dead_code)]
    name: String,
    #[serde(default)]
    seeds: Vec<CodamaPdaSeedNode>,
    /// Owning program (base58), when the PDA belongs to another program
    /// (e.g. associated token accounts). Defaults to the instruction's program.
    #[serde(rename = "programId", default)]
    program_id: Option<String>,
}

/// The `pda` field of a `pdaValueNode`: either a link to a top-level PDA
/// definition or an inline one.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind")]
enum CodamaPdaSource {
    #[serde(rename = "pdaLinkNode")]
    Link { name: String },
    #[serde(rename = "pdaNode")]
    Inline {
        #[allow(dead_code)]
        name: String,
        #[serde(default)]
        seeds: Vec<CodamaPdaSeedNode>,
        #[serde(rename = "programId", default)]
        program_id: Option<String>,
    },
}

/// A single seed in a `pdaNode` definition.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind")]
enum CodamaPdaSeedNode {
    #[serde(rename = "constantPdaSeedNode")]
    Constant {
        #[serde(rename = "type", default)]
        seed_type: Option<CodamaTypeNode>,
        value: CodamaValueNode,
    },
    #[serde(rename = "variablePdaSeedNode")]
    Variable {
        name: String,
        #[serde(rename = "type", default)]
        seed_type: Option<CodamaTypeNode>,
    },
}

/// A seed-value binding in a `pdaValueNode` (`pdaSeedValueNode`).
#[derive(Debug, Deserialize)]
struct CodamaPdaSeedValue {
    name: String,
    value: CodamaValueNode,
}

/// Build an `IdlPda` for an instruction account whose default value is a PDA.
///
/// Returns `None` (degrading to user-provided) when the PDA references an
/// unknown definition or uses seed kinds the runtime cannot represent.
fn codama_account_pda(
    account: &CodamaInstructionAccount,
    pda_defs: &HashMap<&str, &CodamaPdaNode>,
) -> Option<IdlPda> {
    let CodamaValueNode::Pda { pda, seeds } = account.default_value.as_ref()? else {
        return None;
    };

    let (pda_seeds, pda_program_id): (&[CodamaPdaSeedNode], Option<&str>) = match pda {
        CodamaPdaSource::Inline {
            seeds, program_id, ..
        } => (seeds.as_slice(), program_id.as_deref()),
        CodamaPdaSource::Link { name } => {
            let def = pda_defs.get(name.as_str())?;
            (def.seeds.as_slice(), def.program_id.as_deref())
        }
    };

    // Map variable seed names to their bound values for this account.
    let bindings: HashMap<&str, &CodamaValueNode> = seeds
        .iter()
        .map(|seed| (seed.name.as_str(), &seed.value))
        .collect();

    let mut idl_seeds = Vec::with_capacity(pda_seeds.len());
    for seed in pda_seeds {
        match seed {
            CodamaPdaSeedNode::Constant { seed_type, value } => {
                idl_seeds.push(IdlPdaSeed::Const {
                    value: codama_constant_seed_bytes(value, seed_type.as_ref())?,
                });
            }
            CodamaPdaSeedNode::Variable { name, seed_type } => {
                match bindings.get(name.as_str())? {
                    CodamaValueNode::Account { name } => idl_seeds.push(IdlPdaSeed::Account {
                        path: name.clone(),
                        account: None,
                    }),
                    CodamaValueNode::Argument { name } => idl_seeds.push(IdlPdaSeed::Arg {
                        path: name.clone(),
                        arg_type: seed_type.as_ref().and_then(codama_seed_type_name),
                    }),
                    // Constant bindings (rare) or unsupported value kinds. The
                    // variable seed's declared type drives the encoding width.
                    value @ (CodamaValueNode::StringValue { .. }
                    | CodamaValueNode::BytesValue { .. }
                    | CodamaValueNode::Number { .. }
                    | CodamaValueNode::PublicKey { .. }) => idl_seeds.push(IdlPdaSeed::Const {
                        value: codama_constant_seed_bytes(value, seed_type.as_ref())?,
                    }),
                    _ => return None,
                }
            }
        }
    }

    Some(IdlPda {
        seeds: idl_seeds,
        program: pda_program_id.map(|program_id| IdlPdaProgram::Literal {
            kind: "programId".to_string(),
            value: program_id.to_string(),
        }),
    })
}

/// Map a Codama seed type node to the simple type name carried by
/// `IdlPdaSeed::Arg` (e.g. `u64`, `publicKey`, `string`).
fn codama_seed_type_name(type_node: &CodamaTypeNode) -> Option<String> {
    match type_node {
        CodamaTypeNode::Number { format } => Some(format.clone()),
        CodamaTypeNode::PublicKey {} => Some("publicKey".to_string()),
        CodamaTypeNode::String {} => Some("string".to_string()),
        _ => None,
    }
}

/// Decode a Codama constant seed value into raw bytes.
///
/// Numbers are encoded little-endian at the width declared by the seed's type
/// node; an unknown or missing numeric type degrades (returns `None`) rather
/// than guessing a width and deriving a wrong address.
fn codama_constant_seed_bytes(
    value: &CodamaValueNode,
    seed_type: Option<&CodamaTypeNode>,
) -> Option<Vec<u8>> {
    match value {
        CodamaValueNode::StringValue { string } => Some(string.as_bytes().to_vec()),
        CodamaValueNode::Number { number } => {
            let Some(CodamaTypeNode::Number { format }) = seed_type else {
                return None;
            };
            let (width, signed) = match format.as_str() {
                "u8" => (1usize, false),
                "i8" => (1, true),
                "u16" => (2, false),
                "i16" => (2, true),
                "u32" => (4, false),
                "i32" => (4, true),
                "u64" => (8, false),
                "i64" => (8, true),
                "u128" => (16, false),
                "i128" => (16, true),
                _ => return None,
            };
            let bytes = number.to_le_bytes();
            let sign_byte: u8 = if signed && *number < 0 { 0xFF } else { 0x00 };
            if width <= bytes.len() {
                // Reject values that do not fit the declared width. For signed
                // widths the discarded high bytes must be a sign extension of
                // the kept top byte, whose sign bit must agree with `number`;
                // for unsigned widths they must simply be zero.
                if signed {
                    let top = bytes[width - 1];
                    if ((top & 0x80) == 0) != (*number >= 0) {
                        return None;
                    }
                }
                if bytes[width..].iter().any(|b| *b != sign_byte) {
                    return None;
                }
                Some(bytes[..width].to_vec())
            } else {
                // Widen (e.g. an i64 literal into an i128 slot): sign- or
                // zero-extend rather than pad with zeros unconditionally.
                let mut out = bytes.to_vec();
                out.resize(width, sign_byte);
                Some(out)
            }
        }
        CodamaValueNode::PublicKey { public_key } => {
            let decoded = bs58::decode(public_key).into_vec().ok()?;
            (decoded.len() == 32).then_some(decoded)
        }
        CodamaValueNode::BytesValue { data, encoding } => match encoding.as_deref() {
            Some("base16") | Some("hex") => decode_hex(data),
            Some("utf8") | None => Some(data.as_bytes().to_vec()),
            // base64/base58 seeds are uncommon; degrade rather than guess.
            _ => None,
        },
        _ => None,
    }
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
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
    fn test_codama_pda_link_node_resolves_seeds() {
        let json = r#"{
            "kind": "rootNode",
            "program": {
                "kind": "programNode",
                "name": "subscriptions",
                "publicKey": "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44",
                "version": "0.1.0",
                "pdas": [
                    {
                        "kind": "pdaNode",
                        "name": "subscriptionAuthority",
                        "seeds": [
                            { "kind": "constantPdaSeedNode", "type": { "kind": "stringTypeNode", "encoding": "utf8" }, "value": { "kind": "stringValueNode", "string": "SubscriptionAuthority" } },
                            { "kind": "variablePdaSeedNode", "name": "user", "type": { "kind": "publicKeyTypeNode" } },
                            { "kind": "variablePdaSeedNode", "name": "tokenMint", "type": { "kind": "publicKeyTypeNode" } }
                        ]
                    }
                ],
                "instructions": [
                    {
                        "kind": "instructionNode",
                        "name": "initSubscriptionAuthority",
                        "accounts": [
                            { "kind": "instructionAccountNode", "name": "owner", "isSigner": true, "isWritable": true },
                            {
                                "kind": "instructionAccountNode",
                                "name": "subscriptionAuthority",
                                "isSigner": false,
                                "isWritable": true,
                                "defaultValue": {
                                    "kind": "pdaValueNode",
                                    "pda": { "kind": "pdaLinkNode", "name": "subscriptionAuthority" },
                                    "seeds": [
                                        { "kind": "pdaSeedValueNode", "name": "user", "value": { "kind": "accountValueNode", "name": "owner" } },
                                        { "kind": "pdaSeedValueNode", "name": "tokenMint", "value": { "kind": "accountValueNode", "name": "tokenMint" } }
                                    ]
                                }
                            },
                            { "kind": "instructionAccountNode", "name": "tokenMint", "isSigner": false, "isWritable": false }
                        ],
                        "arguments": [
                            { "kind": "instructionArgumentNode", "name": "discriminator", "type": { "kind": "numberTypeNode", "format": "u8" }, "defaultValue": { "kind": "numberValueNode", "number": 0 } }
                        ]
                    }
                ]
            }
        }"#;

        let spec = parse_idl_content(json).expect("codama idl should parse");
        let ix = &spec.instructions[0];
        let pda_account = ix
            .accounts
            .iter()
            .find(|a| a.name == "subscriptionAuthority")
            .expect("subscriptionAuthority account present");

        let pda = pda_account.pda.as_ref().expect("PDA should be resolved");
        assert_eq!(pda.seeds.len(), 3);
        match &pda.seeds[0] {
            IdlPdaSeed::Const { value } => assert_eq!(value, b"SubscriptionAuthority"),
            other => panic!("expected const seed, got {:?}", other),
        }
        match &pda.seeds[1] {
            IdlPdaSeed::Account { path, .. } => assert_eq!(path, "owner"),
            other => panic!("expected account seed bound to owner, got {:?}", other),
        }
        match &pda.seeds[2] {
            IdlPdaSeed::Account { path, .. } => assert_eq!(path, "tokenMint"),
            other => panic!("expected account seed bound to tokenMint, got {:?}", other),
        }

        // Non-PDA accounts stay unresolved.
        let owner = ix.accounts.iter().find(|a| a.name == "owner").unwrap();
        assert!(owner.pda.is_none());
    }

    #[test]
    fn test_codama_fielded_enums_parse() {
        let json = r#"{
            "kind": "rootNode",
            "program": {
                "kind": "programNode",
                "name": "demo",
                "publicKey": "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44",
                "version": "0.1.0",
                "definedTypes": [
                    {
                        "kind": "definedTypeNode",
                        "name": "op",
                        "type": {
                            "kind": "enumTypeNode",
                            "variants": [
                                { "kind": "enumEmptyVariantTypeNode", "name": "noop" },
                                {
                                    "kind": "enumStructVariantTypeNode",
                                    "name": "transfer",
                                    "struct": {
                                        "kind": "structTypeNode",
                                        "fields": [
                                            { "kind": "structFieldTypeNode", "name": "amount", "type": { "kind": "numberTypeNode", "format": "u64" } }
                                        ]
                                    }
                                },
                                {
                                    "kind": "enumTupleVariantTypeNode",
                                    "name": "pair",
                                    "tuple": {
                                        "kind": "tupleTypeNode",
                                        "items": [
                                            { "kind": "numberTypeNode", "format": "u8" },
                                            { "kind": "numberTypeNode", "format": "u16" }
                                        ]
                                    }
                                }
                            ]
                        }
                    }
                ],
                "instructions": []
            }
        }"#;

        let spec = parse_idl_content(json).expect("codama idl with fielded enum should parse");
        let op = spec.types.iter().find(|t| t.name == "op").expect("op type");
        let IdlTypeDefKind::Enum { variants, .. } = &op.type_def else {
            panic!("expected enum, got {:?}", op.type_def);
        };
        assert_eq!(variants.len(), 3);
        assert!(variants[0].fields.is_empty());
        match &variants[1].fields[..] {
            [IdlEnumVariantField::Named(field)] => assert_eq!(field.name, "amount"),
            other => panic!("expected one named field, got {:?}", other),
        }
        match &variants[2].fields[..] {
            [IdlEnumVariantField::Tuple(_), IdlEnumVariantField::Tuple(_)] => {}
            other => panic!("expected two tuple fields, got {:?}", other),
        }
    }

    #[test]
    fn test_codama_is_signer_either_tolerated() {
        let json = r#"{
            "kind": "rootNode",
            "program": {
                "kind": "programNode",
                "name": "demo",
                "publicKey": "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44",
                "version": "0.1.0",
                "instructions": [
                    {
                        "kind": "instructionNode",
                        "name": "doThing",
                        "accounts": [
                            { "kind": "instructionAccountNode", "name": "payer", "isSigner": true, "isWritable": true },
                            { "kind": "instructionAccountNode", "name": "authority", "isSigner": "either", "isWritable": false }
                        ],
                        "arguments": []
                    }
                ]
            }
        }"#;

        let spec = parse_idl_content(json).expect("idl with isSigner either should parse");
        let ix = &spec.instructions[0];
        let payer = ix.accounts.iter().find(|a| a.name == "payer").unwrap();
        assert!(payer.is_signer);
        let authority = ix.accounts.iter().find(|a| a.name == "authority").unwrap();
        assert!(!authority.is_signer, "either maps to non-signer");
        assert!(
            authority.docs.iter().any(|d| d.contains("either")),
            "either is surfaced in docs"
        );
    }

    #[test]
    fn test_codama_typed_seeds_program_id_and_optional_accounts() {
        let json = r#"{
            "kind": "rootNode",
            "program": {
                "kind": "programNode",
                "name": "vaults",
                "publicKey": "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44",
                "version": "0.1.0",
                "instructions": [
                    {
                        "kind": "instructionNode",
                        "name": "deposit",
                        "accounts": [
                            { "kind": "instructionAccountNode", "name": "owner", "isSigner": true, "isWritable": true },
                            { "kind": "instructionAccountNode", "name": "referrer", "isSigner": false, "isWritable": false, "isOptional": true },
                            {
                                "kind": "instructionAccountNode",
                                "name": "vault",
                                "isSigner": false,
                                "isWritable": true,
                                "defaultValue": {
                                    "kind": "pdaValueNode",
                                    "pda": {
                                        "kind": "pdaNode",
                                        "name": "vault",
                                        "programId": "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
                                        "seeds": [
                                            { "kind": "constantPdaSeedNode", "type": { "kind": "numberTypeNode", "format": "u8" }, "value": { "kind": "numberValueNode", "number": 7 } },
                                            { "kind": "constantPdaSeedNode", "type": { "kind": "publicKeyTypeNode" }, "value": { "kind": "publicKeyValueNode", "publicKey": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" } },
                                            { "kind": "variablePdaSeedNode", "name": "amount", "type": { "kind": "numberTypeNode", "format": "u64" } }
                                        ]
                                    },
                                    "seeds": [
                                        { "kind": "pdaSeedValueNode", "name": "amount", "value": { "kind": "argumentValueNode", "name": "amount" } }
                                    ]
                                }
                            }
                        ],
                        "arguments": [
                            { "kind": "instructionArgumentNode", "name": "discriminator", "type": { "kind": "numberTypeNode", "format": "u8" }, "defaultValue": { "kind": "numberValueNode", "number": 0 } },
                            { "kind": "instructionArgumentNode", "name": "amount", "type": { "kind": "numberTypeNode", "format": "u64" } }
                        ]
                    }
                ]
            }
        }"#;

        let spec = parse_idl_content(json).expect("codama idl should parse");
        let ix = &spec.instructions[0];

        let referrer = ix.accounts.iter().find(|a| a.name == "referrer").unwrap();
        assert!(referrer.optional, "isOptional should be parsed");

        let vault = ix.accounts.iter().find(|a| a.name == "vault").unwrap();
        let pda = vault.pda.as_ref().expect("vault PDA should be resolved");

        // u8 constant seed encodes at its declared width, not 8 bytes.
        match &pda.seeds[0] {
            IdlPdaSeed::Const { value } => assert_eq!(value, &vec![7u8]),
            other => panic!("expected 1-byte const seed, got {:?}", other),
        }
        // Pubkey constant seeds decode to their 32 raw bytes.
        match &pda.seeds[1] {
            IdlPdaSeed::Const { value } => assert_eq!(value.len(), 32),
            other => panic!("expected 32-byte pubkey seed, got {:?}", other),
        }
        // Argument-bound seeds carry the declared type for width-aware encoding.
        match &pda.seeds[2] {
            IdlPdaSeed::Arg { path, arg_type } => {
                assert_eq!(path, "amount");
                assert_eq!(arg_type.as_deref(), Some("u64"));
            }
            other => panic!("expected arg seed, got {:?}", other),
        }
        // Cross-program PDAs keep their owning program.
        match pda.program.as_ref().expect("program id should be kept") {
            IdlPdaProgram::Literal { value, .. } => {
                assert_eq!(value, "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL")
            }
            other => panic!("expected literal program id, got {:?}", other),
        }
    }

    #[test]
    fn test_real_subscriptions_idl_resolves_pdas() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../stacks/subscriptions/idl/subscriptions.json"
        );
        let json = match std::fs::read_to_string(path) {
            Ok(c) => c,
            // The stack IDL is not part of this crate; skip if unavailable.
            Err(_) => return,
        };

        let spec = parse_idl_content(&json).expect("subscriptions idl should parse");
        let ix = spec
            .instructions
            .iter()
            .find(|i| i.name == "initSubscriptionAuthority")
            .expect("initSubscriptionAuthority instruction present");
        let pda_account = ix
            .accounts
            .iter()
            .find(|a| a.name == "subscriptionAuthority")
            .expect("subscriptionAuthority account present");
        let pda = pda_account
            .pda
            .as_ref()
            .expect("Codama defaultValue PDA should be resolved from the real IDL");
        assert!(
            pda.seeds.len() >= 2,
            "expected resolved seeds, got {:?}",
            pda.seeds
        );
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
