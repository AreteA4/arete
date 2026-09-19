use crate::types::{Vault};
use arete_sdk::{Stack, StateView, ViewBuilder, ViewHandle, Views};

pub struct A9livesStack;

impl Stack for A9livesStack {
    type Views = A9livesStackViews;
    type Programs = A9livesStackPrograms;

    fn name() -> &'static str {
        "9lives"
    }

    fn url() -> &'static str {
        "" // TODO: Set URL after first deployment in arete.toml
    }
}

pub struct A9livesStackViews {
    pub vault: VaultEntityViews,
}

impl Views for A9livesStackViews {
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
pub struct A9livesStackPrograms {
    pub vault: crate::programs::vault::VaultProgram,
}

impl arete_sdk::Programs for A9livesStackPrograms {
    fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
        Self {
            vault: crate::programs::vault::VaultProgram::from_builder(builder),
        }
    }
}