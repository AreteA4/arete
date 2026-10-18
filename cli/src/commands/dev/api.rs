//! The Arete API routes `a4 dev up` deploys the project's protocol with:
//! the self-serve protocol routes `a4 protocol deploy` uses, with each
//! deployment bound to the project's cloud dev environment, and the
//! environment's stack listing.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};

/// How long activation may answer `202` (release still loading).
const ACTIVATE_TIMEOUT: Duration = Duration::from_secs(180);

/// A deployment as `POST /api/protocols/:id/deployments` returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedDeployment {
    pub deployment_id: String,
    pub short_id: String,
    /// `null` when the API could not read it from its signer.
    pub attestation_pubkey: Option<String>,
}

/// A stack serving in the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvStack {
    pub name: String,
    pub ws_url: String,
    /// The stack deployment's id.
    pub deployment_id: i64,
}

/// The protocol routes of one Arete API, with the account's secret key.
#[derive(Debug, Clone)]
pub struct DevApi {
    origin: String,
    key: String,
    http: reqwest::Client,
}

fn string_field(body: &Value, field: &str, what: &str) -> Result<String> {
    body.get(field)
        .and_then(|value| match value {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("{what} returned no {field}: {body}"))
}

/// The message of an API error body (`{error, code}`), with its code.
fn api_message(body: &Value) -> String {
    let message = body["error"]
        .as_str()
        .or(body["message"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| body.to_string());
    match body["code"].as_str() {
        Some(code) => format!("{message} ({code})"),
        None => message,
    }
}

impl DevApi {
    pub fn new(origin: &str, key: &str) -> Self {
        Self {
            origin: arete_mcp::dev_env::api_origin(origin),
            key: key.to_string(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
        }
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(reqwest::StatusCode, Value)> {
        let url = format!("{}{path}", self.origin);
        let mut request = self
            .http
            .request(method.clone(), &url)
            .bearer_auth(&self.key);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("The Arete API is unreachable ({method} {path})"))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let value = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        };
        Ok((status, value))
    }

    async fn expect(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
        what: &str,
    ) -> Result<Value> {
        let (status, value) = self.call(method, path, body).await?;
        if !status.is_success() {
            bail!("{what} failed ({status}): {}", api_message(&value));
        }
        Ok(value)
    }

    /// Create the protocol, or find the account's protocol with this slug.
    pub async fn ensure_protocol(&self, slug: &str, name: &str) -> Result<String> {
        let (status, body) = self
            .call(
                reqwest::Method::POST,
                "/api/protocols",
                Some(&json!({ "slug": slug, "name": name })),
            )
            .await?;
        if status.is_success() {
            return string_field(&body, "id", "Creating the protocol");
        }
        if status == reqwest::StatusCode::CONFLICT {
            let (status, list) = self
                .call(reqwest::Method::GET, "/api/protocols", None)
                .await?;
            if status.is_success() {
                let items = list
                    .as_array()
                    .or_else(|| list.get("protocols").and_then(Value::as_array))
                    .or_else(|| list.get("items").and_then(Value::as_array));
                if let Some(found) = items
                    .into_iter()
                    .flatten()
                    .find(|item| item.get("slug").and_then(Value::as_str) == Some(slug))
                {
                    return string_field(found, "id", "Listing protocols");
                }
            }
            bail!(
                "The protocol slug '{slug}' is taken by another account; set [protocol].slug in arete.toml to another one"
            );
        }
        bail!(
            "Creating protocol '{slug}' failed ({status}): {}",
            api_message(&body)
        )
    }

    /// Upload a release; returns the response (`releaseHash`, `hasHandlers`, …).
    pub async fn upload_release(
        &self,
        protocol_id: &str,
        spec: &Value,
        bundle: Option<&[u8]>,
    ) -> Result<Value> {
        let mut body = json!({ "spec": spec });
        if let Some(bundle) = bundle {
            body["bundleBase64"] = json!(base64::engine::general_purpose::STANDARD.encode(bundle));
        }
        let response = self
            .expect(
                reqwest::Method::POST,
                &format!("/api/protocols/{protocol_id}/releases"),
                Some(&body),
                "Uploading the release",
            )
            .await?;
        string_field(&response, "releaseHash", "Uploading the release")?;
        Ok(response)
    }

    /// Store a fee policy document; its id.
    pub async fn create_fee_policy(&self, protocol_id: &str, document: &Value) -> Result<String> {
        let response = self
            .expect(
                reqwest::Method::POST,
                &format!("/api/protocols/{protocol_id}/fee-policies"),
                Some(&json!({ "document": document })),
                "Storing the fee policy",
            )
            .await?;
        string_field(&response, "id", "Storing the fee policy")
    }

    /// Create and publish a version of the resource `name` for
    /// `release_hash`; the published version number.
    pub async fn publish_resource(
        &self,
        protocol_id: &str,
        name: &str,
        value: &Value,
        release_hash: &str,
    ) -> Result<i64> {
        let what = format!("Publishing resource {name}");
        let draft = self
            .expect(
                reqwest::Method::POST,
                &format!("/api/protocols/{protocol_id}/resources/{name}/versions"),
                Some(&json!({ "value": value, "releaseHash": release_hash })),
                &what,
            )
            .await?;
        let version = draft
            .get("version")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow!("{what} returned no version: {draft}"))?;
        let published = self
            .expect(
                reqwest::Method::POST,
                &format!(
                    "/api/protocols/{protocol_id}/resources/{name}/versions/{version}/publish"
                ),
                Some(&json!({ "releaseHash": release_hash })),
                &what,
            )
            .await?;
        Ok(published
            .get("version")
            .and_then(Value::as_i64)
            .unwrap_or(version))
    }

    /// Create a private deployment of `release_hash` in the dev environment
    /// `environment`: its chain reads, simulations and sends go to the
    /// environment, and only the account can use it.
    pub async fn create_deployment(
        &self,
        protocol_id: &str,
        release_hash: &str,
        resources: &BTreeMap<String, i64>,
        fee_policy_id: &str,
        environment: &str,
    ) -> Result<CreatedDeployment> {
        let response = self
            .expect(
                reqwest::Method::POST,
                &format!("/api/protocols/{protocol_id}/deployments"),
                Some(&json!({
                    "releaseHash": release_hash,
                    "resources": resources,
                    "feePolicyId": fee_policy_id,
                    "accessMode": "private",
                    "environment": environment,
                })),
                "Creating the deployment",
            )
            .await?;
        Ok(CreatedDeployment {
            deployment_id: string_field(&response, "id", "Creating the deployment")?,
            short_id: string_field(&response, "shortId", "Creating the deployment")?,
            attestation_pubkey: response
                .get("attestationPubkey")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    /// Activate, retrying while the API answers `202` (still preparing).
    pub async fn activate(&self, deployment_id: &str) -> Result<Value> {
        let deadline = Instant::now() + ACTIVATE_TIMEOUT;
        loop {
            let (status, body) = self
                .call(
                    reqwest::Method::POST,
                    &format!("/api/protocol-deployments/{deployment_id}/activate"),
                    Some(&json!({})),
                )
                .await?;
            if status == reqwest::StatusCode::ACCEPTED {
                if Instant::now() >= deadline {
                    bail!(
                        "Activation is still pending after {}s: {}",
                        ACTIVATE_TIMEOUT.as_secs(),
                        api_message(&body)
                    );
                }
                tokio::time::sleep(Duration::from_millis(1_000)).await;
                continue;
            }
            if !status.is_success() {
                bail!(
                    "Activating the deployment failed ({status}): {}",
                    api_message(&body)
                );
            }
            return Ok(body);
        }
    }

    /// Disable a deployment (the owner route). One that is already disabled
    /// (`409`) counts as disabled.
    pub async fn disable(&self, deployment_id: &str) -> Result<()> {
        let (status, body) = self
            .call(
                reqwest::Method::POST,
                &format!("/api/protocol-deployments/{deployment_id}/disable"),
                Some(&json!({})),
            )
            .await?;
        if status.is_success() || status == reqwest::StatusCode::CONFLICT {
            return Ok(());
        }
        bail!(
            "Disabling deployment {deployment_id} failed ({status}): {}",
            api_message(&body)
        )
    }

    /// The stack of this exact protocol deployment in the environment.
    pub async fn protocol_env_stack(
        &self,
        environment: &str,
        protocol_deployment: &str,
    ) -> Result<Option<EnvStack>> {
        let stacks = self.env_stacks(environment).await?;
        Ok(stacks.into_iter().find_map(|stack| {
            (stack["protocolDeploymentId"].as_str() == Some(protocol_deployment))
                .then(|| env_stack(&stack))
                .flatten()
        }))
    }

    pub async fn env_stacks(&self, environment: &str) -> Result<Vec<Value>> {
        let listing = self
            .expect(
                reqwest::Method::GET,
                &format!("/api/dev/environments/{environment}/stacks"),
                None,
                "Listing the environment's stacks",
            )
            .await?;
        Ok(listing["stacks"].as_array().cloned().unwrap_or_default())
    }
}

pub fn env_stack(stack: &Value) -> Option<EnvStack> {
    if stack["assigned"] != true {
        return None;
    }
    Some(EnvStack {
        name: stack["specName"].as_str()?.to_string(),
        ws_url: stack["websocketUrl"].as_str()?.to_string(),
        deployment_id: stack["deploymentId"].as_i64()?,
    })
}
