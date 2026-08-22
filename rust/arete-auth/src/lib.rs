//! Arete Authentication Library
//!
//! This crate provides authentication and authorization utilities for Arete,
//! including JWT token handling, claims validation, and key management.

pub mod audit;
pub mod claims;
pub mod error;
pub mod keys;
pub mod metrics;
pub mod multi_key;
pub mod program_read;
pub mod revocation;
pub mod solana_gateway;
pub mod token;
pub mod verifier;

pub use audit::{
    auth_failure_event, auth_success_event, rate_limit_event, AuditEvent, AuditSeverity,
    ChannelAuditLogger, NoOpAuditLogger, SecurityAuditEvent, SecurityAuditLogger,
};
pub use claims::{
    AuthContext, KeyClass, Limits, PolicyClaimsError, SessionClaims, TargetKind,
    MAX_POLICY_IDENTITY_BYTES, PLAN_ANONYMOUS,
};
pub use error::{AuthError, AuthErrorCode, RetryPolicy, VerifyError};
pub use keys::{KeyLoader, SigningKey, VerifyingKey};
pub use metrics::{AuthMetrics, AuthMetricsCollector, AuthMetricsSnapshot};
pub use multi_key::{MultiKeyVerifier, MultiKeyVerifierBuilder, RotationKey};
pub use program_read::{ProgramReadAuthorization, ProgramReadAuthorizationError};
pub use revocation::{RevocationChecker, TokenRevocationList};
pub use solana_gateway::{
    SolanaGatewayAuthorization, SolanaGatewayAuthorizationError, SolanaGatewayScope,
};
pub use token::{TokenError, TokenSigner, TokenVerifier};
pub use verifier::{AsyncVerifier, SimpleVerifier};

/// Stable JWT audience for the shared program-read service.
pub const PROGRAM_READ_AUDIENCE: &str = "arete:program-read";

/// Stable JWT audience shared by all regional Solana gateways.
pub const SOLANA_GATEWAY_AUDIENCE: &str = "arete:solana-gateway";

/// Exact scope for chain reads.
pub const SCOPE_READ: &str = "read";

/// Exact scope for transaction inspection operations.
pub const SCOPE_TRANSACTION_INSPECT: &str = "transaction:inspect";

/// Exact scope for transaction submission.
pub const SCOPE_TRANSACTION_SEND: &str = "transaction:send";

/// Default session token TTL in seconds (5 minutes)
pub const DEFAULT_SESSION_TTL_SECONDS: u64 = 300;

/// Refresh window in seconds before expiry (60 seconds)
pub const DEFAULT_REFRESH_WINDOW_SECONDS: u64 = 60;
