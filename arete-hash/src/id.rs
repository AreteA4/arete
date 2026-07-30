use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::str::FromStr;

use crate::{
    CanonicalizationProfile, HashError, HashKindName, Kind, HASH_ALGORITHM, HASH_PROTOCOL_LABEL,
    HASH_PROTOCOL_VERSION,
};

pub struct HashId<K: Kind> {
    digest: [u8; 32],
    marker: PhantomData<fn() -> K>,
}

impl<K: Kind> HashId<K> {
    pub const fn from_digest(digest: [u8; 32]) -> Self {
        Self {
            digest,
            marker: PhantomData,
        }
    }

    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    pub fn digest_hex(&self) -> String {
        hex::encode(self.digest)
    }

    pub fn into_any(self) -> AnyHashId {
        AnyHashId {
            kind: K::NAME,
            digest: self.digest,
        }
    }
}

impl<K: Kind> Copy for HashId<K> {}

impl<K: Kind> Clone for HashId<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: Kind> PartialEq for HashId<K> {
    fn eq(&self, other: &Self) -> bool {
        self.digest == other.digest
    }
}

impl<K: Kind> Eq for HashId<K> {}

impl<K: Kind> Hash for HashId<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.digest.hash(state);
    }
}

impl<K: Kind> fmt::Display for HashId<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "arete:h{}:{}:{}:{}",
            HASH_PROTOCOL_VERSION,
            K::NAME,
            HASH_ALGORITHM,
            hex::encode(self.digest)
        )
    }
}

impl<K: Kind> fmt::Debug for HashId<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("HashId")
            .field(&self.to_string())
            .finish()
    }
}

impl<K: Kind> FromStr for HashId<K> {
    type Err = HashError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let any = AnyHashId::from_str(value)?;
        if any.kind != K::NAME {
            return Err(HashError::UnexpectedKind {
                expected: K::NAME.to_string(),
                actual: any.kind.to_string(),
            });
        }
        Ok(Self::from_digest(any.digest))
    }
}

impl<K: Kind> Serialize for HashId<K> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de, K: Kind> Deserialize<'de> for HashId<K> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnyHashId {
    kind: HashKindName,
    digest: [u8; 32],
}

impl AnyHashId {
    pub const fn from_parts(kind: HashKindName, digest: [u8; 32]) -> Self {
        Self { kind, digest }
    }

    pub const fn kind(self) -> HashKindName {
        self.kind
    }

    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    pub fn typed<K: Kind>(self) -> Result<HashId<K>, HashError> {
        if self.kind != K::NAME {
            return Err(HashError::UnexpectedKind {
                expected: K::NAME.to_string(),
                actual: self.kind.to_string(),
            });
        }
        Ok(HashId::from_digest(self.digest))
    }
}

impl fmt::Display for AnyHashId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "arete:h{}:{}:{}:{}",
            HASH_PROTOCOL_VERSION,
            self.kind,
            HASH_ALGORITHM,
            hex::encode(self.digest)
        )
    }
}

impl fmt::Debug for AnyHashId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AnyHashId")
            .field(&self.to_string())
            .finish()
    }
}

impl FromStr for AnyHashId {
    type Err = HashError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut parts = value.split(':');
        if parts.next() != Some("arete") {
            return Err(HashError::InvalidHashId("protocol must be 'arete'"));
        }
        let version = parts
            .next()
            .ok_or(HashError::InvalidHashId("missing version"))?;
        if version != "h1" {
            return Err(HashError::UnknownVersion(version.to_string()));
        }
        let kind = parts
            .next()
            .ok_or(HashError::InvalidHashId("missing kind"))?
            .parse()?;
        let algorithm = parts
            .next()
            .ok_or(HashError::InvalidHashId("missing algorithm"))?;
        if algorithm != HASH_ALGORITHM {
            return Err(HashError::UnknownAlgorithm(algorithm.to_string()));
        }
        let digest = parts
            .next()
            .ok_or(HashError::InvalidHashId("missing digest"))?;
        if parts.next().is_some() {
            return Err(HashError::InvalidHashId("too many components"));
        }
        if digest.len() != 64 {
            return Err(HashError::InvalidHashId(
                "digest must contain 64 hex digits",
            ));
        }
        if !digest
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(HashError::InvalidHashId(
                "digest must be lowercase hexadecimal",
            ));
        }
        let mut decoded = [0_u8; 32];
        hex::decode_to_slice(digest, &mut decoded)
            .map_err(|_| HashError::InvalidHashId("invalid digest"))?;
        Ok(Self {
            kind,
            digest: decoded,
        })
    }
}

impl Serialize for AnyHashId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for AnyHashId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

pub fn framed_preimage(
    kind: HashKindName,
    profile: CanonicalizationProfile,
    payload: &[u8],
) -> Vec<u8> {
    let mut preimage = Vec::with_capacity(
        8 + HASH_PROTOCOL_LABEL.len()
            + 4
            + 8
            + kind.as_str().len()
            + 8
            + profile.as_str().len()
            + 8
            + payload.len(),
    );
    push_framed_bytes(&mut preimage, HASH_PROTOCOL_LABEL.as_bytes());
    preimage.extend_from_slice(&HASH_PROTOCOL_VERSION.to_be_bytes());
    push_framed_bytes(&mut preimage, kind.as_str().as_bytes());
    push_framed_bytes(&mut preimage, profile.as_str().as_bytes());
    push_framed_bytes(&mut preimage, payload);
    preimage
}

pub(crate) fn hash_canonical_payload<K: Kind>(payload: &[u8]) -> HashId<K> {
    let preimage = framed_preimage(K::NAME, K::PROFILE, payload);
    HashId::from_digest(Sha256::digest(preimage).into())
}

pub(crate) fn require_profile<K: Kind>(actual: CanonicalizationProfile) -> Result<(), HashError> {
    if K::PROFILE != actual {
        return Err(HashError::ProfileMismatch {
            kind: K::NAME.to_string(),
            expected: K::PROFILE.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

pub(crate) fn push_framed_bytes(output: &mut Vec<u8>, bytes: &[u8]) {
    output.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    output.extend_from_slice(bytes);
}
