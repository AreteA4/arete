//! The protocol client: sessions, prepare/submit, reads, metadata and the
//! instance event stream.
//!
//! Mirror of `createProtocolClient` in `typescript/core/src/protocol/client.ts`.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use base64::Engine as _;
use futures_util::Stream;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, CACHE_CONTROL};
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};
use tokio::sync::mpsc;

use super::handle::ProtocolLiveViews;
use super::metadata_proof::{
    now_unix_seconds, protocol_metadata_proof_message, sign_protocol_metadata_proof,
    MetadataProofError, ProtocolMetadataProofInput, ProtocolMetadataProofMessage,
    ProtocolMetadataWalletProof, SignMetadataProofError,
    PROTOCOL_METADATA_PROOF_DEFAULT_TTL_SECONDS,
};
use super::sse::{ServerSentEvent, ServerSentEventParser};
use super::types::*;
use crate::auth::{classify_api_key, ApiKeyClass, AuthConfig};
use crate::error::{AreteError, AuthErrorCode};
use crate::http::{
    parse_wire_error_code, AuthTokenRequest, AuthTokenTarget, HttpAuthClient, TokenSource,
    HEADER_ERROR_CODE,
};

/// Session scope for every read.
pub const PROTOCOL_READ_SCOPE: &str = "protocol:read";
/// Session scope for submitting signed transactions.
pub const PROTOCOL_SUBMIT_SCOPE: &str = "protocol:submit";
/// Session scope a secret key asks for: prepare any client-callable step.
pub const PROTOCOL_PREPARE_ANY_SCOPE: &str = "protocol:prepare:*";

/// The prepare scope of one step: `protocol:prepare:<step>`.
pub fn protocol_prepare_scope(step: &str) -> String {
    format!("protocol:prepare:{step}")
}

const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// A protocol client failure. Rejections (guards and access refusals) are
/// not errors: they come back as [`ProtocolPrepareResult::Rejected`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProtocolError {
    /// Session minting or authentication configuration failed.
    #[error(transparent)]
    Auth(#[from] AreteError),
    /// The hosted API answered with a non-success status.
    #[error("{what} returned {status}: {message}")]
    Request {
        what: String,
        status: u16,
        /// Raw `X-Error-Code` header or body `code`.
        wire_code: Option<String>,
        message: String,
        body: Option<Value>,
    },
    /// The request could not be sent or its response not read.
    #[error("Protocol request failed: {0}")]
    Transport(String),
    /// The hosted API answered with a body this client cannot read.
    #[error("{0}")]
    UnexpectedResponse(String),
    /// A caller argument is invalid.
    #[error("Invalid protocol input: {0}")]
    InvalidInput(String),
    /// The event stream reported an error event.
    #[error("Event stream error: {code}")]
    Stream { code: String },
    #[error(transparent)]
    MetadataProof(#[from] MetadataProofError),
}

impl ProtocolError {
    /// HTTP status of a [`Self::Request`] failure.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Request { status, .. } => Some(*status),
            Self::Auth(AreteError::AuthRequestFailed { status, .. }) => Some(*status),
            _ => None,
        }
    }

    /// The raw wire code of a failure, when the server sent one.
    pub fn wire_code(&self) -> Option<&str> {
        match self {
            Self::Request { wire_code, .. } => wire_code.as_deref(),
            Self::Stream { code } => Some(code),
            Self::Auth(AreteError::AuthRequestFailed {
                problem: Some(problem),
                ..
            }) => problem.code.as_deref(),
            Self::Auth(AreteError::UntrustedCredentialDestination { .. }) => {
                Some(crate::error::UNTRUSTED_CREDENTIAL_DESTINATION)
            }
            _ => None,
        }
    }

    /// The wire code as an [`AuthErrorCode`], when it is one.
    pub fn auth_code(&self) -> Option<AuthErrorCode> {
        match self {
            Self::Auth(error) => error.auth_code(),
            other => other.wire_code().and_then(parse_wire_error_code),
        }
    }
}

/// How a [`ProtocolClient`] authenticates and where it calls.
#[derive(Clone, Default)]
pub struct ProtocolClientOptions {
    /// `publishable_key` for code shipped to browsers (origin-bound, `public`
    /// steps only) or `secret_key` on servers (every client-callable step).
    /// `token`, `token_provider` and `token_endpoint` work too. Without any,
    /// `ARETE_API_KEY` and then the `a4` CLI login supply a secret key.
    pub auth: Option<AuthConfig>,
    /// The key class when it cannot be read from `auth` (tokens, providers,
    /// endpoints). Defaults to publishable.
    pub key_class: Option<ProtocolKeyClass>,
    /// Steps a publishable-key client may prepare; every `public` step when
    /// `None`. Secret-key clients may prepare every step.
    pub steps: Option<Vec<String>>,
    /// The protocol API, `<origin>/api`, instead of the generated one. When
    /// unset, [`ARETE_API_URL_ENV`] (the API origin, e.g.
    /// `https://api.arete.run`) is the default, so one generated client runs
    /// against another API by environment alone. The session endpoint
    /// follows: `<origin>/ws/sessions`. A secret key passed in code may be
    /// sent to this origin; the generated descriptor's URLs never widen
    /// where a key may go.
    pub api_url: Option<String>,
    /// The session endpoint, instead of the one derived from the API URL or
    /// generated.
    pub session_endpoint: Option<String>,
    /// Call this deployment instead of the generated one (advanced: another
    /// deployment of the same release). The generated client already carries
    /// the installed deployment (the dev environment's, while `a4 dev`
    /// generates it).
    pub deployment_id: Option<String>,
    /// The HTTP client to send requests with.
    pub http_client: Option<reqwest::Client>,
    /// Connected stacks whose views instance handles follow when the
    /// definition names a live stack. [`crate::Session::protocol_client`]
    /// sets the session's members.
    pub live_views: Option<Arc<dyn ProtocolLiveViews>>,
}

impl std::fmt::Debug for ProtocolClientOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtocolClientOptions")
            .field("auth", &self.auth)
            .field("key_class", &self.key_class)
            .field("steps", &self.steps)
            .field("api_url", &self.api_url)
            .field("session_endpoint", &self.session_endpoint)
            .field("deployment_id", &self.deployment_id)
            .field("live_views", &self.live_views.is_some())
            .finish_non_exhaustive()
    }
}

impl ProtocolClientOptions {
    pub fn with_auth(mut self, auth: AuthConfig) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Authenticate with a publishable key (`a4_pk_...`).
    pub fn with_publishable_key(self, key: impl Into<String>) -> Self {
        self.with_auth(AuthConfig::default().with_publishable_key(key))
    }

    /// Authenticate with a secret or agent key (`a4_sk_...`, `a4_ak_...`).
    pub fn with_secret_key(self, key: impl Into<String>) -> Self {
        self.with_auth(AuthConfig::default().with_secret_key(key))
    }

    pub fn with_key_class(mut self, key_class: ProtocolKeyClass) -> Self {
        self.key_class = Some(key_class);
        self
    }

    pub fn with_steps<I, S>(mut self, steps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.steps = Some(steps.into_iter().map(Into::into).collect());
        self
    }

    pub fn with_api_url(mut self, api_url: impl Into<String>) -> Self {
        self.api_url = Some(api_url.into());
        self
    }

    pub fn with_session_endpoint(mut self, session_endpoint: impl Into<String>) -> Self {
        self.session_endpoint = Some(session_endpoint.into());
        self
    }

    /// Call `deployment_id` instead of the generated deployment.
    pub fn with_deployment_id(mut self, deployment_id: impl Into<String>) -> Self {
        self.deployment_id = Some(deployment_id.into());
        self
    }

    pub fn with_http_client(mut self, http_client: reqwest::Client) -> Self {
        self.http_client = Some(http_client);
        self
    }

    /// Follow live stack rows from `views`, e.g. a
    /// [`ProtocolStackViews`](super::ProtocolStackViews) of connected stacks.
    pub fn with_live_views(mut self, views: Arc<dyn ProtocolLiveViews>) -> Self {
        self.live_views = Some(views);
        self
    }
}

/// Options of one prepare.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProtocolPrepareOptions {
    /// `v1` (default) or `v0`; `v0` may yield several transactions.
    pub transaction_format: ProtocolTransactionFormat,
    /// Reuse to make a retried prepare return the same intent. Generated
    /// when `None`.
    pub idempotency_key: Option<String>,
    /// Your own request id, recorded with the intent.
    pub request_id: Option<String>,
}

impl ProtocolPrepareOptions {
    pub fn with_transaction_format(mut self, format: ProtocolTransactionFormat) -> Self {
        self.transaction_format = format;
        self
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }
}

/// A wallet-signed transaction to submit: base64 text or raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignedTransaction {
    Base64(String),
    Bytes(Vec<u8>),
}

impl SignedTransaction {
    fn into_base64(self) -> String {
        match self {
            Self::Base64(text) => text,
            Self::Bytes(bytes) => base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }
}

impl From<String> for SignedTransaction {
    fn from(value: String) -> Self {
        Self::Base64(value)
    }
}

impl From<&str> for SignedTransaction {
    fn from(value: &str) -> Self {
        Self::Base64(value.to_string())
    }
}

impl From<Vec<u8>> for SignedTransaction {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<&[u8]> for SignedTransaction {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

/// Filters of an instance list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProtocolInstanceListFilter {
    pub state: Option<String>,
    /// Exact-match filters on `filterable` instance metadata fields.
    pub metadata: Vec<(String, String)>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

impl ProtocolInstanceListFilter {
    pub fn with_state(mut self, state: impl Into<String>) -> Self {
        self.state = Some(state.into());
        self
    }

    pub fn with_metadata(mut self, field: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.push((field.into(), value.into()));
        self
    }

    pub fn with_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }

    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// A read of what one wallet can do on the deployment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProtocolWalletActionsRequest {
    pub wallet: String,
    /// Only this instance.
    pub instance: Option<String>,
    pub cursor: Option<String>,
    /// Instances per page; the API caps it at 50.
    pub limit: Option<u32>,
}

impl ProtocolWalletActionsRequest {
    pub fn new(wallet: impl Into<String>) -> Self {
        Self {
            wallet: wallet.into(),
            ..Self::default()
        }
    }

    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    pub fn with_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }

    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// Options of a metadata update.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProtocolMetadataUpdateOptions {
    /// Update this participant's (wallet's) metadata instead of the instance's.
    pub participant: Option<String>,
    /// Proof for participant-writer fields; see
    /// [`ProtocolClient::sign_metadata_proof`].
    pub wallet_proof: Option<ProtocolMetadataWalletProof>,
}

/// Options of an instance event stream.
#[derive(Debug, Clone)]
pub struct ProtocolEventsOptions {
    /// Resume after this event id.
    pub last_event_id: Option<String>,
    /// First reconnect delay after a failure; doubles per failure.
    pub initial_backoff: Duration,
    /// Cap on the reconnect delay.
    pub max_backoff: Duration,
}

impl Default for ProtocolEventsOptions {
    fn default() -> Self {
        Self {
            last_event_id: None,
            initial_backoff: INITIAL_RECONNECT_DELAY,
            max_backoff: MAX_RECONNECT_DELAY,
        }
    }
}

impl ProtocolEventsOptions {
    pub fn resume_after(mut self, last_event_id: impl Into<String>) -> Self {
        self.last_event_id = Some(last_event_id.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Authenticated HTTP
// ---------------------------------------------------------------------------

struct Http {
    client: reqwest::Client,
    tokens: HttpAuthClient,
    refreshable: bool,
    deployment_id: String,
}

struct Outgoing<'a> {
    method: Method,
    url: String,
    scopes: &'a [String],
    body: Option<Value>,
    headers: HeaderMap,
    force_refresh: bool,
}

impl<'a> Outgoing<'a> {
    fn new(method: Method, url: String, scopes: &'a [String]) -> Self {
        Self {
            method,
            url,
            scopes,
            body: None,
            headers: HeaderMap::new(),
            force_refresh: false,
        }
    }

    fn json(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }
}

enum ReplyBody {
    Pending(reqwest::Response),
    Read(Vec<u8>),
}

struct Reply {
    status: u16,
    header_code: Option<String>,
    body: ReplyBody,
}

impl Reply {
    fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    async fn bytes(self) -> Result<Vec<u8>, ProtocolError> {
        match self.body {
            ReplyBody::Read(bytes) => Ok(bytes),
            ReplyBody::Pending(response) => response
                .bytes()
                .await
                .map(|bytes| bytes.to_vec())
                .map_err(|error| ProtocolError::Transport(error.to_string())),
        }
    }
}

fn is_refresh_code(code: Option<&str>) -> bool {
    code.and_then(parse_wire_error_code)
        .is_some_and(AuthErrorCode::should_refresh_token)
}

fn body_code(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body)
        .ok()?
        .get("code")?
        .as_str()
        .map(str::to_string)
}

impl Http {
    fn token_request(&self, scopes: &[String]) -> AuthTokenRequest {
        AuthTokenRequest::targeted(
            scopes.to_vec(),
            AuthTokenTarget::protocol_deployment(self.deployment_id.clone()),
        )
    }

    async fn attempt(
        &self,
        request: &Outgoing<'_>,
        force_refresh: bool,
    ) -> Result<Reply, ProtocolError> {
        let token = self
            .tokens
            .token(&self.token_request(request.scopes), force_refresh)
            .await?;
        let mut builder = self
            .client
            .request(request.method.clone(), &request.url)
            .headers(request.headers.clone());
        if let Some(token) = token {
            let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
                ProtocolError::InvalidInput("the session token is not a valid header".into())
            })?;
            builder = builder.header(AUTHORIZATION, value);
        }
        if let Some(body) = &request.body {
            builder = builder.json(body);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| ProtocolError::Transport(error.to_string()))?;
        Ok(Reply {
            status: response.status().as_u16(),
            header_code: response
                .headers()
                .get(HEADER_ERROR_CODE)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.trim().to_string()),
            body: ReplyBody::Pending(response),
        })
    }

    /// Send once; on a refreshable authentication failure, re-mint the
    /// session and send exactly once more.
    async fn send(&self, request: &Outgoing<'_>) -> Result<Reply, ProtocolError> {
        let reply = self.attempt(request, request.force_refresh).await?;
        if reply.is_success() || !self.refreshable {
            return Ok(reply);
        }
        let status = reply.status;
        let header_code = reply.header_code.clone();
        let refresh_header = is_refresh_code(header_code.as_deref());
        if !refresh_header && status != 401 {
            return Ok(reply);
        }
        let body = reply.bytes().await?;
        if refresh_header || is_refresh_code(body_code(&body).as_deref()) {
            self.tokens.invalidate(&self.token_request(request.scopes));
            return self.attempt(request, true).await;
        }
        Ok(Reply {
            status,
            header_code,
            body: ReplyBody::Read(body),
        })
    }
}

fn parse_body(bytes: &[u8]) -> Option<Value> {
    if bytes.is_empty() {
        return None;
    }
    Some(
        serde_json::from_slice(bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).into_owned())),
    )
}

fn wire_code_of(header_code: Option<&str>, body: Option<&Value>) -> Option<String> {
    header_code
        .filter(|code| !code.is_empty())
        .map(str::to_string)
        .or_else(|| {
            body.and_then(|body| body.get("code"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn response_error(
    what: &str,
    status: u16,
    header_code: Option<&str>,
    body: Option<Value>,
) -> ProtocolError {
    let message = body
        .as_ref()
        .and_then(|body| {
            body.get("error")
                .and_then(Value::as_str)
                .or_else(|| body.get("message").and_then(Value::as_str))
                .or_else(|| body.as_str().filter(|text| !text.is_empty()))
        })
        .map(str::to_string)
        .unwrap_or_else(|| {
            reqwest::StatusCode::from_u16(status)
                .ok()
                .and_then(|status| status.canonical_reason())
                .unwrap_or("request failed")
                .to_string()
        });
    ProtocolError::Request {
        what: what.to_string(),
        status,
        wire_code: wire_code_of(header_code, body.as_ref()),
        message,
        body,
    }
}

/// `encodeURIComponent`.
fn segment(value: &str, name: &str) -> Result<String, ProtocolError> {
    if value.is_empty() {
        return Err(ProtocolError::InvalidInput(format!(
            "{name} must be a non-empty string"
        )));
    }
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(out)
}

fn access_rejection<G: ProtocolGuardCode>(
    code: ProtocolAccessRejectionCode,
    step: &str,
    detail: Option<Map<String, Value>>,
) -> ProtocolPrepareResult<G> {
    ProtocolPrepareResult::Rejected(ProtocolRejection {
        code: ProtocolRejectionCode::Access(code),
        step: step.to_string(),
        guard: None,
        detail,
    })
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

struct Inner {
    definition: ProtocolDefinition,
    deployment_id: String,
    api_url: String,
    key_class: ProtocolKeyClass,
    http: Http,
    read_scopes: Vec<String>,
    transaction_scopes: Vec<String>,
    live_views: Option<Arc<dyn ProtocolLiveViews>>,
}

/// A client for one protocol deployment.
///
/// Read calls mint a read-only session (`protocol:read`); the first prepare
/// or submit mints a session that can prepare the client's steps and submit.
/// Generated protocol crates wrap it in typed publishable and secret clients
/// whose step methods are exactly the steps the key class can call.
#[derive(Clone)]
pub struct ProtocolClient {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for ProtocolClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtocolClient")
            .field("protocol", &self.inner.definition.name)
            .field("deployment_id", &self.inner.deployment_id)
            .field("api_url", &self.inner.api_url)
            .field("key_class", &self.inner.key_class)
            .finish_non_exhaustive()
    }
}

fn runtime_key_class(
    auth: Option<&AuthConfig>,
    declared: Option<ProtocolKeyClass>,
) -> ProtocolKeyClass {
    if let Some(auth) = auth {
        if auth.secret_key.is_some() {
            return ProtocolKeyClass::Secret;
        }
        if let Some(key) = &auth.publishable_key {
            return if classify_api_key(key) == ApiKeyClass::Secret {
                ProtocolKeyClass::Secret
            } else {
                ProtocolKeyClass::Publishable
            };
        }
    }
    declared.unwrap_or(ProtocolKeyClass::Publishable)
}

/// Environment variable read for the API origin when no `api_url` option is
/// given.
pub const ARETE_API_URL_ENV: &str = "ARETE_API_URL";

/// Where a [`ProtocolClient`] calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolEndpoints {
    pub deployment_id: String,
    pub api_url: String,
    pub session_endpoint: String,
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// The deployment, protocol API and session endpoint: the options first,
/// then [`ARETE_API_URL_ENV`] read through `env`, then the generated
/// descriptor. An `api_url` without a `session_endpoint` carries its
/// origin's session endpoint, so overriding the API never mints sessions
/// elsewhere.
pub fn resolve_protocol_endpoints(
    definition: &ProtocolDefinition,
    options: &ProtocolClientOptions,
    env: impl Fn(&str) -> Option<String>,
) -> ProtocolEndpoints {
    let deployment_id = non_empty(options.deployment_id.as_deref())
        .unwrap_or(definition.deployment.deployment_id)
        .to_string();
    let (api_url, session_endpoint) = if let Some(api) = non_empty(options.api_url.as_deref()) {
        let api = api.trim_end_matches('/');
        let origin = api.strip_suffix("/api").unwrap_or(api);
        (api.to_string(), format!("{origin}/ws/sessions"))
    } else if let Some(origin) = non_empty(env(ARETE_API_URL_ENV).as_deref())
        .map(|origin| origin.trim_end_matches('/'))
        .filter(|origin| !origin.is_empty())
    {
        (format!("{origin}/api"), format!("{origin}/ws/sessions"))
    } else {
        (
            definition
                .deployment
                .api_url
                .trim_end_matches('/')
                .to_string(),
            definition.deployment.session_endpoint.to_string(),
        )
    };
    ProtocolEndpoints {
        deployment_id,
        api_url,
        session_endpoint: non_empty(options.session_endpoint.as_deref())
            .map(str::to_string)
            .unwrap_or(session_endpoint),
    }
}

impl ProtocolClient {
    /// A client for `definition`, a generated `<NAME>_PROTOCOL` constant.
    pub fn new(
        definition: &ProtocolDefinition,
        options: ProtocolClientOptions,
    ) -> Result<Self, ProtocolError> {
        let ProtocolEndpoints {
            deployment_id,
            api_url,
            session_endpoint,
        } = resolve_protocol_endpoints(definition, &options, |name| std::env::var(name).ok());
        if deployment_id.is_empty() {
            return Err(ProtocolError::InvalidInput(
                "the protocol definition names no deployment".into(),
            ));
        }

        let auth = crate::auth::resolve_auth_config(options.auth)?;
        let key_class = runtime_key_class(auth.as_ref(), options.key_class);
        let explicit_publishable = auth
            .as_ref()
            .and_then(|auth| auth.publishable_key.as_deref())
            .is_some_and(|key| classify_api_key(key) != ApiKeyClass::Secret);
        if options.key_class == Some(ProtocolKeyClass::Secret) && explicit_publishable {
            return Err(ProtocolError::Auth(AreteError::InvalidConfig(
                "a publishable key cannot back a secret-key protocol client; pass a secret key \
                 (a4_sk_...) or agent key (a4_ak_...)"
                    .into(),
            )));
        }
        // Keys mint through the generated session endpoint unless the app
        // brought its own token strategy. An `api_url` option is the app's
        // own configuration, so a key passed in code may go to its origin;
        // the generated descriptor's URLs never widen that.
        let explicit_api_url = non_empty(options.api_url.as_deref()).map(str::to_string);
        let auth = auth.map(|mut auth| {
            if auth.token.is_none() && auth.get_token.is_none() && auth.token_endpoint.is_none() {
                auth.token_endpoint = Some(session_endpoint.clone());
            }
            if auth.api_url.is_none() {
                auth.api_url = explicit_api_url;
            }
            auth
        });
        let refreshable = auth
            .as_ref()
            .is_some_and(|auth| auth.get_token.is_some() || auth.token.is_none());
        let client = options.http_client.unwrap_or_default();
        let tokens = HttpAuthClient::new(auth, None, client.clone());

        let read_scopes = vec![PROTOCOL_READ_SCOPE.to_string()];
        let mut transaction_scopes = vec![
            PROTOCOL_READ_SCOPE.to_string(),
            PROTOCOL_SUBMIT_SCOPE.to_string(),
        ];
        match key_class {
            ProtocolKeyClass::Secret => transaction_scopes.push(PROTOCOL_PREPARE_ANY_SCOPE.into()),
            ProtocolKeyClass::Publishable => {
                let steps = options.steps.unwrap_or_else(|| {
                    definition
                        .steps
                        .iter()
                        .filter(|step| step.access == ProtocolStepAccess::Public)
                        .map(|step| step.name.to_string())
                        .collect()
                });
                transaction_scopes.extend(steps.iter().map(|step| protocol_prepare_scope(step)));
            }
        }

        Ok(Self {
            inner: Arc::new(Inner {
                definition: *definition,
                http: Http {
                    client,
                    tokens,
                    refreshable,
                    deployment_id: deployment_id.clone(),
                },
                deployment_id,
                api_url,
                key_class,
                read_scopes,
                transaction_scopes,
                live_views: options.live_views,
            }),
        })
    }

    pub fn definition(&self) -> &ProtocolDefinition {
        &self.inner.definition
    }

    pub fn deployment_id(&self) -> &str {
        &self.inner.deployment_id
    }

    /// The key class this client authenticates with at runtime.
    pub fn key_class(&self) -> ProtocolKeyClass {
        self.inner.key_class
    }

    /// The live views instance handles follow, if the client has any.
    pub fn live_views(&self) -> Option<Arc<dyn ProtocolLiveViews>> {
        self.inner.live_views.clone()
    }

    /// The scopes a prepare or submit session asks for.
    pub fn transaction_scopes(&self) -> &[String] {
        &self.inner.transaction_scopes
    }

    fn instance_url(&self, id: &str) -> Result<String, ProtocolError> {
        Ok(format!(
            "{}/protocol-instances/{}",
            self.inner.api_url,
            segment(id, "instance")?
        ))
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        what: &str,
        url: String,
    ) -> Result<T, ProtocolError> {
        let request = Outgoing::new(Method::GET, url, &self.inner.read_scopes);
        self.complete(what, request).await
    }

    async fn complete<T: DeserializeOwned>(
        &self,
        what: &str,
        request: Outgoing<'_>,
    ) -> Result<T, ProtocolError> {
        let reply = self.inner.http.send(&request).await?;
        let status = reply.status;
        let success = reply.is_success();
        let header_code = reply.header_code.clone();
        let bytes = reply.bytes().await?;
        if !success {
            return Err(response_error(
                what,
                status,
                header_code.as_deref(),
                parse_body(&bytes),
            ));
        }
        serde_json::from_slice(&bytes).map_err(|error| {
            ProtocolError::UnexpectedResponse(format!(
                "{what} returned an unexpected body: {error}"
            ))
        })
    }

    /// Ask the hosted API to build a step's transactions.
    ///
    /// `instance` is required for `instance` and `participant` steps and
    /// ignored for `create` steps. `args` must serialize to a JSON object.
    /// Resolves with the execution intent and the partially signed
    /// transactions for the wallet to sign, or with the rejection (a guard
    /// code or an access code).
    pub async fn prepare<G: ProtocolGuardCode, A: Serialize + ?Sized>(
        &self,
        step_name: &str,
        instance: Option<&str>,
        wallet: &str,
        args: &A,
        options: ProtocolPrepareOptions,
    ) -> Result<ProtocolPrepareResult<G>, ProtocolError> {
        let Some(step) = self.inner.definition.step(step_name).copied() else {
            return Ok(access_rejection(
                ProtocolAccessRejectionCode::StepNotClientCallable,
                step_name,
                None,
            ));
        };
        if self.inner.key_class == ProtocolKeyClass::Publishable
            && step.access == ProtocolStepAccess::Secret
        {
            return Ok(access_rejection(
                ProtocolAccessRejectionCode::StepRequiresSecretKey,
                step_name,
                None,
            ));
        }
        if wallet.is_empty() {
            return Err(ProtocolError::InvalidInput(format!(
                "{step_name}.prepare requires a wallet"
            )));
        }
        let args = serde_json::to_value(args)
            .map_err(|error| ProtocolError::InvalidInput(format!("{step_name} args: {error}")))?;
        let args = match args {
            Value::Object(args) => args,
            Value::Null => Map::new(),
            _ => {
                return Err(ProtocolError::InvalidInput(format!(
                    "{step_name} args must be an object"
                )))
            }
        };
        let url = if step.scope == ProtocolStepScope::Create {
            format!(
                "{}/protocol-deployments/{}/steps/{}/prepare",
                self.inner.api_url,
                segment(&self.inner.deployment_id, "deploymentId")?,
                segment(step_name, "step")?
            )
        } else {
            let instance = instance.filter(|id| !id.is_empty()).ok_or_else(|| {
                ProtocolError::InvalidInput(format!("{step_name}.prepare requires an instance id"))
            })?;
            format!(
                "{}/steps/{}/prepare",
                self.instance_url(instance)?,
                segment(step_name, "step")?
            )
        };
        let mut body = json!({
            "idempotencyKey": options
                .idempotency_key
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            "wallet": wallet,
            "args": args,
            "transactionFormat": options.transaction_format,
        });
        if let Some(request_id) = options.request_id {
            body["clientContext"] = json!({ "requestId": request_id });
        }

        let request = Outgoing::new(Method::POST, url, &self.inner.transaction_scopes).json(body);
        let reply = match self.inner.http.send(&request).await {
            Ok(reply) => reply,
            Err(ProtocolError::Auth(error)) => {
                // The session mint refuses a step the key cannot prepare.
                if let AreteError::AuthRequestFailed {
                    problem: Some(problem),
                    ..
                } = &error
                {
                    if let Some(code) = problem
                        .code
                        .as_deref()
                        .and_then(ProtocolAccessRejectionCode::from_wire)
                    {
                        return Ok(access_rejection(code, step_name, None));
                    }
                }
                return Err(ProtocolError::Auth(error));
            }
            Err(error) => return Err(error),
        };
        let status = reply.status;
        let success = reply.is_success();
        let header_code = reply.header_code.clone();
        let bytes = reply.bytes().await?;
        let body = parse_body(&bytes);
        if success {
            let prepared = body
                .clone()
                .ok_or_else(|| ProtocolError::UnexpectedResponse("Prepare returned no body".into()))
                .and_then(|body| {
                    serde_json::from_value::<ProtocolPrepared>(body).map_err(|error| {
                        ProtocolError::UnexpectedResponse(format!(
                            "Prepare returned an unexpected body: {error}"
                        ))
                    })
                })?;
            return Ok(ProtocolPrepareResult::Prepared(prepared));
        }
        if status == 422 {
            if let Some(rejected) = body
                .as_ref()
                .and_then(|body| body.get("rejected"))
                .and_then(Value::as_str)
            {
                let body = body.as_ref().expect("checked above");
                return Ok(ProtocolPrepareResult::Rejected(ProtocolRejection {
                    code: ProtocolRejectionCode::from_wire(rejected),
                    step: body
                        .get("step")
                        .and_then(Value::as_str)
                        .unwrap_or(step_name)
                        .to_string(),
                    guard: body
                        .get("guard")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    detail: body.get("detail").and_then(Value::as_object).cloned(),
                }));
            }
        }
        if status == 403 {
            if let Some(code) = wire_code_of(header_code.as_deref(), body.as_ref())
                .as_deref()
                .and_then(ProtocolAccessRejectionCode::from_wire)
            {
                let detail = body
                    .as_ref()
                    .and_then(|body| body.get("detail"))
                    .and_then(Value::as_object)
                    .cloned();
                return Ok(access_rejection(code, step_name, detail));
            }
        }
        Err(response_error(
            &format!("Prepare of step '{step_name}'"),
            status,
            header_code.as_deref(),
            body,
        ))
    }

    /// Submit the wallet-signed transactions of a prepared intent.
    ///
    /// The hosted API refuses transactions whose bytes differ from what it
    /// prepared. When the result is
    /// [`ProtocolIntentStatus::SubmittedUnknown`], never sign or submit the
    /// step again: read the intent with [`Self::intent`] until it settles.
    pub async fn submit<I, T>(
        &self,
        execution_id: &str,
        signed_transactions: I,
    ) -> Result<ProtocolIntentState, ProtocolError>
    where
        I: IntoIterator<Item = T>,
        T: Into<SignedTransaction>,
    {
        let encoded = signed_transactions
            .into_iter()
            .map(|transaction| transaction.into().into_base64())
            .collect::<Vec<_>>();
        if encoded.is_empty() {
            return Err(ProtocolError::InvalidInput(
                "submit requires the signed transactions".into(),
            ));
        }
        let url = format!(
            "{}/execution-intents/{}/submit",
            self.inner.api_url,
            segment(execution_id, "executionId")?
        );
        let request = Outgoing::new(Method::POST, url, &self.inner.transaction_scopes)
            .json(json!({ "signedTransactions": encoded }));
        self.complete("Submit", request).await
    }

    /// Squads onboarding for the calling account: its proposer and its
    /// membership status. Arete builds no Squads transactions; the team adds
    /// the proposer on the dashboard, in the Squads app, or from code with
    /// the squads-v4 program SDK.
    pub fn squads(&self) -> ProtocolSquads<'_> {
        ProtocolSquads { client: self }
    }

    fn squads_url(&self, route: &str) -> Result<String, ProtocolError> {
        Ok(format!(
            "{}/protocol-deployments/{}/squads/{route}",
            self.inner.api_url,
            segment(&self.inner.deployment_id, "deploymentId")?
        ))
    }

    /// Read an execution intent's state.
    pub async fn intent(&self, execution_id: &str) -> Result<ProtocolIntentState, ProtocolError> {
        let url = format!(
            "{}/execution-intents/{}",
            self.inner.api_url,
            segment(execution_id, "executionId")?
        );
        self.get_json("Execution intent read", url).await
    }

    /// Fetch the signed execution receipt of a transaction this deployment
    /// landed. Check it offline with [`super::verify_receipt`] against
    /// [`Self::receipt_keys`], and against the transaction itself with
    /// [`super::verify_receipt_on_chain`]. Answers 404
    /// `protocol-receipt-not-found` for a signature with no receipt.
    pub async fn get_receipt(
        &self,
        signature: &str,
    ) -> Result<super::ProtocolExecutionReceipt, ProtocolError> {
        let url = format!(
            "{}/protocol-receipts/{}",
            self.inner.api_url,
            segment(signature, "signature")?
        );
        self.get_json("Protocol receipt read", url).await
    }

    /// The published receipt keys, including retired ones.
    pub async fn receipt_keys(&self) -> Result<super::ProtocolReceiptKeySet, ProtocolError> {
        let url = format!("{}/protocol-receipts/keys", self.inner.api_url);
        self.get_json("Protocol receipt keys read", url).await
    }

    /// List the deployment's instances.
    pub async fn list_instances(
        &self,
        filter: ProtocolInstanceListFilter,
    ) -> Result<ProtocolInstancePage, ProtocolError> {
        let mut url = url::Url::parse(&format!(
            "{}/protocol-deployments/{}/instances",
            self.inner.api_url,
            segment(&self.inner.deployment_id, "deploymentId")?
        ))
        .map_err(|error| ProtocolError::InvalidInput(format!("invalid API URL: {error}")))?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(state) = &filter.state {
                query.append_pair("state", state);
            }
            if let Some(cursor) = &filter.cursor {
                query.append_pair("cursor", cursor);
            }
            if let Some(limit) = filter.limit {
                query.append_pair("limit", &limit.to_string());
            }
            for (field, value) in &filter.metadata {
                let filterable = self
                    .inner
                    .definition
                    .instance_metadata
                    .iter()
                    .any(|declared| declared.name == field && declared.filterable);
                if !filterable {
                    return Err(ProtocolError::InvalidInput(format!(
                        "Instance metadata field '{field}' is not filterable"
                    )));
                }
                query.append_pair(&format!("metadata.{field}"), value);
            }
        }
        let mut url = url.to_string();
        if url.ends_with('?') {
            url.pop();
        }
        self.get_json("Protocol instance list", url).await
    }

    /// Read one instance. With `wallet`, `allowed_steps` is evaluated for it.
    pub async fn instance(
        &self,
        id: &str,
        wallet: Option<&str>,
    ) -> Result<ProtocolInstanceDetail, ProtocolError> {
        let mut url = self.instance_url(id)?;
        if let Some(wallet) = wallet.filter(|wallet| !wallet.is_empty()) {
            url.push_str(&format!("?wallet={}", segment(wallet, "wallet")?));
        }
        self.get_json("Protocol instance read", url).await
    }

    /// Read one participant of an instance.
    pub async fn participant(
        &self,
        instance: &str,
        wallet: &str,
    ) -> Result<ProtocolParticipantDetail, ProtocolError> {
        let url = format!(
            "{}/participants/{}",
            self.instance_url(instance)?,
            segment(wallet, "wallet")?
        );
        self.get_json("Protocol participant read", url).await
    }

    /// Which steps `wallet` can call on `instance` now, and why not.
    pub async fn allowed_steps(
        &self,
        instance: &str,
        wallet: &str,
    ) -> Result<Vec<ProtocolStepAvailability>, ProtocolError> {
        segment(wallet, "wallet")?;
        Ok(self.instance(instance, Some(wallet)).await?.allowed_steps)
    }

    /// What `request.wallet` can do on the deployment: the `create` steps it
    /// can call, and per instance it is related to (participant, creator or
    /// role holder) the steps it can call there, and why not. Paged by
    /// `cursor`; `instance` narrows the read to one instance.
    pub async fn actions(
        &self,
        request: ProtocolWalletActionsRequest,
    ) -> Result<ProtocolWalletActions, ProtocolError> {
        let mut url = url::Url::parse(&format!(
            "{}/protocol-deployments/{}/wallets/{}/actions",
            self.inner.api_url,
            segment(&self.inner.deployment_id, "deploymentId")?,
            segment(&request.wallet, "wallet")?
        ))
        .map_err(|error| ProtocolError::InvalidInput(format!("invalid API URL: {error}")))?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(instance) = request.instance.as_deref().filter(|id| !id.is_empty()) {
                query.append_pair("instance", instance);
            }
            if let Some(cursor) = &request.cursor {
                query.append_pair("cursor", cursor);
            }
            if let Some(limit) = request.limit {
                query.append_pair("limit", &limit.to_string());
            }
        }
        let mut url = url.to_string();
        if url.ends_with('?') {
            url.pop();
        }
        self.get_json("Protocol wallet actions read", url).await
    }

    /// An instance's metadata document and version.
    pub async fn instance_metadata(
        &self,
        instance: &str,
    ) -> Result<ProtocolMetadataState, ProtocolError> {
        let detail = self.instance(instance, None).await?;
        Ok(ProtocolMetadataState {
            metadata: detail.metadata,
            version: detail.metadata_version,
        })
    }

    /// A participant's metadata document.
    pub async fn participant_metadata(
        &self,
        instance: &str,
        wallet: &str,
    ) -> Result<ProtocolMetadataState, ProtocolError> {
        let detail = self.participant(instance, wallet).await?;
        Ok(ProtocolMetadataState {
            metadata: detail.metadata,
            version: None,
        })
    }

    /// Apply a JSON merge patch (`null` removes a field) to an instance's
    /// metadata, or a participant's with `options.participant`.
    pub async fn update_metadata(
        &self,
        instance: &str,
        patch: Map<String, Value>,
        options: ProtocolMetadataUpdateOptions,
    ) -> Result<ProtocolMetadataUpdateResult, ProtocolError> {
        let mut body = json!({ "patch": patch });
        if let Some(participant) = options.participant {
            body["participant"] = json!(participant);
        }
        if let Some(proof) = options.wallet_proof {
            body["proof"] = serde_json::to_value(proof)
                .map_err(|error| ProtocolError::InvalidInput(error.to_string()))?;
        }
        let url = format!("{}/metadata", self.instance_url(instance)?);
        let request = Outgoing::new(Method::PATCH, url, &self.inner.read_scopes).json(body);
        self.complete("Metadata update", request).await
    }

    /// The message `wallet` signs to authorise `patch` on `instance`.
    /// `expires_at` defaults to four minutes from now.
    pub fn metadata_proof_message(
        &self,
        instance: &str,
        wallet: &str,
        patch: &Map<String, Value>,
        expires_at: Option<u64>,
    ) -> Result<ProtocolMetadataProofMessage, ProtocolError> {
        let now = now_unix_seconds();
        Ok(protocol_metadata_proof_message(
            &ProtocolMetadataProofInput {
                deployment_id: &self.inner.deployment_id,
                instance_id: instance,
                wallet,
                patch,
                expires_at: expires_at.unwrap_or(now + PROTOCOL_METADATA_PROOF_DEFAULT_TTL_SECONDS),
                now: Some(now),
            },
        )?)
    }

    /// Build the proof message, have `sign` sign it with the wallet's Ed25519
    /// key (returning the 64-byte signature), and return the `proof` to pass
    /// in [`ProtocolMetadataUpdateOptions::wallet_proof`].
    pub async fn sign_metadata_proof<F, Fut, E>(
        &self,
        instance: &str,
        wallet: &str,
        patch: &Map<String, Value>,
        expires_at: Option<u64>,
        sign: F,
    ) -> Result<ProtocolMetadataWalletProof, SignMetadataProofError<E>>
    where
        F: FnOnce(Vec<u8>) -> Fut,
        Fut: std::future::Future<Output = Result<Vec<u8>, E>>,
    {
        let now = now_unix_seconds();
        sign_protocol_metadata_proof(
            &ProtocolMetadataProofInput {
                deployment_id: &self.inner.deployment_id,
                instance_id: instance,
                wallet,
                patch,
                expires_at: expires_at.unwrap_or(now + PROTOCOL_METADATA_PROOF_DEFAULT_TTL_SECONDS),
                now: Some(now),
            },
            sign,
        )
        .await
    }

    /// Follow an instance's event stream. Reconnects and resumes after the
    /// last event id transparently; stream failures come through as `Err`
    /// items and the stream keeps going, unless the hosted API refuses it
    /// outright (a 4xx other than 408 and 429), which ends it after that
    /// error. Dropping the stream stops it. Needs a Tokio runtime.
    pub fn events(&self, instance_id: &str, options: ProtocolEventsOptions) -> ProtocolEventStream {
        let (sender, receiver) = mpsc::channel(64);
        let last_event_id = Arc::new(Mutex::new(options.last_event_id.clone()));
        let url = self
            .instance_url(instance_id)
            .map(|url| format!("{url}/events"));
        let client = self.clone();
        let shared = last_event_id.clone();
        let task = tokio::spawn(async move {
            match url {
                Ok(url) => client.run_events(url, options, shared, sender).await,
                Err(error) => {
                    let _ = sender.send(Err(error)).await;
                }
            }
        });
        ProtocolEventStream {
            receiver,
            last_event_id,
            task,
        }
    }

    async fn run_events(
        &self,
        url: String,
        options: ProtocolEventsOptions,
        last_event_id: Arc<Mutex<Option<String>>>,
        sender: mpsc::Sender<Result<ProtocolStreamEvent, ProtocolError>>,
    ) {
        let current_id = || {
            last_event_id
                .lock()
                .expect("event id lock poisoned")
                .clone()
        };
        let backoff = |failures: u32| {
            options
                .initial_backoff
                .saturating_mul(2u32.saturating_pow(failures.saturating_sub(1)))
                .min(options.max_backoff)
        };
        let mut failures = 0u32;
        let mut force_refresh = false;
        loop {
            let mut request = Outgoing::new(Method::GET, url.clone(), &self.inner.read_scopes);
            request
                .headers
                .insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
            request
                .headers
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            if let Some(id) = current_id() {
                if let Ok(value) = HeaderValue::from_str(&id) {
                    request
                        .headers
                        .insert(HeaderName::from_static("last-event-id"), value);
                }
            }
            request.force_refresh = force_refresh;
            let reply = match self.inner.http.send(&request).await {
                Ok(reply) => reply,
                Err(error) => {
                    if sender.send(Err(error)).await.is_err() {
                        return;
                    }
                    failures += 1;
                    tokio::time::sleep(backoff(failures)).await;
                    continue;
                }
            };
            force_refresh = false;
            if !reply.is_success() {
                let status = reply.status;
                let header_code = reply.header_code.clone();
                let body = reply
                    .bytes()
                    .await
                    .ok()
                    .and_then(|bytes| parse_body(&bytes));
                let error = response_error("Event stream", status, header_code.as_deref(), body);
                let fatal = (400..500).contains(&status) && status != 408 && status != 429;
                if sender.send(Err(error)).await.is_err() || fatal {
                    return;
                }
                failures += 1;
                tokio::time::sleep(backoff(failures)).await;
                continue;
            }

            let mut parser = ServerSentEventParser::new(current_id());
            failures = 0;
            let mut response = match reply.body {
                ReplyBody::Pending(response) => Some(response),
                ReplyBody::Read(bytes) => {
                    for event in parser.push(&bytes) {
                        if !self
                            .dispatch_event(event, &last_event_id, &sender, &mut force_refresh)
                            .await
                        {
                            return;
                        }
                    }
                    None
                }
            };
            let mut failed = false;
            while let Some(stream) = response.as_mut() {
                match stream.chunk().await {
                    Ok(Some(chunk)) => {
                        for event in parser.push(&chunk) {
                            if !self
                                .dispatch_event(event, &last_event_id, &sender, &mut force_refresh)
                                .await
                            {
                                return;
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        if sender
                            .send(Err(ProtocolError::Transport(error.to_string())))
                            .await
                            .is_err()
                        {
                            return;
                        }
                        failed = true;
                        break;
                    }
                }
            }
            parser.end();
            if failed {
                failures += 1;
                tokio::time::sleep(backoff(failures)).await;
            } else {
                // The server ends streams periodically and when the session
                // expires: reconnect after the last id.
                tokio::time::sleep(Duration::from_millis(parser.retry_ms.unwrap_or(0))).await;
            }
        }
    }

    /// Forward one SSE event. Returns `false` once the receiver is gone.
    async fn dispatch_event(
        &self,
        event: ServerSentEvent,
        last_event_id: &Mutex<Option<String>>,
        sender: &mpsc::Sender<Result<ProtocolStreamEvent, ProtocolError>>,
        force_refresh: &mut bool,
    ) -> bool {
        if let Some(id) = &event.id {
            *last_event_id.lock().expect("event id lock poisoned") = Some(id.clone());
        }
        let item = match stream_item(event) {
            StreamItem::Event(event) => Ok(event),
            StreamItem::Refresh => {
                *force_refresh = true;
                return true;
            }
            StreamItem::Error(error) => Err(error),
            StreamItem::Ignored => return true,
        };
        sender.send(item).await.is_ok()
    }
}

enum StreamItem {
    Event(ProtocolStreamEvent),
    Error(ProtocolError),
    Refresh,
    Ignored,
}

fn stream_item(event: ServerSentEvent) -> StreamItem {
    let data = match serde_json::from_str::<Value>(&event.data) {
        Ok(data) => data,
        Err(_) => {
            return StreamItem::Error(ProtocolError::UnexpectedResponse(format!(
                "Event stream sent invalid JSON for '{}'",
                event.event
            )))
        }
    };
    let id = event.id.unwrap_or_default();
    match (event.event.as_str(), data) {
        ("error", data) => {
            let code = data
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            if is_refresh_code(Some(&code)) || code.to_ascii_lowercase().contains("expired") {
                StreamItem::Refresh
            } else {
                StreamItem::Error(ProtocolError::Stream { code })
            }
        }
        ("emit", data) => match serde_json::from_value::<ProtocolEmitEventData>(data) {
            Ok(data) => StreamItem::Event(ProtocolStreamEvent::Emit { id, data }),
            Err(error) => StreamItem::Error(ProtocolError::UnexpectedResponse(format!(
                "Event stream sent an invalid emit event: {error}"
            ))),
        },
        (SQUADS_PROPOSER_MISSING_EVENT, data) => {
            match serde_json::from_value::<ProtocolSquadsProposerMissingData>(data) {
                Ok(data) => {
                    StreamItem::Event(ProtocolStreamEvent::SquadsProposerMissing { id, data })
                }
                Err(error) => StreamItem::Error(ProtocolError::UnexpectedResponse(format!(
                    "Event stream sent an invalid {SQUADS_PROPOSER_MISSING_EVENT} event: {error}"
                ))),
            }
        }
        ("transition", Value::Object(data)) => {
            StreamItem::Event(ProtocolStreamEvent::Transition { id, data })
        }
        ("participant", Value::Object(data)) => {
            StreamItem::Event(ProtocolStreamEvent::Participant { id, data })
        }
        ("observation", Value::Object(data)) => {
            StreamItem::Event(ProtocolStreamEvent::Observation { id, data })
        }
        _ => StreamItem::Ignored,
    }
}

/// Squads onboarding from [`ProtocolClient::squads`]. Admin steps are
/// proposed to a team's Squads v4 multisig by a proposer key that belongs to
/// the calling account and may only initiate; Arete never votes or executes.
#[derive(Debug, Clone, Copy)]
pub struct ProtocolSquads<'a> {
    client: &'a ProtocolClient,
}

impl ProtocolSquads<'_> {
    /// The calling account's proposer and the permission it needs.
    pub async fn proposer(&self) -> Result<ProtocolSquadsProposer, ProtocolError> {
        let url = self.client.squads_url("proposer")?;
        self.client.get_json("Squads proposer read", url).await
    }

    /// Whether the proposer is an Initiate-only member of `multisig`.
    pub async fn status(&self, multisig: &str) -> Result<ProtocolSquadsStatus, ProtocolError> {
        let url = format!(
            "{}?multisig={}",
            self.client.squads_url("status")?,
            segment(multisig, "multisig")?
        );
        self.client.get_json("Squads proposer status", url).await
    }
}

/// An instance's event stream from [`ProtocolClient::events`].
pub struct ProtocolEventStream {
    receiver: mpsc::Receiver<Result<ProtocolStreamEvent, ProtocolError>>,
    last_event_id: Arc<Mutex<Option<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl ProtocolEventStream {
    /// The id of the last event received; reconnects resume after it.
    pub fn last_event_id(&self) -> Option<String> {
        self.last_event_id
            .lock()
            .expect("event id lock poisoned")
            .clone()
    }

    /// Stop following the stream.
    pub fn close(&mut self) {
        self.task.abort();
        self.receiver.close();
    }
}

impl std::fmt::Debug for ProtocolEventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtocolEventStream")
            .field("last_event_id", &self.last_event_id())
            .finish_non_exhaustive()
    }
}

impl Stream for ProtocolEventStream {
    type Item = Result<ProtocolStreamEvent, ProtocolError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}

impl Drop for ProtocolEventStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}
