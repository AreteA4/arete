mod entity;
mod types;
pub mod programs;

pub use entity::{VaultStreamStack, VaultStreamStackViews, VaultEntityViews, VaultStreamStackPrograms};
pub use types::*;

pub use arete_sdk::{ConnectionState, Arete, Stack, Update, Views};

// Hand-authored devex extensions (staged from extensions.json; not generated).
/// Generated items, as the extension bundle beside this module imports them
/// (`super::generated::…`).
#[allow(unused_imports)]
mod generated {
    pub use super::entity::*;
    pub use super::types::*;
    pub use super::programs;
}
pub mod extensions;
pub use extensions::*;
