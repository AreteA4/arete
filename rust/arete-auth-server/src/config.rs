use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    /// Server host address
    pub host: String,
    /// Server port
    pub port: u16,
    /// Issuer name for tokens
    pub issuer: String,
    /// Default audience for tokens
    pub default_audience: String,
    /// Default token TTL in seconds
    pub default_ttl_seconds: u64,
    /// Path to signing key file (base64-encoded Ed25519 key)
    pub signing_key_path: String,
    /// Path to verifying key file (base64-encoded Ed25519 public key)
    pub verifying_key_path: String,
    /// Secret API keys (comma-separated for simple mode)
    pub secret_keys: Vec<String>,
    /// Publishable API keys (comma-separated for simple mode)
    pub publishable_keys: Vec<String>,
    /// Maximum connections per subject
    pub max_connections_per_subject: u32,
    /// Maximum subscriptions per connection
    pub max_subscriptions_per_connection: u32,
    /// Enable rate limiting
    pub enable_rate_limit: bool,
    /// Rate limit per minute for token minting
    pub rate_limit_per_minute: u32,
}

impl Config {
    /// Load configuration from environment variables
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();

        Ok(Self {
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()
                .unwrap_or(8080),
            issuer: env::var("ISSUER").unwrap_or_else(|_| "arete-auth".to_string()),
            default_audience: env::var("DEFAULT_AUDIENCE")
                .unwrap_or_else(|_| "arete".to_string()),
            default_ttl_seconds: env::var("DEFAULT_TTL_SECONDS")
                .unwrap_or_else(|_| "300".to_string())
                .parse()
                .unwrap_or(300),
            signing_key_path: env::var("SIGNING_KEY_PATH")
                .unwrap_or_else(|_| "/etc/arete/auth/signing.key".to_string()),
            verifying_key_path: env::var("VERIFYING_KEY_PATH")
                .unwrap_or_else(|_| "/etc/arete/auth/verifying.key".to_string()),
            secret_keys: env::var("SECRET_KEYS")
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect(),
            publishable_keys: env::var("PUBLISHABLE_KEYS")
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect(),
            max_connections_per_subject: env::var("MAX_CONNECTIONS_PER_SUBJECT")
                .unwrap_or_else(|_| "10".to_string())
                .parse()
                .unwrap_or(10),
            max_subscriptions_per_connection: env::var("MAX_SUBSCRIPTIONS_PER_CONNECTION")
                .unwrap_or_else(|_| "100".to_string())
                .parse()
                .unwrap_or(100),
            enable_rate_limit: env::var("ENABLE_RATE_LIMIT")
                .unwrap_or_else(|_| "false".to_string())
                .parse()
                .unwrap_or(false),
            rate_limit_per_minute: env::var("RATE_LIMIT_PER_MINUTE")
                .unwrap_or_else(|_| "60".to_string())
                .parse()
                .unwrap_or(60),
        })
    }

    /// Generate new keys if they don't exist
    pub fn generate_keys_if_missing(
        &self,
    ) -> anyhow::Result<(arete_auth::SigningKey, arete_auth::VerifyingKey)> {
        use arete_auth::keys::KeyLoader;
        use std::path::Path;

        let signing_path = Path::new(&self.signing_key_path);
        let verifying_path = Path::new(&self.verifying_key_path);

        if signing_path.exists() && verifying_path.exists() {
            // Load existing keys
            let signing_key = KeyLoader::signing_key_from_file(signing_path)?;
            let verifying_key = KeyLoader::verifying_key_from_file(verifying_path)?;
            return Ok((signing_key, verifying_key));
        }

        // Generate new keys
        tracing::info!("Generating new signing and verifying keys...");
        std::fs::create_dir_all(signing_path.parent().unwrap_or(Path::new(".")))?;
        std::fs::create_dir_all(verifying_path.parent().unwrap_or(Path::new(".")))?;

        let (signing_key, verifying_key) =
            KeyLoader::generate_and_save_keys(signing_path, verifying_path)?;

        tracing::info!("Keys generated and saved successfully");
        Ok((signing_key, verifying_key))
    }
}
