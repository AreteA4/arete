use serde::{Deserialize, Serialize};
use arete_sdk::serde_utils;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlphaVaultId {
    #[serde(default)]
    pub address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlphaVaultState {
    #[serde(default)]
    pub header: Option<Option<Header>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlphaVault {
    #[serde(default)]
    pub id: AlphaVaultId,
    #[serde(default)]
    pub state: AlphaVaultState,
}



#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Header {
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub version: Option<u64>,
    #[serde(default)]
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BetaVaultId {
    #[serde(default)]
    pub address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BetaVaultState {
    #[serde(default)]
    pub header: Option<Option<BetaHeader>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BetaVault {
    #[serde(default)]
    pub id: BetaVaultId,
    #[serde(default)]
    pub state: BetaVaultState,
}



#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BetaHeader {
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub version: Option<u64>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub flags: Option<u64>,
}


/// Wrapper for event data that includes context metadata.
/// Events are automatically wrapped in this structure at runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventWrapper<T> {
    /// Unix timestamp when the event was processed.
    #[serde(default, deserialize_with = "serde_utils::deserialize_i64")]
    pub timestamp: i64,
    /// The event-specific data.
    pub data: T,
    /// Optional blockchain slot number.
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub slot: Option<u64>,
    /// Optional transaction signature.
    #[serde(default)]
    pub signature: Option<String>,
    /// Absolute log-line index; set only for events decoded from a log.
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub event_index: Option<u64>,
    /// 0-based instruction path within the transaction (e.g. `"0.1"`).
    #[serde(default)]
    pub ix_path: Option<String>,
}

impl<T: Default> Default for EventWrapper<T> {
    fn default() -> Self {
        Self {
            timestamp: 0,
            data: T::default(),
            slot: None,
            signature: None,
            event_index: None,
            ix_path: None,
        }
    }
}

/// Wrapper for account data captured with `#[capture]`, including context
/// metadata. Captured accounts are automatically wrapped in this structure at
/// runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureWrapper<T> {
    /// Unix timestamp when the account was captured.
    #[serde(default, deserialize_with = "serde_utils::deserialize_i64")]
    pub timestamp: i64,
    /// The account address (base58 encoded public key).
    #[serde(default)]
    pub account_address: String,
    /// The captured account data.
    pub data: T,
    /// Optional blockchain slot number.
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub slot: Option<u64>,
    /// Optional transaction signature.
    #[serde(default)]
    pub signature: Option<String>,
}

impl<T: Default> Default for CaptureWrapper<T> {
    fn default() -> Self {
        Self {
            timestamp: 0,
            account_address: String::new(),
            data: T::default(),
            slot: None,
            signature: None,
        }
    }
}
