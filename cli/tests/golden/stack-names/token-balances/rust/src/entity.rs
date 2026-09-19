use crate::types::{Vault};
use arete_sdk::{Stack, StateView, ViewBuilder, ViewHandle, Views};

pub struct TokenBalancesStack;

impl Stack for TokenBalancesStack {
    type Views = TokenBalancesStackViews;
    type Programs = TokenBalancesStackPrograms;

    fn name() -> &'static str {
        "token-balances"
    }

    fn url() -> &'static str {
        "" // TODO: Set URL after first deployment in arete.toml
    }
}

pub struct TokenBalancesStackViews {
    pub vault: VaultEntityViews,
}

impl Views for TokenBalancesStackViews {
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
pub struct TokenBalancesStackPrograms {
    pub vault: crate::programs::vault::VaultProgram,
}

impl arete_sdk::Programs for TokenBalancesStackPrograms {
    fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
        Self {
            vault: crate::programs::vault::VaultProgram::from_builder(builder),
        }
    }
}