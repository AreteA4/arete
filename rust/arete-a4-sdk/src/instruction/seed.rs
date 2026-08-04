//! Typed serialization of PDA seed values and program-address derivation.
//!
//! When a seed carries a declared type (from the IDL), encoding is exact:
//! pubkeys are base58-decoded to 32 bytes, integers are little-endian at the
//! declared width. Without a type, legacy heuristics apply for backward
//! compatibility.

use std::str::FromStr;

use serde_json::Value;
use solana_pubkey::Pubkey;

use super::types::{json_kind, InstructionError};

/// Maximum number of seeds accepted by the runtime.
const MAX_SEEDS: usize = 16;
/// Maximum length of a single seed, in bytes.
const MAX_SEED_LEN: usize = 32;

/// Derives a PDA from serialized seeds and a base58 program ID.
///
/// Returns the derived address together with the canonical bump seed.
pub fn derive_program_address(
    seeds: &[Vec<u8>],
    program_id: &str,
) -> Result<(Pubkey, u8), InstructionError> {
    if seeds.len() > MAX_SEEDS {
        return Err(InstructionError::Pda(format!(
            "Cannot derive a PDA from more than {MAX_SEEDS} seeds (got {})",
            seeds.len()
        )));
    }
    if let Some(seed) = seeds.iter().find(|seed| seed.len() > MAX_SEED_LEN) {
        return Err(InstructionError::Pda(format!(
            "Seed exceeds the maximum length of {MAX_SEED_LEN} bytes (got {})",
            seed.len()
        )));
    }
    let program = Pubkey::from_str(program_id)
        .map_err(|_| InstructionError::InvalidPubkey(program_id.to_string()))?;
    let seed_refs: Vec<&[u8]> = seeds.iter().map(Vec::as_slice).collect();
    Pubkey::try_find_program_address(&seed_refs, &program).ok_or_else(|| {
        InstructionError::Pda("Unable to find a viable program address bump seed".to_string())
    })
}

/// Canonical seed types recognized by [`normalize_seed_type`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalSeedType {
    /// 32-byte public key.
    Pubkey,
    /// UTF-8 string.
    String,
    /// Fixed-width integer.
    Int {
        /// Width in bits (8..=128).
        bits: u32,
        /// Whether the type is signed.
        signed: bool,
    },
}

/// Normalizes the type-name variants that IDLs and codegen produce
/// (`"Pubkey"`, `"publicKey"`, `"solana_pubkey::Pubkey"`, `"String"`, ...) to
/// a canonical seed type. Returns `None` for types that cannot be a seed.
pub fn normalize_seed_type(arg_type: Option<&str>) -> Option<CanonicalSeedType> {
    let raw = arg_type?;
    // Strip any path qualifier (e.g. solana_pubkey::Pubkey).
    let t = raw.rsplit("::").next().unwrap_or(raw).trim();
    match t {
        "pubkey" | "Pubkey" | "publicKey" | "PublicKey" => Some(CanonicalSeedType::Pubkey),
        "string" | "String" | "str" => Some(CanonicalSeedType::String),
        _ => {
            let signed = t.starts_with('i');
            if !signed && !t.starts_with('u') {
                return None;
            }
            let bits = match &t[1..] {
                "8" => 8,
                "16" => 16,
                "32" => 32,
                "64" => 64,
                "128" => 128,
                _ => return None,
            };
            Some(CanonicalSeedType::Int { bits, signed })
        }
    }
}

/// Serializes one PDA seed value.
///
/// With a recognized `arg_type`, encoding is strict and width-exact; an
/// incompatible value errors rather than deriving a wrong address. Without
/// one, the legacy heuristics apply: byte arrays pass through, 43/44-character
/// strings are tried as base58, other strings are UTF-8, integers are 8-byte
/// little-endian.
pub fn serialize_seed_value(
    value: &Value,
    arg_type: Option<&str>,
) -> Result<Vec<u8>, InstructionError> {
    // Raw byte arrays pass through regardless of the declared type (mirror of
    // the TypeScript Uint8Array passthrough).
    if let Value::Array(items) = value {
        return items
            .iter()
            .map(|item| {
                item.as_u64()
                    .filter(|b| *b <= 255)
                    .map(|b| b as u8)
                    .ok_or_else(|| {
                        InstructionError::Pda(
                            "Byte-array seeds must contain integers in 0..=255".to_string(),
                        )
                    })
            })
            .collect();
    }

    match normalize_seed_type(arg_type) {
        Some(CanonicalSeedType::Pubkey) => {
            let Value::String(text) = value else {
                return Err(InstructionError::Pda(format!(
                    "Pubkey seed requires a base58 string, got {}",
                    json_kind(value)
                )));
            };
            let decoded = bs58::decode(text).into_vec().map_err(|_| {
                InstructionError::Pda(format!("Pubkey seed '{text}' is not valid base58"))
            })?;
            if decoded.len() != 32 {
                return Err(InstructionError::Pda(format!(
                    "Pubkey seed '{text}' decoded to {} bytes, expected 32",
                    decoded.len()
                )));
            }
            Ok(decoded)
        }
        Some(CanonicalSeedType::String) => {
            let Value::String(text) = value else {
                return Err(InstructionError::Pda(format!(
                    "String seed requires a string value, got {}",
                    json_kind(value)
                )));
            };
            Ok(text.as_bytes().to_vec())
        }
        Some(CanonicalSeedType::Int { bits, signed }) => {
            let number = seed_int(value).ok_or_else(|| {
                InstructionError::Pda(format!(
                    "Numeric seed of type {}{bits} requires an integer, got {}",
                    if signed { 'i' } else { 'u' },
                    json_kind(value)
                ))
            })?;
            encode_seed_int(&number, (bits / 8) as usize, signed)
        }
        None => match value {
            Value::String(text) => {
                // 43/44-character strings are likely base58 addresses; fall
                // back to UTF-8 when they are not.
                if text.len() == 43 || text.len() == 44 {
                    if let Ok(decoded) = bs58::decode(text).into_vec() {
                        return Ok(decoded);
                    }
                }
                Ok(text.as_bytes().to_vec())
            }
            other => {
                let number = seed_int(other).ok_or_else(|| {
                    InstructionError::Pda(format!(
                        "Cannot serialize value for PDA seed: {}",
                        json_kind(other)
                    ))
                })?;
                encode_seed_int(&number, 8, true)
            }
        },
    }
}

/// A seed integer decoded from a JSON number or decimal string. Non-negative
/// values are held unsigned so the full u128 range is representable.
enum SeedInt {
    Unsigned(u128),
    Signed(i128),
}

impl SeedInt {
    fn to_display(&self) -> String {
        match self {
            SeedInt::Unsigned(u) => u.to_string(),
            SeedInt::Signed(i) => i.to_string(),
        }
    }
}

fn seed_int(value: &Value) -> Option<SeedInt> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .map(|u| SeedInt::Unsigned(u128::from(u)))
            .or_else(|| n.as_i64().map(|i| SeedInt::Signed(i128::from(i)))),
        Value::String(s) => {
            let trimmed = s.trim();
            trimmed
                .parse::<u128>()
                .ok()
                .map(SeedInt::Unsigned)
                .or_else(|| trimmed.parse::<i128>().ok().map(SeedInt::Signed))
        }
        _ => None,
    }
}

/// Little-endian two's-complement encoding at a fixed byte width, with an
/// overflow check so out-of-range values fail instead of silently truncating.
fn encode_seed_int(number: &SeedInt, size: usize, signed: bool) -> Result<Vec<u8>, InstructionError> {
    let (bytes, fits) = match number {
        SeedInt::Unsigned(u) => {
            let fits = size >= 16 || (u >> (size * 8)) == 0;
            (u.to_le_bytes(), fits)
        }
        SeedInt::Signed(i) => {
            // Signed only ever holds negative values (non-negative decode as
            // Unsigned), so an unsigned target never fits.
            let fits = if size >= 16 {
                signed
            } else {
                let remainder = i >> (size * 8);
                signed && (remainder == 0 || remainder == -1)
            };
            (i.to_le_bytes(), fits)
        }
    };
    if !fits {
        return Err(InstructionError::Pda(format!(
            "Seed value {} does not fit in {} bits",
            number.to_display(),
            size * 8
        )));
    }
    Ok(bytes[..size].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

    #[test]
    fn canonicalizes_pubkey_and_string_spellings() {
        for t in ["pubkey", "Pubkey", "publicKey", "PublicKey", "solana_pubkey::Pubkey"] {
            assert_eq!(normalize_seed_type(Some(t)), Some(CanonicalSeedType::Pubkey));
        }
        for t in ["string", "String", "str"] {
            assert_eq!(normalize_seed_type(Some(t)), Some(CanonicalSeedType::String));
        }
    }

    #[test]
    fn passes_integer_widths_through_and_rejects_everything_else() {
        assert_eq!(
            normalize_seed_type(Some("u32")),
            Some(CanonicalSeedType::Int { bits: 32, signed: false })
        );
        assert_eq!(
            normalize_seed_type(Some("i64")),
            Some(CanonicalSeedType::Int { bits: 64, signed: true })
        );
        assert_eq!(normalize_seed_type(Some("u24")), None);
        assert_eq!(normalize_seed_type(Some("Vec<u8>")), None);
        assert_eq!(normalize_seed_type(None), None);
    }

    #[test]
    fn encodes_typed_integers_little_endian_at_the_declared_width() {
        assert_eq!(serialize_seed_value(&json!(1), Some("u8")).unwrap(), vec![1]);
        assert_eq!(serialize_seed_value(&json!(258), Some("u16")).unwrap(), vec![2, 1]);
        assert_eq!(serialize_seed_value(&json!(7), Some("u32")).unwrap(), vec![7, 0, 0, 0]);
        assert_eq!(
            serialize_seed_value(&json!(42), Some("u64")).unwrap(),
            vec![42, 0, 0, 0, 0, 0, 0, 0]
        );
        // Decimal strings mirror the TS bigint path.
        assert_eq!(
            serialize_seed_value(&json!("42"), Some("u64")).unwrap(),
            vec![42, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn encodes_negative_signed_integers_in_twos_complement() {
        assert_eq!(serialize_seed_value(&json!(-1), Some("i64")).unwrap(), vec![0xff; 8]);
    }

    #[test]
    fn rejects_values_that_overflow_the_declared_width() {
        for (value, ty) in [(json!(256), "u8"), (json!(-1), "u32")] {
            let err = serialize_seed_value(&value, Some(ty)).unwrap_err();
            assert!(err.to_string().contains("does not fit"), "unexpected error: {err}");
        }
    }

    #[test]
    fn decodes_pubkey_seeds_from_base58_to_32_bytes() {
        let expected = bs58::decode(TOKEN_PROGRAM).into_vec().unwrap();
        assert_eq!(serialize_seed_value(&json!(TOKEN_PROGRAM), Some("pubkey")).unwrap(), expected);
        assert_eq!(
            serialize_seed_value(&json!(TOKEN_PROGRAM), Some("solana_pubkey::Pubkey")).unwrap(),
            expected
        );
        assert!(
            serialize_seed_value(&json!("abc"), Some("pubkey"))
                .unwrap_err()
                .to_string()
                .contains("expected 32")
        );
        assert!(
            serialize_seed_value(&json!(42), Some("pubkey"))
                .unwrap_err()
                .to_string()
                .contains("base58 string")
        );
    }

    #[test]
    fn utf8_encodes_typed_string_seeds_without_base58_guessing() {
        // 44 chars: the heuristic path would base58-decode this; typed must not.
        let forty_four = "a".repeat(44);
        assert_eq!(
            serialize_seed_value(&json!(forty_four), Some("string")).unwrap(),
            forty_four.as_bytes()
        );
    }

    #[test]
    fn passes_byte_arrays_through() {
        assert_eq!(serialize_seed_value(&json!([1, 2, 255]), None).unwrap(), vec![1, 2, 255]);
    }

    #[test]
    fn untyped_heuristics_try_base58_for_address_length_strings_and_utf8_otherwise() {
        assert_eq!(serialize_seed_value(&json!(TOKEN_PROGRAM), None).unwrap().len(), 32);
        assert_eq!(
            serialize_seed_value(&json!("treasury"), None).unwrap(),
            b"treasury".to_vec()
        );
    }

    #[test]
    fn untyped_numbers_encode_as_8_byte_little_endian() {
        assert_eq!(
            serialize_seed_value(&json!(256), None).unwrap(),
            vec![0, 1, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn derive_program_address_is_deterministic_and_validates_inputs() {
        let seeds = vec![b"treasury".to_vec()];
        let (addr1, bump1) = derive_program_address(&seeds, TOKEN_PROGRAM).unwrap();
        let (addr2, bump2) = derive_program_address(&seeds, TOKEN_PROGRAM).unwrap();
        assert_eq!(addr1, addr2);
        assert_eq!(bump1, bump2);
        assert!(!addr1.is_on_curve());

        let (other, _) = derive_program_address(&[b"other".to_vec()], TOKEN_PROGRAM).unwrap();
        assert_ne!(addr1, other);

        let too_many = vec![b"x".to_vec(); 17];
        assert!(derive_program_address(&too_many, TOKEN_PROGRAM)
            .unwrap_err()
            .to_string()
            .contains("16 seeds"));
        assert!(derive_program_address(&[vec![0; 33]], TOKEN_PROGRAM)
            .unwrap_err()
            .to_string()
            .contains("maximum length"));
        assert!(matches!(
            derive_program_address(&seeds, "not-base58!"),
            Err(InstructionError::InvalidPubkey(_))
        ));
    }
}
