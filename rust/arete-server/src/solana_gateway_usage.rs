use arete_auth::AuthContext;
use async_trait::async_trait;

/// The Solana Gateway surface that completed an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolanaGatewayUsageSurface {
    ChainRead,
    Transaction,
}

/// A completed Solana Gateway operation.
///
/// This is deliberately a transport-neutral observation. Hosted deployments
/// can turn it into their own accounting events without putting billing
/// identities, wire formats, retry policy, or persistence into `arete-server`.
#[derive(Clone, Debug)]
pub struct SolanaGatewayUsageObservation {
    pub target_id: String,
    pub auth_context: Option<AuthContext>,
    pub surface: SolanaGatewayUsageSurface,
    pub operation: &'static str,
    pub result: &'static str,
    pub address_count: u64,
    pub response_bytes: u64,
}

/// Observes completed Solana Gateway operations.
///
/// The server awaits this callback before returning the response. Implementors
/// should accept the observation quickly, then perform network delivery and
/// retry work outside the request path. An implementation that needs durable
/// accounting is responsible for making the observation durable before this
/// method returns.
#[async_trait]
pub trait SolanaGatewayUsageObserver: Send + Sync {
    async fn observe(&self, observation: SolanaGatewayUsageObservation);
}
