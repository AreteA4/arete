//! Managed Solana capabilities v1. Wire fixtures live in tests/fixtures/managed-solana-v1.
use serde::{Deserialize, Serialize};

pub const CONTRACT_VERSION: &str = "managed-solana/v1";
pub const MAX_BATCH_ADDRESSES: usize = 100;
pub const MAX_PAGE_SIZE: u16 = 100;
pub const MAX_CURSOR_BYTES: usize = 2048;

/// A chain account's deletion, independent of any entity/view mapping.
/// Ingestion emits this only after recognizing an authoritative tombstone and
/// enforcing its replay watermark; empty data alone does not imply deletion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountTombstone {
    pub address: String,
    #[serde(with = "decimal_u64")]
    pub slot: u64,
    #[serde(with = "decimal_u64")]
    pub write_version: u64,
}

impl AccountTombstone {
    pub fn validate(&self) -> Result<(), String> {
        validate_address(&self.address)
    }
}

pub mod decimal_u64 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(serde::de::Error::custom("expected decimal u64 string"));
        }
        value.parse().map_err(serde::de::Error::custom)
    }
}

pub mod optional_decimal_u64 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(value) => serializer.serialize_some(&value.to_string()),
            None => serializer.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<u64>, D::Error> {
        let value = Option::<String>::deserialize(deserializer)?;
        value
            .map(|value| {
                if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(serde::de::Error::custom("expected decimal u64 string"));
                }
                value.parse().map_err(serde::de::Error::custom)
            })
            .transpose()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Commitment {
    Processed,
    Confirmed,
    Finalized,
}

impl Commitment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Processed => "processed",
            Self::Confirmed => "confirmed",
            Self::Finalized => "finalized",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commitment: Option<Commitment>,
    #[serde(
        default,
        with = "optional_decimal_u64",
        skip_serializing_if = "Option::is_none"
    )]
    pub min_context_slot: Option<u64>,
}

impl ReadOptions {
    pub fn rpc_config(self) -> serde_json::Value {
        let mut value = serde_json::json!({ "encoding": "base64", "commitment": self.commitment.unwrap_or(Commitment::Confirmed).as_str() });
        if let Some(slot) = self.min_context_slot {
            value["minContextSlot"] = slot.into();
        }
        value
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadContext {
    #[serde(with = "decimal_u64")]
    pub slot: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct Contextual<T> {
    /// None only when no underlying read took place (an empty batch).
    #[serde(deserialize_with = "required_value")]
    pub context: Option<ReadContext>,
    #[serde(deserialize_with = "required_value")]
    pub value: T,
}

fn required_value<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    T::deserialize(deserializer)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryProvenance {
    /// Index/provider identity. This does not assert a chain commitment.
    pub source: String,
    /// Provider observation time as RFC3339, never fabricated by an SDK.
    pub observed_at: String,
    #[serde(
        default,
        with = "optional_decimal_u64",
        skip_serializing_if = "Option::is_none"
    )]
    pub watermark: Option<u64>,
}

impl DiscoveryProvenance {
    pub fn validate(&self) -> Result<(), String> {
        if self.source.is_empty()
            || chrono::DateTime::parse_from_rfc3339(&self.observed_at).is_err()
        {
            return Err("provider omitted valid RFC3339 provenance".into());
        }
        Ok(())
    }
}

fn default_limit() -> u16 {
    MAX_PAGE_SIZE
}
pub fn validate_address(address: &str) -> Result<(), String> {
    if address.len() <= 44
        && bs58::decode(address)
            .into_vec()
            .is_ok_and(|bytes| bytes.len() == 32)
    {
        Ok(())
    } else {
        Err("address must be a base58-encoded 32-byte value".into())
    }
}
pub fn validate_page(limit: u16, cursor: Option<&str>) -> Result<(), String> {
    if limit == 0 || limit > MAX_PAGE_SIZE {
        return Err(format!("limit must be between 1 and {MAX_PAGE_SIZE}"));
    }
    if cursor.is_some_and(|c| c.is_empty() || c.len() > MAX_CURSOR_BYTES) {
        return Err("cursor must contain 1..2048 bytes".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnerTokenAccountsRequest {
    pub owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_program: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}
impl OwnerTokenAccountsRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_address(&self.owner)?;
        for address in [&self.mint, &self.token_program].into_iter().flatten() {
            validate_address(address)?;
        }
        validate_page(self.limit, self.cursor.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerTokenAccount {
    pub address: String,
    pub mint: String,
    pub token_program: String,
    pub owner: String,
    #[serde(with = "decimal_u64")]
    pub amount: u64,
    pub decimals: u8,
    /// initialized, frozen, or uninitialized (provider must supply actual state).
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegate: Option<String>,
    #[serde(
        default,
        with = "optional_decimal_u64",
        skip_serializing_if = "Option::is_none"
    )]
    pub delegated_amount: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_authority: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerTokenAccountsPage {
    pub items: Vec<OwnerTokenAccount>,
    #[serde(deserialize_with = "required_value")]
    pub next_cursor: Option<String>,
    pub discovery: DiscoveryProvenance,
}
impl OwnerTokenAccountsPage {
    pub fn validate(&self, request: &OwnerTokenAccountsRequest) -> Result<(), String> {
        validate_page(request.limit, self.next_cursor.as_deref())?;
        if self.items.len() > request.limit as usize {
            return Err("provider exceeded page limit".into());
        }
        self.discovery.validate()?;
        for item in &self.items {
            for address in [&item.address, &item.owner, &item.mint, &item.token_program] {
                validate_address(address)?;
            }
            for address in [&item.delegate, &item.close_authority]
                .into_iter()
                .flatten()
            {
                validate_address(address)?;
            }
            if item.owner != request.owner
                || request.mint.as_ref().is_some_and(|m| *m != item.mint)
                || request
                    .token_program
                    .as_ref()
                    .is_some_and(|p| *p != item.token_program)
            {
                return Err("provider returned an account outside the requested filters".into());
            }
            if !matches!(
                item.state.as_str(),
                "initialized" | "frozen" | "uninitialized"
            ) {
                return Err("provider returned an unsupported token state".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativePositionQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}
impl NativePositionQuery {
    pub fn validate(&self) -> Result<(), String> {
        if self.owner.is_none() && self.pool.is_none() {
            return Err("owner or pool is required".into());
        }
        for address in [&self.owner, &self.pool].into_iter().flatten() {
            validate_address(address)?;
        }
        validate_page(self.limit, self.cursor.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePositionPage {
    /// Discovery returns addresses; verified typed reads are a separate contextual operation.
    pub addresses: Vec<String>,
    #[serde(deserialize_with = "required_value")]
    pub next_cursor: Option<String>,
    pub discovery: DiscoveryProvenance,
}

impl Default for NativePositionQuery {
    fn default() -> Self {
        Self {
            owner: None,
            pool: None,
            limit: MAX_PAGE_SIZE,
            cursor: None,
        }
    }
}

use serde_json::Value;
/// Preserve parsed instructions and future fields. Monetary/resource integers are decimal strings;
/// all other integers outside JavaScript's exact range are strings too. Account indexes stay numeric.
pub fn precision_safe_json(value: &Value, key: &str) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), precision_safe_json(value, key)))
                .collect(),
        ),
        Value::Array(array) => Value::Array(
            array
                .iter()
                .map(|value| precision_safe_json(value, key))
                .collect(),
        ),
        Value::Number(number) if number.is_u64() || number.is_i64() => {
            let exact_field = matches!(
                key,
                "fee"
                    | "preBalances"
                    | "postBalances"
                    | "lamports"
                    | "postBalance"
                    | "computeUnitsConsumed"
                    | "costUnits"
                    | "loadedAccountsDataSize"
            );
            let unsafe_integer = number.as_u64().is_some_and(|n| n > 9_007_199_254_740_991)
                || number.as_i64().is_some_and(|n| n < -9_007_199_254_740_991);
            if exact_field || unsafe_integer {
                Value::String(number.to_string())
            } else {
                value.clone()
            }
        }
        _ => value.clone(),
    }
}
