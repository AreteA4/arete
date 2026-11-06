use std::collections::{BTreeMap, HashMap, HashSet};
use std::marker::PhantomData;
use std::sync::OnceLock;

use futures::future::join_all;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Context provided to primary key resolver functions
pub struct ResolveContext<'a> {
    #[allow(dead_code)]
    pub(crate) state_id: u32,
    pub(crate) slot: u64,
    pub(crate) signature: String,
    pub(crate) reverse_lookups:
        &'a mut std::collections::HashMap<String, crate::vm::PdaReverseLookup>,
}

impl<'a> ResolveContext<'a> {
    /// Create a new ResolveContext (primarily for use by generated code)
    pub fn new(
        state_id: u32,
        slot: u64,
        signature: String,
        reverse_lookups: &'a mut std::collections::HashMap<String, crate::vm::PdaReverseLookup>,
    ) -> Self {
        Self {
            state_id,
            slot,
            signature,
            reverse_lookups,
        }
    }

    /// Try to reverse lookup a PDA address to find the seed value
    /// This is typically used to find the primary key from a PDA account address
    pub fn pda_reverse_lookup(&mut self, pda_address: &str) -> Option<String> {
        let lookup_name = "default_pda_lookup";
        self.reverse_lookups
            .get_mut(lookup_name)
            .and_then(|t| t.lookup(pda_address))
    }

    pub fn slot(&self) -> u64 {
        self.slot
    }

    pub fn signature(&self) -> &str {
        &self.signature
    }
}

/// Result of attempting to resolve a primary key
pub enum KeyResolution {
    /// Primary key successfully resolved
    Found(String),

    /// Queue this update until we see one of these instruction discriminators
    /// The discriminators identify which instructions can populate the reverse lookup
    ///
    /// Owned rather than `&'static [u8]`: the bytes are never read after the
    /// variant is constructed, so nothing justified forcing callers that build
    /// them at runtime to leak them.
    QueueUntil(Vec<u8>),

    /// Skip this update entirely (don't queue)
    Skip,
}

/// Context provided to instruction hook functions
pub struct InstructionContext<'a> {
    pub(crate) accounts: HashMap<String, String>,
    #[allow(dead_code)]
    pub(crate) state_id: u32,
    pub(crate) reverse_lookup_tx: *mut (dyn ReverseLookupUpdater + 'a),
    pub(crate) pending_updates: Vec<crate::vm::PendingAccountUpdate>,
    pub(crate) registers: Option<*mut Vec<crate::vm::RegisterValue>>,
    pub(crate) state_reg: Option<crate::vm::Register>,
    #[allow(dead_code)]
    pub(crate) compiled_paths: Option<*const HashMap<String, crate::metrics_context::CompiledPath>>,
    pub(crate) instruction_data: Option<&'a serde_json::Value>,
    pub(crate) slot: Option<u64>,
    pub(crate) signature: Option<String>,
    pub(crate) timestamp: Option<i64>,
    update_context: Option<crate::UpdateContext>,
    pub(crate) dirty_tracker: crate::vm::DirtyTracker,
    _borrow: PhantomData<&'a mut ()>,
}

pub trait ReverseLookupUpdater {
    fn update(
        &mut self,
        pda_address: String,
        seed_value: String,
    ) -> Vec<crate::vm::PendingAccountUpdate>;
    fn flush_pending(&mut self, pda_address: &str) -> Vec<crate::vm::PendingAccountUpdate>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMetadata {
    pub mint: String,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub decimals: Option<u8>,
    pub logo_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedSlotHash {
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct ResolverTypeScriptSchema {
    pub name: &'static str,
    pub definition: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct ResolverComputedMethod {
    pub name: &'static str,
    pub arg_count: usize,
}

pub trait ResolverDefinition: Send + Sync {
    fn name(&self) -> &'static str;
    fn output_type(&self) -> &'static str;
    fn computed_methods(&self) -> &'static [ResolverComputedMethod];
    fn evaluate_computed(
        &self,
        method: &str,
        args: &[Value],
    ) -> std::result::Result<Value, Box<dyn std::error::Error>>;
    fn typescript_interface(&self) -> Option<&'static str> {
        None
    }
    fn typescript_schema(&self) -> Option<ResolverTypeScriptSchema> {
        None
    }
    fn extra_output_types(&self) -> &'static [&'static str] {
        &[]
    }
}

pub struct ResolverRegistry {
    resolvers: BTreeMap<String, Box<dyn ResolverDefinition>>,
}

impl Default for ResolverRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ResolverRegistry {
    pub fn new() -> Self {
        Self {
            resolvers: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, resolver: Box<dyn ResolverDefinition>) {
        self.resolvers.insert(resolver.name().to_string(), resolver);
    }

    pub fn resolver(&self, name: &str) -> Option<&dyn ResolverDefinition> {
        self.resolvers.get(name).map(|resolver| resolver.as_ref())
    }

    pub fn definitions(&self) -> impl Iterator<Item = &dyn ResolverDefinition> {
        self.resolvers.values().map(|resolver| resolver.as_ref())
    }

    pub fn is_output_type(&self, type_name: &str) -> bool {
        self.resolvers.values().any(|resolver| {
            resolver.output_type() == type_name
                || resolver.extra_output_types().contains(&type_name)
        })
    }

    pub fn evaluate_computed(
        &self,
        resolver: &str,
        method: &str,
        args: &[Value],
    ) -> std::result::Result<Value, Box<dyn std::error::Error>> {
        let resolver_impl = self
            .resolver(resolver)
            .ok_or_else(|| format!("Unknown resolver '{}'", resolver))?;

        let method_spec = resolver_impl
            .computed_methods()
            .iter()
            .find(|spec| spec.name == method)
            .ok_or_else(|| {
                format!(
                    "Resolver '{}' does not provide method '{}'",
                    resolver, method
                )
            })?;

        if method_spec.arg_count != args.len() {
            return Err(format!(
                "Resolver '{}' method '{}' expects {} args, got {}",
                resolver,
                method,
                method_spec.arg_count,
                args.len()
            )
            .into());
        }

        resolver_impl.evaluate_computed(method, args)
    }

    pub fn validate_computed_expr(
        &self,
        expr: &crate::ast::ComputedExpr,
        errors: &mut Vec<String>,
    ) {
        match expr {
            crate::ast::ComputedExpr::ResolverComputed {
                resolver,
                method,
                args,
            } => {
                let resolver_impl = self.resolver(resolver);
                if resolver_impl.is_none() {
                    errors.push(format!("Unknown resolver '{}'", resolver));
                } else if let Some(resolver_impl) = resolver_impl {
                    let method_spec = resolver_impl
                        .computed_methods()
                        .iter()
                        .find(|spec| spec.name == method);
                    if let Some(method_spec) = method_spec {
                        if method_spec.arg_count != args.len() {
                            errors.push(format!(
                                "Resolver '{}' method '{}' expects {} args, got {}",
                                resolver,
                                method,
                                method_spec.arg_count,
                                args.len()
                            ));
                        }
                    } else {
                        errors.push(format!(
                            "Resolver '{}' does not provide method '{}'",
                            resolver, method
                        ));
                    }
                }

                for arg in args {
                    self.validate_computed_expr(arg, errors);
                }
            }
            crate::ast::ComputedExpr::FieldRef { .. }
            | crate::ast::ComputedExpr::Literal { .. }
            | crate::ast::ComputedExpr::None
            | crate::ast::ComputedExpr::Var { .. }
            | crate::ast::ComputedExpr::ByteArray { .. }
            | crate::ast::ComputedExpr::ContextSlot
            | crate::ast::ComputedExpr::ContextTimestamp => {}
            crate::ast::ComputedExpr::UnwrapOr { expr, .. }
            | crate::ast::ComputedExpr::Cast { expr, .. }
            | crate::ast::ComputedExpr::Paren { expr }
            | crate::ast::ComputedExpr::Some { value: expr }
            | crate::ast::ComputedExpr::Slice { expr, .. }
            | crate::ast::ComputedExpr::Index { expr, .. }
            | crate::ast::ComputedExpr::U64FromLeBytes { bytes: expr }
            | crate::ast::ComputedExpr::U64FromBeBytes { bytes: expr }
            | crate::ast::ComputedExpr::JsonToBytes { expr }
            | crate::ast::ComputedExpr::Keccak256 { expr }
            | crate::ast::ComputedExpr::Unary { expr, .. } => {
                self.validate_computed_expr(expr, errors);
            }
            crate::ast::ComputedExpr::Binary { left, right, .. } => {
                self.validate_computed_expr(left, errors);
                self.validate_computed_expr(right, errors);
            }
            crate::ast::ComputedExpr::MethodCall { expr, args, .. } => {
                self.validate_computed_expr(expr, errors);
                for arg in args {
                    self.validate_computed_expr(arg, errors);
                }
            }
            crate::ast::ComputedExpr::Let { value, body, .. } => {
                self.validate_computed_expr(value, errors);
                self.validate_computed_expr(body, errors);
            }
            crate::ast::ComputedExpr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.validate_computed_expr(condition, errors);
                self.validate_computed_expr(then_branch, errors);
                self.validate_computed_expr(else_branch, errors);
            }
            crate::ast::ComputedExpr::Closure { body, .. } => {
                self.validate_computed_expr(body, errors);
            }
        }
    }
}

static BUILTIN_RESOLVER_REGISTRY: OnceLock<ResolverRegistry> = OnceLock::new();

pub fn register_builtin_resolvers(registry: &mut ResolverRegistry) {
    registry.register(Box::new(SlotHashResolver));
    registry.register(Box::new(TokenMetadataResolver));
}

pub fn builtin_resolver_registry() -> &'static ResolverRegistry {
    BUILTIN_RESOLVER_REGISTRY.get_or_init(|| {
        let mut registry = ResolverRegistry::new();
        register_builtin_resolvers(&mut registry);
        registry
    })
}

pub fn evaluate_resolver_computed(
    resolver: &str,
    method: &str,
    args: &[Value],
) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    builtin_resolver_registry().evaluate_computed(resolver, method, args)
}

pub fn validate_resolver_computed_specs(
    specs: &[crate::ast::ComputedFieldSpec],
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let registry = builtin_resolver_registry();
    let mut errors = Vec::new();

    for spec in specs {
        registry.validate_computed_expr(&spec.expression, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n").into())
    }
}

pub fn is_resolver_output_type(type_name: &str) -> bool {
    builtin_resolver_registry().is_output_type(type_name)
}

const DEFAULT_DAS_BATCH_SIZE: usize = 100;
const DEFAULT_DAS_TIMEOUT_SECS: u64 = 10;
const DAS_API_ENDPOINT_ENV: &str = "DAS_API_ENDPOINT";
const DAS_API_BATCH_ENV: &str = "DAS_API_BATCH_SIZE";

pub struct TokenMetadataResolverClient {
    endpoint: String,
    client: reqwest::Client,
    batch_size: usize,
}

impl TokenMetadataResolverClient {
    pub fn new(
        endpoint: String,
        batch_size: usize,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(DEFAULT_DAS_TIMEOUT_SECS))
            .build()?;

        Ok(Self {
            endpoint,
            client,
            batch_size: batch_size.max(1),
        })
    }

    pub fn from_env() -> Result<Option<Self>, Box<dyn std::error::Error + Send + Sync>> {
        let Some(endpoint) = std::env::var(DAS_API_ENDPOINT_ENV).ok() else {
            return Ok(None);
        };

        let batch_size = std::env::var(DAS_API_BATCH_ENV)
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(DEFAULT_DAS_BATCH_SIZE);

        Ok(Some(Self::new(endpoint, batch_size)?))
    }

    pub async fn resolve_token_metadata(
        &self,
        mints: &[String],
    ) -> Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
        let mut unique = HashSet::new();
        let mut deduped = Vec::new();

        for mint in mints {
            if mint.is_empty() {
                continue;
            }
            if unique.insert(mint.clone()) {
                deduped.push(mint.clone());
            }
        }

        let mut results = HashMap::new();
        if deduped.is_empty() {
            return Ok(results);
        }

        for chunk in deduped.chunks(self.batch_size) {
            let assets = self.fetch_assets(chunk).await?;
            for asset in assets {
                if let Some((mint, value)) = Self::build_token_metadata(&asset) {
                    results.insert(mint, value);
                }
            }
        }

        Ok(results)
    }

    async fn fetch_assets(
        &self,
        ids: &[String],
    ) -> Result<Vec<Value>, Box<dyn std::error::Error + Send + Sync>> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "getAssetBatch",
            "params": {
                "ids": ids,
                "options": {
                    "showFungible": true,
                },
            },
        });

        let response = self
            .client
            .post(&self.endpoint)
            .json(&payload)
            .send()
            .await?;
        let response = response.error_for_status()?;
        let value = response.json::<Value>().await?;

        if let Some(error) = value.get("error") {
            return Err(format!("Resolver response error: {}", error).into());
        }

        let assets = value
            .get("result")
            .and_then(|result| match result {
                Value::Array(items) => Some(items.clone()),
                Value::Object(obj) => obj.get("items").and_then(|items| items.as_array()).cloned(),
                _ => None,
            })
            .ok_or_else(|| "Resolver response missing result".to_string())?;

        let assets = assets.into_iter().filter(|a| !a.is_null()).collect();
        Ok(assets)
    }

    fn build_token_metadata(asset: &Value) -> Option<(String, Value)> {
        let mint = asset
            .get("id")
            .and_then(|value| value.as_str())?
            .to_string();

        let name = asset
            .pointer("/content/metadata/name")
            .and_then(|value| value.as_str());

        let symbol = asset
            .pointer("/content/metadata/symbol")
            .and_then(|value| value.as_str());

        let token_info = asset
            .get("token_info")
            .or_else(|| asset.pointer("/content/token_info"));

        let decimals = token_info
            .and_then(|info| info.get("decimals"))
            .and_then(|value| value.as_u64());

        let logo_uri = asset
            .pointer("/content/links/image")
            .and_then(|value| value.as_str())
            .or_else(|| {
                asset
                    .pointer("/content/links/image_uri")
                    .and_then(|value| value.as_str())
            });

        let mut obj = serde_json::Map::new();
        obj.insert("mint".to_string(), serde_json::json!(mint));
        obj.insert(
            "name".to_string(),
            name.map(|value| serde_json::json!(value))
                .unwrap_or(Value::Null),
        );
        obj.insert(
            "symbol".to_string(),
            symbol
                .map(|value| serde_json::json!(value))
                .unwrap_or(Value::Null),
        );
        obj.insert(
            "decimals".to_string(),
            decimals
                .map(|value| serde_json::json!(value))
                .unwrap_or(Value::Null),
        );
        obj.insert(
            "logo_uri".to_string(),
            logo_uri
                .map(|value| serde_json::json!(value))
                .unwrap_or(Value::Null),
        );

        Some((mint, Value::Object(obj)))
    }
}

// ============================================================================
// URL Resolver Client - Fetch and parse data from external URLs
// ============================================================================

const DEFAULT_URL_TIMEOUT_SECS: u64 = 30;
const DEFAULT_URL_CONNECT_TIMEOUT_SECS: u64 = 10;
const DEFAULT_URL_MAX_BODY_BYTES: usize = 1024 * 1024;
const DEFAULT_URL_MAX_REDIRECTS: usize = 5;
const DEFAULT_URL_MAX_CONCURRENT: usize = 32;
const DEFAULT_URL_MAX_CONCURRENT_PER_HOST: usize = 4;
const URL_MAX_BODY_BYTES_ENV: &str = "ARETE_URL_RESOLVER_MAX_BODY_BYTES";
const URL_ALLOW_PRIVATE_NETWORKS_ENV: &str = "ARETE_URL_RESOLVER_ALLOW_PRIVATE_NETWORKS";

/// Limits applied to URL resolver requests.
#[derive(Debug, Clone)]
pub struct UrlResolverOptions {
    /// Total time for a request, from connecting until the body is read.
    pub timeout: std::time::Duration,
    pub connect_timeout: std::time::Duration,
    /// Responses with a larger body are aborted.
    pub max_body_bytes: usize,
    pub max_redirects: usize,
    pub max_concurrent_requests: usize,
    pub max_concurrent_per_host: usize,
    /// Allow requests to loopback, private, link-local and other non-public
    /// addresses. Off by default; meant for local development only.
    pub allow_private_networks: bool,
}

impl Default for UrlResolverOptions {
    fn default() -> Self {
        Self {
            timeout: std::time::Duration::from_secs(DEFAULT_URL_TIMEOUT_SECS),
            connect_timeout: std::time::Duration::from_secs(DEFAULT_URL_CONNECT_TIMEOUT_SECS),
            max_body_bytes: DEFAULT_URL_MAX_BODY_BYTES,
            max_redirects: DEFAULT_URL_MAX_REDIRECTS,
            max_concurrent_requests: DEFAULT_URL_MAX_CONCURRENT,
            max_concurrent_per_host: DEFAULT_URL_MAX_CONCURRENT_PER_HOST,
            allow_private_networks: false,
        }
    }
}

impl UrlResolverOptions {
    /// Defaults, with `ARETE_URL_RESOLVER_MAX_BODY_BYTES` and
    /// `ARETE_URL_RESOLVER_ALLOW_PRIVATE_NETWORKS` applied when set.
    pub fn from_env() -> Self {
        let mut options = Self::default();
        if let Some(max_body_bytes) = std::env::var(URL_MAX_BODY_BYTES_ENV)
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
        {
            options.max_body_bytes = max_body_bytes;
        }
        options.allow_private_networks = std::env::var(URL_ALLOW_PRIVATE_NETWORKS_ENV)
            .map(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        if options.allow_private_networks {
            tracing::warn!(
                "{URL_ALLOW_PRIVATE_NETWORKS_ENV} is set; URL resolvers may reach private and \
                 loopback addresses"
            );
        }
        options
    }
}

/// Fetches JSON for URL resolvers.
///
/// URLs come from stack definitions and on-chain data, so requests are kept
/// to public `http`/`https` destinations: every resolved address and every
/// redirect hop is checked, the connection uses the checked addresses, bodies
/// are size-capped and concurrency is limited per host.
pub struct UrlResolverClient {
    client: reqwest::Client,
    policy: std::sync::Arc<crate::url_guard::AddressPolicy>,
    limiter: crate::url_guard::HostLimiter,
    max_body_bytes: usize,
}

impl Default for UrlResolverClient {
    fn default() -> Self {
        Self::new()
    }
}

impl UrlResolverClient {
    pub fn new() -> Self {
        Self::with_options(UrlResolverOptions::from_env())
    }

    pub fn with_timeout(timeout_secs: u64) -> Self {
        Self::with_options(UrlResolverOptions {
            timeout: std::time::Duration::from_secs(timeout_secs),
            ..UrlResolverOptions::from_env()
        })
    }

    pub fn with_options(options: UrlResolverOptions) -> Self {
        Self::build(
            options,
            crate::url_guard::AddressPolicy::default(),
            crate::url_guard::system_lookup(),
        )
    }

    fn build(
        options: UrlResolverOptions,
        mut policy: crate::url_guard::AddressPolicy,
        lookup: crate::url_guard::LookupFn,
    ) -> Self {
        policy.allow_private_networks = options.allow_private_networks;
        let policy = std::sync::Arc::new(policy);
        let client = reqwest::Client::builder()
            .timeout(options.timeout)
            .connect_timeout(options.connect_timeout)
            // A proxy would resolve the target itself, bypassing the checks.
            .no_proxy()
            .dns_resolver(std::sync::Arc::new(crate::url_guard::GuardedResolver::new(
                lookup,
                policy.clone(),
            )))
            .redirect(crate::url_guard::redirect_policy(
                policy.clone(),
                options.max_redirects,
            ))
            .build()
            .expect("Failed to create HTTP client for URL resolver");

        Self {
            client,
            policy,
            limiter: crate::url_guard::HostLimiter::new(
                options.max_concurrent_requests,
                options.max_concurrent_per_host,
            ),
            max_body_bytes: options.max_body_bytes,
        }
    }

    /// Resolve a URL and return the parsed JSON response
    pub async fn resolve(
        &self,
        url: &str,
        method: &crate::ast::HttpMethod,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        if url.is_empty() {
            return Err("URL is empty".into());
        }

        let url = reqwest::Url::parse(url)?;
        self.policy.check_url(&url)?;
        let host = url.host_str().unwrap_or_default().to_string();
        let _permits = self.limiter.acquire(&host).await;

        let request = match method {
            crate::ast::HttpMethod::Get => self.client.get(url),
            crate::ast::HttpMethod::Post => self.client.post(url),
        };
        let response = request.send().await?.error_for_status()?;
        if response.status().is_redirection() {
            // A redirect the client could not follow, e.g. to an unsupported scheme.
            return Err(
                format!("URL resolver redirect not followed ({})", response.status()).into(),
            );
        }
        let body = Self::read_body_capped(response, self.max_body_bytes).await?;

        Ok(serde_json::from_slice(&body)?)
    }

    async fn read_body_capped(
        mut response: reqwest::Response,
        max_bytes: usize,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let too_large = || format!("URL resolver response exceeds {max_bytes} bytes");
        if response
            .content_length()
            .is_some_and(|len| len > max_bytes as u64)
        {
            return Err(too_large().into());
        }

        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > max_bytes {
                return Err(too_large().into());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    /// Resolve a URL and extract a specific JSON path from the response
    pub async fn resolve_with_extract(
        &self,
        url: &str,
        method: &crate::ast::HttpMethod,
        extract_path: Option<&str>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let response = self.resolve(url, method).await?;

        if let Some(path) = extract_path {
            Self::extract_json_path(&response, path)
        } else {
            Ok(response)
        }
    }

    /// Extract a value from a JSON object using dot-notation path
    /// e.g., "data.image" extracts response["data"]["image"]
    pub fn extract_json_path(
        value: &Value,
        path: &str,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        if path.is_empty() {
            return Ok(value.clone());
        }

        let mut current = value;
        for segment in path.split('.') {
            // Try as object key first
            if let Some(next) = current.get(segment) {
                current = next;
            } else if let Ok(index) = segment.parse::<usize>() {
                // Try as array index
                if let Some(next) = current.get(index) {
                    current = next;
                } else {
                    return Err(
                        format!("Index '{}' out of bounds in path '{}'", index, path).into(),
                    );
                }
            } else {
                return Err(format!("Key '{}' not found in path '{}'", segment, path).into());
            }
        }

        Ok(current.clone())
    }

    /// Batch resolve multiple URLs in parallel with deduplication.
    /// Returns raw JSON keyed by method+URL. Identical requests are only fetched once.
    pub async fn resolve_batch(
        &self,
        urls: &[(String, crate::ast::HttpMethod)],
    ) -> HashMap<(String, crate::ast::HttpMethod), Value> {
        let mut unique: HashMap<(String, crate::ast::HttpMethod), ()> = HashMap::new();
        for (url, method) in urls {
            if !url.is_empty() {
                unique.entry((url.clone(), method.clone())).or_insert(());
            }
        }

        let futures = unique.into_keys().map(|(url, method)| async move {
            let result = self.resolve(&url, &method).await;
            ((url, method), result)
        });

        join_all(futures)
            .await
            .into_iter()
            .filter_map(|((url, method), result)| match result {
                Ok(value) => Some(((url, method), value)),
                Err(e) => {
                    tracing::warn!(url = %url, error = %e, "Failed to resolve URL");
                    None
                }
            })
            .collect()
    }
}

/// Resolver for looking up slot hashes by slot number
/// Uses the global slot hash cache populated from gRPC stream
struct SlotHashResolver;

const SLOT_HASH_METHODS: &[ResolverComputedMethod] = &[
    ResolverComputedMethod {
        name: "slot_hash",
        arg_count: 1,
    },
    ResolverComputedMethod {
        name: "keccak_rng",
        arg_count: 3,
    },
];

impl SlotHashResolver {
    fn optional_u64(value: &Value) -> Option<u64> {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
    }

    /// Compute keccak256(slot_hash || seed || samples_le_bytes) and XOR-fold into a u64.
    /// args[0] = slot_hash bytes (JSON array of 32 bytes)
    /// args[1] = seed bytes (JSON array of 32 bytes)
    /// args[2] = samples (u64 number)
    fn evaluate_keccak_rng(args: &[Value]) -> Result<Value, Box<dyn std::error::Error>> {
        if args.len() != 3 {
            return Ok(Value::Null);
        }

        // slot_hash() returns { bytes: [...] }, so extract the bytes array
        let slot_hash_bytes = match &args[0] {
            Value::Object(obj) => obj.get("bytes").cloned().unwrap_or(Value::Null),
            _ => args[0].clone(),
        };
        let slot_hash = Self::json_array_to_bytes(&slot_hash_bytes, 32);
        let seed = Self::json_array_to_bytes(&args[1], 32);
        let samples = Self::optional_u64(&args[2]);

        let (slot_hash, seed, samples) = match (slot_hash, seed, samples) {
            (Some(s), Some(sd), Some(sm)) => (s, sd, sm),
            _ => return Ok(Value::Null),
        };

        // Build input: slot_hash[32] || seed[32] || samples_le_bytes[8]
        let mut input = Vec::with_capacity(72);
        input.extend_from_slice(&slot_hash);
        input.extend_from_slice(&seed);
        input.extend_from_slice(&samples.to_le_bytes());

        // keccak256
        use sha3::{Digest, Keccak256};
        let hash = Keccak256::digest(&input);

        // XOR-fold four u64 chunks
        let r1 = u64::from_le_bytes(hash[0..8].try_into()?);
        let r2 = u64::from_le_bytes(hash[8..16].try_into()?);
        let r3 = u64::from_le_bytes(hash[16..24].try_into()?);
        let r4 = u64::from_le_bytes(hash[24..32].try_into()?);
        let rng = r1 ^ r2 ^ r3 ^ r4;

        Ok(Value::Number(serde_json::Number::from(rng)))
    }

    /// Extract a byte array of expected length from a JSON array value.
    fn json_array_to_bytes(value: &Value, expected_len: usize) -> Option<Vec<u8>> {
        let arr = value.as_array()?;
        let bytes: Vec<u8> = arr
            .iter()
            .filter_map(|v| v.as_u64().and_then(|n| u8::try_from(n).ok()))
            .collect();
        if bytes.len() == expected_len {
            Some(bytes)
        } else {
            tracing::debug!(
                got = bytes.len(),
                expected = expected_len,
                "json_array_to_bytes: length mismatch or out-of-range element"
            );
            None
        }
    }

    fn evaluate_slot_hash(args: &[Value]) -> Result<Value, Box<dyn std::error::Error>> {
        if args.len() != 1 {
            return Ok(Value::Null);
        }

        let slot = Self::optional_u64(&args[0]).unwrap_or(0);

        if slot == 0 {
            return Ok(Value::Null);
        }

        // Try to get the slot hash from the global cache
        let slot_hash = crate::slot_hash_cache::get_slot_hash(slot);

        match slot_hash {
            Some(hash) => {
                // Convert the base58 encoded slot hash to bytes
                // The slot hash is a 32-byte value base58 encoded
                match bs58::decode(&hash).into_vec() {
                    Ok(bytes) if bytes.len() == 32 => {
                        // Return as { bytes: [...] } to match the SlotHashBytes TypeScript interface
                        let json_bytes: Vec<Value> =
                            bytes.into_iter().map(|b| Value::Number(b.into())).collect();
                        let mut obj = serde_json::Map::new();
                        obj.insert("bytes".to_string(), Value::Array(json_bytes));
                        Ok(Value::Object(obj))
                    }
                    _ => {
                        tracing::warn!(slot = slot, hash = hash, "Failed to decode slot hash");
                        Ok(Value::Null)
                    }
                }
            }
            None => {
                tracing::debug!(slot = slot, "Slot hash not found in cache");
                Ok(Value::Null)
            }
        }
    }
}

impl ResolverDefinition for SlotHashResolver {
    fn name(&self) -> &'static str {
        "SlotHash"
    }

    fn output_type(&self) -> &'static str {
        "SlotHash"
    }

    fn computed_methods(&self) -> &'static [ResolverComputedMethod] {
        SLOT_HASH_METHODS
    }

    fn evaluate_computed(
        &self,
        method: &str,
        args: &[Value],
    ) -> std::result::Result<Value, Box<dyn std::error::Error>> {
        match method {
            "slot_hash" => Self::evaluate_slot_hash(args),
            "keccak_rng" => Self::evaluate_keccak_rng(args),
            _ => Err(format!("Unknown SlotHash method '{}'", method).into()),
        }
    }

    fn typescript_interface(&self) -> Option<&'static str> {
        Some(
            r#"export interface SlotHashBytes {
  /** 32-byte slot hash as array of numbers (0-255) */
  bytes: number[];
}

export type KeccakRngValue = string;"#,
        )
    }

    fn extra_output_types(&self) -> &'static [&'static str] {
        &["SlotHashBytes", "KeccakRngValue"]
    }

    fn typescript_schema(&self) -> Option<ResolverTypeScriptSchema> {
        Some(ResolverTypeScriptSchema {
            name: "SlotHashTypes",
            definition: r#"export const SlotHashBytesSchema = z.object({
  bytes: z.array(z.number().int().min(0).max(255)).length(32),
});

export const KeccakRngValueSchema = z.string();"#,
        })
    }
}

struct TokenMetadataResolver;

const TOKEN_METADATA_METHODS: &[ResolverComputedMethod] = &[
    ResolverComputedMethod {
        name: "ui_amount",
        arg_count: 2,
    },
    ResolverComputedMethod {
        name: "raw_amount",
        arg_count: 2,
    },
];

impl TokenMetadataResolver {
    fn optional_f64(value: &Value) -> Option<f64> {
        if value.is_null() {
            return None;
        }
        match value {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.parse::<f64>().ok(),
            _ => None,
        }
    }

    fn optional_u8(value: &Value) -> Option<u8> {
        if value.is_null() {
            return None;
        }
        match value {
            Value::Number(number) => number
                .as_u64()
                .or_else(|| {
                    number
                        .as_i64()
                        .and_then(|v| if v >= 0 { Some(v as u64) } else { None })
                })
                .and_then(|v| u8::try_from(v).ok()),
            Value::String(text) => text.parse::<u8>().ok(),
            _ => None,
        }
    }

    fn evaluate_ui_amount(
        args: &[Value],
    ) -> std::result::Result<Value, Box<dyn std::error::Error>> {
        let raw_value = Self::optional_f64(&args[0]);
        let decimals = Self::optional_u8(&args[1]);

        match (raw_value, decimals) {
            (Some(value), Some(decimals)) => {
                let factor = 10_f64.powi(decimals as i32);
                let result = value / factor;
                if result.is_finite() {
                    serde_json::Number::from_f64(result)
                        .map(Value::Number)
                        .ok_or_else(|| "Failed to serialize ui_amount".into())
                } else {
                    Err("ui_amount result is not finite".into())
                }
            }
            _ => Ok(Value::Null),
        }
    }

    fn evaluate_raw_amount(
        args: &[Value],
    ) -> std::result::Result<Value, Box<dyn std::error::Error>> {
        let ui_value = Self::optional_f64(&args[0]);
        let decimals = Self::optional_u8(&args[1]);

        match (ui_value, decimals) {
            (Some(value), Some(decimals)) => {
                let factor = 10_f64.powi(decimals as i32);
                let result = value * factor;
                if !result.is_finite() || result < 0.0 {
                    return Err("raw_amount result is not finite".into());
                }
                let rounded = result.round();
                if rounded > u64::MAX as f64 {
                    return Err("raw_amount result exceeds u64".into());
                }
                Ok(Value::Number(serde_json::Number::from(rounded as u64)))
            }
            _ => Ok(Value::Null),
        }
    }
}

impl ResolverDefinition for TokenMetadataResolver {
    fn name(&self) -> &'static str {
        "TokenMetadata"
    }

    fn output_type(&self) -> &'static str {
        "TokenMetadata"
    }

    fn computed_methods(&self) -> &'static [ResolverComputedMethod] {
        TOKEN_METADATA_METHODS
    }

    fn evaluate_computed(
        &self,
        method: &str,
        args: &[Value],
    ) -> std::result::Result<Value, Box<dyn std::error::Error>> {
        match method {
            "ui_amount" => Self::evaluate_ui_amount(args),
            "raw_amount" => Self::evaluate_raw_amount(args),
            _ => Err(format!("Unknown TokenMetadata method '{}'", method).into()),
        }
    }

    fn typescript_interface(&self) -> Option<&'static str> {
        Some(
            r#"export interface TokenMetadata {
  mint: string;
  name?: string | null;
  symbol?: string | null;
  decimals?: number | null;
  logoUri?: string | null;
}"#,
        )
    }

    fn typescript_schema(&self) -> Option<ResolverTypeScriptSchema> {
        Some(ResolverTypeScriptSchema {
            name: "TokenMetadataSchema",
            definition: r#"export const TokenMetadataSchema = z.object({
  mint: z.string(),
  name: z.string().nullable().optional(),
  symbol: z.string().nullable().optional(),
  decimals: z.number().nullable().optional(),
  logo_uri: z.string().nullable().optional(),
}).transform((value) => ({
  mint: value.mint,
  ...(value.name !== undefined ? { name: value.name } : {}),
  ...(value.symbol !== undefined ? { symbol: value.symbol } : {}),
  ...(value.decimals !== undefined ? { decimals: value.decimals } : {}),
  ...(value.logo_uri !== undefined ? { logoUri: value.logo_uri } : {}),
}));

export const TokenMetadataPatchSchema = z.object({
  mint: z.string().optional(),
  name: z.string().nullable().optional(),
  symbol: z.string().nullable().optional(),
  decimals: z.number().nullable().optional(),
  logo_uri: z.string().nullable().optional(),
}).transform((value) => ({
  ...(value.mint !== undefined ? { mint: value.mint } : {}),
  ...(value.name !== undefined ? { name: value.name } : {}),
  ...(value.symbol !== undefined ? { symbol: value.symbol } : {}),
  ...(value.decimals !== undefined ? { decimals: value.decimals } : {}),
  ...(value.logo_uri !== undefined ? { logoUri: value.logo_uri } : {}),
}));"#,
        })
    }
}

impl<'a> InstructionContext<'a> {
    /// Transaction observations from the originating instruction, if retained.
    pub fn solana_transaction(
        &self,
    ) -> serde_json::Result<Option<crate::SolanaTransactionMetadata>> {
        match &self.update_context {
            Some(context) => context.solana_transaction(),
            None => Ok(None),
        }
    }

    pub fn new(
        accounts: HashMap<String, String>,
        state_id: u32,
        reverse_lookup_tx: &'a mut dyn ReverseLookupUpdater,
    ) -> Self {
        Self {
            accounts,
            state_id,
            reverse_lookup_tx: reverse_lookup_tx as *mut (dyn ReverseLookupUpdater + 'a),
            pending_updates: Vec::new(),
            registers: None,
            state_reg: None,
            compiled_paths: None,
            instruction_data: None,
            slot: None,
            signature: None,
            timestamp: None,
            update_context: None,
            dirty_tracker: crate::vm::DirtyTracker::new(),
            _borrow: PhantomData,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_metrics(
        accounts: HashMap<String, String>,
        state_id: u32,
        vm: &'a mut crate::vm::VmContext,
        state_reg: crate::vm::Register,
        instruction_data: &'a serde_json::Value,
        slot: Option<u64>,
        signature: Option<String>,
        timestamp: i64,
    ) -> Self {
        let update_context = vm.current_context().cloned();
        // Store raw pointers so the hook context can access VM internals without
        // holding overlapping Rust references to `vm` and `vm.registers`.
        let reverse_lookup_tx =
            vm as *mut crate::vm::VmContext as *mut (dyn ReverseLookupUpdater + 'a);
        let registers = vm.registers_mut() as *mut Vec<crate::vm::RegisterValue>;
        let compiled_paths =
            vm.path_cache() as *const HashMap<String, crate::metrics_context::CompiledPath>;

        Self {
            accounts,
            state_id,
            reverse_lookup_tx,
            pending_updates: Vec::new(),
            registers: Some(registers),
            state_reg: Some(state_reg),
            compiled_paths: Some(compiled_paths),
            instruction_data: Some(instruction_data),
            slot,
            signature,
            timestamp: Some(timestamp),
            update_context,
            dirty_tracker: crate::vm::DirtyTracker::new(),
            _borrow: PhantomData,
        }
    }

    fn registers(&self) -> Option<&Vec<crate::vm::RegisterValue>> {
        self.registers.map(|registers| unsafe { &*registers })
    }

    fn registers_mut(&mut self) -> Option<&mut Vec<crate::vm::RegisterValue>> {
        self.registers.map(|registers| unsafe { &mut *registers })
    }

    /// Get an account address by its name from the instruction
    pub fn account(&self, name: &str) -> Option<String> {
        self.accounts.get(name).cloned()
    }

    /// Register a reverse lookup: PDA address -> seed value
    /// This also flushes any pending account updates for this PDA
    ///
    /// The pending account updates are accumulated internally and can be retrieved
    /// via `take_pending_updates()` after all hooks have executed.
    pub fn register_pda_reverse_lookup(&mut self, pda_address: &str, seed_value: &str) {
        let pending = unsafe {
            (&mut *self.reverse_lookup_tx).update(pda_address.to_string(), seed_value.to_string())
        };
        self.pending_updates.extend(pending);
    }

    /// Take all accumulated pending updates
    ///
    /// This should be called after all instruction hooks have executed to retrieve
    /// any pending account updates that need to be reprocessed.
    pub fn take_pending_updates(&mut self) -> Vec<crate::vm::PendingAccountUpdate> {
        std::mem::take(&mut self.pending_updates)
    }

    pub fn dirty_tracker(&self) -> &crate::vm::DirtyTracker {
        &self.dirty_tracker
    }

    pub fn dirty_tracker_mut(&mut self) -> &mut crate::vm::DirtyTracker {
        &mut self.dirty_tracker
    }

    /// Get the current state register value (for generating mutations)
    pub fn state_value(&self) -> Option<&serde_json::Value> {
        let state_reg = self.state_reg?;
        let registers = self.registers()?;
        Some(&registers[state_reg])
    }

    /// Get a field value from the entity state
    /// This allows reading aggregated values or other entity fields
    pub fn get<T: serde::de::DeserializeOwned>(&self, field_path: &str) -> Option<T> {
        let state_reg = self.state_reg?;
        let registers = self.registers()?;
        let state = &registers[state_reg];
        self.get_nested_value(state, field_path)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    pub fn set<T: serde::Serialize>(&mut self, field_path: &str, value: T) {
        let Some(state_reg) = self.state_reg else {
            println!("      ⚠️  Cannot set field '{}': metrics not configured (registers={}, state_reg={:?})", 
                field_path, self.registers.is_some(), self.state_reg);
            return;
        };

        if let Some(registers) = self.registers_mut() {
            let serialized = serde_json::to_value(value).ok();
            if let Some(val) = serialized {
                Self::set_nested_value_static(&mut registers[state_reg], field_path, val);
                self.dirty_tracker.mark_replaced(field_path);
                println!("      ✓ Set field '{}' and marked as dirty", field_path);
            }
        } else {
            println!("      ⚠️  Cannot set field '{}': metrics not configured (registers={}, state_reg={:?})", 
                field_path, self.registers.is_some(), self.state_reg);
        }
    }

    pub fn increment(&mut self, field_path: &str, amount: i64) {
        let current = self.get::<i64>(field_path).unwrap_or(0);
        self.set(field_path, current + amount);
    }

    pub fn append<T: serde::Serialize>(&mut self, field_path: &str, value: T) {
        let Some(state_reg) = self.state_reg else {
            println!(
                "      ⚠️  Cannot append to '{}': metrics not configured",
                field_path
            );
            return;
        };

        if let Some(registers) = self.registers_mut() {
            let serialized = serde_json::to_value(&value).ok();
            if let Some(val) = serialized {
                Self::append_to_array_static(&mut registers[state_reg], field_path, val.clone());
                self.dirty_tracker.mark_appended(field_path, val);
                println!(
                    "      ✓ Appended to '{}' and marked as appended",
                    field_path
                );
            }
        } else {
            println!(
                "      ⚠️  Cannot append to '{}': metrics not configured",
                field_path
            );
        }
    }

    fn append_to_array_static(
        value: &mut serde_json::Value,
        path: &str,
        new_value: serde_json::Value,
    ) {
        let segments: Vec<&str> = path.split('.').collect();
        if segments.is_empty() {
            return;
        }

        let mut current = value;
        for segment in &segments[..segments.len() - 1] {
            if !current.is_object() {
                *current = serde_json::json!({});
            }
            let obj = current.as_object_mut().unwrap();
            current = obj
                .entry(segment.to_string())
                .or_insert(serde_json::json!({}));
        }

        let last_segment = segments[segments.len() - 1];
        if !current.is_object() {
            *current = serde_json::json!({});
        }
        let obj = current.as_object_mut().unwrap();
        let arr = obj
            .entry(last_segment.to_string())
            .or_insert_with(|| serde_json::json!([]));
        if let Some(arr) = arr.as_array_mut() {
            arr.push(new_value);
        }
    }

    fn get_nested_value<'b>(
        &self,
        value: &'b serde_json::Value,
        path: &str,
    ) -> Option<&'b serde_json::Value> {
        let mut current = value;
        for segment in path.split('.') {
            current = current.get(segment)?;
        }
        Some(current)
    }

    fn set_nested_value_static(
        value: &mut serde_json::Value,
        path: &str,
        new_value: serde_json::Value,
    ) {
        let segments: Vec<&str> = path.split('.').collect();
        if segments.is_empty() {
            return;
        }

        let mut current = value;
        for segment in &segments[..segments.len() - 1] {
            if !current.is_object() {
                *current = serde_json::json!({});
            }
            let obj = current.as_object_mut().unwrap();
            current = obj
                .entry(segment.to_string())
                .or_insert(serde_json::json!({}));
        }

        if !current.is_object() {
            *current = serde_json::json!({});
        }
        if let Some(obj) = current.as_object_mut() {
            obj.insert(segments[segments.len() - 1].to_string(), new_value);
        }
    }

    /// Access instruction data field
    pub fn data<T: serde::de::DeserializeOwned>(&self, field: &str) -> Option<T> {
        self.instruction_data
            .and_then(|data| data.get(field))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Get the current timestamp
    pub fn timestamp(&self) -> i64 {
        self.timestamp.unwrap_or(0)
    }

    /// Get the current slot
    pub fn slot(&self) -> Option<u64> {
        self.slot
    }

    /// Get the current signature
    pub fn signature(&self) -> Option<&str> {
        self.signature.as_deref()
    }

    /// Position of this occurrence within its transaction
    pub fn event_index(&self) -> Option<u64> {
        self.update_context
            .as_ref()
            .and_then(|context| context.event_index)
    }

    /// 0-based instruction path within the transaction (e.g. `"0.1"`)
    pub fn ix_path(&self) -> Option<&str> {
        self.update_context
            .as_ref()
            .and_then(|context| context.ix_path.as_deref())
    }
}

#[cfg(test)]
mod url_resolver_tests {
    use super::*;
    use crate::ast::HttpMethod;
    use crate::url_guard::{AddressPolicy, LookupFn};
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const LOCAL: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    fn json(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn redirect(location: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\
             Connection: close\r\n\r\n"
        )
    }

    /// Serves canned responses by path and closes each connection after one
    /// response, so every request opens a new connection.
    async fn serve(routes: impl Fn(u16) -> Vec<(&'static str, String)>) -> u16 {
        let listener = tokio::net::TcpListener::bind((LOCAL, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes = Arc::new(routes(port));
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let routes = routes.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => request.extend_from_slice(&buf[..n]),
                        }
                    }
                    let request = String::from_utf8_lossy(&request);
                    let path = request.split_whitespace().nth(1).unwrap_or("/");
                    let response = routes
                        .iter()
                        .find(|(route, _)| *route == path)
                        .map(|(_, response)| response.clone())
                        .unwrap_or_else(|| {
                            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\
                             Connection: close\r\n\r\n"
                                .to_string()
                        });
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        port
    }

    /// Resolves `*.test` names from a fixed table.
    fn lookup(table: &'static [(&'static str, &'static str)]) -> LookupFn {
        Arc::new(move |host: String| {
            Box::pin(async move {
                table
                    .iter()
                    .find(|(name, _)| *name == host)
                    .map(|(_, ip)| vec![ip.parse().unwrap()])
                    .ok_or_else(|| std::io::Error::other(format!("unknown host {host}")))
            })
        })
    }

    /// A client that may reach the local test server at 127.0.0.1 and no
    /// other non-public address.
    fn client(lookup: LookupFn, options: UrlResolverOptions) -> UrlResolverClient {
        UrlResolverClient::build(
            options,
            AddressPolicy {
                allow_private_networks: false,
                exempt: vec![LOCAL],
            },
            lookup,
        )
    }

    fn assert_blocked(result: Result<Value, Box<dyn std::error::Error + Send + Sync>>) {
        let err = result.expect_err("request should be blocked");
        let mut source: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
        while let Some(current) = source {
            if current.to_string().contains("URL resolver request blocked") {
                return;
            }
            source = current.source();
        }
        panic!("expected a blocked-request error, got: {err:?}");
    }

    const HOSTS: &[(&str, &str)] = &[
        ("origin.test", "127.0.0.1"),
        ("internal.test", "10.0.0.1"),
        ("metadata.test", "169.254.169.254"),
    ];

    #[tokio::test]
    async fn fetches_json_from_a_permitted_host() {
        let port = serve(|_| vec![("/data", json(r#"{"name":"ok"}"#))]).await;
        let client = client(lookup(HOSTS), UrlResolverOptions::default());
        let value = client
            .resolve(&format!("http://origin.test:{port}/data"), &HttpMethod::Get)
            .await
            .unwrap();
        assert_eq!(value, serde_json::json!({ "name": "ok" }));
    }

    #[tokio::test]
    async fn internal_destinations_are_refused() {
        let port = serve(|_| vec![("/data", json("{}"))]).await;

        // A live local server is refused by a client with the default policy,
        // whether named by hostname or IP literal.
        let default_client = UrlResolverClient::with_options(UrlResolverOptions::default());
        for url in [
            format!("http://localhost:{port}/data"),
            format!("http://127.0.0.1:{port}/data"),
            format!("http://[::ffff:127.0.0.1]:{port}/data"),
            format!("http://2130706433:{port}/data"),
        ] {
            assert_blocked(default_client.resolve(&url, &HttpMethod::Get).await);
        }

        let client = client(lookup(HOSTS), UrlResolverOptions::default());
        for url in [
            "http://169.254.169.254/latest/meta-data/".to_string(),
            "http://10.0.0.1/".to_string(),
            "http://192.168.0.1/".to_string(),
            "http://[fd00::1]/".to_string(),
            format!("http://internal.test:{port}/data"),
            format!("http://metadata.test:{port}/data"),
            "file:///etc/passwd".to_string(),
        ] {
            assert_blocked(client.resolve(&url, &HttpMethod::Get).await);
        }
    }

    #[tokio::test]
    async fn redirects_to_internal_destinations_are_refused() {
        let port = serve(|port| {
            vec![
                ("/data", json("{}")),
                (
                    "/to-metadata",
                    redirect("http://169.254.169.254/latest/meta-data/"),
                ),
                (
                    "/to-loopback",
                    redirect(&format!("http://127.0.0.2:{port}/data")),
                ),
                (
                    "/to-internal-name",
                    redirect(&format!("http://internal.test:{port}/data")),
                ),
                ("/to-file", redirect("file:///etc/passwd")),
                (
                    "/to-ok",
                    redirect(&format!("http://origin.test:{port}/data")),
                ),
            ]
        })
        .await;
        let client = client(lookup(HOSTS), UrlResolverOptions::default());
        let base = format!("http://origin.test:{port}");

        for path in ["/to-metadata", "/to-loopback", "/to-internal-name"] {
            assert_blocked(
                client
                    .resolve(&format!("{base}{path}"), &HttpMethod::Get)
                    .await,
            );
        }

        // A redirect to another scheme is never followed.
        let err = client
            .resolve(&format!("{base}/to-file"), &HttpMethod::Get)
            .await
            .expect_err("redirect to file:// should fail");
        assert!(err.to_string().contains("redirect not followed"), "{err}");

        // Redirects between permitted destinations are still followed.
        let value = client
            .resolve(&format!("{base}/to-ok"), &HttpMethod::Get)
            .await
            .unwrap();
        assert_eq!(value, serde_json::json!({}));
    }

    #[tokio::test]
    async fn redirect_chains_are_capped() {
        let port = serve(|port| {
            vec![(
                "/loop",
                redirect(&format!("http://origin.test:{port}/loop")),
            )]
        })
        .await;
        let options = UrlResolverOptions {
            max_redirects: 2,
            ..UrlResolverOptions::default()
        };
        let client = client(lookup(HOSTS), options);
        assert_blocked(
            client
                .resolve(&format!("http://origin.test:{port}/loop"), &HttpMethod::Get)
                .await,
        );
    }

    #[tokio::test]
    async fn every_connection_rechecks_dns_so_rebinding_is_refused() {
        let port = serve(|_| vec![("/data", json("{}"))]).await;

        // The name first resolves to the permitted server, then to an
        // internal address, as a rebinding DNS server would answer.
        let calls = Arc::new(AtomicUsize::new(0));
        let rebinding: LookupFn = {
            let calls = calls.clone();
            Arc::new(move |_host: String| {
                let answer = if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    LOCAL
                } else {
                    "10.0.0.1".parse().unwrap()
                };
                Box::pin(async move { Ok(vec![answer]) })
            })
        };
        let client = client(rebinding, UrlResolverOptions::default());
        let url = format!("http://rebind.test:{port}/data");

        client.resolve(&url, &HttpMethod::Get).await.unwrap();
        assert_blocked(client.resolve(&url, &HttpMethod::Get).await);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn bodies_over_the_cap_are_aborted() {
        let big = format!(r#"{{"data":"{}"}}"#, "x".repeat(4096));
        let port = serve(move |_| {
            vec![
                ("/small", json(r#"{"ok":true}"#)),
                ("/declared", json(&big)),
                // No Content-Length: the body runs until the connection closes.
                (
                    "/streamed",
                    format!("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{big}"),
                ),
            ]
        })
        .await;
        let options = UrlResolverOptions {
            max_body_bytes: 1024,
            ..UrlResolverOptions::default()
        };
        let client = client(lookup(HOSTS), options);
        let base = format!("http://origin.test:{port}");

        assert!(client
            .resolve(&format!("{base}/small"), &HttpMethod::Get)
            .await
            .is_ok());
        for path in ["/declared", "/streamed"] {
            let err = client
                .resolve(&format!("{base}{path}"), &HttpMethod::Get)
                .await
                .expect_err("oversized body should be rejected");
            assert!(
                err.to_string().contains("exceeds 1024 bytes"),
                "{path}: {err}"
            );
        }
    }
}
