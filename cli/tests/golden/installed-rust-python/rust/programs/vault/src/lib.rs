mod types;
pub mod programs;

pub use programs::VaultPrograms;
pub use types::*;

pub use arete_sdk::{ProgramSdk, Programs};

// Hand-authored devex extensions (staged from extensions.json; not generated).
/// Generated items, as the extension bundle beside this module imports them
/// (`super::generated::…`).
#[allow(unused_imports)]
mod generated {
    pub use super::programs::vault::*;
    pub use super::types::*;
    pub use super::types::Vault;
}
pub mod vault_math;
pub mod extensions;
pub use extensions::*;
