//! Wallet proofs for participant-written protocol metadata.
//!
//! Mirror of `typescript/core/src/protocol/metadata-proof.ts`. The wallet
//! signs (Ed25519) a short text message naming the deployment, instance,
//! wallet, the SHA-256 of the patch in RFC 8785 canonical form, and an
//! expiry. This module builds the message; signing stays with the caller's
//! wallet, so no key material passes through the SDK.

use std::future::Future;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

use crate::encoding::{encode_base58, sha256};

/// First line of a metadata wallet proof message.
pub const PROTOCOL_METADATA_PROOF_DOMAIN: &str = "arete.protocol-metadata/v1";
/// How far ahead a metadata proof may expire, in seconds.
pub const PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS: u64 = 300;
/// The expiry clients pick when none is given, in seconds from now.
pub const PROTOCOL_METADATA_PROOF_DEFAULT_TTL_SECONDS: u64 = 240;

/// Largest integer every JSON implementation reads exactly (2^53 - 1).
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Why a metadata proof could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MetadataProofError {
    /// A value is not I-JSON: a number outside the safe integer range, or a
    /// non-finite float.
    #[error("{path} {message}")]
    InvalidJson { path: String, message: String },
    /// An id or wallet is empty or spans several lines.
    #[error("{0} must be a non-empty single-line string")]
    InvalidField(&'static str),
    /// `expires_at` is not in the future, or more than 300 seconds ahead.
    #[error("expiresAt must be in the future and at most {PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS} seconds ahead")]
    ExpiresAt,
    /// The wallet's signature is not 64 bytes.
    #[error("signature must be the 64-byte Ed25519 signature")]
    Signature,
}

/// Serialize a JSON value in the RFC 8785 JSON Canonicalization Scheme (JCS)
/// form: object members sorted by the UTF-16 code units of their names, no
/// whitespace, numbers in ECMAScript `Number.prototype.toString` form and
/// strings with the minimal JSON escapes.
///
/// Integers beyond 2^53 - 1 are refused: send them as decimal strings.
pub fn canonicalize_json(value: &Value) -> Result<String, MetadataProofError> {
    let mut out = String::new();
    write_canonical(value, "$", &mut out)?;
    Ok(out)
}

/// Lowercase hex SHA-256 of the JCS form of a JSON value.
pub fn canonical_json_sha256(value: &Value) -> Result<String, MetadataProofError> {
    Ok(hex(&sha256(&[canonicalize_json(value)?.as_bytes()])))
}

fn write_canonical(value: &Value, path: &str, out: &mut String) -> Result<(), MetadataProofError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Value::Number(number) => out.push_str(&canonical_number(number, path)?),
        Value::String(value) => out.push_str(&json_string(value)),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, &format!("{path}[{index}]"), out)?;
            }
            out.push(']');
        }
        Value::Object(members) => {
            let mut keys = members.keys().collect::<Vec<_>>();
            keys.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&json_string(key));
                out.push(':');
                write_canonical(&members[key], &format!("{path}.{key}"), out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn json_string(value: &str) -> String {
    // serde_json escapes exactly what JSON.stringify does: `"`, `\`, the
    // short escapes \b \f \n \r \t, and other controls as lowercase \u00xx.
    serde_json::to_string(value).expect("string serialization is infallible")
}

fn canonical_number(number: &Number, path: &str) -> Result<String, MetadataProofError> {
    let unsafe_integer = || MetadataProofError::InvalidJson {
        path: path.to_string(),
        message: "is an integer beyond 2^53 - 1; send it as a decimal string".into(),
    };
    if let Some(value) = number.as_u64() {
        return if value <= MAX_SAFE_INTEGER {
            Ok(value.to_string())
        } else {
            Err(unsafe_integer())
        };
    }
    if let Some(value) = number.as_i64() {
        return if value.unsigned_abs() <= MAX_SAFE_INTEGER {
            Ok(value.to_string())
        } else {
            Err(unsafe_integer())
        };
    }
    let value = number
        .as_f64()
        .ok_or_else(|| MetadataProofError::InvalidJson {
            path: path.to_string(),
            message: "is not a finite number".into(),
        })?;
    if !value.is_finite() {
        return Err(MetadataProofError::InvalidJson {
            path: path.to_string(),
            message: "is not a finite number".into(),
        });
    }
    Ok(ecmascript_number(value))
}

/// `Number.prototype.toString()` of a finite double (ECMA-262 §6.1.6.1.20).
fn ecmascript_number(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    if value < 0.0 {
        return format!("-{}", ecmascript_number(-value));
    }
    // Rust's `{:e}` prints the shortest round-tripping digits, as ECMAScript
    // requires: `d.ddddde<exp>`.
    let formatted = format!("{value:e}");
    let (mantissa, exponent) = formatted
        .split_once('e')
        .expect("exponent formatting has an exponent");
    let digits = mantissa.replace('.', "");
    let k = digits.len() as i32;
    let n = exponent.parse::<i32>().expect("exponent is an integer") + 1;
    if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let exponent = n - 1;
        let sign = if exponent < 0 { '-' } else { '+' };
        let (first, rest) = digits.split_at(1);
        if rest.is_empty() {
            format!("{first}e{sign}{}", exponent.abs())
        } else {
            format!("{first}.{rest}e{sign}{}", exponent.abs())
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Inputs of a metadata proof message.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolMetadataProofInput<'a> {
    pub deployment_id: &'a str,
    pub instance_id: &'a str,
    /// The wallet that signs.
    pub wallet: &'a str,
    /// The JSON merge patch being authorised (`null` removes a field).
    pub patch: &'a Map<String, Value>,
    /// Unix seconds; in the future and at most 300 seconds ahead.
    pub expires_at: u64,
    /// Unix seconds used to check `expires_at`; the current time when `None`.
    pub now: Option<u64>,
}

/// The message a wallet signs to authorise a metadata patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolMetadataProofMessage {
    /// The UTF-8 message, lines joined by `\n`.
    pub message: String,
    /// Lowercase hex SHA-256 of the patch's JCS form.
    pub patch_sha256: String,
    pub expires_at: u64,
}

impl ProtocolMetadataProofMessage {
    /// The bytes the wallet signs.
    pub fn bytes(&self) -> &[u8] {
        self.message.as_bytes()
    }
}

/// The `proof` sent with a metadata update a participant wallet authorises.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolMetadataWalletProof {
    pub wallet: String,
    /// Base58 Ed25519 signature over the proof message.
    pub signature: String,
    /// Unix seconds.
    pub expires_at: u64,
}

pub(crate) fn now_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Build the message a wallet signs to authorise a metadata patch:
///
/// ```text
/// arete.protocol-metadata/v1
/// <deployment id>
/// <instance id>
/// <wallet>
/// <lowercase hex SHA-256 of the patch's RFC 8785 canonical JSON>
/// <expiresAt, unix seconds>
/// ```
pub fn protocol_metadata_proof_message(
    input: &ProtocolMetadataProofInput<'_>,
) -> Result<ProtocolMetadataProofMessage, MetadataProofError> {
    for (name, value) in [
        ("deploymentId", input.deployment_id),
        ("instanceId", input.instance_id),
        ("wallet", input.wallet),
    ] {
        if value.is_empty() || value.contains(['\r', '\n']) {
            return Err(MetadataProofError::InvalidField(name));
        }
    }
    let now = input.now.unwrap_or_else(now_unix_seconds);
    if input.expires_at <= now || input.expires_at - now > PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS {
        return Err(MetadataProofError::ExpiresAt);
    }
    let patch_sha256 = canonical_json_sha256(&Value::Object(input.patch.clone()))?;
    let message = [
        PROTOCOL_METADATA_PROOF_DOMAIN,
        input.deployment_id,
        input.instance_id,
        input.wallet,
        &patch_sha256,
        &input.expires_at.to_string(),
    ]
    .join("\n");
    Ok(ProtocolMetadataProofMessage {
        message,
        patch_sha256,
        expires_at: input.expires_at,
    })
}

/// Turn a wallet's 64-byte Ed25519 signature into the metadata update `proof`.
pub fn protocol_metadata_proof(
    wallet: &str,
    signature: &[u8],
    expires_at: u64,
) -> Result<ProtocolMetadataWalletProof, MetadataProofError> {
    if signature.len() != 64 {
        return Err(MetadataProofError::Signature);
    }
    Ok(ProtocolMetadataWalletProof {
        wallet: wallet.to_string(),
        signature: encode_base58(signature),
        expires_at,
    })
}

/// Build the proof message, have `sign` sign its bytes with the wallet's
/// Ed25519 key, and return the `proof` for a metadata update.
pub async fn sign_protocol_metadata_proof<F, Fut, E>(
    input: &ProtocolMetadataProofInput<'_>,
    sign: F,
) -> Result<ProtocolMetadataWalletProof, SignMetadataProofError<E>>
where
    F: FnOnce(Vec<u8>) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, E>>,
{
    let message = protocol_metadata_proof_message(input).map_err(SignMetadataProofError::Proof)?;
    let signature = sign(message.bytes().to_vec())
        .await
        .map_err(SignMetadataProofError::Sign)?;
    protocol_metadata_proof(input.wallet, &signature, message.expires_at)
        .map_err(SignMetadataProofError::Proof)
}

/// Failure of [`sign_protocol_metadata_proof`]: the proof inputs, or the
/// caller's signer.
#[derive(Debug, thiserror::Error)]
pub enum SignMetadataProofError<E> {
    #[error(transparent)]
    Proof(MetadataProofError),
    #[error("the wallet did not sign the metadata proof")]
    Sign(E),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vectors() -> Value {
        serde_json::from_str(include_str!(
            "../../../../test-vectors/protocol-client/metadata-proof-v1.json"
        ))
        .expect("metadata proof vectors are JSON")
    }

    #[test]
    fn matches_the_cross_language_vectors() {
        let vectors = vectors();
        assert_eq!(
            vectors["maxTtlSeconds"].as_u64(),
            Some(PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS)
        );
        let cases = vectors["cases"].as_array().unwrap();
        assert!(cases.len() >= 5);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let input = &case["input"];
            let patch = input["patch"].as_object().unwrap();
            let result = protocol_metadata_proof_message(&ProtocolMetadataProofInput {
                deployment_id: input["deploymentId"].as_str().unwrap(),
                instance_id: input["instanceId"].as_str().unwrap(),
                wallet: input["wallet"].as_str().unwrap(),
                patch,
                expires_at: input["expiresAt"].as_u64().unwrap(),
                now: input["now"].as_u64(),
            });
            let expect = &case["expected"];
            if expect.get("error").is_none() {
                let message = result.unwrap_or_else(|error| panic!("{name}: {error}"));
                assert_eq!(
                    canonicalize_json(&Value::Object(patch.clone())).unwrap(),
                    expect["canonicalPatch"].as_str().unwrap(),
                    "{name}"
                );
                assert_eq!(message.patch_sha256, expect["patchSha256"], "{name}");
                assert_eq!(message.message, expect["message"], "{name}");
                if let Some(digest) = expect["messageSha256"].as_str() {
                    assert_eq!(hex(&sha256(&[message.bytes()])), digest, "{name}");
                }
            } else {
                let error = result.expect_err(name);
                let code = match error {
                    MetadataProofError::ExpiresAt => "expires-at",
                    MetadataProofError::InvalidField(_) => "invalid-field",
                    other => panic!("{name}: unexpected {other}"),
                };
                assert_eq!(code, expect["error"], "{name}");
            }
        }
    }

    #[test]
    fn numbers_use_the_ecmascript_form() {
        for (value, expected) in [
            (json!(0.1), "0.1"),
            (json!(-0.0), "0"),
            (json!(1e21), "1e+21"),
            (json!(1e20), "100000000000000000000"),
            (json!(1.5e-7), "1.5e-7"),
            (json!(0.000001), "0.000001"),
            (json!(123.456), "123.456"),
            (json!(-1e-7), "-1e-7"),
            (json!(5e300), "5e+300"),
            (json!(9007199254740991u64), "9007199254740991"),
            (json!(-9007199254740991i64), "-9007199254740991"),
        ] {
            assert_eq!(canonicalize_json(&value).unwrap(), expected, "{value}");
        }
        assert!(canonicalize_json(&json!({ "a": [9007199254740992u64] })).is_err());
        assert!(canonicalize_json(&json!(-9007199254740992i64)).is_err());
    }

    #[test]
    fn proofs_carry_a_base58_signature() {
        let proof = protocol_metadata_proof("w", &[1u8; 64], 10).unwrap();
        assert_eq!(proof.signature, encode_base58(&[1u8; 64]));
        assert_eq!(
            protocol_metadata_proof("w", &[1u8; 63], 10),
            Err(MetadataProofError::Signature)
        );
    }

    #[tokio::test]
    async fn sign_hands_the_message_bytes_to_the_signer() {
        let patch = json!({ "name": "x" }).as_object().unwrap().clone();
        let input = ProtocolMetadataProofInput {
            deployment_id: "dep",
            instance_id: "inst",
            wallet: "wal",
            patch: &patch,
            expires_at: 1_000,
            now: Some(900),
        };
        let expected = protocol_metadata_proof_message(&input).unwrap();
        let proof = sign_protocol_metadata_proof(&input, |bytes| async move {
            assert_eq!(bytes, expected.bytes());
            Ok::<_, std::convert::Infallible>(vec![7u8; 64])
        })
        .await
        .unwrap();
        assert_eq!(proof.expires_at, 1_000);
        assert_eq!(proof.wallet, "wal");
    }
}
