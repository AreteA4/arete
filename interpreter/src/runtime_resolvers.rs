use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ast::{HttpMethod, ResolverType};
use crate::compiler::MultiEntityBytecode;
use crate::resolvers::{TokenMetadataResolverClient, UrlResolverClient};
use crate::vm::{
    CachedResolverValue, ResolverRequest, TokenResolverMissAction, UpdateContext, VmContext,
};
use crate::Mutation;

pub type ResolverBatchResult =
    Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>>;
pub type ResolverBatchFuture<'a> = Pin<Box<dyn Future<Output = ResolverBatchResult> + Send + 'a>>;
pub type ResolverApplyFuture<'a> = Pin<Box<dyn Future<Output = Vec<Mutation>> + Send + 'a>>;
pub type SharedRuntimeResolver = std::sync::Arc<dyn RuntimeResolver>;

#[derive(Clone, Copy)]
struct RuntimeResolverProfileConfig {
    enabled: bool,
    slow_ms: f64,
}

fn runtime_resolver_profile_config() -> RuntimeResolverProfileConfig {
    static CONFIG: OnceLock<RuntimeResolverProfileConfig> = OnceLock::new();
    *CONFIG.get_or_init(|| RuntimeResolverProfileConfig {
        enabled: std::env::var("ARETE_PROFILE_CODEPATHS")
            .ok()
            .or_else(|| std::env::var("ARETE_PROFILE_RUNTIME_RESOLVER").ok())
            .map(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false),
        slow_ms: std::env::var("ARETE_PROFILE_CODEPATHS_SLOW_MS")
            .ok()
            .or_else(|| std::env::var("ARETE_PROFILE_RUNTIME_RESOLVER_SLOW_MS").ok())
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(5.0),
    })
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn should_log_runtime_resolver_profile(total_ms: f64) -> bool {
    let config = runtime_resolver_profile_config();
    config.enabled && total_ms >= config.slow_ms
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeResolverRequest {
    TokenMetadata {
        key: String,
        mint: String,
    },
    UrlJson {
        key: String,
        url: String,
        method: HttpMethod,
    },
}

impl RuntimeResolverRequest {
    pub fn key(&self) -> &str {
        match self {
            Self::TokenMetadata { key, .. } | Self::UrlJson { key, .. } => key,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeResolverBatchRequest {
    pub requests: Vec<RuntimeResolverRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeResolverResponse {
    pub key: String,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeResolverBatchResponse {
    #[serde(default)]
    pub resolved: Vec<RuntimeResolverResponse>,
}

#[derive(Debug)]
struct PendingRuntimeResolverRequest {
    request: ResolverRequest,
    backend_request: RuntimeResolverRequest,
}

pub trait RuntimeResolver: Send + Sync {
    fn resolve_batch<'a>(
        &'a self,
        requests: &'a [RuntimeResolverRequest],
    ) -> ResolverBatchFuture<'a>;

    fn resolve_and_apply<'a>(
        &'a self,
        vm: &'a std::sync::Mutex<VmContext>,
        bytecode: &'a MultiEntityBytecode,
        requests: Vec<ResolverRequest>,
        apply_context: Option<UpdateContext>,
    ) -> ResolverApplyFuture<'a> {
        Box::pin(async move {
            let total_started_at = Instant::now();
            if requests.is_empty() {
                return Vec::new();
            }

            let request_count = requests.len();
            let mut cached = Vec::new();
            let mut pending = Vec::new();
            let mut invalid = Vec::new();
            let mut negative_cached = Vec::new();

            let classify_started_at = Instant::now();
            let (
                classify_ms,
                cached_count,
                pending_count,
                invalid_count,
                negative_cached_count,
                unique_request_count,
            ) = {
                let mut vm_guard = vm.lock().unwrap_or_else(|e| e.into_inner());

                for request in requests {
                    let canonical_key =
                        runtime_resolver_cache_key(&request.resolver, &request.input);

                    if let Some(cached_value) = vm_guard.get_cached_resolver_value(&canonical_key) {
                        match cached_value {
                            CachedResolverValue::Resolved(resolved_value) => {
                                cached.push((request, resolved_value));
                            }
                            CachedResolverValue::Negative => {
                                negative_cached.push(request);
                            }
                        }
                        continue;
                    }

                    match runtime_request_from_vm_request(&request) {
                        Some(backend_request) => pending.push(PendingRuntimeResolverRequest {
                            request,
                            backend_request,
                        }),
                        None => invalid.push(request),
                    }
                }

                let invalid_count = invalid.len();
                if !invalid.is_empty() {
                    vm_guard.restore_resolver_requests(invalid);
                }

                let classify_ms = elapsed_ms(classify_started_at);
                let cached_count = cached.len();
                let pending_count = pending.len();
                let negative_cached_count = negative_cached.len();
                let unique_request_count = if pending.is_empty() {
                    0
                } else {
                    let mut unique = HashMap::new();
                    for entry in &pending {
                        unique
                            .entry(entry.backend_request.key().to_string())
                            .or_insert_with(|| entry.backend_request.clone());
                    }
                    unique.len()
                };

                (
                    classify_ms,
                    cached_count,
                    pending_count,
                    invalid_count,
                    negative_cached_count,
                    unique_request_count,
                )
            };

            let resolve_batch_started_at = Instant::now();
            let resolved_map = if pending.is_empty() {
                Ok(HashMap::new())
            } else {
                let mut unique = HashMap::new();
                for entry in &pending {
                    unique
                        .entry(entry.backend_request.key().to_string())
                        .or_insert_with(|| entry.backend_request.clone());
                }

                let unique_requests: Vec<RuntimeResolverRequest> = unique.into_values().collect();
                self.resolve_batch(&unique_requests).await
            };
            let resolve_batch_ms = elapsed_ms(resolve_batch_started_at);

            let mut mutations = Vec::new();
            let mut failed = Vec::new();
            let mut retry_scheduled_count = 0usize;
            let mut negative_cached_miss_count = 0usize;

            let apply_cached_started_at = Instant::now();
            let mut vm_guard = vm.lock().unwrap_or_else(|e| e.into_inner());
            let previous_context = if apply_context.is_some() {
                let previous = vm_guard.current_context().cloned();
                vm_guard.set_current_context(apply_context.clone());
                previous
            } else {
                None
            };

            for (request, resolved_value) in cached {
                match vm_guard.apply_resolver_result(bytecode, &request.cache_key, resolved_value) {
                    Ok(mut new_mutations) => mutations.append(&mut new_mutations),
                    Err(err) => {
                        tracing::warn!(
                            cache_key = %request.cache_key,
                            error = %err,
                            "Failed to apply cached resolver result"
                        );
                        failed.push(request);
                    }
                }
            }
            let apply_cached_ms = elapsed_ms(apply_cached_started_at);

            let apply_pending_started_at = Instant::now();
            match resolved_map {
                Ok(resolved_map) => {
                    for entry in pending {
                        match resolved_map.get(entry.backend_request.key()) {
                            Some(resolved_value) => match vm_guard.apply_resolver_result(
                                bytecode,
                                &entry.request.cache_key,
                                resolved_value.clone(),
                            ) {
                                Ok(mut new_mutations) => mutations.append(&mut new_mutations),
                                Err(err) => {
                                    tracing::warn!(
                                        cache_key = %entry.request.cache_key,
                                        error = %err,
                                        "Failed to apply resolver result"
                                    );
                                    failed.push(entry.request);
                                }
                            },
                            None => match entry.backend_request {
                                RuntimeResolverRequest::TokenMetadata { .. } => {
                                    match vm_guard
                                        .handle_token_resolver_miss(&entry.request.cache_key)
                                    {
                                        TokenResolverMissAction::RetryScheduled => {
                                            retry_scheduled_count += 1;
                                        }
                                        TokenResolverMissAction::NegativeCached => {
                                            negative_cached_miss_count += 1;
                                        }
                                        TokenResolverMissAction::Dropped => {}
                                    }
                                }
                                RuntimeResolverRequest::UrlJson { .. } => {
                                    failed.push(entry.request)
                                }
                            },
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "Runtime resolver backend request failed");
                    for entry in pending {
                        match entry.backend_request {
                            RuntimeResolverRequest::TokenMetadata { .. } => {
                                match vm_guard.handle_token_resolver_miss(&entry.request.cache_key)
                                {
                                    TokenResolverMissAction::RetryScheduled => {
                                        retry_scheduled_count += 1;
                                    }
                                    TokenResolverMissAction::NegativeCached => {
                                        negative_cached_miss_count += 1;
                                    }
                                    TokenResolverMissAction::Dropped => {}
                                }
                            }
                            RuntimeResolverRequest::UrlJson { .. } => failed.push(entry.request),
                        }
                    }
                }
            }
            let apply_pending_ms = elapsed_ms(apply_pending_started_at);

            let failed_count = failed.len();
            let restore_failed_started_at = Instant::now();
            if !failed.is_empty() {
                vm_guard.restore_resolver_requests(failed);
            }
            let restore_failed_ms = elapsed_ms(restore_failed_started_at);
            if apply_context.is_some() {
                vm_guard.set_current_context(previous_context);
            }

            let total_ms = elapsed_ms(total_started_at);
            if should_log_runtime_resolver_profile(total_ms) {
                tracing::info!(
                    stage = "runtime.resolver_apply",
                    request_count,
                    cached_count,
                    pending_count,
                    invalid_count,
                    negative_cached_count,
                    unique_request_count,
                    failed_count,
                    retry_scheduled_count,
                    negative_cached_miss_count,
                    mutation_count = mutations.len(),
                    total_ms,
                    classify_ms,
                    resolve_batch_ms,
                    apply_cached_ms,
                    apply_pending_ms,
                    restore_failed_ms,
                    "Runtime resolver profile"
                );
            }

            mutations
        })
    }
}

pub struct InProcessResolver {
    token_client: Option<TokenMetadataResolverClient>,
    url_client: UrlResolverClient,
}

impl InProcessResolver {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self {
            token_client: TokenMetadataResolverClient::from_env()?,
            url_client: UrlResolverClient::new(),
        })
    }

    pub fn new(
        token_client: Option<TokenMetadataResolverClient>,
        url_client: UrlResolverClient,
    ) -> Self {
        Self {
            token_client,
            url_client,
        }
    }

    pub async fn resolve_batch_internal(
        &self,
        requests: &[RuntimeResolverRequest],
    ) -> ResolverBatchResult {
        let total_started_at = Instant::now();
        let mut results = HashMap::new();
        let mut token_requests = Vec::new();
        let mut url_requests = Vec::new();

        let classify_started_at = Instant::now();
        for request in requests {
            match request {
                RuntimeResolverRequest::TokenMetadata { key, mint } => {
                    token_requests.push((key.clone(), mint.clone()));
                }
                RuntimeResolverRequest::UrlJson { key, url, method } => {
                    url_requests.push((key.clone(), url.clone(), method.clone()));
                }
            }
        }
        let classify_ms = elapsed_ms(classify_started_at);

        let token_request_count = token_requests.len();
        let url_request_count = url_requests.len();
        let mut token_batch_ms = 0.0_f64;
        let mut url_dedupe_ms = 0.0_f64;
        let mut url_batch_ms = 0.0_f64;
        let mut url_unique_request_count = 0usize;

        if !token_requests.is_empty() {
            let token_started_at = Instant::now();
            if let Some(token_client) = &self.token_client {
                let mints: Vec<String> = token_requests
                    .iter()
                    .map(|(_, mint)| mint.clone())
                    .collect();
                match token_client.resolve_token_metadata(&mints).await {
                    Ok(resolved) => {
                        for (key, mint) in token_requests {
                            if let Some(value) = resolved.get(&mint) {
                                results.insert(key, value.clone());
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "Failed to resolve token metadata batch");
                    }
                }
            } else {
                tracing::warn!(
                    count = token_requests.len(),
                    "DAS_API_ENDPOINT not set; token resolver requests will be re-queued"
                );
            }
            token_batch_ms = elapsed_ms(token_started_at);
        }

        if !url_requests.is_empty() {
            let url_dedupe_started_at = Instant::now();
            let mut unique = HashMap::new();
            for (key, url, method) in &url_requests {
                unique
                    .entry((url.clone(), method.clone()))
                    .or_insert_with(Vec::new)
                    .push(key.clone());
            }
            url_unique_request_count = unique.len();

            let batch_input: Vec<(String, HttpMethod)> = unique.keys().cloned().collect();
            url_dedupe_ms = elapsed_ms(url_dedupe_started_at);
            let url_batch_started_at = Instant::now();
            let resolved = self.url_client.resolve_batch(&batch_input).await;
            url_batch_ms = elapsed_ms(url_batch_started_at);

            for ((url, method), keys) in unique {
                if let Some(value) = resolved.get(&(url, method)) {
                    for key in keys {
                        results.insert(key, value.clone());
                    }
                }
            }
        }

        let total_ms = elapsed_ms(total_started_at);
        if should_log_runtime_resolver_profile(total_ms) {
            tracing::info!(
                stage = "runtime.resolver_batch",
                request_count = requests.len(),
                token_request_count,
                url_request_count,
                url_unique_request_count,
                resolved_count = results.len(),
                total_ms,
                classify_ms,
                token_batch_ms,
                url_dedupe_ms,
                url_batch_ms,
                "Runtime resolver batch profile"
            );
        }

        Ok(results)
    }
}

impl RuntimeResolver for InProcessResolver {
    fn resolve_batch<'a>(
        &'a self,
        requests: &'a [RuntimeResolverRequest],
    ) -> ResolverBatchFuture<'a> {
        Box::pin(async move { self.resolve_batch_internal(requests).await })
    }
}

pub fn runtime_resolver_cache_key(resolver: &ResolverType, input: &Value) -> String {
    crate::vm::resolver_cache_key(resolver, input)
}

fn runtime_request_from_vm_request(request: &ResolverRequest) -> Option<RuntimeResolverRequest> {
    match &request.resolver {
        ResolverType::Token => extract_mint_from_input(&request.input).map(|mint| {
            RuntimeResolverRequest::TokenMetadata {
                key: request.cache_key.clone(),
                mint,
            }
        }),
        ResolverType::Url(config) => match &request.input {
            Value::String(url) if !url.is_empty() => Some(RuntimeResolverRequest::UrlJson {
                key: request.cache_key.clone(),
                url: url.clone(),
                method: config.method.clone(),
            }),
            _ => None,
        },
    }
}

fn extract_mint_from_input(input: &Value) -> Option<String> {
    match input {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::Object(map) => map
            .get("mint")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_request_extracts_mint_from_object_input() {
        let request = ResolverRequest {
            cache_key: "token:mint".to_string(),
            resolver: ResolverType::Token,
            input: serde_json::json!({ "mint": "abc" }),
        };

        let runtime_request = runtime_request_from_vm_request(&request).unwrap();
        assert_eq!(
            runtime_request,
            RuntimeResolverRequest::TokenMetadata {
                key: "token:mint".to_string(),
                mint: "abc".to_string(),
            }
        );
    }

    #[test]
    fn url_request_uses_existing_cache_key() {
        let request = ResolverRequest {
            cache_key: "url:get:https://example.com".to_string(),
            resolver: ResolverType::Url(crate::ast::UrlResolverConfig {
                url_source: crate::ast::UrlSource::FieldPath("metadata_url".to_string()),
                method: HttpMethod::Get,
                extract_path: None,
            }),
            input: serde_json::json!("https://example.com"),
        };

        let runtime_request = runtime_request_from_vm_request(&request).unwrap();
        assert_eq!(
            runtime_request,
            RuntimeResolverRequest::UrlJson {
                key: "url:get:https://example.com".to_string(),
                url: "https://example.com".to_string(),
                method: HttpMethod::Get,
            }
        );
    }
}
