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
    /// Origins publishable keys may mint for (comma-separated). Empty means
    /// any origin.
    pub publishable_key_origins: Vec<String>,
    /// Maximum connections per subject
    pub max_connections_per_subject: u32,
    /// Maximum subscriptions per connection
    pub max_subscriptions_per_connection: u32,
    /// Enable rate limiting
    pub enable_rate_limit: bool,
    /// Token mints per minute allowed for one API key from one client IP
    pub rate_limit_per_minute: u32,
    /// Token mints per minute allowed for one API key across all client IPs
    /// (0 disables the per-key cap)
    pub rate_limit_per_key_per_minute: u32,
    /// Take the client IP from `X-Forwarded-For` / `X-Real-IP`. Disable when
    /// the server is reachable without a proxy that sets these headers, or
    /// callers can pick their own IP.
    pub trust_proxy_headers: bool,
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
            default_audience: env::var("DEFAULT_AUDIENCE").unwrap_or_else(|_| "arete".to_string()),
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
            publishable_key_origins: parse_origins(
                &env::var("PUBLISHABLE_KEY_ORIGINS").unwrap_or_default(),
            ),
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
            rate_limit_per_key_per_minute: env::var("RATE_LIMIT_PER_KEY_PER_MINUTE")
                .unwrap_or_else(|_| "600".to_string())
                .parse()
                .unwrap_or(600),
            trust_proxy_headers: env::var("TRUST_PROXY_HEADERS")
                .unwrap_or_else(|_| "true".to_string())
                .parse()
                .unwrap_or(true),
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
            restrict_to_owner(signing_path)?;
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

/// Parse a comma-separated origin list, ignoring blanks and trailing slashes.
fn parse_origins(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|origin| origin.trim().trim_end_matches('/'))
        .filter(|origin| !origin.is_empty())
        .map(str::to_string)
        .collect()
}

/// Tighten an existing signing key file to owner-only (`0600`) if other
/// users can read or write it.
///
/// Keys supplied externally, such as on a read-only secret mount, may not be
/// changeable by the server; it then logs a warning and keeps the key's
/// permissions as they are instead of refusing to start.
fn restrict_to_owner(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            match std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
                Ok(()) => tracing::warn!(
                    path = %path.display(),
                    mode = format!("{mode:o}"),
                    "signing key file was readable by other users; restricted it to 0600"
                ),
                Err(error) => tracing::warn!(
                    path = %path.display(),
                    mode = format!("{mode:o}"),
                    %error,
                    "signing key file is readable by other users and could not be restricted; \
                     restrict it where it is provisioned"
                ),
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
impl Config {
    pub(crate) fn for_tests() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 0,
            issuer: "arete-auth".to_string(),
            default_audience: "arete".to_string(),
            default_ttl_seconds: 300,
            signing_key_path: String::new(),
            verifying_key_path: String::new(),
            secret_keys: Vec::new(),
            publishable_keys: Vec::new(),
            publishable_key_origins: Vec::new(),
            max_connections_per_subject: 10,
            max_subscriptions_per_connection: 100,
            enable_rate_limit: true,
            rate_limit_per_minute: 60,
            rate_limit_per_key_per_minute: 600,
            trust_proxy_headers: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_origins_trims_entries() {
        assert_eq!(
            parse_origins(" https://app.example.com/ ,,https://admin.example.com"),
            vec!["https://app.example.com", "https://admin.example.com"]
        );
        assert!(parse_origins("").is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn existing_world_readable_signing_key_is_tightened_on_load() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("arete-auth-keys-{}", uuid_like()));
        std::fs::create_dir_all(&dir).unwrap();
        let signing = dir.join("signing.key");
        let verifying = dir.join("verifying.key");
        arete_auth::keys::KeyLoader::generate_and_save_keys(&signing, &verifying).unwrap();
        std::fs::set_permissions(&signing, std::fs::Permissions::from_mode(0o644)).unwrap();

        let config = Config {
            signing_key_path: signing.to_string_lossy().into_owned(),
            verifying_key_path: verifying.to_string_lossy().into_owned(),
            ..Config::for_tests()
        };
        config.generate_keys_if_missing().unwrap();

        let mode = std::fs::metadata(&signing).unwrap().permissions().mode() & 0o777;
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(mode, 0o600);
    }

    fn uuid_like() -> String {
        use rand::Rng;
        format!("{:016x}", rand::thread_rng().gen::<u64>())
    }
}
