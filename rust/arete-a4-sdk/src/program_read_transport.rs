//! `program-read-http/v1` transport.
//!
//! Port of `typescript/core/src/program-read-transport.ts`; wire fixtures in
//! `typescript/core/src/program-read-contract-v1.fixture.json`.
//!
//! - `GET  <endpoint>/v1/releases/<release>/accounts/<Account>/<address>` —
//!   fetch one account (`null` body means missing).
//! - `GET  …/<address>/exists` — `{"exists": bool}`.
//! - `POST …/accounts/<Account>` with `{"addresses": […]}` — batched fetch
//!   returning per-address `ok`/`missing`/`error` items.
//!
//! Release hashes are `encodeURIComponent`-encoded with `%3A` restored to
//! `:`. `X-Error-Code` headers win over `{"error":{"code"}}`/`{"code"}` body
//! codes. Only an HTTP 401 with a refresh-worthy code triggers a targeted
//! token invalidation and a single replay.

use std::sync::Arc;

use serde_json::Value;

use crate::error::AuthErrorCode;
use crate::read::{ProgramReadBinding, ProgramReleaseReference, ReadError, ReadRequestError};

/// Contract identifier for the release-addressed program read HTTP surface.
pub const PROGRAM_READ_CONTRACT_VERSION: &str = "program-read-http/v1";

const READ_SCOPES: &[&str] = &["read"];

/// Targeted token descriptor forwarded to a [`BearerTokenSource`]
/// (TS `ProgramReadBindingAuthTarget`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadAuthTarget {
    /// `"program-read-binding"` for program read transports.
    pub target_kind: String,
    pub target_id: String,
    pub program_release_hash: Option<String>,
}

/// Source of `Authorization: Bearer …` tokens for HTTP reads.
///
/// A later integration pass adapts the shared `HttpAuthClient` token
/// machinery to this trait; tests may provide mock implementations.
#[async_trait::async_trait]
pub trait BearerTokenSource: Send + Sync {
    /// Return a bearer token for the given scopes and optional target, or
    /// `None` when the request should go out unauthenticated.
    async fn bearer_token(
        &self,
        scopes: &[&str],
        target: Option<&ReadAuthTarget>,
        force_refresh: bool,
    ) -> Result<Option<String>, crate::error::AreteError>;

    /// Drop any cached token for the given scopes/target.
    fn invalidate(&self, scopes: &[&str], target: Option<&ReadAuthTarget>);
}

/// One program read operation (TS `ProgramReadRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramReadRequest<'a> {
    Fetch {
        account: &'a str,
        address: &'a str,
    },
    FetchMany {
        account: &'a str,
        addresses: &'a [&'a str],
    },
    Exists {
        account: &'a str,
        address: &'a str,
    },
}

impl ProgramReadRequest<'_> {
    fn account(&self) -> &str {
        match self {
            Self::Fetch { account, .. }
            | Self::FetchMany { account, .. }
            | Self::Exists { account, .. } => account,
        }
    }
}

/// Release-addressed program read transport (`program-read-http/v1`), for
/// both local HTTP endpoints and hosted bindings.
#[derive(Clone)]
pub struct ProgramReadTransport {
    endpoint: String,
    release: ProgramReleaseReference,
    auth: Option<Arc<dyn BearerTokenSource>>,
    auth_target: Option<ReadAuthTarget>,
    http: reqwest::Client,
}

impl ProgramReadTransport {
    pub fn new(
        endpoint: impl Into<String>,
        release: ProgramReleaseReference,
        auth: Option<Arc<dyn BearerTokenSource>>,
        auth_target: Option<ReadAuthTarget>,
        http: reqwest::Client,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            release,
            auth,
            auth_target,
            http,
        }
    }

    /// Local HTTP transport addressing the connect-time HTTP endpoint
    /// (TS `kind: 'local-http'`). Local reads carry no auth.
    pub fn local_http(
        connect_http_url: impl Into<String>,
        release: ProgramReleaseReference,
        http: reqwest::Client,
    ) -> Self {
        Self::new(connect_http_url, release, None, None, http)
    }

    /// Hosted binding transport addressing the generated binding endpoint
    /// (TS `kind: 'hosted-binding'`). The auth target is derived from the
    /// binding; pass `auth: None` to skip auth entirely (e.g. when the
    /// binding metadata says `required == false` and no runtime auth is
    /// configured — TS `hostedAuthConfig`).
    pub fn hosted(
        binding: &ProgramReadBinding,
        release: ProgramReleaseReference,
        auth: Option<Arc<dyn BearerTokenSource>>,
        http: reqwest::Client,
    ) -> Self {
        let auth_target = auth.as_ref().map(|_| ReadAuthTarget {
            target_kind: "program-read-binding".to_string(),
            target_id: binding.program_read_binding_id.clone(),
            program_release_hash: Some(release.program_release_hash.clone()),
        });
        Self::new(binding.endpoint.clone(), release, auth, auth_target, http)
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn release(&self) -> &ProgramReleaseReference {
        &self.release
    }

    /// Release-addressed request path (TS `requestPath`): the release hash is
    /// `encodeURIComponent`-encoded with `%3A` restored to `:` so typed
    /// hashes (`arete:h1:…`) stay readable.
    pub fn request_path(&self, request: &ProgramReadRequest<'_>) -> String {
        let release_hash =
            encode_uri_component(&self.release.program_release_hash).replace("%3A", ":");
        let root = format!(
            "/v1/releases/{release_hash}/accounts/{}",
            encode_uri_component(request.account())
        );
        match request {
            ProgramReadRequest::FetchMany { .. } => root,
            ProgramReadRequest::Fetch { address, .. } => {
                format!(
                    "{}/{}",
                    root.trim_end_matches('/'),
                    encode_uri_component(address)
                )
            }
            ProgramReadRequest::Exists { address, .. } => {
                format!(
                    "{}/{}/exists",
                    root.trim_end_matches('/'),
                    encode_uri_component(address)
                )
            }
        }
    }

    /// Execute one read and return the raw JSON wire value (a `null` body for
    /// a fetch means the account is missing).
    ///
    /// Errors: non-2xx responses become [`ReadError::Request`] with the
    /// `X-Error-Code` header (or body error code) preserved; a 2xx response
    /// with an invalid JSON body becomes [`ReadError::InvalidResponse`]. An
    /// HTTP 401 carrying a refresh-worthy code triggers exactly one targeted
    /// token invalidation and replay when an auth source and target are
    /// configured.
    pub async fn read(&self, request: &ProgramReadRequest<'_>) -> Result<Value, ReadError> {
        let path = self.request_path(request);
        let url = append_url(&self.endpoint, &path);

        let (mut status, mut header_code, mut body) =
            self.attempt(&url, &path, request, false).await?;
        let wire_code = header_code.clone().or_else(|| body_error_code(&body));
        let refreshable = wire_code
            .as_deref()
            .map(is_refreshable_error_code)
            .unwrap_or(false);
        if status == 401 && refreshable && self.auth_target.is_some() {
            if let Some(auth) = &self.auth {
                auth.invalidate(READ_SCOPES, self.auth_target.as_ref());
                (status, header_code, body) = self.attempt(&url, &path, request, true).await?;
            }
        }

        if !(200..300).contains(&status) {
            let server_error_code = header_code.or_else(|| body_error_code(&body));
            return Err(ReadRequestError {
                status,
                path,
                body,
                server_error_code,
            }
            .into());
        }
        serde_json::from_str(&body).map_err(|source| ReadError::InvalidResponse { path, source })
    }

    async fn attempt(
        &self,
        url: &str,
        path: &str,
        request: &ProgramReadRequest<'_>,
        force_refresh: bool,
    ) -> Result<(u16, Option<String>, String), ReadError> {
        let token = match &self.auth {
            Some(auth) => {
                auth.bearer_token(READ_SCOPES, self.auth_target.as_ref(), force_refresh)
                    .await?
            }
            None => None,
        };
        let mut builder = match request {
            ProgramReadRequest::FetchMany { addresses, .. } => self
                .http
                .post(url)
                .json(&serde_json::json!({ "addresses": addresses })),
            _ => self.http.get(url),
        };
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let response = builder.send().await.map_err(|source| ReadError::Network {
            path: path.to_string(),
            source,
        })?;
        let status = response.status().as_u16();
        let header_code = response
            .headers()
            .get("X-Error-Code")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = response.text().await.map_err(|source| ReadError::Network {
            path: path.to_string(),
            source,
        })?;
        Ok((status, header_code, body))
    }
}

/// `<base>/<path>` with duplicate slashes collapsed at the join (TS
/// `appendUrl`).
fn append_url(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// JavaScript `encodeURIComponent`: everything except
/// `A-Z a-z 0-9 - _ . ! ~ * ' ( )` is percent-encoded as UTF-8 bytes.
fn encode_uri_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(*byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Body-level error code: `{"error":{"code"}}` wins over top-level
/// `{"code"}` (TS `responseErrorCode`, body half).
fn body_error_code(body: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    if let Some(code) = parsed
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
    {
        return Some(code.to_string());
    }
    parsed
        .get("code")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// TS `isRefreshableErrorCode`: normalize `_` to `-`, parse as a wire auth
/// error code, and ask whether it warrants a token refresh.
pub(crate) fn is_refreshable_error_code(code: &str) -> bool {
    AuthErrorCode::from_wire(&code.trim().replace('_', "-"))
        .map(AuthErrorCode::should_refresh_token)
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Test support (shared with `read.rs` unit tests)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_support {
    use super::{BearerTokenSource, ReadAuthTarget};
    use crate::error::AreteError;
    use crate::read::{HttpAuthMetadata, ProgramReadBinding, ProgramReleaseReference};
    use axum::body::{to_bytes, Body};
    use axum::extract::{Request, State};
    use axum::response::Response;
    use axum::Router;
    use serde_json::Value;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// Copy of `typescript/core/src/program-read-contract-v1.fixture.json`.
    pub(crate) const PROGRAM_READ_CONTRACT_V1_FIXTURE: &str = r#"{
  "contractVersion": "program-read-http/v1",
  "success": {
    "rawValue": {
      "value": "decoded",
      "count": 7
    },
    "missing": null,
    "exists": {
      "exists": true
    },
    "batch": {
      "items": [
        {
          "address": "present",
          "status": "ok",
          "value": {
            "value": "decoded",
            "count": 7
          }
        },
        {
          "address": "missing",
          "status": "missing"
        },
        {
          "address": "broken",
          "status": "error",
          "error": {
            "code": "ACCOUNT_DECODE_FAILED"
          }
        }
      ]
    }
  },
  "errors": {
    "nested": {
      "error": {
        "code": "ACCOUNT_DECODE_FAILED"
      }
    },
    "refreshable": {
      "error": {
        "code": "TOKEN_EXPIRED"
      }
    },
    "nonRefreshable": {
      "error": {
        "code": "AUTH_REQUIRED"
      }
    }
  }
}"#;

    pub(crate) fn fixture() -> Value {
        serde_json::from_str(PROGRAM_READ_CONTRACT_V1_FIXTURE).expect("fixture parses")
    }

    pub(crate) const TEST_BINDING_ID: &str = "prb_00000000000000000000000000000001";

    pub(crate) fn test_release() -> ProgramReleaseReference {
        ProgramReleaseReference {
            program_release_hash: "release-alpha".into(),
            program_spec_hash: "spec-alpha".into(),
        }
    }

    pub(crate) fn test_binding(endpoint: &str, required: Option<bool>) -> ProgramReadBinding {
        ProgramReadBinding {
            endpoint: endpoint.to_string(),
            program_read_binding_id: TEST_BINDING_ID.into(),
            auth: HttpAuthMetadata {
                required,
                mode: None,
                session_endpoint: "https://auth.example.test/session".into(),
                jwks_url: None,
                token_transport: None,
                audience: None,
                target_kind: "program-read-binding".into(),
                target_id: TEST_BINDING_ID.into(),
                scopes: Some(vec!["read".into()]),
                accepted_key_classes: None,
            },
        }
    }

    #[derive(Debug, Clone)]
    pub(crate) struct CapturedRequest {
        pub method: String,
        pub path: String,
        pub authorization: Option<String>,
        pub content_type: Option<String>,
        pub body: String,
    }

    #[derive(Debug, Clone)]
    pub(crate) struct CannedResponse {
        pub status: u16,
        pub headers: Vec<(&'static str, String)>,
        pub body: String,
    }

    impl CannedResponse {
        pub(crate) fn json(body: impl Into<String>) -> Self {
            Self {
                status: 200,
                headers: Vec::new(),
                body: body.into(),
            }
        }

        pub(crate) fn with_status(mut self, status: u16) -> Self {
            self.status = status;
            self
        }

        pub(crate) fn with_header(mut self, name: &'static str, value: impl Into<String>) -> Self {
            self.headers.push((name, value.into()));
            self
        }
    }

    #[derive(Clone, Default)]
    pub(crate) struct TestServerState {
        requests: Arc<Mutex<Vec<CapturedRequest>>>,
        responses: Arc<Mutex<VecDeque<CannedResponse>>>,
    }

    pub(crate) struct TestServer {
        state: TestServerState,
        pub base_url: String,
        handle: tokio::task::JoinHandle<()>,
    }

    impl TestServer {
        pub(crate) async fn spawn(responses: Vec<CannedResponse>) -> Self {
            let state = TestServerState {
                requests: Arc::new(Mutex::new(Vec::new())),
                responses: Arc::new(Mutex::new(responses.into())),
            };
            let app = Router::new()
                .fallback(handle_request)
                .with_state(state.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind test server");
            let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
            let handle = tokio::spawn(async move {
                axum::serve(listener, app).await.expect("test server runs");
            });
            Self {
                state,
                base_url,
                handle,
            }
        }

        pub(crate) fn requests(&self) -> Vec<CapturedRequest> {
            self.state.requests.lock().expect("requests lock").clone()
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    async fn handle_request(
        State(state): State<TestServerState>,
        request: Request,
    ) -> Response<Body> {
        let method = request.method().to_string();
        let path = request.uri().path().to_string();
        let authorization = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let content_type = request
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = to_bytes(request.into_body(), usize::MAX)
            .await
            .expect("request body");
        let body = String::from_utf8_lossy(&bytes).to_string();
        state
            .requests
            .lock()
            .expect("requests lock")
            .push(CapturedRequest {
                method,
                path,
                authorization,
                content_type,
                body,
            });

        let canned = state
            .responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .unwrap_or_else(|| CannedResponse::json("{}"));
        let mut builder = Response::builder()
            .status(canned.status)
            .header("content-type", "application/json");
        for (name, value) in &canned.headers {
            builder = builder.header(*name, value);
        }
        builder
            .body(Body::from(canned.body))
            .expect("response builds")
    }

    #[derive(Debug, Clone)]
    pub(crate) struct BearerCall {
        pub scopes: Vec<String>,
        pub target: Option<ReadAuthTarget>,
        pub force_refresh: bool,
    }

    #[derive(Debug, Clone)]
    pub(crate) struct InvalidateCall {
        pub scopes: Vec<String>,
        pub target: Option<ReadAuthTarget>,
    }

    /// Deterministic token source: issues `token-1`, `token-2`, … and records
    /// every call.
    #[derive(Default)]
    pub(crate) struct MockTokenSource {
        counter: AtomicUsize,
        bearer_calls: Mutex<Vec<BearerCall>>,
        invalidations: Mutex<Vec<InvalidateCall>>,
    }

    impl MockTokenSource {
        pub(crate) fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        pub(crate) fn bearer_calls(&self) -> Vec<BearerCall> {
            self.bearer_calls.lock().expect("bearer lock").clone()
        }

        pub(crate) fn invalidations(&self) -> Vec<InvalidateCall> {
            self.invalidations.lock().expect("invalidate lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl BearerTokenSource for MockTokenSource {
        async fn bearer_token(
            &self,
            scopes: &[&str],
            target: Option<&ReadAuthTarget>,
            force_refresh: bool,
        ) -> Result<Option<String>, AreteError> {
            let index = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
            self.bearer_calls
                .lock()
                .expect("bearer lock")
                .push(BearerCall {
                    scopes: scopes.iter().map(|scope| scope.to_string()).collect(),
                    target: target.cloned(),
                    force_refresh,
                });
            Ok(Some(format!("token-{index}")))
        }

        fn invalidate(&self, scopes: &[&str], target: Option<&ReadAuthTarget>) {
            self.invalidations
                .lock()
                .expect("invalidate lock")
                .push(InvalidateCall {
                    scopes: scopes.iter().map(|scope| scope.to_string()).collect(),
                    target: target.cloned(),
                });
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use serde_json::json;

    fn local_transport(server: &TestServer) -> ProgramReadTransport {
        ProgramReadTransport::local_http(
            server.base_url.clone(),
            test_release(),
            reqwest::Client::new(),
        )
    }

    fn hosted_transport(
        server: &TestServer,
        auth: Option<Arc<dyn BearerTokenSource>>,
        required: Option<bool>,
    ) -> ProgramReadTransport {
        ProgramReadTransport::hosted(
            &test_binding(&server.base_url, required),
            test_release(),
            auth,
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_issues_get_on_the_release_addressed_path() {
        let raw_value = fixture()["success"]["rawValue"].clone();
        let server = TestServer::spawn(vec![CannedResponse::json(raw_value.to_string())]).await;
        let transport = local_transport(&server);

        let value = transport
            .read(&ProgramReadRequest::Fetch {
                account: "State",
                address: "present",
            })
            .await
            .unwrap();
        assert_eq!(value, raw_value);

        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(
            requests[0].path,
            "/v1/releases/release-alpha/accounts/State/present"
        );
        assert!(requests[0].authorization.is_none());
        assert!(requests[0].body.is_empty());
    }

    #[tokio::test]
    async fn fetch_missing_returns_null_wire_value() {
        let server = TestServer::spawn(vec![CannedResponse::json("null")]).await;
        let transport = local_transport(&server);

        let value = transport
            .read(&ProgramReadRequest::Fetch {
                account: "State",
                address: "missing",
            })
            .await
            .unwrap();
        assert!(value.is_null());
    }

    #[tokio::test]
    async fn exists_addresses_the_exists_suffix() {
        let exists = fixture()["success"]["exists"].clone();
        let server = TestServer::spawn(vec![CannedResponse::json(exists.to_string())]).await;
        let transport = local_transport(&server);

        let value = transport
            .read(&ProgramReadRequest::Exists {
                account: "State",
                address: "present",
            })
            .await
            .unwrap();
        assert_eq!(value, exists);
        assert_eq!(
            server.requests()[0].path,
            "/v1/releases/release-alpha/accounts/State/present/exists"
        );
    }

    #[tokio::test]
    async fn fetch_many_posts_addresses_to_the_account_root() {
        let batch = fixture()["success"]["batch"].clone();
        let server = TestServer::spawn(vec![CannedResponse::json(batch.to_string())]).await;
        let transport = local_transport(&server);

        let value = transport
            .read(&ProgramReadRequest::FetchMany {
                account: "State",
                addresses: &["present", "missing", "broken"],
            })
            .await
            .unwrap();
        assert_eq!(value, batch);

        let requests = server.requests();
        assert_eq!(requests[0].method, "POST");
        assert_eq!(
            requests[0].path,
            "/v1/releases/release-alpha/accounts/State"
        );
        assert!(requests[0]
            .content_type
            .as_deref()
            .unwrap_or_default()
            .starts_with("application/json"));
        assert_eq!(
            serde_json::from_str::<Value>(&requests[0].body).unwrap(),
            json!({ "addresses": ["present", "missing", "broken"] })
        );
    }

    #[tokio::test]
    async fn preserves_typed_release_hashes_and_encodes_path_segments() {
        let release_hash = "arete:h1:program-release:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let server = TestServer::spawn(vec![CannedResponse::json(r#"{"value":"typed"}"#)]).await;
        let transport = ProgramReadTransport::local_http(
            format!("{}/local/api/", server.base_url),
            ProgramReleaseReference {
                program_release_hash: release_hash.into(),
                program_spec_hash: "spec-alpha".into(),
            },
            reqwest::Client::new(),
        );

        transport
            .read(&ProgramReadRequest::Fetch {
                account: "State",
                address: "addr/one two+",
            })
            .await
            .unwrap();

        assert_eq!(
            server.requests()[0].path,
            format!("/local/api/v1/releases/{release_hash}/accounts/State/addr%2Fone%20two%2B")
        );
    }

    #[tokio::test]
    async fn refreshable_401_invalidates_and_replays_exactly_once() {
        let refreshable = fixture()["errors"]["refreshable"].clone();
        let raw_value = fixture()["success"]["rawValue"].clone();
        let server = TestServer::spawn(vec![
            CannedResponse::json(refreshable.to_string()).with_status(401),
            CannedResponse::json(raw_value.to_string()),
        ])
        .await;
        let auth = MockTokenSource::new();
        let transport = hosted_transport(&server, Some(auth.clone()), None);

        let value = transport
            .read(&ProgramReadRequest::Fetch {
                account: "State",
                address: "address",
            })
            .await
            .unwrap();
        assert_eq!(value, raw_value);

        let requests = server.requests();
        assert_eq!(requests.len(), 2, "expected exactly one replay");
        assert_eq!(requests[0].authorization.as_deref(), Some("Bearer token-1"));
        assert_eq!(requests[1].authorization.as_deref(), Some("Bearer token-2"));

        let expected_target = ReadAuthTarget {
            target_kind: "program-read-binding".into(),
            target_id: TEST_BINDING_ID.into(),
            program_release_hash: Some("release-alpha".into()),
        };
        let invalidations = auth.invalidations();
        assert_eq!(invalidations.len(), 1);
        assert_eq!(invalidations[0].scopes, vec!["read".to_string()]);
        assert_eq!(invalidations[0].target.as_ref(), Some(&expected_target));

        let bearer_calls = auth.bearer_calls();
        assert_eq!(bearer_calls.len(), 2);
        assert_eq!(bearer_calls[0].scopes, vec!["read".to_string()]);
        assert_eq!(bearer_calls[0].target.as_ref(), Some(&expected_target));
        assert!(!bearer_calls[0].force_refresh);
        assert!(bearer_calls[1].force_refresh);
    }

    #[tokio::test]
    async fn non_refreshable_401_does_not_replay() {
        let non_refreshable = fixture()["errors"]["nonRefreshable"].clone();
        let server = TestServer::spawn(vec![
            CannedResponse::json(non_refreshable.to_string()).with_status(401)
        ])
        .await;
        let auth = MockTokenSource::new();
        let transport = hosted_transport(&server, Some(auth.clone()), None);

        let err = transport
            .read(&ProgramReadRequest::Fetch {
                account: "State",
                address: "address",
            })
            .await
            .unwrap_err();
        let ReadError::Request(request_error) = err else {
            panic!("expected ReadError::Request, got {err:?}");
        };
        assert_eq!(request_error.status, 401);
        assert_eq!(
            request_error.server_error_code.as_deref(),
            Some("AUTH_REQUIRED")
        );
        assert_eq!(server.requests().len(), 1);
        assert!(auth.invalidations().is_empty());
    }

    #[tokio::test]
    async fn refreshable_code_on_403_does_not_replay() {
        let refreshable = fixture()["errors"]["refreshable"].clone();
        let server = TestServer::spawn(vec![
            CannedResponse::json(refreshable.to_string()).with_status(403)
        ])
        .await;
        let auth = MockTokenSource::new();
        let transport = hosted_transport(&server, Some(auth.clone()), None);

        let err = transport
            .read(&ProgramReadRequest::Fetch {
                account: "State",
                address: "address",
            })
            .await
            .unwrap_err();
        let ReadError::Request(request_error) = err else {
            panic!("expected ReadError::Request, got {err:?}");
        };
        assert_eq!(request_error.status, 403);
        assert_eq!(
            request_error.server_error_code.as_deref(),
            Some("TOKEN_EXPIRED")
        );
        assert_eq!(server.requests().len(), 1);
        assert!(auth.invalidations().is_empty());
    }

    #[tokio::test]
    async fn refreshable_401_without_auth_source_does_not_replay() {
        let refreshable = fixture()["errors"]["refreshable"].clone();
        let server = TestServer::spawn(vec![
            CannedResponse::json(refreshable.to_string()).with_status(401)
        ])
        .await;
        let transport = local_transport(&server);

        let err = transport
            .read(&ProgramReadRequest::Fetch {
                account: "State",
                address: "address",
            })
            .await
            .unwrap_err();
        assert!(matches!(err, ReadError::Request(_)));
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn header_error_code_wins_over_nested_body_code() {
        let nested = fixture()["errors"]["nested"].clone();
        let server = TestServer::spawn(vec![CannedResponse::json(nested.to_string())
            .with_status(422)
            .with_header("X-Error-Code", "ACCOUNT_OWNER_MISMATCH")])
        .await;
        let transport = local_transport(&server);

        let err = transport
            .read(&ProgramReadRequest::Fetch {
                account: "State",
                address: "broken",
            })
            .await
            .unwrap_err();
        let ReadError::Request(request_error) = err else {
            panic!("expected ReadError::Request, got {err:?}");
        };
        assert_eq!(request_error.status, 422);
        assert_eq!(
            request_error.path,
            "/v1/releases/release-alpha/accounts/State/broken"
        );
        assert_eq!(request_error.body, nested.to_string());
        assert_eq!(
            request_error.server_error_code.as_deref(),
            Some("ACCOUNT_OWNER_MISMATCH")
        );
        assert_eq!(
            request_error.to_string(),
            format!(
                "Read request to '/v1/releases/release-alpha/accounts/State/broken' failed (422): {nested}"
            )
        );
    }

    #[tokio::test]
    async fn nested_body_code_is_read_without_header() {
        let nested = fixture()["errors"]["nested"].clone();
        let server = TestServer::spawn(vec![
            CannedResponse::json(nested.to_string()).with_status(422)
        ])
        .await;
        let transport = local_transport(&server);

        let err = transport
            .read(&ProgramReadRequest::Fetch {
                account: "State",
                address: "broken",
            })
            .await
            .unwrap_err();
        let ReadError::Request(request_error) = err else {
            panic!("expected ReadError::Request, got {err:?}");
        };
        assert_eq!(
            request_error.server_error_code.as_deref(),
            Some("ACCOUNT_DECODE_FAILED")
        );
    }

    #[tokio::test]
    async fn invalid_json_on_success_is_an_error() {
        let server = TestServer::spawn(vec![CannedResponse::json("not-json")]).await;
        let transport = local_transport(&server);

        let err = transport
            .read(&ProgramReadRequest::Fetch {
                account: "State",
                address: "present",
            })
            .await
            .unwrap_err();
        assert!(matches!(
            &err,
            ReadError::InvalidResponse { path, .. }
                if path == "/v1/releases/release-alpha/accounts/State/present"
        ));
        assert_eq!(
            err.to_string(),
            "Program read '/v1/releases/release-alpha/accounts/State/present' returned invalid JSON"
        );
    }

    #[tokio::test]
    async fn hosted_without_auth_source_sends_no_authorization_header() {
        let server = TestServer::spawn(vec![CannedResponse::json(r#"{"value":"open"}"#)]).await;
        let transport = hosted_transport(&server, None, Some(false));

        transport
            .read(&ProgramReadRequest::Fetch {
                account: "State",
                address: "address",
            })
            .await
            .unwrap();
        assert!(server.requests()[0].authorization.is_none());
    }

    #[test]
    fn encodes_uri_components_like_javascript() {
        assert_eq!(encode_uri_component("abc-AZ_09.!~*'()"), "abc-AZ_09.!~*'()");
        assert_eq!(encode_uri_component("a b/c:d+e"), "a%20b%2Fc%3Ad%2Be");
        assert_eq!(encode_uri_component("é"), "%C3%A9");
    }

    #[test]
    fn refreshable_error_codes_follow_auth_error_semantics() {
        assert!(is_refreshable_error_code("TOKEN_EXPIRED"));
        assert!(is_refreshable_error_code("token-expired"));
        assert!(is_refreshable_error_code(" token-invalid-signature "));
        assert!(!is_refreshable_error_code("AUTH_REQUIRED"));
        assert!(!is_refreshable_error_code("nonsense"));
    }
}
