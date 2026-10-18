//! Webhook signature verification for emitted protocol events.
//!
//! Mirror of `typescript/core/src/protocol/webhooks.ts`. A spec's `emit`
//! reactions publish named events; a webhook endpoint subscribed to given
//! event names receives a signed POST. Payloads carry ids and states only;
//! receivers read details with their own credentials.
//!
//! Signature scheme (`v1`):
//!
//! - `arete-webhook-id`: the delivery id, stable across retries;
//! - `arete-webhook-timestamp`: Unix seconds when the delivery was signed;
//! - `arete-webhook-signature`: one or more space-separated `v1=<hex>`
//!   entries, each a lowercase hex HMAC-SHA256 over `{timestamp}.{body}`
//!   keyed with the endpoint secret. Several entries appear while a secret
//!   is being rotated; any one matching is enough.

use std::collections::{BTreeMap, HashMap};

use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use super::metadata_proof::now_unix_seconds;
use super::types::ProtocolEventPayloadV1;

pub const WEBHOOK_ID_HEADER: &str = "arete-webhook-id";
pub const WEBHOOK_TIMESTAMP_HEADER: &str = "arete-webhook-timestamp";
pub const WEBHOOK_SIGNATURE_HEADER: &str = "arete-webhook-signature";
pub const WEBHOOK_TOLERANCE_SECONDS: u64 = 300;

/// Why a webhook delivery failed verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WebhookErrorCode {
    MissingHeader,
    InvalidTimestamp,
    TimestampOutOfTolerance,
    InvalidSignature,
    InvalidPayload,
}

impl WebhookErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingHeader => "missing-header",
            Self::InvalidTimestamp => "invalid-timestamp",
            Self::TimestampOutOfTolerance => "timestamp-out-of-tolerance",
            Self::InvalidSignature => "invalid-signature",
            Self::InvalidPayload => "invalid-payload",
        }
    }
}

impl std::fmt::Display for WebhookErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("webhook verification failed ({code}): {message}")]
pub struct WebhookVerificationError {
    pub code: WebhookErrorCode,
    pub message: String,
}

impl WebhookVerificationError {
    fn new(code: WebhookErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Request headers, looked up case-insensitively. Several values of one
/// header are joined with a space.
pub trait WebhookHeaders {
    fn webhook_header(&self, name: &str) -> Option<String>;
}

impl WebhookHeaders for reqwest::header::HeaderMap {
    fn webhook_header(&self, name: &str) -> Option<String> {
        let values = self
            .get_all(name)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect::<Vec<_>>();
        (!values.is_empty()).then(|| values.join(" "))
    }
}

fn find_header<'a, K: AsRef<str> + 'a, V: AsRef<str> + 'a>(
    entries: impl Iterator<Item = (&'a K, &'a V)>,
    name: &str,
) -> Option<String> {
    let values = entries
        .filter(|(key, _)| key.as_ref().eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_ref().to_string())
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.join(" "))
}

impl<K: AsRef<str>, V: AsRef<str>, S> WebhookHeaders for HashMap<K, V, S> {
    fn webhook_header(&self, name: &str) -> Option<String> {
        find_header(self.iter(), name)
    }
}

impl<K: AsRef<str>, V: AsRef<str>> WebhookHeaders for BTreeMap<K, V> {
    fn webhook_header(&self, name: &str) -> Option<String> {
        find_header(self.iter(), name)
    }
}

impl<K: AsRef<str>, V: AsRef<str>> WebhookHeaders for [(K, V)] {
    fn webhook_header(&self, name: &str) -> Option<String> {
        find_header(self.iter().map(|(key, value)| (key, value)), name)
    }
}

impl<K: AsRef<str>, V: AsRef<str>> WebhookHeaders for Vec<(K, V)> {
    fn webhook_header(&self, name: &str) -> Option<String> {
        self.as_slice().webhook_header(name)
    }
}

impl<T: WebhookHeaders + ?Sized> WebhookHeaders for &T {
    fn webhook_header(&self, name: &str) -> Option<String> {
        (**self).webhook_header(name)
    }
}

/// Verification options.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VerifyWebhookOptions {
    /// Unix seconds; the current time when `None`.
    pub now: Option<u64>,
    /// Allowed clock skew in seconds; 300 when `None`.
    pub tolerance_seconds: Option<u64>,
}

/// A verified delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedWebhook<T = ProtocolEventPayloadV1> {
    pub id: String,
    pub timestamp: u64,
    pub payload: T,
}

fn hmac_sha256(key: &[u8], message: &[&[u8]]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|byte| byte ^ 0x36));
    for part in message {
        inner.update(part);
    }
    let mut outer = Sha256::new();
    outer.update(block.map(|byte| byte ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// The `v1` signature for a body: lowercase hex HMAC-SHA256 over
/// `{timestamp}.{body}`.
pub fn sign_webhook(secret: impl AsRef<[u8]>, timestamp: u64, body: impl AsRef<[u8]>) -> String {
    let prefix = format!("{timestamp}.");
    hmac_sha256(secret.as_ref(), &[prefix.as_bytes(), body.as_ref()])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Constant-time comparison of two equal-length strings.
fn timing_safe_equal(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}

fn valid_timestamp(text: &str) -> bool {
    // /^(0|[1-9][0-9]{0,15})$/
    text == "0"
        || (!text.is_empty()
            && text.len() <= 16
            && text.as_bytes()[0] != b'0'
            && text.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Verify a webhook delivery and return its parsed payload.
///
/// Pass the raw request body exactly as received, not re-serialized JSON.
/// `T` defaults to [`ProtocolEventPayloadV1`]; a generated protocol crate's
/// typed payload works too.
pub fn verify_webhook<T: DeserializeOwned>(
    secret: impl AsRef<[u8]>,
    body: impl AsRef<[u8]>,
    headers: &(impl WebhookHeaders + ?Sized),
    options: VerifyWebhookOptions,
) -> Result<VerifiedWebhook<T>, WebhookVerificationError> {
    let present = |name: &str| {
        headers
            .webhook_header(name)
            .filter(|value| !value.is_empty())
    };
    let missing = |name: &str| {
        WebhookVerificationError::new(WebhookErrorCode::MissingHeader, format!("missing {name}"))
    };
    let id = present(WEBHOOK_ID_HEADER).ok_or_else(|| missing(WEBHOOK_ID_HEADER))?;
    let timestamp_text =
        present(WEBHOOK_TIMESTAMP_HEADER).ok_or_else(|| missing(WEBHOOK_TIMESTAMP_HEADER))?;
    let signature_text =
        present(WEBHOOK_SIGNATURE_HEADER).ok_or_else(|| missing(WEBHOOK_SIGNATURE_HEADER))?;
    if !valid_timestamp(&timestamp_text) {
        return Err(WebhookVerificationError::new(
            WebhookErrorCode::InvalidTimestamp,
            "the timestamp is not Unix seconds",
        ));
    }
    let timestamp: u64 = timestamp_text.parse().map_err(|_| {
        WebhookVerificationError::new(
            WebhookErrorCode::InvalidTimestamp,
            "the timestamp is not Unix seconds",
        )
    })?;
    let now = options.now.unwrap_or_else(now_unix_seconds);
    let tolerance = options
        .tolerance_seconds
        .unwrap_or(WEBHOOK_TOLERANCE_SECONDS);
    if now.abs_diff(timestamp) > tolerance {
        return Err(WebhookVerificationError::new(
            WebhookErrorCode::TimestampOutOfTolerance,
            "the delivery is outside the allowed clock skew",
        ));
    }
    let body = body.as_ref();
    let expected = sign_webhook(secret, timestamp, body);
    let mut matched = false;
    for candidate in signature_text
        .split_whitespace()
        .filter_map(|entry| entry.strip_prefix("v1="))
    {
        matched |= timing_safe_equal(candidate, &expected);
    }
    if !matched {
        return Err(WebhookVerificationError::new(
            WebhookErrorCode::InvalidSignature,
            "no v1 signature matches",
        ));
    }
    let payload = serde_json::from_slice(body).map_err(|_| {
        WebhookVerificationError::new(WebhookErrorCode::InvalidPayload, "the body is not JSON")
    })?;
    Ok(VerifiedWebhook {
        id,
        timestamp,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn vectors() -> Value {
        serde_json::from_str(include_str!(
            "../../../../test-vectors/protocol-client/webhook-signature-v1.json"
        ))
        .expect("webhook vectors are JSON")
    }

    #[test]
    fn matches_the_cross_language_vectors() {
        let vectors = vectors();
        assert_eq!(
            vectors["toleranceSeconds"].as_u64(),
            Some(WEBHOOK_TOLERANCE_SECONDS)
        );
        let cases = vectors["cases"].as_array().unwrap();
        assert!(cases.len() > 10);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let input = &case["input"];
            let expected = &case["expected"];
            if let Some(signature) = expected["signature"].as_str() {
                assert_eq!(
                    sign_webhook(
                        input["secret"].as_str().unwrap(),
                        input["timestamp"].as_u64().unwrap(),
                        input["body"].as_str().unwrap()
                    ),
                    signature,
                    "{name}"
                );
                continue;
            }
            let headers = input["headers"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_string()))
                .collect::<Vec<_>>();
            let verify = || {
                verify_webhook::<Value>(
                    input["secret"].as_str().unwrap(),
                    input["body"].as_str().unwrap(),
                    &headers,
                    VerifyWebhookOptions {
                        now: input["now"].as_u64(),
                        tolerance_seconds: None,
                    },
                )
            };
            match expected["error"].as_str() {
                Some(code) => {
                    assert_eq!(verify().expect_err(name).code.as_str(), code, "{name}")
                }
                None => {
                    let verified = verify().unwrap_or_else(|error| panic!("{name}: {error}"));
                    assert_eq!(verified.id, expected["id"].as_str().unwrap(), "{name}");
                    assert_eq!(
                        Some(verified.timestamp),
                        expected["timestamp"].as_u64(),
                        "{name}"
                    );
                    let body: Value =
                        serde_json::from_str(input["body"].as_str().unwrap()).unwrap();
                    assert_eq!(verified.payload, body, "{name}");
                }
            }
        }
    }

    #[test]
    fn hmac_matches_rfc_4231_for_keys_longer_than_a_block() {
        // RFC 4231 test case 6.
        let key = [0xaa_u8; 131];
        let digest = hmac_sha256(
            &key,
            &[b"Test Using Larger Than Block-Size Key - Hash Key First"],
        );
        let hex = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            hex,
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn reads_reqwest_header_maps_and_long_keys() {
        let secret = vec![9u8; 100];
        let body = br#"{"ok":true}"#;
        let signature = sign_webhook(&secret, 5, body);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Arete-Webhook-Id", "d".parse().unwrap());
        headers.insert("arete-webhook-timestamp", "5".parse().unwrap());
        headers.insert(
            "arete-webhook-signature",
            format!("v1={signature}").parse().unwrap(),
        );
        let verified = verify_webhook::<Value>(
            &secret,
            body,
            &headers,
            VerifyWebhookOptions {
                now: Some(5),
                tolerance_seconds: None,
            },
        )
        .unwrap();
        assert_eq!(verified.payload["ok"], true);
    }
}
