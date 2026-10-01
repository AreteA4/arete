use anyhow::{Context, Result};
use arete_mcp::credentials::{
    inferred_profile_for_key, lookup_credentials, normalize_api_url, validate_key_for_profile,
    validate_profile_name, ENV_VAR_API_KEY, ENV_VAR_CREDENTIALS_PATH, ENV_VAR_PROFILE,
};
use arete_sdk::{ApiProblemV1, ReadyRecoveryActionV1, RecoveryAction};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn ensure_safe_credentials_path(path: &Path) -> Result<()> {
    // The credential file and its dedicated directory must not be symlinks.
    // Do not reject symlinks in filesystem-owned ancestors: on macOS, for
    // example, `/var` and `/tmp` intentionally resolve through `/private`.
    for candidate in std::iter::once(path).chain(path.parent()) {
        match fs::symlink_metadata(candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::bail!(
                    "Credentials path must not contain symlinks: {}",
                    candidate.display()
                );
            }
            Ok(metadata) if candidate == path && !metadata.is_file() => {
                anyhow::bail!(
                    "Credentials target must be a regular file: {}",
                    candidate.display()
                );
            }
            Ok(metadata) if candidate != path && !metadata.is_dir() => {
                anyhow::bail!(
                    "Credentials path parent must be a directory: {}",
                    candidate.display()
                );
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

fn ensure_owner_only_directory(path: &Path, created: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if created {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).with_context(|| {
                format!("Failed to protect credentials directory {}", path.display())
            })?;
        }
        let mode = fs::metadata(path)?.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            anyhow::bail!(
                "Credentials directory permissions are too broad ({mode:o}); set {} to mode 700",
                path.display()
            );
        }
    }

    #[cfg(not(unix))]
    let _ = (path, created);

    Ok(())
}

fn write_credentials_atomic(path: &Path, content: &[u8]) -> Result<()> {
    ensure_safe_credentials_path(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Credentials path must have a parent directory"))?;
    let parent_existed = parent.exists();
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "Failed to create credentials directory {}",
            parent.display()
        )
    })?;
    ensure_safe_credentials_path(path)?;
    ensure_owner_only_directory(parent, !parent_existed)?;

    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("Credentials filename must be valid UTF-8"))?;
    let temporary = parent.join(format!(".{filename}.{}.tmp", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let result = (|| -> Result<()> {
        let mut file = options.open(&temporary).with_context(|| {
            format!(
                "Failed to create temporary credentials file {}",
                temporary.display()
            )
        })?;
        file.write_all(content)
            .context("Failed to write temporary credentials file")?;
        file.flush()
            .context("Failed to flush temporary credentials file")?;
        file.sync_all()
            .context("Failed to sync temporary credentials file")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        drop(file);
        fs::rename(&temporary, path)
            .with_context(|| format!("Failed to atomically replace {}", path.display()))?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .with_context(|| {
                format!("Failed to sync credentials directory {}", parent.display())
            })?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_credentials_value(path: &Path) -> Result<toml::Value> {
    match fs::read_to_string(path) {
        Ok(content) if content.is_empty() => Ok(toml::Value::Table(toml::map::Map::new())),
        Ok(content) => toml::from_str(&content)
            .context("Existing credentials file is malformed; refusing to replace it"),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            Ok(toml::Value::Table(toml::map::Map::new()))
        }
        Err(error) => Err(error).context("Failed to read existing credentials file"),
    }
}

fn profile_table_mut<'a>(
    credentials: &'a mut toml::Value,
    profile: &str,
) -> Result<&'a mut toml::map::Map<String, toml::Value>> {
    credentials
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("Invalid credentials format"))?
        .entry("profiles")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("Invalid profiles format"))?
        .entry(profile)
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("Invalid profile `{profile}` format"))
}

fn url_value<'a>(
    values: &'a toml::map::Map<String, toml::Value>,
    api_url: &str,
) -> Option<&'a toml::Value> {
    if let Some(value) = values.get(api_url) {
        return Some(value);
    }
    let wanted = normalize_api_url(api_url);
    if let Some(value) = values.get(&wanted) {
        return Some(value);
    }
    values
        .iter()
        .filter(|(url, _)| normalize_api_url(url) == wanted)
        .min_by_key(|(url, _)| *url)
        .map(|(_, value)| value)
}

/// Production API URL (used by default in release builds)
#[cfg(not(feature = "local"))]
const DEFAULT_API_URL: &str = "https://api.arete.run";
const DEFAULT_APP_ORIGIN: &str = "https://app.arete.run";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCredential {
    pub profile: Option<String>,
    pub api_url: String,
    pub masked_key: String,
}

/// A process-scoped exclusive lock for credentials mutations. The lock file
/// remains on disk, but the kernel releases the lock if the process exits.
pub struct CredentialsLock {
    file: fs::File,
}

impl Drop for CredentialsLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

fn mask_api_key(api_key: &str) -> String {
    if api_key.len() <= 12 {
        return "****".to_string();
    }
    format!("{}...{}", &api_key[..8], &api_key[api_key.len() - 4..])
}

fn remove_url_key(keys: &mut toml::map::Map<String, toml::Value>, api_url: &str) -> bool {
    take_url_value(keys, api_url).is_some()
}

fn take_url_value(
    values: &mut toml::map::Map<String, toml::Value>,
    api_url: &str,
) -> Option<toml::Value> {
    if let Some(value) = values.remove(api_url) {
        return Some(value);
    }
    let wanted = normalize_api_url(api_url);
    if wanted != api_url {
        if let Some(value) = values.remove(&wanted) {
            return Some(value);
        }
    }
    let mut candidates = values
        .keys()
        .filter(|url| normalize_api_url(url) == wanted)
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
        .first()
        .and_then(|candidate| values.remove(candidate))
}

fn take_equivalent_url_values(
    values: &mut toml::map::Map<String, toml::Value>,
    api_url: &str,
) -> Vec<(String, toml::Value)> {
    let wanted = normalize_api_url(api_url);
    let mut urls = values
        .keys()
        .filter(|url| normalize_api_url(url) == wanted)
        .cloned()
        .collect::<Vec<_>>();
    urls.sort();
    urls.into_iter()
        .filter_map(|url| values.remove(&url).map(|value| (url, value)))
        .collect()
}

#[derive(Clone, PartialEq, Eq)]
pub struct PendingAgentSignup {
    pub credential: String,
    pub idempotency_key: String,
}

impl std::fmt::Debug for PendingAgentSignup {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingAgentSignup")
            .field("credential", &"[REDACTED]")
            .field("idempotency_key", &"[REDACTED]")
            .finish()
    }
}

fn validate_signup_idempotency_key(value: &str) -> Result<()> {
    let parsed = uuid::Uuid::parse_str(value)
        .context("Pending agent signup idempotency key is malformed")?;
    if parsed.get_version_num() != 4 || parsed.to_string() != value {
        anyhow::bail!("Pending agent signup idempotency key is malformed");
    }
    Ok(())
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

/// A non-success API response with its status, message, and stable error
/// code used by synthetic errors in command tests and compatibility adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiHttpError {
    pub status: u16,
    pub status_text: String,
    pub message: String,
    pub code: Option<String>,
    /// The command a refusal suggests running instead, when it names one
    /// (for example the version that replaces a retired stack version).
    pub upgrade_command: Option<String>,
}

impl std::fmt::Display for ApiHttpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.code {
            Some(code) => write!(
                formatter,
                "API error ({}): {} ({code})",
                self.status_text, self.message
            ),
            None => write!(
                formatter,
                "API error ({}): {}",
                self.status_text, self.message
            ),
        }
    }
}

impl std::error::Error for ApiHttpError {}

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

#[derive(Debug)]
pub struct ApiClientError {
    pub status: reqwest::StatusCode,
    pub headers: reqwest::header::HeaderMap,
    pub problem: ApiProblemV1,
}

impl ApiClientError {
    pub fn recovery_action(&self) -> Option<&RecoveryAction> {
        self.problem.recovery_action()
    }

    pub fn retry_after_seconds(&self) -> Option<u64> {
        self.headers
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .or(self.problem.retry_after_seconds)
    }
}

impl std::fmt::Display for ApiClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "API error ({}): {}",
            self.status, self.problem.error
        )?;
        if let Some(code) = self.problem.code.as_deref() {
            write!(formatter, " ({code})")?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiClientError {}

/// Compatibility view used by command code while API failures migrate from
/// the legacy flat error to the structured problem contract.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ApiErrorDetails<'a> {
    pub status: u16,
    pub message: &'a str,
    pub code: Option<&'a str>,
    pub upgrade_command: Option<&'a str>,
}

pub(crate) fn api_error_details(error: &anyhow::Error) -> Option<ApiErrorDetails<'_>> {
    if let Some(error) = error.downcast_ref::<ApiClientError>() {
        return Some(ApiErrorDetails {
            status: error.status.as_u16(),
            message: &error.problem.error,
            code: error.problem.code.as_deref(),
            upgrade_command: error
                .problem
                .extra
                .get("upgradeCommand")
                .and_then(serde_json::Value::as_str),
        });
    }
    error
        .downcast_ref::<ApiHttpError>()
        .map(|error| ApiErrorDetails {
            status: error.status,
            message: &error.message,
            code: error.code.as_deref(),
            upgrade_command: error.upgrade_command.as_deref(),
        })
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
    /// The program SDK each listed ProgramSpec should carry. Present, even
    /// empty, to ask for the response's `programSdks` report; absent only
    /// for a registry that does not know the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_sdks: Option<Vec<ProgramSdkReference>>,
}

/// The program SDK one ProgramSpec of a deployed StackManifest should carry:
/// an exact program package release.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramSdkReference {
    pub program_spec_hash: String,
    pub program_package_release: String,
}

/// The program SDK one program of a deployment plan carries. No `source`
/// means the core program SDK only, and `reason` says why.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramSdkAssignment {
    pub program_id: String,
    pub program_spec_hash: String,
    pub program_release_hash: String,
    pub program_package_release: Option<String>,
    pub package: Option<String>,
    pub version: Option<String>,
    /// `requested`, `catalog` or `owner`.
    pub source: Option<String>,
    pub reason: Option<String>,
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
    /// Present only when the request carried `programSdks`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_sdks: Option<Vec<ProgramSdkAssignment>>,
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

fn default_standard_service_class() -> String {
    "standard".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryStackItem {
    pub name: String,
    pub description: Option<String>,
    pub websocket_url: String,
    pub entities: Vec<String>,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(rename = "serviceClass", default = "default_standard_service_class")]
    pub service_class: String,
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
    /// Extension API contract the bundle was written against: a positive
    /// integer the installed SDK runtime must report as its own
    /// `extensionApi`. `sdkRange` stays as the floor for older CLIs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_api: Option<std::num::NonZeroU32>,
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
    #[serde(default = "default_standard_service_class")]
    pub service_class: String,
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
    /// The program package release a stack references for this program: its
    /// program SDK identity. Present only under the `program-sdks` resolver
    /// opt-in, and only when the stack references a program package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_package: Option<RegistryProgramPackageReference>,
    /// The referenced program package's SDK extensions for the requested
    /// targets. `None` when the registry did not send the field, in which
    /// case the legacy `definition.extensions` applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_extensions: Option<Vec<crate::project::resolver::ResolvedSdkExtension>>,
}

/// One exact program package release: the identity of a program SDK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryProgramPackageReference {
    pub package: String,
    pub version: String,
    pub package_release_hash: String,
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

        let api_key = Self::load_optional_api_key_for_url(&base_url)?;

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

    /// The knowledge document an active catalog entry publishes: for a
    /// stack, entity and view summaries and curated field descriptions.
    ///
    /// Optional context, so every failure is `None`: no catalog entry, no
    /// document, a registry that predates the route (404), a non-JSON body
    /// or a transport error. The body is returned as raw JSON and read
    /// leniently by the caller, since documents gain keys over time. It is
    /// abandoned after [`arete_mcp::stack_knowledge::LOOKUP_TIMEOUT`],
    /// connecting included, so it never holds up the command it accompanies
    /// for longer.
    pub fn catalog_entry_knowledge(&self, kind: &str, slug: &str) -> Option<serde_json::Value> {
        let response = self
            .with_optional_auth(self.client.get(format!(
                "{}/api/registry/v1/catalog/entries/{}/{}/knowledge",
                self.base_url, kind, slug
            )))
            .timeout(arete_mcp::stack_knowledge::LOOKUP_TIMEOUT)
            .send()
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        response
            .json::<serde_json::Value>()
            .ok()
            .filter(serde_json::Value::is_object)
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
    ///
    /// The batch opts into `include=delivery,delivery-lifecycle,program-sdks`,
    /// so every resolved stack names its delivery mode, a hosted stack carries
    /// its live and gateway bindings, a version being retired says until when
    /// it is served (and a retired version resolves as `retired` instead of
    /// failing), and each stack program names the program package release (and
    /// its SDK extensions) the stack references, all in the same response: one
    /// request, however many stacks. Registries that predate an opt-in ignore
    /// it, so every field it adds is optional.
    pub fn resolve_registry_dependencies(
        &self,
        request: &crate::project::resolver::RegistryResolveRequest,
    ) -> Result<crate::project::resolver::RegistryResolveResponse> {
        let response = self
            .with_optional_auth(
                self.client
                    .post(format!(
                        "{}/api/registry/v1/resolve?include=delivery,delivery-lifecycle,program-sdks",
                        self.base_url
                    ))
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

    fn response_error(response: reqwest::blocking::Response) -> ApiClientError {
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.text().unwrap_or_default();
        let problem = serde_json::from_str::<ApiProblemV1>(&body).unwrap_or_else(|_| {
            let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
            ApiProblemV1 {
                schema_version: None,
                error: if compact.is_empty() {
                    "Empty error response".to_string()
                } else {
                    compact.chars().take(1024).collect()
                },
                code: None,
                retryable: status.is_server_error().then_some(true),
                request_id: None,
                retry_after_seconds: None,
                usage: None,
                action: None,
                extra: BTreeMap::new(),
            }
        });
        ApiClientError {
            status,
            headers,
            problem,
        }
    }

    fn handle_response<T: for<'de> Deserialize<'de>>(
        response: reqwest::blocking::Response,
    ) -> Result<T> {
        if response.status().is_success() {
            response.json().context("Failed to parse response JSON")
        } else {
            Err(Self::response_error(response).into())
        }
    }

    // Credentials management

    fn credentials_path() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os(ENV_VAR_CREDENTIALS_PATH) {
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

    pub fn selected_profile() -> Result<Option<String>> {
        std::env::var(ENV_VAR_PROFILE)
            .ok()
            .map(|profile| validate_profile_name(&profile).map(str::to_string))
            .transpose()
    }

    #[allow(dead_code)]
    pub fn save_api_key(api_key: &str, api_url: Option<&str>) -> Result<()> {
        let profile = inferred_profile_for_key(api_key);
        Self::save_api_key_for_profile(api_key, api_url, profile)
    }

    pub fn save_api_key_for_profile(
        api_key: &str,
        api_url: Option<&str>,
        profile: &str,
    ) -> Result<()> {
        let profile = validate_profile_name(profile)?;
        validate_key_for_profile(profile, api_key)?;
        let _lock = Self::lock_credentials()?;
        let path = Self::credentials_path()?;
        ensure_safe_credentials_path(&path)?;

        let target_url = api_url
            .map(|s| s.to_string())
            .or_else(|| std::env::var("ARETE_API_URL").ok())
            .unwrap_or_else(|| DEFAULT_API_URL.to_string());

        // Read existing credentials or create new
        let creds_content = if path.exists() {
            fs::read_to_string(&path).context("Failed to read existing credentials file")?
        } else {
            String::new()
        };

        // Parse existing or create new
        let mut creds: toml::Value = if creds_content.is_empty() {
            toml::Value::Table(toml::map::Map::new())
        } else {
            toml::from_str(&creds_content)
                .context("Existing credentials file is malformed; refusing to replace it")?
        };

        // Get or create [profiles.<profile>.keys]. Legacy [keys] entries are
        // preserved for older clients and are used only when compatible with
        // an explicitly selected built-in profile.
        let profiles = creds
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("Invalid credentials format"))?
            .entry("profiles")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("Invalid profiles format"))?;
        let profile_table = profiles
            .entry(profile)
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("Invalid profile `{profile}` format"))?;
        let keys = profile_table
            .entry("keys")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("Invalid profile `{profile}` keys format"))?;

        // Add or update the key for this URL
        keys.insert(target_url.clone(), toml::Value::String(api_key.to_string()));

        // Write back
        let content = toml::to_string_pretty(&creds)?;
        write_credentials_atomic(&path, content.as_bytes()).context("Failed to save API key")?;

        Ok(())
    }

    fn parse_api_key_for_profile(
        content: &str,
        api_url: &str,
        profile: Option<&str>,
    ) -> Result<Option<String>> {
        lookup_credentials(content, api_url, profile)
            .context("Failed to parse credentials file")
            .map(|lookup| lookup.key)
    }

    #[cfg(test)]
    fn parse_api_key(content: &str, api_url: &str) -> Result<Option<String>> {
        Self::parse_api_key_for_profile(content, api_url, None)
    }

    /// Load API key for a specific URL (new URL-based format)
    pub fn load_api_key_for_url(api_url: &str) -> Result<String> {
        Self::load_optional_api_key_for_url(api_url)?.ok_or_else(|| {
            let profile = Self::selected_profile()
                .ok()
                .flatten()
                .map(|profile| format!(" for profile `{profile}`"))
                .unwrap_or_default();
            anyhow::anyhow!(
                "No API key found{profile} for API URL: {api_url}. Run 'a4 auth signup' for an agent or 'a4 auth login --profile human' for a human key."
            )
        })
    }

    /// Load an API key when credentials are genuinely absent.
    ///
    /// Broken credential paths remain errors instead of silently becoming
    /// anonymous access.
    pub fn load_optional_api_key_for_url(api_url: &str) -> Result<Option<String>> {
        let profile = Self::selected_profile()?;
        if profile.is_none() {
            if let Some(key) = std::env::var(ENV_VAR_API_KEY)
                .ok()
                .map(|key| key.trim().to_string())
                .filter(|key| !key.is_empty())
            {
                return Ok(Some(key));
            }
        }
        Self::load_optional_api_key_for_profile(api_url, profile.as_deref())
    }

    pub fn load_optional_api_key_for_profile(
        api_url: &str,
        profile: Option<&str>,
    ) -> Result<Option<String>> {
        let path = Self::credentials_path()?;
        ensure_safe_credentials_path(&path)?;
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => {
                return Err(error).context("Failed to read credentials file");
            }
        };
        Self::parse_api_key_for_profile(&content, api_url, profile)
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

    pub fn list_credentials() -> Result<Vec<StoredCredential>> {
        let path = Self::credentials_path()?;
        ensure_safe_credentials_path(&path)?;
        let content = fs::read_to_string(&path).context("Failed to read credentials file")?;

        let creds: toml::Value =
            toml::from_str(&content).context("Failed to parse credentials file")?;

        let mut result = Vec::new();
        if let Some(profiles) = creds.get("profiles").and_then(toml::Value::as_table) {
            for (profile, value) in profiles {
                let Some(keys) = value.get("keys").and_then(toml::Value::as_table) else {
                    continue;
                };
                for (url, key_value) in keys {
                    if let Some(key) = key_value.as_str() {
                        result.push(StoredCredential {
                            profile: Some(profile.clone()),
                            api_url: url.clone(),
                            masked_key: mask_api_key(key),
                        });
                    }
                }
            }
        }

        // Preserve visibility into legacy URL-keyed entries during migration.
        if let Some(keys) = creds.get("keys").and_then(|k| k.as_table()) {
            for (url, key_value) in keys.iter() {
                if let Some(key) = key_value.as_str() {
                    result.push(StoredCredential {
                        profile: None,
                        api_url: url.clone(),
                        masked_key: mask_api_key(key),
                    });
                }
            }
        }

        if let Some(key) = creds.get("api_key").and_then(toml::Value::as_str) {
            result.push(StoredCredential {
                profile: None,
                api_url: DEFAULT_API_URL.to_string(),
                masked_key: mask_api_key(key),
            });
        }

        result.sort_by(|left, right| {
            left.profile
                .cmp(&right.profile)
                .then_with(|| left.api_url.cmp(&right.api_url))
        });
        Ok(result)
    }

    pub fn delete_api_key_for_url(api_url: &str) -> Result<()> {
        let profile = Self::selected_profile()?;
        Self::delete_api_key_for_profile(api_url, profile.as_deref())
    }

    pub fn delete_api_key_for_profile(api_url: &str, profile: Option<&str>) -> Result<()> {
        let _lock = Self::lock_credentials()?;
        let path = Self::credentials_path()?;
        ensure_safe_credentials_path(&path)?;
        if !path.exists() {
            anyhow::bail!("No credentials file found");
        }

        let content = fs::read_to_string(&path)?;
        let mut creds: toml::Value = toml::from_str(&content)?;
        let lookup = lookup_credentials(&content, api_url, profile)?;
        if lookup.key.is_none() {
            anyhow::bail!("No API key found for URL: {api_url}");
        }

        let removed = match lookup.profile.as_deref() {
            Some(profile_name) => creds
                .get_mut("profiles")
                .and_then(toml::Value::as_table_mut)
                .and_then(|profiles| profiles.get_mut(profile_name))
                .and_then(toml::Value::as_table_mut)
                .and_then(|profile| profile.get_mut("keys"))
                .and_then(toml::Value::as_table_mut)
                .is_some_and(|keys| remove_url_key(keys, api_url)),
            None => {
                let removed_url_key = creds
                    .get_mut("keys")
                    .and_then(toml::Value::as_table_mut)
                    .is_some_and(|keys| remove_url_key(keys, api_url));
                if removed_url_key {
                    true
                } else {
                    creds
                        .as_table_mut()
                        .is_some_and(|table| table.remove("api_key").is_some())
                }
            }
        };

        if removed {
            let content = toml::to_string_pretty(&creds)?;
            write_credentials_atomic(&path, content.as_bytes())?;
            Ok(())
        } else {
            anyhow::bail!("No API key found for URL: {api_url}")
        }
    }

    pub fn delete_all_api_keys() -> Result<()> {
        let _lock = Self::lock_credentials()?;
        let path = Self::credentials_path()?;
        ensure_safe_credentials_path(&path)?;
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

/// Response from `POST /api/agents/signup/v2`. The client-generated secret is
/// deliberately absent from the response contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSignupResponse {
    pub schema_version: u8,
    pub slug: String,
    pub display_name: String,
    pub created_at: String,
    pub plan: String,
    pub entitlement_expires_at: String,
    pub claim_state: String,
    pub idempotent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMeResponse {
    pub slug: String,
    pub display_name: String,
    pub status: String,
    pub created_at: String,
    #[serde(default)]
    pub last_seen_at: Option<String>,
    #[serde(rename = "claimState", alias = "claim_state")]
    pub claim_state: String,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(rename = "entitlementExpiresAt", default)]
    pub entitlement_expires_at: Option<String>,
    #[serde(rename = "trialAccessEnabled", default)]
    pub trial_access_enabled: Option<bool>,
    #[serde(rename = "starterGuidance", default)]
    pub starter_guidance: Option<String>,
    #[serde(default)]
    pub usage: Option<AgentUsageSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUsageSummary {
    pub window: String,
    pub window_start: String,
    #[serde(default)]
    pub window_end: Option<String>,
    pub meters: Vec<AgentUsageMeterSummary>,
    pub exhausted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUsageMeterSummary {
    pub meter: String,
    pub consumed: u64,
    pub allowance: u64,
    pub remaining: u64,
    pub exhausted: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSignupRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<&'a str>,
    credential: &'a str,
    idempotency_key: &'a str,
}

impl ApiClient {
    /// Build a client against an explicit base URL with no stored key.
    /// Used by unauthenticated signup and by login's explicit-key verification
    /// so neither operation can accidentally inherit another profile.
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

    /// Serialize every read-modify-write of the shared credentials file.
    pub fn lock_credentials() -> Result<CredentialsLock> {
        use fs2::FileExt;

        let credentials_path = Self::credentials_path()?;
        ensure_safe_credentials_path(&credentials_path)?;
        let parent = credentials_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Credentials path must have a parent directory"))?;
        let parent_existed = parent.exists();
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create credentials directory {}",
                parent.display()
            )
        })?;
        ensure_owner_only_directory(parent, !parent_existed)?;
        let filename = credentials_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("Credentials filename must be valid UTF-8"))?;
        let lock_path = parent.join(format!(".{filename}.lock"));
        ensure_safe_credentials_path(&lock_path)?;

        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&lock_path)
            .with_context(|| format!("Failed to open credentials lock {}", lock_path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        ensure_safe_credentials_path(&lock_path)?;
        file.lock_exclusive()
            .context("Failed to acquire the credentials lock")?;
        Ok(CredentialsLock { file })
    }

    pub fn load_pending_agent_signup(
        api_url: &str,
        profile: &str,
    ) -> Result<Option<PendingAgentSignup>> {
        let profile = validate_profile_name(profile)?;
        let path = Self::credentials_path()?;
        ensure_safe_credentials_path(&path)?;
        if !path.exists() {
            return Ok(None);
        }
        let credentials = read_credentials_value(&path)?;
        let Some(entry) = credentials
            .get("profiles")
            .and_then(toml::Value::as_table)
            .and_then(|profiles| profiles.get(profile))
            .and_then(toml::Value::as_table)
            .and_then(|profile| profile.get("pendingSignup"))
            .and_then(toml::Value::as_table)
            .and_then(|pending| url_value(pending, api_url))
            .and_then(toml::Value::as_table)
        else {
            return Ok(None);
        };
        let credential = entry
            .get("credential")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Pending agent signup credential is malformed"))?;
        let idempotency_key = entry
            .get("idempotencyKey")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Pending agent signup idempotency key is malformed"))?;
        validate_key_for_profile(profile, credential)?;
        validate_signup_idempotency_key(idempotency_key)?;
        Ok(Some(PendingAgentSignup {
            credential: credential.to_string(),
            idempotency_key: idempotency_key.to_string(),
        }))
    }

    pub fn save_pending_agent_signup(
        api_url: &str,
        profile: &str,
        pending: &PendingAgentSignup,
    ) -> Result<()> {
        let profile = validate_profile_name(profile)?;
        validate_key_for_profile(profile, &pending.credential)?;
        validate_signup_idempotency_key(&pending.idempotency_key)?;
        let path = Self::credentials_path()?;
        ensure_safe_credentials_path(&path)?;
        let mut credentials = read_credentials_value(&path)?;
        let profile_table = profile_table_mut(&mut credentials, profile)?;
        let pending_table = profile_table
            .entry("pendingSignup")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("Invalid profile `{profile}` pending signup format"))?;
        let mut entry = toml::map::Map::new();
        entry.insert(
            "credential".to_string(),
            toml::Value::String(pending.credential.clone()),
        );
        entry.insert(
            "idempotencyKey".to_string(),
            toml::Value::String(pending.idempotency_key.clone()),
        );
        pending_table.insert(normalize_api_url(api_url), toml::Value::Table(entry));
        let content = toml::to_string_pretty(&credentials)?;
        write_credentials_atomic(&path, content.as_bytes())
            .context("Failed to save pending agent signup")
    }

    pub fn clear_pending_agent_signup(api_url: &str, profile: &str) -> Result<()> {
        let profile = validate_profile_name(profile)?;
        let path = Self::credentials_path()?;
        ensure_safe_credentials_path(&path)?;
        if !path.exists() {
            return Ok(());
        }
        let mut credentials = read_credentials_value(&path)?;
        let removed = credentials
            .get_mut("profiles")
            .and_then(toml::Value::as_table_mut)
            .and_then(|profiles| profiles.get_mut(profile))
            .and_then(toml::Value::as_table_mut)
            .and_then(|profile| profile.get_mut("pendingSignup"))
            .and_then(toml::Value::as_table_mut)
            .is_some_and(|pending| take_url_value(pending, api_url).is_some());
        if removed {
            if let Some(profile_table) = credentials
                .get_mut("profiles")
                .and_then(toml::Value::as_table_mut)
                .and_then(|profiles| profiles.get_mut(profile))
                .and_then(toml::Value::as_table_mut)
            {
                let pending_is_empty = profile_table
                    .get("pendingSignup")
                    .and_then(toml::Value::as_table)
                    .is_some_and(toml::map::Map::is_empty);
                if pending_is_empty {
                    profile_table.remove("pendingSignup");
                }
            }
            let content = toml::to_string_pretty(&credentials)?;
            write_credentials_atomic(&path, content.as_bytes())?;
        }
        Ok(())
    }

    /// Promote a verified pending credential in one atomic file replacement.
    /// If active keys are replaced, retain local recovery copies under their
    /// original URL spellings.
    pub fn promote_pending_agent_signup(
        api_url: &str,
        profile: &str,
        expected: &PendingAgentSignup,
    ) -> Result<()> {
        let profile = validate_profile_name(profile)?;
        let path = Self::credentials_path()?;
        ensure_safe_credentials_path(&path)?;
        let mut credentials = read_credentials_value(&path)?;
        let profile_table = profile_table_mut(&mut credentials, profile)?;
        let pending_table = profile_table
            .get_mut("pendingSignup")
            .and_then(toml::Value::as_table_mut)
            .ok_or_else(|| anyhow::anyhow!("Pending agent signup state is missing"))?;
        let stored = take_url_value(pending_table, api_url)
            .and_then(|value| value.as_table().cloned())
            .ok_or_else(|| anyhow::anyhow!("Pending agent signup state is missing"))?;
        if pending_table.is_empty() {
            profile_table.remove("pendingSignup");
        }
        let stored_credential = stored
            .get("credential")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Pending agent signup credential is malformed"))?;
        let stored_idempotency_key = stored
            .get("idempotencyKey")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Pending agent signup idempotency key is malformed"))?;
        if stored_credential != expected.credential
            || stored_idempotency_key != expected.idempotency_key
        {
            anyhow::bail!("Pending agent signup changed while it was being verified");
        }

        let target_url = normalize_api_url(api_url);
        let old_keys = profile_table
            .entry("keys")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("Invalid profile `{profile}` keys format"))
            .map(|keys| {
                let old = take_equivalent_url_values(keys, api_url);
                keys.insert(
                    target_url.clone(),
                    toml::Value::String(expected.credential.clone()),
                );
                old
            })?;
        let old_keys = old_keys
            .into_iter()
            .filter(|(_, value)| {
                value
                    .as_str()
                    .is_some_and(|value| value != expected.credential)
            })
            .collect::<Vec<_>>();
        if !old_keys.is_empty() {
            let backup_keys = profile_table
                .entry("backupKeys")
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("Invalid profile `{profile}` backup keys format"))?;
            for (url, old_key) in old_keys {
                backup_keys.insert(url, old_key);
            }
        }
        let content = toml::to_string_pretty(&credentials)?;
        write_credentials_atomic(&path, content.as_bytes())
            .context("Failed to activate the verified agent credential")
    }

    /// Register this machine as an agent trial (unauthenticated).
    pub fn agent_signup(
        &self,
        display_name: Option<&str>,
        credential: &str,
        idempotency_key: &str,
    ) -> Result<AgentSignupResponse> {
        let response = self
            .client
            .post(format!("{}/api/agents/signup/v2", self.base_url))
            .timeout(Duration::from_secs(30))
            .json(&AgentSignupRequest {
                display_name,
                credential,
                idempotency_key,
            })
            .send()
            .context("Failed to reach the signup endpoint")?;
        Self::handle_response(response)
    }

    /// `GET /api/agents/me` with the stored agent key.
    pub fn agent_me(&self) -> Result<AgentMeResponse> {
        let api_key = self.require_api_key()?;
        let response = self
            .client
            .get(format!("{}/api/agents/me", self.base_url))
            .timeout(Duration::from_secs(30))
            .bearer_auth(api_key)
            .send()
            .context("Failed to fetch agent identity")?;
        Self::handle_response(response)
    }

    /// `GET /api/auth/me`: the caller's account kind, plan and capabilities,
    /// for human and agent keys alike. Decoded with
    /// [`AccountCapabilities::from_value`], which tolerates unknown fields.
    pub fn account_capabilities(&self) -> Result<AccountCapabilities> {
        let api_key = self.require_api_key()?;
        let response = self
            .client
            .get(format!("{}/api/auth/me", self.base_url))
            .bearer_auth(api_key)
            .send()
            .context("Failed to fetch account capabilities")?;
        let value: serde_json::Value = Self::handle_response(response)?;
        AccountCapabilities::from_value(&value)
            .ok_or_else(|| anyhow::anyhow!("the account response did not list capabilities"))
    }

    /// Materialize a short-lived human claim link for the current agent.
    /// Callers must validate the returned URL before displaying it.
    pub fn agent_claim_link(&self) -> Result<ReadyRecoveryActionV1> {
        let api_key = self.require_api_key()?;
        let response = self
            .client
            .post(format!("{}/api/agents/me/claim-links", self.base_url))
            .bearer_auth(api_key)
            .send()
            .context("Failed to create agent claim link")?;
        Self::handle_response(response)
    }

    pub fn configured_claim_app_origin() -> Result<url::Url> {
        let value = std::env::var("ARETE_APP_ORIGIN").unwrap_or_else(|_| {
            if cfg!(feature = "local") {
                "http://localhost:3000".to_string()
            } else {
                DEFAULT_APP_ORIGIN.to_string()
            }
        });
        let mut origin = url::Url::parse(&value)
            .with_context(|| format!("Invalid ARETE_APP_ORIGIN: {value}"))?;
        if !origin.username().is_empty()
            || origin.password().is_some()
            || origin.query().is_some()
            || origin.fragment().is_some()
        {
            anyhow::bail!(
                "ARETE_APP_ORIGIN must be an origin without credentials, query, or fragment"
            );
        }
        origin.set_path("");
        Ok(origin)
    }
}

/// Capability that lets an account inspect (simulate) transactions.
pub const CAPABILITY_TRANSACTION_INSPECT: &str = "transaction_inspect";
/// Capability that lets an account submit transactions.
pub const CAPABILITY_TRANSACTION_SEND: &str = "transaction_send";
/// Capability that lets an account deploy stacks (`a4 up`).
pub const CAPABILITY_CREATE_DEPLOYMENT: &str = "create_deployment";

/// What `GET /api/auth/me` reports about the caller's account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountCapabilities {
    /// `human` or `agent`, when reported.
    pub account_kind: Option<String>,
    /// The account's plan name, when reported.
    pub plan: Option<String>,
    pub capabilities: Vec<String>,
}

impl AccountCapabilities {
    /// Decode tolerantly: camelCase or snake_case keys, and capabilities as
    /// strings or as `{ "name" | "id" | "capability": … }` objects. `None`
    /// when there is no capability list to judge readiness by.
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        let text = |keys: &[&str]| {
            keys.iter()
                .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
                .map(str::to_string)
        };
        let capabilities = ["capabilities", "capability"]
            .iter()
            .find_map(|key| value.get(*key).and_then(serde_json::Value::as_array))?
            .iter()
            .filter_map(|capability| {
                capability.as_str().or_else(|| {
                    ["name", "id", "capability"]
                        .iter()
                        .find_map(|key| capability.get(*key).and_then(serde_json::Value::as_str))
                })
            })
            .map(|capability| capability.trim().to_string())
            .filter(|capability| !capability.is_empty())
            .collect();
        Some(Self {
            account_kind: text(&["accountKind", "account_kind", "kind"]),
            plan: text(&["plan", "planName", "plan_name"]),
            capabilities,
        })
    }

    /// The entries of `required` this account lacks, in order.
    pub fn missing<'a>(&self, required: &[&'a str]) -> Vec<&'a str> {
        required
            .iter()
            .copied()
            .filter(|required| {
                !self
                    .capabilities
                    .iter()
                    .any(|capability| capability == required)
            })
            .collect()
    }
}

/// Minimal canned-response HTTP server for unit tests of `ApiClient` and the
/// commands built on it.
#[cfg(test)]
pub(crate) mod test_support {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::sync::Mutex;

    /// `ARETE_API_URL` and `ARETE_CREDENTIALS_PATH` are process-global;
    /// every test that sets them serialises on this lock.
    pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());
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

    const CREDENTIAL: &str = "a4_ak_0123456789012345678901234567890123456789";
    const IDEMPOTENCY_KEY: &str = "11111111-1111-4111-8111-111111111111";
    const SIGNUP_RESPONSE: &str = r#"{"schemaVersion":1,"slug":"agent-7f3a","displayName":"Robo","createdAt":"2026-09-29T00:00:00Z","plan":"agent_trial","entitlementExpiresAt":"2026-10-06T00:00:00Z","claimState":"unclaimed","idempotent":false}"#;

    #[test]
    fn agent_signup_posts_display_name_without_auth_and_parses_response() {
        let server = MockServer::json(200, SIGNUP_RESPONSE);
        let client = ApiClient::with_base_url(server.base_url());
        let resp = client
            .agent_signup(Some("Robo"), CREDENTIAL, IDEMPOTENCY_KEY)
            .expect("signup succeeds");
        assert_eq!(resp.slug, "agent-7f3a");
        assert_eq!(resp.display_name, "Robo");
        assert_eq!(resp.plan, "agent_trial");
        assert!(!resp.idempotent);

        let req = server.request();
        assert_eq!(req.request_line, "POST /api/agents/signup/v2 HTTP/1.1");
        assert!(
            req.header("authorization").is_none(),
            "signup is unauthenticated"
        );
        let body: serde_json::Value = serde_json::from_str(&req.body).expect("json body");
        assert_eq!(
            body,
            serde_json::json!({
                "displayName": "Robo",
                "credential": CREDENTIAL,
                "idempotencyKey": IDEMPOTENCY_KEY,
            })
        );
    }

    #[test]
    fn agent_signup_omits_display_name() {
        let server = MockServer::json(201, SIGNUP_RESPONSE);
        let client = ApiClient::with_base_url(server.base_url());
        client
            .agent_signup(None, CREDENTIAL, IDEMPOTENCY_KEY)
            .expect("signup succeeds");
        let req = server.request();
        let body: serde_json::Value = serde_json::from_str(&req.body).expect("json body");
        assert_eq!(
            body,
            serde_json::json!({
                "credential": CREDENTIAL,
                "idempotencyKey": IDEMPOTENCY_KEY,
            })
        );
    }

    #[test]
    fn agent_signup_preserves_structured_rate_limit_problem() {
        let server = MockServer::json(
            429,
            r#"{"schemaVersion":1,"error":"rate limited","code":"rate_limit_exceeded","retryable":true,"retryAfterSeconds":60}"#,
        );
        let client = ApiClient::with_base_url(server.base_url());
        let err = client
            .agent_signup(None, CREDENTIAL, IDEMPOTENCY_KEY)
            .expect_err("429 is an error");
        let api_error = err
            .downcast_ref::<ApiClientError>()
            .expect("typed API error");
        assert_eq!(api_error.status, reqwest::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            api_error.problem.code.as_deref(),
            Some("rate_limit_exceeded")
        );
        assert_eq!(api_error.retry_after_seconds(), Some(60));
    }

    #[test]
    fn account_capabilities_reads_auth_me_with_the_stored_key() {
        let server = MockServer::json(
            200,
            r#"{"accountKind":"agent","plan":"example-plan","capabilities":["transaction_inspect",{"name":"create_deployment"}],"future":true}"#,
        );
        let client = ApiClient::with_base_url(server.base_url()).with_api_key("a4_ak_me".into());
        let account = client.account_capabilities().expect("capabilities decode");
        assert_eq!(account.account_kind.as_deref(), Some("agent"));
        assert_eq!(account.plan.as_deref(), Some("example-plan"));
        assert_eq!(
            account.missing(&[CAPABILITY_TRANSACTION_INSPECT, CAPABILITY_TRANSACTION_SEND]),
            vec![CAPABILITY_TRANSACTION_SEND]
        );
        assert!(account.missing(&[CAPABILITY_CREATE_DEPLOYMENT]).is_empty());
        let req = server.request();
        assert_eq!(req.request_line, "GET /api/auth/me HTTP/1.1");
        assert_eq!(req.header("authorization"), Some("Bearer a4_ak_me"));
    }

    #[test]
    fn account_capabilities_decodes_snake_case_and_refuses_a_missing_list() {
        let account = AccountCapabilities::from_value(&serde_json::json!({
            "account_kind": "human",
            "capabilities": [" transaction_send ", ""]
        }))
        .expect("capability list present");
        assert_eq!(account.account_kind.as_deref(), Some("human"));
        assert_eq!(account.plan, None);
        assert_eq!(account.capabilities, vec!["transaction_send"]);
        assert_eq!(
            AccountCapabilities::from_value(&serde_json::json!({"plan": "x"})),
            None
        );

        let server = MockServer::json(404, r#"{"error":"not found"}"#);
        let client = ApiClient::with_base_url(server.base_url()).with_api_key("a4_ak_me".into());
        let err = client.account_capabilities().expect_err("404 is an error");
        assert_eq!(api_error_details(&err).map(|error| error.status), Some(404));
    }

    #[test]
    fn agent_signup_other_errors_use_api_error_format() {
        let server = MockServer::json(500, r#"{"error":"boom"}"#);
        let client = ApiClient::with_base_url(server.base_url());
        let err = client
            .agent_signup(None, CREDENTIAL, IDEMPOTENCY_KEY)
            .expect_err("500 is an error");
        assert_eq!(
            err.to_string(),
            "API error (500 Internal Server Error): boom"
        );
    }

    #[test]
    fn agent_me_sends_bearer_and_returns_typed_identity() {
        let server = MockServer::json(
            200,
            r#"{"slug":"agent-1","display_name":"Agent One","status":"active","created_at":"2026-09-22T00:00:00Z","last_seen_at":null,"claimState":"unclaimed"}"#,
        );
        let client =
            ApiClient::with_base_url(server.base_url()).with_api_key("a4_ak_me".to_string());
        let me = client.agent_me().expect("me succeeds");
        assert_eq!(me.slug, "agent-1");
        assert_eq!(me.claim_state, "unclaimed");
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
    use super::ensure_safe_credentials_path;
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[test]
    fn dangling_credentials_symlink_is_not_treated_as_missing() {
        let root =
            std::env::temp_dir().join(format!("a4-dangling-credentials-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let credentials = root.join("credentials.toml");
        symlink(root.join("missing.toml"), &credentials).unwrap();

        let error = ensure_safe_credentials_path(&credentials).unwrap_err();

        assert!(error.to_string().contains("must not contain symlinks"));
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
            ensure_safe_credentials_path(&credentials_dir.join("credentials.toml")).unwrap_err();

        assert!(error.to_string().contains("must not contain symlinks"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn atomic_credentials_write_is_owner_only() {
        let root = tempfile::tempdir().expect("tempdir");
        let credentials_dir = root.path().join(".arete");
        let credentials = credentials_dir.join("credentials.toml");

        write_credentials_atomic(&credentials, b"[keys]\n").expect("secure write");

        assert_eq!(
            fs::metadata(&credentials_dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&credentials).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read(&credentials).unwrap(), b"[keys]\n");
    }

    #[test]
    fn atomic_credentials_write_refuses_symlink_without_touching_target() {
        let root = tempfile::tempdir().expect("tempdir");
        let target = root.path().join("target.toml");
        let credentials = root.path().join("credentials.toml");
        fs::write(&target, "sentinel").unwrap();
        symlink(&target, &credentials).unwrap();

        let error = write_credentials_atomic(&credentials, b"replacement").unwrap_err();

        assert!(error.to_string().contains("must not contain symlinks"));
        assert_eq!(fs::read_to_string(target).unwrap(), "sentinel");
    }

    #[test]
    fn credential_mutations_wait_for_the_shared_lock() {
        use std::sync::mpsc;
        use std::time::Duration;

        let _environment = test_support::ENV_LOCK.lock().unwrap();
        let root = tempfile::tempdir().expect("tempdir");
        let credentials = root.path().join(".arete/credentials.toml");
        std::env::set_var(ENV_VAR_CREDENTIALS_PATH, &credentials);

        let held = ApiClient::lock_credentials().expect("first lock");
        let (finished_tx, finished_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            let result = ApiClient::save_api_key_for_profile(
                "a4_sk_human-test-key",
                Some("https://api.arete.run"),
                "human",
            );
            finished_tx.send(result).unwrap();
        });
        assert!(finished_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        drop(held);
        finished_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("writer acquires released lock")
            .expect("writer succeeds");
        writer.join().unwrap();

        ApiClient::save_api_key_for_profile(
            "a4_ak_agent-test-key",
            Some("https://api.arete.run"),
            "agent",
        )
        .expect("second profile is preserved");
        assert!(ApiClient::load_optional_api_key_for_profile(
            "https://api.arete.run",
            Some("human")
        )
        .unwrap()
        .is_some());
        assert!(ApiClient::load_optional_api_key_for_profile(
            "https://api.arete.run",
            Some("agent")
        )
        .unwrap()
        .is_some());

        std::env::remove_var(ENV_VAR_CREDENTIALS_PATH);
    }

    #[test]
    fn malformed_credentials_are_errors_instead_of_anonymous_fallback() {
        let error = ApiClient::parse_api_key("[keys\n", "https://api.arete.run")
            .expect_err("malformed TOML must be preserved as an error");
        assert!(error
            .to_string()
            .contains("Failed to parse credentials file"));
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
        assert_eq!(artifact.manifest.extension_api, None);
        // An absent extensionApi is not written back, so cached manifests
        // round-trip unchanged.
        assert!(!serde_json::to_string(&artifact.manifest)
            .unwrap()
            .contains("extensionApi"));
    }

    #[test]
    fn sdk_extension_manifest_accepts_only_a_positive_extension_api() {
        let manifest = |extension_api: serde_json::Value| {
            serde_json::from_value::<RegistrySdkExtensionManifest>(json!({
                "entry": "index.ts",
                "files": ["index.ts"],
                "inputKind": null,
                "inputHash": null,
                "sdkRange": "^0.23.0",
                "extensionApi": extension_api
            }))
        };
        assert_eq!(
            manifest(json!(1)).unwrap().extension_api.map(|v| v.get()),
            Some(1)
        );
        assert_eq!(manifest(json!(null)).unwrap().extension_api, None);
        for invalid in [json!(0), json!(-1), json!(1.5), json!("1")] {
            assert!(manifest(invalid.clone()).is_err(), "{invalid}");
        }
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
            "serviceClass": "standard",
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
            program_sdks: None,
        };
        let mut expected = preflight_value;
        expected["idempotencyKey"] = json!("8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a");
        assert_eq!(serde_json::to_value(&plan).unwrap(), expected);

        // Asking for the program SDK report sends the key, even empty.
        let mut empty = plan.clone();
        empty.program_sdks = Some(Vec::new());
        expected["programSdks"] = json!([]);
        assert_eq!(serde_json::to_value(&empty).unwrap(), expected);
        let mut requested = plan;
        requested.program_sdks = Some(vec![ProgramSdkReference {
            program_spec_hash: "arete:h1:program-spec:sha256:1".into(),
            program_package_release: "arete:registry-package-release:v2:sha256:2".into(),
        }]);
        expected["programSdks"] = json!([{
            "programSpecHash": "arete:h1:program-spec:sha256:1",
            "programPackageRelease": "arete:registry-package-release:v2:sha256:2",
        }]);
        assert_eq!(serde_json::to_value(&requested).unwrap(), expected);
    }

    #[test]
    fn deployment_plan_responses_carry_the_program_sdk_report_when_asked() {
        let mut plan_value = plan_response_snapshot();
        plan_value["programSdks"] = json!([
            {
                "programId": "ore111",
                "programSpecHash": "arete:h1:program-spec:sha256:1",
                "programReleaseHash": "arete:h1:program-release:sha256:1",
                "programPackageRelease": "arete:registry-package-release:v2:sha256:2",
                "package": "ore",
                "version": "1.0.2",
                "source": "requested",
                "reason": null,
            },
            {
                "programId": "entropy111",
                "programSpecHash": "arete:h1:program-spec:sha256:3",
                "programReleaseHash": "arete:h1:program-release:sha256:3",
                "programPackageRelease": null,
                "package": null,
                "version": null,
                "source": null,
                "reason": "no program package is published for this program",
            },
        ]);
        let plan: StackDeploymentPlanResponse = serde_json::from_value(plan_value.clone()).unwrap();
        let report = plan.program_sdks.as_ref().unwrap();
        assert_eq!(report[0].source.as_deref(), Some("requested"));
        assert_eq!(report[1].source, None);
        assert_eq!(serde_json::to_value(plan).unwrap(), plan_value);

        plan_value["programSdks"][0]["private"] = json!(true);
        assert!(serde_json::from_value::<StackDeploymentPlanResponse>(plan_value).is_err());
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
            "serviceClass": "standard",
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
    fn registry_install_without_service_class_defaults_to_standard() {
        let mut value = registry_install_snapshot(1);
        value.as_object_mut().unwrap().remove("serviceClass");
        let response: RegistryStackInstallResponse = serde_json::from_value(value).unwrap();
        assert_eq!(response.service_class, "standard");
    }

    #[test]
    fn registry_list_item_without_service_class_defaults_to_standard() {
        let item: RegistryStackItem = serde_json::from_value(serde_json::json!({
            "name": "legacy-public",
            "description": null,
            "websocket_url": "wss://legacy-public.stack.arete.run",
            "entities": ["Position"],
            "visibility": "public"
        }))
        .expect("legacy registry item");
        assert_eq!(item.service_class, "standard");
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
