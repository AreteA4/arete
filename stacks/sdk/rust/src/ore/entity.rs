use super::types::{OreMiner, OreRound, OreTreasury};
use hyperstack_sdk::{Stack, StateView, ViewBuilder, ViewHandle, Views};

pub struct OreStreamStack;

impl Stack for OreStreamStack {
    type Views = OreStreamStackViews;

    fn name() -> &'static str {
        "ore-stream"
    }

    fn url() -> &'static str {
        "wss://ore.stack.usehyperstack.com"
    }
}

pub struct OreStreamStackViews {
    pub ore_round: OreRoundEntityViews,
    pub ore_treasury: OreTreasuryEntityViews,
    pub ore_miner: OreMinerEntityViews,
}

impl Views for OreStreamStackViews {
    fn from_builder(builder: ViewBuilder) -> Self {
        Self {
            ore_round: OreRoundEntityViews {
                builder: builder.clone(),
            },
            ore_treasury: OreTreasuryEntityViews {
                builder: builder.clone(),
            },
            ore_miner: OreMinerEntityViews { builder },
        }
    }
}

pub struct OreRoundEntityViews {
    builder: ViewBuilder,
}

impl OreRoundEntityViews {
    pub fn state(&self) -> StateView<OreRound> {
        StateView::new(
            self.builder.connection().clone(),
            self.builder.store().clone(),
            "OreRound/state".to_string(),
            self.builder.initial_data_timeout(),
        )
    }

    pub fn list(&self) -> ViewHandle<OreRound> {
        self.builder.view("OreRound/list")
    }

    pub fn latest(&self) -> ViewHandle<OreRound> {
        self.builder.view("OreRound/latest")
    }
}

pub struct OreTreasuryEntityViews {
    builder: ViewBuilder,
}

impl OreTreasuryEntityViews {
    pub fn state(&self) -> StateView<OreTreasury> {
        StateView::new(
            self.builder.connection().clone(),
            self.builder.store().clone(),
            "OreTreasury/state".to_string(),
            self.builder.initial_data_timeout(),
        )
    }

    pub fn list(&self) -> ViewHandle<OreTreasury> {
        self.builder.view("OreTreasury/list")
    }
}

pub struct OreMinerEntityViews {
    builder: ViewBuilder,
}

impl OreMinerEntityViews {
    pub fn state(&self) -> StateView<OreMiner> {
        StateView::new(
            self.builder.connection().clone(),
            self.builder.store().clone(),
            "OreMiner/state".to_string(),
            self.builder.initial_data_timeout(),
        )
    }

    pub fn list(&self) -> ViewHandle<OreMiner> {
        self.builder.view("OreMiner/list")
    }
}
