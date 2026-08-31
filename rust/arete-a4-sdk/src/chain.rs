//! Chain read client (`/chain/*` HTTP routes).
//!
//! Port of `typescript/core/src/chain.ts`: the [`ChainClient`] trait with the
//! nine chain read methods and [`HttpChainClient`], the HTTP implementation
//! authenticated through [`crate::http`] with the `read` scope.
//!
//! `u64` values are decimal strings on the wire (validated as `^\d+$` within
//! `u64::MAX`); account data is base64.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use crate::error::{AreteError, AuthErrorCode};
use crate::http::{fetch_json, AuthedRequest, HttpMethod, TokenSource};

/// Errors produced by chain read requests.
#[derive(Debug, Error)]
pub enum ChainError {
    /// The server answered with a non-2xx status.
    #[error("Read request to '{path}' failed ({status}): {body}")]
    Request {
        status: u16,
        path: String,
        body: String,
        code: Option<AuthErrorCode>,
    },

    /// The response body did not match the wire contract.
    #[error("Invalid chain response for '{path}': {message}")]
    InvalidResponse { path: String, message: String },

    /// The call was rejected locally, before any request was made.
    #[error("Invalid chain request: {0}")]
    InvalidRequest(String),

    /// Transport or authentication failure from the SDK core.
    #[error(transparent)]
    Sdk(#[from] AreteError),
}

impl From<ChainError> for AreteError {
    fn from(error: ChainError) -> Self {
        match error {
            ChainError::Request {
                status,
                code,
                path,
                body,
            } => {
                if code.is_some() {
                    AreteError::AuthRequestFailed {
                        status,
                        message: format!("Read request to '{path}' failed: {body}"),
                        code,
                    }
                } else {
                    AreteError::ConnectionFailed(format!(
                        "Read request to '{path}' failed ({status}): {body}"
                    ))
                }
            }
            ChainError::InvalidResponse { path, message } => {
                AreteError::Serialization(format!("Invalid chain response for '{path}': {message}"))
            }
            ChainError::InvalidRequest(message) => {
                AreteError::InvalidConfig(format!("Invalid chain request: {message}"))
            }
            ChainError::Sdk(inner) => inner,
        }
    }
}

/// Cluster clock as reported by `/chain/clock`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainClock {
    pub slot: u64,
    #[serde(default)]
    pub epoch: Option<u64>,
    #[serde(default)]
    pub leader_schedule_epoch: Option<u64>,
    pub unix_timestamp: i64,
}

/// Mint account summary from `/chain/mints/<address>`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MintAccountInfo {
    pub address: String,
    pub owner_program: String,
    #[serde(default)]
    pub decimals: Option<u8>,
    #[serde(default)]
    pub supply: Option<String>,
    #[serde(default)]
    pub mint_authority: Option<String>,
    #[serde(default)]
    pub freeze_authority: Option<String>,
}

/// Token account summary from `/chain/token-accounts/<address>`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenAccountInfo {
    pub address: String,
    pub owner_program: String,
    #[serde(default)]
    pub mint: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub amount: Option<String>,
    #[serde(default)]
    pub ui_amount_string: Option<String>,
}

/// Token balance from `POST /chain/balances`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenBalanceInfo {
    pub exists: bool,
    pub address: Option<String>,
    pub owner: String,
    pub mint: String,
    pub token_program: Option<String>,
    /// Raw amount as a decimal string (mirrors the TS surface).
    pub amount: String,
    pub decimals: Option<u8>,
    pub ui_amount_string: Option<String>,
    pub context_slot: u64,
}

/// Native SOL balance from `POST /chain/native-balance`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeBalanceInfo {
    pub lamports: u64,
    pub context_slot: u64,
}

/// Optional `minContextSlot` constraint for balance reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContextSlotOptions {
    pub min_context_slot: Option<u64>,
}

/// Input for [`ChainClient::balance`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenBalanceInput {
    pub owner: String,
    pub mint: String,
    pub token_program: Option<String>,
}

/// Raw account info from `/chain/accounts/<address>` (data base64-decoded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawAccountInfo {
    pub address: String,
    pub owner_program: String,
    pub lamports: u64,
    pub executable: bool,
    pub data: Vec<u8>,
}

/// Read access to Solana chain state through the stack's `/chain/*` routes.
#[async_trait]
pub trait ChainClient: Send + Sync {
    /// `GET /chain/exists/<address>` — whether the account exists.
    async fn exists(&self, address: &str) -> Result<bool, ChainError>;

    /// `GET /chain/lamports/<address>` — the account's lamport balance.
    async fn lamports(&self, address: &str) -> Result<u64, ChainError>;

    /// `POST /chain/native-balance` — lamports + context slot.
    async fn native_balance(
        &self,
        address: &str,
        options: ContextSlotOptions,
    ) -> Result<NativeBalanceInfo, ChainError>;

    /// `GET /chain/rent-exemption/<space>` — rent-exempt minimum for `space` bytes.
    async fn minimum_balance_for_rent_exemption(&self, space: u64) -> Result<u64, ChainError>;

    /// `GET /chain/clock` — cluster clock.
    async fn clock(&self) -> Result<ChainClock, ChainError>;

    /// `GET /chain/accounts/<address>` — raw account (base64 data decoded).
    async fn account(&self, address: &str) -> Result<Option<RawAccountInfo>, ChainError>;

    /// `POST /chain/accounts` — up to [`MAX_BATCH_ADDRESSES`] accounts in one call. Results
    /// are positionally aligned with `addresses`; `None` where the account is absent.
    async fn accounts(
        &self,
        addresses: &[String],
    ) -> Result<Vec<Option<RawAccountInfo>>, ChainError>;

    /// `GET /chain/mints/<address>` — mint account summary.
    async fn mint(&self, address: &str) -> Result<Option<MintAccountInfo>, ChainError>;

    /// `GET /chain/token-accounts/<address>` — token account summary.
    async fn token_account(&self, address: &str) -> Result<Option<TokenAccountInfo>, ChainError>;

    /// `POST /chain/balances` — token balance for an owner + mint.
    async fn balance(
        &self,
        input: &TokenBalanceInput,
        options: ContextSlotOptions,
    ) -> Result<TokenBalanceInfo, ChainError>;
}

/// Derives the HTTP endpoint from a WebSocket URL (`ws` → `http`,
/// `wss` → `https`, single trailing slash stripped). Mirrors the TS
/// `deriveHttpEndpoint`, including the string-surgery fallback for inputs the
/// URL parser rejects.
pub fn derive_http_endpoint(ws_url: &str) -> String {
    fn fallback(ws_url: &str) -> String {
        let lower = ws_url.to_ascii_lowercase();
        if lower.starts_with("wss:") {
            format!("https:{}", &ws_url[4..])
        } else if lower.starts_with("ws:") {
            format!("http:{}", &ws_url[3..])
        } else {
            ws_url.to_string()
        }
    }

    match url::Url::parse(ws_url) {
        Ok(mut parsed) => {
            let mapped = match parsed.scheme() {
                "ws" => parsed.set_scheme("http").is_ok(),
                "wss" => parsed.set_scheme("https").is_ok(),
                _ => true,
            };
            if !mapped {
                return fallback(ws_url);
            }
            let rendered = parsed.to_string();
            rendered
                .strip_suffix('/')
                .map(str::to_string)
                .unwrap_or(rendered)
        }
        Err(_) => fallback(ws_url),
    }
}

/// Percent-encodes a URL path segment like JavaScript's `encodeURIComponent`.
fn encode_uri_component(value: &str) -> String {
    const UNRESERVED: &[u8] = b"-_.!~*'()";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || UNRESERVED.contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn parse_decimal_u64(value: &str, field: &str, path: &str) -> Result<u64, ChainError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ChainError::InvalidResponse {
            path: path.to_string(),
            message: format!("{field} must be a decimal u64 string"),
        });
    }
    value
        .parse::<u64>()
        .map_err(|_| ChainError::InvalidResponse {
            path: path.to_string(),
            message: format!("{field} exceeds u64"),
        })
}

/// HTTP [`ChainClient`] over a stack's base URL, authenticated with the
/// `read` scope through a [`TokenSource`].
pub struct HttpChainClient {
    base_url: String,
    tokens: Arc<dyn TokenSource>,
    http: reqwest::Client,
}

impl HttpChainClient {
    pub fn new(base_url: impl Into<String>, tokens: Arc<dyn TokenSource>) -> Self {
        Self::with_http_client(base_url, tokens, reqwest::Client::new())
    }

    pub fn with_http_client(
        base_url: impl Into<String>,
        tokens: Arc<dyn TokenSource>,
        http_client: reqwest::Client,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            tokens,
            http: http_client,
        }
    }

    fn join_url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        if path.starts_with('/') {
            format!("{base}{path}")
        } else {
            format!("{base}/{path}")
        }
    }

    async fn request<T: serde::de::DeserializeOwned>(
        &self,
        method: HttpMethod,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T, ChainError> {
        let request = AuthedRequest {
            method,
            url: self.join_url(path),
            body,
            scopes: vec![crate::http::DEFAULT_READ_SCOPE.to_string()],
            target: None,
            require_predispatch_marker: false,
        };
        let response = fetch_json(&self.http, self.tokens.as_ref(), &request).await?;
        if !response.is_success() {
            return Err(ChainError::Request {
                status: response.status,
                path: path.to_string(),
                body: response.body_text(),
                code: response.error_code,
            });
        }
        response
            .json()
            .map_err(|error| ChainError::InvalidResponse {
                path: path.to_string(),
                message: error.to_string(),
            })
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ChainError> {
        self.request(HttpMethod::Get, path, None).await
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T, ChainError> {
        self.request(HttpMethod::Post, path, Some(body)).await
    }
}

fn with_context_slot(
    mut body: serde_json::Value,
    options: ContextSlotOptions,
) -> serde_json::Value {
    if let Some(min_context_slot) = options.min_context_slot {
        body["minContextSlot"] = json!(min_context_slot.to_string());
    }
    body
}

#[derive(Debug, Deserialize)]
struct ExistsResponse {
    exists: bool,
}

#[derive(Debug, Deserialize)]
struct LamportsResponse {
    lamports: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeBalanceWire {
    lamports: String,
    context_slot: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAccountWire {
    address: String,
    owner_program: String,
    lamports: String,
    executable: bool,
    data: String,
}

#[derive(Debug, Deserialize)]
struct AccountsWire {
    items: Vec<Option<RawAccountWire>>,
}

/// Solana's `getMultipleAccounts` ceiling, mirrored so an oversized batch fails here rather
/// than as a remote 400.
pub const MAX_BATCH_ADDRESSES: usize = 100;

fn decode_raw_account(wire: RawAccountWire, path: &str) -> Result<RawAccountInfo, ChainError> {
    let data = base64::engine::general_purpose::STANDARD
        .decode(wire.data.as_bytes())
        .map_err(|error| ChainError::InvalidResponse {
            path: path.to_string(),
            message: format!("account data is not valid base64: {error}"),
        })?;
    // Decimal string on the wire so a balance above 2^53 survives a JavaScript client.
    let lamports = wire
        .lamports
        .parse::<u64>()
        .map_err(|error| ChainError::InvalidResponse {
            path: path.to_string(),
            message: format!("lamports is not a u64: {error}"),
        })?;
    Ok(RawAccountInfo {
        address: wire.address,
        owner_program: wire.owner_program,
        lamports,
        executable: wire.executable,
        data,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenBalanceWire {
    exists: bool,
    #[serde(default)]
    address: Option<String>,
    owner: String,
    mint: String,
    #[serde(default)]
    token_program: Option<String>,
    amount: String,
    #[serde(default)]
    decimals: Option<u8>,
    #[serde(default)]
    ui_amount_string: Option<String>,
    context_slot: String,
}

#[async_trait]
impl ChainClient for HttpChainClient {
    async fn exists(&self, address: &str) -> Result<bool, ChainError> {
        let path = format!("/chain/exists/{}", encode_uri_component(address));
        let body: ExistsResponse = self.get(&path).await?;
        Ok(body.exists)
    }

    async fn lamports(&self, address: &str) -> Result<u64, ChainError> {
        let path = format!("/chain/lamports/{}", encode_uri_component(address));
        let body: LamportsResponse = self.get(&path).await?;
        Ok(body.lamports)
    }

    async fn native_balance(
        &self,
        address: &str,
        options: ContextSlotOptions,
    ) -> Result<NativeBalanceInfo, ChainError> {
        let path = "/chain/native-balance";
        let body = with_context_slot(json!({ "address": address }), options);
        let wire: NativeBalanceWire = self.post(path, body).await?;
        Ok(NativeBalanceInfo {
            lamports: parse_decimal_u64(&wire.lamports, "lamports", path)?,
            context_slot: parse_decimal_u64(&wire.context_slot, "contextSlot", path)?,
        })
    }

    async fn minimum_balance_for_rent_exemption(&self, space: u64) -> Result<u64, ChainError> {
        let path = format!(
            "/chain/rent-exemption/{}",
            encode_uri_component(&space.to_string())
        );
        let body: LamportsResponse = self.get(&path).await?;
        Ok(body.lamports)
    }

    async fn clock(&self) -> Result<ChainClock, ChainError> {
        self.get("/chain/clock").await
    }

    async fn account(&self, address: &str) -> Result<Option<RawAccountInfo>, ChainError> {
        let path = format!("/chain/accounts/{}", encode_uri_component(address));
        let wire: Option<RawAccountWire> = self.get(&path).await?;
        wire.map(|wire| decode_raw_account(wire, &path)).transpose()
    }

    async fn accounts(
        &self,
        addresses: &[String],
    ) -> Result<Vec<Option<RawAccountInfo>>, ChainError> {
        if addresses.len() > MAX_BATCH_ADDRESSES {
            return Err(ChainError::InvalidRequest(format!(
                "addresses exceeds the {MAX_BATCH_ADDRESSES}-address limit for one batch"
            )));
        }
        if addresses.is_empty() {
            return Ok(Vec::new());
        }

        let path = "/chain/accounts";
        let wire: AccountsWire = self
            .post(path, serde_json::json!({ "addresses": addresses }))
            .await?;
        if wire.items.len() != addresses.len() {
            return Err(ChainError::InvalidResponse {
                path: path.to_string(),
                message: format!(
                    "expected {} items, got {}",
                    addresses.len(),
                    wire.items.len()
                ),
            });
        }
        wire.items
            .into_iter()
            .map(|item| item.map(|wire| decode_raw_account(wire, path)).transpose())
            .collect()
    }

    async fn mint(&self, address: &str) -> Result<Option<MintAccountInfo>, ChainError> {
        let path = format!("/chain/mints/{}", encode_uri_component(address));
        self.get(&path).await
    }

    async fn token_account(&self, address: &str) -> Result<Option<TokenAccountInfo>, ChainError> {
        let path = format!("/chain/token-accounts/{}", encode_uri_component(address));
        self.get(&path).await
    }

    async fn balance(
        &self,
        input: &TokenBalanceInput,
        options: ContextSlotOptions,
    ) -> Result<TokenBalanceInfo, ChainError> {
        let path = "/chain/balances";
        let mut body = json!({ "owner": input.owner, "mint": input.mint });
        if let Some(token_program) = &input.token_program {
            body["tokenProgram"] = json!(token_program);
        }
        let body = with_context_slot(body, options);
        let wire: TokenBalanceWire = self.post(path, body).await?;
        Ok(TokenBalanceInfo {
            exists: wire.exists,
            address: wire.address,
            owner: wire.owner,
            mint: wire.mint,
            token_program: wire.token_program,
            amount: wire.amount,
            decimals: wire.decimals,
            ui_amount_string: wire.ui_amount_string,
            context_slot: parse_decimal_u64(&wire.context_slot, "contextSlot", path)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::AuthTokenRequest;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::Value;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[derive(Default)]
    struct MockTokens {
        issued: AtomicUsize,
        invalidations: AtomicUsize,
        forced: AtomicUsize,
        scopes: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl TokenSource for MockTokens {
        async fn token(
            &self,
            request: &AuthTokenRequest,
            force_refresh: bool,
        ) -> Result<Option<String>, AreteError> {
            let n = self.issued.fetch_add(1, Ordering::SeqCst);
            if force_refresh {
                self.forced.fetch_add(1, Ordering::SeqCst);
            }
            self.scopes.lock().unwrap().push(request.scopes.clone());
            Ok(Some(format!("token-{n}")))
        }

        fn invalidate(&self, _request: &AuthTokenRequest) {
            self.invalidations.fetch_add(1, Ordering::SeqCst);
        }
    }

    async fn spawn(router: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{addr}")
    }

    fn client(base: &str) -> (HttpChainClient, Arc<MockTokens>) {
        let tokens = Arc::new(MockTokens::default());
        (
            HttpChainClient::new(format!("{base}/"), tokens.clone()),
            tokens,
        )
    }

    #[test]
    fn derives_http_endpoints() {
        assert_eq!(
            derive_http_endpoint("ws://localhost:8080/socket"),
            "http://localhost:8080/socket"
        );
        assert_eq!(
            derive_http_endpoint("wss://demo.stack.arete.run/"),
            "https://demo.stack.arete.run"
        );
        assert_eq!(
            derive_http_endpoint("wss://demo.stack.arete.run"),
            "https://demo.stack.arete.run"
        );
        assert_eq!(
            derive_http_endpoint("https://already.http"),
            "https://already.http"
        );
    }

    #[test]
    fn encodes_uri_components() {
        assert_eq!(encode_uri_component("Abc123-_.!~*'()"), "Abc123-_.!~*'()");
        assert_eq!(encode_uri_component("a/b c"), "a%2Fb%20c");
        assert_eq!(encode_uri_component("é"), "%C3%A9");
    }

    #[tokio::test]
    async fn exists_and_lamports_hit_get_routes_with_auth() {
        let captured = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
        let captured_handler = captured.clone();
        let router = Router::new()
            .route(
                "/chain/exists/:address",
                get(move |headers: HeaderMap| {
                    let captured = captured_handler.clone();
                    async move {
                        captured.lock().unwrap().push(
                            headers
                                .get("authorization")
                                .and_then(|v| v.to_str().ok())
                                .map(str::to_string),
                        );
                        Json(serde_json::json!({ "exists": true }))
                    }
                }),
            )
            .route(
                "/chain/lamports/:address",
                get(|| async { Json(serde_json::json!({ "lamports": 5_000_000u64 })) }),
            );
        let base = spawn(router).await;
        let (chain, tokens) = client(&base);

        assert!(chain.exists("addr1").await.unwrap());
        assert_eq!(chain.lamports("addr1").await.unwrap(), 5_000_000);
        assert_eq!(
            captured.lock().unwrap()[0].as_deref(),
            Some("Bearer token-0")
        );
        assert_eq!(tokens.scopes.lock().unwrap()[0], vec!["read".to_string()]);
    }

    #[tokio::test]
    async fn native_balance_posts_context_slot_and_parses_decimal_strings() {
        let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
        let bodies_handler = bodies.clone();
        let router = Router::new().route(
            "/chain/native-balance",
            post(move |Json(body): Json<Value>| {
                let bodies = bodies_handler.clone();
                async move {
                    bodies.lock().unwrap().push(body);
                    Json(serde_json::json!({
                        "lamports": "18446744073709551615",
                        "contextSlot": "12345",
                    }))
                }
            }),
        );
        let base = spawn(router).await;
        let (chain, _) = client(&base);

        let balance = chain
            .native_balance(
                "addr",
                ContextSlotOptions {
                    min_context_slot: Some(42),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            balance,
            NativeBalanceInfo {
                lamports: u64::MAX,
                context_slot: 12345,
            }
        );
        assert_eq!(
            bodies.lock().unwrap()[0],
            serde_json::json!({ "address": "addr", "minContextSlot": "42" })
        );
    }

    #[tokio::test]
    async fn accounts_posts_every_address_and_keeps_absent_slots() {
        let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
        let bodies_handler = bodies.clone();
        let router = Router::new().route(
            "/chain/accounts",
            post(move |Json(body): Json<Value>| {
                let bodies = bodies_handler.clone();
                async move {
                    bodies.lock().unwrap().push(body);
                    Json(serde_json::json!({
                        "items": [
                            {
                                "address": "a",
                                "ownerProgram": "prog",
                                "lamports": "7",
                                "executable": false,
                                "data": "AQID",
                            },
                            Value::Null,
                        ]
                    }))
                }
            }),
        );
        let base = spawn(router).await;
        let (chain, _tokens) = client(&base);

        let addresses = vec!["a".to_string(), "b".to_string()];
        let items = chain.accounts(&addresses).await.unwrap();

        assert_eq!(items.len(), 2);
        let first = items[0].as_ref().expect("first account present");
        assert_eq!(first.address, "a");
        assert_eq!(first.lamports, 7);
        assert_eq!(first.data, vec![1, 2, 3], "base64 decoded");
        assert!(items[1].is_none(), "absent account keeps its slot");
        assert_eq!(
            bodies.lock().unwrap()[0],
            serde_json::json!({ "addresses": ["a", "b"] })
        );
    }

    /// An over-long batch must fail locally: the server would reject it anyway, and a
    /// doomed round trip hides a caller bug behind a network error.
    #[tokio::test]
    async fn accounts_rejects_an_oversized_batch_without_requesting() {
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_handler = hits.clone();
        let router = Router::new().route(
            "/chain/accounts",
            post(move || {
                let hits = hits_handler.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    Json(serde_json::json!({ "items": [] }))
                }
            }),
        );
        let base = spawn(router).await;
        let (chain, _tokens) = client(&base);

        let addresses: Vec<String> = (0..=MAX_BATCH_ADDRESSES).map(|i| i.to_string()).collect();
        let error = chain
            .accounts(&addresses)
            .await
            .expect_err("over the limit");

        assert!(
            matches!(&error, ChainError::InvalidRequest(m) if m.contains("100-address")),
            "unexpected error: {error:?}"
        );
        assert!(
            chain.accounts(&[]).await.unwrap().is_empty(),
            "empty is free"
        );
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "neither call may reach the server"
        );
    }

    /// A short `items` array would silently shift every later account onto the wrong
    /// address, so it must be an error rather than a truncated result.
    #[tokio::test]
    async fn accounts_rejects_a_length_mismatch() {
        let router = Router::new().route(
            "/chain/accounts",
            post(|| async { Json(serde_json::json!({ "items": [Value::Null] })) }),
        );
        let base = spawn(router).await;
        let (chain, _tokens) = client(&base);

        let addresses = vec!["a".to_string(), "b".to_string()];
        let error = chain
            .accounts(&addresses)
            .await
            .expect_err("length mismatch");

        assert!(
            matches!(&error, ChainError::InvalidResponse { message, .. } if message.contains("got 1")),
            "unexpected error: {error:?}"
        );
    }

    #[tokio::test]
    async fn invalid_decimal_strings_are_rejected() {
        let router = Router::new().route(
            "/chain/native-balance",
            post(|| async {
                Json(serde_json::json!({
                    "lamports": "18446744073709551616", // u64::MAX + 1
                    "contextSlot": "1",
                }))
            }),
        );
        let base = spawn(router).await;
        let (chain, _) = client(&base);
        let error = chain
            .native_balance("addr", ContextSlotOptions::default())
            .await
            .unwrap_err();
        assert!(
            matches!(error, ChainError::InvalidResponse { ref message, .. }
            if message.contains("exceeds u64"))
        );

        let router = Router::new().route(
            "/chain/native-balance",
            post(|| async { Json(serde_json::json!({ "lamports": "12x4", "contextSlot": "1" })) }),
        );
        let base = spawn(router).await;
        let (chain, _) = client(&base);
        let error = chain
            .native_balance("addr", ContextSlotOptions::default())
            .await
            .unwrap_err();
        assert!(
            matches!(error, ChainError::InvalidResponse { ref message, .. }
            if message.contains("decimal u64 string"))
        );
    }

    #[tokio::test]
    async fn clock_account_mint_and_token_account_decode() {
        let router = Router::new()
            .route(
                "/chain/clock",
                get(|| async {
                    Json(serde_json::json!({
                        "slot": 987654321u64,
                        "epoch": 500u64,
                        "unixTimestamp": 1_754_000_000i64,
                    }))
                }),
            )
            .route(
                "/chain/accounts/:address",
                get(|| async {
                    Json(serde_json::json!({
                        "address": "acc",
                        "ownerProgram": "prog",
                        "lamports": "1000",
                        "executable": false,
                        "data": "aGVsbG8=",
                    }))
                }),
            )
            .route(
                "/chain/mints/:address",
                get(|| async {
                    Json(serde_json::json!({
                        "address": "mint",
                        "ownerProgram": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
                        "decimals": 6,
                        "supply": "1000000",
                        "mintAuthority": null,
                        "freezeAuthority": null,
                    }))
                }),
            )
            .route(
                "/chain/token-accounts/:address",
                get(|| async { Json(serde_json::json!(null)) }),
            );
        let base = spawn(router).await;
        let (chain, _) = client(&base);

        let clock = chain.clock().await.unwrap();
        assert_eq!(clock.slot, 987654321);
        assert_eq!(clock.epoch, Some(500));
        assert_eq!(clock.leader_schedule_epoch, None);
        assert_eq!(clock.unix_timestamp, 1_754_000_000);

        let account = chain.account("acc").await.unwrap().unwrap();
        assert_eq!(account.data, b"hello");
        assert_eq!(account.owner_program, "prog");

        let mint = chain.mint("mint").await.unwrap().unwrap();
        assert_eq!(mint.decimals, Some(6));
        assert_eq!(mint.supply.as_deref(), Some("1000000"));
        assert_eq!(mint.mint_authority, None);

        assert_eq!(chain.token_account("missing").await.unwrap(), None);
    }

    #[tokio::test]
    async fn null_account_maps_to_none() {
        let router = Router::new().route(
            "/chain/accounts/:address",
            get(|| async { Json(serde_json::json!(null)) }),
        );
        let base = spawn(router).await;
        let (chain, _) = client(&base);
        assert_eq!(chain.account("missing").await.unwrap(), None);
    }

    #[tokio::test]
    async fn balance_round_trips_token_balance_wire() {
        let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
        let bodies_handler = bodies.clone();
        let router = Router::new().route(
            "/chain/balances",
            post(move |Json(body): Json<Value>| {
                let bodies = bodies_handler.clone();
                async move {
                    bodies.lock().unwrap().push(body);
                    Json(serde_json::json!({
                        "exists": true,
                        "address": "ata",
                        "owner": "owner",
                        "mint": "mint",
                        "tokenProgram": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
                        "amount": "2500000",
                        "decimals": 6,
                        "uiAmountString": "2.5",
                        "contextSlot": "777",
                    }))
                }
            }),
        );
        let base = spawn(router).await;
        let (chain, _) = client(&base);

        let balance = chain
            .balance(
                &TokenBalanceInput {
                    owner: "owner".to_string(),
                    mint: "mint".to_string(),
                    token_program: Some("prog".to_string()),
                },
                ContextSlotOptions {
                    min_context_slot: Some(7),
                },
            )
            .await
            .unwrap();
        assert_eq!(balance.amount, "2500000");
        assert_eq!(balance.context_slot, 777);
        assert_eq!(balance.decimals, Some(6));
        assert_eq!(
            bodies.lock().unwrap()[0],
            serde_json::json!({
                "owner": "owner",
                "mint": "mint",
                "tokenProgram": "prog",
                "minContextSlot": "7",
            })
        );
    }

    #[tokio::test]
    async fn error_bodies_surface_status_code_and_wire_code() {
        let router = Router::new().route(
            "/chain/clock",
            get(|| async {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": "upstream unavailable",
                        "code": "internal-error",
                    })),
                )
            }),
        );
        let base = spawn(router).await;
        let (chain, _) = client(&base);
        let error = chain.clock().await.unwrap_err();
        match error {
            ChainError::Request {
                status, code, body, ..
            } => {
                assert_eq!(status, 500);
                assert_eq!(code, Some(AuthErrorCode::InternalError));
                assert!(body.contains("upstream unavailable"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn refresh_worthy_401_replays_exactly_once() {
        #[derive(Clone)]
        struct Calls(Arc<AtomicUsize>);
        async fn handler(State(calls): State<Calls>) -> axum::response::Response {
            let n = calls.0.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                (
                    StatusCode::UNAUTHORIZED,
                    [("X-Error-Code", "token-expired")],
                    Json(serde_json::json!({ "error": "expired" })),
                )
                    .into_response()
            } else {
                Json(serde_json::json!({ "exists": true })).into_response()
            }
        }
        let calls = Calls(Arc::new(AtomicUsize::new(0)));
        let router = Router::new()
            .route("/chain/exists/:address", get(handler))
            .with_state(calls.clone());
        let base = spawn(router).await;
        let (chain, tokens) = client(&base);

        assert!(chain.exists("addr").await.unwrap());
        assert_eq!(calls.0.load(Ordering::SeqCst), 2);
        assert_eq!(tokens.invalidations.load(Ordering::SeqCst), 1);
        assert_eq!(tokens.forced.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn persistent_401_fails_after_single_replay() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_handler = calls.clone();
        let router = Router::new().route(
            "/chain/exists/:address",
            get(move || {
                let calls = calls_handler.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    (
                        StatusCode::UNAUTHORIZED,
                        [("X-Error-Code", "token-expired")],
                        Json(serde_json::json!({ "error": "expired" })),
                    )
                }
            }),
        );
        let base = spawn(router).await;
        let (chain, tokens) = client(&base);

        let error = chain.exists("addr").await.unwrap_err();
        assert!(matches!(
            error,
            ChainError::Request {
                status: 401,
                code: Some(AuthErrorCode::TokenExpired),
                ..
            }
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(tokens.invalidations.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn non_refresh_worthy_errors_are_not_replayed() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_handler = calls.clone();
        let router = Router::new().route(
            "/chain/exists/:address",
            get(move || {
                let calls = calls_handler.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    (
                        StatusCode::FORBIDDEN,
                        [("X-Error-Code", "origin-not-allowed")],
                        Json(serde_json::json!({ "error": "nope" })),
                    )
                }
            }),
        );
        let base = spawn(router).await;
        let (chain, tokens) = client(&base);

        let error = chain.exists("addr").await.unwrap_err();
        assert!(matches!(
            error,
            ChainError::Request {
                status: 403,
                code: Some(AuthErrorCode::OriginNotAllowed),
                ..
            }
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(tokens.invalidations.load(Ordering::SeqCst), 0);
    }
}
