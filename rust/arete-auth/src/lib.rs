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
pub mod revocation;
pub mod token;
pub mod verifier;

pub use audit::{
    auth_failure_event, auth_success_event, rate_limit_event, AuditEvent, AuditSeverity,
    ChannelAuditLogger, NoOpAuditLogger, SecurityAuditEvent, SecurityAuditLogger,
};
pub use claims::{AuthContext, KeyClass, Limits, SessionClaims};
pub use error::{AuthError, AuthErrorCode, RetryPolicy, VerifyError};
pub use keys::{KeyLoader, SigningKey, VerifyingKey};
pub use metrics::{AuthMetrics, AuthMetricsCollector, AuthMetricsSnapshot};
pub use multi_key::{MultiKeyVerifier, MultiKeyVerifierBuilder, RotationKey};
pub use revocation::{RevocationChecker, TokenRevocationList};
pub use token::{TokenError, TokenSigner, TokenVerifier};
pub use verifier::{AsyncVerifier, SimpleVerifier};

/// Default session token TTL in seconds (5 minutes)
pub const DEFAULT_SESSION_TTL_SECONDS: u64 = 300;

/// Refresh window in seconds before expiry (60 seconds)
pub const DEFAULT_REFRESH_WINDOW_SECONDS: u64 = 60;
