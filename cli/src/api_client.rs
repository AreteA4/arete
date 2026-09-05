use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    #[serde(default)]
    code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StackDestroyStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
}

impl std::fmt::Display for StackDestroyStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Succeeded => write!(f, "succeeded"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackDestroyResponse {
    pub schema: String,
    pub operation_id: String,
    pub spec_id: i32,
    pub status: StackDestroyStatus,
    pub target_count: i64,
    pub pending_targets: i64,
    pub running_targets: i64,
    pub succeeded_targets: i64,
    pub failed_targets: i64,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

impl StackDestroyResponse {
    fn validate(&self, spec_id: i32, operation_id: Option<&str>) -> Result<()> {
        if self.schema != "arete.stack-destroy/v1" {
            anyhow::bail!("API returned an unsupported stack destroy schema");
        }
        if self.spec_id != spec_id {
            anyhow::bail!("API returned a mismatched stack destroy spec identifier");
        }
        let parsed_operation_id = uuid::Uuid::parse_str(&self.operation_id)
            .context("API returned an invalid stack destroy operation identifier")?;
        if let Some(expected) = operation_id {
            let expected = uuid::Uuid::parse_str(expected)
                .context("Invalid stack destroy operation identifier")?;
            if parsed_operation_id != expected {
                anyhow::bail!("API returned a mismatched stack destroy operation identifier");
            }
        }
        let counts = [
            self.target_count,
            self.pending_targets,
            self.running_targets,
            self.succeeded_targets,
            self.failed_targets,
        ];
        if counts.iter().any(|count| *count < 0)
            || self.pending_targets
                + self.running_targets
                + self.succeeded_targets
                + self.failed_targets
                != self.target_count
        {
            anyhow::bail!("API returned inconsistent stack destroy target counts");
        }
        match self.status {
            StackDestroyStatus::Pending | StackDestroyStatus::Running => {
                if self.completed_at.is_some()
                    || self.error_code.is_some()
                    || self.error_message.is_some()
                {
                    anyhow::bail!("API returned an inconsistent active stack destroy");
                }
                if self.status == StackDestroyStatus::Pending && self.started_at.is_some() {
                    anyhow::bail!("API returned a started time for a pending stack destroy");
                }
                if self.status == StackDestroyStatus::Running && self.started_at.is_none() {
                    anyhow::bail!("API omitted the started time for a running stack destroy");
                }
            }
            StackDestroyStatus::Succeeded => {
                if self.completed_at.is_none()
                    || self.pending_targets != 0
                    || self.running_targets != 0
                    || self.failed_targets != 0
                    || self.succeeded_targets != self.target_count
                    || self.error_code.is_some()
                    || self.error_message.is_some()
                {
                    anyhow::bail!("API returned an inconsistent successful stack destroy");
                }
            }
            StackDestroyStatus::Failed => {
                if self.completed_at.is_none()
                    || self.error_code.is_none()
                    || self.failed_targets == 0
                {
                    anyhow::bail!("API returned an incomplete failed stack destroy");
                }
            }
        }
        Ok(())
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateArtifactBuildRequest {
    pub spec_id: i32,
    pub program_specs: Vec<arete_artifacts::ProgramSpecArtifact>,
    pub live_specs: Vec<CreateAliasedLiveSpecArtifact>,
    pub stack_manifest: arete_artifacts::StackManifestArtifactV2,
    pub target_live_alias: String,
    pub deployment_plan_id: String,
    pub selection_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAliasedLiveSpecArtifact {
    pub alias: String,
    pub artifact: arete_artifacts::LiveSpecArtifactV2,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateBuildResponse {
    pub build_id: i32,
    #[allow(dead_code)]
    pub message: String,
    #[serde(default, alias = "deploymentPlanId")]
    pub deployment_plan_id: Option<String>,
    #[serde(default, alias = "selectionDigest")]
    pub selection_digest: Option<String>,
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

pub const STACK_DEPLOYMENT_PLAN_REQUEST_SCHEMA: &str = "arete.stack-deployment-plan-request/v2";
pub const STACK_DEPLOYMENT_PREFLIGHT_SCHEMA: &str = "arete.stack-deployment-preflight/v2";
pub const STACK_DEPLOYMENT_PLAN_SCHEMA: &str = "arete.stack-deployment-plan/v2";

pub const USER_PROGRAM_UPLOAD_SCHEMA: &str = "arete.user-program-upload/v1";
pub const USER_PROGRAM_SCHEMA: &str = "arete.user-program/v1";
pub const USER_PROGRAM_LIST_SCHEMA: &str = "arete.user-program-list/v1";
pub const USER_PROGRAM_EVENTS_SCHEMA: &str = "arete.user-program-events/v1";
pub const USER_PROGRAM_PROMOTION_SCHEMA: &str = "arete.program-promotion-request/v1";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateUserProgramRequest {
    pub schema: String,
    pub idempotency_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub program_spec: arete_artifacts::ProgramSpecArtifact,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateProgramPromotionRequest {
    pub make_idl_public: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserProgramHealth {
    pub status: String,
    #[serde(default)]
    pub assessed_at: Option<String>,
    #[serde(default)]
    pub schema_relevant_attempts: u64,
    #[serde(default)]
    pub schema_failure_rate_basis_points: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserProgramResponse {
    pub schema: String,
    pub user_program_id: String,
    pub program_id: String,
    pub program_spec_hash: String,
    pub alias: Option<String>,
    pub lifecycle_state: String,
    pub admission_state: String,
    pub visibility: String,
    pub program_release_hash: Option<String>,
    pub program_read_binding_id: Option<String>,
    pub operational_status: String,
    pub health: UserProgramHealth,
    pub event_cursor: String,
    #[serde(default)]
    pub diagnostic_codes: Vec<String>,
    #[serde(default)]
    pub idempotent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserProgramListResponse {
    pub schema: String,
    pub items: Vec<UserProgramResponse>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserProgramEvent {
    pub cursor: String,
    pub event_type: String,
    pub occurred_at: String,
    pub state: Option<String>,
    pub diagnostic_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserProgramEventsResponse {
    pub schema: String,
    pub items: Vec<UserProgramEvent>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserProgramPromotionResponse {
    pub schema: String,
    pub promotion_request_id: String,
    pub user_program_id: String,
    pub status: String,
    pub requested_at: String,
    pub idempotent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackDeploymentPreflightRequest {
    pub schema: String,
    pub program_specs: Vec<arete_artifacts::ProgramSpecArtifact>,
    pub live_specs: Vec<CreateAliasedLiveSpecArtifact>,
    pub stack_manifest: arete_artifacts::StackManifestArtifactV2,
    pub branch: Option<String>,
    pub allow_unverified_programs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackDeploymentPlanRequest {
    pub schema: String,
    pub program_specs: Vec<arete_artifacts::ProgramSpecArtifact>,
    pub live_specs: Vec<CreateAliasedLiveSpecArtifact>,
    pub stack_manifest: arete_artifacts::StackManifestArtifactV2,
    pub branch: Option<String>,
    pub allow_unverified_programs: bool,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackDeploymentTarget {
    pub alias: String,
    pub live_spec_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectedProgramRelease {
    pub program_id: String,
    pub program_spec_hash: String,
    pub program_release_hash: String,
    pub release_profile: String,
    pub operational_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentPlanWarning {
    pub code: String,
    pub program_id: String,
    pub program_spec_hash: String,
    pub program_release_hash: String,
    pub operational_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackDeploymentPreflightResponse {
    pub schema: String,
    pub persisted: bool,
    pub stack_manifest_hash: String,
    pub branch: Option<String>,
    pub targets: Vec<StackDeploymentTarget>,
    pub selection_digest: String,
    pub releases: Vec<SelectedProgramRelease>,
    pub warnings: Vec<DeploymentPlanWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackDeploymentPlanResponse {
    pub schema: String,
    pub persisted: bool,
    pub deployment_plan_id: String,
    pub stack_manifest_hash: String,
    pub branch: Option<String>,
    pub targets: Vec<StackDeploymentTarget>,
    pub selection_digest: String,
    pub releases: Vec<SelectedProgramRelease>,
    pub created_at: String,
    pub expires_at: String,
    pub idempotent: bool,
}

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
    pub deployment_plan_id: String,
    pub selection_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindStackCompositionResponse {
    pub composition_id: i64,
    pub stack_manifest_hash: String,
    pub deployment_plan_id: String,
    pub selection_digest: String,
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
    pub operation_id: i32,
    pub status: String,
    pub message: String,
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
#[serde(rename_all = "camelCase")]
pub struct RegistryProgramItem {
    pub install_name: String,
    pub display_name: String,
    pub program_id: String,
    pub program_release_hash: String,
    pub program_spec_hash: String,
    pub sdk_targets: Vec<String>,
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
    /// Target SDK language of the hosted bundle (`"rust"`, `"python"`, or
    /// absent / `"typescript"`). Optional until the registry exposes a
    /// language dimension on sdk_extension_contents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_spec_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_spec: Option<serde_json::Value>,
    #[serde(default)]
    pub live_specs: Vec<RegistryLiveSpecInstallDescriptor>,
    pub stack_manifest_hash: String,
    pub stack_manifest: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_binding: Option<RegistryCapabilityInstallBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_binding: Option<RegistryCapabilityInstallBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_binding: Option<RegistryCapabilityInstallBinding>,
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

    pub(crate) fn has_api_key(&self) -> bool {
        self.api_key.is_some()
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

    pub fn request_stack_destroy(&self, spec_id: i32) -> Result<StackDestroyResponse> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .post(format!("{}/api/specs/{}/destroy", self.base_url, spec_id))
            .bearer_auth(api_key)
            .send()
            .context("Failed to request stack destruction")?;
        let response: StackDestroyResponse = Self::handle_response(response)?;
        response.validate(spec_id, None)?;
        Ok(response)
    }

    pub fn get_stack_destroy_with_timeout(
        &self,
        spec_id: i32,
        operation_id: &str,
        request_timeout: Duration,
    ) -> Result<StackDestroyResponse> {
        let api_key = self.require_api_key()?;
        uuid::Uuid::parse_str(operation_id)
            .context("Invalid stack destroy operation identifier")?;
        let response = self
            .client
            .get(format!(
                "{}/api/specs/{}/destroy/{}",
                self.base_url, spec_id, operation_id
            ))
            .bearer_auth(api_key)
            .timeout(request_timeout)
            .send()
            .context("Failed to inspect stack destruction")?;
        let response: StackDestroyResponse = Self::handle_response(response)?;
        response.validate(spec_id, Some(operation_id))?;
        Ok(response)
    }

    // Spec version endpoints

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

    /// List complete installable programs. Auth expands results to global
    /// visibility, matching the stack registry collection.
    pub fn list_registry_programs(&self) -> Result<Vec<RegistryProgramItem>> {
        let response = self
            .with_optional_auth(
                self.client
                    .get(format!("{}/api/registry/programs", self.base_url)),
            )
            .send()
            .context("Failed to send registry program list request")?;

        Self::handle_response(response)
    }

    // ========================================================================
    // Catalog endpoints (public active set; auth widens to global entries)
    // ========================================================================

    /// Search the active catalog. Raw JSON is returned so `--json` prints
    /// exactly what the platform sent; rendering parses leniently.
    #[allow(clippy::too_many_arguments)]
    pub fn catalog_search(
        &self,
        query: Option<&str>,
        concept: Option<&str>,
        category: Option<&str>,
        kind: Option<&str>,
        mode: Option<&str>,
        target: Option<&str>,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut params: Vec<(&str, String)> = Vec::new();
        for (name, value) in [
            ("q", query),
            ("concept", concept),
            ("category", category),
            ("kind", kind),
            ("mode", mode),
            ("target", target),
            ("cursor", cursor),
        ] {
            if let Some(value) = value {
                params.push((name, value.to_string()));
            }
        }
        if let Some(limit) = limit {
            params.push(("limit", limit.to_string()));
        }
        let response = self
            .with_optional_auth(
                self.client
                    .get(format!("{}/api/registry/v1/catalog/search", self.base_url))
                    .query(&params),
            )
            .send()
            .context("Failed to send catalog search request")?;
        Self::handle_response(response)
    }

    /// One active catalog entry by kind and slug.
    pub fn catalog_entry(&self, kind: &str, slug: &str) -> Result<serde_json::Value> {
        let response = self
            .with_optional_auth(self.client.get(format!(
                "{}/api/registry/v1/catalog/entries/{}/{}",
                self.base_url, kind, slug
            )))
            .send()
            .context("Failed to send catalog entry request")?;
        Self::handle_response(response)
    }

    /// Concept and category vocabularies of the active catalog snapshot.
    pub fn catalog_vocabulary(&self) -> Result<serde_json::Value> {
        let response = self
            .with_optional_auth(self.client.get(format!(
                "{}/api/registry/v1/catalog/vocabulary",
                self.base_url
            )))
            .send()
            .context("Failed to send catalog vocabulary request")?;
        Self::handle_response(response)
    }

    // ========================================================================
    // Knowledge endpoints (API key required on every route)
    // ========================================================================
    //
    // Responses come back as raw `serde_json::Value` rather than typed
    // structs: `--json` must print what the platform sent, and the knowledge
    // payload shapes are additive over time — parsing into closed structs
    // here would silently drop new fields. `commands::know` parses leniently
    // for its readable rendering.

    /// The concept and category vocabularies of the knowledge layer.
    pub fn knowledge_vocabulary(&self) -> Result<serde_json::Value> {
        let api_key = self.require_api_key()?;
        let response = self
            .client
            .get(format!(
                "{}/api/registry/knowledge/vocabulary",
                self.base_url
            ))
            .bearer_auth(api_key)
            .send()
            .context("Failed to send knowledge vocabulary request")?;
        Self::handle_response(response)
    }

    /// Intent search across protocols, programs, stacks, and recipes. The
    /// platform requires at least one of `query`/`concept`/`category`;
    /// `commands::know` validates that before calling.
    pub fn knowledge_search(
        &self,
        query: Option<&str>,
        concept: Option<&str>,
        category: Option<&str>,
        limit: Option<usize>,
    ) -> Result<serde_json::Value> {
        let api_key = self.require_api_key()?;
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(query) = query {
            params.push(("q", query.to_string()));
        }
        if let Some(concept) = concept {
            params.push(("concept", concept.to_string()));
        }
        if let Some(category) = category {
            params.push(("category", category.to_string()));
        }
        if let Some(limit) = limit {
            params.push(("limit", limit.to_string()));
        }
        let response = self
            .client
            .get(format!("{}/api/registry/knowledge/search", self.base_url))
            .query(&params)
            .bearer_auth(api_key)
            .send()
            .context("Failed to send knowledge search request")?;
        Self::handle_response(response)
    }

    /// Curated knowledge for one protocol by slug.
    pub fn knowledge_protocol(&self, slug: &str) -> Result<serde_json::Value> {
        let api_key = self.require_api_key()?;
        let response = self
            .client
            .get(format!(
                "{}/api/registry/knowledge/protocols/{}",
                self.base_url, slug
            ))
            .bearer_auth(api_key)
            .send()
            .context("Failed to send knowledge protocol request")?;
        Self::handle_response(response)
    }

    /// Curated annotations for one program by slug. `section` is validated by
    /// `commands::know`; `None` means the server default (`summary`).
    pub fn knowledge_program(
        &self,
        slug: &str,
        section: Option<&str>,
    ) -> Result<serde_json::Value> {
        let api_key = self.require_api_key()?;
        let mut request = self.client.get(format!(
            "{}/api/registry/knowledge/programs/{}",
            self.base_url, slug
        ));
        if let Some(section) = section {
            request = request.query(&[("section", section)]);
        }
        let response = request
            .bearer_auth(api_key)
            .send()
            .context("Failed to send knowledge program request")?;
        Self::handle_response(response)
    }

    /// One cross-protocol recipe by slug.
    pub fn knowledge_recipe(&self, slug: &str) -> Result<serde_json::Value> {
        let api_key = self.require_api_key()?;
        let response = self
            .client
            .get(format!(
                "{}/api/registry/knowledge/recipes/{}",
                self.base_url, slug
            ))
            .bearer_auth(api_key)
            .send()
            .context("Failed to send knowledge recipe request")?;
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

    /// Get deployment-pinned install data for a hosted stack.
    ///
    /// `language` selects the hosted devex-extension bundle language. The
    /// TypeScript path passes `None`, keeping the request byte-identical to
    /// pre-selector CLIs; Rust generation passes `Some("rust")` and Python
    /// generation passes `Some("python")`.
    pub fn get_registry_stack_install(
        &self,
        stack: &str,
        language: Option<&str>,
    ) -> Result<RegistryStackInstallResponse> {
        let url = registry_install_url(
            &self.base_url,
            &format!("/api/registry/stacks/{}/install", stack),
            language,
        );
        let response = self
            .with_optional_auth(self.client.get(url))
            .send()
            .context("Failed to send registry stack install request")?;

        Self::handle_response(response)
    }

    /// Get canonical install data for a hosted program SDK.
    ///
    /// See [`Self::get_registry_stack_install`] for the `language` contract.
    pub fn get_registry_program_install(
        &self,
        program: &str,
        language: Option<&str>,
    ) -> Result<RegistryProgramInstallResponse> {
        let url = registry_install_url(
            &self.base_url,
            &format!("/api/registry/programs/{}/install", program),
            language,
        );
        let response = self
            .with_optional_auth(self.client.get(url))
            .send()
            .context("Failed to send registry program install request")?;

        Self::handle_response(response)
    }

    /// Resolve a complete project dependency batch against one exact registry snapshot.
    pub fn resolve_registry_dependencies(
        &self,
        request: &crate::project::resolver::RegistryResolveRequest,
    ) -> Result<crate::project::resolver::RegistryResolveResponse> {
        let response = self
            .with_optional_auth(
                self.client
                    .post(format!("{}/api/registry/v1/resolve", self.base_url))
                    .json(request),
            )
            .send()
            .context("Failed to send registry dependency resolver request")?;
        Self::handle_response(response)
    }

    // ========================================================================
    // Build endpoints
    // ========================================================================

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

    /// Validate one complete StackManifest without persisting a deployment plan.
    pub fn preflight_stack_deployment(
        &self,
        req: StackDeploymentPreflightRequest,
    ) -> Result<StackDeploymentPreflightResponse> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .post(format!("{}/api/deployments/plans/preflight", self.base_url))
            .bearer_auth(api_key)
            .json(&req)
            .send()
            .context("Failed to send stack deployment preflight request")?;

        Self::handle_response(response)
    }

    /// Resolve and persist one immutable release selection for a StackManifest.
    pub fn create_stack_deployment_plan(
        &self,
        req: StackDeploymentPlanRequest,
    ) -> Result<StackDeploymentPlanResponse> {
        let api_key = self.require_api_key()?;

        let response = self
            .client
            .post(format!("{}/api/deployments/plans", self.base_url))
            .bearer_auth(api_key)
            .json(&req)
            .send()
            .context("Failed to send stack deployment plan request")?;

        Self::handle_response(response)
    }

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
            .post(format!(
                "{}/api/deployments/{}/stop",
                self.base_url, deployment_id
            ))
            .bearer_auth(api_key)
            .json(&serde_json::json!({
                "reason": "Requested from a4 CLI"
            }))
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

    pub fn create_user_program(
        &self,
        request: &CreateUserProgramRequest,
    ) -> Result<UserProgramResponse> {
        let api_key = self.require_api_key()?;
        let response = self
            .client
            .post(format!("{}/api/programs", self.base_url))
            .bearer_auth(api_key)
            .json(request)
            .send()
            .context("Failed to send ProgramSpec upload request")?;
        Self::handle_response(response)
    }

    pub fn list_user_programs(
        &self,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<UserProgramListResponse> {
        let api_key = self.require_api_key()?;
        let mut request = self
            .client
            .get(format!("{}/api/programs", self.base_url))
            .query(&[("limit", limit.to_string())]);
        if let Some(cursor) = cursor {
            request = request.query(&[("cursor", cursor)]);
        }
        let response = request
            .bearer_auth(api_key)
            .send()
            .context("Failed to list uploaded programs")?;
        Self::handle_response(response)
    }

    pub fn get_user_program(&self, user_program_id: &str) -> Result<UserProgramResponse> {
        let api_key = self.require_api_key()?;
        let response = self
            .client
            .get(format!(
                "{}/api/programs/{}",
                self.base_url, user_program_id
            ))
            .bearer_auth(api_key)
            .send()
            .context("Failed to get uploaded program status")?;
        Self::handle_response(response)
    }

    pub fn list_user_program_events(
        &self,
        user_program_id: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<UserProgramEventsResponse> {
        let api_key = self.require_api_key()?;
        let mut request = self
            .client
            .get(format!(
                "{}/api/programs/{}/events",
                self.base_url, user_program_id
            ))
            .query(&[("limit", limit.to_string())]);
        if let Some(after) = after {
            request = request.query(&[("after", after)]);
        }
        let response = request
            .bearer_auth(api_key)
            .send()
            .context("Failed to list uploaded program events")?;
        Self::handle_response(response)
    }

    pub fn archive_user_program(&self, user_program_id: &str) -> Result<UserProgramResponse> {
        let api_key = self.require_api_key()?;
        let response = self
            .client
            .post(format!(
                "{}/api/programs/{}/archive",
                self.base_url, user_program_id
            ))
            .bearer_auth(api_key)
            .send()
            .context("Failed to archive uploaded program")?;
        Self::handle_response(response)
    }

    pub fn request_user_program_promotion(
        &self,
        user_program_id: &str,
    ) -> Result<UserProgramPromotionResponse> {
        let api_key = self.require_api_key()?;
        let response = self
            .client
            .post(format!(
                "{}/api/programs/{}/promotion-requests",
                self.base_url, user_program_id
            ))
            .bearer_auth(api_key)
            .json(&CreateProgramPromotionRequest {
                make_idl_public: true,
            })
            .send()
            .context("Failed to request uploaded program promotion")?;
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
                .map(|error| match error.code {
                    Some(code) => format!("{} ({code})", error.error),
                    None => error.error,
                })
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
        if let Some(path) = std::env::var_os("ARETE_CREDENTIALS_PATH") {
            let path = PathBuf::from(path);
            if path.as_os_str().is_empty() {
                anyhow::bail!("ARETE_CREDENTIALS_PATH must not be empty");
            }
            return Ok(path);
        }
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

/// Build a registry install URL and request the managed Solana gateway
/// capability contract. Hosted generation must never silently inherit a
/// tenant HTTP endpoint for chain reads or transaction dispatch.
fn registry_install_url(base_url: &str, path: &str, language: Option<&str>) -> String {
    match language {
        Some(language) => {
            format!("{base_url}{path}?language={language}&capabilities=managed-solana-gateway-v1")
        }
        None => format!("{base_url}{path}?capabilities=managed-solana-gateway-v1"),
    }
}

// ============================================================================
// Agent self-registration (WP9: `a4 auth signup`, `a4 doctor`)
// ============================================================================

/// Error message for HTTP 429 from `/api/agents/signup`.
pub const SIGNUP_RATE_LIMIT_MESSAGE: &str = "Signup limit reached (5 per hour per IP). Retry later, or use a key from https://arete.run/keys: a4 auth login --key <a4_ak_...>";

/// Response from `POST /api/agents/signup`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSignupResponse {
    pub slug: String,
    pub display_name: String,
    pub api_key: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Serialize)]
struct AgentSignupRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<&'a str>,
}

impl ApiClient {
    /// Build a client against an explicit base URL with no stored key.
    /// Test-only: production code goes through [`ApiClient::new`].
    #[cfg(test)]
    pub(crate) fn with_base_url(base_url: &str) -> Self {
        ApiClient {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: None,
            client: reqwest::blocking::Client::new(),
        }
    }

    /// Path of the credentials file that `save_api_key` writes
    /// (`ARETE_CREDENTIALS_PATH` or `~/.arete/credentials.toml`).
    pub fn credentials_file_path() -> Result<PathBuf> {
        Self::credentials_path()
    }

    /// Register this machine as an agent (unauthenticated). On HTTP 429 the
    /// error message is exactly [`SIGNUP_RATE_LIMIT_MESSAGE`].
    pub fn agent_signup(&self, display_name: Option<&str>) -> Result<AgentSignupResponse> {
        let response = self
            .client
            .post(format!("{}/api/agents/signup", self.base_url))
            .json(&AgentSignupRequest { display_name })
            .send()
            .context("Failed to reach the signup endpoint")?;
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            anyhow::bail!("{}", SIGNUP_RATE_LIMIT_MESSAGE);
        }
        Self::handle_response(response)
    }

    /// `GET /api/agents/me` with the stored key; the raw JSON is returned so
    /// callers (`a4 doctor`) can report whatever the server includes.
    pub fn agent_me(&self) -> Result<serde_json::Value> {
        let api_key = self.require_api_key()?;
        let response = self
            .client
            .get(format!("{}/api/agents/me", self.base_url))
            .bearer_auth(api_key)
            .send()
            .context("Failed to fetch agent identity")?;
        Self::handle_response(response)
    }
}

/// Minimal canned-response HTTP server for unit tests of `ApiClient` and the
/// commands built on it.
#[cfg(test)]
pub(crate) mod test_support {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// The request the mock server received.
    #[derive(Debug, Clone)]
    pub(crate) struct ReceivedRequest {
        pub(crate) request_line: String,
        pub(crate) headers: Vec<(String, String)>,
        pub(crate) body: String,
    }

    impl ReceivedRequest {
        pub(crate) fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        }
    }

    pub(crate) struct MockServer {
        base_url: String,
        received: mpsc::Receiver<ReceivedRequest>,
    }

    impl MockServer {
        /// Serve exactly one request with `status` and a JSON `body`.
        pub(crate) fn json(status: u16, body: &str) -> Self {
            Self::json_sequence(vec![(status, body.to_string())])
        }

        pub(crate) fn json_delayed(status: u16, body: &str, delay: Duration) -> Self {
            Self::json_sequence_with_delays(vec![(status, body.to_string(), delay)])
        }

        /// Serve one request for each response, in order.
        pub(crate) fn json_sequence(responses: Vec<(u16, String)>) -> Self {
            Self::json_sequence_with_delays(
                responses
                    .into_iter()
                    .map(|(status, body)| (status, body, Duration::ZERO))
                    .collect(),
            )
        }

        fn json_sequence_with_delays(responses: Vec<(u16, String, Duration)>) -> Self {
            assert!(!responses.is_empty(), "mock server needs a response");
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
            let addr = listener.local_addr().expect("mock server addr");
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                for (status, body, delay) in responses {
                    let (mut stream, _) = listener.accept().expect("accept");
                    stream
                        .set_read_timeout(Some(Duration::from_secs(10)))
                        .expect("read timeout");
                    let mut raw = Vec::new();
                    let mut buf = [0u8; 4096];
                    let (head_len, content_length) = loop {
                        let n = stream.read(&mut buf).expect("read request");
                        if n == 0 {
                            break (raw.len(), 0);
                        }
                        raw.extend_from_slice(&buf[..n]);
                        if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&raw[..pos]).to_string();
                            let content_length = head
                                .lines()
                                .find_map(|line| {
                                    let (k, v) = line.split_once(':')?;
                                    k.trim()
                                        .eq_ignore_ascii_case("content-length")
                                        .then(|| v.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                                .unwrap_or(0);
                            break (pos + 4, content_length);
                        }
                    };
                    while raw.len() < head_len + content_length {
                        let n = stream.read(&mut buf).expect("read body");
                        if n == 0 {
                            break;
                        }
                        raw.extend_from_slice(&buf[..n]);
                    }
                    let head = String::from_utf8_lossy(&raw[..head_len]).to_string();
                    let mut lines = head.lines();
                    let request_line = lines.next().unwrap_or_default().to_string();
                    let headers = lines
                        .filter_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            Some((k.trim().to_string(), v.trim().to_string()))
                        })
                        .collect();
                    let body_bytes = &raw[head_len..(head_len + content_length).min(raw.len())];
                    let _ = tx.send(ReceivedRequest {
                        request_line,
                        headers,
                        body: String::from_utf8_lossy(body_bytes).to_string(),
                    });
                    let reason = match status {
                        200 => "OK",
                        201 => "Created",
                        202 => "Accepted",
                        400 => "Bad Request",
                        401 => "Unauthorized",
                        404 => "Not Found",
                        409 => "Conflict",
                        429 => "Too Many Requests",
                        500 => "Internal Server Error",
                        _ => "Status",
                    };
                    let response = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    thread::sleep(delay);
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                }
            });
            MockServer {
                base_url: format!("http://{addr}"),
                received: rx,
            }
        }

        pub(crate) fn base_url(&self) -> &str {
            &self.base_url
        }

        /// The request the server handled (waits up to 10 s).
        pub(crate) fn request(&self) -> ReceivedRequest {
            self.received
                .recv_timeout(Duration::from_secs(10))
                .expect("mock server received a request")
        }
    }
}

#[cfg(test)]
mod agent_tests {
    use super::test_support::MockServer;
    use super::*;

    #[test]
    fn agent_signup_posts_display_name_without_auth_and_parses_response() {
        let server = MockServer::json(
            200,
            r#"{"slug":"agent-7f3a","display_name":"Robo","api_key":"a4_ak_test","message":"welcome"}"#,
        );
        let client = ApiClient::with_base_url(server.base_url());
        let resp = client.agent_signup(Some("Robo")).expect("signup succeeds");
        assert_eq!(resp.slug, "agent-7f3a");
        assert_eq!(resp.display_name, "Robo");
        assert_eq!(resp.api_key, "a4_ak_test");
        assert_eq!(resp.message.as_deref(), Some("welcome"));

        let req = server.request();
        assert_eq!(req.request_line, "POST /api/agents/signup HTTP/1.1");
        assert!(
            req.header("authorization").is_none(),
            "signup is unauthenticated"
        );
        let body: serde_json::Value = serde_json::from_str(&req.body).expect("json body");
        assert_eq!(body, serde_json::json!({"display_name": "Robo"}));
    }

    #[test]
    fn agent_signup_omits_display_name_and_tolerates_missing_message() {
        let server = MockServer::json(
            201,
            r#"{"slug":"agent-1","display_name":"agent-1","api_key":"a4_ak_x"}"#,
        );
        let client = ApiClient::with_base_url(server.base_url());
        let resp = client.agent_signup(None).expect("signup succeeds");
        assert_eq!(resp.message, None);
        let req = server.request();
        let body: serde_json::Value = serde_json::from_str(&req.body).expect("json body");
        assert_eq!(body, serde_json::json!({}));
    }

    #[test]
    fn agent_signup_maps_429_to_rate_limit_message() {
        let server = MockServer::json(429, r#"{"error":"rate limited"}"#);
        let client = ApiClient::with_base_url(server.base_url());
        let err = client.agent_signup(None).expect_err("429 is an error");
        assert_eq!(err.to_string(), SIGNUP_RATE_LIMIT_MESSAGE);
    }

    #[test]
    fn agent_signup_other_errors_use_api_error_format() {
        let server = MockServer::json(500, r#"{"error":"boom"}"#);
        let client = ApiClient::with_base_url(server.base_url());
        let err = client.agent_signup(None).expect_err("500 is an error");
        assert_eq!(
            err.to_string(),
            "API error (500 Internal Server Error): boom"
        );
    }

    #[test]
    fn agent_me_sends_bearer_and_returns_raw_json() {
        let server = MockServer::json(200, r#"{"slug":"agent-1","plan":"free"}"#);
        let client =
            ApiClient::with_base_url(server.base_url()).with_api_key("a4_ak_me".to_string());
        let me = client.agent_me().expect("me succeeds");
        assert_eq!(me["slug"], "agent-1");
        let req = server.request();
        assert_eq!(req.request_line, "GET /api/agents/me HTTP/1.1");
        assert_eq!(req.header("authorization"), Some("Bearer a4_ak_me"));
    }

    #[test]
    fn agent_me_requires_a_key() {
        let client = ApiClient::with_base_url("http://127.0.0.1:1");
        let err = client.agent_me().expect_err("no key");
        assert!(err.to_string().contains("Not authenticated"));
    }
}

#[cfg(test)]
mod stack_destroy_tests {
    use super::test_support::MockServer;
    use super::*;

    const OPERATION_ID: &str = "11111111-1111-4111-8111-111111111111";

    fn response_json(spec_id: i32, operation_id: &str, status: &str) -> String {
        let (pending, running, succeeded, failed, completed_at, error_code, error_message) =
            match status {
                "pending" => (1, 0, 0, 0, None, None, None),
                "running" => (0, 1, 0, 0, None, None, None),
                "succeeded" => (0, 0, 1, 0, Some("2026-09-04T12:00:02Z"), None, None),
                "failed" => (
                    0,
                    0,
                    0,
                    1,
                    Some("2026-09-04T12:00:02Z"),
                    Some("kubernetes-destroy-failed"),
                    Some("one or more targets failed"),
                ),
                other => panic!("unsupported test status {other}"),
            };
        serde_json::json!({
            "schema": "arete.stack-destroy/v1",
            "operationId": operation_id,
            "specId": spec_id,
            "status": status,
            "targetCount": 1,
            "pendingTargets": pending,
            "runningTargets": running,
            "succeededTargets": succeeded,
            "failedTargets": failed,
            "errorCode": error_code,
            "errorMessage": error_message,
            "createdAt": "2026-09-04T12:00:00Z",
            "startedAt": if status == "pending" { None } else { Some("2026-09-04T12:00:01Z") },
            "completedAt": completed_at,
        })
        .to_string()
    }

    fn client(server: &MockServer) -> ApiClient {
        ApiClient::with_base_url(server.base_url()).with_api_key("a4_ak_test".to_string())
    }

    #[test]
    fn request_stack_destroy_posts_once_and_accepts_terminal_success() {
        let server = MockServer::json(200, &response_json(42, OPERATION_ID, "succeeded"));
        let response = client(&server)
            .request_stack_destroy(42)
            .expect("valid response");

        assert_eq!(response.operation_id, OPERATION_ID);
        assert_eq!(response.status, StackDestroyStatus::Succeeded);
        let request = server.request();
        assert_eq!(request.request_line, "POST /api/specs/42/destroy HTTP/1.1");
        assert_eq!(request.header("authorization"), Some("Bearer a4_ak_test"));
    }

    #[test]
    fn get_stack_destroy_uses_spec_and_operation_path() {
        let server = MockServer::json(200, &response_json(42, OPERATION_ID, "running"));
        let response = client(&server)
            .get_stack_destroy_with_timeout(42, OPERATION_ID, Duration::from_secs(1))
            .expect("valid response");

        assert_eq!(response.status, StackDestroyStatus::Running);
        assert_eq!(
            server.request().request_line,
            format!("GET /api/specs/42/destroy/{OPERATION_ID} HTTP/1.1")
        );
    }

    #[test]
    fn get_stack_destroy_honors_per_request_timeout() {
        let server = MockServer::json_delayed(
            200,
            &response_json(42, OPERATION_ID, "running"),
            Duration::from_secs(1),
        );
        let started = std::time::Instant::now();
        let error = client(&server)
            .get_stack_destroy_with_timeout(42, OPERATION_ID, Duration::from_millis(20))
            .expect_err("request times out");

        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(error
            .to_string()
            .contains("Failed to inspect stack destruction"));
        assert_eq!(
            server.request().request_line,
            format!("GET /api/specs/42/destroy/{OPERATION_ID} HTTP/1.1")
        );
    }

    #[test]
    fn stack_destroy_rejects_schema_and_identity_mismatches() {
        let mut wrong_schema: serde_json::Value =
            serde_json::from_str(&response_json(42, OPERATION_ID, "pending")).unwrap();
        wrong_schema["schema"] = serde_json::json!("arete.stack-destroy/v2");
        let server = MockServer::json(200, &wrong_schema.to_string());
        let error = client(&server)
            .request_stack_destroy(42)
            .expect_err("schema mismatch");
        assert!(error
            .to_string()
            .contains("unsupported stack destroy schema"));

        let server = MockServer::json(200, &response_json(7, OPERATION_ID, "pending"));
        let error = client(&server)
            .request_stack_destroy(42)
            .expect_err("spec mismatch");
        assert!(error.to_string().contains("mismatched stack destroy spec"));

        let server = MockServer::json(
            200,
            &response_json(42, "22222222-2222-4222-8222-222222222222", "running"),
        );
        let error = client(&server)
            .get_stack_destroy_with_timeout(42, OPERATION_ID, Duration::from_secs(1))
            .expect_err("operation mismatch");
        assert!(error
            .to_string()
            .contains("mismatched stack destroy operation"));
    }

    #[test]
    fn stack_destroy_rejects_malformed_counts_and_unknown_fields() {
        let mut malformed: serde_json::Value =
            serde_json::from_str(&response_json(42, OPERATION_ID, "pending")).unwrap();
        malformed["targetCount"] = serde_json::json!(2);
        let server = MockServer::json(200, &malformed.to_string());
        let error = client(&server)
            .request_stack_destroy(42)
            .expect_err("count mismatch");
        assert!(error
            .to_string()
            .contains("inconsistent stack destroy target counts"));

        let mut unknown: serde_json::Value =
            serde_json::from_str(&response_json(42, OPERATION_ID, "pending")).unwrap();
        unknown["unversionedField"] = serde_json::json!(true);
        let server = MockServer::json(200, &unknown.to_string());
        let error = client(&server)
            .request_stack_destroy(42)
            .expect_err("unknown response field");
        assert!(error.to_string().contains("Failed to parse response JSON"));
    }

    #[test]
    fn stack_destroy_preserves_machine_error_codes() {
        for (status, code) in [(401, "unauthorized"), (404, "spec-not-found")] {
            let body = serde_json::json!({"error": "request rejected", "code": code}).to_string();
            let server = MockServer::json(status, &body);
            let error = client(&server)
                .request_stack_destroy(42)
                .expect_err("request should fail");
            assert!(error.to_string().contains(code));
        }
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
    fn registry_install_urls_require_managed_gateway_capabilities() {
        assert_eq!(
            registry_install_url(
                "https://api.example.test",
                "/api/registry/stacks/ore/install",
                None
            ),
            "https://api.example.test/api/registry/stacks/ore/install?capabilities=managed-solana-gateway-v1"
        );
        assert_eq!(
            registry_install_url(
                "https://api.example.test",
                "/api/registry/programs/spl-token/install",
                None
            ),
            "https://api.example.test/api/registry/programs/spl-token/install?capabilities=managed-solana-gateway-v1"
        );
        // The Rust rung opts in explicitly.
        assert_eq!(
            registry_install_url(
                "https://api.example.test",
                "/api/registry/stacks/ore/install",
                Some("rust")
            ),
            "https://api.example.test/api/registry/stacks/ore/install?language=rust&capabilities=managed-solana-gateway-v1"
        );
        // ...and the Python rung uses the same selector mechanism.
        assert_eq!(
            registry_install_url(
                "https://api.example.test",
                "/api/registry/stacks/ore/install",
                Some("python")
            ),
            "https://api.example.test/api/registry/stacks/ore/install?language=python&capabilities=managed-solana-gateway-v1"
        );
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
            "liveSpecHash": "live-spec",
            "liveSpec": {"kind": "live-spec"},
            "stackManifestHash": "stack-manifest",
            "stackManifest": {"kind": "stack-manifest"},
            "extensions": null,
            "programs": [descriptor("Program222"), descriptor("Program111")]
        }))
        .expect("stack install should deserialize");

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
            deployment_plan_id: "8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a".into(),
            selection_digest: format!("sha256:{}", "a".repeat(64)),
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
                    "deploymentPlanId": "8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a",
                    "selectionDigest": format!("sha256:{}", "a".repeat(64)),
                    "branch": "preview-contract",
                })
            );
            assert!(value.get("liveSpec").is_none());
            serde_json::from_value::<CreateArtifactBuildRequest>(value).unwrap();
        }
    }

    fn preflight_response_snapshot() -> serde_json::Value {
        json!({
            "schema": "arete.stack-deployment-preflight/v2",
            "persisted": false,
            "stackManifestHash": "arete:h1:stack-manifest:sha256:manifest",
            "branch": null,
            "targets": [
                {"alias": "first", "liveSpecHash": "arete:h1:live-spec:sha256:first"},
                {"alias": "second", "liveSpecHash": "arete:h1:live-spec:sha256:second"},
            ],
            "selectionDigest": format!("sha256:{}", "a".repeat(64)),
            "releases": [{
                "programId": "Program111",
                "programSpecHash": "arete:h1:program-spec:sha256:spec",
                "programReleaseHash": "arete:h1:program-release:sha256:release",
                "releaseProfile": "hosted-managed",
                "operationalStatus": "exact",
            }],
            "warnings": [],
        })
    }

    fn plan_response_snapshot() -> serde_json::Value {
        let preflight = preflight_response_snapshot();
        json!({
            "schema": "arete.stack-deployment-plan/v2",
            "persisted": true,
            "deploymentPlanId": "8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a",
            "stackManifestHash": preflight["stackManifestHash"],
            "branch": preflight["branch"],
            "targets": preflight["targets"],
            "selectionDigest": preflight["selectionDigest"],
            "releases": preflight["releases"],
            "createdAt": "2026-08-10T12:00:00Z",
            "expiresAt": "2026-08-10T12:30:00Z",
            "idempotent": false,
        })
    }

    #[test]
    fn deployment_plan_request_snapshots_are_exact_and_branch_is_explicit() {
        let build = artifact_build_request(2);
        let preflight = StackDeploymentPreflightRequest {
            schema: STACK_DEPLOYMENT_PLAN_REQUEST_SCHEMA.into(),
            program_specs: build.program_specs.clone(),
            live_specs: build.live_specs.clone(),
            stack_manifest: build.stack_manifest.clone(),
            branch: None,
            allow_unverified_programs: false,
        };
        let preflight_value = serde_json::to_value(&preflight).unwrap();
        assert_eq!(
            preflight_value,
            json!({
                "schema": "arete.stack-deployment-plan-request/v2",
                "programSpecs": build.program_specs,
                "liveSpecs": build.live_specs,
                "stackManifest": build.stack_manifest,
                "branch": null,
                "allowUnverifiedPrograms": false,
            })
        );
        assert!(preflight_value.get("idempotencyKey").is_none());

        let plan = StackDeploymentPlanRequest {
            schema: preflight.schema,
            program_specs: preflight.program_specs,
            live_specs: preflight.live_specs,
            stack_manifest: preflight.stack_manifest,
            branch: preflight.branch,
            allow_unverified_programs: preflight.allow_unverified_programs,
            idempotency_key: "8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a".into(),
        };
        let mut expected = preflight_value;
        expected["idempotencyKey"] = json!("8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a");
        assert_eq!(serde_json::to_value(plan).unwrap(), expected);
    }

    #[test]
    fn deployment_plan_response_snapshots_are_exact() {
        let preflight_value = preflight_response_snapshot();
        let preflight: StackDeploymentPreflightResponse =
            serde_json::from_value(preflight_value.clone()).unwrap();
        assert_eq!(serde_json::to_value(preflight).unwrap(), preflight_value);

        let plan_value = plan_response_snapshot();
        let plan: StackDeploymentPlanResponse = serde_json::from_value(plan_value.clone()).unwrap();
        assert_eq!(serde_json::to_value(plan).unwrap(), plan_value);
    }

    #[test]
    fn deployment_selection_responses_reject_private_and_lifecycle_fields() {
        for field in [
            "relationId",
            "relation",
            "programReleaseRelationId",
            "attestationId",
            "attestation",
            "promotionAttestationId",
            "route",
            "routeId",
            "rpc",
            "rpcUrl",
            "fixture",
            "fixtureSetHash",
            "executableIdentity",
        ] {
            let mut preflight = preflight_response_snapshot();
            preflight["releases"][0][field] = json!("private");
            assert!(
                serde_json::from_value::<StackDeploymentPreflightResponse>(preflight).is_err(),
                "accepted private release field {field}"
            );

            let mut plan = plan_response_snapshot();
            plan["releases"][0][field] = json!("private");
            assert!(
                serde_json::from_value::<StackDeploymentPlanResponse>(plan).is_err(),
                "accepted private release field {field}"
            );
        }

        for field in ["deploymentPlanId", "createdAt", "expiresAt"] {
            let mut preflight = preflight_response_snapshot();
            preflight[field] = json!("not-public-in-preflight");
            assert!(
                serde_json::from_value::<StackDeploymentPreflightResponse>(preflight).is_err(),
                "accepted preflight lifecycle field {field}"
            );
        }
    }

    #[test]
    fn composition_bind_request_and_response_snapshots_are_exact() {
        let request = BindStackCompositionRequest {
            stack_manifest_hash: "manifest-hash".into(),
            deployments: BTreeMap::from([("first".into(), 11), ("second".into(), 12)]),
            deployment_plan_id: "8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a".into(),
            selection_digest: format!("sha256:{}", "a".repeat(64)),
            branch: Some("preview-contract".into()),
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({
                "stackManifestHash": "manifest-hash",
                "deployments": {"first": 11, "second": 12},
                "deploymentPlanId": "8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a",
                "selectionDigest": format!("sha256:{}", "a".repeat(64)),
                "branch": "preview-contract",
            })
        );

        let response_value = json!({
            "compositionId": 91,
            "stackManifestHash": "manifest-hash",
            "deploymentPlanId": "8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a",
            "selectionDigest": format!("sha256:{}", "a".repeat(64)),
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
            "deployments": {"live-0": 11},
            "deploymentPlanId": "8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a",
            "selectionDigest": format!("sha256:{}", "a".repeat(64)),
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
