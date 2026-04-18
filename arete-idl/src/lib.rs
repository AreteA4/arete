//! IDL parsing and type system for Arete
//!
//! This crate provides types and utilities for parsing and working with
//! Arete IDL (Interface Definition Language) specifications.

pub mod analysis;
pub mod discriminator;
pub mod error;
pub mod parse;
pub mod search;
pub mod snapshot;
pub mod types;
pub mod utils;

pub use discriminator::*;
pub use error::*;
pub use search::*;
pub use snapshot::*;
pub use types::*;
