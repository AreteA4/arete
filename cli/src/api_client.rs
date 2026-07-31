use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

fn ensure_no_dangling_symlink(path: &Path) -> Result<()> {
    for candidate in path.ancestors() {
        match fs::symlink_metadata(candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                fs::metadata(candidate).with_context(|| {
                    format!(
                        "Credentials path contains a dangling symlink: {}",
                        candidate.display()
                    )
                })?;
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to inspect credentials path component {}",
                        candidate.display()
                    )
                });
            }
        }
    }
    Ok(())
}

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
    pub portable_ast_hash: Option<String>,
    pub version_created_at: String,
    // AST content info
    pub state_name: String,
    pub program_id: Option<String>,
    pub handler_count: i32,
    pub section_count: i32,
}

impl SpecVersionWithContent {
    pub fn portable_hash(&self) -> &str {
        self.portable_ast_hash.as_deref().unwrap_or("unavailable")
    }

    pub fn short_hash(&self) -> String {
        self.portable_hash()
            .rsplit(':')
            .next()
            .unwrap_or("unavailable")
            .chars()
            .take(12)
            .collect()
    }
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
    #[serde(default)]
    pub portable_ast_hash: Option<String>,
    #[serde(default)]
    pub deployment_release_hash: Option<String>,
    pub status: BuildStatus,
    #[serde(default)]
    pub error_category: Option<String>,
    pub status_message: Option<String>,
    pub phase: Option<String>,
    pub progress: Option<i32>,
    pub websocket_url: Option<String>,
    #[serde(default)]
    pub websocket_auth: Option<serde_json::Value>,
    #[serde(default)]
    pub http_auth: Option<serde_json::Value>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateArtifactBuildRequest {
    pub spec_id: i32,
    pub program_specs: Vec<arete_artifacts::ProgramSpecArtifact>,
    pub live_specs: Vec<CreateAliasedLiveSpecArtifact>,
    pub stack_manifest: arete_artifacts::StackManifestArtifactV2,
    pub target_live_alias: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAliasedLiveSpecArtifact {
    pub alias: String,
    pub artifact: arete_artifacts::LiveSpecArtifactV2,
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
    #[serde(default)]
    pub related_deployment_id: Option<i32>,
    #[serde(default)]
    pub provenance: Option<serde_json::Value>,
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
    pub current_spec_version_id: Option<i32>,
    pub current_version: Option<i32>,
    pub portable_ast_hash: Option<String>,
    pub deployment_release_hash: Option<String>,
    #[serde(default)]
    pub current_idl_program_ids: Vec<String>,
    pub current_image_tag: Option<String>,
    pub websocket_url: String,
    pub http_url: String,
    #[serde(default)]
    pub websocket_auth: serde_json::Value,
    #[serde(default)]
    pub http_auth: serde_json::Value,
    #[serde(default)]
    pub transaction_relay_enabled: bool,
    pub status: DeploymentStatus,
    pub status_message: Option<String>,
    pub first_deployed_at: Option<String>,
    pub last_deployed_at: Option<String>,
    pub live_status: DeploymentLiveStatus,
    #[serde(default)]
    pub latest_operation: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentPhase {
    Missing,
    ScaledDown,
    Running,
    Updating,
    Degraded,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentLiveStatus {
    pub phase: DeploymentPhase,
    pub desired_replicas: Option<i32>,
    pub ready_replicas: Option<i32>,
    pub available_replicas: Option<i32>,
    pub updated_replicas: Option<i32>,
    pub last_transition_time: Option<String>,
    pub source: String,
    pub error_category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindStackCompositionRequest {
    pub stack_manifest_hash: String,
    pub deployments: BTreeMap<String, i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindStackCompositionResponse {
    pub composition_id: i64,
    pub stack_manifest_hash: String,
    pub branch: Option<String>,
    pub live_specs: Vec<CompositionLiveBindingResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompositionLiveBindingResponse {
    pub alias: String,
    pub live_spec_hash: String,
    pub deployment_id: i32,
    pub websocket_endpoint: String,
    pub query_endpoint: String,
    pub websocket_auth_policy: String,
    pub query_auth_policy: String,
    pub observed_generation: i64,
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

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryAstResponse {
    pub name: String,
    pub stack: String,
    pub websocket_url: String,
    pub http_url: String,
    pub websocket_auth: serde_json::Value,
    pub http_auth: serde_json::Value,
    pub description: Option<String>,
    pub visibility: String,
    pub ast_payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegistrySdkExtensionInputKind {
    StackAst,
    StackManifest,
    ProgramIdl,
    ProgramSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistrySdkExtensionManifest {
    pub entry: String,
    pub files: Vec<String>,
    pub input_kind: Option<RegistrySdkExtensionInputKind>,
    pub input_hash: Option<String>,
    pub sdk_range: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistrySdkExtensionArtifact {
    pub artifact_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_extension_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_output_tree_hash: Option<String>,
    pub manifest: RegistrySdkExtensionManifest,
    pub files: BTreeMap<String, String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryStackInstallResponse {
    pub name: String,
    pub stack: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_auth: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_auth: Option<serde_json::Value>,
    pub description: Option<String>,
    pub visibility: String,
    pub spec_version_id: Option<i32>,
    pub ast_content_hash: String,
    pub portable_ast_hash: String,
    pub ast_payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_spec_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_spec: Option<serde_json::Value>,
    #[serde(default)]
    pub live_specs: Vec<RegistryLiveSpecInstallDescriptor>,
    pub stack_manifest_hash: String,
    pub stack_manifest: serde_json::Value,
    #[serde(default)]
    pub chain_binding: Option<RegistryCapabilityInstallBinding>,
    #[serde(default)]
    pub transaction_binding: Option<RegistryCapabilityInstallBinding>,
    pub extensions: Option<RegistrySdkExtensionArtifact>,
    pub programs: Vec<RegistryProgramInstallResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryLiveSpecInstallDescriptor {
    pub alias: String,
    pub live_spec_hash: String,
    pub artifact: serde_json::Value,
    pub binding: RegistryLiveSpecInstallBinding,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryLiveSpecInstallBinding {
    pub deployment_id: i32,
    pub websocket_endpoint: String,
    pub query_endpoint: String,
    pub websocket_auth_policy: String,
    pub query_auth_policy: String,
    pub observed_generation: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryCapabilityInstallBinding {
    pub endpoint: String,
    pub auth_policy: String,
    pub solana_gateway_binding_id: String,
    pub cluster: String,
    pub region: String,
    pub auth: RegistrySolanaGatewayAuthMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistrySolanaGatewayAuthMetadata {
    pub required: bool,
    pub mode: String,
    pub session_endpoint: String,
    pub jwks_url: String,
    pub token_transport: String,
    pub audience: String,
    pub target_kind: String,
    pub target_id: String,
    pub scopes: Vec<String>,
    pub accepted_key_classes: Vec<String>,
    pub transaction_entitlement_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryProgramInstallResponse {
    pub install_name: String,
    pub display_name: String,
    pub definition: RegistryProgramInstallDefinition,
    pub release: RegistryProgramInstallRelease,
    pub transport: RegistryProgramInstallTransport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum RegistryProgramInstallTransport {
    HostedBinding {
        binding: RegistryProgramInstallBinding,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryProgramInstallDefinition {
    pub program_id: String,
    pub program_spec_hash: String,
    pub idl_content_hash: String,
    pub normalized_idl_hash: String,
    pub idl_payload: serde_json::Value,
    pub program_spec: serde_json::Value,
    pub extensions: Option<RegistrySdkExtensionArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryProgramInstallRelease {
    pub program_release_hash: String,
    pub program_spec_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryProgramInstallBinding {
    pub endpoint: String,
    pub program_read_binding_id: String,
    pub auth: serde_json::Value,
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
    #[allow(dead_code)]
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

    /// Get deployment-pinned install data for a hosted stack.
    pub fn get_registry_stack_install(&self, stack: &str) -> Result<RegistryStackInstallResponse> {
        let response = self
            .with_optional_auth(self.client.get(format!(
                "{}/api/registry/stacks/{}/install",
                self.base_url, stack
            )))
            .send()
            .context("Failed to send registry stack install request")?;

        Self::handle_response(response)
    }

    /// Get canonical install data for a hosted program SDK.
    pub fn get_registry_program_install(
        &self,
        program: &str,
    ) -> Result<RegistryProgramInstallResponse> {
        let response = self
            .with_optional_auth(self.client.get(format!(
                "{}/api/registry/programs/{}/install",
                self.base_url, program
            )))
            .send()
            .context("Failed to send registry program install request")?;

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

    /// Create a build from explicit public artifacts.
    pub fn create_artifact_build(
        &self,
        req: CreateArtifactBuildRequest,
    ) -> Result<CreateBuildResponse> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .post(format!("{}/api/builds/artifacts", self.base_url))
            .bearer_auth(api_key)
            .json(&req)
            .send()
            .context("Failed to send artifact build request")?;

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
        self.list_deployments_page(limit, 0)
    }

    pub fn list_deployments_page(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<DeploymentResponse>> {
        let api_key = self.require_api_key()?;

        let url = format!(
            "{}/api/deployments?limit={}&offset={}",
            self.base_url, limit, offset
        );

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

    /// Atomically bind the exact healthy child deployments for a StackManifest.
    pub fn bind_stack_composition(
        &self,
        req: BindStackCompositionRequest,
    ) -> Result<BindStackCompositionResponse> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .post(format!("{}/api/deployments/compositions", self.base_url))
            .bearer_auth(api_key)
            .json(&req)
            .send()
            .context("Failed to send composition bind request")?;

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
            let body = response.text().unwrap_or_default();
            let message = serde_json::from_str::<ErrorResponse>(&body)
                .map(|error| error.error)
                .unwrap_or_else(|_| {
                    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
                    if compact.is_empty() {
                        "Empty error response".to_string()
                    } else {
                        compact.chars().take(1024).collect()
                    }
                });
            anyhow::bail!("API error ({}): {}", status, message);
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

    fn parse_api_key(content: &str, api_url: &str) -> Result<Option<String>> {
        let creds: toml::Value =
            toml::from_str(content).context("Failed to parse credentials file")?;

        // Try new format first: [keys] table with URL mapping
        if let Some(keys) = creds.get("keys").and_then(|k| k.as_table()) {
            // Look for exact match first
            if let Some(key) = keys.get(api_url).and_then(|v| v.as_str()) {
                return Ok(Some(key.to_string()));
            }

            // For localhost URLs, try to match any localhost URL
            if api_url.contains("localhost") || api_url.contains("127.0.0.1") {
                for (url, key_value) in keys.iter() {
                    if url.contains("localhost") || url.contains("127.0.0.1") {
                        if let Some(key) = key_value.as_str() {
                            return Ok(Some(key.to_string()));
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
            toml::from_str(content).context("Failed to parse credentials file")?;

        if let Some(key) = legacy.api_key {
            return Ok(Some(key));
        }

        Ok(None)
    }

    /// Load API key for a specific URL (new URL-based format)
    pub fn load_api_key_for_url(api_url: &str) -> Result<String> {
        Self::load_optional_api_key_for_url(api_url)?.ok_or_else(|| {
            anyhow::anyhow!(
                "No API key found for API URL: {}. Run 'a4 auth login' first.",
                api_url
            )
        })
    }

    /// Load an API key when credentials are genuinely absent.
    ///
    /// Broken credential paths remain errors instead of silently becoming
    /// anonymous access.
    pub fn load_optional_api_key_for_url(api_url: &str) -> Result<Option<String>> {
        let path = Self::credentials_path()?;
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                ensure_no_dangling_symlink(&path)?;
                return Ok(None);
            }
            Err(error) => {
                return Err(error).context("Failed to read credentials file");
            }
        };
        Self::parse_api_key(&content, api_url)
    }

    /// Load API key for the current configured URL
    #[allow(dead_code)]
    pub fn load_api_key() -> Result<String> {
        let base_url =
            std::env::var("ARETE_API_URL").unwrap_or_else(|_| DEFAULT_API_URL.to_string());
        Self::load_api_key_for_url(&base_url)
    }

    /// Load an optional API key for the current configured URL.
    pub fn load_optional_api_key() -> Result<Option<String>> {
        let base_url =
            std::env::var("ARETE_API_URL").unwrap_or_else(|_| DEFAULT_API_URL.to_string());
        Self::load_optional_api_key_for_url(&base_url)
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

#[cfg(all(test, unix))]
mod tests {
    use super::ensure_no_dangling_symlink;
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn dangling_credentials_symlink_is_not_treated_as_missing() {
        let root =
            std::env::temp_dir().join(format!("a4-dangling-credentials-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let credentials = root.join("credentials.toml");
        symlink(root.join("missing.toml"), &credentials).unwrap();

        let error = ensure_no_dangling_symlink(&credentials).unwrap_err();

        assert!(error.to_string().contains("dangling symlink"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dangling_parent_symlink_is_not_treated_as_missing() {
        let root =
            std::env::temp_dir().join(format!("a4-dangling-credentials-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let credentials_dir = root.join(".arete");
        symlink(root.join("missing-dir"), &credentials_dir).unwrap();

        let error =
            ensure_no_dangling_symlink(&credentials_dir.join("credentials.toml")).unwrap_err();

        assert!(error.to_string().contains("dangling symlink"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sdk_extension_artifact_deserializes_typed_hashes() {
        let artifact: RegistrySdkExtensionArtifact = serde_json::from_value(json!({
            "artifactHash": "legacy-extension-sha256",
            "sdkExtensionHash": "arete:h1:sdk-extension:sha256:typed-extension",
            "sdkOutputTreeHash": "arete:h1:sdk-output-tree:sha256:typed-tree",
            "manifest": {
                "entry": "index.ts",
                "files": ["index.ts"],
                "inputKind": null,
                "inputHash": null,
                "sdkRange": null
            },
            "files": {"index.ts": "export {};"},
            "createdAt": "2026-07-28T00:00:00Z"
        }))
        .expect("typed extension hashes should deserialize");

        assert_eq!(
            artifact.sdk_extension_hash.as_deref(),
            Some("arete:h1:sdk-extension:sha256:typed-extension")
        );
        assert_eq!(
            artifact.sdk_output_tree_hash.as_deref(),
            Some("arete:h1:sdk-output-tree:sha256:typed-tree")
        );
    }

    #[test]
    fn nested_program_install_descriptor_deserializes_exact_platform_shape() {
        let value = json!({
            "installName": "program-two",
            "displayName": "Program Two",
            "definition": {
                "programId": "Program222",
                "programSpecHash": "arete:h1:program-spec:sha256:spec-two",
                "idlContentHash": "arete:h1:idl-content:sha256:content-two",
                "normalizedIdlHash": "arete:h1:idl-normalized:sha256:normalized-two",
                "idlPayload": {"name": "program_two"},
                "programSpec": {
                    "artifactVersion": "1.0.0",
                    "kind": "program-spec",
                    "artifactHash": "arete:h1:program-spec:sha256:spec-two",
                    "payload": {"programId": "Program222"}
                },
                "extensions": null
            },
            "release": {
                "programReleaseHash": "arete:h1:program-release:sha256:hosted-two",
                "programSpecHash": "arete:h1:program-spec:sha256:spec-two"
            },
            "transport": {
                "kind": "hosted-binding",
                "binding": {
                    "endpoint": "https://reads.example.test/exact/prefix/",
                    "programReadBindingId": "prb_00000000000000000000000000000002",
                    "auth": {
                        "required": true,
                        "mode": "signed_session",
                        "sessionEndpoint": "https://api.example.test/exact/ws/sessions",
                        "targetKind": "program-read-binding",
                        "targetId": "prb_00000000000000000000000000000002"
                    }
                }
            }
        });

        let descriptor: RegistryProgramInstallResponse =
            serde_json::from_value(value.clone()).expect("nested descriptor should deserialize");

        assert_eq!(descriptor.install_name, "program-two");
        assert_eq!(descriptor.definition.program_id, "Program222");
        assert_eq!(
            descriptor.release.program_release_hash,
            "arete:h1:program-release:sha256:hosted-two"
        );
        let RegistryProgramInstallTransport::HostedBinding { binding } = &descriptor.transport;
        assert_eq!(binding.endpoint, "https://reads.example.test/exact/prefix/");
        assert_eq!(binding.auth["mode"], "signed_session");
        assert_eq!(serde_json::to_value(descriptor).unwrap(), value);
    }

    #[test]
    fn stack_install_preserves_portable_hash_and_program_order() {
        let descriptor = |program_id: &str| {
            let binding_id = format!("prb_{program_id:0>32}");
            json!({
                "installName": program_id,
                "displayName": program_id,
                "definition": {
                    "programId": program_id,
                    "programSpecHash": format!("spec-{program_id}"),
                    "idlContentHash": format!("content-{program_id}"),
                    "normalizedIdlHash": format!("normalized-{program_id}"),
                    "idlPayload": {},
                    "programSpec": {
                        "artifactVersion": "1.0.0",
                        "kind": "program-spec",
                        "artifactHash": format!("spec-{program_id}"),
                        "payload": {"programId": program_id}
                    },
                    "extensions": null
                },
                "release": {
                    "programReleaseHash": format!("release-{program_id}"),
                    "programSpecHash": format!("spec-{program_id}")
                },
                "transport": {
                    "kind": "hosted-binding",
                    "binding": {
                        "endpoint": format!("https://reads.example.test/{program_id}/"),
                        "programReadBindingId": binding_id.clone(),
                        "auth": {
                            "program": program_id,
                            "sessionEndpoint": "https://auth.example.test/session",
                            "targetKind": "program-read-binding",
                            "targetId": binding_id
                        }
                    }
                }
            })
        };
        let response: RegistryStackInstallResponse = serde_json::from_value(json!({
            "name": "ordered",
            "stack": "ordered-stack",
            "websocketUrl": "wss://stack.example.test/exact/ws",
            "httpUrl": "https://stack.example.test/exact/http",
            "websocketAuth": {},
            "httpAuth": {},
            "description": null,
            "visibility": "public",
            "specVersionId": 7,
            "astContentHash": "ast-content",
            "portableAstHash": "portable-ast",
            "astPayload": {},
            "liveSpecHash": "live-spec",
            "liveSpec": {"kind": "live-spec"},
            "stackManifestHash": "stack-manifest",
            "stackManifest": {"kind": "stack-manifest"},
            "extensions": null,
            "programs": [descriptor("Program222"), descriptor("Program111")]
        }))
        .expect("stack install should deserialize");

        assert_eq!(response.portable_ast_hash, "portable-ast");
        assert_eq!(response.programs[0].definition.program_id, "Program222");
        assert_eq!(response.programs[1].definition.program_id, "Program111");
    }

    fn artifact_build_request(live_count: usize) -> CreateArtifactBuildRequest {
        let live = arete_artifacts::LiveSpecArtifactV2::new(arete_artifacts::LiveSpecV2::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ))
        .unwrap();
        let live_specs = (0..live_count)
            .map(|index| CreateAliasedLiveSpecArtifact {
                alias: format!("live-{index}"),
                artifact: live.clone(),
            })
            .collect::<Vec<_>>();
        let stack_manifest = arete_artifacts::compose_stack_manifest_v2(
            "Snapshot",
            &[],
            live_specs
                .iter()
                .map(|live| (live.alias.clone(), &live.artifact))
                .collect(),
            Vec::new(),
        )
        .unwrap();
        CreateArtifactBuildRequest {
            spec_id: 41,
            program_specs: Vec::new(),
            live_specs,
            stack_manifest,
            target_live_alias: format!("live-{}", live_count - 1),
            branch: Some("preview-contract".into()),
        }
    }

    #[test]
    fn artifact_build_collection_request_snapshot_is_canonical() {
        for live_count in [1, 2, 3] {
            let request = artifact_build_request(live_count);
            let value = serde_json::to_value(&request).unwrap();
            let expected_lives = request
                .live_specs
                .iter()
                .map(|live| {
                    json!({
                        "alias": live.alias,
                        "artifact": live.artifact,
                    })
                })
                .collect::<Vec<_>>();
            assert_eq!(
                value,
                json!({
                    "specId": 41,
                    "programSpecs": [],
                    "liveSpecs": expected_lives,
                    "stackManifest": request.stack_manifest,
                    "targetLiveAlias": format!("live-{}", live_count - 1),
                    "branch": "preview-contract",
                })
            );
            assert!(value.get("liveSpec").is_none());
            serde_json::from_value::<CreateArtifactBuildRequest>(value).unwrap();
        }
    }

    #[test]
    fn composition_bind_request_and_response_snapshots_are_exact() {
        let request = BindStackCompositionRequest {
            stack_manifest_hash: "manifest-hash".into(),
            deployments: BTreeMap::from([("first".into(), 11), ("second".into(), 12)]),
            branch: Some("preview-contract".into()),
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({
                "stackManifestHash": "manifest-hash",
                "deployments": {"first": 11, "second": 12},
                "branch": "preview-contract",
            })
        );

        let response_value = json!({
            "compositionId": 91,
            "stackManifestHash": "manifest-hash",
            "branch": "preview-contract",
            "liveSpecs": [{
                "alias": "first",
                "liveSpecHash": "live-hash",
                "deploymentId": 11,
                "websocketEndpoint": "wss://first.example.test",
                "queryEndpoint": "https://first.example.test",
                "websocketAuthPolicy": "signed_session",
                "queryAuthPolicy": "signed_session",
                "observedGeneration": 4,
            }],
        });
        let response: BindStackCompositionResponse =
            serde_json::from_value(response_value.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), response_value);
    }

    fn registry_install_snapshot(live_count: usize) -> serde_json::Value {
        let live_specs = (0..live_count)
            .map(|index| {
                json!({
                    "alias": format!("live-{index}"),
                    "liveSpecHash": format!("live-hash-{index}"),
                    "artifact": {"kind": "live-spec", "index": index},
                    "binding": {
                        "deploymentId": 100 + index,
                        "websocketEndpoint": format!("wss://live-{index}.example.test"),
                        "queryEndpoint": format!("https://live-{index}.example.test"),
                        "websocketAuthPolicy": "signed_session",
                        "queryAuthPolicy": "signed_session",
                        "observedGeneration": 7,
                    },
                })
            })
            .collect::<Vec<_>>();
        let gateway_id = "sgb_00000000000000000000000000000001";
        let gateway_auth = |scopes: Vec<&str>, accepted_key_classes: Vec<&str>, entitlement| {
            json!({
                "required": true,
                "mode": "signed_session",
                "sessionEndpoint": "https://api.example.test/ws/sessions",
                "jwksUrl": "https://api.example.test/.well-known/jwks.json",
                "tokenTransport": "bearer",
                "audience": "arete:solana-gateway",
                "targetKind": "solana-gateway-binding",
                "targetId": gateway_id,
                "scopes": scopes,
                "acceptedKeyClasses": accepted_key_classes,
                "transactionEntitlementRequired": entitlement,
            })
        };
        let mut value = json!({
            "name": "Snapshot",
            "stack": "snapshot-stack",
            "description": null,
            "visibility": "public",
            "specVersionId": 5,
            "astContentHash": "ast-content",
            "portableAstHash": "portable-ast",
            "astPayload": {},
            "liveSpecs": live_specs,
            "stackManifestHash": "manifest-hash",
            "stackManifest": {"kind": "stack-manifest"},
            "chainBinding": {
                "endpoint": "https://solana.example.test/gateway/",
                "authPolicy": "signed_session",
                "solanaGatewayBindingId": gateway_id,
                "cluster": "mainnet-beta",
                "region": "us-west-1",
                "auth": gateway_auth(
                    vec!["read"],
                    vec!["anonymous", "publishable", "secret"],
                    false,
                ),
            },
            "transactionBinding": {
                "endpoint": "https://solana.example.test/gateway/",
                "authPolicy": "signed_session",
                "solanaGatewayBindingId": gateway_id,
                "cluster": "mainnet-beta",
                "region": "us-west-1",
                "auth": gateway_auth(
                    vec!["transaction:inspect", "transaction:send"],
                    vec!["publishable", "secret"],
                    true,
                ),
            },
            "extensions": null,
            "programs": [],
        });
        if live_count == 1 {
            value["websocketUrl"] = json!("wss://live-0.example.test");
            value["httpUrl"] = json!("https://live-0.example.test");
            value["websocketAuth"] = json!({"mode": "signed_session"});
            value["httpAuth"] = json!({"mode": "signed_session"});
            value["liveSpecHash"] = json!("live-hash-0");
            value["liveSpec"] = json!({"kind": "live-spec", "index": 0});
        }
        value
    }

    #[test]
    fn one_two_and_three_live_registry_response_snapshots_are_exact() {
        for live_count in [1, 2, 3] {
            let value = registry_install_snapshot(live_count);
            let response: RegistryStackInstallResponse =
                serde_json::from_value(value.clone()).unwrap();
            assert_eq!(response.live_specs.len(), live_count);
            assert_eq!(
                response.chain_binding.as_ref().unwrap().auth.target_kind,
                "solana-gateway-binding"
            );
            assert!(
                response
                    .transaction_binding
                    .as_ref()
                    .unwrap()
                    .auth
                    .transaction_entitlement_required
            );
            assert_eq!(serde_json::to_value(response).unwrap(), value);
        }
    }

    #[test]
    fn singular_registry_response_without_live_specs_remains_compatible() {
        let mut value = registry_install_snapshot(1);
        value.as_object_mut().unwrap().remove("liveSpecs");
        value.as_object_mut().unwrap().remove("chainBinding");
        value.as_object_mut().unwrap().remove("transactionBinding");
        let response: RegistryStackInstallResponse = serde_json::from_value(value).unwrap();
        assert!(response.live_specs.is_empty());
        assert_eq!(response.live_spec_hash.as_deref(), Some("live-hash-0"));
    }

    #[test]
    fn public_contract_dtos_reject_private_or_unknown_fields() {
        let mut build = serde_json::to_value(artifact_build_request(1)).unwrap();
        build["runtimeArtifactHash"] = json!("private");
        assert!(serde_json::from_value::<CreateArtifactBuildRequest>(build).is_err());

        let mut bind = json!({
            "stackManifestHash": "manifest-hash",
            "deployments": {"live-0": 11}
        });
        bind["authSecret"] = json!("private");
        assert!(serde_json::from_value::<BindStackCompositionRequest>(bind).is_err());

        let mut install = registry_install_snapshot(2);
        install["runtimeArtifact"] = json!({"private": true});
        assert!(serde_json::from_value::<RegistryStackInstallResponse>(install).is_err());

        let mut nested = registry_install_snapshot(2);
        nested["liveSpecs"][0]["decoderBinding"] = json!({"private": true});
        assert!(serde_json::from_value::<RegistryStackInstallResponse>(nested).is_err());

        let mut gateway = registry_install_snapshot(2);
        gateway["chainBinding"]["auth"]["privateSigningKey"] = json!("private");
        assert!(serde_json::from_value::<RegistryStackInstallResponse>(gateway).is_err());
    }
}
