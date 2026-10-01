//! Provider boundary for indexed owner token-account discovery. Credentials stay in the provider.
pub use arete_solana_contracts::{OwnerTokenAccountsPage, OwnerTokenAccountsRequest};

#[derive(Debug, thiserror::Error)]
pub enum TokenDiscoveryError {
    #[error("{0}")]
    InvalidCursor(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Unavailable(String),
}

#[async_trait::async_trait]
pub trait OwnerTokenAccountsProvider: Send + Sync {
    /// Cursors must be opaque and bound to owner, mint and token program. Reject a cursor reused
    /// with different filters. Return actual observation time and an optional real index watermark.
    async fn owner_token_accounts(
        &self,
        request: &OwnerTokenAccountsRequest,
    ) -> Result<OwnerTokenAccountsPage, TokenDiscoveryError>;
}
