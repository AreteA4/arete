//! Hosted Solana gateway transports (port of
//! `typescript/core/src/solana-gateway.ts`).
//!
//! Generated gateway descriptors (`sgb_…` bindings) become explicit
//! [`ChainClient`] and [`TransactionTransport`] implementations pointed at
//! the hosted gateway endpoints. Tokens are isolated by exact binding target
//! and scope: every capability mints `solana-gateway-binding`-targeted tokens
//! for its own binding ID, `transaction:send` requests only replay after a
//! refresh when the response carries the `X-Arete-Upstream-Attempted: false`
//! predispatch marker, and chain reads never require the marker.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::auth::AuthConfig;
use crate::chain::{ChainClient, HttpChainClient};
use crate::error::AreteError;
use crate::http::{AuthTokenRequest, AuthTokenTarget, HttpAuthClient, TokenSource};
use crate::transactions::{HttpTransactionTransport, TransactionTransport};

/// Complete public auth metadata emitted for a hosted Solana gateway binding
/// (TS `SolanaGatewayAuthMetadata`; serde matches the TS-generated camelCase
/// JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaGatewayAuthMetadata {
    pub required: bool,
    pub mode: String,
    pub session_endpoint: String,
    pub jwks_url: String,
    pub token_transport: String,
    /// Must be `"arete:solana-gateway"`.
    pub audience: String,
    /// Must be `"solana-gateway-binding"`.
    pub target_kind: String,
    pub target_id: String,
    pub scopes: Vec<String>,
    pub accepted_key_classes: Vec<String>,
    pub transaction_entitlement_required: bool,
}

/// One generated, non-inheriting hosted Solana gateway capability binding
/// (TS `HostedSolanaGatewayCapabilityBinding`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedSolanaGatewayCapabilityBinding {
    pub endpoint: String,
    pub auth_policy: String,
    pub solana_gateway_binding_id: String,
    pub cluster: String,
    pub region: String,
    pub auth: SolanaGatewayAuthMetadata,
}

/// Generated gateway descriptors: one binding per capability
/// (TS `HostedSolanaGatewayBindings`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedSolanaGatewayBindings {
    pub chain: HostedSolanaGatewayCapabilityBinding,
    pub transactions: HostedSolanaGatewayCapabilityBinding,
}

/// The transports constructed from a validated gateway descriptor pair:
/// `(chain, transactions)`.
pub type HostedSolanaGatewayTransports = (Arc<dyn ChainClient>, Arc<dyn TransactionTransport>);

/// Scopes a chain binding must grant.
pub const CHAIN_REQUIRED_SCOPES: &[&str] = &["read"];
/// Scopes a transactions binding must grant.
pub const TRANSACTIONS_REQUIRED_SCOPES: &[&str] = &["transaction:inspect", "transaction:send"];

fn is_secure_or_loopback_http_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    match url.scheme() {
        "https" => true,
        "http" => match url.host() {
            Some(url::Host::Domain(host)) => host == "localhost",
            Some(url::Host::Ipv4(ip)) => ip == std::net::Ipv4Addr::LOCALHOST,
            Some(url::Host::Ipv6(ip)) => ip == std::net::Ipv6Addr::LOCALHOST,
            None => false,
        },
        _ => false,
    }
}

/// `^sgb_[A-Za-z0-9_-]{32}$`
fn is_canonical_gateway_binding_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("sgb_") else {
        return false;
    };
    rest.len() == 32
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Validate one capability binding (port of the TS `validateBinding`).
pub fn validate_gateway_binding(
    capability: &str,
    binding: &HostedSolanaGatewayCapabilityBinding,
    required_scopes: &[&str],
) -> Result<(), AreteError> {
    let auth = &binding.auth;
    let complete = is_secure_or_loopback_http_url(&binding.endpoint)
        && is_canonical_gateway_binding_id(&binding.solana_gateway_binding_id)
        && !binding.cluster.trim().is_empty()
        && !binding.region.trim().is_empty()
        && auth.mode == binding.auth_policy
        && is_secure_or_loopback_http_url(&auth.session_endpoint)
        && is_secure_or_loopback_http_url(&auth.jwks_url)
        && auth.token_transport == "bearer"
        && auth.audience == "arete:solana-gateway"
        && auth.target_kind == "solana-gateway-binding"
        && auth.target_id == binding.solana_gateway_binding_id
        && required_scopes
            .iter()
            .all(|scope| auth.scopes.iter().any(|granted| granted == scope));
    if complete {
        Ok(())
    } else {
        Err(AreteError::InvalidConfig(format!(
            "Hosted Solana gateway {capability} binding is incomplete or inconsistent"
        )))
    }
}

fn has_runtime_auth_strategy(auth: Option<&AuthConfig>) -> bool {
    auth.is_some_and(|auth| {
        auth.token.is_some() || auth.get_token.is_some() || auth.token_endpoint.is_some()
    })
}

/// TS `bindingAuthConfig`: a configured runtime strategy wins; bindings that
/// do not require auth keep whatever runtime auth exists; otherwise tokens
/// are minted from the binding's session endpoint (keeping any publishable
/// key and custom headers from the runtime config).
fn binding_auth_config(
    binding: &HostedSolanaGatewayCapabilityBinding,
    runtime_auth: Option<&AuthConfig>,
) -> Option<AuthConfig> {
    if has_runtime_auth_strategy(runtime_auth) {
        return runtime_auth.cloned();
    }
    if !binding.auth.required {
        return runtime_auth.cloned();
    }
    let mut config = runtime_auth.cloned().unwrap_or_default();
    config = config.with_token_endpoint(binding.auth.session_endpoint.clone());
    Some(config)
}

/// [`TokenSource`] adapter that pins every token request to one
/// `solana-gateway-binding` target, so per-capability tokens are isolated by
/// binding ID and scope regardless of what the transport asks for.
struct TargetedTokenSource {
    inner: Arc<HttpAuthClient>,
    target: AuthTokenTarget,
}

impl TargetedTokenSource {
    fn retarget(&self, request: &AuthTokenRequest) -> AuthTokenRequest {
        AuthTokenRequest {
            scopes: request.scopes.clone(),
            target: Some(self.target.clone()),
        }
    }
}

#[async_trait]
impl TokenSource for TargetedTokenSource {
    async fn token(
        &self,
        request: &AuthTokenRequest,
        force_refresh: bool,
    ) -> Result<Option<String>, AreteError> {
        self.inner
            .token(&self.retarget(request), force_refresh)
            .await
    }

    fn invalidate(&self, request: &AuthTokenRequest) {
        self.inner.invalidate(&self.retarget(request));
    }
}

/// Construct explicit hosted chain and transaction transports from generated
/// gateway descriptors (port of the TS
/// `createHostedSolanaGatewayTransports`).
///
/// Both bindings are fully validated first (`sgb_` binding IDs, https or
/// loopback endpoints, `auth.mode == authPolicy`, bearer token transport,
/// `arete:solana-gateway` audience, target consistency, and the required
/// scopes — `read` for chain, `transaction:inspect` + `transaction:send` for
/// transactions). Each capability then gets a token source targeting its own
/// `solana-gateway-binding`; token-minting state is shared between the two
/// capabilities when they resolve to the same strategy identity (runtime
/// strategy, or the same session endpoint), mirroring the TS manager cache.
pub fn create_hosted_solana_gateway_transports(
    bindings: &HostedSolanaGatewayBindings,
    auth: Option<AuthConfig>,
    http: Option<reqwest::Client>,
) -> Result<HostedSolanaGatewayTransports, AreteError> {
    validate_gateway_binding("chain", &bindings.chain, CHAIN_REQUIRED_SCOPES)?;
    validate_gateway_binding(
        "transactions",
        &bindings.transactions,
        TRANSACTIONS_REQUIRED_SCOPES,
    )?;

    let http = http.unwrap_or_default();
    let mut clients: HashMap<String, Arc<HttpAuthClient>> = HashMap::new();
    let mut token_source_for =
        |binding: &HostedSolanaGatewayCapabilityBinding| -> Arc<dyn TokenSource> {
            let effective = binding_auth_config(binding, auth.as_ref());
            let identity = if has_runtime_auth_strategy(auth.as_ref()) {
                "runtime-auth-strategy".to_string()
            } else {
                format!("session-endpoint:{}", binding.auth.session_endpoint)
            };
            let client = clients
                .entry(identity)
                .or_insert_with(|| Arc::new(HttpAuthClient::new(effective, None, http.clone())))
                .clone();
            Arc::new(TargetedTokenSource {
                inner: client,
                target: AuthTokenTarget::solana_gateway_binding(
                    binding.solana_gateway_binding_id.clone(),
                ),
            })
        };

    let chain_tokens = token_source_for(&bindings.chain);
    let transaction_tokens = token_source_for(&bindings.transactions);

    let chain: Arc<dyn ChainClient> = Arc::new(HttpChainClient::with_http_client(
        bindings.chain.endpoint.clone(),
        chain_tokens,
        http.clone(),
    ));
    let transactions: Arc<dyn TransactionTransport> =
        Arc::new(HttpTransactionTransport::with_http_client(
            bindings.transactions.endpoint.clone(),
            transaction_tokens,
            http,
        ));
    Ok((chain, transactions))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::{json, Value};
    use std::sync::Mutex;
    use tokio::net::TcpListener;

    const CHAIN_ID: &str = "sgb_0000000000000000000000000000chn1";
    const TX_ID: &str = "sgb_00000000000000000000000000000tx1";

    fn metadata(id: &str, scopes: &[&str], session_endpoint: &str) -> SolanaGatewayAuthMetadata {
        SolanaGatewayAuthMetadata {
            required: true,
            mode: "publishable-key".to_string(),
            session_endpoint: session_endpoint.to_string(),
            jwks_url: "https://gateway.example/jwks.json".to_string(),
            token_transport: "bearer".to_string(),
            audience: "arete:solana-gateway".to_string(),
            target_kind: "solana-gateway-binding".to_string(),
            target_id: id.to_string(),
            scopes: scopes.iter().map(|scope| scope.to_string()).collect(),
            accepted_key_classes: vec!["publishable".to_string()],
            transaction_entitlement_required: false,
        }
    }

    fn binding(
        id: &str,
        endpoint: &str,
        scopes: &[&str],
        session_endpoint: &str,
    ) -> HostedSolanaGatewayCapabilityBinding {
        HostedSolanaGatewayCapabilityBinding {
            endpoint: endpoint.to_string(),
            auth_policy: "publishable-key".to_string(),
            solana_gateway_binding_id: id.to_string(),
            cluster: "mainnet-beta".to_string(),
            region: "iad".to_string(),
            auth: metadata(id, scopes, session_endpoint),
        }
    }

    fn valid_bindings() -> HostedSolanaGatewayBindings {
        HostedSolanaGatewayBindings {
            chain: binding(
                CHAIN_ID,
                "https://gateway.example/chain",
                &["read"],
                "https://gateway.example/sessions",
            ),
            transactions: binding(
                TX_ID,
                "https://gateway.example/transactions",
                &["transaction:inspect", "transaction:send"],
                "https://gateway.example/sessions",
            ),
        }
    }

    fn assert_invalid(bindings: HostedSolanaGatewayBindings) {
        let error = create_hosted_solana_gateway_transports(&bindings, None, None)
            .err()
            .unwrap();
        assert!(
            matches!(error, AreteError::InvalidConfig(ref message)
                if message.contains("Hosted Solana gateway")),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn valid_bindings_construct_transports() {
        assert!(create_hosted_solana_gateway_transports(&valid_bindings(), None, None).is_ok());
    }

    #[test]
    fn validation_rejects_inconsistent_bindings() {
        // Insecure endpoint.
        let mut bindings = valid_bindings();
        bindings.chain.endpoint = "http://gateway.example/chain".to_string();
        assert_invalid(bindings);

        // Non-canonical binding ID.
        let mut bindings = valid_bindings();
        bindings.chain.solana_gateway_binding_id = "sgb_short".to_string();
        bindings.chain.auth.target_id = "sgb_short".to_string();
        assert_invalid(bindings);

        // Empty cluster.
        let mut bindings = valid_bindings();
        bindings.transactions.cluster = "  ".to_string();
        assert_invalid(bindings);

        // auth.mode must equal authPolicy.
        let mut bindings = valid_bindings();
        bindings.chain.auth.mode = "jwt".to_string();
        assert_invalid(bindings);

        // Insecure session endpoint.
        let mut bindings = valid_bindings();
        bindings.chain.auth.session_endpoint = "http://gateway.example/sessions".to_string();
        assert_invalid(bindings);

        // Insecure jwks url.
        let mut bindings = valid_bindings();
        bindings.transactions.auth.jwks_url = "ftp://gateway.example/jwks".to_string();
        assert_invalid(bindings);

        // Token transport must be bearer.
        let mut bindings = valid_bindings();
        bindings.chain.auth.token_transport = "query".to_string();
        assert_invalid(bindings);

        // Audience must be arete:solana-gateway.
        let mut bindings = valid_bindings();
        bindings.chain.auth.audience = "arete:other".to_string();
        assert_invalid(bindings);

        // Target kind must be solana-gateway-binding.
        let mut bindings = valid_bindings();
        bindings.chain.auth.target_kind = "program-read-binding".to_string();
        assert_invalid(bindings);

        // Target ID must match the binding ID.
        let mut bindings = valid_bindings();
        bindings.transactions.auth.target_id = CHAIN_ID.to_string();
        assert_invalid(bindings);

        // Missing required scope (transactions needs transaction:send).
        let mut bindings = valid_bindings();
        bindings.transactions.auth.scopes = vec!["transaction:inspect".to_string()];
        assert_invalid(bindings);

        // Missing required scope (chain needs read).
        let mut bindings = valid_bindings();
        bindings.chain.auth.scopes = vec!["transaction:inspect".to_string()];
        assert_invalid(bindings);
    }

    #[test]
    fn loopback_endpoints_are_accepted() {
        let bindings = HostedSolanaGatewayBindings {
            chain: binding(
                CHAIN_ID,
                "http://127.0.0.1:9/chain",
                &["read"],
                "http://localhost:9/sessions",
            ),
            transactions: binding(
                TX_ID,
                "http://[::1]:9/transactions",
                &["transaction:inspect", "transaction:send"],
                "http://127.0.0.1:9/sessions",
            ),
        };
        assert!(create_hosted_solana_gateway_transports(&bindings, None, None).is_ok());
    }

    #[test]
    fn bindings_deserialize_from_ts_generated_json() {
        let value = json!({
            "chain": {
                "endpoint": "https://gateway.example/chain",
                "authPolicy": "publishable-key",
                "solanaGatewayBindingId": CHAIN_ID,
                "cluster": "mainnet-beta",
                "region": "iad",
                "auth": {
                    "required": true,
                    "mode": "publishable-key",
                    "sessionEndpoint": "https://gateway.example/sessions",
                    "jwksUrl": "https://gateway.example/jwks.json",
                    "tokenTransport": "bearer",
                    "audience": "arete:solana-gateway",
                    "targetKind": "solana-gateway-binding",
                    "targetId": CHAIN_ID,
                    "scopes": ["read"],
                    "acceptedKeyClasses": ["publishable"],
                    "transactionEntitlementRequired": false
                }
            },
            "transactions": {
                "endpoint": "https://gateway.example/transactions",
                "authPolicy": "publishable-key",
                "solanaGatewayBindingId": TX_ID,
                "cluster": "mainnet-beta",
                "region": "iad",
                "auth": {
                    "required": true,
                    "mode": "publishable-key",
                    "sessionEndpoint": "https://gateway.example/sessions",
                    "jwksUrl": "https://gateway.example/jwks.json",
                    "tokenTransport": "bearer",
                    "audience": "arete:solana-gateway",
                    "targetKind": "solana-gateway-binding",
                    "targetId": TX_ID,
                    "scopes": ["transaction:inspect", "transaction:send"],
                    "acceptedKeyClasses": ["publishable"],
                    "transactionEntitlementRequired": false
                }
            }
        });
        let parsed: HostedSolanaGatewayBindings = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(parsed, valid_bindings());
        assert_eq!(serde_json::to_value(&parsed).unwrap(), value);
    }

    #[derive(Clone, Default)]
    struct GatewayServerState {
        token_bodies: Arc<Mutex<Vec<Value>>>,
        chain_auth_headers: Arc<Mutex<Vec<Option<String>>>>,
    }

    async fn spawn_gateway(state: GatewayServerState) -> String {
        async fn sessions(
            State(state): State<GatewayServerState>,
            Json(body): Json<Value>,
        ) -> Json<Value> {
            state.token_bodies.lock().unwrap().push(body);
            Json(json!({ "token": "gateway-token", "expires_at": 4_102_444_800u64 }))
        }

        async fn clock(State(state): State<GatewayServerState>, headers: HeaderMap) -> Json<Value> {
            state.chain_auth_headers.lock().unwrap().push(
                headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string),
            );
            Json(json!({ "slot": 123u64, "epoch": 5u64, "unixTimestamp": 1_700_000_000i64 }))
        }

        let router = Router::new()
            .route("/sessions", post(sessions))
            .route("/chain/clock", get(clock))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn end_to_end_chain_read_uses_targeted_session_tokens() {
        let state = GatewayServerState::default();
        let base = spawn_gateway(state.clone()).await;
        let bindings = HostedSolanaGatewayBindings {
            chain: binding(CHAIN_ID, &base, &["read"], &format!("{base}/sessions")),
            transactions: binding(
                TX_ID,
                &base,
                &["transaction:inspect", "transaction:send"],
                &format!("{base}/sessions"),
            ),
        };
        // No runtime strategy: tokens are minted from the binding session
        // endpoint with the exact solana-gateway-binding target.
        let auth = AuthConfig::default().with_publishable_key("hspk_test");
        let (chain, _transactions) =
            create_hosted_solana_gateway_transports(&bindings, Some(auth), None).unwrap();

        let clock = chain.clock().await.unwrap();
        assert_eq!(clock.slot, 123);

        let bodies = state.token_bodies.lock().unwrap();
        assert_eq!(
            bodies.as_slice(),
            &[json!({
                "targetKind": "solana-gateway-binding",
                "targetId": CHAIN_ID,
                "scopes": ["read"],
            })]
        );
        let auth_headers = state.chain_auth_headers.lock().unwrap();
        assert_eq!(
            auth_headers.as_slice(),
            &[Some("Bearer gateway-token".to_string())]
        );
    }
}
