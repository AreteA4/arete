use arete_idl::{
    normalize_idl_snapshot_v1, IdlAmountDecimalsSource, IdlAmountHint, IdlErrorSnapshot,
    IdlSnapshotV1, IdlSpec, IdlType, IdlTypeArrayElement, IdlTypeDefinedInner,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    canonicalize_jcs, hash_jcs, hash_raw_bytes, HashError, HashId, IdlContent, IdlNormalized,
    IdlPortable, IdlSource, OssGeneratedProgramReleaseV1, ProgramRelease, ProgramSpec,
};

pub const PROGRAM_SPEC_SCHEMA_V1: &str = "arete.program-spec/v1";

#[derive(Debug, Clone)]
pub struct IdlHashes {
    pub source: HashId<IdlSource>,
    pub content: HashId<IdlContent>,
    pub portable: HashId<IdlPortable>,
    pub normalized: HashId<IdlNormalized>,
}

/// Strictly parsed IDL plus every authoritative v1 IDL projection.
#[derive(Debug, Clone)]
pub struct CanonicalIdlDocument {
    source_bytes: Vec<u8>,
    content: Value,
    portable: Value,
    idl: IdlSpec,
    program_id: String,
    snapshot: IdlSnapshotV1,
    hashes: IdlHashes,
}

impl CanonicalIdlDocument {
    pub fn parse(bytes: &[u8], explicit_program_id: Option<&str>) -> Result<Self, HashError> {
        let mut content = crate::parse_json_bytes_strict(bytes)?;
        let source_program_ids = collect_program_ids(&content)?;
        let source_has_program_id = !source_program_ids.is_empty();
        let program_id = resolve_program_id(source_program_ids, explicit_program_id)?;

        if !source_has_program_id {
            content
                .as_object_mut()
                .ok_or_else(|| HashError::InvalidIdl("IDL root must be an object".to_string()))?
                .insert("address".to_string(), Value::String(program_id.clone()));
        }

        let parser_input = serde_json::to_string(&content)
            .map_err(|error| HashError::Serialization(error.to_string()))?;
        let mut idl =
            arete_idl::parse::parse_idl_content(&parser_input).map_err(HashError::InvalidIdl)?;
        idl.address = Some(program_id.clone());

        let mut snapshot = normalize_idl_snapshot_v1(&idl);
        snapshot.snapshot.program_id = Some(program_id.clone());
        let portable = portable_idl_projection(&content)?;
        let hashes = IdlHashes {
            source: hash_raw_bytes(bytes)?,
            content: hash_jcs(&content)?,
            portable: hash_jcs(&portable)?,
            normalized: hash_jcs(&snapshot)?,
        };

        Ok(Self {
            source_bytes: bytes.to_vec(),
            content,
            portable,
            idl,
            program_id,
            snapshot,
            hashes,
        })
    }

    pub fn source_bytes(&self) -> &[u8] {
        &self.source_bytes
    }

    pub fn content_projection(&self) -> &Value {
        &self.content
    }

    pub fn portable_projection(&self) -> &Value {
        &self.portable
    }

    pub fn parsed_idl(&self) -> &IdlSpec {
        &self.idl
    }

    pub fn program_id(&self) -> &str {
        &self.program_id
    }

    pub fn normalized_snapshot(&self) -> &IdlSnapshotV1 {
        &self.snapshot
    }

    pub fn hashes(&self) -> &IdlHashes {
        &self.hashes
    }

    pub fn content_payload(&self) -> Result<Vec<u8>, HashError> {
        canonicalize_jcs(&self.content)
    }

    pub fn portable_payload(&self) -> Result<Vec<u8>, HashError> {
        canonicalize_jcs(&self.portable)
    }

    pub fn normalized_payload(&self) -> Result<Vec<u8>, HashError> {
        canonicalize_jcs(&self.snapshot)
    }
}

pub fn portable_idl_projection(source: &Value) -> Result<Value, HashError> {
    let mut portable = source.clone();
    let object = portable
        .as_object_mut()
        .ok_or_else(|| HashError::InvalidIdl("IDL root must be an object".to_string()))?;
    object.remove("address");
    object.remove("program_id");
    if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.remove("address");
    }
    if let Some(program) = object.get_mut("program").and_then(Value::as_object_mut) {
        program.remove("publicKey");
    }
    Ok(portable)
}

fn collect_program_ids(value: &Value) -> Result<Vec<(&'static str, String)>, HashError> {
    let object = value
        .as_object()
        .ok_or_else(|| HashError::InvalidIdl("IDL root must be an object".to_string()))?;
    let mut values = Vec::new();
    collect_program_id(&mut values, "address", object.get("address"))?;
    collect_program_id(&mut values, "program_id", object.get("program_id"))?;
    collect_nested_program_id(
        &mut values,
        "metadata.address",
        object.get("metadata"),
        "address",
    )?;
    collect_nested_program_id(
        &mut values,
        "program.publicKey",
        object.get("program"),
        "publicKey",
    )?;
    Ok(values)
}

fn collect_nested_program_id(
    output: &mut Vec<(&'static str, String)>,
    location: &'static str,
    parent: Option<&Value>,
    key: &str,
) -> Result<(), HashError> {
    match parent {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Object(object)) => collect_program_id(output, location, object.get(key)),
        Some(_) => Err(HashError::InvalidProgramIdLocation { location }),
    }
}

fn collect_program_id(
    output: &mut Vec<(&'static str, String)>,
    location: &'static str,
    value: Option<&Value>,
) -> Result<(), HashError> {
    match value {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(value)) if value.is_empty() => Ok(()),
        Some(Value::String(value)) => {
            output.push((location, value.clone()));
            Ok(())
        }
        Some(_) => Err(HashError::InvalidProgramIdLocation { location }),
    }
}

fn resolve_program_id(
    mut values: Vec<(&'static str, String)>,
    explicit: Option<&str>,
) -> Result<String, HashError> {
    if let Some(explicit) = explicit {
        if explicit.is_empty() {
            return Err(HashError::MissingProgramId);
        }
        values.push(("explicit", explicit.to_string()));
    }
    if values.is_empty() {
        return Err(HashError::MissingProgramId);
    }

    let distinct: BTreeSet<&str> = values.iter().map(|(_, value)| value.as_str()).collect();
    if distinct.len() != 1 {
        let detail = values
            .iter()
            .map(|(location, value)| format!("{location}={value}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(HashError::ConflictingProgramIds(detail));
    }
    Ok(values.remove(0).1)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramSpecV1 {
    pub schema: String,
    pub program_id: String,
    pub idl_content_hash: HashId<IdlContent>,
    pub portable_idl_hash: HashId<IdlPortable>,
    pub normalized_idl_hash: HashId<IdlNormalized>,
    pub idl_snapshot: IdlSnapshotV1,
    pub pdas: BTreeMap<String, PdaDefinitionV1>,
    pub instructions: Vec<InstructionDefinitionV1>,
}

impl ProgramSpecV1 {
    pub fn from_document(document: &CanonicalIdlDocument) -> Self {
        let pdas = extract_pdas(document.parsed_idl());
        let instructions =
            extract_instructions(document.parsed_idl(), &pdas, document.program_id());
        Self {
            schema: PROGRAM_SPEC_SCHEMA_V1.to_string(),
            program_id: document.program_id.clone(),
            idl_content_hash: document.hashes.content,
            portable_idl_hash: document.hashes.portable,
            normalized_idl_hash: document.hashes.normalized,
            idl_snapshot: document.snapshot.clone(),
            pdas,
            instructions,
        }
    }

    pub fn hash(&self) -> Result<HashId<ProgramSpec>, HashError> {
        self.validate()?;
        hash_jcs(self)
    }

    pub fn validate(&self) -> Result<(), HashError> {
        if self.schema != PROGRAM_SPEC_SCHEMA_V1 {
            return Err(HashError::UnknownVersion(self.schema.clone()));
        }
        if self.idl_snapshot.normalization_version != arete_idl::IDL_NORMALIZATION_VERSION {
            return Err(HashError::UnknownVersion(format!(
                "IDL normalization version {}",
                self.idl_snapshot.normalization_version
            )));
        }
        if self.program_id.is_empty() {
            return Err(HashError::MissingProgramId);
        }
        if self.idl_snapshot.snapshot.program_id.as_deref() != Some(self.program_id.as_str()) {
            return Err(HashError::InvalidProjection {
                projection: "program spec",
                reason: "programId must match idlSnapshot.program_id".to_string(),
            });
        }
        for pda in self.pdas.values() {
            validate_pda_seeds(&pda.seeds)?;
        }
        for instruction in &self.instructions {
            for account in &instruction.accounts {
                if let AccountResolutionV1::PdaInline { seeds, .. } = &account.resolution {
                    validate_pda_seeds(seeds)?;
                }
            }
        }
        Ok(())
    }

    pub fn oss_release(&self) -> Result<OssGeneratedProgramReleaseV1, HashError> {
        Ok(OssGeneratedProgramReleaseV1::new(
            self.program_id.clone(),
            self.hash()?,
            self.idl_content_hash,
            self.normalized_idl_hash,
        ))
    }

    pub fn oss_release_hash(&self) -> Result<HashId<crate::ProgramRelease>, HashError> {
        self.oss_release()?.hash()
    }

    pub fn oss_identity(&self) -> Result<OssProgramIdentityV1, HashError> {
        OssProgramIdentityV1::new(self.clone())
    }
}

#[derive(Debug, Clone)]
pub struct OssProgramIdentityV1 {
    pub program_spec: ProgramSpecV1,
    pub program_spec_hash: HashId<ProgramSpec>,
    pub release: OssGeneratedProgramReleaseV1,
    pub release_hash: HashId<ProgramRelease>,
}

impl OssProgramIdentityV1 {
    pub fn new(program_spec: ProgramSpecV1) -> Result<Self, HashError> {
        let program_spec_hash = program_spec.hash()?;
        let release = OssGeneratedProgramReleaseV1::new(
            program_spec.program_id.clone(),
            program_spec_hash,
            program_spec.idl_content_hash,
            program_spec.normalized_idl_hash,
        );
        let release_hash = release.hash()?;
        Ok(Self {
            program_spec,
            program_spec_hash,
            release,
            release_hash,
        })
    }

    pub fn from_document(document: &CanonicalIdlDocument) -> Result<Self, HashError> {
        Self::new(ProgramSpecV1::from_document(document))
    }
}

pub fn build_program_spec_v1_from_bytes(
    bytes: &[u8],
    explicit_program_id: Option<&str>,
) -> Result<ProgramSpecV1, HashError> {
    let document = CanonicalIdlDocument::parse(bytes, explicit_program_id)?;
    Ok(ProgramSpecV1::from_document(&document))
}

pub fn build_oss_program_identity_v1_from_bytes(
    bytes: &[u8],
    explicit_program_id: Option<&str>,
) -> Result<OssProgramIdentityV1, HashError> {
    OssProgramIdentityV1::new(build_program_spec_v1_from_bytes(
        bytes,
        explicit_program_id,
    )?)
}

/// Compatibility adapter for callers that no longer have the original bytes.
///
/// New ingress should always call `build_program_spec_v1_from_bytes` so
/// `idl-content` is derived from the complete parsed source document.
pub fn build_program_spec_v1_from_idl(
    idl: &IdlSpec,
    explicit_program_id: Option<&str>,
) -> Result<ProgramSpecV1, HashError> {
    let bytes =
        serde_json::to_vec(idl).map_err(|error| HashError::Serialization(error.to_string()))?;
    build_program_spec_v1_from_bytes(&bytes, explicit_program_id)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdaDefinitionV1 {
    pub name: String,
    pub seeds: Vec<PdaSeedV1>,
    /// Legacy/static PDA program. Retained so existing ProgramSpec hashes do
    /// not change when the PDA program is a literal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    /// Dynamic PDA program selector. This is emitted only when the owning
    /// program must be resolved from another account or instruction argument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<PdaProgramV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PdaProgramV1 {
    AccountRef { account_name: String },
    ArgRef { arg_name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PdaSeedV1 {
    Literal {
        value: String,
    },
    Bytes {
        value: Vec<u8>,
    },
    ArgRef {
        arg_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arg_type: Option<String>,
    },
    AccountRef {
        account_name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "category", rename_all = "camelCase")]
pub enum AccountResolutionV1 {
    Signer,
    Known {
        address: String,
    },
    PdaRef {
        pda_name: String,
    },
    PdaInline {
        seeds: Vec<PdaSeedV1>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        program_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        program: Option<PdaProgramV1>,
    },
    UserProvided,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstructionAccountV1 {
    pub name: String,
    #[serde(default)]
    pub is_signer: bool,
    #[serde(default)]
    pub is_writable: bool,
    pub resolution: AccountResolutionV1,
    #[serde(default)]
    pub is_optional: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstructionAmountHintV1 {
    pub decimals_source: AmountDecimalsSourceV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AmountDecimalsSourceV1 {
    ArgMint { arg_name: String },
    ArgDecimals { arg_name: String },
    KnownAccount { account_name: String },
    Constant { decimals: u8 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstructionArgumentV1 {
    pub name: String,
    #[serde(rename = "type")]
    pub arg_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_hint: Option<InstructionAmountHintV1>,
}

fn default_discriminator_size() -> usize {
    8
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstructionDefinitionV1 {
    pub name: String,
    pub discriminator: Vec<u8>,
    #[serde(default = "default_discriminator_size")]
    pub discriminator_size: usize,
    pub accounts: Vec<InstructionAccountV1>,
    pub args: Vec<InstructionArgumentV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<IdlErrorSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<String>,
}

fn extract_pdas(idl: &IdlSpec) -> BTreeMap<String, PdaDefinitionV1> {
    let mut pdas = BTreeMap::new();
    let mut named_pdas = BTreeSet::new();
    for pda in &idl.pdas {
        let name = sanitize_identifier(&pda.name);
        named_pdas.insert(name.clone());
        pdas.insert(
            name.clone(),
            convert_pda(&name, &pda.seeds, pda.program.as_ref()),
        );
    }
    let mut conflicting_account_pdas = BTreeSet::new();
    for instruction in &idl.instructions {
        for account in instruction.flattened_accounts() {
            if let Some(pda) = &account.pda {
                let name = sanitize_identifier(pda.name.as_deref().unwrap_or(&account.name));
                if named_pdas.contains(&name) || conflicting_account_pdas.contains(&name) {
                    continue;
                }
                let candidate = convert_pda(&name, &pda.seeds, pda.program.as_ref());
                match pdas.get(&name) {
                    None => {
                        pdas.insert(name, candidate);
                    }
                    Some(existing) if existing == &candidate => {}
                    Some(_) => {
                        // Account-level PDAs are instruction-local. Publishing one
                        // arbitrary definition under a shared name makes other
                        // instructions derive a plausible but incorrect address.
                        pdas.remove(&name);
                        conflicting_account_pdas.insert(name);
                    }
                }
            }
        }
    }
    pdas
}

fn extract_instructions(
    idl: &IdlSpec,
    pdas: &BTreeMap<String, PdaDefinitionV1>,
    program_id: &str,
) -> Vec<InstructionDefinitionV1> {
    let discriminator_size = idl.instruction_discriminator_size();

    idl.instructions
        .iter()
        .map(|instruction| InstructionDefinitionV1 {
            name: instruction.name.clone(),
            discriminator: instruction.get_discriminator(),
            discriminator_size,
            accounts: instruction
                .flattened_accounts()
                .iter()
                .map(|account| convert_account(account, pdas))
                .collect(),
            args: instruction
                .args
                .iter()
                .map(|argument| InstructionArgumentV1 {
                    name: argument.name.clone(),
                    arg_type: idl_type_to_rust_string(&argument.type_),
                    docs: Vec::new(),
                    amount_hint: argument.amount_hint.as_ref().map(convert_amount_hint),
                })
                .collect(),
            errors: Vec::new(),
            program_id: Some(program_id.to_string()),
            docs: instruction.docs.clone(),
        })
        .collect()
}

fn convert_pda(
    name: &str,
    seeds: &[arete_idl::IdlPdaSeed],
    program: Option<&arete_idl::IdlPdaProgram>,
) -> PdaDefinitionV1 {
    let seeds = seeds
        .iter()
        .map(|seed| match seed {
            arete_idl::IdlPdaSeed::Const { value } => convert_const_pda_seed(value),
            arete_idl::IdlPdaSeed::Account { path, .. } => PdaSeedV1::AccountRef {
                account_name: sanitize_seed_path(path),
            },
            arete_idl::IdlPdaSeed::Arg { path, arg_type } => PdaSeedV1::ArgRef {
                arg_name: sanitize_seed_path(path),
                arg_type: arg_type.clone(),
            },
        })
        .collect();
    let (program_id, program) = match program {
        Some(arete_idl::IdlPdaProgram::Literal { value, .. }) => (Some(value.clone()), None),
        Some(arete_idl::IdlPdaProgram::Const { value, .. }) => {
            (Some(bs58::encode(value).into_string()), None)
        }
        Some(arete_idl::IdlPdaProgram::Account { path, .. }) => (
            None,
            Some(PdaProgramV1::AccountRef {
                account_name: sanitize_seed_path(path),
            }),
        ),
        None => (None, None),
    };
    PdaDefinitionV1 {
        name: name.to_string(),
        seeds,
        program_id,
        program,
    }
}

fn convert_const_pda_seed(value: &[u8]) -> PdaSeedV1 {
    match String::from_utf8(value.to_vec()) {
        Ok(value) if !value.contains('\0') => PdaSeedV1::Literal { value },
        _ => PdaSeedV1::Bytes {
            value: value.to_vec(),
        },
    }
}

fn validate_pda_seeds(seeds: &[PdaSeedV1]) -> Result<(), HashError> {
    if seeds
        .iter()
        .any(|seed| matches!(seed, PdaSeedV1::Literal { value } if value.contains('\0')))
    {
        return Err(HashError::InvalidProjection {
            projection: "program spec",
            reason: "literal PDA seeds must not contain NUL bytes; use a bytes seed".to_string(),
        });
    }
    Ok(())
}

fn convert_account(
    account: &arete_idl::IdlAccountArg,
    pdas: &BTreeMap<String, PdaDefinitionV1>,
) -> InstructionAccountV1 {
    let resolution = if account.is_signer && account.address.is_none() && account.pda.is_none() {
        AccountResolutionV1::Signer
    } else if let Some(address) = &account.address {
        AccountResolutionV1::Known {
            address: address.clone(),
        }
    } else if let Some(pda) = &account.pda {
        let name = sanitize_identifier(pda.name.as_deref().unwrap_or(&account.name));
        let converted = convert_pda(&name, &pda.seeds, pda.program.as_ref());
        if pdas.get(&name) == Some(&converted) {
            AccountResolutionV1::PdaRef { pda_name: name }
        } else {
            AccountResolutionV1::PdaInline {
                seeds: converted.seeds,
                program_id: converted.program_id,
                program: converted.program,
            }
        }
    } else {
        let name = sanitize_identifier(&account.name);
        if pdas.contains_key(&name) {
            AccountResolutionV1::PdaRef { pda_name: name }
        } else {
            AccountResolutionV1::UserProvided
        }
    };
    InstructionAccountV1 {
        name: sanitize_identifier(&account.name),
        is_signer: account.is_signer,
        is_writable: account.is_mut,
        resolution,
        is_optional: account.optional,
        docs: account.docs.clone(),
    }
}

fn convert_amount_hint(hint: &IdlAmountHint) -> InstructionAmountHintV1 {
    InstructionAmountHintV1 {
        decimals_source: match &hint.decimals_source {
            IdlAmountDecimalsSource::ArgMint { arg_name } => AmountDecimalsSourceV1::ArgMint {
                arg_name: arg_name.clone(),
            },
            IdlAmountDecimalsSource::ArgDecimals { arg_name } => {
                AmountDecimalsSourceV1::ArgDecimals {
                    arg_name: arg_name.clone(),
                }
            }
            IdlAmountDecimalsSource::KnownAccount { account_name } => {
                AmountDecimalsSourceV1::KnownAccount {
                    account_name: account_name.clone(),
                }
            }
            IdlAmountDecimalsSource::Constant { decimals } => AmountDecimalsSourceV1::Constant {
                decimals: *decimals,
            },
        },
    }
}

fn sanitize_identifier(name: &str) -> String {
    let mut sanitized = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            sanitized.push(character);
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
        .is_some_and(|character| character.is_ascii_digit())
    {
        return format!("_{sanitized}");
    }
    sanitized
}

fn sanitize_seed_path(path: &str) -> String {
    path.split('.')
        .map(sanitize_identifier)
        .collect::<Vec<_>>()
        .join(".")
}

fn idl_type_to_rust_string(idl_type: &IdlType) -> String {
    match idl_type {
        IdlType::Simple(simple) => match simple.as_str() {
            "string" => "String".to_string(),
            "publicKey" | "pubkey" => "solana_pubkey::Pubkey".to_string(),
            "bytes" => "Vec<u8>".to_string(),
            other => other.to_string(),
        },
        IdlType::Array(array) if array.array.len() == 2 => {
            match (&array.array[0], &array.array[1]) {
                (IdlTypeArrayElement::Type(name), IdlTypeArrayElement::Size(size)) => {
                    format!(
                        "[{}; {size}]",
                        idl_type_to_rust_string(&IdlType::Simple(name.clone()))
                    )
                }
                (IdlTypeArrayElement::Nested(ty), IdlTypeArrayElement::Size(size)) => {
                    format!("[{}; {size}]", idl_type_to_rust_string(ty))
                }
                _ => "Vec<u8>".to_string(),
            }
        }
        IdlType::Array(_) => "Vec<u8>".to_string(),
        IdlType::Option(option) => format!("Option<{}>", idl_type_to_rust_string(&option.option)),
        // `VecU64Len<T>` signals the bincode-style prefix to the SDK generators.
        IdlType::Vec(vec_type) => {
            let inner = idl_type_to_rust_string(&vec_type.vec);
            if matches!(
                vec_type.length_prefix,
                Some(arete_idl::types::IdlLengthPrefix::U64)
            ) {
                format!("VecU64Len<{inner}>")
            } else {
                format!("Vec<{inner}>")
            }
        }
        IdlType::HashMap(hash_map) => format!(
            "std::collections::HashMap<{}, {}>",
            idl_type_to_rust_string(&hash_map.hash_map.0),
            idl_type_to_rust_string(&hash_map.hash_map.1)
        ),
        IdlType::Tuple(tuple) => format!(
            "({})",
            tuple
                .tuple
                .iter()
                .map(idl_type_to_rust_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        IdlType::Defined(defined) => match &defined.defined {
            IdlTypeDefinedInner::Named { name } => name.clone(),
            IdlTypeDefinedInner::Simple(simple) => simple.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_account_selected_pda_programs() {
        let definition = convert_pda(
            "metadata",
            &[arete_idl::IdlPdaSeed::Const {
                value: b"metadata".to_vec(),
            }],
            Some(&arete_idl::IdlPdaProgram::Account {
                kind: "account".to_string(),
                path: "metadata_program".to_string(),
            }),
        );

        assert_eq!(definition.program_id, None);
        assert_eq!(
            definition.program,
            Some(PdaProgramV1::AccountRef {
                account_name: "metadata_program".to_string(),
            })
        );
    }

    #[test]
    fn preserves_nul_const_pda_seeds_as_bytes() {
        let definition = convert_pda(
            "binary",
            &[
                arete_idl::IdlPdaSeed::Const {
                    value: b"text".to_vec(),
                },
                arete_idl::IdlPdaSeed::Const { value: vec![0, 0] },
            ],
            None,
        );

        assert_eq!(
            definition.seeds,
            vec![
                PdaSeedV1::Literal {
                    value: "text".to_string(),
                },
                PdaSeedV1::Bytes { value: vec![0, 0] },
            ]
        );
    }

    #[test]
    fn rejects_nul_in_literal_pda_seeds() {
        let source = br#"{
            "address":"11111111111111111111111111111111",
            "metadata":{"name":"demo","version":"0.1.0","spec":"0.1.0"},
            "instructions":[
                {"name":"create","discriminator":[1],"accounts":[
                    {"name":"state","pda":{"seeds":[{"kind":"const","value":[115,116,97,116,101]}]}}
                ],"args":[]}
            ],
            "accounts":[],"types":[],"events":[],"errors":[]
        }"#;
        let document = CanonicalIdlDocument::parse(source, None).unwrap();
        let mut spec = ProgramSpecV1::from_document(&document);
        spec.pdas.get_mut("state").unwrap().seeds[0] = PdaSeedV1::Literal {
            value: "\0".to_string(),
        };

        assert!(matches!(
            spec.validate(),
            Err(HashError::InvalidProjection {
                projection: "program spec",
                ..
            })
        ));
    }

    #[test]
    fn keeps_conflicting_account_pdas_inline_per_instruction() {
        let source = br#"{
            "address":"11111111111111111111111111111111",
            "metadata":{"name":"demo","version":"0.1.0","spec":"0.1.0"},
            "instructions":[
                {"name":"create","discriminator":[1,0,0,0,0,0,0,0],"accounts":[
                    {"name":"state","pda":{"seeds":[{"kind":"const","value":[99,114,101,97,116,101]}]}}
                ],"args":[]},
                {"name":"update","discriminator":[2,0,0,0,0,0,0,0],"accounts":[
                    {"name":"state","pda":{"seeds":[{"kind":"const","value":[117,112,100,97,116,101]}]}}
                ],"args":[]}
            ],
            "accounts":[],"types":[],"events":[],"errors":[]
        }"#;
        let document = CanonicalIdlDocument::parse(source, None).unwrap();
        let spec = ProgramSpecV1::from_document(&document);

        assert!(!spec.pdas.contains_key("state"));
        for instruction in &spec.instructions {
            assert!(matches!(
                &instruction.accounts[0].resolution,
                AccountResolutionV1::PdaInline { .. }
            ));
        }
    }
}
