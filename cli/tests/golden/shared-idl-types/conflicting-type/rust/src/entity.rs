use crate::types::{AlphaVault, BetaVault};
use arete_sdk::{Stack, StateView, ViewBuilder, ViewHandle, Views};

pub struct HeaderStreamStack;

impl Stack for HeaderStreamStack {
    type Views = HeaderStreamStackViews;
    type Programs = HeaderStreamStackPrograms;

    fn name() -> &'static str {
        "header-stream"
    }

    fn url() -> &'static str {
        "" // TODO: Set URL after first deployment in arete.toml
    }
}

pub struct HeaderStreamStackViews {
    pub alpha_vault: AlphaVaultEntityViews,
    pub beta_vault: BetaVaultEntityViews,
}

impl Views for HeaderStreamStackViews {
    fn from_builder(builder: ViewBuilder) -> Self {
        Self {
            alpha_vault: AlphaVaultEntityViews { builder: builder.clone() },
            beta_vault: BetaVaultEntityViews { builder },
        }
    }
}

pub struct AlphaVaultEntityViews {
    builder: ViewBuilder,
}

impl AlphaVaultEntityViews {
    pub fn state(&self) -> StateView<AlphaVault> {
        StateView::new(
            self.builder.connection().clone(),
            self.builder.store().clone(),
            "AlphaVault/state".to_string(),
            self.builder.initial_data_timeout(),
        )
    }

    pub fn list(&self) -> ViewHandle<AlphaVault> {
        self.builder.view("AlphaVault/list")
    }
}

pub struct BetaVaultEntityViews {
    builder: ViewBuilder,
}

impl BetaVaultEntityViews {
    pub fn state(&self) -> StateView<BetaVault> {
        StateView::new(
            self.builder.connection().clone(),
            self.builder.store().clone(),
            "BetaVault/state".to_string(),
            self.builder.initial_data_timeout(),
        )
    }

    pub fn list(&self) -> ViewHandle<BetaVault> {
        self.builder.view("BetaVault/list")
    }
}
pub struct HeaderStreamStackPrograms {
    pub alpha: crate::programs::alpha::AlphaProgram,
    pub beta: crate::programs::beta::BetaProgram,
}

impl arete_sdk::Programs for HeaderStreamStackPrograms {
    fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
        Self {
            alpha: crate::programs::alpha::AlphaProgram::from_builder(builder.clone()),
            beta: crate::programs::beta::BetaProgram::from_builder(builder),
        }
    }
}