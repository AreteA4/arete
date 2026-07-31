use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};

use crate::HashError;

macro_rules! prefixed_identifier {
    ($type:ident, $projection:literal, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $type(String);

        impl $type {
            pub fn new(value: impl Into<String>) -> Result<Self, HashError> {
                let value = value.into();
                let suffix =
                    value
                        .strip_prefix($prefix)
                        .ok_or_else(|| HashError::InvalidProjection {
                            projection: $projection,
                            reason: concat!("identifier must begin with '", $prefix, "'")
                                .to_string(),
                        })?;
                if suffix.len() != 32
                    || !suffix
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
                {
                    return Err(HashError::InvalidProjection {
                        projection: $projection,
                        reason: "identifier suffix must contain exactly 32 URL-safe characters"
                            .to_string(),
                    });
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $type {
            type Err = HashError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}

prefixed_identifier!(ProgramReadBindingId, "program read binding", "prb_");
prefixed_identifier!(DecoderBindingId, "decoder binding", "dec_");

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct DecoderEngineId(String);

impl DecoderEngineId {
    pub fn new(value: impl Into<String>) -> Result<Self, HashError> {
        let value = value.into();
        if value.is_empty() || value.len() > 128 {
            return Err(HashError::InvalidProjection {
                projection: "decoder engine",
                reason: "identifier must contain between 1 and 128 bytes".to_string(),
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DecoderEngineId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for DecoderEngineId {
    type Err = HashError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for DecoderEngineId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
