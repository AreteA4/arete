use std::fmt;

/// A contract validation failure with a stable, language-neutral code.
///
/// `path` is a JSON Pointer (RFC 6901) into the document that failed. Rust
/// and TypeScript report the same `code` and `path` for every vector in
/// `test-vectors/protocol-v1.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolContractError {
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

impl ProtocolContractError {
    pub fn new(code: &'static str, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ProtocolContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = if self.path.is_empty() {
            "/"
        } else {
            &self.path
        };
        write!(formatter, "{} at {}: {}", self.code, path, self.message)
    }
}

impl std::error::Error for ProtocolContractError {}

pub type ProtocolResult<T> = Result<T, ProtocolContractError>;

/// Every stable error code a protocol contract validator can return.
pub const PROTOCOL_ERROR_CODES: &[&str] = &[
    // JSON well-formedness (shared with the hash profile).
    "invalid-json",
    "duplicate-json-key",
    "unsafe-json-integer",
    "float-not-allowed",
    // Shape.
    "invalid-type",
    "unknown-field",
    "missing-field",
    "null-not-allowed",
    "invalid-schema-tag",
    "invalid-value",
    "invalid-identifier",
    "invalid-decimal",
    "invalid-pubkey",
    "invalid-hash-id",
    "invalid-semver",
    "invalid-expression",
    "invalid-signature",
    "string-too-long",
    "array-too-large",
    "map-too-large",
    "integer-out-of-range",
    "duplicate-entry",
    "unsorted-entries",
    "invalid-schema",
    // References.
    "unknown-program",
    "unknown-read",
    "unknown-state",
    "unknown-step",
    "unknown-event",
    "unknown-role",
    "unknown-ephemeral",
    "unknown-bound-account",
    "unknown-arg",
    // Machines.
    "unreachable-state",
    "dead-end-state",
    "terminal-state-exit",
    "derived-state-without-rule",
    "derive-targets-recorded-state",
    "transition-targets-derived-state",
    "missing-participant-machine",
    // Steps.
    "duplicate-guard-code",
    "reserved-guard-code",
    "create-step-count",
    "invalid-authority",
    "missing-access",
    "forbidden-access",
    "missing-role-guard",
    "role-not-bound",
    "bind-outside-create",
    "create-step-transition",
    "invalid-fee-basis",
    "handler-abi-required",
    "handler-abi-unused",
    "remaining-account-signer",
    // Reactions and events.
    "reaction-action-mismatch",
    "event-not-emitted",
    "event-machine-mismatch",
];
