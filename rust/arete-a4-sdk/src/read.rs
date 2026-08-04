//! Program and stack HTTP read primitives.
//!
//! Port of `typescript/core/src/read.ts` plus the program-read descriptor
//! validation and account value parsing from `typescript/core/src/client.ts`
//! (`validateProgramReadDescriptor`, `parseProgramAccountValue`,
//! `normalizeProgramAccountWireKeys`). Wire behavior follows
//! `docs/internal/sdk-api-surface.md` §2.2/§3.3 and the
//! `program-read-http/v1` contract fixture.

use std::marker::PhantomData;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::program_read_transport::{
    is_refreshable_error_code, BearerTokenSource, ProgramReadRequest, ProgramReadTransport,
};

const READ_SCOPES: &[&str] = &["read"];

// ---------------------------------------------------------------------------
// Release / binding descriptor types (TS `types.ts`, camelCase on the wire)
// ---------------------------------------------------------------------------

/// Generated release identity for a program (TS `ProgramReleaseReference`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramReleaseReference {
    pub program_release_hash: String,
    pub program_spec_hash: String,
}

/// Public, non-secret metadata describing how an HTTP bearer token is
/// acquired (TS `HttpAuthMetadata`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpAuthMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    pub session_endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jwks_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_transport: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience: Option<String>,
    /// Must be `"program-read-binding"` for program read bindings.
    pub target_kind: String,
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_key_classes: Option<Vec<String>>,
}

/// One generated, non-inheriting hosted program read binding
/// (TS `ProgramReadBinding`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramReadBinding {
    pub endpoint: String,
    pub program_read_binding_id: String,
    pub auth: HttpAuthMetadata,
}

/// Transport discriminant of a [`ProgramReadDescriptor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramReadTransportKind {
    LocalHttp,
    HostedBinding,
}

/// Generated release identity with one explicit, non-inheriting read
/// transport (TS `ProgramReadDescriptor`).
///
/// Serializes to the exact TS-generated JSON shape:
/// `{"release": …, "transport": {"kind": "local-http", "endpointSource":
/// "connect-http-url"}}` or `{"release": …, "transport": {"kind":
/// "hosted-binding", "binding": …}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
// Descriptors are configuration values created once per program; the size
// difference between variants is irrelevant and boxing would obscure the
// TS-mirrored shape.
#[allow(clippy::large_enum_variant)]
pub enum ProgramReadDescriptor {
    LocalHttp {
        release: ProgramReleaseReference,
    },
    HostedBinding {
        release: ProgramReleaseReference,
        binding: ProgramReadBinding,
    },
}

impl ProgramReadDescriptor {
    pub fn release(&self) -> &ProgramReleaseReference {
        match self {
            Self::LocalHttp { release } | Self::HostedBinding { release, .. } => release,
        }
    }

    pub fn transport_kind(&self) -> ProgramReadTransportKind {
        match self {
            Self::LocalHttp { .. } => ProgramReadTransportKind::LocalHttp,
            Self::HostedBinding { .. } => ProgramReadTransportKind::HostedBinding,
        }
    }

    pub fn binding(&self) -> Option<&ProgramReadBinding> {
        match self {
            Self::LocalHttp { .. } => None,
            Self::HostedBinding { binding, .. } => Some(binding),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescriptorWire {
    release: ProgramReleaseReference,
    transport: TransportWire,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind")]
#[allow(clippy::large_enum_variant)]
enum TransportWire {
    #[serde(rename = "local-http", rename_all = "camelCase")]
    LocalHttp { endpoint_source: EndpointSource },
    #[serde(rename = "hosted-binding")]
    HostedBinding { binding: ProgramReadBinding },
}

/// Local HTTP descriptors must use the connect-time HTTP endpoint; any other
/// `endpointSource` is rejected during deserialization (mirrors the TS
/// `endpointSource !== 'connect-http-url'` validation rule).
#[derive(Serialize, Deserialize)]
enum EndpointSource {
    #[serde(rename = "connect-http-url")]
    ConnectHttpUrl,
}

impl Serialize for ProgramReadDescriptor {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let wire = match self {
            Self::LocalHttp { release } => DescriptorWire {
                release: release.clone(),
                transport: TransportWire::LocalHttp {
                    endpoint_source: EndpointSource::ConnectHttpUrl,
                },
            },
            Self::HostedBinding { release, binding } => DescriptorWire {
                release: release.clone(),
                transport: TransportWire::HostedBinding {
                    binding: binding.clone(),
                },
            },
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ProgramReadDescriptor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = DescriptorWire::deserialize(deserializer)?;
        Ok(match wire.transport {
            TransportWire::LocalHttp { .. } => Self::LocalHttp {
                release: wire.release,
            },
            TransportWire::HostedBinding { binding } => Self::HostedBinding {
                release: wire.release,
                binding,
            },
        })
    }
}

// ---------------------------------------------------------------------------
// Descriptor validation (TS `client.ts` `validateProgramReadDescriptor`)
// ---------------------------------------------------------------------------

fn is_non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

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

/// `^prb_[A-Za-z0-9_-]{32}$`
fn is_canonical_binding_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("prb_") else {
        return false;
    };
    rest.len() == 32
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Port of TS `validateProgramReadDescriptor`: complete release hashes; for
/// hosted bindings a secure (or loopback-http) endpoint and session endpoint,
/// a canonical `prb_…` binding ID, and matching `program-read-binding` auth
/// target metadata.
pub fn validate_program_read_descriptor(
    program_name: &str,
    descriptor: &ProgramReadDescriptor,
) -> Result<(), ReadError> {
    let release = descriptor.release();
    if !is_non_empty(&release.program_release_hash) || !is_non_empty(&release.program_spec_hash) {
        return Err(ReadError::InvalidConfig {
            message: format!(
                "Program '{program_name}' read descriptor requires a complete release"
            ),
        });
    }
    match descriptor {
        ProgramReadDescriptor::LocalHttp { .. } => Ok(()),
        ProgramReadDescriptor::HostedBinding { binding, .. } => {
            let auth = &binding.auth;
            let valid = is_secure_or_loopback_http_url(&binding.endpoint)
                && is_canonical_binding_id(&binding.program_read_binding_id)
                && auth.target_kind == "program-read-binding"
                && auth.target_id == binding.program_read_binding_id
                && is_secure_or_loopback_http_url(&auth.session_endpoint);
            if valid {
                Ok(())
            } else {
                Err(ReadError::InvalidConfig {
                    message: format!(
                        "Program '{program_name}' hosted binding requires secure endpoints, a canonical binding ID, and matching program-read-binding auth metadata"
                    ),
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Failed (non-2xx) HTTP read (TS `ReadRequestError`).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("Read request to '{path}' failed ({status}): {body}")]
pub struct ReadRequestError {
    pub status: u16,
    pub path: String,
    pub body: String,
    /// `X-Error-Code` header if present, otherwise the error code found in
    /// the response body. Preserved verbatim from the wire.
    pub server_error_code: Option<String>,
}

/// Errors produced by the program/stack HTTP read surface.
#[derive(Debug, Error)]
pub enum ReadError {
    #[error(transparent)]
    Request(#[from] ReadRequestError),

    /// A 2xx response carried a body that was not valid JSON
    /// (TS `INVALID_RESPONSE`).
    #[error("Program read '{path}' returned invalid JSON")]
    InvalidResponse {
        path: String,
        #[source]
        source: serde_json::Error,
    },

    /// Invalid descriptor or configuration (TS `INVALID_CONFIG`).
    #[error("{message}")]
    InvalidConfig { message: String },

    /// The decoded account value did not match the generated account type,
    /// even after wire-key normalization.
    #[error("Program account read '{account}' failed schema validation")]
    SchemaValidation { account: String },

    /// The query result did not match the requested result type.
    #[error("Query '{name}' failed schema validation")]
    QueryValidation { name: String },

    #[error("Read request to '{path}' failed: {source}")]
    Network {
        path: String,
        #[source]
        source: reqwest::Error,
    },

    /// Token acquisition failed.
    #[error(transparent)]
    Auth(#[from] crate::error::AreteError),
}

// ---------------------------------------------------------------------------
// Account value parsing (TS `parseProgramAccountValue` +
// `normalizeProgramAccountWireKeys`)
// ---------------------------------------------------------------------------

/// Port of TS `normalizeProgramAccountWireKeys`: recursively rewrite
/// camelCase object keys to snake_case (each ASCII uppercase letter becomes
/// its lowercase form, prefixed with `_` unless it is the first character).
fn normalize_program_account_wire_keys(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(normalize_program_account_wire_keys)
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, nested)| {
                    (
                        camel_to_snake(&key),
                        normalize_program_account_wire_keys(nested),
                    )
                })
                .collect(),
        ),
        other => other,
    }
}

fn camel_to_snake(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for (index, ch) in key.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index != 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Deserialize a raw account wire value into the generated account type,
/// retrying once with camelCase→snake_case key normalization (TS
/// `parseProgramAccountValue`).
fn parse_program_account_value<T: DeserializeOwned>(
    account: &str,
    value: Value,
) -> Result<T, ReadError> {
    match serde_json::from_value::<T>(value.clone()) {
        Ok(parsed) => Ok(parsed),
        Err(_) => {
            serde_json::from_value::<T>(normalize_program_account_wire_keys(value)).map_err(|_| {
                ReadError::SchemaValidation {
                    account: account.to_string(),
                }
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Typed account reader (TS `TypedAccountReader`)
// ---------------------------------------------------------------------------

/// One item of a batched account read (TS `ProgramAccountBatchResult` item,
/// discriminated by `status`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountBatchItem<T> {
    Ok { address: String, value: T },
    Missing { address: String },
    Error { address: String, code: String },
}

impl<T> AccountBatchItem<T> {
    pub fn address(&self) -> &str {
        match self {
            Self::Ok { address, .. } | Self::Missing { address } | Self::Error { address, .. } => {
                address
            }
        }
    }
}

/// Result of [`AccountReader::fetch_many`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountBatchResult<T> {
    pub items: Vec<AccountBatchItem<T>>,
}

#[derive(Deserialize)]
struct WireBatch {
    items: Vec<WireBatchItem>,
}

#[derive(Deserialize)]
#[serde(tag = "status")]
enum WireBatchItem {
    #[serde(rename = "ok")]
    Ok { address: String, value: Value },
    #[serde(rename = "missing")]
    Missing { address: String },
    #[serde(rename = "error")]
    Error {
        address: String,
        error: WireBatchErrorCode,
    },
}

#[derive(Deserialize)]
struct WireBatchErrorCode {
    code: String,
}

#[derive(Deserialize)]
struct ExistsResponse {
    exists: bool,
}

/// Typed reader over one generated program account (TS
/// `client.programs.<name>.accounts.<Account>`), backed by a
/// [`ProgramReadTransport`].
#[derive(Clone)]
pub struct AccountReader<T> {
    account: String,
    transport: Arc<ProgramReadTransport>,
    _marker: PhantomData<fn() -> T>,
}

impl<T> AccountReader<T> {
    pub fn new(account: impl Into<String>, transport: Arc<ProgramReadTransport>) -> Self {
        Self {
            account: account.into(),
            transport,
            _marker: PhantomData,
        }
    }

    pub fn account(&self) -> &str {
        &self.account
    }
}

impl<T: DeserializeOwned> AccountReader<T> {
    /// Fetch one decoded account. A wire body of `null` means the account is
    /// missing and yields `Ok(None)`.
    pub async fn fetch(&self, address: &str) -> Result<Option<T>, ReadError> {
        let request = ProgramReadRequest::Fetch {
            account: &self.account,
            address,
        };
        let value = self.transport.read(&request).await?;
        if value.is_null() {
            return Ok(None);
        }
        parse_program_account_value(&self.account, value).map(Some)
    }

    /// Fetch a mixed batch; per-address `ok`/`missing`/`error` statuses are
    /// preserved (TS `fetchMany`).
    pub async fn fetch_many(&self, addresses: &[&str]) -> Result<AccountBatchResult<T>, ReadError> {
        let request = ProgramReadRequest::FetchMany {
            account: &self.account,
            addresses,
        };
        let path = self.transport.request_path(&request);
        let value = self.transport.read(&request).await?;
        let batch: WireBatch = serde_json::from_value(value)
            .map_err(|source| ReadError::InvalidResponse { path, source })?;
        let mut items = Vec::with_capacity(batch.items.len());
        for item in batch.items {
            items.push(match item {
                WireBatchItem::Ok { address, value } => AccountBatchItem::Ok {
                    address,
                    value: parse_program_account_value(&self.account, value)?,
                },
                WireBatchItem::Missing { address } => AccountBatchItem::Missing { address },
                WireBatchItem::Error { address, error } => AccountBatchItem::Error {
                    address,
                    code: error.code,
                },
            });
        }
        Ok(AccountBatchResult { items })
    }

    /// Existence probe (`…/<address>/exists` → `{"exists": bool}`).
    pub async fn exists(&self, address: &str) -> Result<bool, ReadError> {
        let request = ProgramReadRequest::Exists {
            account: &self.account,
            address,
        };
        let path = self.transport.request_path(&request);
        let value = self.transport.read(&request).await?;
        let parsed: ExistsResponse = serde_json::from_value(value)
            .map_err(|source| ReadError::InvalidResponse { path, source })?;
        Ok(parsed.exists)
    }
}

// ---------------------------------------------------------------------------
// Queries (TS `programQuery` / `stackQuery` + `createQueryExecutor`)
// ---------------------------------------------------------------------------

/// HTTP method for query definitions (TS `ReadTransportMethod`, default POST).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReadMethod {
    Get,
    #[default]
    Post,
}

/// Generated program-scoped query definition (TS `ProgramQueryDefinition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramQueryDef {
    pub name: String,
    pub path: String,
    pub method: ReadMethod,
}

impl ProgramQueryDef {
    pub fn new(name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
            method: ReadMethod::default(),
        }
    }
}

/// Generated stack-scoped query definition (TS `StackQueryDefinition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackQueryDef {
    pub name: String,
    pub path: String,
    pub method: ReadMethod,
}

impl StackQueryDef {
    pub fn new(name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
            method: ReadMethod::default(),
        }
    }
}

/// `X-Error-Code` fallback used by stack/query reads: only the top-level
/// `code` body field is consulted (TS `read.ts` `getServerErrorCode`).
fn top_level_error_code(body: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    parsed.get("code")?.as_str().map(str::to_string)
}

fn resolve_read_url(base: &str, path: &str) -> String {
    let base = base.strip_suffix('/').unwrap_or(base);
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

/// Executes stack/program queries against a stack HTTP base URL (TS
/// `createQueryExecutor` + `readJson`): JSON params, `read`-scoped bearer
/// auth, refresh-replay on a refresh-worthy `X-Error-Code`, and typed JSON
/// results.
#[derive(Clone)]
pub struct QueryExecutor {
    http_base: String,
    http: reqwest::Client,
    auth: Option<Arc<dyn BearerTokenSource>>,
}

impl QueryExecutor {
    pub fn new(http_base: impl Into<String>, http: reqwest::Client) -> Self {
        Self {
            http_base: http_base.into(),
            http,
            auth: None,
        }
    }

    pub fn with_auth(mut self, auth: Arc<dyn BearerTokenSource>) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Execute a program-scoped query.
    pub async fn execute<TParams, TResult>(
        &self,
        query: &ProgramQueryDef,
        params: &TParams,
    ) -> Result<TResult, ReadError>
    where
        TParams: Serialize + ?Sized,
        TResult: DeserializeOwned,
    {
        self.run(&query.name, &query.path, query.method, params)
            .await
    }

    /// Execute a stack-scoped query.
    pub async fn execute_stack<TParams, TResult>(
        &self,
        query: &StackQueryDef,
        params: &TParams,
    ) -> Result<TResult, ReadError>
    where
        TParams: Serialize + ?Sized,
        TResult: DeserializeOwned,
    {
        self.run(&query.name, &query.path, query.method, params)
            .await
    }

    async fn run<TParams, TResult>(
        &self,
        name: &str,
        path: &str,
        method: ReadMethod,
        params: &TParams,
    ) -> Result<TResult, ReadError>
    where
        TParams: Serialize + ?Sized,
        TResult: DeserializeOwned,
    {
        let url = resolve_read_url(&self.http_base, path);
        let mut outcome = self.attempt(&url, path, method, params, false).await?;
        if !(200..300).contains(&outcome.0) {
            let refreshable = outcome
                .1
                .as_deref()
                .map(is_refreshable_error_code)
                .unwrap_or(false);
            if refreshable {
                if let Some(auth) = &self.auth {
                    auth.invalidate(READ_SCOPES, None);
                    outcome = self.attempt(&url, path, method, params, true).await?;
                }
            }
        }
        let (status, header_code, body) = outcome;
        if !(200..300).contains(&status) {
            let server_error_code = header_code.or_else(|| top_level_error_code(&body));
            return Err(ReadRequestError {
                status,
                path: path.to_string(),
                body,
                server_error_code,
            }
            .into());
        }
        let value: Value =
            serde_json::from_str(&body).map_err(|source| ReadError::InvalidResponse {
                path: path.to_string(),
                source,
            })?;
        serde_json::from_value(value).map_err(|_| ReadError::QueryValidation {
            name: name.to_string(),
        })
    }

    async fn attempt<TParams>(
        &self,
        url: &str,
        path: &str,
        method: ReadMethod,
        params: &TParams,
        force_refresh: bool,
    ) -> Result<(u16, Option<String>, String), ReadError>
    where
        TParams: Serialize + ?Sized,
    {
        let token = match &self.auth {
            Some(auth) => auth.bearer_token(READ_SCOPES, None, force_refresh).await?,
            None => None,
        };
        let mut builder = match method {
            ReadMethod::Post => self.http.post(url).json(params),
            ReadMethod::Get => self.http.get(url),
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program_read_transport::test_support::{
        fixture, test_binding, test_release, CannedResponse, MockTokenSource, TestServer,
    };
    use serde_json::json;

    #[derive(Debug, Deserialize, PartialEq)]
    struct FixtureAccount {
        value: String,
        count: u64,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Inner {
        inner_value: String,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct NormalizedAccount {
        value_count: u64,
        inner: Inner,
        items: Vec<Inner>,
    }

    fn reader<T>(server: &TestServer) -> AccountReader<T> {
        AccountReader::new(
            "State",
            Arc::new(ProgramReadTransport::local_http(
                server.base_url.clone(),
                test_release(),
                reqwest::Client::new(),
            )),
        )
    }

    fn valid_hosted_descriptor() -> ProgramReadDescriptor {
        ProgramReadDescriptor::HostedBinding {
            release: test_release(),
            binding: test_binding("https://reads.example.test", None),
        }
    }

    // -- descriptor serde -------------------------------------------------

    #[test]
    fn descriptor_serde_matches_ts_wire_shape() {
        let local_json = json!({
            "release": {
                "programReleaseHash": "release-alpha",
                "programSpecHash": "spec-alpha"
            },
            "transport": {
                "kind": "local-http",
                "endpointSource": "connect-http-url"
            }
        });
        let local: ProgramReadDescriptor = serde_json::from_value(local_json.clone()).unwrap();
        assert_eq!(
            local,
            ProgramReadDescriptor::LocalHttp {
                release: test_release()
            }
        );
        assert_eq!(serde_json::to_value(&local).unwrap(), local_json);
        assert_eq!(local.transport_kind(), ProgramReadTransportKind::LocalHttp);

        let hosted_json = json!({
            "release": {
                "programReleaseHash": "release-alpha",
                "programSpecHash": "spec-alpha"
            },
            "transport": {
                "kind": "hosted-binding",
                "binding": {
                    "endpoint": "https://reads.example.test",
                    "programReadBindingId": "prb_00000000000000000000000000000001",
                    "auth": {
                        "sessionEndpoint": "https://auth.example.test/session",
                        "targetKind": "program-read-binding",
                        "targetId": "prb_00000000000000000000000000000001",
                        "scopes": ["read"]
                    }
                }
            }
        });
        let hosted: ProgramReadDescriptor = serde_json::from_value(hosted_json.clone()).unwrap();
        assert_eq!(
            hosted.transport_kind(),
            ProgramReadTransportKind::HostedBinding
        );
        assert_eq!(
            hosted.binding().unwrap().program_read_binding_id,
            "prb_00000000000000000000000000000001"
        );
        assert_eq!(serde_json::to_value(&hosted).unwrap(), hosted_json);
    }

    #[test]
    fn descriptor_deserialization_rejects_unknown_endpoint_source() {
        let bad = json!({
            "release": {
                "programReleaseHash": "release-alpha",
                "programSpecHash": "spec-alpha"
            },
            "transport": {
                "kind": "local-http",
                "endpointSource": "stack-http"
            }
        });
        assert!(serde_json::from_value::<ProgramReadDescriptor>(bad).is_err());
    }

    // -- descriptor validation table --------------------------------------

    fn hosted_with(
        mutate: impl FnOnce(&mut ProgramReleaseReference, &mut ProgramReadBinding),
    ) -> ProgramReadDescriptor {
        let mut release = test_release();
        let mut binding = test_binding("https://reads.example.test", None);
        mutate(&mut release, &mut binding);
        ProgramReadDescriptor::HostedBinding { release, binding }
    }

    #[test]
    fn validates_descriptor_rules() {
        // Accepted descriptors.
        let accepted = [
            (
                "local http",
                ProgramReadDescriptor::LocalHttp {
                    release: test_release(),
                },
            ),
            ("hosted https", valid_hosted_descriptor()),
            (
                "hosted http localhost",
                hosted_with(|_, binding| binding.endpoint = "http://localhost:8899".into()),
            ),
            (
                "hosted http 127.0.0.1",
                hosted_with(|_, binding| binding.endpoint = "http://127.0.0.1:1234/prefix".into()),
            ),
            (
                "hosted loopback session endpoint",
                hosted_with(|_, binding| {
                    binding.auth.session_endpoint = "http://127.0.0.1:9000/session".into()
                }),
            ),
        ];
        for (label, descriptor) in accepted {
            assert!(
                validate_program_read_descriptor("alpha", &descriptor).is_ok(),
                "expected '{label}' to validate"
            );
        }

        // Rejected descriptors.
        let rejected = [
            (
                "empty release hash",
                hosted_with(|release, _| release.program_release_hash = "".into()),
            ),
            (
                "whitespace spec hash",
                hosted_with(|release, _| release.program_spec_hash = "   ".into()),
            ),
            (
                "local with empty release",
                ProgramReadDescriptor::LocalHttp {
                    release: ProgramReleaseReference {
                        program_release_hash: "".into(),
                        program_spec_hash: "spec-alpha".into(),
                    },
                },
            ),
            (
                "insecure endpoint scheme",
                hosted_with(|_, binding| binding.endpoint = "http://reads.example.test".into()),
            ),
            (
                "unparseable endpoint",
                hosted_with(|_, binding| binding.endpoint = "not a url".into()),
            ),
            (
                "insecure session endpoint scheme",
                hosted_with(|_, binding| {
                    binding.auth.session_endpoint = "http://auth.example.test/session".into()
                }),
            ),
            (
                "empty session endpoint",
                hosted_with(|_, binding| binding.auth.session_endpoint = "".into()),
            ),
            (
                "short binding id",
                hosted_with(|_, binding| {
                    binding.program_read_binding_id = "prb_too-short".into();
                    binding.auth.target_id = "prb_too-short".into();
                }),
            ),
            (
                "invalid binding id character",
                hosted_with(|_, binding| {
                    let id = format!("prb_{}!", "0".repeat(31));
                    binding.program_read_binding_id = id.clone();
                    binding.auth.target_id = id;
                }),
            ),
            (
                "wrong auth target kind",
                hosted_with(|_, binding| {
                    binding.auth.target_kind = "solana-gateway-binding".into()
                }),
            ),
            (
                "mismatched auth target id",
                hosted_with(|_, binding| {
                    binding.auth.target_id = "prb_00000000000000000000000000000002".into()
                }),
            ),
        ];
        for (label, descriptor) in rejected {
            assert!(
                validate_program_read_descriptor("alpha", &descriptor).is_err(),
                "expected '{label}' to be rejected"
            );
        }

        // Exact TS error messages.
        let err = validate_program_read_descriptor(
            "alpha",
            &ProgramReadDescriptor::LocalHttp {
                release: ProgramReleaseReference {
                    program_release_hash: "".into(),
                    program_spec_hash: "".into(),
                },
            },
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Program 'alpha' read descriptor requires a complete release"
        );
        let err = validate_program_read_descriptor(
            "alpha",
            &hosted_with(|_, binding| binding.endpoint = "http://reads.example.test".into()),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Program 'alpha' hosted binding requires secure endpoints, a canonical binding ID, and matching program-read-binding auth metadata"
        );
    }

    // -- key normalization ------------------------------------------------

    #[test]
    fn normalizes_camel_case_keys_like_ts() {
        assert_eq!(camel_to_snake("innerValue"), "inner_value");
        assert_eq!(camel_to_snake("Value"), "value");
        assert_eq!(camel_to_snake("ABC"), "a_b_c");
        assert_eq!(camel_to_snake("already_snake"), "already_snake");

        let normalized = normalize_program_account_wire_keys(json!({
            "valueCount": 7,
            "Nested": { "innerValue": "x" },
            "items": [{ "innerValue": "y" }]
        }));
        assert_eq!(
            normalized,
            json!({
                "value_count": 7,
                "nested": { "inner_value": "x" },
                "items": [{ "inner_value": "y" }]
            })
        );
    }

    // -- typed account reader over the contract fixture --------------------

    #[tokio::test]
    async fn fetch_decodes_typed_account_from_fixture() {
        let raw_value = fixture()["success"]["rawValue"].clone();
        let server = TestServer::spawn(vec![CannedResponse::json(raw_value.to_string())]).await;
        let reader = reader::<FixtureAccount>(&server);

        let account = reader.fetch("present").await.unwrap();
        assert_eq!(
            account,
            Some(FixtureAccount {
                value: "decoded".into(),
                count: 7
            })
        );
    }

    #[tokio::test]
    async fn fetch_returns_none_for_missing_account() {
        let server = TestServer::spawn(vec![CannedResponse::json("null")]).await;
        let reader = reader::<FixtureAccount>(&server);
        assert_eq!(reader.fetch("missing").await.unwrap(), None);
    }

    #[tokio::test]
    async fn exists_translates_fixture_object_to_bool() {
        let exists_body = fixture()["success"]["exists"].to_string();
        let server = TestServer::spawn(vec![CannedResponse::json(exists_body)]).await;
        let reader = reader::<FixtureAccount>(&server);
        assert!(reader.exists("present").await.unwrap());
    }

    #[tokio::test]
    async fn fetch_many_preserves_mixed_batch_statuses() {
        let batch_body = fixture()["success"]["batch"].to_string();
        let server = TestServer::spawn(vec![CannedResponse::json(batch_body)]).await;
        let reader = reader::<FixtureAccount>(&server);

        let result = reader
            .fetch_many(&["present", "missing", "broken"])
            .await
            .unwrap();
        assert_eq!(
            result,
            AccountBatchResult {
                items: vec![
                    AccountBatchItem::Ok {
                        address: "present".into(),
                        value: FixtureAccount {
                            value: "decoded".into(),
                            count: 7
                        },
                    },
                    AccountBatchItem::Missing {
                        address: "missing".into()
                    },
                    AccountBatchItem::Error {
                        address: "broken".into(),
                        code: "ACCOUNT_DECODE_FAILED".into(),
                    },
                ]
            }
        );

        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(
            serde_json::from_str::<Value>(&requests[0].body).unwrap(),
            json!({ "addresses": ["present", "missing", "broken"] })
        );
    }

    #[tokio::test]
    async fn fetch_retries_with_key_normalization_before_failing() {
        let camel_body = json!({
            "valueCount": 7,
            "inner": { "innerValue": "x" },
            "items": [{ "innerValue": "y" }]
        })
        .to_string();
        let server = TestServer::spawn(vec![CannedResponse::json(camel_body)]).await;
        let reader = reader::<NormalizedAccount>(&server);

        let account = reader.fetch("present").await.unwrap().unwrap();
        assert_eq!(
            account,
            NormalizedAccount {
                value_count: 7,
                inner: Inner {
                    inner_value: "x".into()
                },
                items: vec![Inner {
                    inner_value: "y".into()
                }],
            }
        );
    }

    #[tokio::test]
    async fn fetch_reports_schema_validation_failure() {
        let server = TestServer::spawn(vec![CannedResponse::json(r#"{"unexpected":true}"#)]).await;
        let reader = reader::<FixtureAccount>(&server);

        let err = reader.fetch("present").await.unwrap_err();
        assert!(matches!(
            &err,
            ReadError::SchemaValidation { account } if account == "State"
        ));
        assert_eq!(
            err.to_string(),
            "Program account read 'State' failed schema validation"
        );
    }

    // -- query executor ----------------------------------------------------

    #[derive(Debug, Serialize)]
    struct EchoParams {
        limit: u64,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct EchoResult {
        value: String,
    }

    #[tokio::test]
    async fn query_executor_posts_json_params_and_decodes_result() {
        let server = TestServer::spawn(vec![CannedResponse::json(r#"{"value":"ok"}"#)]).await;
        let executor =
            QueryExecutor::new(format!("{}/api/", server.base_url), reqwest::Client::new());
        let query = ProgramQueryDef::new("echo", "/queries/echo");

        let result: EchoResult = executor
            .execute(&query, &EchoParams { limit: 2 })
            .await
            .unwrap();
        assert_eq!(result, EchoResult { value: "ok".into() });

        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/api/queries/echo");
        assert_eq!(
            serde_json::from_str::<Value>(&requests[0].body).unwrap(),
            json!({ "limit": 2 })
        );
        assert!(requests[0]
            .content_type
            .as_deref()
            .unwrap_or_default()
            .starts_with("application/json"));
    }

    #[tokio::test]
    async fn query_executor_supports_get_without_body() {
        let server = TestServer::spawn(vec![CannedResponse::json(r#"{"value":"got"}"#)]).await;
        let executor = QueryExecutor::new(server.base_url.clone(), reqwest::Client::new());
        let query = StackQueryDef {
            name: "status".into(),
            path: "queries/status".into(),
            method: ReadMethod::Get,
        };

        let result: EchoResult = executor.execute_stack(&query, &()).await.unwrap();
        assert_eq!(result.value, "got");

        let requests = server.requests();
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path, "/queries/status");
        assert!(requests[0].body.is_empty());
    }

    #[tokio::test]
    async fn query_executor_surfaces_read_request_error_with_top_level_code() {
        let body = r#"{"error":"missing","code":"not-found"}"#;
        let server = TestServer::spawn(vec![CannedResponse::json(body).with_status(404)]).await;
        let executor = QueryExecutor::new(server.base_url.clone(), reqwest::Client::new());
        let query = ProgramQueryDef::new("echo", "/queries/echo");

        let err = executor
            .execute::<_, EchoResult>(&query, &EchoParams { limit: 1 })
            .await
            .unwrap_err();
        let ReadError::Request(request_error) = err else {
            panic!("expected ReadError::Request, got {err:?}");
        };
        assert_eq!(request_error.status, 404);
        assert_eq!(request_error.path, "/queries/echo");
        assert_eq!(request_error.body, body);
        assert_eq!(
            request_error.server_error_code.as_deref(),
            Some("not-found")
        );
        assert_eq!(
            request_error.to_string(),
            format!("Read request to '/queries/echo' failed (404): {body}")
        );
    }

    #[tokio::test]
    async fn query_executor_replays_once_on_refreshable_header_code() {
        let server = TestServer::spawn(vec![
            CannedResponse::json(r#"{"error":"expired"}"#)
                .with_status(401)
                .with_header("X-Error-Code", "token-expired"),
            CannedResponse::json(r#"{"value":"refreshed"}"#),
        ])
        .await;
        let auth = MockTokenSource::new();
        let executor = QueryExecutor::new(server.base_url.clone(), reqwest::Client::new())
            .with_auth(auth.clone());
        let query = ProgramQueryDef::new("echo", "/queries/echo");

        let result: EchoResult = executor
            .execute(&query, &EchoParams { limit: 1 })
            .await
            .unwrap();
        assert_eq!(result.value, "refreshed");

        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].authorization.as_deref(), Some("Bearer token-1"));
        assert_eq!(requests[1].authorization.as_deref(), Some("Bearer token-2"));
        let invalidations = auth.invalidations();
        assert_eq!(invalidations.len(), 1);
        assert_eq!(invalidations[0].scopes, vec!["read".to_string()]);
        assert!(invalidations[0].target.is_none());
        assert!(auth.bearer_calls()[1].force_refresh);
    }

    #[tokio::test]
    async fn query_executor_reports_result_schema_validation() {
        let server = TestServer::spawn(vec![CannedResponse::json(r#"{"value":42}"#)]).await;
        let executor = QueryExecutor::new(server.base_url.clone(), reqwest::Client::new());
        let query = ProgramQueryDef::new("echo", "/queries/echo");

        let err = executor
            .execute::<_, EchoResult>(&query, &EchoParams { limit: 1 })
            .await
            .unwrap_err();
        assert!(matches!(&err, ReadError::QueryValidation { name } if name == "echo"));
        assert_eq!(err.to_string(), "Query 'echo' failed schema validation");
    }
}
