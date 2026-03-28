//! Hyperstack Authentication Library
//!
//! This crate provides authentication and authorization utilities for Hyperstack,
//! including JWT token handling, claims validation, and key management.

pub mod claims;
pub mod error;
pub mod keys;
pub mod token;
pub mod verifier;

pub use claims::{AuthContext, KeyClass, Limits, SessionClaims};
pub use error::{AuthError, VerifyError};
pub use keys::{KeyLoader, SigningKey, VerifyingKey};
pub use token::{TokenSigner, TokenVerifier};

/// Default session token TTL in seconds (5 minutes)
pub const DEFAULT_SESSION_TTL_SECONDS: u64 = 300;

/// Refresh window in seconds before expiry (60 seconds)
pub const DEFAULT_REFRESH_WINDOW_SECONDS: u64 = 60;
