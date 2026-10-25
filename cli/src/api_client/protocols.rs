//! Self-serve protocol endpoints: protocols, releases, resources, fee
//! policies, deployments, webhook endpoints and registry listings.
//!
//! Every route needs a secret key, except the public listing search and
//! lookup (`/api/registry/protocols`). Response types are lenient (unknown
//! fields are ignored) so a newer server stays compatible with this CLI.

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ApiClient;

/// A protocol the caller owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolRecord {
    pub id: String,
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// The result of uploading a spec (and handler bundle).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolReleaseUpload {
    #[serde(default)]
    pub schema: Option<String>,
    pub protocol_id: String,
    pub release_hash: String,
    #[serde(default)]
    pub spec_hash: Option<String>,
    #[serde(default)]
    pub stack_source_hash: Option<String>,
    #[serde(default)]
    pub handler_bundle_hash: Option<String>,
    pub has_handlers: bool,
    /// `pending`, `passed` or `failed`.
    pub template_check: String,
    pub reviewed: bool,
    /// `false` when the identical release was already registered.
    pub created: bool,
}

/// What the server's release check reports for a release it would accept:
/// the identities an upload of the same body derives, and its template
/// check. A rejected release is an API error with the upload's problem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolReleaseCheck {
    #[serde(default)]
    pub schema: Option<String>,
    pub release_hash: String,
    #[serde(default)]
    pub spec_hash: Option<String>,
    #[serde(default)]
    pub stack_source_hash: Option<String>,
    #[serde(default)]
    pub handler_bundle_hash: Option<String>,
    pub has_handlers: bool,
    /// `passed`, or `pending` while a pinned program spec is not on the
    /// server yet.
    pub template_check: String,
    /// `{alias, programSpecHash}` of each program spec the server lacks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_program_specs: Vec<Value>,
}

/// A draft or published resource version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolResourceVersion {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub version: i64,
    pub status: String,
    #[serde(default)]
    pub resource_hash: Option<String>,
}

/// A stored fee policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolFeePolicy {
    pub id: String,
    #[serde(default)]
    pub version: Option<i64>,
    #[serde(default)]
    pub document_sha256: Option<String>,
}

/// A protocol deployment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolDeployment {
    pub id: String,
    #[serde(default)]
    pub protocol_id: Option<String>,
    /// Cloud dev environment binding; absent for production deployments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    pub short_id: String,
    /// `disabled`, `activating` or `active`.
    pub status: String,
    #[serde(default)]
    pub revision: Option<i64>,
    pub release_hash: String,
    pub access_mode: String,
    #[serde(default)]
    pub attestation_key_id: Option<String>,
    /// `reviewed` or `unreviewed`.
    #[serde(default)]
    pub trust_tier: Option<String>,
    #[serde(default)]
    pub runtime_kind: Option<String>,
}

/// The result of activating a deployment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolDeploymentTransition {
    pub deployment: ProtocolDeployment,
    /// What activation still waits for, when it has not finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Value>,
    /// The owner-private registry package the activation published (or
    /// found, or skipped), when the server reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_package: Option<Value>,
}

/// A deployment's registry listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolListingStatus {
    pub deployment_id: String,
    /// `requested`, `listed`, `rejected` or `delisted`.
    pub status: String,
    /// `reviewed` or `unreviewed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// A webhook endpoint subscribed to a deployment's emitted events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolWebhook {
    pub id: String,
    #[serde(default)]
    pub deployment_id: Option<String>,
    pub url: String,
    /// `publisher` or `integrator`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Value>,
    #[serde(default)]
    pub events: Vec<String>,
    pub status: String,
    /// The end of the signing secret, to tell endpoints apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// The signing secret: returned once, when the endpoint is created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

/// The result of sending a test delivery to a webhook endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolWebhookTest {
    pub delivery_id: String,
    pub delivered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Largest page of `GET /api/registry/protocols`.
pub const MAX_PROTOCOL_LISTING_LIMIT: usize = 50;

/// Readable guidance for the listing refusal and lookup codes.
pub fn protocol_listing_message(code: &str, details: Option<&Value>) -> Option<String> {
    Some(match code {
        "protocol-listing-not-public" => "Only public deployments can be listed, and this one is private. Self-serve deployments are private: ask Arete to make the deployment public, then request the listing again".to_string(),
        "protocol-listing-edges-private" => {
            let edges: Vec<String> = details
                .and_then(|details| details["edges"].as_array())
                .into_iter()
                .flatten()
                .map(|edge| match edge {
                    Value::String(edge) => edge.clone(),
                    other => other.to_string(),
                })
                .collect();
            format!(
                "A listed protocol may only use public stacks and programs, and this one uses private ones{}. Publish them (or switch to public packages), deploy again and request the listing for the new deployment",
                if edges.is_empty() {
                    String::new()
                } else {
                    format!(": {}", edges.join(", "))
                }
            )
        }
        "protocol-listing-fee-policy" => "The deployment's fee policy does not meet the listing rules (the minimum platform fee or the fee destination). Deploy with a fee policy that meets them, then request the listing for the new deployment".to_string(),
        "protocol-listing-not-owner" => "This deployment belongs to another account. Use a secret key of the account that deployed it".to_string(),
        "protocol-listing-account-standing" => "This account cannot list protocols right now because of its standing. Contact Arete support".to_string(),
        "protocol-listing-not-found" => "No listed protocol has that slug. Search the listings with `a4 protocol search`".to_string(),
        _ => return None,
    })
}

impl ApiClient {
    /// Rewrite a listing refusal's message with [`protocol_listing_message`].
    fn listing_result<T>(result: Result<T>) -> Result<T> {
        result.map_err(|error| match error.downcast::<super::ApiClientError>() {
            Ok(mut api_error) => {
                if let Some(message) = api_error.problem.code.as_deref().and_then(|code| {
                    protocol_listing_message(code, api_error.problem.extra.get("details"))
                }) {
                    api_error.problem.error = message;
                }
                api_error.into()
            }
            Err(error) => error,
        })
    }

    /// Search the listed protocol packages (`GET /api/registry/protocols`).
    /// The raw page is returned so `--json` prints what the registry sent.
    pub fn search_protocol_listings(
        &self,
        query: Option<&str>,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Value> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) {
            params.push(("q", query.to_string()));
        }
        if let Some(cursor) = cursor.map(str::trim).filter(|cursor| !cursor.is_empty()) {
            params.push(("cursor", cursor.to_string()));
        }
        if let Some(limit) = limit {
            params.push((
                "limit",
                limit.clamp(1, MAX_PROTOCOL_LISTING_LIMIT).to_string(),
            ));
        }
        let response = self
            .with_optional_auth(
                self.client
                    .get(format!("{}/api/registry/protocols", self.base_url))
                    .query(&params),
            )
            .send()
            .context("Failed to send protocol listing search request")?;
        Self::handle_response(response)
    }

    /// One listed protocol package by slug.
    pub fn get_protocol_listing(&self, slug: &str) -> Result<Value> {
        let response = self
            .with_optional_auth(self.client.get(format!(
                "{}/api/registry/protocols/{}",
                self.base_url,
                super::registry_path_segment(slug)?
            )))
            .send()
            .context("Failed to send protocol listing request")?;
        Self::listing_result(Self::handle_response(response))
    }

    /// Ask for a deployment to be listed in the public registry.
    pub fn request_protocol_listing(
        &self,
        deployment_id: &str,
        note: Option<&str>,
    ) -> Result<ProtocolListingStatus> {
        let mut body = json!({});
        if let Some(note) = note.map(str::trim).filter(|note| !note.is_empty()) {
            body["note"] = Value::String(note.to_string());
        }
        Self::listing_result(self.protocol_post(
            &format!("/api/protocol-deployments/{deployment_id}/listing"),
            &body,
            "request protocol listing",
        ))
    }

    /// A deployment's listing state.
    pub fn get_protocol_listing_status(
        &self,
        deployment_id: &str,
    ) -> Result<ProtocolListingStatus> {
        Self::listing_result(self.protocol_get(
            &format!("/api/protocol-deployments/{deployment_id}/listing"),
            "read protocol listing",
        ))
    }

    /// Whether the configured key is a publishable (browser) key, which the
    /// protocol routes refuse.
    pub(crate) fn api_key_is_publishable(&self) -> bool {
        self.api_key
            .as_deref()
            .is_some_and(|key| key.starts_with("a4_pk_"))
    }

    fn protocol_get<T: DeserializeOwned>(&self, path: &str, what: &str) -> Result<T> {
        let api_key = self.require_api_key()?;
        let response = self
            .client
            .get(format!("{}{path}", self.base_url))
            .bearer_auth(api_key)
            .send()
            .with_context(|| format!("Failed to send {what} request"))?;
        Self::handle_response(response)
    }

    fn protocol_delete<T: DeserializeOwned>(&self, path: &str, what: &str) -> Result<T> {
        let api_key = self.require_api_key()?;
        let response = self
            .client
            .delete(format!("{}{path}", self.base_url))
            .bearer_auth(api_key)
            .send()
            .with_context(|| format!("Failed to send {what} request"))?;
        Self::handle_response(response)
    }

    fn protocol_post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
        what: &str,
    ) -> Result<T> {
        let api_key = self.require_api_key()?;
        let response = self
            .client
            .post(format!("{}{path}", self.base_url))
            .bearer_auth(api_key)
            .json(body)
            .send()
            .with_context(|| format!("Failed to send {what} request"))?;
        Self::handle_response(response)
    }

    pub fn list_protocols(&self) -> Result<Vec<ProtocolRecord>> {
        self.protocol_get("/api/protocols", "list protocols")
    }

    pub fn create_protocol(&self, slug: &str, name: &str) -> Result<ProtocolRecord> {
        self.protocol_post(
            "/api/protocols",
            &json!({ "slug": slug, "name": name }),
            "create protocol",
        )
    }

    /// Upload a spec and, exactly when a step has a handler, its bundle as
    /// padded standard base64.
    pub fn upload_protocol_release(
        &self,
        protocol_id: &str,
        spec: &Value,
        bundle_base64: Option<&str>,
    ) -> Result<ProtocolReleaseUpload> {
        let mut body = json!({ "spec": spec });
        if let Some(bundle) = bundle_base64 {
            body["bundleBase64"] = Value::String(bundle.to_string());
        }
        self.protocol_post(
            &format!("/api/protocols/{protocol_id}/releases"),
            &body,
            "upload protocol release",
        )
    }

    /// Run the upload's checks on a spec (and bundle) without uploading it
    /// or naming a protocol. A rejection is the upload's own API error.
    pub fn check_protocol_release(
        &self,
        spec: &Value,
        bundle_base64: Option<&str>,
    ) -> Result<ProtocolReleaseCheck> {
        let mut body = json!({ "spec": spec });
        if let Some(bundle) = bundle_base64 {
            body["bundleBase64"] = Value::String(bundle.to_string());
        }
        self.protocol_post("/api/protocols/check", &body, "check protocol release")
    }

    pub fn create_protocol_resource_version(
        &self,
        protocol_id: &str,
        name: &str,
        value: &Value,
        release_hash: &str,
    ) -> Result<ProtocolResourceVersion> {
        self.protocol_post(
            &format!("/api/protocols/{protocol_id}/resources/{name}/versions"),
            &json!({ "value": value, "releaseHash": release_hash }),
            "create protocol resource version",
        )
    }

    pub fn publish_protocol_resource_version(
        &self,
        protocol_id: &str,
        name: &str,
        version: i64,
        release_hash: &str,
    ) -> Result<ProtocolResourceVersion> {
        self.protocol_post(
            &format!("/api/protocols/{protocol_id}/resources/{name}/versions/{version}/publish"),
            &json!({ "releaseHash": release_hash }),
            "publish protocol resource version",
        )
    }

    pub fn create_protocol_fee_policy(
        &self,
        protocol_id: &str,
        document: &Value,
    ) -> Result<ProtocolFeePolicy> {
        self.protocol_post(
            &format!("/api/protocols/{protocol_id}/fee-policies"),
            &json!({ "document": document }),
            "create protocol fee policy",
        )
    }

    pub fn list_protocol_deployments(&self, protocol_id: &str) -> Result<Vec<ProtocolDeployment>> {
        self.protocol_get(
            &format!("/api/protocols/{protocol_id}/deployments"),
            "list protocol deployments",
        )
    }

    /// Create a private deployment. The server derives the attestation key.
    pub fn create_protocol_deployment(
        &self,
        protocol_id: &str,
        release_hash: &str,
        resources: &std::collections::BTreeMap<String, i64>,
        fee_policy_id: &str,
    ) -> Result<ProtocolDeployment> {
        self.protocol_post(
            &format!("/api/protocols/{protocol_id}/deployments"),
            &json!({
                "releaseHash": release_hash,
                "resources": resources,
                "feePolicyId": fee_policy_id,
                "accessMode": "private",
            }),
            "create protocol deployment",
        )
    }

    pub fn activate_protocol_deployment(
        &self,
        deployment_id: &str,
    ) -> Result<ProtocolDeploymentTransition> {
        self.protocol_post(
            &format!("/api/protocol-deployments/{deployment_id}/activate"),
            &json!({}),
            "activate protocol deployment",
        )
    }

    /// Subscribe `url` to `events` of a deployment. The response carries the
    /// signing secret, which the server never returns again.
    pub fn create_protocol_webhook(
        &self,
        deployment_id: &str,
        url: &str,
        events: &[String],
    ) -> Result<ProtocolWebhook> {
        self.protocol_post(
            &format!("/api/protocol-deployments/{deployment_id}/webhooks"),
            &json!({ "url": url, "events": events }),
            "create protocol webhook",
        )
    }

    /// The caller's webhook endpoints on a deployment.
    pub fn list_protocol_webhooks(&self, deployment_id: &str) -> Result<Vec<ProtocolWebhook>> {
        self.protocol_get(
            &format!("/api/protocol-deployments/{deployment_id}/webhooks"),
            "list protocol webhooks",
        )
    }

    /// Disable a webhook endpoint: it receives no further deliveries.
    pub fn disable_protocol_webhook(
        &self,
        deployment_id: &str,
        webhook_id: &str,
    ) -> Result<ProtocolWebhook> {
        self.protocol_delete(
            &format!("/api/protocol-deployments/{deployment_id}/webhooks/{webhook_id}"),
            "disable protocol webhook",
        )
    }

    /// Send a signed test delivery to a webhook endpoint now.
    pub fn test_protocol_webhook(
        &self,
        deployment_id: &str,
        webhook_id: &str,
    ) -> Result<ProtocolWebhookTest> {
        self.protocol_post(
            &format!("/api/protocol-deployments/{deployment_id}/webhooks/{webhook_id}/test"),
            &json!({}),
            "test protocol webhook",
        )
    }
}
