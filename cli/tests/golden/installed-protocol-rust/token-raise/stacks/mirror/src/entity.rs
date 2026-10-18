use crate::types::{OreRound, OreBoard, OreTreasury, OreMiner};
use arete_sdk::{Stack, StateView, ViewBuilder, ViewHandle, Views};

pub struct OreStreamStack;

impl Stack for OreStreamStack {
    type Views = OreStreamStackViews;
    type Programs = OreStreamStackPrograms;

    fn name() -> &'static str {
        "ore-stream"
    }

    fn url() -> &'static str {
        "wss://ore.stack.example.test"
    }

    fn http_url() -> &'static str {
        "https://ore.stack.example.test"
    }

    fn stack_manifest_hash() -> Option<&'static str> {
        Some("arete:h1:stack-manifest:sha256:338c718dfbb5a260392414ee5e7f2e07ceb01a07fdcccbf748822d43cdb89a09")
    }

    fn live_alias() -> Option<&'static str> {
        Some("live")
    }

    fn gateway() -> Option<arete_sdk::HostedSolanaGatewayBindings> {
        Some(serde_json::from_str("{\"chain\":{\"auth\":{\"acceptedKeyClasses\":[\"anonymous\",\"publishable\",\"secret\"],\"audience\":\"arete:solana-gateway\",\"jwksUrl\":\"https://api.example.test/.well-known/jwks.json\",\"mode\":\"signed_session\",\"required\":true,\"scopes\":[\"read\"],\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"sgb_00000000000000000000000000000001\",\"targetKind\":\"solana-gateway-binding\",\"tokenTransport\":\"bearer\",\"transactionEntitlementRequired\":false},\"authPolicy\":\"signed_session\",\"cluster\":\"mainnet-beta\",\"endpoint\":\"https://solana.example.test/gateway/\",\"region\":\"us-west-1\",\"solanaGatewayBindingId\":\"sgb_00000000000000000000000000000001\"},\"transactions\":{\"auth\":{\"acceptedKeyClasses\":[\"publishable\",\"secret\"],\"audience\":\"arete:solana-gateway\",\"jwksUrl\":\"https://api.example.test/.well-known/jwks.json\",\"mode\":\"signed_session\",\"required\":true,\"scopes\":[\"transaction:inspect\",\"transaction:send\"],\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"sgb_00000000000000000000000000000001\",\"targetKind\":\"solana-gateway-binding\",\"tokenTransport\":\"bearer\",\"transactionEntitlementRequired\":true},\"authPolicy\":\"signed_session\",\"cluster\":\"mainnet-beta\",\"endpoint\":\"https://solana.example.test/gateway/\",\"region\":\"us-west-1\",\"solanaGatewayBindingId\":\"sgb_00000000000000000000000000000001\"}}").expect("generated hosted Solana gateway descriptor must be valid"))
    }
}

pub struct OreStreamStackViews {
    pub ore_round: OreRoundEntityViews,
    pub ore_board: OreBoardEntityViews,
    pub ore_treasury: OreTreasuryEntityViews,
    pub ore_miner: OreMinerEntityViews,
}

impl Views for OreStreamStackViews {
    fn from_builder(builder: ViewBuilder) -> Self {
        Self {
            ore_round: OreRoundEntityViews { builder: builder.clone() },
            ore_board: OreBoardEntityViews { builder: builder.clone() },
            ore_treasury: OreTreasuryEntityViews { builder: builder.clone() },
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

pub struct OreBoardEntityViews {
    builder: ViewBuilder,
}

impl OreBoardEntityViews {
    pub fn state(&self) -> StateView<OreBoard> {
        StateView::new(
            self.builder.connection().clone(),
            self.builder.store().clone(),
            "OreBoard/state".to_string(),
            self.builder.initial_data_timeout(),
        )
    }

    pub fn list(&self) -> ViewHandle<OreBoard> {
        self.builder.view("OreBoard/list")
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
pub struct OreStreamStackPrograms {
    pub ore: crate::programs::ore::OreProgram,
    pub entropy: crate::programs::entropy::EntropyProgram,
}

impl arete_sdk::Programs for OreStreamStackPrograms {
    fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
        Self {
            ore: crate::programs::ore::OreProgram::from_builder(builder.clone()),
            entropy: crate::programs::entropy::EntropyProgram::from_builder(builder),
        }
    }
}