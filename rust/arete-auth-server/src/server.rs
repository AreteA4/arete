use crate::config::Config;
use crate::error::AuthServerError;
use crate::keys::ApiKeyStore;
use crate::rate_limiter::MintRateLimiter;
use arete_auth::{TokenSigner, VerifyingKey};
use std::time::Duration;

pub struct AppState {
    pub config: Config,
    pub token_signer: TokenSigner,
    pub verifying_key: VerifyingKey,
    pub key_store: ApiKeyStore,
    pub rate_limiter: Option<MintRateLimiter>,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self, AuthServerError> {
        // Generate or load keys
        let (signing_key, verifying_key) = config.generate_keys_if_missing()?;

        // Create token signer
        let token_signer = TokenSigner::new(signing_key, config.issuer.clone());

        // Create key store
        let key_store =
            ApiKeyStore::new(config.secret_keys.clone(), config.publishable_keys.clone());
        let rate_limiter = if config.enable_rate_limit {
            Some(MintRateLimiter::new(Duration::from_secs(60)))
        } else {
            None
        };

        Ok(Self {
            config,
            token_signer,
            verifying_key,
            key_store,
            rate_limiter,
        })
    }
}
