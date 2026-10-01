//! Vault stack extension (Rust).

pub mod defaults {
    pub struct VaultLimits {
        pub max_deposit: u64,
    }

    pub fn limits() -> VaultLimits {
        VaultLimits {
            max_deposit: 1_000_000,
        }
    }
}

pub mod read {
    use arete_sdk::Arete;

    use super::super::generated::{Vault, VaultStreamStack};

    /// The streamed vault under `key`, once its view has data.
    pub async fn vault(a4: &Arete<VaultStreamStack>, key: &str) -> Option<Vault> {
        a4.views.vault.state().get(key).await
    }

    /// The vault program's account reader, as the stack binds it.
    pub fn program_id(a4: &Arete<VaultStreamStack>) -> &'static str {
        let _ = &a4.programs.vault;
        super::super::generated::programs::vault::PROGRAM_ID
    }
}
