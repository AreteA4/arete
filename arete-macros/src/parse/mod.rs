//! Parsing module for arete-macros.
//!
//! This module contains all parsing logic including:
//! - `attributes` - Parsing of #[map], #[event], #[snapshot], etc. macro attributes
//! - `idl` - Parsing of Anchor IDL JSON files
//! - `proto` - Parsing of Protocol Buffer (.proto) files
//! - `conditions` - Parsing of condition expressions
//! - `pdas` - Parsing of pdas! macro blocks

pub mod attributes;
pub mod conditions;
pub mod idl;
pub mod pda_validation;
pub mod pdas;
pub mod proto;

pub use attributes::*;
