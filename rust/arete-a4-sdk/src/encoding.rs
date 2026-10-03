//! Hashing and base58 helpers for extension bundles
//! (`docs/internal/sdk-core-api.md` §9).
//!
//! Extensions use these instead of carrying their own implementations: the
//! TypeScript SDK's `encodeBase58`/`decodeBase58` (with its error text), and
//! the Keccak-256 and SHA-256 digests Solana programs hash with
//! (`keccak::hashv`, `hash::hashv`). Each hash takes its input as parts and
//! hashes their concatenation, as `hashv` does.

use sha2::Digest as _;
use thiserror::Error;

use crate::error::AreteError;

/// The base58 alphabet Solana addresses use (Bitcoin's).
pub const BASE58_ALPHABET: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Text that is not base58. `Display` is the TypeScript `decodeBase58`
/// message, `Invalid base58 character: <character>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("Invalid base58 character: {character}")]
pub struct Base58Error {
    /// The first character of the text that is not in [`BASE58_ALPHABET`].
    pub character: char,
}

/// Text that is not base58 is an extension input error
/// ([`AreteError::InvalidInput`]) carrying the TypeScript message.
impl From<Base58Error> for AreteError {
    fn from(error: Base58Error) -> Self {
        AreteError::InvalidInput(error.to_string())
    }
}

/// Keccak-256 of the concatenation of `parts`: the original Keccak padding
/// that Solana's `keccak::hashv` (and Ethereum) use, which is not SHA3-256.
pub fn keccak256(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = sha3::Keccak256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

/// SHA-256 of the concatenation of `parts` (Solana's `hash::hashv`).
pub fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = sha2::Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

/// Encodes bytes as base58 (TypeScript `encodeBase58`). Each leading zero
/// byte becomes a leading `1`; no bytes encode as the empty string.
pub fn encode_base58(bytes: &[u8]) -> String {
    bs58::encode(bytes).into_string()
}

/// Decodes base58 text (TypeScript `decodeBase58`). Each leading `1` becomes
/// a leading zero byte; the empty string decodes to no bytes. Text with a
/// character outside [`BASE58_ALPHABET`] fails with the TypeScript message.
pub fn decode_base58(text: &str) -> Result<Vec<u8>, Base58Error> {
    bs58::decode(text).into_vec().map_err(|_| Base58Error {
        // The decoder stops at the first character outside the alphabet,
        // which is the one TypeScript reports.
        character: text
            .chars()
            .find(|character| !BASE58_ALPHABET.contains(*character))
            .unwrap_or(char::REPLACEMENT_CHARACTER),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn keccak256_matches_known_vectors() {
        assert_eq!(
            hex(&keccak256(&[])),
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
        assert_eq!(
            hex(&keccak256(&[b""])),
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
        assert_eq!(
            hex(&keccak256(&[b"abc"])),
            "4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45"
        );
        assert_eq!(
            hex(&keccak256(&[
                b"The quick brown fox jumps over the lazy dog"
            ])),
            "4d741b6f1eb29cb2a9b9911c82f56fa8d73b04959d3d9d222895df6c0b28aa15"
        );
    }

    #[test]
    fn keccak256_is_not_sha3_256() {
        // SHA3-256("") is a7ffc6f8…; Keccak-256 pads differently.
        assert_ne!(
            hex(&keccak256(&[b""])),
            "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a"
        );
    }

    #[test]
    fn keccak256_hashes_the_concatenation_of_its_parts() {
        // Inputs around the 136-byte rate: a part boundary must not matter.
        let message: Vec<u8> = (0..=255u8).cycle().take(300).collect();
        for split in [0, 1, 135, 136, 137, 272, 300] {
            let (left, right) = message.split_at(split);
            assert_eq!(keccak256(&[left, right]), keccak256(&[&message]));
        }
        assert_eq!(
            keccak256(&[b"The quick ", b"brown fox ", b"jumps over the lazy dog"]),
            keccak256(&[b"The quick brown fox jumps over the lazy dog"])
        );
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            hex(&sha256(&[])),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(&[b"abc"])),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(&[b"a", b"b", b"c"])),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(&[
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            ])),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    /// Bitcoin Core's `base58_encode_decode.json` vectors.
    const BASE58_VECTORS: &[(&str, &str)] = &[
        ("", ""),
        ("61", "2g"),
        ("626262", "a3gV"),
        ("636363", "aPEr"),
        (
            "73696d706c792061206c6f6e6720737472696e67",
            "2cFupjhnEsSn59qHXstmK2ffpLv2",
        ),
        (
            "00eb15231dfceb60925886b67d065299925915aeb172c06647",
            "1NS17iag9jJgTHD1VXjvLCEnZuQ3rJDE9L",
        ),
        ("516b6fcd0f", "ABnLTmg"),
        ("bf4f89001e670274dd", "3SEo3LWLoPntC"),
        ("572e4794", "3EFU7m"),
        ("ecac89cad93923c02321", "EJDM8drfXA6uyA"),
        ("10c8511e", "Rt5zm"),
        ("00000000000000000000", "1111111111"),
    ];

    #[test]
    fn base58_round_trips_known_vectors() {
        for (bytes, text) in BASE58_VECTORS {
            assert_eq!(encode_base58(&unhex(bytes)), *text, "encode {bytes}");
            assert_eq!(decode_base58(text).unwrap(), unhex(bytes), "decode {text}");
        }
    }

    #[test]
    fn base58_round_trips_solana_addresses() {
        // The System Program is 32 zero bytes.
        assert_eq!(encode_base58(&[0; 32]), "1".repeat(32));
        assert_eq!(decode_base58(&"1".repeat(32)).unwrap(), vec![0; 32]);

        let token_program = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
        let decoded = decode_base58(token_program).unwrap();
        assert_eq!(decoded.len(), 32);
        assert_eq!(encode_base58(&decoded), token_program);
    }

    #[test]
    fn decode_base58_reports_the_first_invalid_character_as_typescript_does() {
        for (text, character) in [
            ("0", '0'),
            ("abc0O", '0'),
            ("I", 'I'),
            ("l1", 'l'),
            ("1é0", 'é'),
            ("11+", '+'),
            (" 1", ' '),
        ] {
            let error = decode_base58(text).unwrap_err();
            assert_eq!(error, Base58Error { character }, "{text:?}");
            assert_eq!(
                error.to_string(),
                format!("Invalid base58 character: {character}")
            );
        }
    }

    #[test]
    fn base58_errors_convert_to_invalid_input() {
        let error: AreteError = decode_base58("0").unwrap_err().into();
        assert!(matches!(error, AreteError::InvalidInput(_)));
        assert_eq!(error.to_string(), "Invalid base58 character: 0");
    }
}
