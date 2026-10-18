//! Public contracts for Arete Protocols.
//!
//! Every contract is camelCase JSON that rejects unknown fields, floats and
//! `null` for optional fields, is bounded, and is hashed with the Arete JCS
//! profile under its own hash kind. Rust and TypeScript run the shared
//! vectors in `test-vectors/protocol-v1.json` and must agree on every hash
//! and every error code.

mod derive;
mod error;
mod handler;
mod intent;
mod model;
mod receipt;
mod release;
mod shape;
mod spec;
mod stack;

use serde_json::Value;

pub use derive::*;
pub use error::*;
pub use handler::*;
pub use intent::*;
pub use model::*;
pub use receipt::*;
pub use release::*;
pub use spec::*;
pub use stack::*;

use crate::{hash_raw_bytes, HashError, HashId, ProtocolHandlerBundle};

/// Parse contract JSON bytes with the strict hash-profile parser, mapping its
/// failures to protocol error codes.
pub fn parse_contract_json(bytes: &[u8]) -> ProtocolResult<Value> {
    crate::parse_json_bytes_strict(bytes).map_err(|error| {
        let code = match &error {
            HashError::DuplicateJsonKey(_) => "duplicate-json-key",
            HashError::UnsafeJsonInteger(_) => "unsafe-json-integer",
            HashError::NonFiniteNumber => "float-not-allowed",
            _ => "invalid-json",
        };
        ProtocolContractError::new(code, "", error.to_string())
    })
}

/// The identity of a handler bundle: its exact ESM bytes.
pub fn hash_handler_bundle(bytes: &[u8]) -> Result<HashId<ProtocolHandlerBundle>, HashError> {
    hash_raw_bytes::<ProtocolHandlerBundle>(bytes)
}

/// A contract kind that the shared vectors exercise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolContractKind {
    Spec,
    StackSource,
    Release,
    ReleaseReview,
    BuildReceipt,
    Resource,
    ProviderSnapshot,
    ExecutionIntent,
    PreparedOperation,
    ExecutionReceipt,
    ReceiptKeySet,
}

impl ProtocolContractKind {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "protocol-spec" => Self::Spec,
            "protocol-stack-source" => Self::StackSource,
            "protocol-release" => Self::Release,
            "protocol-release-review" => Self::ReleaseReview,
            "protocol-build-receipt" => Self::BuildReceipt,
            "protocol-resource" => Self::Resource,
            "provider-snapshot" => Self::ProviderSnapshot,
            "execution-intent" => Self::ExecutionIntent,
            "prepared-operation" => Self::PreparedOperation,
            "protocol-execution-receipt" => Self::ExecutionReceipt,
            "protocol-receipt-keys" => Self::ReceiptKeySet,
            _ => return None,
        })
    }

    /// Validate a value and return its hash identifier, when the contract
    /// has a hash kind.
    pub fn validate(self, value: &Value) -> ProtocolResult<Option<String>> {
        let hashed = |result: Result<String, HashError>| {
            result
                .map(Some)
                .map_err(|error| ProtocolContractError::new("invalid-value", "", error.to_string()))
        };
        match self {
            Self::Spec => hashed(
                validate_protocol_spec_value(value)?
                    .hash()
                    .map(|h| h.to_string()),
            ),
            Self::StackSource => hashed(
                validate_protocol_stack_source_value(value)?
                    .hash()
                    .map(|h| h.to_string()),
            ),
            Self::Release => hashed(
                validate_protocol_release_value(value)?
                    .hash()
                    .map(|h| h.to_string()),
            ),
            Self::ReleaseReview => hashed(
                validate_protocol_release_review_value(value)?
                    .hash()
                    .map(|h| h.to_string()),
            ),
            Self::BuildReceipt => validate_protocol_build_receipt_value(value).map(|_| None),
            Self::Resource => hashed(
                validate_protocol_resource_value(value)?
                    .hash()
                    .map(|h| h.to_string()),
            ),
            Self::ProviderSnapshot => hashed(
                validate_provider_snapshot_value(value)?
                    .hash()
                    .map(|h| h.to_string()),
            ),
            Self::ExecutionIntent => hashed(
                validate_execution_intent_value(value)?
                    .hash()
                    .map(|h| h.to_string()),
            ),
            Self::PreparedOperation => validate_prepared_operation_value(value).map(|_| None),
            Self::ExecutionReceipt => {
                validate_protocol_execution_receipt_value(value).map(|_| None)
            }
            Self::ReceiptKeySet => validate_protocol_receipt_key_set_value(value).map(|_| None),
        }
    }
}
