use crate::error::{AreteError, AuthErrorCode};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use url::Url;

pub const TOKEN_REFRESH_BUFFER_SECONDS: u64 = 60;
pub const MIN_REFRESH_DELAY_SECONDS: u64 = 1;
pub const DEFAULT_QUERY_PARAMETER: &str = "hs_token";
pub const DEFAULT_HOSTED_TOKEN_ENDPOINT: &str = "https://api.arete.run/ws/sessions";
pub const HOSTED_WEBSOCKET_SUFFIX: &str = ".stack.arete.run";

/// Environment variable that supplies [`AuthConfig::with_secret_key`] when no
/// auth option is set.
pub const ARETE_API_KEY_ENV: &str = "ARETE_API_KEY";

const PUBLISHABLE_KEY_PREFIXES: [&str; 2] = ["a4_pk_", "hspk_"];
const SECRET_KEY_PREFIXES: [&str; 3] = ["a4_sk_", "a4_ak_", "hsk_"];
const CREATE_PUBLISHABLE_HINT: &str =
    "create one with `a4 auth keys create-publishable --origin <scheme://host[:port]>`";

/// Key class inferred from an API key's prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeyClass {
    /// Origin-bound publishable key (`a4_pk_...`) for code shipped to a browser.
    Publishable,
    /// Secret key (`a4_sk_...`) or agent key (`a4_ak_...`); server-side only.
    Secret,
    /// A key this SDK does not recognise; passed through unvalidated.
    Unknown,
}

/// Classify an API key by its prefix.
pub fn classify_api_key(key: &str) -> ApiKeyClass {
    let key = key.trim();
    if PUBLISHABLE_KEY_PREFIXES
        .iter()
        .any(|prefix| key.starts_with(prefix))
    {
        ApiKeyClass::Publishable
    } else if SECRET_KEY_PREFIXES
        .iter()
        .any(|prefix| key.starts_with(prefix))
    {
        ApiKeyClass::Secret
    } else {
        ApiKeyClass::Unknown
    }
}

/// Environment variable naming extra hosted suffixes, comma separated.
///
/// A deployment can be served on a hostname outside the default suffix - the
/// live service's test domain is the first - and a client that does not know
/// the suffix will not mint a session for it. It then connects unauthenticated
/// and the host answers 401, which reads as a credential problem rather than
/// an unrecognised hostname.
pub const HOSTED_WEBSOCKET_SUFFIXES_ENV: &str = "ARETE_HOSTED_WEBSOCKET_SUFFIXES";

/// Every suffix treated as hosted Arete: the default plus anything in
/// `ARETE_HOSTED_WEBSOCKET_SUFFIXES`. Entries are normalised to lowercase and
/// to a leading dot, so `cell.arete.run` and `.cell.arete.run` both work.
pub fn hosted_websocket_suffixes() -> Vec<String> {
    let mut suffixes = vec![HOSTED_WEBSOCKET_SUFFIX.to_string()];
    let Ok(configured) = std::env::var(HOSTED_WEBSOCKET_SUFFIXES_ENV) else {
        return suffixes;
    };
    for entry in configured.split(',') {
        let entry = entry.trim().trim_end_matches('.').to_ascii_lowercase();
        if entry.is_empty() {
            continue;
        }
        let entry = if entry.starts_with('.') {
            entry
        } else {
            format!(".{entry}")
        };
        if !suffixes.contains(&entry) {
            suffixes.push(entry);
        }
    }
    suffixes
}

/// True when `host` is served by hosted Arete under any known suffix.
pub fn is_hosted_websocket_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    hosted_websocket_suffixes()
        .iter()
        .any(|suffix| host.ends_with(suffix.as_str()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthToken {
    pub token: String,
    pub expires_at: Option<u64>,
}

impl AuthToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            expires_at: None,
        }
    }

    pub fn with_expiry(mut self, expires_at: u64) -> Self {
        self.expires_at = Some(expires_at);
        self
    }
}

impl From<String> for AuthToken {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for AuthToken {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

pub type TokenProviderFuture = Pin<Box<dyn Future<Output = Result<AuthToken, AreteError>> + Send>>;
pub type TokenProvider = dyn Fn() -> TokenProviderFuture + Send + Sync;

/// The served version a generated stack names in its WebSocket session
/// request: one live alias of one StackManifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackRelease {
    /// `arete:h1:stack-manifest:sha256:<64 hex>`.
    pub stack_manifest_hash: String,
    /// StackManifest live alias the stack serves.
    pub live_alias: String,
}

impl StackRelease {
    /// The release a [`crate::Stack`] was generated for, when it names both
    /// halves.
    pub fn of<S: crate::Stack>() -> Option<Self> {
        match (S::stack_manifest_hash(), S::live_alias()) {
            (Some(hash), Some(alias)) if !hash.is_empty() && !alias.is_empty() => Some(Self {
                stack_manifest_hash: hash.to_string(),
                live_alias: alias.to_string(),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TokenTransport {
    #[default]
    QueryParameter,
    Bearer,
}

#[derive(Clone, Default)]
pub struct AuthConfig {
    pub(crate) token: Option<String>,
    pub(crate) get_token: Option<Arc<TokenProvider>>,
    pub(crate) token_endpoint: Option<String>,
    pub(crate) publishable_key: Option<String>,
    pub(crate) secret_key: Option<String>,
    pub(crate) token_endpoint_headers: HashMap<String, String>,
    pub(crate) token_transport: TokenTransport,
    /// Served stack version named in untargeted session requests. Set by the
    /// client from its generated [`crate::Stack`], never by callers.
    pub(crate) stack_release: Option<StackRelease>,
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field("has_token", &self.token.is_some())
            .field("has_get_token", &self.get_token.is_some())
            .field("token_endpoint", &self.token_endpoint)
            .field(
                "publishable_key",
                &self.publishable_key.as_ref().map(|_| "***"),
            )
            .field("secret_key", &self.secret_key.as_ref().map(|_| "***"))
            .field(
                "token_endpoint_headers",
                &self.token_endpoint_headers.keys().collect::<Vec<_>>(),
            )
            .field("token_transport", &self.token_transport)
            .field("stack_release", &self.stack_release)
            .finish()
    }
}

impl AuthConfig {
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Publishable key (`a4_pk_...`) for code shipped to a browser. It is
    /// bound to the origins it was created for; create one with
    /// `a4 auth keys create-publishable --origin <scheme://host[:port]>`.
    pub fn with_publishable_key(mut self, publishable_key: impl Into<String>) -> Self {
        self.publishable_key = Some(publishable_key.into());
        self
    }

    /// Agent key (`a4_ak_...`) or secret key (`a4_sk_...`) for servers,
    /// agents and local scripts. Sent to the token endpoint as the bearer
    /// credential; no `Origin` header is needed. When no auth option is set,
    /// [`ARETE_API_KEY_ENV`] supplies it at connect time.
    pub fn with_secret_key(mut self, secret_key: impl Into<String>) -> Self {
        self.secret_key = Some(secret_key.into());
        self
    }

    /// Set the key field matching an API key's prefix: publishable keys go
    /// to [`Self::with_publishable_key`], everything else to
    /// [`Self::with_secret_key`].
    pub fn with_api_key(self, api_key: impl Into<String>) -> Self {
        let api_key = api_key.into();
        match classify_api_key(&api_key) {
            ApiKeyClass::Publishable => self.with_publishable_key(api_key),
            ApiKeyClass::Secret | ApiKeyClass::Unknown => self.with_secret_key(api_key),
        }
    }

    /// The key sent as the token endpoint bearer credential, if any.
    pub(crate) fn api_key(&self) -> Option<&str> {
        self.secret_key
            .as_deref()
            .or(self.publishable_key.as_deref())
    }

    fn has_explicit_auth(&self) -> bool {
        self.token.is_some()
            || self.get_token.is_some()
            || self.token_endpoint.is_some()
            || self.publishable_key.is_some()
            || self.secret_key.is_some()
    }

    pub fn with_token_endpoint(mut self, token_endpoint: impl Into<String>) -> Self {
        self.token_endpoint = Some(token_endpoint.into());
        self
    }

    pub fn with_token_endpoint_header(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.token_endpoint_headers.insert(key.into(), value.into());
        self
    }

    pub fn with_token_transport(mut self, transport: TokenTransport) -> Self {
        self.token_transport = transport;
        self
    }

    pub fn with_token_provider<F, Fut>(mut self, provider: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<AuthToken, AreteError>> + Send + 'static,
    {
        self.get_token = Some(Arc::new(move || Box::pin(provider())));
        self
    }

    pub(crate) fn with_stack_release(mut self, release: Option<StackRelease>) -> Self {
        self.stack_release = release;
        self
    }

    pub(crate) fn resolve_strategy(&self, websocket_url: &str) -> ResolvedAuthStrategy {
        if let Some(token) = self.token.clone() {
            return ResolvedAuthStrategy::StaticToken(token);
        }

        if let Some(get_token) = self.get_token.clone() {
            return ResolvedAuthStrategy::TokenProvider(get_token);
        }

        if let Some(token_endpoint) = self.token_endpoint.clone() {
            return ResolvedAuthStrategy::TokenEndpoint(token_endpoint);
        }

        if self.api_key().is_some() && is_hosted_arete_websocket_url(websocket_url) {
            return ResolvedAuthStrategy::TokenEndpoint(DEFAULT_HOSTED_TOKEN_ENDPOINT.to_string());
        }

        ResolvedAuthStrategy::None
    }

    pub(crate) fn has_refreshable_auth(&self, websocket_url: &str) -> bool {
        matches!(
            self.resolve_strategy(websocket_url),
            ResolvedAuthStrategy::TokenProvider(_) | ResolvedAuthStrategy::TokenEndpoint(_)
        )
    }
}

#[derive(Clone)]
pub(crate) enum ResolvedAuthStrategy {
    None,
    StaticToken(String),
    TokenProvider(Arc<TokenProvider>),
    TokenEndpoint(String),
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenEndpointResponse {
    pub token: String,
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default, rename = "expiresAt")]
    pub expires_at_camel: Option<u64>,
}

impl TokenEndpointResponse {
    pub fn into_auth_token(self) -> AuthToken {
        AuthToken {
            token: self.token,
            expires_at: self.expires_at.or(self.expires_at_camel),
        }
    }
}

/// Untargeted WebSocket session request. Without a stack release it
/// serializes exactly as older clients send it: `{"websocket_url": ...}`.
#[derive(Debug, Serialize)]
pub(crate) struct TokenEndpointRequest<'a> {
    pub websocket_url: &'a str,
    #[serde(rename = "stackManifestHash", skip_serializing_if = "Option::is_none")]
    pub stack_manifest_hash: Option<&'a str>,
    #[serde(rename = "liveAlias", skip_serializing_if = "Option::is_none")]
    pub live_alias: Option<&'a str>,
}

impl<'a> TokenEndpointRequest<'a> {
    pub fn new(websocket_url: &'a str, release: Option<&'a StackRelease>) -> Self {
        Self {
            websocket_url,
            stack_manifest_hash: release.map(|release| release.stack_manifest_hash.as_str()),
            live_alias: release.map(|release| release.live_alias.as_str()),
        }
    }
}

pub(crate) fn parse_jwt_expiry(token: &str) -> Option<u64> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let _signature = parts.next()?;

    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.as_bytes())
        .ok()?;
    let payload: JwtPayload = serde_json::from_slice(&decoded).ok()?;
    payload.exp
}

pub(crate) fn token_is_expiring(expires_at: Option<u64>, now_epoch_seconds: u64) -> bool {
    match expires_at {
        Some(exp) => now_epoch_seconds >= exp.saturating_sub(TOKEN_REFRESH_BUFFER_SECONDS),
        None => false,
    }
}

pub(crate) fn token_refresh_delay(expires_at: Option<u64>, now_epoch_seconds: u64) -> Option<u64> {
    let expires_at = expires_at?;
    let refresh_at = expires_at.saturating_sub(TOKEN_REFRESH_BUFFER_SECONDS);
    Some(
        refresh_at
            .saturating_sub(now_epoch_seconds)
            .max(MIN_REFRESH_DELAY_SECONDS),
    )
}

pub(crate) fn is_hosted_arete_websocket_url(websocket_url: &str) -> bool {
    Url::parse(websocket_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| is_hosted_websocket_host(&host))
}

pub(crate) fn build_websocket_url(
    websocket_url: &str,
    token: Option<&str>,
    transport: TokenTransport,
) -> Result<String, AreteError> {
    if transport == TokenTransport::Bearer || token.is_none() {
        return Ok(websocket_url.to_string());
    }

    let mut url = Url::parse(websocket_url)
        .map_err(|error| AreteError::ConnectionFailed(error.to_string()))?;
    url.query_pairs_mut()
        .append_pair(DEFAULT_QUERY_PARAMETER, token.expect("checked is_some"));
    Ok(url.to_string())
}

/// Validate the configured API keys and apply the [`ARETE_API_KEY_ENV`]
/// fallback. Error and warning text never includes key material.
pub(crate) fn resolve_auth_config(
    auth: Option<AuthConfig>,
) -> Result<Option<AuthConfig>, AreteError> {
    resolve_auth_config_with_env(auth, |name| std::env::var(name).ok())
}

fn warn_once(flag: &std::sync::atomic::AtomicBool, message: &str) {
    if !flag.swap(true, std::sync::atomic::Ordering::Relaxed) {
        tracing::warn!("{message}");
    }
}

static WARNED_SECRET_IN_PUBLISHABLE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static WARNED_PUBLISHABLE_IN_ENV: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(crate) fn resolve_auth_config_with_env(
    auth: Option<AuthConfig>,
    read_env: impl Fn(&str) -> Option<String>,
) -> Result<Option<AuthConfig>, AreteError> {
    if let Some(config) = auth.as_ref() {
        if let Some(secret_key) = config.secret_key.as_deref() {
            if secret_key.trim().is_empty() {
                return Err(AreteError::InvalidConfig("auth.secret_key is empty".into()));
            }
            if classify_api_key(secret_key) == ApiKeyClass::Publishable {
                return Err(AreteError::InvalidConfig(
                    "auth.secret_key was given a publishable key (a4_pk_...). Pass it as \
                     auth.publishable_key instead; auth.secret_key takes an agent key \
                     (a4_ak_...) or secret key (a4_sk_...)."
                        .into(),
                ));
            }
            if config.publishable_key.is_some() {
                return Err(AreteError::InvalidConfig(
                    "Set either auth.secret_key (servers and scripts) or \
                     auth.publishable_key (browsers), not both."
                        .into(),
                ));
            }
        }
        if config
            .publishable_key
            .as_deref()
            .is_some_and(|key| classify_api_key(key) == ApiKeyClass::Secret)
        {
            warn_once(
                &WARNED_SECRET_IN_PUBLISHABLE,
                &format!(
                    "auth.publishable_key was given a secret-class key (a4_sk_... or \
                     a4_ak_...). Pass it as auth.secret_key instead, or set {ARETE_API_KEY_ENV}. \
                     Publishable keys are for code shipped to a browser; {CREATE_PUBLISHABLE_HINT}."
                ),
            );
        }
        if config.has_explicit_auth() {
            return Ok(auth);
        }
    }

    let Some(env_key) = read_env(ARETE_API_KEY_ENV)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(auth);
    };
    if classify_api_key(&env_key) == ApiKeyClass::Publishable {
        warn_once(
            &WARNED_PUBLISHABLE_IN_ENV,
            &format!(
                "{ARETE_API_KEY_ENV} holds a publishable key (a4_pk_...) and was ignored. Set it \
                 to an agent key (a4_ak_...) or secret key (a4_sk_...), or pass the publishable \
                 key as auth.publishable_key."
            ),
        );
        return Ok(auth);
    }
    Ok(Some(auth.unwrap_or_default().with_secret_key(env_key)))
}

pub(crate) fn hosted_auth_required_error() -> AreteError {
    AreteError::WebSocket {
        message: "Hosted Arete websocket connections require auth.secret_key (or ARETE_API_KEY), auth.publishable_key, auth.get_token, auth.token_endpoint, or auth.token".to_string(),
        code: Some(AuthErrorCode::AuthRequired),
    }
}

#[derive(Debug, Deserialize)]
struct JwtPayload {
    exp: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{
        hosted_websocket_suffixes, is_hosted_websocket_host, HOSTED_WEBSOCKET_SUFFIXES_ENV,
    };

    /// The environment is process-wide, so these run under one lock and put it
    /// back; a leaked value would silently change what every other test treats
    /// as hosted.
    static SUFFIX_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_suffixes<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
        let _guard = SUFFIX_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var(HOSTED_WEBSOCKET_SUFFIXES_ENV).ok();
        match value {
            Some(value) => std::env::set_var(HOSTED_WEBSOCKET_SUFFIXES_ENV, value),
            None => std::env::remove_var(HOSTED_WEBSOCKET_SUFFIXES_ENV),
        }
        let outcome = body();
        match previous {
            Some(value) => std::env::set_var(HOSTED_WEBSOCKET_SUFFIXES_ENV, value),
            None => std::env::remove_var(HOSTED_WEBSOCKET_SUFFIXES_ENV),
        }
        outcome
    }

    #[test]
    fn the_default_suffix_is_hosted_without_configuration() {
        with_suffixes(None, || {
            assert!(is_hosted_websocket_host("ore.stack.arete.run"));
            assert_eq!(hosted_websocket_suffixes(), vec![".stack.arete.run"]);
        });
    }

    #[test]
    fn an_unconfigured_suffix_is_not_hosted() {
        // This is the failure the change exists for: unrecognised means no
        // session is minted, and the refusal arrives later as a 401.
        with_suffixes(None, || {
            assert!(!is_hosted_websocket_host("ore-vwqmxr.cell.arete.run"));
        });
    }

    #[test]
    fn a_configured_suffix_is_hosted_and_the_default_still_is() {
        with_suffixes(Some("cell.arete.run"), || {
            assert!(is_hosted_websocket_host("ore-vwqmxr.cell.arete.run"));
            assert!(is_hosted_websocket_host("ore.stack.arete.run"));
        });
    }

    #[test]
    fn suffixes_are_accepted_with_or_without_a_leading_dot_and_any_case() {
        with_suffixes(Some(" .Cell.Arete.Run , , second.example. "), || {
            assert!(is_hosted_websocket_host("ORE.cell.arete.run"));
            assert!(is_hosted_websocket_host("a.second.example"));
            assert!(!is_hosted_websocket_host("elsewhere.example"));
        });
    }

    #[test]
    fn a_trailing_dot_on_the_host_still_matches() {
        with_suffixes(Some("cell.arete.run"), || {
            assert!(is_hosted_websocket_host("ore-vwqmxr.cell.arete.run."));
        });
    }

    use super::*;

    fn encode_base64url(input: &str) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(input.as_bytes())
    }

    #[test]
    fn publishable_key_on_hosted_url_uses_default_token_endpoint() {
        let auth = AuthConfig::default().with_publishable_key("hspk_test");
        let strategy = auth.resolve_strategy("wss://demo.stack.arete.run");

        assert!(matches!(
            strategy,
            ResolvedAuthStrategy::TokenEndpoint(ref endpoint)
                if endpoint == DEFAULT_HOSTED_TOKEN_ENDPOINT
        ));
    }

    const SECRET: &str = "a4_sk_supersecretvalue";
    const AGENT: &str = "a4_ak_agentsecretvalue";
    const PUBLISHABLE: &str = "a4_pk_publicvalue";

    fn env_with(value: Option<&'static str>) -> impl Fn(&str) -> Option<String> {
        move |name| {
            assert_eq!(name, ARETE_API_KEY_ENV);
            value.map(str::to_string)
        }
    }

    fn assert_no_key_material(text: &str) {
        for key in [SECRET, AGENT, PUBLISHABLE] {
            assert!(!text.contains(key), "{text}");
            assert!(!text.contains(&key[6..]), "{text}");
        }
    }

    #[test]
    fn classifies_api_keys_by_prefix() {
        assert_eq!(classify_api_key(PUBLISHABLE), ApiKeyClass::Publishable);
        assert_eq!(classify_api_key("hspk_legacy"), ApiKeyClass::Publishable);
        assert_eq!(classify_api_key(SECRET), ApiKeyClass::Secret);
        assert_eq!(classify_api_key(AGENT), ApiKeyClass::Secret);
        assert_eq!(classify_api_key("hsk_legacy"), ApiKeyClass::Secret);
        assert_eq!(classify_api_key("custom-key"), ApiKeyClass::Unknown);
    }

    #[test]
    fn secret_key_on_hosted_url_uses_default_token_endpoint() {
        let auth = AuthConfig::default().with_secret_key(SECRET);
        assert_eq!(auth.api_key(), Some(SECRET));
        assert!(matches!(
            auth.resolve_strategy("wss://demo.stack.arete.run"),
            ResolvedAuthStrategy::TokenEndpoint(ref endpoint)
                if endpoint == DEFAULT_HOSTED_TOKEN_ENDPOINT
        ));
    }

    #[test]
    fn with_api_key_routes_by_prefix() {
        let secret = AuthConfig::default().with_api_key(AGENT);
        assert_eq!(secret.secret_key.as_deref(), Some(AGENT));
        assert_eq!(secret.publishable_key, None);
        let publishable = AuthConfig::default().with_api_key(PUBLISHABLE);
        assert_eq!(publishable.publishable_key.as_deref(), Some(PUBLISHABLE));
        assert_eq!(publishable.secret_key, None);
    }

    #[test]
    fn debug_output_masks_keys() {
        let debug = format!("{:?}", AuthConfig::default().with_secret_key(SECRET));
        assert!(debug.contains("secret_key"));
        assert_no_key_material(&debug);
    }

    #[test]
    fn env_key_applies_only_without_other_auth() {
        let resolved =
            resolve_auth_config_with_env(None, env_with(Some(" a4_ak_agentsecretvalue\n")))
                .unwrap()
                .unwrap();
        assert_eq!(resolved.secret_key.as_deref(), Some(AGENT));

        let with_transport = AuthConfig::default().with_token_transport(TokenTransport::Bearer);
        let resolved = resolve_auth_config_with_env(Some(with_transport), env_with(Some(AGENT)))
            .unwrap()
            .unwrap();
        assert_eq!(resolved.secret_key.as_deref(), Some(AGENT));
        assert_eq!(resolved.token_transport, TokenTransport::Bearer);

        for explicit in [
            AuthConfig::default().with_secret_key(SECRET),
            AuthConfig::default().with_publishable_key(PUBLISHABLE),
            AuthConfig::default().with_token("static"),
            AuthConfig::default().with_token_endpoint("https://auth.example.com/token"),
        ] {
            let resolved = resolve_auth_config_with_env(Some(explicit), env_with(Some(AGENT)))
                .unwrap()
                .unwrap();
            assert_ne!(resolved.api_key(), Some(AGENT));
        }

        assert!(resolve_auth_config_with_env(None, env_with(None))
            .unwrap()
            .is_none());
        assert!(resolve_auth_config_with_env(None, env_with(Some("  ")))
            .unwrap()
            .is_none());
    }

    #[test]
    fn publishable_key_in_env_is_ignored() {
        assert!(
            resolve_auth_config_with_env(None, env_with(Some(PUBLISHABLE)))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn publishable_key_as_secret_key_is_refused_without_leaking_it() {
        let error = resolve_auth_config_with_env(
            Some(AuthConfig::default().with_secret_key(PUBLISHABLE)),
            env_with(None),
        )
        .unwrap_err();
        assert!(matches!(error, AreteError::InvalidConfig(_)));
        let message = error.to_string();
        assert!(message.contains("Pass it as auth.publishable_key"));
        assert_no_key_material(&message);
    }

    #[test]
    fn empty_and_conflicting_keys_are_refused() {
        assert!(resolve_auth_config_with_env(
            Some(AuthConfig::default().with_secret_key(" ")),
            env_with(None)
        )
        .is_err());
        let error = resolve_auth_config_with_env(
            Some(
                AuthConfig::default()
                    .with_secret_key(SECRET)
                    .with_publishable_key(PUBLISHABLE),
            ),
            env_with(None),
        )
        .unwrap_err();
        assert!(error.to_string().contains("not both"));
        assert_no_key_material(&error.to_string());
    }

    #[test]
    fn secret_class_key_as_publishable_key_still_works() {
        let resolved = resolve_auth_config_with_env(
            Some(AuthConfig::default().with_publishable_key(SECRET)),
            env_with(Some(AGENT)),
        )
        .unwrap()
        .unwrap();
        assert_eq!(resolved.api_key(), Some(SECRET));
    }

    #[test]
    fn static_token_takes_precedence_over_endpoint_flow() {
        let auth = AuthConfig::default()
            .with_publishable_key("hspk_test")
            .with_token_endpoint("https://custom.example/ws/sessions")
            .with_token("static-token");

        assert!(matches!(
            auth.resolve_strategy("wss://demo.stack.arete.run"),
            ResolvedAuthStrategy::StaticToken(ref token) if token == "static-token"
        ));
    }

    #[test]
    fn build_websocket_url_adds_query_token_for_query_transport() {
        let url = build_websocket_url(
            "wss://demo.stack.arete.run/socket",
            Some("abc123"),
            TokenTransport::QueryParameter,
        )
        .expect("query auth url should build");

        assert!(url.contains("hs_token=abc123"));
    }

    #[test]
    fn session_request_without_a_release_is_the_legacy_body() {
        let body = serde_json::to_string(&TokenEndpointRequest::new(
            "wss://demo.stack.arete.run/socket",
            None,
        ))
        .expect("session request should serialize");

        assert_eq!(
            body,
            r#"{"websocket_url":"wss://demo.stack.arete.run/socket"}"#
        );
    }

    #[test]
    fn session_request_names_the_stack_release() {
        let release = StackRelease {
            stack_manifest_hash: format!("arete:h1:stack-manifest:sha256:{}", "a".repeat(64)),
            live_alias: "live".to_string(),
        };
        let body = serde_json::to_string(&TokenEndpointRequest::new(
            "wss://demo.stack.arete.run/socket",
            Some(&release),
        ))
        .expect("session request should serialize");

        assert_eq!(
            body,
            format!(
                r#"{{"websocket_url":"wss://demo.stack.arete.run/socket","stackManifestHash":"arete:h1:stack-manifest:sha256:{}","liveAlias":"live"}}"#,
                "a".repeat(64)
            )
        );
    }

    #[test]
    fn parse_jwt_expiry_reads_exp_claim() {
        let header = encode_base64url(r#"{"alg":"none","typ":"JWT"}"#);
        let payload = encode_base64url(r#"{"exp":12345}"#);
        let token = format!("{}.{}.sig", header, payload);

        assert_eq!(parse_jwt_expiry(&token), Some(12345));
    }

    #[test]
    fn token_refresh_delay_respects_refresh_buffer() {
        let now = 1_000;
        let expires_at = Some(now + TOKEN_REFRESH_BUFFER_SECONDS + 15);

        assert_eq!(token_refresh_delay(expires_at, now), Some(15));
        assert_eq!(
            token_refresh_delay(Some(now + 10), now),
            Some(MIN_REFRESH_DELAY_SECONDS)
        );
    }
}
