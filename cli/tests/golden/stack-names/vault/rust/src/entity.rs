use crate::types::{Vault};
use arete_sdk::{Stack, StateView, ViewBuilder, ViewHandle, Views};

pub struct vaultStack;

impl Stack for vaultStack {
    type Views = vaultStackViews;
    type Programs = vaultStackPrograms;

    fn name() -> &'static str {
        "vault"
    }

    fn url() -> &'static str {
        "" // TODO: Set URL after first deployment in arete.toml
    }
}

pub struct vaultStackViews {
    pub vault: VaultEntityViews,
}

impl Views for vaultStackViews {
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
pub struct vaultStackPrograms {
    pub vault: crate::programs::vault::VaultProgram,
}

impl arete_sdk::Programs for vaultStackPrograms {
    fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
        Self {
            vault: crate::programs::vault::VaultProgram::from_builder(builder),
        }
    }
}