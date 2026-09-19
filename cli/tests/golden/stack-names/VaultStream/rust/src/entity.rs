use crate::types::{Vault};
use arete_sdk::{Stack, StateView, ViewBuilder, ViewHandle, Views};

pub struct VaultStreamStack;

impl Stack for VaultStreamStack {
    type Views = VaultStreamStackViews;
    type Programs = VaultStreamStackPrograms;

    fn name() -> &'static str {
        "vault-stream"
    }

    fn url() -> &'static str {
        "" // TODO: Set URL after first deployment in arete.toml
    }
}

pub struct VaultStreamStackViews {
    pub vault: VaultEntityViews,
}

impl Views for VaultStreamStackViews {
    fn from_builder(builder: ViewBuilder) -> Self {
        Self {
            vault: VaultEntityViews { builder },
        }
    }
}

pub struct VaultEntityViews {
    builder: ViewBuilder,
}

impl VaultEntityViews {
    pub fn state(&self) -> StateView<Vault> {
        StateView::new(
            self.builder.connection().clone(),
            self.builder.store().clone(),
            "Vault/state".to_string(),
            self.builder.initial_data_timeout(),
        )
    }

    pub fn list(&self) -> ViewHandle<Vault> {
        self.builder.view("Vault/list")
    }
}
pub struct VaultStreamStackPrograms {
    pub vault: crate::programs::vault::VaultProgram,
}

impl arete_sdk::Programs for VaultStreamStackPrograms {
    fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
        Self {
            vault: crate::programs::vault::VaultProgram::from_builder(builder),
        }
    }
}