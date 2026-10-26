//! Sealed claims: account-only session claims that the bearer cannot read.
//!
//! A session token is a signed JWT, so whoever holds it can decode every
//! claim. That is fine for a token held by a server, but a token handed to a
//! browser visitor should not tell the visitor about the account that issued
//! it: its plan, its limits, its billing identity.
//!
//! Sealing moves those claims into one `sealed` claim, encrypted with
//! XChaCha20-Poly1305 under a key shared by the issuer and the services that
//! verify the token. The token stays signed as a whole, so the sealed claim is
//! as trustworthy as the rest. Verifiers configured with the key open it and
//! put the claims back before building an [`AuthContext`](crate::AuthContext),
//! so limits and metering work exactly as they do on an unsealed token. A
//! verifier without the key refuses a sealed token rather than enforcing
//! nothing.
//!
//! The sealed value is `<key id>.<base64url(nonce || ciphertext)>`. The
//! ciphertext is bound to the token's `iss`, `aud` and `jti`, so a sealed
//! value cannot be moved into another token.

use std::fmt;
use std::sync::Arc;

use base64::Engine;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde_json::{Map, Value};

/// Name of the claim that carries the encrypted claims.
pub const SEALED_CLAIM: &str = "sealed";

/// The account-only claims a visitor session seals: billing identity, plan,
/// limits and policy identities.
pub const ACCOUNT_CLAIMS: &[&str] = &[
    "metering_key",
    "limits",
    "plan",
    "actor_key",
    "account_key",
    "consumer_key",
    "policy_version",
    "account_limits",
];

const NONCE_BYTES: usize = 24;
const MAX_KEY_ID_BYTES: usize = 64;
const AAD_PREFIX: &[u8] = b"arete-sealed-claims:v1";

/// Failure to seal or open sealed claims.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SealError {
    #[error("claim seal key id must be 1-{MAX_KEY_ID_BYTES} bytes of [A-Za-z0-9_-]")]
    InvalidKeyId,
    #[error("claim seal key must be 32 bytes of base64")]
    InvalidKey,
    #[error("token carries sealed claims but this verifier has no claim seal key")]
    KeyRequired,
    #[error("sealed claims use an unknown key: {0}")]
    UnknownKey(String),
    #[error("sealed claims are malformed")]
    Malformed,
    #[error("sealed claims could not be opened")]
    Open,
    #[error("sealed claim {0} is also present in the clear")]
    Duplicate(String),
    #[error("claims to seal need {0}")]
    MissingBinding(&'static str),
    #[error("claims to seal already carry a sealed claim")]
    AlreadySealed,
}

/// One symmetric key for sealing and opening claims.
#[derive(Clone, PartialEq, Eq)]
pub struct ClaimSealKey {
    id: String,
    key: [u8; 32],
}

impl fmt::Debug for ClaimSealKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClaimSealKey")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl ClaimSealKey {
    /// A key from its id and 32 raw bytes.
    pub fn new(id: impl Into<String>, key: [u8; 32]) -> Result<Self, SealError> {
        let id = id.into();
        let valid_id = !id.is_empty()
            && id.len() <= MAX_KEY_ID_BYTES
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        if !valid_id {
            return Err(SealError::InvalidKeyId);
        }
        Ok(Self { id, key })
    }

    /// A key from its id and 32 bytes in standard or URL-safe base64.
    pub fn from_base64(id: impl Into<String>, key: &str) -> Result<Self, SealError> {
        let key = key.trim();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(key)
            .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(key))
            .map_err(|_| SealError::InvalidKey)?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| SealError::InvalidKey)?;
        Self::new(id, bytes)
    }

    /// The key id written in front of every value this key seals.
    pub fn id(&self) -> &str {
        &self.id
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(&self.key.into())
    }
}

/// The keys a verifier opens sealed claims with. The first key also seals.
///
/// Rotation: add the new key behind the current one on every verifier, then
/// make it first on the issuer, then drop the old key once its tokens expire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimSealKeys {
    keys: Arc<[ClaimSealKey]>,
}

impl ClaimSealKeys {
    /// A key set with one key.
    pub fn new(key: ClaimSealKey) -> Self {
        Self {
            keys: Arc::from(vec![key]),
        }
    }

    /// A key set from a list; the first key seals. Key ids must be unique.
    pub fn from_keys(keys: Vec<ClaimSealKey>) -> Result<Self, SealError> {
        if keys.is_empty() {
            return Err(SealError::InvalidKey);
        }
        for (index, key) in keys.iter().enumerate() {
            if keys[..index].iter().any(|other| other.id == key.id) {
                return Err(SealError::InvalidKeyId);
            }
        }
        Ok(Self {
            keys: Arc::from(keys),
        })
    }

    /// Parse `id=base64[,id=base64...]`; the first key seals.
    pub fn parse(value: &str) -> Result<Self, SealError> {
        let keys = value
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                let (id, key) = entry.split_once('=').ok_or(SealError::InvalidKey)?;
                ClaimSealKey::from_base64(id.trim(), key)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::from_keys(keys)
    }

    /// The key new tokens are sealed with.
    pub fn sealing_key(&self) -> &ClaimSealKey {
        &self.keys[0]
    }

    fn find(&self, id: &str) -> Option<&ClaimSealKey> {
        self.keys.iter().find(|key| key.id == id)
    }
}

fn binding(claims: &Map<String, Value>) -> Result<Vec<u8>, SealError> {
    let field = |name: &'static str| {
        claims
            .get(name)
            .and_then(Value::as_str)
            .ok_or(SealError::MissingBinding(name))
    };
    let mut aad = AAD_PREFIX.to_vec();
    for value in [field("iss")?, field("aud")?, field("jti")?] {
        aad.push(0);
        aad.extend_from_slice(value.as_bytes());
    }
    Ok(aad)
}

/// Move the named claims, where present, into an encrypted `sealed` claim.
///
/// The claims must carry `iss`, `aud` and `jti`. Nothing is added when none of
/// the named claims is present.
pub fn seal_claims(
    claims: &mut Map<String, Value>,
    names: &[&str],
    key: &ClaimSealKey,
) -> Result<(), SealError> {
    if claims.contains_key(SEALED_CLAIM) {
        return Err(SealError::AlreadySealed);
    }
    let aad = binding(claims)?;
    let mut private = Map::new();
    for name in names {
        if let Some(value) = claims.remove(*name) {
            private.insert((*name).to_string(), value);
        }
    }
    if private.is_empty() {
        return Ok(());
    }
    let plaintext = serde_json::to_vec(&private).map_err(|_| SealError::Malformed)?;
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = key
        .cipher()
        .encrypt(
            &nonce,
            Payload {
                msg: &plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| SealError::Malformed)?;
    let mut sealed = nonce.to_vec();
    sealed.extend_from_slice(&ciphertext);
    claims.insert(
        SEALED_CLAIM.to_string(),
        Value::String(format!(
            "{}.{}",
            key.id,
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sealed)
        )),
    );
    Ok(())
}

/// Open the `sealed` claim, if any, and put its claims back in the clear.
///
/// A token without a `sealed` claim is left as it is. A sealed token needs
/// `keys`; without them, or when the value does not open, this fails.
pub fn unseal_claims(
    claims: &mut Map<String, Value>,
    keys: Option<&ClaimSealKeys>,
) -> Result<(), SealError> {
    let Some(sealed) = claims.get(SEALED_CLAIM) else {
        return Ok(());
    };
    let keys = keys.ok_or(SealError::KeyRequired)?;
    let sealed = sealed.as_str().ok_or(SealError::Malformed)?;
    let (key_id, payload) = sealed.split_once('.').ok_or(SealError::Malformed)?;
    let key = keys
        .find(key_id)
        .ok_or_else(|| SealError::UnknownKey(key_id.to_string()))?;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| SealError::Malformed)?;
    if payload.len() <= NONCE_BYTES {
        return Err(SealError::Malformed);
    }
    let (nonce, ciphertext) = payload.split_at(NONCE_BYTES);
    let aad = binding(claims)?;
    let plaintext = key
        .cipher()
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| SealError::Open)?;
    let private: Map<String, Value> =
        serde_json::from_slice(&plaintext).map_err(|_| SealError::Malformed)?;
    claims.remove(SEALED_CLAIM);
    for (name, value) in private {
        if claims.contains_key(&name) {
            return Err(SealError::Duplicate(name));
        }
        claims.insert(name, value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(id: &str, byte: u8) -> ClaimSealKey {
        ClaimSealKey::new(id, [byte; 32]).unwrap()
    }

    fn claims() -> Map<String, Value> {
        json!({
            "iss": "issuer", "sub": "anon:visitor", "aud": "audience", "jti": "j-1",
            "scope": "read", "key_class": "publishable",
            "metering_key": "account:42", "plan": "pro",
            "limits": {"max_connections": 5},
            "account_key": "account:42", "account_limits": {"max_connections": 100},
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn sealing_hides_the_named_claims_and_opening_restores_them() {
        let key = key("k1", 7);
        let original = claims();
        let mut sealed = original.clone();
        seal_claims(&mut sealed, ACCOUNT_CLAIMS, &key).unwrap();

        for name in ACCOUNT_CLAIMS {
            assert!(!sealed.contains_key(*name), "{name} is still visible");
        }
        let text = serde_json::to_string(&sealed).unwrap();
        assert!(!text.contains("account:42"), "{text}");
        assert!(!text.contains("pro"), "{text}");
        assert_eq!(sealed["sub"], "anon:visitor");
        assert!(sealed[SEALED_CLAIM].as_str().unwrap().starts_with("k1."));

        unseal_claims(&mut sealed, Some(&ClaimSealKeys::new(key))).unwrap();
        assert_eq!(sealed, original);
    }

    #[test]
    fn unsealed_claims_pass_through_without_keys() {
        let mut plain = claims();
        unseal_claims(&mut plain, None).unwrap();
        assert_eq!(plain, claims());
    }

    #[test]
    fn nothing_to_seal_adds_no_sealed_claim() {
        let mut plain = json!({"iss": "i", "aud": "a", "jti": "j", "sub": "s"})
            .as_object()
            .unwrap()
            .clone();
        seal_claims(&mut plain, ACCOUNT_CLAIMS, &key("k1", 1)).unwrap();
        assert!(!plain.contains_key(SEALED_CLAIM));
    }

    #[test]
    fn sealed_claims_need_the_right_key_and_the_same_token() {
        let mut sealed = claims();
        seal_claims(&mut sealed, ACCOUNT_CLAIMS, &key("k1", 7)).unwrap();

        assert_eq!(
            unseal_claims(&mut sealed.clone(), None),
            Err(SealError::KeyRequired)
        );
        assert_eq!(
            unseal_claims(&mut sealed.clone(), Some(&ClaimSealKeys::new(key("k2", 7)))),
            Err(SealError::UnknownKey("k1".into()))
        );
        assert_eq!(
            unseal_claims(&mut sealed.clone(), Some(&ClaimSealKeys::new(key("k1", 8)))),
            Err(SealError::Open)
        );

        let mut moved = sealed.clone();
        moved.insert("jti".into(), json!("j-2"));
        assert_eq!(
            unseal_claims(&mut moved, Some(&ClaimSealKeys::new(key("k1", 7)))),
            Err(SealError::Open)
        );

        let mut doubled = sealed;
        doubled.insert("plan".into(), json!("free"));
        assert_eq!(
            unseal_claims(&mut doubled, Some(&ClaimSealKeys::new(key("k1", 7)))),
            Err(SealError::Duplicate("plan".into()))
        );
    }

    #[test]
    fn a_rotated_key_set_still_opens_the_previous_key() {
        let mut sealed = claims();
        seal_claims(&mut sealed, ACCOUNT_CLAIMS, &key("old", 1)).unwrap();
        let keys = ClaimSealKeys::from_keys(vec![key("new", 2), key("old", 1)]).unwrap();
        assert_eq!(keys.sealing_key().id(), "new");
        unseal_claims(&mut sealed, Some(&keys)).unwrap();
        assert_eq!(sealed, claims());
    }

    #[test]
    fn key_sets_parse_and_reject_bad_input() {
        let encoded = base64::engine::general_purpose::STANDARD.encode([3u8; 32]);
        let keys = ClaimSealKeys::parse(&format!("a={encoded}, b={encoded}")).unwrap();
        assert_eq!(keys.sealing_key().id(), "a");
        assert!(ClaimSealKeys::parse("").is_err());
        assert!(ClaimSealKeys::parse(&format!("a={encoded},a={encoded}")).is_err());
        assert!(ClaimSealKeys::parse("a=short").is_err());
        assert!(ClaimSealKeys::parse(&format!("a.b={encoded}")).is_err());
        assert!(!format!("{:?}", keys.sealing_key()).contains(&encoded));
    }
}
