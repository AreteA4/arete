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
        "wss://vault.stack.example.test"
    }

    fn http_url() -> &'static str {
        "https://vault.stack.example.test"
    }

    fn stack_manifest_hash() -> Option<&'static str> {
        Some("arete:h1:stack-manifest:sha256:53809b392576092808edfbf619ec0b9b4174373a84c2dac843af4a71537fbb72")
    }

    fn live_alias() -> Option<&'static str> {
        Some("live")
    }

    fn gateway() -> Option<arete_sdk::HostedSolanaGatewayBindings> {
        Some(serde_json::from_str("{\"chain\":{\"auth\":{\"acceptedKeyClasses\":[\"publishable\",\"secret\"],\"audience\":\"arete:solana-gateway\",\"jwksUrl\":\"https://api.example.test/.well-known/jwks.json\",\"mode\":\"signed_session\",\"required\":true,\"scopes\":[\"read\"],\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"sgb_00000000000000000000000000000001\",\"targetKind\":\"solana-gateway-binding\",\"tokenTransport\":\"bearer\",\"transactionEntitlementRequired\":false},\"authPolicy\":\"signed_session\",\"cluster\":\"mainnet-beta\",\"endpoint\":\"https://solana.example.test/gateway/\",\"region\":\"us-west-1\",\"solanaGatewayBindingId\":\"sgb_00000000000000000000000000000001\"},\"transactions\":{\"auth\":{\"acceptedKeyClasses\":[\"publishable\",\"secret\"],\"audience\":\"arete:solana-gateway\",\"jwksUrl\":\"https://api.example.test/.well-known/jwks.json\",\"mode\":\"signed_session\",\"required\":true,\"scopes\":[\"transaction:inspect\",\"transaction:send\"],\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"sgb_00000000000000000000000000000001\",\"targetKind\":\"solana-gateway-binding\",\"tokenTransport\":\"bearer\",\"transactionEntitlementRequired\":true},\"authPolicy\":\"signed_session\",\"cluster\":\"mainnet-beta\",\"endpoint\":\"https://solana.example.test/gateway/\",\"region\":\"us-west-1\",\"solanaGatewayBindingId\":\"sgb_00000000000000000000000000000001\"}}").expect("generated hosted Solana gateway descriptor must be valid"))
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