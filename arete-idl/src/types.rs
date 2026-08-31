//! Core type definitions for IDL

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlSpec {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
    pub instructions: Vec<IdlInstruction>,
    #[serde(default)]
    pub accounts: Vec<IdlAccount>,
    #[serde(default)]
    pub types: Vec<IdlTypeDef>,
    #[serde(default)]
    pub events: Vec<IdlEvent>,
    #[serde(default)]
    pub errors: Vec<IdlError>,
    #[serde(default)]
    pub constants: Vec<IdlConstant>,
    #[serde(default)]
    pub pdas: Vec<IdlNamedPda>,
    pub metadata: Option<IdlMetadata>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlConstant {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: IdlType,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlMetadata {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub spec: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
}

/// Steel-style discriminant format: {"type": "u8", "value": N}
///
/// Fields are private so [`try_new`](Self::try_new) is the only way to build one. A struct literal
/// bypassed the range check when it lived in the deserializer alone, which is how a Codama IDL
/// reached `{"type": "u8", "value": 256}` and encoded it as `[0]`.
#[derive(Debug, Clone, Serialize)]
pub struct SteelDiscriminant {
    #[serde(rename = "type")]
    type_: String,
    value: u64,
}

impl SteelDiscriminant {
    /// The only constructor. Rejects a value that does not fit its declared type rather than
    /// truncating it: `{"type": "u32", "value": 4294967296}` would encode as `[0, 0, 0, 0]` and
    /// collide with whichever instruction genuinely declares discriminant zero — two instructions
    /// sharing a tag, decided silently at parse time.
    pub fn try_new(type_: impl Into<String>, value: u64) -> Result<Self, String> {
        let type_ = type_.into();
        let max = Self::max_for_width(Self::width_of(&type_));
        if value > max {
            return Err(format!(
                "discriminant value {value} does not fit its declared type {type_} (max {max})"
            ));
        }

        Ok(Self { type_, value })
    }

    /// Declared type name, e.g. `"u32"`.
    pub fn declared_type(&self) -> &str {
        &self.type_
    }

    /// The tag's numeric value.
    pub fn value(&self) -> u64 {
        self.value
    }

    /// Encoded width in bytes, taken from the declared type.
    ///
    /// Steel writes a single byte. Bincode-encoded native programs (System Program, Address
    /// Lookup Table) write a little-endian `u32` enum tag instead, so the width cannot be
    /// assumed. An unrecognised type falls back to one byte, which is what every Steel IDL
    /// declares.
    pub fn width(&self) -> usize {
        Self::width_of(&self.type_)
    }

    /// Width for a declared type name, for callers holding the name rather than the struct: the
    /// snapshot deserializer infers a width before any `SteelDiscriminant` exists.
    pub fn width_of(type_: &str) -> usize {
        match type_ {
            "u16" => 2,
            "u32" => 4,
            "u64" => 8,
            _ => 1,
        }
    }

    /// Largest value the declared type can hold.
    fn max_for_width(width: usize) -> u64 {
        // `1u64 << 64` overflows, so u64 is named rather than computed.
        if width >= 8 {
            u64::MAX
        } else {
            (1u64 << (width * 8)) - 1
        }
    }

    /// The tag as it appears on the wire: little-endian, truncated to [`width`](Self::width).
    ///
    /// Truncation is safe because construction rejects a value wider than its declared type.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.value.to_le_bytes()[..self.width()].to_vec()
    }
}

/// Routes through [`SteelDiscriminant::try_new`] so a parsed IDL cannot carry a tag its declared
/// type could not hold. Mirrored by `parseDiscriminant` in the TypeScript hash implementation.
impl<'de> Deserialize<'de> for SteelDiscriminant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(rename = "type")]
            type_: String,
            value: u64,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::try_new(wire.type_, wire.value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlInstruction {
    pub name: String,
    /// Anchor-style discriminator: 8-byte array
    #[serde(default)]
    pub discriminator: Vec<u8>,
    /// Steel-style discriminant: {"type": "u8", "value": N}
    #[serde(default)]
    pub discriminant: Option<SteelDiscriminant>,
    #[serde(default)]
    pub docs: Vec<String>,
    pub accounts: Vec<IdlAccountArg>,
    pub args: Vec<IdlField>,
}

impl IdlInstruction {
    pub fn get_discriminator(&self) -> Vec<u8> {
        if !self.discriminator.is_empty() {
            return self.discriminator.clone();
        }

        if let Some(disc) = &self.discriminant {
            return disc.to_bytes();
        }

        crate::discriminator::anchor_discriminator(&format!("global:{}", to_snake_case(&self.name)))
    }

    pub fn flattened_accounts(&self) -> Vec<IdlAccountArg> {
        self.accounts
            .iter()
            .flat_map(|account| account.flattened(None))
            .collect()
    }
}

/// PDA definition in Anchor IDL format
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlPda {
    #[serde(default)]
    pub name: Option<String>,
    pub seeds: Vec<IdlPdaSeed>,
    #[serde(default)]
    pub program: Option<IdlPdaProgram>,
}

/// Named PDA definition at the program/IDL level.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlNamedPda {
    pub name: String,
    pub seeds: Vec<IdlPdaSeed>,
    #[serde(default)]
    pub program: Option<IdlPdaProgram>,
}

/// PDA seed in Anchor IDL format
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum IdlPdaSeed {
    /// Constant byte array seed
    Const { value: Vec<u8> },
    /// Reference to another account in the instruction
    Account {
        path: String,
        #[serde(default)]
        account: Option<String>,
    },
    /// Reference to an instruction argument
    Arg {
        path: String,
        #[serde(rename = "type", default)]
        arg_type: Option<String>,
    },
}

/// Program reference for cross-program PDAs
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum IdlPdaProgram {
    /// Reference to another account that holds the program ID
    Account { kind: String, path: String },
    /// Literal program ID
    Literal { kind: String, value: String },
    /// Constant program ID as bytes
    Const { kind: String, value: Vec<u8> },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlAccountArg {
    pub name: String,
    #[serde(rename = "isMut", alias = "writable", default)]
    pub is_mut: bool,
    #[serde(rename = "isSigner", alias = "signer", default)]
    pub is_signer: bool,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default, alias = "isOptional")]
    pub optional: bool,
    #[serde(default)]
    pub docs: Vec<String>,
    #[serde(default)]
    pub pda: Option<IdlPda>,
    #[serde(default)]
    pub accounts: Vec<IdlAccountArg>,
}

impl IdlAccountArg {
    pub fn flattened(&self, prefix: Option<&str>) -> Vec<IdlAccountArg> {
        self.flattened_with_siblings(prefix, None)
    }

    fn flattened_with_siblings(
        &self,
        prefix: Option<&str>,
        sibling_names: Option<&HashSet<String>>,
    ) -> Vec<IdlAccountArg> {
        let flattened_name = match prefix {
            Some(prefix) => format!("{}{}", prefix, to_pascal_case(&self.name)),
            None => self.name.clone(),
        };

        if self.accounts.is_empty() {
            let mut account = self.clone();
            account.name = flattened_name;
            account.accounts = Vec::new();
            if let Some(pda) = account.pda.as_mut() {
                if prefix.is_some() {
                    pda.name = None;
                }
                for seed in &mut pda.seeds {
                    if let IdlPdaSeed::Account { path, .. } = seed {
                        if let (Some(prefix), Some(siblings)) = (prefix, sibling_names) {
                            if siblings.contains(path.as_str()) {
                                *path = format!("{}{}", prefix, to_pascal_case(path));
                            }
                        }
                    }
                }
            }
            return vec![account];
        }

        let sibling_names: HashSet<String> = self
            .accounts
            .iter()
            .map(|account| account.name.clone())
            .collect();

        self.accounts
            .iter()
            .flat_map(|account| {
                account.flattened_with_siblings(Some(&flattened_name), Some(&sibling_names))
            })
            .collect()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlAccount {
    pub name: String,
    #[serde(default)]
    pub discriminator: Vec<u8>,
    #[serde(default)]
    pub docs: Vec<String>,
    /// Steel format embedded type definition
    #[serde(rename = "type", default)]
    pub type_def: Option<IdlTypeDefKind>,
}

impl IdlAccount {
    pub fn get_discriminator(&self) -> Vec<u8> {
        if !self.discriminator.is_empty() {
            return self.discriminator.clone();
        }

        crate::discriminator::anchor_discriminator(&format!("account:{}", self.name))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlTypeDefStruct {
    pub kind: String,
    pub fields: Vec<IdlField>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlField {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: IdlType,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "amountHint"
    )]
    pub amount_hint: Option<IdlAmountHint>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdlAmountHint {
    pub decimals_source: IdlAmountDecimalsSource,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum IdlAmountDecimalsSource {
    ArgMint { arg_name: String },
    ArgDecimals { arg_name: String },
    KnownAccount { account_name: String },
    Constant { decimals: u8 },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum IdlType {
    Simple(String),
    Array(IdlTypeArray),
    Option(IdlTypeOption),
    Vec(IdlTypeVec),
    HashMap(IdlTypeHashMap),
    Tuple(IdlTypeTuple),
    Defined(IdlTypeDefined),
}

/// Inline tuple type, e.g. `{"tuple": [{"defined": "A"}, "u64"]}`.
///
/// Emitted by Codama/Kinobi legacy-Anchor renders (e.g. mpl-core) for
/// `Vec<(A, B)>`-style fields. Borsh encodes a tuple as its elements in
/// declaration order, so this is fully representable with no loss.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlTypeTuple {
    pub tuple: Vec<IdlType>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlTypeOption {
    pub option: Box<IdlType>,
}

/// Width of the length prefix on a variable-length sequence. Borsh (Anchor, Steel) uses
/// `u32`; bincode, used by the native Solana programs, uses `u64`. Absent means `U32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IdlLengthPrefix {
    U32,
    U64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlTypeVec {
    pub vec: Box<IdlType>,
    /// `{"vec": T, "lengthPrefix": "u64"}` selects the bincode-style prefix.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "lengthPrefix"
    )]
    pub length_prefix: Option<IdlLengthPrefix>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlTypeHashMap {
    #[serde(alias = "bTreeMap")]
    #[serde(rename = "hashMap")]
    pub hash_map: (Box<IdlType>, Box<IdlType>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlTypeArray {
    pub array: Vec<IdlTypeArrayElement>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum IdlTypeArrayElement {
    Nested(IdlType),
    Type(String),
    Size(u32),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlTypeDefined {
    pub defined: IdlTypeDefinedInner,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum IdlTypeDefinedInner {
    Named { name: String },
    Simple(String),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlRepr {
    pub kind: String,
    #[serde(default)]
    pub packed: Option<bool>,
}

/// Account serialization format as specified in the IDL.
/// Defaults to Borsh when not specified.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IdlSerialization {
    #[default]
    Borsh,
    Bytemuck,
    #[serde(alias = "bytemuckunsafe")]
    BytemuckUnsafe,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlTypeDef {
    pub name: String,
    #[serde(default)]
    pub docs: Vec<String>,
    /// Serialization format: "borsh" (default), "bytemuck", or "bytemuckunsafe"
    #[serde(default)]
    pub serialization: Option<IdlSerialization>,
    /// Repr annotation for zero-copy types (e.g., {"kind": "c"})
    #[serde(default)]
    pub repr: Option<IdlRepr>,
    #[serde(rename = "type")]
    pub type_def: IdlTypeDefKind,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum IdlTypeDefKind {
    Struct {
        kind: String,
        fields: Vec<IdlField>,
    },
    TupleStruct {
        kind: String,
        fields: Vec<IdlType>,
    },
    Enum {
        kind: String,
        variants: Vec<IdlEnumVariant>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlEnumVariant {
    pub name: String,
    /// Variant payload, when the variant carries data. Empty for fieldless
    /// variants — and skipped during serialization so their wire format is
    /// unchanged from before this field existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<IdlEnumVariantField>,
}

/// One field of a data-carrying enum variant. Anchor IDLs encode struct
/// variants as `{name, type}` objects and tuple variants as bare types.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum IdlEnumVariantField {
    Named(IdlField),
    Tuple(IdlType),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlEventDataRef {
    #[serde(default)]
    pub kind: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IdlEventFieldSource {
    Inline,
    EventDataType { type_name: String },
    MatchingType { type_name: String },
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct ResolvedEventFields<'a> {
    pub source: IdlEventFieldSource,
    pub fields: Vec<&'a IdlField>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlEvent {
    pub name: String,
    #[serde(default)]
    pub discriminator: Vec<u8>,
    #[serde(default)]
    pub docs: Vec<String>,
    #[serde(default)]
    pub fields: Vec<IdlField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<IdlEventDataRef>,
}

impl IdlEvent {
    pub fn get_discriminator(&self) -> Vec<u8> {
        if !self.discriminator.is_empty() {
            return self.discriminator.clone();
        }
        crate::discriminator::anchor_discriminator(&format!("event:{}", self.name))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdlError {
    pub code: u32,
    pub name: String,
    #[serde(default)]
    pub msg: Option<String>,
}

pub type IdlInstructionAccount = IdlAccountArg;
pub type IdlInstructionArg = IdlField;
pub type IdlTypeDefTy = IdlTypeDefKind;

impl IdlSpec {
    pub fn get_name(&self) -> &str {
        self.name
            .as_deref()
            .or_else(|| self.metadata.as_ref().and_then(|m| m.name.as_deref()))
            .unwrap_or("unknown")
    }

    pub fn get_version(&self) -> &str {
        self.version
            .as_deref()
            .or_else(|| self.metadata.as_ref().and_then(|m| m.version.as_deref()))
            .unwrap_or("0.1.0")
    }

    /// Width in bytes of the instruction discriminator this IDL's instructions share.
    ///
    /// Anchor IDLs carry an 8-byte `discriminator`. Steel and bincode IDLs declare a
    /// `discriminant` whose type gives the width, so it is read rather than assumed: a native
    /// program's `u32` enum tag is four bytes, not one.
    pub fn instruction_discriminator_size(&self) -> usize {
        self.instructions
            .iter()
            .filter(|ix| ix.discriminator.is_empty())
            .find_map(|ix| ix.discriminant.as_ref())
            .map_or(8, SteelDiscriminant::width)
    }

    pub fn find_event(&self, event_name: &str) -> Option<&IdlEvent> {
        self.events
            .iter()
            .find(|event| event.name.eq_ignore_ascii_case(event_name))
    }

    pub fn resolve_event_fields<'a>(&'a self, event_name: &str) -> Option<ResolvedEventFields<'a>> {
        self.find_event(event_name)
            .map(|event| self.resolve_event_fields_for(event))
    }

    pub fn resolve_event_fields_for<'a>(&'a self, event: &'a IdlEvent) -> ResolvedEventFields<'a> {
        if !event.fields.is_empty() {
            return ResolvedEventFields {
                source: IdlEventFieldSource::Inline,
                fields: event.fields.iter().collect(),
            };
        }

        if let Some(data) = &event.data {
            if let Some(fields) = self.lookup_struct_fields(&data.name) {
                return ResolvedEventFields {
                    source: IdlEventFieldSource::EventDataType {
                        type_name: data.name.clone(),
                    },
                    fields,
                };
            }
        }

        if let Some(fields) = self.lookup_struct_fields(&event.name) {
            return ResolvedEventFields {
                source: IdlEventFieldSource::MatchingType {
                    type_name: event.name.clone(),
                },
                fields,
            };
        }

        ResolvedEventFields {
            source: IdlEventFieldSource::Unavailable,
            fields: Vec::new(),
        }
    }

    fn lookup_struct_fields<'a>(&'a self, type_name: &str) -> Option<Vec<&'a IdlField>> {
        self.types
            .iter()
            .find(|ty| ty.name.eq_ignore_ascii_case(type_name))
            .and_then(|ty| match &ty.type_def {
                IdlTypeDefKind::Struct { fields, .. } => Some(fields.iter().collect()),
                _ => None,
            })
    }

    /// Check if a field is an account (vs an arg/data field) for a given instruction
    /// Returns Some("accounts") if it's an account, Some("data") if it's an arg, None if not found
    pub fn get_instruction_field_prefix(
        &self,
        instruction_name: &str,
        field_name: &str,
    ) -> Option<&'static str> {
        let normalized_name = to_snake_case(instruction_name);

        for instruction in &self.instructions {
            if instruction.name == normalized_name
                || instruction.name.eq_ignore_ascii_case(instruction_name)
            {
                for account in instruction.flattened_accounts() {
                    if account.name == field_name {
                        return Some("accounts");
                    }
                }
                for arg in &instruction.args {
                    if arg.name == field_name {
                        return Some("data");
                    }
                }
                return None;
            }
        }
        None
    }

    /// Get the discriminator bytes for an instruction by name
    pub fn get_instruction_discriminator(&self, instruction_name: &str) -> Option<Vec<u8>> {
        let normalized_name = to_snake_case(instruction_name);
        for instruction in &self.instructions {
            if instruction.name == normalized_name {
                let disc = instruction.get_discriminator();
                if !disc.is_empty() {
                    return Some(disc);
                }
            }
        }
        None
    }
}

pub fn to_snake_case(s: &str) -> String {
    let mut result = String::new();

    for c in s.chars() {
        if c.is_uppercase() {
            if !result.is_empty() {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap());
        } else {
            result.push(c);
        }
    }

    result
}

pub fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

#[cfg(test)]
mod length_prefix_tests {
    use super::*;

    #[test]
    fn parses_u64_length_prefix_on_a_vec() {
        let t: IdlType = serde_json::from_str(r#"{"vec":"publicKey","lengthPrefix":"u64"}"#)
            .expect("parse u64 vec");
        match t {
            IdlType::Vec(v) => {
                assert_eq!(v.length_prefix, Some(IdlLengthPrefix::U64));
            }
            other => panic!("expected Vec, got {other:?}"),
        }
    }

    #[test]
    fn plain_vec_defaults_to_no_prefix_and_round_trips_unchanged() {
        let t: IdlType = serde_json::from_str(r#"{"vec":"publicKey"}"#).expect("parse vec");
        match &t {
            IdlType::Vec(v) => assert_eq!(v.length_prefix, None),
            other => panic!("expected Vec, got {other:?}"),
        }
        // Hash stability: an ordinary vec must serialize byte-identically to before.
        assert_eq!(serde_json::to_string(&t).unwrap(), r#"{"vec":"publicKey"}"#);
    }
}

#[cfg(test)]
mod discriminant_width_tests {
    use super::*;

    fn instruction(discriminant: &str) -> IdlInstruction {
        serde_json::from_str(&format!(
            r#"{{"name":"extend_lookup_table","discriminant":{discriminant},"accounts":[],"args":[]}}"#
        ))
        .expect("parse instruction")
    }

    /// Every Steel IDL in the catalog declares `u8`, so this path must stay one byte.
    #[test]
    fn a_u8_discriminant_stays_a_single_byte() {
        let ix = instruction(r#"{"type":"u8","value":7}"#);
        assert_eq!(ix.get_discriminator(), vec![7]);
    }

    /// Bincode encodes an enum tag as little-endian `u32`. Emitting one byte would leave the
    /// payload three bytes short of where the program reads it.
    #[test]
    fn a_u32_discriminant_is_four_little_endian_bytes() {
        let ix = instruction(r#"{"type":"u32","value":2}"#);
        assert_eq!(ix.get_discriminator(), vec![2, 0, 0, 0]);
    }

    /// A value past one byte must not wrap into a different instruction's tag.
    #[test]
    fn a_wide_value_is_not_truncated_into_a_collision() {
        let ix = instruction(r#"{"type":"u32","value":256}"#);
        assert_eq!(ix.get_discriminator(), vec![0, 1, 0, 0]);
    }

    /// The high half of a `u64` tag must survive. The TypeScript mirror of this encoding reaches
    /// for bitwise shifts, where a count of 32 or more wraps and repeats the low bytes; the
    /// shared `idl-u64-discriminant` vector holds both languages to this result.
    #[test]
    fn a_u64_discriminant_keeps_its_high_bytes() {
        let ix = instruction(r#"{"type":"u64","value":4328719365}"#);
        assert_eq!(ix.get_discriminator(), vec![5, 4, 3, 2, 1, 0, 0, 0]);
    }

    #[test]
    fn an_unknown_type_falls_back_to_one_byte() {
        let ix = instruction(r#"{"type":"nonsense","value":3}"#);
        assert_eq!(ix.get_discriminator(), vec![3]);
    }

    #[test]
    fn an_anchor_discriminator_wins_over_a_declared_discriminant() {
        let ix: IdlInstruction = serde_json::from_str(
            r#"{"name":"buy","discriminator":[1,2,3,4,5,6,7,8],
                "discriminant":{"type":"u32","value":9},"accounts":[],"args":[]}"#,
        )
        .expect("parse instruction");
        assert_eq!(ix.get_discriminator(), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    fn spec(instructions: &str) -> IdlSpec {
        serde_json::from_str(&format!(r#"{{"name":"p","instructions":{instructions}}}"#))
            .expect("parse spec")
    }

    #[test]
    fn spec_size_follows_the_declared_width() {
        let idl = spec(
            r#"[{"name":"a","discriminant":{"type":"u32","value":0},"accounts":[],"args":[]}]"#,
        );
        assert_eq!(idl.instruction_discriminator_size(), 4);
    }

    #[test]
    fn spec_size_defaults_to_eight_for_anchor() {
        let idl =
            spec(r#"[{"name":"a","discriminator":[1,2,3,4,5,6,7,8],"accounts":[],"args":[]}]"#);
        assert_eq!(idl.instruction_discriminator_size(), 8);
    }

    /// A value wider than its declared type used to be truncated: `u32` with 2^32 became
    /// `[0, 0, 0, 0]`, silently colliding with whichever instruction declares zero. Rejecting at
    /// deserialization means no caller can reach the collision.
    #[test]
    fn a_value_wider_than_its_declared_type_is_rejected() {
        let error = serde_json::from_str::<IdlInstruction>(
            r#"{"name":"a","discriminant":{"type":"u32","value":4294967296},"accounts":[],"args":[]}"#,
        )
        .expect_err("2^32 does not fit a u32");

        assert!(
            error.to_string().contains("does not fit its declared type"),
            "unexpected error: {error}"
        );
    }

    /// The boundary itself must still parse, or every legitimate maximum tag is refused.
    #[test]
    fn the_largest_value_for_a_declared_type_still_parses() {
        let ix = instruction(r#"{"type":"u32","value":4294967295}"#);
        assert_eq!(ix.get_discriminator(), vec![255, 255, 255, 255]);

        let byte = instruction(r#"{"type":"u8","value":255}"#);
        assert_eq!(byte.get_discriminator(), vec![255]);
    }

    /// A `u8` declaration with 256 is the same defect one width down.
    #[test]
    fn a_byte_declaration_rejects_a_value_above_255() {
        assert!(serde_json::from_str::<IdlInstruction>(
            r#"{"name":"a","discriminant":{"type":"u8","value":256},"accounts":[],"args":[]}"#,
        )
        .is_err());
    }

    /// `u64` has no headroom above it, so the check must not overflow while computing a maximum.
    #[test]
    fn a_u64_declaration_accepts_the_whole_range() {
        let ix = instruction(r#"{"type":"u64","value":18446744073709551615}"#);
        assert_eq!(ix.get_discriminator(), vec![255; 8]);
    }
}

#[cfg(test)]
mod optional_account_tests {
    use super::*;

    /// The generated instruction parser warns about IDL drift only when an instruction receives
    /// fewer accounts than it *requires*, counting `optional` ones as absent-able. If either
    /// spelling stopped parsing, every optional account would count as required and the parser
    /// would warn on every legitimate call that omits one.
    #[test]
    fn both_spellings_of_optional_parse() {
        for field in [r#""optional":true"#, r#""isOptional":true"#] {
            let account: IdlAccountArg = serde_json::from_str(&format!(
                r#"{{"name":"payer","isMut":true,"isSigner":true,{field}}}"#
            ))
            .unwrap_or_else(|e| panic!("parsing {field}: {e}"));
            assert!(account.optional, "{field} should mark the account optional");
        }
    }

    /// Absent means required, so a plain Anchor account is never treated as omittable.
    #[test]
    fn an_account_without_the_field_is_required() {
        let account: IdlAccountArg =
            serde_json::from_str(r#"{"name":"config","isMut":false,"isSigner":false}"#)
                .expect("parse account");
        assert!(!account.optional);
    }

    /// The count the parser compares against: optional accounts are excluded.
    #[test]
    fn required_count_excludes_optional_accounts() {
        let ix: IdlInstruction = serde_json::from_str(
            r#"{"name":"extend_lookup_table","accounts":[
                 {"name":"lookup_table","isMut":true,"isSigner":false},
                 {"name":"authority","isMut":false,"isSigner":true},
                 {"name":"payer","isMut":true,"isSigner":true,"optional":true},
                 {"name":"system_program","isMut":false,"isSigner":false,"optional":true}
               ],"args":[]}"#,
        )
        .expect("parse instruction");

        assert_eq!(ix.accounts.len(), 4, "declared");
        assert_eq!(
            ix.accounts.iter().filter(|a| !a.optional).count(),
            2,
            "required"
        );
    }

    /// The generated warning silences a short account count only when every omitted declaration is
    /// optional. Counting required accounts instead accepted this shape, where the omitted optional
    /// sits in the middle: two accounts supplied against `[authority, payer?, system_program]`
    /// meets the required count of two, yet position 1 is the system program being labelled
    /// `payer`. `meteora_dlmm.json`'s `add_liquidity` has exactly this layout.
    #[test]
    fn a_non_trailing_optional_leaves_the_omitted_suffix_required() {
        let ix: IdlInstruction = serde_json::from_str(
            r#"{"name":"add_liquidity","accounts":[
                 {"name":"authority","isMut":false,"isSigner":true},
                 {"name":"payer","isMut":true,"isSigner":true,"optional":true},
                 {"name":"system_program","isMut":false,"isSigner":false}
               ],"args":[]}"#,
        )
        .expect("parse instruction");

        let optional: Vec<bool> = ix.accounts.iter().map(|a| a.optional).collect();
        let actual_count = 2;

        // What the generated code now asks. The omitted suffix is [system_program], required, so
        // the mismatch must be reported.
        let omitted_are_all_optional = optional.iter().skip(actual_count).all(|o| *o);
        assert!(
            !omitted_are_all_optional,
            "the omitted declaration is required, so the count must not be silenced"
        );

        // The old rule counted required accounts and found the floor met.
        assert_eq!(
            optional.iter().filter(|o| !**o).count(),
            actual_count,
            "the required floor is met, which is why counting it was not enough"
        );
    }

    /// A genuinely trailing optional must stay silent, or every correct IDL warns.
    #[test]
    fn a_trailing_optional_suffix_stays_silent() {
        let ix: IdlInstruction = serde_json::from_str(
            r#"{"name":"extend","accounts":[
                 {"name":"lookup_table","isMut":true,"isSigner":false},
                 {"name":"payer","isMut":true,"isSigner":true,"optional":true},
                 {"name":"system_program","isMut":false,"isSigner":false,"optional":true}
               ],"args":[]}"#,
        )
        .expect("parse instruction");

        let optional: Vec<bool> = ix.accounts.iter().map(|a| a.optional).collect();

        assert!(optional.iter().skip(1).all(|o| *o), "tail is all optional");
    }
}
