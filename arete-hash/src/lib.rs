//! Versioned, typed, domain-separated artifact identities for Arete.
//!
//! This crate is additive. Existing bare SHA-256 values in other Arete crates
//! retain their legacy definitions until an explicit migration names them.

mod canonical;
mod error;
mod fixture;
mod id;
mod identifier;
mod idl;
mod kind;
mod projection;
mod tree;
mod vectors;

pub use canonical::*;
pub use error::*;
pub use fixture::*;
pub use id::*;
pub use identifier::ProgramReadBindingId;
pub use idl::*;
pub use kind::*;
pub use projection::*;
pub use tree::*;
pub use vectors::*;

#[doc(hidden)]
pub mod internal {
    pub use crate::identifier::{DecoderBindingId, DecoderEngineId};
}

pub const HASH_PROTOCOL_VERSION: u32 = 1;
pub const HASH_PROTOCOL_LABEL: &str = "arete-hash";
pub const HASH_ALGORITHM: &str = "sha256";
