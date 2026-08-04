//! Shared HTTP authentication token machinery.
//!
//! Port of the HTTP-token half of the TypeScript `ConnectionManager`
//! (`getHttpAuthToken` / `clearHttpAuthToken`) plus the authenticated JSON
//! fetch with refresh-replay used by the chain, transaction, and read
//! transports.
//!
//! Semantics mirrored from `typescript/core/src/connection.ts`:
//!
//! - Strategy order: static token > token provider > token endpoint > hosted
//!   default endpoint (when the WebSocket host ends with `.stack.arete.run`).
//! - Untargeted tokens share one state; requested scopes accumulate and
//!   refreshes mint the union of granted and requested scopes.
//! - Targeted tokens (`program-read-binding` / `solana-gateway-binding`) are
//!   cached per identity `(target_kind, target_id, release_hash, sorted
//!   scopes)` in an LRU capped at [`MAX_HTTP_AUTH_TOKEN_STATES`].
//! - Tokens are considered expired at `exp - 60s` (JWT `exp` fallback).

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::auth::{parse_jwt_expiry, token_is_expiring, AuthConfig, ResolvedAuthStrategy};
use crate::error::{AreteError, AuthErrorCode};

/// Header carrying the kebab-case wire error code on failed responses.
pub const HEADER_ERROR_CODE: &str = "X-Error-Code";
/// Marker header: `false` means the upstream dispatch was definitely not
/// attempted, so replaying the request is safe even for send-style routes.
pub const HEADER_UPSTREAM_ATTEMPTED: &str = "X-Arete-Upstream-Attempted";
/// Cap on distinct targeted token identities kept in the LRU cache.
pub const MAX_HTTP_AUTH_TOKEN_STATES: usize = 32;
/// Default scope requested when callers do not specify one.
pub const DEFAULT_READ_SCOPE: &str = "read";

/// Supported targeted-token kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenTargetKind {
    /// Hosted program-read binding (`prb_…`); requires a release hash.
    ProgramReadBinding,
    /// Hosted Solana gateway binding (`sgb_…`); never carries a release hash.
    SolanaGatewayBinding,
}

impl TokenTargetKind {
    /// Wire representation used in token endpoint requests and identities.
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::ProgramReadBinding => "program-read-binding",
            Self::SolanaGatewayBinding => "solana-gateway-binding",
        }
    }
}

impl serde::Serialize for TokenTargetKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_wire())
    }
}

impl<'de> serde::Deserialize<'de> for TokenTargetKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        match raw.as_str() {
            "program-read-binding" => Ok(Self::ProgramReadBinding),
            "solana-gateway-binding" => Ok(Self::SolanaGatewayBinding),
            other => Err(serde::de::Error::custom(format!(
                "unknown token target kind '{other}'"
            ))),
        }
    }
}

/// Identity of a targeted authentication token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthTokenTarget {
    pub target_kind: TokenTargetKind,
    pub target_id: String,
    pub program_release_hash: Option<String>,
}

impl AuthTokenTarget {
    /// Program-read binding target (release hash is mandatory).
    pub fn program_read_binding(
        target_id: impl Into<String>,
        program_release_hash: impl Into<String>,
    ) -> Self {
        Self {
            target_kind: TokenTargetKind::ProgramReadBinding,
            target_id: target_id.into(),
            program_release_hash: Some(program_release_hash.into()),
        }
    }

    /// Solana gateway binding target.
    pub fn solana_gateway_binding(target_id: impl Into<String>) -> Self {
        Self {
            target_kind: TokenTargetKind::SolanaGatewayBinding,
            target_id: target_id.into(),
            program_release_hash: None,
        }
    }

    fn validate(&self) -> Result<(), AreteError> {
        let complete = match self.target_kind {
            TokenTargetKind::ProgramReadBinding => self.program_release_hash.is_some(),
            TokenTargetKind::SolanaGatewayBinding => self.program_release_hash.is_none(),
        };
        if complete && !self.target_id.is_empty() {
            Ok(())
        } else {
            Err(invalid_auth_error(
                "Targeted authentication requires a complete supported target identity",
                None,
            ))
        }
    }
}

/// A request for an HTTP authentication token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthTokenRequest {
    pub scopes: Vec<String>,
    pub target: Option<AuthTokenTarget>,
}

impl AuthTokenRequest {
    /// Untargeted request with the default `read` scope.
    pub fn read() -> Self {
        Self::scoped(vec![DEFAULT_READ_SCOPE.to_string()])
    }

    /// Untargeted request with explicit scopes.
    pub fn scoped(scopes: Vec<String>) -> Self {
        Self {
            scopes,
            target: None,
        }
    }

    /// Targeted request with explicit scopes.
    pub fn targeted(scopes: Vec<String>, target: AuthTokenTarget) -> Self {
        Self {
            scopes,
            target: Some(target),
        }
    }

    /// Deduped + sorted scopes and a validated target.
    pub fn normalized(&self) -> Result<Self, AreteError> {
        if let Some(target) = &self.target {
            target.validate()?;
        }
        Ok(Self {
            scopes: normalize_scopes(&self.scopes),
            target: self.target.clone(),
        })
    }
}

/// Dedupe and sort scopes (the canonical order used everywhere: request
/// bodies, cache identities, coverage checks).
pub(crate) fn normalize_scopes(scopes: &[String]) -> Vec<String> {
    scopes
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn targeted_identity(target: &AuthTokenTarget, sorted_scopes: &[String]) -> String {
    // Mirror of the TS identity: JSON [kind, id, releaseHash ?? null, scopes].
    serde_json::to_string(&json!([
        target.target_kind.as_wire(),
        target.target_id,
        target.program_release_hash,
        sorted_scopes,
    ]))
    .expect("identity serialization is infallible")
}

/// Source of bearer tokens for authenticated HTTP requests.
#[async_trait]
pub trait TokenSource: Send + Sync {
    /// Returns a token satisfying `request`, or `None` when the configuration
    /// carries no authentication. `force_refresh` bypasses any cached token.
    async fn token(
        &self,
        request: &AuthTokenRequest,
        force_refresh: bool,
    ) -> Result<Option<String>, AreteError>;

    /// Drops any cached token matching `request` so the next call re-mints.
    fn invalidate(&self, request: &AuthTokenRequest);
}

fn invalid_auth_error(message: &str, code: Option<AuthErrorCode>) -> AreteError {
    AreteError::AuthRequestFailed {
        status: 0,
        message: message.to_string(),
        code,
    }
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Normalizes a raw wire code (kebab- or snake-case) into an [`AuthErrorCode`].
pub fn parse_wire_error_code(raw: &str) -> Option<AuthErrorCode> {
    AuthErrorCode::from_wire(&raw.trim().replace('_', "-"))
}

#[derive(Debug, Default, Clone)]
struct SharedTokenState {
    token: Option<String>,
    expires_at: Option<u64>,
    scopes: BTreeSet<String>,
    requested_scopes: BTreeSet<String>,
}

impl SharedTokenState {
    fn valid_token_covering(&self, required: &[String], now: u64) -> Option<String> {
        let token = self.token.as_ref()?;
        if token_is_expiring(self.expires_at, now) {
            return None;
        }
        if required.iter().all(|scope| self.scopes.contains(scope)) {
            Some(token.clone())
        } else {
            None
        }
    }

    fn clear(&mut self) {
        // Requested scopes survive a clear, matching the TS `clearTokenState`.
        self.token = None;
        self.expires_at = None;
        self.scopes.clear();
    }
}

#[derive(Debug, Clone)]
struct CachedTargetToken {
    token: String,
    expires_at: Option<u64>,
}

#[derive(Debug, Default)]
struct AuthTokenCache {
    shared: SharedTokenState,
    /// LRU order: index 0 is the oldest entry.
    targeted: Vec<(String, CachedTargetToken)>,
}

impl AuthTokenCache {
    fn targeted_get(&mut self, identity: &str, now: u64) -> Option<String> {
        let index = self.targeted.iter().position(|(key, _)| key == identity)?;
        if token_is_expiring(self.targeted[index].1.expires_at, now) {
            return None;
        }
        // Touch: move to the most-recently-used position.
        let entry = self.targeted.remove(index);
        let token = entry.1.token.clone();
        self.targeted.push(entry);
        Some(token)
    }

    fn targeted_insert(&mut self, identity: String, token: CachedTargetToken) {
        self.targeted.retain(|(key, _)| key != &identity);
        self.targeted.push((identity, token));
        while self.targeted.len() > MAX_HTTP_AUTH_TOKEN_STATES {
            self.targeted.remove(0);
        }
    }

    fn targeted_remove(&mut self, identity: &str) {
        self.targeted.retain(|(key, _)| key != identity);
    }
}

struct MintedToken {
    token: String,
    expires_at: Option<u64>,
    scopes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct EndpointTokenResponse {
    token: String,
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default, rename = "expiresAt")]
    expires_at_camel: Option<u64>,
    #[serde(default)]
    scopes: Option<Vec<String>>,
}

/// Shared token machinery for authenticated HTTP transports.
///
/// Built from the SDK [`AuthConfig`] plus the (optional) WebSocket URL the
/// client was configured with; the URL selects the hosted default token
/// endpoint and is echoed in untargeted token endpoint requests.
pub struct HttpAuthClient {
    auth: Option<AuthConfig>,
    websocket_url: Option<String>,
    http: reqwest::Client,
    cache: Mutex<AuthTokenCache>,
    fetch_permit: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for HttpAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpAuthClient")
            .field("auth", &self.auth)
            .field("websocket_url", &self.websocket_url)
            .finish_non_exhaustive()
    }
}

impl HttpAuthClient {
    /// Creates a token client mirroring the TS `ConnectionManager` HTTP-token
    /// behavior.
    pub fn new(
        auth: Option<AuthConfig>,
        websocket_url: Option<String>,
        http_client: reqwest::Client,
    ) -> Self {
        Self {
            auth,
            websocket_url,
            http: http_client,
            cache: Mutex::new(AuthTokenCache::default()),
            fetch_permit: tokio::sync::Mutex::new(()),
        }
    }

    /// Clears every cached token (untargeted and targeted).
    pub fn clear_all(&self) {
        let mut cache = self.cache.lock().expect("auth cache lock poisoned");
        cache.shared.clear();
        cache.targeted.clear();
    }

    fn strategy(&self) -> ResolvedAuthStrategy {
        match &self.auth {
            Some(auth) => auth.resolve_strategy(self.websocket_url.as_deref().unwrap_or("")),
            None => ResolvedAuthStrategy::None,
        }
    }

    async fn mint_token(
        &self,
        target: Option<&AuthTokenTarget>,
        scopes: &[String],
    ) -> Result<Option<MintedToken>, AreteError> {
        match self.strategy() {
            ResolvedAuthStrategy::None => Ok(None),
            ResolvedAuthStrategy::StaticToken(token) => Ok(Some(MintedToken {
                token,
                expires_at: None,
                scopes: None,
            })),
            ResolvedAuthStrategy::TokenProvider(provider) => {
                let token = provider().await?;
                Ok(Some(MintedToken {
                    token: token.token,
                    expires_at: token.expires_at,
                    scopes: None,
                }))
            }
            ResolvedAuthStrategy::TokenEndpoint(endpoint) => self
                .fetch_token_from_endpoint(&endpoint, target, scopes)
                .await
                .map(Some),
        }
    }

    fn token_endpoint_body(
        &self,
        target: Option<&AuthTokenTarget>,
        scopes: &[String],
    ) -> serde_json::Value {
        match target {
            Some(target) => {
                let mut body = json!({
                    "targetKind": target.target_kind.as_wire(),
                    "targetId": target.target_id,
                    "scopes": scopes,
                });
                if let Some(hash) = &target.program_release_hash {
                    body["programReleaseHash"] = json!(hash);
                }
                body
            }
            None => json!({
                "websocket_url": self.websocket_url.as_deref().unwrap_or(""),
                "scopes": scopes,
            }),
        }
    }

    async fn fetch_token_from_endpoint(
        &self,
        endpoint: &str,
        target: Option<&AuthTokenTarget>,
        scopes: &[String],
    ) -> Result<MintedToken, AreteError> {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};

        let mut headers = HeaderMap::new();
        if let Some(auth) = &self.auth {
            if let Some(publishable_key) = &auth.publishable_key {
                headers.insert(
                    AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {publishable_key}")).map_err(
                        |error| {
                            AreteError::ConnectionFailed(format!(
                                "Invalid publishable key: {error}"
                            ))
                        },
                    )?,
                );
            }
            for (name, value) in &auth.token_endpoint_headers {
                let name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                    AreteError::ConnectionFailed(format!("Invalid token endpoint header: {error}"))
                })?;
                let value = HeaderValue::from_str(value).map_err(|error| {
                    AreteError::ConnectionFailed(format!("Invalid token endpoint header: {error}"))
                })?;
                headers.insert(name, value);
            }
        }

        let response = self
            .http
            .post(endpoint)
            .headers(headers)
            .json(&self.token_endpoint_body(target, scopes))
            .send()
            .await
            .map_err(|error| AreteError::ConnectionFailed(error.to_string()))?;

        let status = response.status();
        let header_code = response
            .headers()
            .get(HEADER_ERROR_CODE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = response
            .bytes()
            .await
            .map_err(|error| AreteError::ConnectionFailed(error.to_string()))?;

        if !status.is_success() {
            return Err(AreteError::from_auth_response(
                status.as_u16(),
                header_code.as_deref(),
                Some(&body),
                status.canonical_reason(),
            ));
        }

        let data: EndpointTokenResponse = serde_json::from_slice(&body)
            .map_err(|error| AreteError::Serialization(error.to_string()))?;
        if data.token.is_empty() {
            return Err(invalid_auth_error(
                "Token endpoint did not return a token",
                None,
            ));
        }
        Ok(MintedToken {
            expires_at: data.expires_at.or(data.expires_at_camel),
            scopes: data.scopes,
            token: data.token,
        })
    }

    fn finalize_minted(
        minted: MintedToken,
        requested_scopes: &[String],
    ) -> Result<(String, Option<u64>, BTreeSet<String>), AreteError> {
        if minted.token.is_empty() {
            return Err(invalid_auth_error(
                "Authentication provider returned an empty token",
                None,
            ));
        }
        let granted: BTreeSet<String> = minted
            .scopes
            .map(|scopes| scopes.into_iter().collect())
            .unwrap_or_else(|| requested_scopes.iter().cloned().collect());
        let expires_at = minted.expires_at.or_else(|| parse_jwt_expiry(&minted.token));
        if token_is_expiring(expires_at, now_epoch_seconds()) {
            return Err(invalid_auth_error(
                "Authentication token is expired",
                Some(AuthErrorCode::TokenExpired),
            ));
        }
        Ok((minted.token, expires_at, granted))
    }

    fn scope_coverage_error(required: &[String]) -> AreteError {
        invalid_auth_error(
            &format!(
                "Authentication token was not granted required scopes: {}",
                required.join(", ")
            ),
            Some(AuthErrorCode::AuthRequired),
        )
    }

    async fn shared_token(
        &self,
        required_scopes: &[String],
        force_refresh: bool,
    ) -> Result<Option<String>, AreteError> {
        let now = now_epoch_seconds();
        {
            let mut cache = self.cache.lock().expect("auth cache lock poisoned");
            for scope in required_scopes {
                cache.shared.requested_scopes.insert(scope.clone());
            }
            if !force_refresh {
                if let Some(token) = cache.shared.valid_token_covering(required_scopes, now) {
                    return Ok(Some(token));
                }
            }
        }

        let _permit = self.fetch_permit.lock().await;
        let fetch_scopes = {
            let cache = self.cache.lock().expect("auth cache lock poisoned");
            if !force_refresh {
                if let Some(token) = cache
                    .shared
                    .valid_token_covering(required_scopes, now_epoch_seconds())
                {
                    // Another caller minted a satisfying token while we waited.
                    return Ok(Some(token));
                }
            }
            let mut union: BTreeSet<String> = cache.shared.scopes.clone();
            union.extend(cache.shared.requested_scopes.iter().cloned());
            union.into_iter().collect::<Vec<_>>()
        };

        let Some(minted) = self.mint_token(None, &fetch_scopes).await? else {
            return Ok(None);
        };
        let (token, expires_at, granted) = Self::finalize_minted(minted, &fetch_scopes)?;

        let covered = required_scopes.iter().all(|scope| granted.contains(scope));
        {
            let mut cache = self.cache.lock().expect("auth cache lock poisoned");
            cache.shared.token = Some(token.clone());
            cache.shared.expires_at = expires_at;
            cache.shared.scopes = granted;
        }
        if !covered {
            return Err(Self::scope_coverage_error(required_scopes));
        }
        Ok(Some(token))
    }

    async fn targeted_token(
        &self,
        target: &AuthTokenTarget,
        required_scopes: &[String],
        force_refresh: bool,
    ) -> Result<Option<String>, AreteError> {
        let identity = targeted_identity(target, required_scopes);
        if !force_refresh {
            let mut cache = self.cache.lock().expect("auth cache lock poisoned");
            if let Some(token) = cache.targeted_get(&identity, now_epoch_seconds()) {
                return Ok(Some(token));
            }
        }

        let _permit = self.fetch_permit.lock().await;
        if !force_refresh {
            let mut cache = self.cache.lock().expect("auth cache lock poisoned");
            if let Some(token) = cache.targeted_get(&identity, now_epoch_seconds()) {
                return Ok(Some(token));
            }
        }

        let Some(minted) = self.mint_token(Some(target), required_scopes).await? else {
            return Ok(None);
        };
        // Targeted grants are checked before caching (TS `updateHttpTokenState`).
        let had_explicit_scopes = minted.scopes.is_some();
        let (token, expires_at, granted) = Self::finalize_minted(minted, required_scopes)?;
        if had_explicit_scopes && !required_scopes.iter().all(|scope| granted.contains(scope)) {
            return Err(Self::scope_coverage_error(required_scopes));
        }

        let mut cache = self.cache.lock().expect("auth cache lock poisoned");
        cache.targeted_insert(
            identity,
            CachedTargetToken {
                token: token.clone(),
                expires_at,
            },
        );
        Ok(Some(token))
    }
}

#[async_trait]
impl TokenSource for HttpAuthClient {
    async fn token(
        &self,
        request: &AuthTokenRequest,
        force_refresh: bool,
    ) -> Result<Option<String>, AreteError> {
        let normalized = request.normalized()?;
        match &normalized.target {
            Some(target) => {
                self.targeted_token(target, &normalized.scopes, force_refresh)
                    .await
            }
            None => self.shared_token(&normalized.scopes, force_refresh).await,
        }
    }

    fn invalidate(&self, request: &AuthTokenRequest) {
        let scopes = normalize_scopes(&request.scopes);
        let mut cache = self.cache.lock().expect("auth cache lock poisoned");
        match &request.target {
            Some(target) => {
                let identity = targeted_identity(target, &scopes);
                cache.targeted_remove(&identity);
            }
            None => cache.shared.clear(),
        }
    }
}

// ---------------------------------------------------------------------------
// BearerTokenSource adapter (program read / query transports)
// ---------------------------------------------------------------------------

fn read_target_to_token_target(
    target: &crate::program_read_transport::ReadAuthTarget,
) -> Result<AuthTokenTarget, AreteError> {
    let target_kind = match target.target_kind.as_str() {
        "program-read-binding" => TokenTargetKind::ProgramReadBinding,
        "solana-gateway-binding" => TokenTargetKind::SolanaGatewayBinding,
        other => {
            return Err(invalid_auth_error(
                &format!("Unsupported token target kind '{other}'"),
                None,
            ))
        }
    };
    Ok(AuthTokenTarget {
        target_kind,
        target_id: target.target_id.clone(),
        program_release_hash: target.program_release_hash.clone(),
    })
}

fn read_request(
    scopes: &[&str],
    target: Option<&crate::program_read_transport::ReadAuthTarget>,
) -> Result<AuthTokenRequest, AreteError> {
    Ok(AuthTokenRequest {
        scopes: scopes.iter().map(|scope| scope.to_string()).collect(),
        target: target.map(read_target_to_token_target).transpose()?,
    })
}

/// Adapter letting the shared [`HttpAuthClient`] serve the program-read and
/// query transports (which speak [`BearerTokenSource`]).
#[async_trait]
impl crate::program_read_transport::BearerTokenSource for HttpAuthClient {
    async fn bearer_token(
        &self,
        scopes: &[&str],
        target: Option<&crate::program_read_transport::ReadAuthTarget>,
        force_refresh: bool,
    ) -> Result<Option<String>, AreteError> {
        let request = read_request(scopes, target)?;
        TokenSource::token(self, &request, force_refresh).await
    }

    fn invalidate(
        &self,
        scopes: &[&str],
        target: Option<&crate::program_read_transport::ReadAuthTarget>,
    ) {
        if let Ok(request) = read_request(scopes, target) {
            TokenSource::invalidate(self, &request);
        }
    }
}

/// HTTP method for [`fetch_json`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

/// An authenticated JSON request executed by [`fetch_json`].
#[derive(Debug, Clone)]
pub struct AuthedRequest {
    pub method: HttpMethod,
    pub url: String,
    pub body: Option<serde_json::Value>,
    pub scopes: Vec<String>,
    pub target: Option<AuthTokenTarget>,
    /// When set, the refresh-replay only happens if the failed response
    /// carries `X-Arete-Upstream-Attempted: false` (i.e. the server proved it
    /// never dispatched the request upstream). Used for transaction sends.
    pub require_predispatch_marker: bool,
}

impl AuthedRequest {
    /// GET request with the default `read` scope.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: HttpMethod::Get,
            url: url.into(),
            body: None,
            scopes: vec![DEFAULT_READ_SCOPE.to_string()],
            target: None,
            require_predispatch_marker: false,
        }
    }

    /// POST request with a JSON body and the default `read` scope.
    pub fn post(url: impl Into<String>, body: serde_json::Value) -> Self {
        Self {
            method: HttpMethod::Post,
            url: url.into(),
            body: Some(body),
            scopes: vec![DEFAULT_READ_SCOPE.to_string()],
            target: None,
            require_predispatch_marker: false,
        }
    }

    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = scopes;
        self
    }

    pub fn with_target(mut self, target: AuthTokenTarget) -> Self {
        self.target = Some(target);
        self
    }

    pub fn with_predispatch_marker(mut self, required: bool) -> Self {
        self.require_predispatch_marker = required;
        self
    }
}

/// Response from [`fetch_json`]: status, wire error code (non-2xx only), and
/// the raw body bytes.
#[derive(Debug, Clone)]
pub struct AuthedResponse {
    pub status: u16,
    pub error_code: Option<AuthErrorCode>,
    pub body: Vec<u8>,
}

impl AuthedResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

struct RawResponse {
    status: u16,
    header_code: Option<String>,
    upstream_attempted: Option<String>,
    body: Vec<u8>,
}

impl RawResponse {
    fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    fn error_code(&self) -> Option<AuthErrorCode> {
        if let Some(code) = self.header_code.as_deref().and_then(parse_wire_error_code) {
            return Some(code);
        }
        #[derive(Deserialize)]
        struct BodyCode {
            #[serde(default)]
            code: Option<String>,
        }
        serde_json::from_slice::<BodyCode>(&self.body)
            .ok()
            .and_then(|payload| payload.code)
            .as_deref()
            .and_then(parse_wire_error_code)
    }

    fn into_authed(self) -> AuthedResponse {
        let error_code = if self.is_success() {
            None
        } else {
            self.error_code()
        };
        AuthedResponse {
            status: self.status,
            error_code,
            body: self.body,
        }
    }
}

async fn attempt(
    http: &reqwest::Client,
    tokens: &dyn TokenSource,
    request: &AuthedRequest,
    token_request: &AuthTokenRequest,
    force_refresh: bool,
) -> Result<RawResponse, AreteError> {
    let token = tokens.token(token_request, force_refresh).await?;
    let mut builder = match request.method {
        HttpMethod::Get => http.get(&request.url),
        HttpMethod::Post => http.post(&request.url),
    };
    if let Some(body) = &request.body {
        builder = builder.json(body);
    }
    if let Some(token) = token {
        builder = builder.bearer_auth(token);
    }
    let response = builder
        .send()
        .await
        .map_err(|error| AreteError::ConnectionFailed(error.to_string()))?;
    let status = response.status().as_u16();
    let header_code = response
        .headers()
        .get(HEADER_ERROR_CODE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let upstream_attempted = response
        .headers()
        .get(HEADER_UPSTREAM_ATTEMPTED)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = response
        .bytes()
        .await
        .map_err(|error| AreteError::ConnectionFailed(error.to_string()))?
        .to_vec();
    Ok(RawResponse {
        status,
        header_code,
        upstream_attempted,
        body,
    })
}

/// Performs an authenticated JSON request.
///
/// On a non-2xx response the wire error code is read from the `X-Error-Code`
/// header (falling back to the `{"error","code"}` JSON body). When the code is
/// refresh-worthy ([`AuthErrorCode::should_refresh_token`]) the cached token
/// is invalidated and the request replayed exactly once — unless
/// `require_predispatch_marker` is set and the response does not carry
/// `X-Arete-Upstream-Attempted: false`.
pub async fn fetch_json(
    http: &reqwest::Client,
    tokens: &dyn TokenSource,
    request: &AuthedRequest,
) -> Result<AuthedResponse, AreteError> {
    let token_request = AuthTokenRequest {
        scopes: request.scopes.clone(),
        target: request.target.clone(),
    };

    let first = attempt(http, tokens, request, &token_request, false).await?;
    if first.is_success() {
        return Ok(first.into_authed());
    }

    let refresh_worthy = first
        .error_code()
        .map(AuthErrorCode::should_refresh_token)
        .unwrap_or(false);
    let explicitly_not_dispatched = first.upstream_attempted.as_deref() == Some("false");
    if refresh_worthy && (!request.require_predispatch_marker || explicitly_not_dispatched) {
        tokens.invalidate(&token_request);
        let second = attempt(http, tokens, request, &token_request, true).await?;
        return Ok(second.into_authed());
    }

    Ok(first.into_authed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::routing::post;
    use axum::{Json, Router};
    use base64::Engine as _;
    use serde_json::Value;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::net::TcpListener;

    fn encode_base64url(input: &str) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(input.as_bytes())
    }

    fn jwt_with_exp(exp: u64) -> String {
        format!(
            "{}.{}.sig",
            encode_base64url(r#"{"alg":"none","typ":"JWT"}"#),
            encode_base64url(&format!(r#"{{"exp":{exp}}}"#))
        )
    }

    type CapturedHeaders = Vec<(Option<String>, Option<String>)>;

    #[derive(Clone, Default)]
    struct EndpointState {
        bodies: Arc<std::sync::Mutex<Vec<Value>>>,
        headers: Arc<std::sync::Mutex<CapturedHeaders>>,
        mints: Arc<AtomicUsize>,
        scopes_override: Option<Vec<String>>,
    }

    async fn spawn_token_endpoint(state: EndpointState) -> String {
        async fn handler(
            State(state): State<EndpointState>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> Json<Value> {
            let n = state.mints.fetch_add(1, Ordering::SeqCst);
            state.bodies.lock().unwrap().push(body);
            state.headers.lock().unwrap().push((
                headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string),
                headers
                    .get("x-custom")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string),
            ));
            let mut response = serde_json::json!({
                "token": format!("token-{n}"),
                "expires_at": now_epoch_seconds() + 3600,
            });
            if let Some(scopes) = &state.scopes_override {
                response["scopes"] = serde_json::json!(scopes);
            }
            Json(response)
        }

        let router = Router::new().route("/", post(handler)).with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{addr}/")
    }

    fn endpoint_client(endpoint: &str) -> HttpAuthClient {
        let auth = AuthConfig::default()
            .with_publishable_key("hspk_test")
            .with_token_endpoint(endpoint)
            .with_token_endpoint_header("x-custom", "custom-value");
        HttpAuthClient::new(
            Some(auth),
            Some("ws://127.0.0.1:9/socket".to_string()),
            reqwest::Client::new(),
        )
    }

    fn target(id: &str) -> AuthTokenTarget {
        AuthTokenTarget::program_read_binding(id, "release-hash")
    }

    #[tokio::test]
    async fn no_auth_config_yields_no_token() {
        let client = HttpAuthClient::new(None, None, reqwest::Client::new());
        let token = client.token(&AuthTokenRequest::read(), false).await.unwrap();
        assert_eq!(token, None);
    }

    #[tokio::test]
    async fn static_token_wins_over_provider_and_endpoint() {
        let auth = AuthConfig::default()
            .with_token("static-token")
            .with_token_provider(|| async { Ok(crate::auth::AuthToken::new("provider-token")) })
            .with_token_endpoint("http://127.0.0.1:9/never");
        let client = HttpAuthClient::new(Some(auth), None, reqwest::Client::new());
        let token = client.token(&AuthTokenRequest::read(), false).await.unwrap();
        assert_eq!(token.as_deref(), Some("static-token"));
    }

    #[tokio::test]
    async fn provider_wins_over_endpoint() {
        let auth = AuthConfig::default()
            .with_token_provider(|| async { Ok(crate::auth::AuthToken::new("provider-token")) })
            .with_token_endpoint("http://127.0.0.1:9/never");
        let client = HttpAuthClient::new(Some(auth), None, reqwest::Client::new());
        let token = client.token(&AuthTokenRequest::read(), false).await.unwrap();
        assert_eq!(token.as_deref(), Some("provider-token"));
    }

    #[tokio::test]
    async fn endpoint_flow_sends_untargeted_body_and_headers() {
        let state = EndpointState::default();
        let endpoint = spawn_token_endpoint(state.clone()).await;
        let client = endpoint_client(&endpoint);

        let token = client
            .token(
                &AuthTokenRequest::scoped(vec![
                    "read".to_string(),
                    "read".to_string(),
                    "transaction:inspect".to_string(),
                ]),
                false,
            )
            .await
            .unwrap();
        assert_eq!(token.as_deref(), Some("token-0"));

        let bodies = state.bodies.lock().unwrap();
        assert_eq!(
            bodies[0],
            serde_json::json!({
                "websocket_url": "ws://127.0.0.1:9/socket",
                "scopes": ["read", "transaction:inspect"],
            })
        );
        let headers = state.headers.lock().unwrap();
        assert_eq!(
            headers[0],
            (
                Some("Bearer hspk_test".to_string()),
                Some("custom-value".to_string())
            )
        );
    }

    #[tokio::test]
    async fn untargeted_tokens_are_cached_and_scopes_accumulate() {
        let state = EndpointState::default();
        let endpoint = spawn_token_endpoint(state.clone()).await;
        let client = endpoint_client(&endpoint);

        let first = client.token(&AuthTokenRequest::read(), false).await.unwrap();
        let cached = client.token(&AuthTokenRequest::read(), false).await.unwrap();
        assert_eq!(first, cached);
        assert_eq!(state.mints.load(Ordering::SeqCst), 1);

        // A new scope forces a refresh minting the accumulated union.
        let widened = client
            .token(
                &AuthTokenRequest::scoped(vec!["transaction:inspect".to_string()]),
                false,
            )
            .await
            .unwrap();
        assert_eq!(widened.as_deref(), Some("token-1"));
        assert_eq!(state.mints.load(Ordering::SeqCst), 2);
        let bodies = state.bodies.lock().unwrap();
        assert_eq!(
            bodies[1]["scopes"],
            serde_json::json!(["read", "transaction:inspect"])
        );
    }

    #[tokio::test]
    async fn force_refresh_and_invalidate_mint_new_tokens() {
        let state = EndpointState::default();
        let endpoint = spawn_token_endpoint(state.clone()).await;
        let client = endpoint_client(&endpoint);

        let first = client.token(&AuthTokenRequest::read(), false).await.unwrap();
        let forced = client.token(&AuthTokenRequest::read(), true).await.unwrap();
        assert_ne!(first, forced);

        client.invalidate(&AuthTokenRequest::read());
        let after_invalidate = client.token(&AuthTokenRequest::read(), false).await.unwrap();
        assert_ne!(forced, after_invalidate);
        assert_eq!(state.mints.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn targeted_body_includes_target_identity() {
        let state = EndpointState::default();
        let endpoint = spawn_token_endpoint(state.clone()).await;
        let client = endpoint_client(&endpoint);

        client
            .token(
                &AuthTokenRequest::targeted(
                    vec!["read".to_string()],
                    AuthTokenTarget::program_read_binding("prb_1", "hash-1"),
                ),
                false,
            )
            .await
            .unwrap();
        client
            .token(
                &AuthTokenRequest::targeted(
                    vec!["transaction:send".to_string()],
                    AuthTokenTarget::solana_gateway_binding("sgb_1"),
                ),
                false,
            )
            .await
            .unwrap();

        let bodies = state.bodies.lock().unwrap();
        assert_eq!(
            bodies[0],
            serde_json::json!({
                "targetKind": "program-read-binding",
                "targetId": "prb_1",
                "scopes": ["read"],
                "programReleaseHash": "hash-1",
            })
        );
        assert_eq!(
            bodies[1],
            serde_json::json!({
                "targetKind": "solana-gateway-binding",
                "targetId": "sgb_1",
                "scopes": ["transaction:send"],
            })
        );
    }

    #[tokio::test]
    async fn targeted_cache_identity_normalizes_scopes() {
        let state = EndpointState::default();
        let endpoint = spawn_token_endpoint(state.clone()).await;
        let client = endpoint_client(&endpoint);

        let scrambled = AuthTokenRequest::targeted(
            vec!["write".to_string(), "read".to_string(), "read".to_string()],
            target("prb_1"),
        );
        let sorted = AuthTokenRequest::targeted(
            vec!["read".to_string(), "write".to_string()],
            target("prb_1"),
        );
        let a = client.token(&scrambled, false).await.unwrap();
        let b = client.token(&sorted, false).await.unwrap();
        assert_eq!(a, b);
        assert_eq!(state.mints.load(Ordering::SeqCst), 1);

        // Different target id is a different identity.
        let other = client
            .token(
                &AuthTokenRequest::targeted(
                    vec!["read".to_string(), "write".to_string()],
                    target("prb_2"),
                ),
                false,
            )
            .await
            .unwrap();
        assert_ne!(a, other);
        assert_eq!(state.mints.load(Ordering::SeqCst), 2);

        // Targeted invalidation only clears the matching identity.
        client.invalidate(&scrambled);
        let re_minted = client.token(&sorted, false).await.unwrap();
        assert_ne!(a, re_minted);
        let other_cached = client
            .token(
                &AuthTokenRequest::targeted(
                    vec!["read".to_string(), "write".to_string()],
                    target("prb_2"),
                ),
                false,
            )
            .await
            .unwrap();
        assert_eq!(other, other_cached);
    }

    #[tokio::test]
    async fn targeted_cache_evicts_oldest_beyond_cap() {
        let state = EndpointState::default();
        let endpoint = spawn_token_endpoint(state.clone()).await;
        let client = endpoint_client(&endpoint);

        for i in 0..=MAX_HTTP_AUTH_TOKEN_STATES {
            client
                .token(
                    &AuthTokenRequest::targeted(
                        vec!["read".to_string()],
                        target(&format!("prb_{i}")),
                    ),
                    false,
                )
                .await
                .unwrap();
        }
        assert_eq!(
            state.mints.load(Ordering::SeqCst),
            MAX_HTTP_AUTH_TOKEN_STATES + 1
        );

        // The newest entry is still cached…
        client
            .token(
                &AuthTokenRequest::targeted(
                    vec!["read".to_string()],
                    target(&format!("prb_{MAX_HTTP_AUTH_TOKEN_STATES}")),
                ),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            state.mints.load(Ordering::SeqCst),
            MAX_HTTP_AUTH_TOKEN_STATES + 1
        );

        // …but the oldest was evicted and re-mints.
        client
            .token(
                &AuthTokenRequest::targeted(vec!["read".to_string()], target("prb_0")),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            state.mints.load(Ordering::SeqCst),
            MAX_HTTP_AUTH_TOKEN_STATES + 2
        );
    }

    #[tokio::test]
    async fn scope_coverage_failure_is_an_auth_error() {
        let state = EndpointState {
            scopes_override: Some(vec!["read".to_string()]),
            ..EndpointState::default()
        };
        let endpoint = spawn_token_endpoint(state.clone()).await;
        let client = endpoint_client(&endpoint);

        let error = client
            .token(
                &AuthTokenRequest::scoped(vec!["transaction:send".to_string()]),
                false,
            )
            .await
            .unwrap_err();
        assert_eq!(error.auth_code(), Some(AuthErrorCode::AuthRequired));
    }

    #[tokio::test]
    async fn jwt_expiry_fallback_rejects_expired_static_tokens() {
        let expired = jwt_with_exp(now_epoch_seconds().saturating_sub(10));
        let auth = AuthConfig::default().with_token(expired);
        let client = HttpAuthClient::new(Some(auth), None, reqwest::Client::new());
        let error = client
            .token(&AuthTokenRequest::read(), false)
            .await
            .unwrap_err();
        assert_eq!(error.auth_code(), Some(AuthErrorCode::TokenExpired));

        let valid = jwt_with_exp(now_epoch_seconds() + 3600);
        let auth = AuthConfig::default().with_token(valid.clone());
        let client = HttpAuthClient::new(Some(auth), None, reqwest::Client::new());
        let token = client.token(&AuthTokenRequest::read(), false).await.unwrap();
        assert_eq!(token, Some(valid));
    }

    #[tokio::test]
    async fn incomplete_targets_are_rejected() {
        let client = HttpAuthClient::new(None, None, reqwest::Client::new());
        let request = AuthTokenRequest::targeted(
            vec!["read".to_string()],
            AuthTokenTarget {
                target_kind: TokenTargetKind::ProgramReadBinding,
                target_id: "prb_1".to_string(),
                program_release_hash: None,
            },
        );
        assert!(client.token(&request, false).await.is_err());

        let request = AuthTokenRequest::targeted(
            vec!["read".to_string()],
            AuthTokenTarget {
                target_kind: TokenTargetKind::SolanaGatewayBinding,
                target_id: "sgb_1".to_string(),
                program_release_hash: Some("unexpected".to_string()),
            },
        );
        assert!(client.token(&request, false).await.is_err());
    }

    #[test]
    fn target_kind_serializes_to_wire_strings() {
        assert_eq!(
            serde_json::to_value(TokenTargetKind::ProgramReadBinding).unwrap(),
            serde_json::json!("program-read-binding")
        );
        assert_eq!(
            serde_json::to_value(TokenTargetKind::SolanaGatewayBinding).unwrap(),
            serde_json::json!("solana-gateway-binding")
        );
        assert_eq!(
            serde_json::from_value::<TokenTargetKind>(serde_json::json!(
                "solana-gateway-binding"
            ))
            .unwrap(),
            TokenTargetKind::SolanaGatewayBinding
        );
    }

    // fetch_json refresh-replay and predispatch-marker semantics are covered
    // end-to-end in the chain and transactions module tests.
}
