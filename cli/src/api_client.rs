use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Production API URL (used by default in release builds)
#[cfg(not(feature = "local"))]
const DEFAULT_API_URL: &str = "https://api.arete.run";

/// Local development API URL (enabled with --features local)
#[cfg(feature = "local")]
const DEFAULT_API_URL: &str = "http://localhost:3000";

/// Default domain suffix for WebSocket URLs
pub const DEFAULT_DOMAIN_SUFFIX: &str = "stack.arete.run";

#[derive(Debug, Clone)]
pub struct ApiClient {
    base_url: String,
    api_key: Option<String>,
    client: reqwest::blocking::Client,
}

// DTOs matching backend models
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spec {
    pub id: i32,
    pub user_id: i32,
    pub name: String,
    pub entity_name: String,
    pub crate_name: String,
    pub module_path: String,
    pub description: Option<String>,
    pub package_name: Option<String>,
    pub output_path: Option<String>,
    pub url_slug: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Spec {
    pub fn websocket_url(&self, domain_suffix: &str) -> String {
        format!(
            "wss://{}-{}.{}",
            self.name.to_lowercase(),
            self.url_slug,
            domain_suffix
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateSpecRequest {
    pub name: String,
    pub entity_name: String,
    pub crate_name: String,
    pub module_path: String,
    pub description: Option<String>,
    pub package_name: Option<String>,
    pub output_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateSpecRequest {
    pub name: Option<String>,
    pub entity_name: Option<String>,
    pub crate_name: Option<String>,
    pub module_path: Option<String>,
    pub description: Option<String>,
    pub package_name: Option<String>,
    pub output_path: Option<String>,
}

// ============================================================================
// Spec Version DTOs
// ============================================================================

/// Combined view of spec version with its AST content
#[derive(Debug, Serialize, Deserialize)]
pub struct SpecVersionWithContent {
    pub id: i32,
    pub spec_id: i32,
    pub version_number: i32,
    pub content_hash: String,
    pub version_created_at: String,
    // AST content info
    pub state_name: String,
    pub program_id: Option<String>,
    pub handler_count: i32,
    pub section_count: i32,
}

#[derive(Debug, Serialize)]
pub struct CreateSpecVersionRequest {
    pub ast_payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct CreateSpecVersionResponse {
    pub version: SpecVersionWithContent,
    /// True if the AST content already existed globally
    pub content_is_new: bool,
    /// True if this spec version is new (same spec didn't have this content before)
    pub version_is_new: bool,
    #[allow(dead_code)]
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct SpecWithVersion {
    #[serde(flatten)]
    #[allow(dead_code)]
    pub spec: Spec,
    pub latest_version: Option<SpecVersionWithContent>,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: String,
}

// ============================================================================
// Build DTOs
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildStatus {
    Pending,
    Uploading,
    Queued,
    Building,
    Pushing,
    Deploying,
    Completed,
    Failed,
    Cancelled,
}

impl std::fmt::Display for BuildStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildStatus::Pending => write!(f, "pending"),
            BuildStatus::Uploading => write!(f, "uploading"),
            BuildStatus::Queued => write!(f, "queued"),
            BuildStatus::Building => write!(f, "building"),
            BuildStatus::Pushing => write!(f, "pushing"),
            BuildStatus::Deploying => write!(f, "deploying"),
            BuildStatus::Completed => write!(f, "completed"),
            BuildStatus::Failed => write!(f, "failed"),
            BuildStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl BuildStatus {
    /// Returns true if this is a terminal state (no more transitions expected)
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            BuildStatus::Completed | BuildStatus::Failed | BuildStatus::Cancelled
        )
    }
}

/// Sanitized Build response from API (excludes AWS internals)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Build {
    pub id: i32,
    pub spec_id: Option<i32>,
    pub spec_version_id: Option<i32>,
    pub status: BuildStatus,
    pub status_message: Option<String>,
    pub phase: Option<String>,
    pub progress: Option<i32>,
    pub websocket_url: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
}

/// Sanitized BuildEvent response from API (excludes raw_payload and event_source)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildEvent {
    pub id: i32,
    pub build_id: i32,
    pub event_type: String,
    pub phase: Option<String>,
    pub previous_status: Option<BuildStatus>,
    pub new_status: Option<BuildStatus>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct CreateBuildRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec_version_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ast_payload: Option<serde_json::Value>,
    /// Branch name for branch deployments (e.g., "preview-abc123")
    /// Branch deployments get URL: {spec-name}-{branch}.stack.arete.run
    /// Production deployments (no branch) get: {spec-name}.stack.arete.run
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBuildResponse {
    pub build_id: i32,
    pub status: BuildStatus,
    #[allow(dead_code)]
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BuildStatusResponse {
    pub build: Build,
    pub events: Vec<BuildEvent>,
}

// ============================================================================
// Deployment DTOs
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentStatus {
    Active,
    Updating,
    Stopped,
    Failed,
}

impl std::fmt::Display for DeploymentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeploymentStatus::Active => write!(f, "active"),
            DeploymentStatus::Updating => write!(f, "updating"),
            DeploymentStatus::Stopped => write!(f, "stopped"),
            DeploymentStatus::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentResponse {
    pub id: i32,
    pub spec_id: i32,
    pub spec_name: String,
    pub atom_name: String,
    pub branch: Option<String>,
    pub current_build_id: Option<i32>,
    pub current_version: Option<i32>,
    pub current_image_tag: Option<String>,
    pub websocket_url: String,
    pub status: DeploymentStatus,
    pub status_message: Option<String>,
    pub first_deployed_at: Option<String>,
    pub last_deployed_at: Option<String>,
}

// ============================================================================
// API Key DTOs
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: i32,
    pub user_id: i32,
    pub name: Option<String>,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub key_class: String,
    pub origin_allowlist: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct CreatePublishableKeyRequest {
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry_days: Option<i64>,
    pub origin_allowlist: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct CreateApiKeyResponse {
    pub id: i32,
    pub key: String,
    pub name: Option<String>,
    pub key_class: String,
    pub expires_at: String,
    pub message: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct StopDeploymentResponse {
    pub message: String,
    pub deployment_id: i32,
    pub status: DeploymentStatus,
}

// ========================================================================
// Registry DTOs
// ========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryStackItem {
    pub name: String,
    pub description: Option<String>,
    pub websocket_url: String,
    pub entities: Vec<String>,
    #[serde(default)]
    pub visibility: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrySchemaResponse {
    pub name: String,
    pub websocket_url: String,
    pub description: Option<String>,
    pub schema: StackSchema,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryAstResponse {
    pub name: String,
    pub stack: String,
    pub websocket_url: String,
    pub description: Option<String>,
    pub visibility: String,
    pub ast_payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackSchema {
    pub stack_name: String,
    pub entities: Vec<EntitySchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySchema {
    pub name: String,
    pub primary_keys: Vec<String>,
    pub fields: Vec<FieldSchema>,
    pub views: Vec<ViewSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSchema {
    pub path: String,
    pub rust_type: String,
    pub nullable: bool,
    pub section: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewSchema {
    pub id: String,
    pub mode: String,
    pub pipeline: Vec<serde_json::Value>,
}

impl ApiClient {
    pub fn new() -> Result<Self> {
        let base_url =
            std::env::var("ARETE_API_URL").unwrap_or_else(|_| DEFAULT_API_URL.to_string());

        let api_key = Self::load_api_key_for_url(&base_url).ok();

        Ok(ApiClient {
            base_url,
            api_key,
            client: reqwest::blocking::Client::new(),
        })
    }

    #[allow(dead_code)]
    pub fn with_api_key(mut self, api_key: String) -> Self {
        self.api_key = Some(api_key);
        self
    }

    // Spec endpoints

    pub fn list_specs(&self) -> Result<Vec<Spec>> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .get(format!("{}/api/specs", self.base_url))
            .bearer_auth(api_key)
            .send()
            .context("Failed to send list specs request")?;

        Self::handle_response(response)
    }

    #[allow(dead_code)]
    pub fn get_spec(&self, spec_id: i32) -> Result<Spec> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .get(format!("{}/api/specs/{}", self.base_url, spec_id))
            .bearer_auth(api_key)
            .send()
            .context("Failed to send get spec request")?;

        Self::handle_response(response)
    }

    pub fn create_spec(&self, req: CreateSpecRequest) -> Result<Spec> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .post(format!("{}/api/specs", self.base_url))
            .bearer_auth(api_key)
            .json(&req)
            .send()
            .context("Failed to send create spec request")?;

        Self::handle_response(response)
    }

    pub fn update_spec(&self, spec_id: i32, req: UpdateSpecRequest) -> Result<Spec> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .put(format!("{}/api/specs/{}", self.base_url, spec_id))
            .bearer_auth(api_key)
            .json(&req)
            .send()
            .context("Failed to send update spec request")?;

        Self::handle_response(response)
    }

    pub fn delete_spec(&self, spec_id: i32) -> Result<()> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .delete(format!("{}/api/specs/{}", self.base_url, spec_id))
            .bearer_auth(api_key)
            .send()
            .context("Failed to send delete spec request")?;

        if response.status().is_success() {
            Ok(())
        } else {
            let error: ErrorResponse = response.json()?;
            anyhow::bail!("API error: {}", error.error);
        }
    }

    // Spec version endpoints

    /// Upload AST to create a new spec version
    pub fn create_spec_version(
        &self,
        spec_id: i32,
        ast_payload: serde_json::Value,
    ) -> Result<CreateSpecVersionResponse> {
        let api_key = self.require_api_key()?;

        let req = CreateSpecVersionRequest { ast_payload };

        let response = self
            .client
            .post(format!("{}/api/specs/{}/versions", self.base_url, spec_id))
            .bearer_auth(api_key)
            .json(&req)
            .send()
            .context("Failed to send create spec version request")?;

        Self::handle_response(response)
    }

    /// Get spec with its latest version info
    pub fn get_spec_with_latest_version(&self, spec_id: i32) -> Result<SpecWithVersion> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .get(format!(
                "{}/api/specs/{}/versions/latest",
                self.base_url, spec_id
            ))
            .bearer_auth(api_key)
            .send()
            .context("Failed to send get spec with version request")?;

        Self::handle_response(response)
    }

    /// List all versions for a spec
    pub fn list_spec_versions(&self, spec_id: i32) -> Result<Vec<SpecVersionWithContent>> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .get(format!("{}/api/specs/{}/versions", self.base_url, spec_id))
            .bearer_auth(api_key)
            .send()
            .context("Failed to send list spec versions request")?;

        Self::handle_response(response)
    }

    /// List all versions for a spec with pagination
    pub fn list_spec_versions_paginated(
        &self,
        spec_id: i32,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<SpecVersionWithContent>> {
        let api_key = self.require_api_key()?;

        let mut url = format!("{}/api/specs/{}/versions", self.base_url, spec_id);
        let mut params = vec![];
        if let Some(l) = limit {
            params.push(format!("limit={}", l));
        }
        if let Some(o) = offset {
            params.push(format!("offset={}", o));
        }
        if !params.is_empty() {
            url = format!("{}?{}", url, params.join("&"));
        }

        let response = self
            .client
            .get(&url)
            .bearer_auth(api_key)
            .send()
            .context("Failed to send list spec versions request")?;

        Self::handle_response(response)
    }

    /// Helper to get spec by name
    pub fn get_spec_by_name(&self, name: &str) -> Result<Option<Spec>> {
        let specs = self.list_specs()?;
        Ok(specs.into_iter().find(|s| s.name == name))
    }

    // ========================================================================
    // Registry endpoints (public, optional auth for global stacks)
    // ========================================================================

    fn with_optional_auth(
        &self,
        request: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        if let Some(api_key) = &self.api_key {
            request.bearer_auth(api_key)
        } else {
            request
        }
    }

    /// List all registry stacks. Auth expands results to global visibility.
    pub fn list_registry(&self) -> Result<Vec<RegistryStackItem>> {
        let response = self
            .with_optional_auth(self.client.get(format!("{}/api/registry", self.base_url)))
            .send()
            .context("Failed to send registry list request")?;

        Self::handle_response(response)
    }

    /// Get a registry stack's info. Auth expands access to global visibility.
    #[allow(dead_code)]
    pub fn get_registry_stack(&self, name: &str) -> Result<RegistryStackItem> {
        let response = self
            .with_optional_auth(
                self.client
                    .get(format!("{}/api/registry/{}", self.base_url, name)),
            )
            .send()
            .context("Failed to send registry get request")?;

        Self::handle_response(response)
    }

    /// Get full schema for a registry stack. Auth expands access to global visibility.
    pub fn get_registry_schema(&self, name: &str) -> Result<RegistrySchemaResponse> {
        let response = self
            .with_optional_auth(
                self.client
                    .get(format!("{}/api/registry/{}/schema", self.base_url, name)),
            )
            .send()
            .context("Failed to send registry schema request")?;

        Self::handle_response(response)
    }

    /// Get raw AST for a deployed registry stack identifier.
    pub fn get_registry_ast_by_stack(&self, stack: &str) -> Result<RegistryAstResponse> {
        let response = self
            .with_optional_auth(self.client.get(format!(
                "{}/api/registry/stacks/{}/ast",
                self.base_url, stack
            )))
            .send()
            .context("Failed to send registry AST request")?;

        Self::handle_response(response)
    }

    // ========================================================================
    // Authenticated schema endpoints
    // ========================================================================

    /// Get schema for user's own spec (requires auth)
    pub fn get_spec_schema(&self, spec_id: i32) -> Result<RegistrySchemaResponse> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .get(format!("{}/api/specs/{}/schema", self.base_url, spec_id))
            .bearer_auth(api_key)
            .send()
            .context("Failed to send spec schema request")?;

        Self::handle_response(response)
    }

    // ========================================================================
    // Build endpoints
    // ========================================================================

    /// Create a new build
    pub fn create_build(&self, req: CreateBuildRequest) -> Result<CreateBuildResponse> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .post(format!("{}/api/builds", self.base_url))
            .bearer_auth(api_key)
            .json(&req)
            .send()
            .context("Failed to send create build request")?;

        Self::handle_response(response)
    }

    /// List builds for the authenticated user
    pub fn list_builds(&self, limit: Option<i64>, offset: Option<i64>) -> Result<Vec<Build>> {
        self.list_builds_filtered(limit, offset, None)
    }

    /// List builds for the authenticated user, optionally filtered by spec_id
    pub fn list_builds_filtered(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
        spec_id: Option<i32>,
    ) -> Result<Vec<Build>> {
        let api_key = self.require_api_key()?;

        let mut url = format!("{}/api/builds", self.base_url);
        let mut params = vec![];
        if let Some(l) = limit {
            params.push(format!("limit={}", l));
        }
        if let Some(o) = offset {
            params.push(format!("offset={}", o));
        }
        if let Some(sid) = spec_id {
            params.push(format!("spec_id={}", sid));
        }
        if !params.is_empty() {
            url = format!("{}?{}", url, params.join("&"));
        }

        let response = self
            .client
            .get(&url)
            .bearer_auth(api_key)
            .send()
            .context("Failed to send list builds request")?;

        Self::handle_response(response)
    }

    /// Get build status and events by ID
    pub fn get_build(&self, build_id: i32) -> Result<BuildStatusResponse> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .get(format!("{}/api/builds/{}", self.base_url, build_id))
            .bearer_auth(api_key)
            .send()
            .context("Failed to send get build request")?;

        Self::handle_response(response)
    }

    // ========================================================================
    // Deployment endpoints
    // ========================================================================

    /// List all deployments for the authenticated user
    pub fn list_deployments(&self, limit: i64) -> Result<Vec<DeploymentResponse>> {
        let api_key = self.require_api_key()?;

        let url = format!("{}/api/deployments?limit={}", self.base_url, limit);

        let response = self
            .client
            .get(&url)
            .bearer_auth(api_key)
            .send()
            .context("Failed to send list deployments request")?;

        Self::handle_response(response)
    }

    /// Get deployment by ID
    #[allow(dead_code)]
    pub fn get_deployment(&self, deployment_id: i32) -> Result<DeploymentResponse> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .get(format!(
                "{}/api/deployments/{}",
                self.base_url, deployment_id
            ))
            .bearer_auth(api_key)
            .send()
            .context("Failed to send get deployment request")?;

        Self::handle_response(response)
    }

    /// Stop a deployment
    pub fn stop_deployment(&self, deployment_id: i32) -> Result<StopDeploymentResponse> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .delete(format!(
                "{}/api/deployments/{}",
                self.base_url, deployment_id
            ))
            .bearer_auth(api_key)
            .send()
            .context("Failed to send stop deployment request")?;

        Self::handle_response(response)
    }

    // ============================================================================
    // API Key endpoints
    // ============================================================================

    /// List all API keys for the authenticated user
    pub fn list_api_keys(&self) -> Result<Vec<ApiKey>> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .get(format!("{}/api/auth/keys", self.base_url))
            .bearer_auth(api_key)
            .send()
            .context("Failed to send list API keys request")?;

        Self::handle_response(response)
    }

    /// Create a new publishable API key for browser use
    pub fn create_publishable_key(
        &self,
        name: Option<String>,
        origins: Vec<String>,
        expiry_days: Option<i64>,
    ) -> Result<CreateApiKeyResponse> {
        let api_key = self.require_api_key()?;

        let req = CreatePublishableKeyRequest {
            name,
            expiry_days,
            origin_allowlist: origins,
        };

        let response = self
            .client
            .post(format!("{}/api/auth/keys/publishable", self.base_url))
            .bearer_auth(api_key)
            .json(&req)
            .send()
            .context("Failed to send create publishable key request")?;

        Self::handle_response(response)
    }

    // Helper methods

    fn require_api_key(&self) -> Result<&str> {
        self.api_key.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Not authenticated for {}. Run 'a4 auth login' first.",
                self.base_url
            )
        })
    }

    fn handle_response<T: for<'de> Deserialize<'de>>(
        response: reqwest::blocking::Response,
    ) -> Result<T> {
        if response.status().is_success() {
            response.json().context("Failed to parse response JSON")
        } else {
            let status = response.status();
            let error: ErrorResponse = response.json().unwrap_or_else(|_| ErrorResponse {
                error: "Unknown error".to_string(),
            });
            anyhow::bail!("API error ({}): {}", status, error.error);
        }
    }

    // Credentials management

    fn credentials_path() -> Result<PathBuf> {
        let home =
            dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?;
        Ok(home.join(".arete").join("credentials.toml"))
    }

    pub fn save_api_key(api_key: &str, api_url: Option<&str>) -> Result<()> {
        let path = Self::credentials_path()?;

        // Create directory if it doesn't exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let target_url = api_url
            .map(|s| s.to_string())
            .or_else(|| std::env::var("ARETE_API_URL").ok())
            .unwrap_or_else(|| DEFAULT_API_URL.to_string());

        // Read existing credentials or create new
        let creds_content = if path.exists() {
            fs::read_to_string(&path).unwrap_or_default()
        } else {
            String::new()
        };

        // Parse existing or create new
        let mut creds: toml::Value = if creds_content.is_empty() {
            toml::Value::Table(toml::map::Map::new())
        } else {
            toml::from_str(&creds_content)
                .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()))
        };

        // Get or create keys table
        let keys = creds
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("Invalid credentials format"))?
            .entry("keys")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("Invalid keys format"))?;

        // Add or update the key for this URL
        keys.insert(target_url.clone(), toml::Value::String(api_key.to_string()));

        // Write back
        let content = toml::to_string_pretty(&creds)?;
        fs::write(&path, content).context("Failed to save API key")?;

        Ok(())
    }

    /// Load API key for a specific URL (new URL-based format)
    pub fn load_api_key_for_url(api_url: &str) -> Result<String> {
        let path = Self::credentials_path()?;
        let content = fs::read_to_string(&path).context("Failed to read credentials file")?;

        let creds: toml::Value =
            toml::from_str(&content).context("Failed to parse credentials file")?;

        // Try new format first: [keys] table with URL mapping
        if let Some(keys) = creds.get("keys").and_then(|k| k.as_table()) {
            // Look for exact match first
            if let Some(key) = keys.get(api_url).and_then(|v| v.as_str()) {
                return Ok(key.to_string());
            }

            // For localhost URLs, try to match any localhost URL
            if api_url.contains("localhost") || api_url.contains("127.0.0.1") {
                for (url, key_value) in keys.iter() {
                    if url.contains("localhost") || url.contains("127.0.0.1") {
                        if let Some(key) = key_value.as_str() {
                            return Ok(key.to_string());
                        }
                    }
                }
            }
        }

        // Fall back to legacy format: api_key = "..."
        #[derive(Deserialize)]
        struct LegacyCredentials {
            api_key: Option<String>,
        }

        let legacy: LegacyCredentials =
            toml::from_str(&content).context("Failed to parse credentials file")?;

        if let Some(key) = legacy.api_key {
            return Ok(key);
        }

        anyhow::bail!(
            "No API key found for API URL: {}. Run 'a4 auth login' first.",
            api_url
        )
    }

    /// Load API key for the current configured URL
    #[allow(dead_code)]
    pub fn load_api_key() -> Result<String> {
        let base_url =
            std::env::var("ARETE_API_URL").unwrap_or_else(|_| DEFAULT_API_URL.to_string());
        Self::load_api_key_for_url(&base_url)
    }

    pub fn list_credentials() -> Result<Vec<(String, String)>> {
        let path = Self::credentials_path()?;
        let content = fs::read_to_string(&path).context("Failed to read credentials file")?;

        let creds: toml::Value =
            toml::from_str(&content).context("Failed to parse credentials file")?;

        // Try new format first
        if let Some(keys) = creds.get("keys").and_then(|k| k.as_table()) {
            let mut result = Vec::new();
            for (url, key_value) in keys.iter() {
                if let Some(key) = key_value.as_str() {
                    // Mask the key for display
                    let masked = if key.len() > 12 {
                        format!("{}...{}", &key[..8], &key[key.len() - 4..])
                    } else {
                        key.to_string()
                    };
                    result.push((url.clone(), masked));
                }
            }
            return Ok(result);
        }

        // Fall back to legacy format
        #[derive(Deserialize)]
        struct LegacyCredentials {
            api_key: Option<String>,
        }

        let legacy: LegacyCredentials = toml::from_str(&content)?;
        if let Some(key) = legacy.api_key {
            let masked = if key.len() > 12 {
                format!("{}...{}", &key[..8], &key[key.len() - 4..])
            } else {
                key.to_string()
            };
            return Ok(vec![(DEFAULT_API_URL.to_string(), masked)]);
        }

        Ok(Vec::new())
    }

    pub fn delete_api_key_for_url(api_url: &str) -> Result<()> {
        let path = Self::credentials_path()?;
        if !path.exists() {
            anyhow::bail!("No credentials file found");
        }

        let content = fs::read_to_string(&path)?;
        let mut creds: toml::Value = toml::from_str(&content)?;

        let keys = creds
            .get_mut("keys")
            .and_then(|k| k.as_table_mut())
            .ok_or_else(|| anyhow::anyhow!("No keys found in credentials file"))?;

        if keys.remove(api_url).is_some() {
            let content = toml::to_string_pretty(&creds)?;
            fs::write(&path, content)?;
            Ok(())
        } else {
            anyhow::bail!("No API key found for URL: {}", api_url)
        }
    }

    pub fn delete_all_api_keys() -> Result<()> {
        let path = Self::credentials_path()?;
        if path.exists() {
            fs::remove_file(&path).context("Failed to delete credentials file")?;
        }
        Ok(())
    }
}
