use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    canonicalize_jcs, hash_jcs, parse_json_bytes_strict, DecoderFixtureSet, HashError, HashId,
    IdlNormalized,
};

pub const DECODER_FIXTURE_SCHEMA_V1: &str = "arete.decoder-fixtures/v1";
pub const DECODER_FIXTURE_PUBLIC_VALUE_DIGEST_PREFIX: &str = "sha256:";
pub const DECODER_FIXTURE_MAX_CASES: usize = 256;
pub const DECODER_FIXTURE_MAX_ACCOUNT_BYTES: usize = 1024 * 1024;
pub const DECODER_FIXTURE_MAX_TOTAL_ACCOUNT_BYTES: usize = 8 * 1024 * 1024;
pub const DECODER_FIXTURE_ACCOUNT_DECODE_ERROR_CATEGORIES: &[&str] = &[
    "owner_mismatch",
    "unknown_account_type",
    "account_type_mismatch",
    "ambiguous_account_type",
    "account_decode_failed",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecoderFixtureSetV1 {
    pub schema: String,
    pub program_id: String,
    pub normalized_idl_hash: HashId<IdlNormalized>,
    pub decoder_engine_id: String,
    pub decoder_abi_version: String,
    pub cases: Vec<DecoderFixtureCaseV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecoderFixtureCaseV1 {
    pub id: String,
    pub account_type: String,
    pub owner: String,
    pub address: String,
    pub account_data_hex: String,
    pub expected: DecoderFixtureExpectedV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_private_diagnostics: Option<DecoderFixturePrivateDiagnosticsV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum DecoderFixtureExpectedV1 {
    Decoded {
        #[serde(rename = "publicValueDigest")]
        public_value_digest: String,
    },
    Error {
        category: DecoderFixtureAccountDecodeErrorCategory,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecoderFixtureAccountDecodeErrorCategory {
    OwnerMismatch,
    UnknownAccountType,
    AccountTypeMismatch,
    AmbiguousAccountType,
    AccountDecodeFailed,
}

impl DecoderFixtureAccountDecodeErrorCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OwnerMismatch => "owner_mismatch",
            Self::UnknownAccountType => "unknown_account_type",
            Self::AccountTypeMismatch => "account_type_mismatch",
            Self::AmbiguousAccountType => "ambiguous_account_type",
            Self::AccountDecodeFailed => "account_decode_failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecoderFixturePrivateDiagnosticsV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trailing_bytes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_count: Option<u32>,
}

impl DecoderFixtureSetV1 {
    pub fn canonical_projection(&self) -> Result<Self, HashError> {
        validate_decoder_fixture_set_v1(self)?;
        let mut projection = self.clone();
        projection
            .cases
            .sort_by(|left, right| left.id.cmp(&right.id));
        Ok(projection)
    }

    pub fn hash(&self) -> Result<HashId<DecoderFixtureSet>, HashError> {
        hash_decoder_fixture_set_v1(self)
    }
}

pub fn parse_decoder_fixture_set_v1(bytes: &[u8]) -> Result<DecoderFixtureSetV1, HashError> {
    let value = parse_json_bytes_strict(bytes)?;
    let fixture: DecoderFixtureSetV1 =
        serde_json::from_value(value).map_err(|error| projection_error(error.to_string()))?;
    validate_decoder_fixture_set_v1(&fixture)?;
    Ok(fixture)
}

pub fn validate_decoder_fixture_set_v1(fixture: &DecoderFixtureSetV1) -> Result<(), HashError> {
    if fixture.schema != DECODER_FIXTURE_SCHEMA_V1 {
        return Err(HashError::UnknownVersion(fixture.schema.clone()));
    }
    validate_pubkey(&fixture.program_id, "programId")?;
    validate_nonempty_identifier(&fixture.decoder_engine_id, "decoderEngineId", 128)?;
    validate_nonempty_identifier(&fixture.decoder_abi_version, "decoderAbiVersion", 64)?;
    if fixture.cases.is_empty() || fixture.cases.len() > DECODER_FIXTURE_MAX_CASES {
        return invalid(format!(
            "cases must contain between 1 and {DECODER_FIXTURE_MAX_CASES} entries"
        ));
    }

    let mut ids = HashSet::with_capacity(fixture.cases.len());
    let mut total_bytes = 0_usize;
    for case in &fixture.cases {
        validate_stable_id(&case.id, "case id", 128)?;
        if !ids.insert(case.id.as_str()) {
            return invalid(format!("case id '{}' is duplicated", case.id));
        }
        validate_nonempty_identifier(&case.account_type, "accountType", 128)?;
        validate_pubkey(&case.owner, "owner")?;
        validate_pubkey(&case.address, "address")?;
        validate_account_data_hex(&case.account_data_hex)?;
        let account_bytes = case.account_data_hex.len() / 2;
        if account_bytes > DECODER_FIXTURE_MAX_ACCOUNT_BYTES {
            return invalid(format!(
                "case '{}' accountDataHex exceeds {DECODER_FIXTURE_MAX_ACCOUNT_BYTES} bytes",
                case.id
            ));
        }
        total_bytes = total_bytes.checked_add(account_bytes).ok_or_else(|| {
            projection_error("total accountDataHex byte length overflowed".to_string())
        })?;
        if total_bytes > DECODER_FIXTURE_MAX_TOTAL_ACCOUNT_BYTES {
            return invalid(format!(
                "fixture accountDataHex exceeds {DECODER_FIXTURE_MAX_TOTAL_ACCOUNT_BYTES} total bytes"
            ));
        }

        match &case.expected {
            DecoderFixtureExpectedV1::Decoded {
                public_value_digest,
            } => validate_public_value_digest(public_value_digest)?,
            DecoderFixtureExpectedV1::Error { .. } => {}
        }

        if let Some(diagnostics) = &case.expected_private_diagnostics {
            if diagnostics.trailing_bytes.is_none() && diagnostics.candidate_count.is_none() {
                return invalid(format!(
                    "case '{}' expectedPrivateDiagnostics must not be empty",
                    case.id
                ));
            }
            if diagnostics.candidate_count == Some(0) {
                return invalid(format!(
                    "case '{}' candidateCount must be greater than zero",
                    case.id
                ));
            }
        }
    }
    Ok(())
}

pub fn hash_decoder_fixture_set_v1(
    fixture: &DecoderFixtureSetV1,
) -> Result<HashId<DecoderFixtureSet>, HashError> {
    hash_jcs(&fixture.canonical_projection()?)
}

pub fn digest_decoder_fixture_public_value_v1<T: Serialize>(
    value: &T,
) -> Result<String, HashError> {
    let canonical = canonicalize_jcs(value)?;
    Ok(format!(
        "{DECODER_FIXTURE_PUBLIC_VALUE_DIGEST_PREFIX}{}",
        hex::encode(Sha256::digest(canonical))
    ))
}

fn validate_account_data_hex(value: &str) -> Result<(), HashError> {
    if !value.len().is_multiple_of(2)
        || !value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return invalid("accountDataHex must contain lowercase hexadecimal byte pairs".to_string());
    }
    Ok(())
}

fn validate_public_value_digest(value: &str) -> Result<(), HashError> {
    let Some(digest) = value.strip_prefix(DECODER_FIXTURE_PUBLIC_VALUE_DIGEST_PREFIX) else {
        return invalid("publicValueDigest must use the sha256:<lowercase-hex> format".to_string());
    };
    if digest.len() != 64
        || !digest
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return invalid("publicValueDigest must use the sha256:<lowercase-hex> format".to_string());
    }
    Ok(())
}

fn validate_pubkey(value: &str, field: &str) -> Result<(), HashError> {
    let decoded = bs58::decode(value)
        .into_vec()
        .map_err(|_| projection_error(format!("{field} must be a base58 Solana public key")))?;
    if decoded.len() != 32 || bs58::encode(decoded).into_string() != value {
        return invalid(format!("{field} must be a base58 Solana public key"));
    }
    Ok(())
}

fn validate_nonempty_identifier(
    value: &str,
    field: &str,
    max_length: usize,
) -> Result<(), HashError> {
    if value.is_empty() || value.trim() != value || value.len() > max_length {
        return invalid(format!(
            "{field} must be a nonempty, trimmed string of at most {max_length} bytes"
        ));
    }
    Ok(())
}

fn validate_stable_id(value: &str, field: &str, max_length: usize) -> Result<(), HashError> {
    if value.is_empty()
        || value.len() > max_length
        || !value.as_bytes().iter().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (index > 0 && matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return invalid(format!(
            "{field} must be a lowercase stable identifier of at most {max_length} bytes"
        ));
    }
    Ok(())
}

fn projection_error(reason: String) -> HashError {
    HashError::InvalidProjection {
        projection: "decoder fixture set",
        reason,
    }
}

fn invalid<T>(reason: String) -> Result<T, HashError> {
    Err(projection_error(reason))
}
