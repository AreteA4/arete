use anyhow::Result;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::api_client::{
    ApiClient, BindStackCompositionRequest, BindStackCompositionResponse, BuildStatus,
    BuildStatusResponse, CreateAliasedLiveSpecArtifact, CreateArtifactBuildRequest,
    CreateBuildRequest, CreateBuildResponse, CreateSpecRequest, DeploymentPhase,
    DeploymentResponse, DeploymentStatus, SelectedProgramRelease, Spec, StackDeploymentPlanRequest,
    StackDeploymentPlanResponse, StackDeploymentPreflightRequest, StackDeploymentPreflightResponse,
    StackDeploymentTarget, DEFAULT_DOMAIN_SUFFIX, STACK_DEPLOYMENT_PLAN_REQUEST_SCHEMA,
    STACK_DEPLOYMENT_PLAN_SCHEMA, STACK_DEPLOYMENT_PREFLIGHT_SCHEMA,
};
use crate::commands::public_artifacts::{load_local_artifact_stack, LocalArtifactStack};
use crate::commands::stack::deployment_selection_key;
use crate::config::{resolve_stacks_to_push, AreteConfig, DiscoveredAst};
use crate::telemetry;
use crate::ui;

const STACK_DEPLOYMENT_RESULT_SCHEMA: &str = "arete.stack-deployment-result/v1";

#[derive(Debug)]
enum LocalDeploymentSource {
    Manifest {
        path: PathBuf,
        deployment_name: Option<String>,
    },
    Legacy(DiscoveredAst),
}

impl LocalDeploymentSource {
    fn is_manifest(&self) -> bool {
        matches!(self, Self::Manifest { .. })
    }
}

fn generate_short_uuid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let digest = Sha256::digest(format!("{timestamp}:{}", std::process::id()).as_bytes());
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostedLiveTarget {
    alias: String,
    live_spec_hash: String,
    spec_name: String,
    entity_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostedDeploymentPlan {
    stack_name: String,
    stack_manifest_hash: String,
    branch: Option<String>,
    targets: Vec<HostedLiveTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedDeploymentSelection {
    deployment_plan_id: Option<String>,
    selection_digest: String,
    releases: Vec<SelectedProgramRelease>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DeploymentResultTarget {
    alias: String,
    live_spec_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DeploymentResultRelease {
    program_id: String,
    program_spec_hash: String,
    program_release_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DeployedTargetResult {
    alias: String,
    live_spec_hash: String,
    deployment_id: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StackDeploymentResult {
    schema: &'static str,
    outcome: &'static str,
    persisted: bool,
    stack_manifest_hash: String,
    branch: Option<String>,
    targets: Vec<DeploymentResultTarget>,
    selection_digest: String,
    releases: Vec<DeploymentResultRelease>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deployment_plan_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    composition_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deployments: Option<Vec<DeployedTargetResult>>,
}

impl StackDeploymentResult {
    fn preflight(
        plan: &HostedDeploymentPlan,
        selection: &ValidatedDeploymentSelection,
    ) -> Result<Self> {
        if selection.deployment_plan_id.is_some() {
            anyhow::bail!("Preflight deployment result cannot contain a deployment plan ID");
        }
        Ok(Self::new("preflight", false, plan, selection, None, None))
    }

    fn healthy(
        orchestration: &HostedOrchestration,
        selection: &ValidatedDeploymentSelection,
        response: &BindStackCompositionResponse,
    ) -> Result<Self> {
        validate_composition_response(orchestration, response)?;
        let deployment_plan_id = selection
            .deployment_plan_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Healthy deployment result requires a plan ID"))?;
        if deployment_plan_id != orchestration.deployment_plan_id {
            anyhow::bail!("Healthy deployment result plan ID mismatch");
        }
        uuid::Uuid::parse_str(deployment_plan_id)
            .map_err(|_| anyhow::anyhow!("Healthy deployment result requires a plan UUID"))?;
        if selection.selection_digest != orchestration.selection_digest {
            anyhow::bail!("Healthy deployment result selection digest mismatch");
        }
        if response.composition_id <= 0 {
            anyhow::bail!("Healthy deployment result requires a composition ID");
        }
        if response
            .live_specs
            .iter()
            .any(|binding| binding.deployment_id <= 0)
        {
            anyhow::bail!("Healthy deployment result requires valid deployment IDs");
        }
        let deployments = response
            .live_specs
            .iter()
            .map(|binding| DeployedTargetResult {
                alias: binding.alias.clone(),
                live_spec_hash: binding.live_spec_hash.clone(),
                deployment_id: binding.deployment_id,
            })
            .collect();
        Ok(Self::new(
            "healthy",
            true,
            &orchestration.plan,
            selection,
            Some(response.composition_id),
            Some(deployments),
        ))
    }

    fn new(
        outcome: &'static str,
        persisted: bool,
        plan: &HostedDeploymentPlan,
        selection: &ValidatedDeploymentSelection,
        composition_id: Option<i64>,
        deployments: Option<Vec<DeployedTargetResult>>,
    ) -> Self {
        let targets = plan
            .targets
            .iter()
            .map(|target| DeploymentResultTarget {
                alias: target.alias.clone(),
                live_spec_hash: target.live_spec_hash.clone(),
            })
            .collect();
        let mut releases = selection
            .releases
            .iter()
            .map(|release| DeploymentResultRelease {
                program_id: release.program_id.clone(),
                program_spec_hash: release.program_spec_hash.clone(),
                program_release_hash: release.program_release_hash.clone(),
            })
            .collect::<Vec<_>>();
        releases.sort_by(|left, right| left.program_spec_hash.cmp(&right.program_spec_hash));
        Self {
            schema: STACK_DEPLOYMENT_RESULT_SCHEMA,
            outcome,
            persisted,
            stack_manifest_hash: plan.stack_manifest_hash.clone(),
            branch: plan.branch.clone(),
            targets,
            selection_digest: selection.selection_digest.clone(),
            releases,
            deployment_plan_id: selection.deployment_plan_id.clone(),
            composition_id,
            deployments,
        }
    }
}

trait HostedDeploymentApi {
    fn preflight_stack_deployment(
        &self,
        req: StackDeploymentPreflightRequest,
    ) -> Result<StackDeploymentPreflightResponse>;
    fn create_stack_deployment_plan(
        &self,
        req: StackDeploymentPlanRequest,
    ) -> Result<StackDeploymentPlanResponse>;
    fn get_spec_by_name(&self, name: &str) -> Result<Option<Spec>>;
    fn create_spec(&self, req: CreateSpecRequest) -> Result<Spec>;
    fn create_artifact_build(&self, req: CreateArtifactBuildRequest)
        -> Result<CreateBuildResponse>;
    fn get_build(&self, build_id: i32) -> Result<BuildStatusResponse>;
    fn get_deployment(&self, deployment_id: i32) -> Result<DeploymentResponse>;
    fn list_deployments_page(&self, limit: i64, offset: i64) -> Result<Vec<DeploymentResponse>>;
    fn bind_stack_composition(
        &self,
        req: BindStackCompositionRequest,
    ) -> Result<BindStackCompositionResponse>;
}

impl HostedDeploymentApi for ApiClient {
    fn preflight_stack_deployment(
        &self,
        req: StackDeploymentPreflightRequest,
    ) -> Result<StackDeploymentPreflightResponse> {
        ApiClient::preflight_stack_deployment(self, req)
    }

    fn create_stack_deployment_plan(
        &self,
        req: StackDeploymentPlanRequest,
    ) -> Result<StackDeploymentPlanResponse> {
        ApiClient::create_stack_deployment_plan(self, req)
    }

    fn get_spec_by_name(&self, name: &str) -> Result<Option<Spec>> {
        ApiClient::get_spec_by_name(self, name)
    }

    fn create_spec(&self, req: CreateSpecRequest) -> Result<Spec> {
        ApiClient::create_spec(self, req)
    }

    fn create_artifact_build(
        &self,
        req: CreateArtifactBuildRequest,
    ) -> Result<CreateBuildResponse> {
        ApiClient::create_artifact_build(self, req)
    }

    fn get_build(&self, build_id: i32) -> Result<BuildStatusResponse> {
        ApiClient::get_build(self, build_id)
    }

    fn get_deployment(&self, deployment_id: i32) -> Result<DeploymentResponse> {
        ApiClient::get_deployment(self, deployment_id)
    }

    fn list_deployments_page(&self, limit: i64, offset: i64) -> Result<Vec<DeploymentResponse>> {
        ApiClient::list_deployments_page(self, limit, offset)
    }

    fn bind_stack_composition(
        &self,
        req: BindStackCompositionRequest,
    ) -> Result<BindStackCompositionResponse> {
        ApiClient::bind_stack_composition(self, req)
    }
}

impl HostedDeploymentPlan {
    #[cfg(test)]
    fn from_stack(stack: &LocalArtifactStack, branch: Option<&str>) -> Result<Self> {
        Self::from_stack_with_deployment_name(stack, branch, None)
    }

    fn from_stack_with_deployment_name(
        stack: &LocalArtifactStack,
        branch: Option<&str>,
        deployment_name: Option<&str>,
    ) -> Result<Self> {
        let stack_name = deployment_name
            .unwrap_or(&stack.stack_manifest.payload.name)
            .to_string();
        if stack_name.is_empty() {
            anyhow::bail!("Hosted deployment name must not be empty");
        }
        if stack.live_specs.is_empty() {
            anyhow::bail!(
                "Hosted deployment requires at least one LiveSpec; StackManifest '{}' is program-only. Install its programs through Program Read instead of `a4 up`.",
                stack_name
            );
        }
        let targets = stack
            .live_specs
            .iter()
            .enumerate()
            .map(|(index, (alias, live))| HostedLiveTarget {
                alias: alias.clone(),
                live_spec_hash: live.artifact_hash.to_string(),
                spec_name: child_spec_name(&stack_name, alias, index),
                entity_name: live
                    .payload
                    .entities
                    .first()
                    .map(|entity| entity.state_name.clone())
                    .unwrap_or_else(|| alias.clone()),
            })
            .collect::<Vec<_>>();
        if targets
            .iter()
            .map(|target| target.spec_name.as_str())
            .collect::<BTreeSet<_>>()
            .len()
            != targets.len()
        {
            anyhow::bail!("Hosted child spec naming produced a collision");
        }
        Ok(Self {
            stack_name,
            stack_manifest_hash: stack.stack_manifest.artifact_hash.to_string(),
            branch: branch.map(str::to_string),
            targets,
        })
    }
}

fn child_spec_name(stack_name: &str, alias: &str, position: usize) -> String {
    if position == 0 {
        return stack_name.to_string();
    }
    let mut slug = alias
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "live" } else { slug };
    let slug = slug.chars().take(24).collect::<String>();
    let digest = Sha256::digest(alias.as_bytes());
    let alias_hash = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let suffix = format!("--live-{}-{slug}-{alias_hash}", position + 1);
    let prefix = stack_name
        .chars()
        .take(63usize.saturating_sub(suffix.chars().count()))
        .collect::<String>();
    let mut name = format!("{prefix}{suffix}");
    if name == stack_name {
        let first_len = name.chars().next().map(char::len_utf8).unwrap_or(0);
        let replacement = if stack_name.starts_with('a') {
            "b"
        } else {
            "a"
        };
        name.replace_range(..first_len, replacement);
    }
    name
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompletedHostedTarget {
    alias: String,
    build_id: i32,
    deployment_id: i32,
}

#[derive(Debug, Clone)]
struct HostedOrchestration {
    plan: HostedDeploymentPlan,
    deployment_plan_id: String,
    selection_digest: String,
    completed: Vec<CompletedHostedTarget>,
    failed: bool,
}

impl HostedOrchestration {
    fn new(
        plan: HostedDeploymentPlan,
        deployment_plan_id: String,
        selection_digest: String,
    ) -> Self {
        Self {
            plan,
            deployment_plan_id,
            selection_digest,
            completed: Vec::new(),
            failed: false,
        }
    }

    fn next_target(&self) -> Option<&HostedLiveTarget> {
        (!self.failed)
            .then(|| self.plan.targets.get(self.completed.len()))
            .flatten()
    }

    fn record_success(&mut self, alias: &str, build_id: i32, deployment_id: i32) -> Result<()> {
        let expected = self
            .next_target()
            .ok_or_else(|| anyhow::anyhow!("No hosted target is awaiting completion"))?;
        if expected.alias != alias {
            anyhow::bail!(
                "Hosted target completed out of order: expected '{}', received '{}'",
                expected.alias,
                alias
            );
        }
        if self
            .completed
            .iter()
            .any(|completed| completed.deployment_id == deployment_id)
        {
            anyhow::bail!("Hosted aliases must use independent deployment IDs");
        }
        self.completed.push(CompletedHostedTarget {
            alias: alias.to_string(),
            build_id,
            deployment_id,
        });
        Ok(())
    }

    fn record_failure(&mut self, alias: &str) -> Result<()> {
        let expected = self
            .next_target()
            .ok_or_else(|| anyhow::anyhow!("No hosted target is awaiting completion"))?;
        if expected.alias != alias {
            anyhow::bail!(
                "Hosted target failed out of order: expected '{}', received '{}'",
                expected.alias,
                alias
            );
        }
        self.failed = true;
        Ok(())
    }

    fn composition_request(&self) -> Option<BindStackCompositionRequest> {
        if self.failed || self.completed.len() != self.plan.targets.len() {
            return None;
        }
        Some(BindStackCompositionRequest {
            stack_manifest_hash: self.plan.stack_manifest_hash.clone(),
            deployments: self
                .completed
                .iter()
                .map(|completed| (completed.alias.clone(), completed.deployment_id))
                .collect(),
            deployment_plan_id: self.deployment_plan_id.clone(),
            selection_digest: self.selection_digest.clone(),
            branch: self.plan.branch.clone(),
        })
    }
}

fn artifact_build_request(
    stack: &LocalArtifactStack,
    target: &HostedLiveTarget,
    spec_id: i32,
    branch: Option<&str>,
    deployment_plan_id: &str,
    selection_digest: &str,
) -> CreateArtifactBuildRequest {
    CreateArtifactBuildRequest {
        spec_id,
        program_specs: stack.program_specs.clone(),
        live_specs: stack
            .live_specs
            .iter()
            .map(|(alias, artifact)| CreateAliasedLiveSpecArtifact {
                alias: alias.clone(),
                artifact: artifact.clone(),
            })
            .collect(),
        stack_manifest: stack.stack_manifest.clone(),
        target_live_alias: target.alias.clone(),
        deployment_plan_id: deployment_plan_id.to_string(),
        selection_digest: selection_digest.to_string(),
        branch: branch.map(str::to_string),
    }
}

fn aliased_live_specs(stack: &LocalArtifactStack) -> Vec<CreateAliasedLiveSpecArtifact> {
    stack
        .live_specs
        .iter()
        .map(|(alias, artifact)| CreateAliasedLiveSpecArtifact {
            alias: alias.clone(),
            artifact: artifact.clone(),
        })
        .collect()
}

fn deployment_preflight_request(
    stack: &LocalArtifactStack,
    branch: Option<&str>,
) -> StackDeploymentPreflightRequest {
    StackDeploymentPreflightRequest {
        schema: STACK_DEPLOYMENT_PLAN_REQUEST_SCHEMA.to_string(),
        program_specs: stack.program_specs.clone(),
        live_specs: aliased_live_specs(stack),
        stack_manifest: stack.stack_manifest.clone(),
        branch: branch.map(str::to_string),
    }
}

fn deployment_plan_request(
    stack: &LocalArtifactStack,
    branch: Option<&str>,
    idempotency_key: String,
) -> StackDeploymentPlanRequest {
    StackDeploymentPlanRequest {
        schema: STACK_DEPLOYMENT_PLAN_REQUEST_SCHEMA.to_string(),
        program_specs: stack.program_specs.clone(),
        live_specs: aliased_live_specs(stack),
        stack_manifest: stack.stack_manifest.clone(),
        branch: branch.map(str::to_string),
        idempotency_key,
    }
}

struct SelectionEcho<'a> {
    schema: &'a str,
    expected_schema: &'a str,
    persisted: bool,
    expected_persisted: bool,
    stack_manifest_hash: &'a str,
    branch: Option<&'a str>,
    targets: &'a [StackDeploymentTarget],
    selection_digest: &'a str,
    releases: &'a [SelectedProgramRelease],
}

fn validate_selection_echo(
    plan: &HostedDeploymentPlan,
    stack: &LocalArtifactStack,
    response: SelectionEcho<'_>,
) -> Result<()> {
    if response.schema != response.expected_schema {
        anyhow::bail!(
            "Deployment selection response schema mismatch: expected '{}', received '{}'",
            response.expected_schema,
            response.schema
        );
    }
    if response.persisted != response.expected_persisted {
        anyhow::bail!(
            "Deployment selection response persisted mismatch: expected {}, received {}",
            response.expected_persisted,
            response.persisted
        );
    }
    if response.stack_manifest_hash != plan.stack_manifest_hash {
        anyhow::bail!("Deployment selection response StackManifest hash mismatch");
    }
    if response.branch != plan.branch.as_deref() {
        anyhow::bail!("Deployment selection response branch mismatch");
    }
    let expected_targets = plan
        .targets
        .iter()
        .map(|target| StackDeploymentTarget {
            alias: target.alias.clone(),
            live_spec_hash: target.live_spec_hash.clone(),
        })
        .collect::<Vec<_>>();
    if response.targets != expected_targets {
        anyhow::bail!("Deployment selection response target order or identity mismatch");
    }
    let Some(digest_hex) = response.selection_digest.strip_prefix("sha256:") else {
        anyhow::bail!("Deployment selection response has an invalid selection digest");
    };
    if digest_hex.len() != 64
        || !digest_hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        anyhow::bail!("Deployment selection response has an invalid selection digest");
    }

    let expected_releases = stack
        .program_specs
        .iter()
        .map(|program| {
            (
                program.artifact_hash.to_string(),
                program.payload.program_id.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if response.releases.len() != expected_releases.len() {
        anyhow::bail!("Deployment selection response does not cover every ProgramSpec");
    }
    if response
        .releases
        .windows(2)
        .any(|pair| pair[0].program_spec_hash.as_str() >= pair[1].program_spec_hash.as_str())
    {
        anyhow::bail!("Deployment selection response releases are not strictly ordered");
    }
    for release in response.releases {
        if expected_releases.get(&release.program_spec_hash).copied()
            != Some(release.program_id.as_str())
        {
            anyhow::bail!(
                "Deployment selection response contains an unexpected ProgramSpec release"
            );
        }
        let Some(release_digest) = release
            .program_release_hash
            .strip_prefix("arete:h1:program-release:sha256:")
        else {
            anyhow::bail!("Deployment selection response has an invalid Program Release hash");
        };
        if release_digest.len() != 64
            || !release_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            anyhow::bail!("Deployment selection response has an invalid Program Release hash");
        }
    }
    Ok(())
}

fn validate_preflight_response(
    plan: &HostedDeploymentPlan,
    stack: &LocalArtifactStack,
    response: &StackDeploymentPreflightResponse,
) -> Result<ValidatedDeploymentSelection> {
    validate_selection_echo(
        plan,
        stack,
        SelectionEcho {
            schema: &response.schema,
            expected_schema: STACK_DEPLOYMENT_PREFLIGHT_SCHEMA,
            persisted: response.persisted,
            expected_persisted: false,
            stack_manifest_hash: &response.stack_manifest_hash,
            branch: response.branch.as_deref(),
            targets: &response.targets,
            selection_digest: &response.selection_digest,
            releases: &response.releases,
        },
    )?;
    Ok(ValidatedDeploymentSelection {
        deployment_plan_id: None,
        selection_digest: response.selection_digest.clone(),
        releases: response.releases.clone(),
    })
}

fn validate_plan_response(
    plan: &HostedDeploymentPlan,
    stack: &LocalArtifactStack,
    response: &StackDeploymentPlanResponse,
) -> Result<ValidatedDeploymentSelection> {
    validate_selection_echo(
        plan,
        stack,
        SelectionEcho {
            schema: &response.schema,
            expected_schema: STACK_DEPLOYMENT_PLAN_SCHEMA,
            persisted: response.persisted,
            expected_persisted: true,
            stack_manifest_hash: &response.stack_manifest_hash,
            branch: response.branch.as_deref(),
            targets: &response.targets,
            selection_digest: &response.selection_digest,
            releases: &response.releases,
        },
    )?;
    uuid::Uuid::parse_str(&response.deployment_plan_id)
        .map_err(|_| anyhow::anyhow!("Deployment plan response has an invalid plan UUID"))?;
    let created_at = chrono::DateTime::parse_from_rfc3339(&response.created_at)
        .map_err(|_| anyhow::anyhow!("Deployment plan response has an invalid createdAt"))?;
    let expires_at = chrono::DateTime::parse_from_rfc3339(&response.expires_at)
        .map_err(|_| anyhow::anyhow!("Deployment plan response has an invalid expiresAt"))?;
    if expires_at <= created_at {
        anyhow::bail!("Deployment plan response expiry is not after creation");
    }
    Ok(ValidatedDeploymentSelection {
        deployment_plan_id: Some(response.deployment_plan_id.clone()),
        selection_digest: response.selection_digest.clone(),
        releases: response.releases.clone(),
    })
}

fn print_selection(selection: &ValidatedDeploymentSelection, persisted: bool) {
    println!("  persisted={persisted}");
    if let Some(plan_id) = &selection.deployment_plan_id {
        println!("  Deployment plan: {plan_id}");
    }
    println!("  Selection digest: {}", selection.selection_digest);
    println!("  Selected public releases:");
    if selection.releases.is_empty() {
        println!("    (none)");
    }
    for release in &selection.releases {
        println!(
            "    {} -> {} ({})",
            release.program_id, release.program_release_hash, release.program_spec_hash
        );
    }
}

fn validate_composition_response(
    orchestration: &HostedOrchestration,
    response: &BindStackCompositionResponse,
) -> Result<()> {
    let request = orchestration.composition_request().ok_or_else(|| {
        anyhow::anyhow!("Composition cannot be bound before every target succeeds")
    })?;
    if response.stack_manifest_hash != request.stack_manifest_hash {
        anyhow::bail!("Composition response StackManifest hash mismatch");
    }
    if response.deployment_plan_id != request.deployment_plan_id {
        anyhow::bail!("Composition response deployment plan mismatch");
    }
    if response.selection_digest != request.selection_digest {
        anyhow::bail!("Composition response selection digest mismatch");
    }
    if response.branch != request.branch {
        anyhow::bail!("Composition response branch mismatch");
    }
    if response.live_specs.len() != orchestration.plan.targets.len() {
        anyhow::bail!("Composition response does not cover every manifest alias");
    }
    for (target, binding) in orchestration.plan.targets.iter().zip(&response.live_specs) {
        if binding.alias != target.alias {
            anyhow::bail!("Composition response alias order mismatch");
        }
        if binding.live_spec_hash != target.live_spec_hash {
            anyhow::bail!(
                "Composition response LiveSpec hash mismatch for alias '{}'",
                target.alias
            );
        }
        if request.deployments.get(&target.alias) != Some(&binding.deployment_id) {
            anyhow::bail!(
                "Composition response deployment mismatch for alias '{}'",
                target.alias
            );
        }
    }
    Ok(())
}

pub fn up(
    config_path: &str,
    stack_name: Option<&str>,
    branch: Option<String>,
    preview: bool,
    dry_run: bool,
    local_only: bool,
    json: bool,
) -> Result<()> {
    let start = std::time::Instant::now();

    if local_only && !dry_run {
        anyhow::bail!("--local-only is valid only with --dry-run");
    }
    if json && local_only {
        anyhow::bail!("--json is unavailable with --local-only because the deployment result contract requires a server-validated release selection");
    }
    if json && stack_name.is_some_and(|target| target.ends_with(".stack.json")) {
        anyhow::bail!("--json up requires a manifest-native deployment; legacy composite .stack.json deployments do not define this result contract");
    }

    let config = AreteConfig::load_optional(config_path)?;

    let branch = if preview {
        Some(format!("preview-{}", generate_short_uuid()))
    } else {
        branch
    };

    let sources = resolve_local_deployment_sources(config.as_ref(), stack_name)?;
    if sources.is_empty() {
        anyhow::bail!("No stacks found to deploy");
    }
    if json && (sources.len() != 1 || !sources[0].is_manifest()) {
        anyhow::bail!("--json up requires exactly one manifest-native deployment; legacy composite .stack.json deployments do not define this result contract");
    }

    let has_legacy_sources = sources
        .iter()
        .any(|source| matches!(source, LocalDeploymentSource::Legacy(_)));
    if has_legacy_sources {
        ui::print_warning(
            "Deploying a composite .stack.json is deprecated and supported only through August 31, 2026. Generate a sibling .stack-manifest.json or pass one explicitly.",
        );
    }

    if dry_run {
        let mut legacy = Vec::new();
        for source in &sources {
            match source {
                LocalDeploymentSource::Manifest {
                    path,
                    deployment_name,
                } => {
                    let stack = load_local_artifact_stack(path)?;
                    if local_only {
                        show_local_artifact_dry_run_with_deployment_name(
                            &stack,
                            branch.as_deref(),
                            deployment_name.as_deref(),
                        )?;
                    } else {
                        let client = ApiClient::new()?;
                        let result = dry_run_artifact_stack_with_deployment_name(
                            &client,
                            &stack,
                            branch.as_deref(),
                            deployment_name.as_deref(),
                            json,
                        )?;
                        if json {
                            println!("{}", serde_json::to_string(&result)?);
                        }
                    }
                }
                LocalDeploymentSource::Legacy(ast) => legacy.push(ast.clone()),
            }
        }
        if !legacy.is_empty() {
            show_dry_run(&legacy, branch.as_deref(), local_only)?;
        }
        return Ok(());
    }

    let client = ApiClient::new()?;

    if sources.len() > 1 && stack_name.is_none() {
        println!(
            "{} Found {} stacks. Deploying all...\n",
            ui::symbols::ARROW.blue().bold(),
            sources.len()
        );
    }

    for source in sources {
        let result = match source {
            LocalDeploymentSource::Manifest {
                path,
                deployment_name,
            } => {
                let stack = load_local_artifact_stack(&path)?;
                Some(deploy_artifact_stack_with_deployment_name(
                    &client,
                    stack,
                    branch.as_deref(),
                    deployment_name.as_deref(),
                    json,
                )?)
            }
            LocalDeploymentSource::Legacy(ast) => {
                deploy_single_stack(&client, &ast, branch.as_deref())?;
                None
            }
        };
        if json {
            if let Some(result) = result {
                println!("{}", serde_json::to_string(&result)?);
            }
        } else {
            println!();
        }
    }

    telemetry::record_stack_deployed(stack_name.unwrap_or(""), start.elapsed());

    Ok(())
}

fn resolve_local_deployment_sources(
    config: Option<&AreteConfig>,
    stack_name: Option<&str>,
) -> Result<Vec<LocalDeploymentSource>> {
    if let Some(target) = stack_name.filter(|target| target.ends_with(".stack-manifest.json")) {
        return Ok(vec![LocalDeploymentSource::Manifest {
            path: PathBuf::from(target),
            deployment_name: None,
        }]);
    }

    let force_legacy = stack_name.is_some_and(|target| target.ends_with(".stack.json"));
    resolve_stacks_to_push(config, stack_name)
        .map(|stacks| select_local_deployment_sources(stacks, force_legacy))
}

fn select_local_deployment_sources(
    stacks: Vec<DiscoveredAst>,
    force_legacy: bool,
) -> Vec<LocalDeploymentSource> {
    stacks
        .into_iter()
        .map(|ast| {
            if !force_legacy {
                if let Some(path) = generated_manifest_path(&ast.path).filter(|path| path.is_file())
                {
                    return LocalDeploymentSource::Manifest {
                        path,
                        deployment_name: Some(ast.stack_name),
                    };
                }
            }
            LocalDeploymentSource::Legacy(ast)
        })
        .collect()
}

fn generated_manifest_path(stack_path: &Path) -> Option<PathBuf> {
    let file_name = stack_path.file_name()?.to_str()?;
    let stem = file_name.strip_suffix(".stack.json")?;
    Some(stack_path.with_file_name(format!("{stem}.stack-manifest.json")))
}

#[cfg(test)]
fn dry_run_artifact_stack<A: HostedDeploymentApi + ?Sized>(
    client: &A,
    stack: &LocalArtifactStack,
    branch: Option<&str>,
) -> Result<StackDeploymentResult> {
    dry_run_artifact_stack_with_deployment_name(client, stack, branch, None, false)
}

fn dry_run_artifact_stack_with_deployment_name<A: HostedDeploymentApi + ?Sized>(
    client: &A,
    stack: &LocalArtifactStack,
    branch: Option<&str>,
    deployment_name: Option<&str>,
    quiet: bool,
) -> Result<StackDeploymentResult> {
    let plan =
        HostedDeploymentPlan::from_stack_with_deployment_name(stack, branch, deployment_name)?;
    let response =
        client.preflight_stack_deployment(deployment_preflight_request(stack, branch))?;
    let selection = validate_preflight_response(&plan, stack, &response)?;
    let result = StackDeploymentResult::preflight(&plan, &selection)?;
    if !quiet {
        ui::print_section("Dry Run - No changes will be made");
        println!();
        print_artifact_plan_details(stack, &plan, branch);
        println!();
        print_selection(&selection, false);
        println!();
        println!("{}", "Run without --dry-run to deploy.".dimmed());
    }
    Ok(result)
}

#[cfg(test)]
fn show_local_artifact_dry_run(stack: &LocalArtifactStack, branch: Option<&str>) -> Result<()> {
    show_local_artifact_dry_run_with_deployment_name(stack, branch, None)
}

fn show_local_artifact_dry_run_with_deployment_name(
    stack: &LocalArtifactStack,
    branch: Option<&str>,
    deployment_name: Option<&str>,
) -> Result<()> {
    let plan =
        HostedDeploymentPlan::from_stack_with_deployment_name(stack, branch, deployment_name)?;
    ui::print_section("Dry Run - No changes will be made");
    println!();
    print_artifact_plan_details(stack, &plan, branch);
    println!();
    println!("  local-only: server release/deployability checks were not performed");
    println!("  No deployment plan was persisted.");
    println!();
    println!("{}", "Run without --dry-run to deploy.".dimmed());
    Ok(())
}

fn print_artifact_plan_details(
    stack: &LocalArtifactStack,
    plan: &HostedDeploymentPlan,
    branch: Option<&str>,
) {
    println!(
        "  {} {}",
        ui::symbols::BULLET.dimmed(),
        stack.stack_manifest.payload.name.green().bold()
    );
    println!("    StackManifest: {}", stack.manifest_path.display());
    println!("    StackManifest hash: {}", plan.stack_manifest_hash);
    if plan.stack_name != stack.stack_manifest.payload.name {
        println!("    Deployment name: {}", plan.stack_name);
    }
    if stack.manifest_hash != plan.stack_manifest_hash {
        println!("    Source compatibility hash: {}", stack.manifest_hash);
    }
    println!("    ProgramSpecs: {}", stack.program_specs.len());
    println!("    Child targets: {}", plan.targets.len());
    for (index, target) in plan.targets.iter().enumerate() {
        println!(
            "      {}. {} -> spec '{}' (LiveSpec {})",
            index + 1,
            target.alias,
            target.spec_name,
            target.live_spec_hash
        );
        println!("         targetLiveAlias: {}", target.alias);
    }
    println!(
        "    Final bind: POST /api/deployments/compositions ({})",
        plan.targets
            .iter()
            .map(|target| format!("{}=<deployment-id>", target.alias))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if let Some(branch_name) = branch {
        println!("    Branch: {}", branch_name.cyan());
    }
}

#[cfg(test)]
fn deploy_artifact_stack<A: HostedDeploymentApi + ?Sized>(
    client: &A,
    stack: LocalArtifactStack,
    branch: Option<&str>,
) -> Result<StackDeploymentResult> {
    deploy_artifact_stack_with_deployment_name(client, stack, branch, None, false)
}

fn deploy_artifact_stack_with_deployment_name<A: HostedDeploymentApi + ?Sized>(
    client: &A,
    stack: LocalArtifactStack,
    branch: Option<&str>,
    deployment_name: Option<&str>,
    quiet: bool,
) -> Result<StackDeploymentResult> {
    let plan =
        HostedDeploymentPlan::from_stack_with_deployment_name(&stack, branch, deployment_name)?;
    let idempotency_key = uuid::Uuid::new_v4().to_string();
    let response = client.create_stack_deployment_plan(deployment_plan_request(
        &stack,
        branch,
        idempotency_key,
    ))?;
    let selection = validate_plan_response(&plan, &stack, &response)?;
    let deployment_plan_id = selection
        .deployment_plan_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Persisted deployment plan response omitted its ID"))?;
    let mut orchestration = HostedOrchestration::new(
        plan.clone(),
        deployment_plan_id.clone(),
        selection.selection_digest.clone(),
    );
    if !quiet {
        ui::print_divider();
        println!(
            "{} Deploying {} from StackManifest",
            ui::symbols::ARROW.blue().bold(),
            plan.stack_name.bold()
        );
        ui::print_divider();
        println!("  StackManifest: {}", plan.stack_manifest_hash);
        println!("  ProgramSpecs: {}", stack.program_specs.len());
        print_selection(&selection, true);
        if let Some(branch_name) = branch {
            println!("  Branch: {}", branch_name.cyan());
        }
    }

    for (index, target) in plan.targets.iter().enumerate() {
        if !quiet {
            ui::print_numbered_step(
                (index + 1) as u32,
                &format!("Deploying live alias '{}'...", target.alias),
            );
        }
        let spec_id = if let Some(spec) = client.get_spec_by_name(&target.spec_name)? {
            if !quiet {
                println!(
                    "  {} Reusing exact spec '{}' (id={})",
                    ui::symbols::SUCCESS.green(),
                    target.spec_name,
                    spec.id
                );
            }
            spec.id
        } else {
            let spinner = ui::create_spinner(&format!("Creating spec '{}'...", target.spec_name));
            let request = CreateSpecRequest {
                name: target.spec_name.clone(),
                entity_name: target.entity_name.clone(),
                crate_name: String::new(),
                module_path: String::new(),
                description: None,
                package_name: None,
                output_path: None,
            };
            match client.create_spec(request) {
                Ok(spec) => {
                    spinner.finish_with_message(format!(
                        "{} Spec '{}' created",
                        ui::symbols::SUCCESS.green(),
                        target.spec_name
                    ));
                    spec.id
                }
                Err(create_error) => {
                    let Some(spec) = client.get_spec_by_name(&target.spec_name)? else {
                        spinner.finish_and_clear();
                        return Err(create_error);
                    };
                    spinner.finish_with_message(format!(
                        "{} Reusing concurrently created spec '{}'",
                        ui::symbols::SUCCESS.green(),
                        target.spec_name
                    ));
                    spec.id
                }
            }
        };

        let response = client.create_artifact_build(artifact_build_request(
            &stack,
            target,
            spec_id,
            branch,
            &deployment_plan_id,
            &selection.selection_digest,
        ))?;
        if response.deployment_plan_id.as_deref() != Some(deployment_plan_id.as_str()) {
            anyhow::bail!(
                "Artifact build response deployment plan mismatch for alias '{}'",
                target.alias
            );
        }
        if response.selection_digest.as_deref() != Some(selection.selection_digest.as_str()) {
            anyhow::bail!(
                "Artifact build response selection digest mismatch for alias '{}'",
                target.alias
            );
        }
        if !quiet {
            println!("  Alias: {}", target.alias);
            println!("  LiveSpec: {}", target.live_spec_hash);
            println!("  Build ID: {}", response.build_id.to_string().bold());
            println!();
        }
        let build = match watch_build_progress(client, response.build_id, quiet) {
            Ok(build) => build,
            Err(error) => {
                orchestration.record_failure(&target.alias)?;
                return Err(anyhow::anyhow!(
                    "Hosted alias '{}' did not deploy: {}",
                    target.alias,
                    error
                ));
            }
        };
        let deployment = wait_for_healthy_current_deployment(
            client,
            spec_id,
            response.build_id,
            branch,
            build.related_deployment_id,
        )?;
        orchestration.record_success(&target.alias, response.build_id, deployment.id)?;
        if !quiet {
            println!(
                "  {} Alias '{}' is healthy on deployment {}",
                ui::symbols::SUCCESS.green(),
                target.alias,
                deployment.id
            );
            println!();
        }
    }

    if !quiet {
        ui::print_numbered_step(
            (plan.targets.len() + 1) as u32,
            "Binding stack composition...",
        );
    }
    let request = orchestration.composition_request().ok_or_else(|| {
        anyhow::anyhow!("Not every hosted target completed; composition not bound")
    })?;
    let response = client.bind_stack_composition(request)?;
    let result = StackDeploymentResult::healthy(&orchestration, &selection, &response)?;
    if !quiet {
        ui::print_success("Stack composition bound successfully!");
        println!("  Composition ID: {}", response.composition_id);
        for binding in &response.live_specs {
            println!(
                "  {} {} -> deployment {}",
                ui::symbols::SUCCESS.green(),
                binding.alias,
                binding.deployment_id
            );
            println!("    WebSocket: {}", binding.websocket_endpoint.cyan());
            println!("    Query: {}", binding.query_endpoint.cyan());
        }
    }
    Ok(result)
}

fn show_dry_run(
    stacks: &[crate::config::DiscoveredAst],
    branch: Option<&str>,
    local_only: bool,
) -> Result<()> {
    ui::print_section("Dry Run - No changes will be made");
    println!();

    println!(
        "{} Would deploy {} stack(s):",
        ui::symbols::ARROW.blue().bold(),
        stacks.len()
    );
    println!();

    let client = if local_only {
        None
    } else {
        ApiClient::new().ok()
    };

    for ast in stacks {
        println!(
            "  {} {}",
            ui::symbols::BULLET.dimmed(),
            ast.stack_name.green().bold()
        );
        println!("    Stack: {}", ast.stack_id);
        println!("    Stack: {}", ast.path.display());
        if !ast.program_ids.is_empty() {
            println!("    Program IDs: {}", ast.program_ids.join(", "));
        }

        let url = get_expected_url(&client, &ast.stack_name, branch);
        println!("    URL: {}", url.cyan());
        println!();
    }

    if let Some(branch_name) = branch {
        println!("  Branch: {}", branch_name.cyan());
    }

    if local_only {
        println!("  local-only: server release/deployability checks were not performed");
    }

    println!();
    println!("{}", "Run without --dry-run to deploy.".dimmed());

    Ok(())
}

fn get_expected_url(client: &Option<ApiClient>, stack_name: &str, branch: Option<&str>) -> String {
    let existing_slug = client
        .as_ref()
        .and_then(|c| c.get_spec_by_name(stack_name).ok())
        .flatten()
        .map(|spec| spec.url_slug);

    let name_lower = stack_name.to_lowercase();

    match (existing_slug, branch) {
        (Some(slug), Some(b)) => {
            format!(
                "wss://{}-{}-{}.{}",
                name_lower, slug, b, DEFAULT_DOMAIN_SUFFIX
            )
        }
        (Some(slug), None) => {
            format!("wss://{}-{}.{}", name_lower, slug, DEFAULT_DOMAIN_SUFFIX)
        }
        (None, Some(b)) => {
            format!(
                "wss://{}-<slug>-{}.{} (slug assigned on first deploy)",
                name_lower, b, DEFAULT_DOMAIN_SUFFIX
            )
        }
        (None, None) => {
            format!(
                "wss://{}-<slug>.{} (slug assigned on first deploy)",
                name_lower, DEFAULT_DOMAIN_SUFFIX
            )
        }
    }
}

fn deploy_single_stack(
    client: &ApiClient,
    ast: &crate::config::DiscoveredAst,
    branch: Option<&str>,
) -> Result<()> {
    ui::print_divider();
    if let Some(branch_name) = branch {
        println!(
            "{} Deploying {} (branch: {})",
            ui::symbols::ARROW.blue().bold(),
            ast.stack_name.bold(),
            branch_name.cyan()
        );
    } else {
        println!(
            "{} Deploying {}",
            ui::symbols::ARROW.blue().bold(),
            ast.stack_name.bold()
        );
    }
    ui::print_divider();

    ui::print_numbered_step(1, "Pushing stack...");

    let remote_spec = client.get_spec_by_name(&ast.stack_name)?;

    let spec_id = if let Some(spec) = remote_spec {
        println!(
            "  {} Stack exists (id={})",
            ui::symbols::SUCCESS.green(),
            spec.id
        );
        spec.id
    } else {
        let spinner = ui::create_spinner("Creating stack...");
        let req = crate::api_client::CreateSpecRequest {
            name: ast.stack_name.clone(),
            entity_name: ast.stack_id.clone(),
            crate_name: String::new(),
            module_path: String::new(),
            description: None,
            package_name: None,
            output_path: None,
        };
        let new_spec = client.create_spec(req)?;
        spinner.finish_with_message(format!("{} Stack created", ui::symbols::SUCCESS.green()));
        new_spec.id
    };

    let spinner = ui::create_spinner("Uploading stack...");
    let ast_payload = ast.load_ast()?;
    let version_response = client.create_spec_version(spec_id, ast_payload)?;

    let hash_short = version_response.version.short_hash();
    if version_response.version_is_new {
        spinner.finish_with_message(format!(
            "{} v{} ({})",
            ui::symbols::SUCCESS.green(),
            version_response.version.version_number,
            hash_short
        ));
    } else {
        spinner.finish_with_message(format!(
            "{} v{} (up to date)",
            ui::symbols::EQUALS.blue(),
            version_response.version.version_number
        ));
    }

    ui::print_numbered_step(2, "Creating build...");

    let req = CreateBuildRequest {
        spec_id: Some(spec_id),
        spec_version_id: Some(version_response.version.id),
        ast_payload: None,
        branch: branch.map(|s| s.to_string()),
    };

    let build_response = client.create_build(req)?;
    println!("  Build ID: {}", build_response.build_id.to_string().bold());
    if let Some(branch_name) = branch {
        println!("  Branch: {}", branch_name.cyan());
    }

    ui::print_numbered_step(3, "Building & deploying...");
    println!();

    watch_build_progress(client, build_response.build_id, false)?;

    Ok(())
}

fn wait_for_healthy_current_deployment<A: HostedDeploymentApi + ?Sized>(
    client: &A,
    spec_id: i32,
    build_id: i32,
    branch: Option<&str>,
    mut deployment_id: Option<i32>,
) -> Result<DeploymentResponse> {
    let spinner = ui::create_spinner("Waiting for healthy current deployment...");
    let start_time = std::time::Instant::now();
    let timeout = Duration::from_secs(ui::DEFAULT_POLL_TIMEOUT_SECS);

    loop {
        if start_time.elapsed() > timeout {
            spinner.finish_and_clear();
            anyhow::bail!(
                "Deployment for build {} did not become healthy/current within {} minutes",
                build_id,
                timeout.as_secs() / 60
            );
        }

        let deployment = if let Some(id) = deployment_id {
            Some(client.get_deployment(id)?)
        } else {
            let candidate = find_deployment(client, spec_id, branch)?;
            if let Some(candidate) = &candidate {
                deployment_id = Some(candidate.id);
            }
            candidate
        };

        if let Some(deployment) = deployment {
            if deployment.spec_id != spec_id || deployment.branch.as_deref() != branch {
                spinner.finish_and_clear();
                anyhow::bail!("Build resolved to an unexpected deployment target");
            }
            if deployment.status == DeploymentStatus::Failed
                || deployment.live_status.phase == DeploymentPhase::Degraded
            {
                spinner.finish_and_clear();
                anyhow::bail!("Deployment {} became unhealthy", deployment.id);
            }
            if deployment.current_build_id == Some(build_id)
                && deployment.status == DeploymentStatus::Active
                && deployment.live_status.phase == DeploymentPhase::Running
            {
                spinner.finish_and_clear();
                return Ok(deployment);
            }
        }

        std::thread::sleep(Duration::from_millis(ui::DEFAULT_POLL_INTERVAL_MS));
    }
}

fn find_deployment<A: HostedDeploymentApi + ?Sized>(
    client: &A,
    spec_id: i32,
    branch: Option<&str>,
) -> Result<Option<DeploymentResponse>> {
    const PAGE_SIZE: i64 = 100;
    const MAX_PAGES: i64 = 100;
    let mut selected: Option<DeploymentResponse> = None;
    for page in 0..MAX_PAGES {
        let deployments = client.list_deployments_page(PAGE_SIZE, page * PAGE_SIZE)?;
        for deployment in deployments.iter().filter(|deployment| {
            deployment.spec_id == spec_id && deployment.branch.as_deref() == branch
        }) {
            let should_replace = selected.as_ref().is_none_or(|current| {
                deployment_selection_key(deployment) > deployment_selection_key(current)
            });
            if should_replace {
                selected = Some(deployment.clone());
            }
        }
        if deployments.len() < PAGE_SIZE as usize {
            return Ok(selected);
        }
    }
    anyhow::bail!("Deployment lookup exceeded the bounded pagination limit")
}

fn watch_build_progress<A: HostedDeploymentApi + ?Sized>(
    client: &A,
    build_id: i32,
    quiet: bool,
) -> Result<BuildStatusResponse> {
    let mut last_phase: Option<String> = None;
    let progress_bar = ProgressBar::new(100);
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("  {spinner:.blue} [{bar:30.green/dim}] {pos}% {msg}")
            .expect("Invalid progress bar template")
            .progress_chars("█░░"),
    );
    progress_bar.enable_steady_tick(Duration::from_millis(80));

    let start_time = std::time::Instant::now();
    let timeout = Duration::from_secs(ui::DEFAULT_POLL_TIMEOUT_SECS);

    loop {
        if start_time.elapsed() > timeout {
            progress_bar.finish_and_clear();
            anyhow::bail!(
                "Build timed out after {} minutes. Check build status with: a4 build status {}",
                timeout.as_secs() / 60,
                build_id
            );
        }

        let response = client.get_build(build_id)?;
        let build = &response.build;

        if last_phase != build.phase {
            if let Some(phase) = &build.phase {
                let phase_display = ui::humanize_phase(phase);
                progress_bar.set_message(phase_display.to_string());
            }
            last_phase = build.phase.clone();
        }

        if let Some(progress) = build.progress {
            progress_bar.set_position(progress as u64);
        }

        if build.status.is_terminal() {
            progress_bar.finish_and_clear();

            match build.status {
                BuildStatus::Completed => {
                    if !quiet {
                        println!();
                        ui::print_success("Deployed successfully!");

                        if let Some(ws_url) = &build.websocket_url {
                            println!();
                            println!("  {} {}", "WebSocket:".bold(), ws_url.cyan().bold());
                        }
                    }
                    return Ok(response);
                }
                BuildStatus::Failed => {
                    if !quiet {
                        ui::print_error("Build failed!");

                        if let Some(msg) = &build.status_message {
                            println!("  {}", msg);
                        } else if let Some(category) = &build.error_category {
                            println!("  Error category: {}", category);
                        }
                    }

                    anyhow::bail!("Deployment failed");
                }
                BuildStatus::Cancelled => {
                    if !quiet {
                        ui::print_warning("Build was cancelled.");
                    }
                    anyhow::bail!("Deployment cancelled");
                }
                _ => {}
            }
        }

        std::thread::sleep(Duration::from_millis(ui::DEFAULT_POLL_INTERVAL_MS));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;

    const PLAN_ID: &str = "8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a";

    fn selection_digest() -> String {
        format!("sha256:{}", "a".repeat(64))
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ApiCall {
        Preflight,
        Plan,
        GetSpec,
        Build(String),
        GetBuild,
        GetDeployment,
        Bind,
    }

    struct FakeHostedApi {
        calls: RefCell<Vec<ApiCall>>,
        preflight_requests: RefCell<Vec<StackDeploymentPreflightRequest>>,
        plan_requests: RefCell<Vec<StackDeploymentPlanRequest>>,
        build_requests: RefCell<Vec<CreateArtifactBuildRequest>>,
        bind_requests: RefCell<Vec<BindStackCompositionRequest>>,
        targets: Vec<StackDeploymentTarget>,
        next_spec_id: Cell<i32>,
        malformed_preflight: bool,
        malformed_plan: bool,
        fail_first_build: bool,
        mismatch_build_echo: bool,
    }

    impl FakeHostedApi {
        fn new(stack: &LocalArtifactStack) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                preflight_requests: RefCell::new(Vec::new()),
                plan_requests: RefCell::new(Vec::new()),
                build_requests: RefCell::new(Vec::new()),
                bind_requests: RefCell::new(Vec::new()),
                targets: stack
                    .live_specs
                    .iter()
                    .map(|(alias, live)| StackDeploymentTarget {
                        alias: alias.clone(),
                        live_spec_hash: live.artifact_hash.to_string(),
                    })
                    .collect(),
                next_spec_id: Cell::new(10),
                malformed_preflight: false,
                malformed_plan: false,
                fail_first_build: false,
                mismatch_build_echo: false,
            }
        }

        fn releases(
            program_specs: &[arete_artifacts::ProgramSpecArtifact],
        ) -> Vec<SelectedProgramRelease> {
            let mut releases = program_specs
                .iter()
                .map(|program| SelectedProgramRelease {
                    program_id: program.payload.program_id.clone(),
                    program_spec_hash: program.artifact_hash.to_string(),
                    program_release_hash: format!(
                        "arete:h1:program-release:sha256:{}",
                        Sha256::digest(program.payload.program_id.as_bytes())
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>()
                    ),
                })
                .collect::<Vec<_>>();
            releases.sort_by(|left, right| left.program_spec_hash.cmp(&right.program_spec_hash));
            releases
        }

        fn spec(id: i32, name: &str) -> Spec {
            Spec {
                id,
                user_id: 1,
                name: name.into(),
                entity_name: name.into(),
                crate_name: String::new(),
                module_path: String::new(),
                description: None,
                package_name: None,
                output_path: None,
                url_slug: format!("slug-{id}"),
                created_at: "2026-08-10T12:00:00Z".into(),
                updated_at: "2026-08-10T12:00:00Z".into(),
            }
        }

        fn build_status(build_id: i32, status: BuildStatus) -> BuildStatusResponse {
            BuildStatusResponse {
                build: crate::api_client::Build {
                    id: build_id,
                    spec_id: Some(build_id - 100),
                    spec_version_id: None,
                    portable_ast_hash: None,
                    deployment_release_hash: None,
                    status,
                    error_category: None,
                    status_message: None,
                    phase: Some("completed".into()),
                    progress: Some(100),
                    websocket_url: None,
                    websocket_auth: None,
                    http_auth: None,
                    started_at: Some("2026-08-10T12:00:00Z".into()),
                    completed_at: Some("2026-08-10T12:01:00Z".into()),
                    created_at: "2026-08-10T12:00:00Z".into(),
                },
                events: Vec::new(),
                related_deployment_id: (status == BuildStatus::Completed).then_some(build_id + 900),
                provenance: None,
            }
        }

        fn deployment(deployment_id: i32) -> DeploymentResponse {
            let build_id = deployment_id - 900;
            let spec_id = build_id - 100;
            DeploymentResponse {
                id: deployment_id,
                spec_id,
                spec_name: format!("spec-{spec_id}"),
                atom_name: format!("atom-{spec_id}"),
                branch: None,
                current_build_id: Some(build_id),
                current_spec_version_id: None,
                current_version: None,
                portable_ast_hash: None,
                deployment_release_hash: None,
                current_idl_program_ids: Vec::new(),
                current_image_tag: None,
                websocket_url: format!("wss://{spec_id}.example.test"),
                http_url: format!("https://{spec_id}.example.test"),
                websocket_auth: serde_json::json!({}),
                http_auth: serde_json::json!({}),
                transaction_relay_enabled: false,
                status: DeploymentStatus::Active,
                status_message: None,
                first_deployed_at: Some("2026-08-10T12:00:00Z".into()),
                last_deployed_at: Some("2026-08-10T12:01:00Z".into()),
                live_status: crate::api_client::DeploymentLiveStatus {
                    phase: DeploymentPhase::Running,
                    desired_replicas: Some(1),
                    ready_replicas: Some(1),
                    available_replicas: Some(1),
                    updated_replicas: Some(1),
                    last_transition_time: Some("2026-08-10T12:01:00Z".into()),
                    source: "kubernetes".into(),
                    error_category: None,
                },
                latest_operation: None,
            }
        }
    }

    impl HostedDeploymentApi for FakeHostedApi {
        fn preflight_stack_deployment(
            &self,
            req: StackDeploymentPreflightRequest,
        ) -> Result<StackDeploymentPreflightResponse> {
            self.calls.borrow_mut().push(ApiCall::Preflight);
            self.preflight_requests.borrow_mut().push(req.clone());
            Ok(StackDeploymentPreflightResponse {
                schema: STACK_DEPLOYMENT_PREFLIGHT_SCHEMA.into(),
                persisted: false,
                stack_manifest_hash: if self.malformed_preflight {
                    "wrong-manifest".into()
                } else {
                    req.stack_manifest.artifact_hash.to_string()
                },
                branch: req.branch,
                targets: self.targets.clone(),
                selection_digest: selection_digest(),
                releases: Self::releases(&req.program_specs),
            })
        }

        fn create_stack_deployment_plan(
            &self,
            req: StackDeploymentPlanRequest,
        ) -> Result<StackDeploymentPlanResponse> {
            self.calls.borrow_mut().push(ApiCall::Plan);
            self.plan_requests.borrow_mut().push(req.clone());
            Ok(StackDeploymentPlanResponse {
                schema: STACK_DEPLOYMENT_PLAN_SCHEMA.into(),
                persisted: true,
                deployment_plan_id: PLAN_ID.into(),
                stack_manifest_hash: if self.malformed_plan {
                    "wrong-manifest".into()
                } else {
                    req.stack_manifest.artifact_hash.to_string()
                },
                branch: req.branch,
                targets: self.targets.clone(),
                selection_digest: selection_digest(),
                releases: Self::releases(&req.program_specs),
                created_at: "2026-08-10T12:00:00Z".into(),
                expires_at: "2026-08-10T12:30:00Z".into(),
                idempotent: false,
            })
        }

        fn get_spec_by_name(&self, name: &str) -> Result<Option<Spec>> {
            self.calls.borrow_mut().push(ApiCall::GetSpec);
            let id = self.next_spec_id.get();
            self.next_spec_id.set(id + 1);
            Ok(Some(Self::spec(id, name)))
        }

        fn create_spec(&self, _req: CreateSpecRequest) -> Result<Spec> {
            anyhow::bail!("fake unexpectedly created a spec")
        }

        fn create_artifact_build(
            &self,
            req: CreateArtifactBuildRequest,
        ) -> Result<CreateBuildResponse> {
            self.calls
                .borrow_mut()
                .push(ApiCall::Build(req.target_live_alias.clone()));
            let build_id = req.spec_id + 100;
            let deployment_plan_id = if self.mismatch_build_echo {
                Some(uuid::Uuid::new_v4().to_string())
            } else {
                Some(req.deployment_plan_id.clone())
            };
            let selection_digest = Some(req.selection_digest.clone());
            self.build_requests.borrow_mut().push(req);
            Ok(CreateBuildResponse {
                build_id,
                status: BuildStatus::Pending,
                message: "queued".into(),
                deployment_plan_id,
                selection_digest,
            })
        }

        fn get_build(&self, build_id: i32) -> Result<BuildStatusResponse> {
            self.calls.borrow_mut().push(ApiCall::GetBuild);
            let status = if self.fail_first_build && self.build_requests.borrow().len() == 1 {
                BuildStatus::Failed
            } else {
                BuildStatus::Completed
            };
            Ok(Self::build_status(build_id, status))
        }

        fn get_deployment(&self, deployment_id: i32) -> Result<DeploymentResponse> {
            self.calls.borrow_mut().push(ApiCall::GetDeployment);
            Ok(Self::deployment(deployment_id))
        }

        fn list_deployments_page(
            &self,
            _limit: i64,
            _offset: i64,
        ) -> Result<Vec<DeploymentResponse>> {
            anyhow::bail!("fake unexpectedly listed deployments")
        }

        fn bind_stack_composition(
            &self,
            req: BindStackCompositionRequest,
        ) -> Result<BindStackCompositionResponse> {
            self.calls.borrow_mut().push(ApiCall::Bind);
            self.bind_requests.borrow_mut().push(req.clone());
            Ok(BindStackCompositionResponse {
                composition_id: 77,
                stack_manifest_hash: req.stack_manifest_hash,
                deployment_plan_id: req.deployment_plan_id,
                selection_digest: req.selection_digest,
                branch: req.branch,
                live_specs: self
                    .targets
                    .iter()
                    .map(|target| crate::api_client::CompositionLiveBindingResponse {
                        alias: target.alias.clone(),
                        live_spec_hash: target.live_spec_hash.clone(),
                        deployment_id: req.deployments[&target.alias],
                        websocket_endpoint: format!("wss://{}.example.test", target.alias),
                        query_endpoint: format!("https://{}.example.test", target.alias),
                        websocket_auth_policy: "signed_session".into(),
                        query_auth_policy: "signed_session".into(),
                        observed_generation: 3,
                    })
                    .collect(),
            })
        }
    }

    fn local_stack(aliases: &[&str]) -> LocalArtifactStack {
        let live = arete_artifacts::LiveSpecArtifactV2::new(arete_artifacts::LiveSpecV2::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ))
        .unwrap();
        let live_specs = aliases
            .iter()
            .map(|alias| ((*alias).to_string(), live.clone()))
            .collect::<Vec<_>>();
        let stack_manifest = arete_artifacts::compose_stack_manifest_v2(
            "HostedComposition",
            &[],
            live_specs
                .iter()
                .map(|(alias, live)| (alias.clone(), live))
                .collect(),
            Vec::new(),
        )
        .unwrap();
        LocalArtifactStack {
            manifest_path: PathBuf::from("HostedComposition.stack-manifest.json"),
            manifest_hash: stack_manifest.artifact_hash.to_string(),
            program_specs: Vec::new(),
            live_specs,
            stack_manifest,
        }
    }

    #[test]
    fn implicit_stack_resolution_prefers_generated_sibling_manifest() {
        let directory = std::env::temp_dir().join(format!(
            "arete-up-manifest-resolution-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let ast_path = directory.join("OreStream.stack.json");
        let manifest_path = directory.join("OreStream.stack-manifest.json");
        std::fs::write(&manifest_path, b"{}").unwrap();
        let ast = DiscoveredAst {
            path: ast_path,
            stack_id: "OreStream".into(),
            program_ids: Vec::new(),
            stack_name: "ore".into(),
        };

        let sources = select_local_deployment_sources(vec![ast], false);

        assert_eq!(sources.len(), 1);
        match &sources[0] {
            LocalDeploymentSource::Manifest {
                path,
                deployment_name,
            } => {
                assert_eq!(path, &manifest_path);
                assert_eq!(deployment_name.as_deref(), Some("ore"));
            }
            LocalDeploymentSource::Legacy(_) => panic!("generated manifest was not preferred"),
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn explicit_stack_json_keeps_legacy_escape_hatch() {
        let directory = std::env::temp_dir().join(format!(
            "arete-up-legacy-resolution-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let ast_path = directory.join("OreStream.stack.json");
        std::fs::write(directory.join("OreStream.stack-manifest.json"), b"{}").unwrap();
        let ast = DiscoveredAst {
            path: ast_path,
            stack_id: "OreStream".into(),
            program_ids: Vec::new(),
            stack_name: "ore".into(),
        };

        let sources = select_local_deployment_sources(vec![ast], true);

        assert!(matches!(
            sources.as_slice(),
            [LocalDeploymentSource::Legacy(_)]
        ));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn configured_deployment_name_overrides_manifest_name_without_changing_identity() {
        let stack = local_stack(&["live"]);
        let manifest_hash = stack.stack_manifest.artifact_hash.to_string();

        let plan = HostedDeploymentPlan::from_stack_with_deployment_name(&stack, None, Some("ore"))
            .unwrap();

        assert_eq!(plan.stack_name, "ore");
        assert_eq!(plan.targets[0].spec_name, "ore");
        assert_eq!(plan.stack_manifest_hash, manifest_hash);
    }

    fn completed_orchestration(stack: &LocalArtifactStack) -> HostedOrchestration {
        let plan = HostedDeploymentPlan::from_stack(stack, Some("preview")).unwrap();
        let mut orchestration = HostedOrchestration::new(
            plan,
            "8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a".into(),
            format!("sha256:{}", "a".repeat(64)),
        );
        for index in 0..orchestration.plan.targets.len() {
            let alias = orchestration.plan.targets[index].alias.clone();
            orchestration
                .record_success(&alias, 200 + index as i32, 100 + index as i32)
                .unwrap();
        }
        orchestration
    }

    fn bind_response(orchestration: &HostedOrchestration) -> BindStackCompositionResponse {
        BindStackCompositionResponse {
            composition_id: 77,
            stack_manifest_hash: orchestration.plan.stack_manifest_hash.clone(),
            deployment_plan_id: orchestration.deployment_plan_id.clone(),
            selection_digest: orchestration.selection_digest.clone(),
            branch: orchestration.plan.branch.clone(),
            live_specs: orchestration
                .plan
                .targets
                .iter()
                .enumerate()
                .map(
                    |(index, target)| crate::api_client::CompositionLiveBindingResponse {
                        alias: target.alias.clone(),
                        live_spec_hash: target.live_spec_hash.clone(),
                        deployment_id: 100 + index as i32,
                        websocket_endpoint: format!("wss://{}.example.test", target.alias),
                        query_endpoint: format!("https://{}.example.test", target.alias),
                        websocket_auth_policy: "signed_session".into(),
                        query_auth_policy: "signed_session".into(),
                        observed_generation: 3,
                    },
                )
                .collect(),
        }
    }

    fn contract_hash(kind: &str, digit: char) -> String {
        format!("arete:h1:{kind}:sha256:{}", digit.to_string().repeat(64))
    }

    fn contract_plan(aliases: &[(&str, char)]) -> HostedDeploymentPlan {
        HostedDeploymentPlan {
            stack_name: "ContractStack".into(),
            stack_manifest_hash: contract_hash("stack-manifest", '1'),
            branch: None,
            targets: aliases
                .iter()
                .map(|(alias, digit)| HostedLiveTarget {
                    alias: (*alias).into(),
                    live_spec_hash: contract_hash("live-spec", *digit),
                    spec_name: format!("ContractStack-{alias}"),
                    entity_name: (*alias).into(),
                })
                .collect(),
        }
    }

    fn contract_selection(plan_id: Option<&str>) -> ValidatedDeploymentSelection {
        ValidatedDeploymentSelection {
            deployment_plan_id: plan_id.map(str::to_string),
            selection_digest: selection_digest(),
            releases: vec![SelectedProgramRelease {
                program_id: "Ore111111111111111111111111111111111111111".into(),
                program_spec_hash: contract_hash("program-spec", '3'),
                program_release_hash: contract_hash("program-release", '4'),
            }],
        }
    }

    #[test]
    fn preflight_result_has_an_exact_one_target_json_contract() {
        let plan = contract_plan(&[("live", '2')]);
        let result = StackDeploymentResult::preflight(&plan, &contract_selection(None)).unwrap();
        let serialized = serde_json::to_string(&result).unwrap();

        assert_eq!(
            serialized,
            format!(
                "{{\"schema\":\"arete.stack-deployment-result/v1\",\"outcome\":\"preflight\",\"persisted\":false,\"stackManifestHash\":\"{}\",\"branch\":null,\"targets\":[{{\"alias\":\"live\",\"liveSpecHash\":\"{}\"}}],\"selectionDigest\":\"{}\",\"releases\":[{{\"programId\":\"Ore111111111111111111111111111111111111111\",\"programSpecHash\":\"{}\",\"programReleaseHash\":\"{}\"}}]}}",
                contract_hash("stack-manifest", '1'),
                contract_hash("live-spec", '2'),
                selection_digest(),
                contract_hash("program-spec", '3'),
                contract_hash("program-release", '4'),
            )
        );
        for forbidden in [
            "deploymentPlanId",
            "compositionId",
            "deployments",
            "buildId",
            "websocketEndpoint",
            "queryEndpoint",
            "auth",
            "createdAt",
            "expiresAt",
        ] {
            assert!(!serialized.contains(forbidden));
        }
    }

    #[test]
    fn healthy_result_has_an_exact_one_target_json_contract() {
        let plan = contract_plan(&[("live", '2')]);
        let selection = contract_selection(Some(PLAN_ID));
        let mut orchestration =
            HostedOrchestration::new(plan, PLAN_ID.into(), selection.selection_digest.clone());
        orchestration.record_success("live", 9, 42).unwrap();
        let response = BindStackCompositionResponse {
            composition_id: 77,
            stack_manifest_hash: orchestration.plan.stack_manifest_hash.clone(),
            deployment_plan_id: PLAN_ID.into(),
            selection_digest: selection.selection_digest.clone(),
            branch: None,
            live_specs: vec![crate::api_client::CompositionLiveBindingResponse {
                alias: "live".into(),
                live_spec_hash: contract_hash("live-spec", '2'),
                deployment_id: 42,
                websocket_endpoint: "wss://private.example.test".into(),
                query_endpoint: "https://private.example.test".into(),
                websocket_auth_policy: "private-policy".into(),
                query_auth_policy: "private-policy".into(),
                observed_generation: 99,
            }],
        };
        validate_composition_response(&orchestration, &response).unwrap();
        let result = StackDeploymentResult::healthy(&orchestration, &selection, &response).unwrap();
        let serialized = serde_json::to_string(&result).unwrap();

        assert_eq!(
            serialized,
            format!(
                "{{\"schema\":\"arete.stack-deployment-result/v1\",\"outcome\":\"healthy\",\"persisted\":true,\"stackManifestHash\":\"{}\",\"branch\":null,\"targets\":[{{\"alias\":\"live\",\"liveSpecHash\":\"{}\"}}],\"selectionDigest\":\"{}\",\"releases\":[{{\"programId\":\"Ore111111111111111111111111111111111111111\",\"programSpecHash\":\"{}\",\"programReleaseHash\":\"{}\"}}],\"deploymentPlanId\":\"{}\",\"compositionId\":77,\"deployments\":[{{\"alias\":\"live\",\"liveSpecHash\":\"{}\",\"deploymentId\":42}}]}}",
                contract_hash("stack-manifest", '1'),
                contract_hash("live-spec", '2'),
                selection_digest(),
                contract_hash("program-spec", '3'),
                contract_hash("program-release", '4'),
                PLAN_ID,
                contract_hash("live-spec", '2'),
            )
        );
        for forbidden in [
            "buildId",
            "websocket",
            "queryEndpoint",
            "auth",
            "observedGeneration",
            "specName",
        ] {
            assert!(!serialized.contains(forbidden));
        }
    }

    #[test]
    fn result_preserves_target_order_and_sorts_releases() {
        let plan = contract_plan(&[("second", '2'), ("first", '5')]);
        let mut selection = contract_selection(None);
        selection.releases = vec![
            SelectedProgramRelease {
                program_id: "later".into(),
                program_spec_hash: contract_hash("program-spec", '9'),
                program_release_hash: contract_hash("program-release", '8'),
            },
            SelectedProgramRelease {
                program_id: "earlier".into(),
                program_spec_hash: contract_hash("program-spec", '3'),
                program_release_hash: contract_hash("program-release", '4'),
            },
        ];

        let result = StackDeploymentResult::preflight(&plan, &selection).unwrap();

        assert_eq!(
            result
                .targets
                .iter()
                .map(|target| target.alias.as_str())
                .collect::<Vec<_>>(),
            vec!["second", "first"]
        );
        assert_eq!(
            result
                .releases
                .iter()
                .map(|release| release.program_id.as_str())
                .collect::<Vec<_>>(),
            vec!["earlier", "later"]
        );
    }

    #[test]
    fn result_constructors_enforce_persistence_invariants() {
        let plan = contract_plan(&[("live", '2')]);
        assert!(
            StackDeploymentResult::preflight(&plan, &contract_selection(Some(PLAN_ID))).is_err()
        );

        let selection = contract_selection(None);
        let orchestration =
            HostedOrchestration::new(plan, PLAN_ID.into(), selection.selection_digest.clone());
        let response = BindStackCompositionResponse {
            composition_id: 77,
            stack_manifest_hash: orchestration.plan.stack_manifest_hash.clone(),
            deployment_plan_id: PLAN_ID.into(),
            selection_digest: selection.selection_digest.clone(),
            branch: None,
            live_specs: Vec::new(),
        };
        assert!(StackDeploymentResult::healthy(&orchestration, &selection, &response).is_err());
    }

    #[test]
    fn multi_child_deployment_creates_one_plan_and_reuses_its_identity_everywhere() {
        let stack = local_stack(&["primary", "replica", "archive"]);
        let api = FakeHostedApi::new(&stack);

        deploy_artifact_stack(&api, stack, None).unwrap();

        let calls = api.calls.borrow();
        assert_eq!(
            calls.iter().filter(|call| **call == ApiCall::Plan).count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| **call == ApiCall::Preflight)
                .count(),
            0
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, ApiCall::Build(_)))
                .count(),
            3
        );
        assert_eq!(
            calls.iter().filter(|call| **call == ApiCall::Bind).count(),
            1
        );
        assert_eq!(calls.first(), Some(&ApiCall::Plan));
        drop(calls);

        let plan_requests = api.plan_requests.borrow();
        assert_eq!(plan_requests.len(), 1);
        assert_eq!(
            plan_requests[0].schema,
            STACK_DEPLOYMENT_PLAN_REQUEST_SCHEMA
        );
        uuid::Uuid::parse_str(&plan_requests[0].idempotency_key).unwrap();
        drop(plan_requests);

        let builds = api.build_requests.borrow();
        assert_eq!(builds.len(), 3);
        for (index, build) in builds.iter().enumerate() {
            assert_eq!(build.deployment_plan_id, PLAN_ID);
            assert_eq!(build.selection_digest, selection_digest());
            assert_eq!(
                build.target_live_alias,
                ["primary", "replica", "archive"][index]
            );
            assert_eq!(
                build
                    .live_specs
                    .iter()
                    .map(|live| live.alias.as_str())
                    .collect::<Vec<_>>(),
                vec!["primary", "replica", "archive"]
            );
            assert!(build.live_specs.iter().all(|live| {
                live.artifact.artifact_hash == build.live_specs[0].artifact.artifact_hash
            }));
        }
        drop(builds);

        let binds = api.bind_requests.borrow();
        assert_eq!(binds.len(), 1);
        assert_eq!(binds[0].deployment_plan_id, PLAN_ID);
        assert_eq!(binds[0].selection_digest, selection_digest());
        assert_eq!(binds[0].deployments.len(), 3);
    }

    #[test]
    fn server_dry_run_calls_preflight_once_and_never_mutates() {
        let stack = local_stack(&["first", "second", "third"]);
        let api = FakeHostedApi::new(&stack);

        dry_run_artifact_stack(&api, &stack, None).unwrap();

        assert_eq!(api.calls.borrow().as_slice(), &[ApiCall::Preflight]);
        assert_eq!(api.preflight_requests.borrow().len(), 1);
        assert!(api.plan_requests.borrow().is_empty());
        assert!(api.build_requests.borrow().is_empty());
        assert!(api.bind_requests.borrow().is_empty());
    }

    #[test]
    fn quiet_dry_run_returns_the_same_preflight_result() {
        let stack = local_stack(&["first"]);
        let api = FakeHostedApi::new(&stack);

        let result =
            dry_run_artifact_stack_with_deployment_name(&api, &stack, None, None, true).unwrap();

        assert_eq!(result.schema, STACK_DEPLOYMENT_RESULT_SCHEMA);
        assert_eq!(api.calls.borrow().as_slice(), &[ApiCall::Preflight]);
        assert!(api.plan_requests.borrow().is_empty());
        assert!(api.build_requests.borrow().is_empty());
        assert!(api.bind_requests.borrow().is_empty());
    }

    #[test]
    fn quiet_deployment_still_binds_and_returns_a_healthy_result() {
        let stack = local_stack(&["primary", "replica"]);
        let api = FakeHostedApi::new(&stack);

        let result =
            deploy_artifact_stack_with_deployment_name(&api, stack, None, None, true).unwrap();

        assert_eq!(result.schema, STACK_DEPLOYMENT_RESULT_SCHEMA);
        let calls = api.calls.borrow();
        assert_eq!(
            calls.iter().filter(|call| **call == ApiCall::Plan).count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, ApiCall::Build(_)))
                .count(),
            2
        );
        assert_eq!(
            calls.iter().filter(|call| **call == ApiCall::Bind).count(),
            1
        );
    }

    #[test]
    fn malformed_preflight_echo_fails_before_a_result_exists() {
        let stack = local_stack(&["first"]);
        let mut api = FakeHostedApi::new(&stack);
        api.malformed_preflight = true;

        let error = dry_run_artifact_stack(&api, &stack, None).unwrap_err();

        assert!(error.to_string().contains("StackManifest hash mismatch"));
        assert_eq!(api.calls.borrow().as_slice(), &[ApiCall::Preflight]);
        assert!(api.plan_requests.borrow().is_empty());
        assert!(api.build_requests.borrow().is_empty());
        assert!(api.bind_requests.borrow().is_empty());
    }

    #[test]
    fn json_rejects_legacy_and_unvalidated_local_only_deployments() {
        let legacy = up(
            "missing-arete.toml",
            Some("Legacy.stack.json"),
            None,
            false,
            true,
            false,
            true,
        )
        .unwrap_err();
        assert!(legacy.to_string().contains("legacy composite .stack.json"));

        let local_only = up(
            "missing-arete.toml",
            Some("Stack.stack-manifest.json"),
            None,
            false,
            true,
            true,
            true,
        )
        .unwrap_err();
        assert!(local_only.to_string().contains("server-validated"));
    }

    #[test]
    fn local_only_dry_run_performs_no_http_operations() {
        let stack = local_stack(&["first", "second", "third"]);
        let api = FakeHostedApi::new(&stack);

        show_local_artifact_dry_run(&stack, None).unwrap();

        assert!(api.calls.borrow().is_empty());
    }

    #[test]
    fn malformed_plan_echo_stops_before_specs_builds_and_composition() {
        let stack = local_stack(&["first", "second", "third"]);
        let mut api = FakeHostedApi::new(&stack);
        api.malformed_plan = true;

        let error = deploy_artifact_stack(&api, stack, None).unwrap_err();

        assert!(error.to_string().contains("StackManifest hash mismatch"));
        assert_eq!(api.calls.borrow().as_slice(), &[ApiCall::Plan]);
        assert!(api.build_requests.borrow().is_empty());
        assert!(api.bind_requests.borrow().is_empty());
    }

    #[test]
    fn failed_first_child_stops_before_later_builds_and_composition() {
        let stack = local_stack(&["first", "second", "third"]);
        let mut api = FakeHostedApi::new(&stack);
        api.fail_first_build = true;

        let error = deploy_artifact_stack(&api, stack, None).unwrap_err();

        assert!(error.to_string().contains("did not deploy"));
        assert_eq!(api.build_requests.borrow().len(), 1);
        assert!(api.bind_requests.borrow().is_empty());
    }

    #[test]
    fn child_plan_echo_mismatch_is_rejected_before_watching_or_binding() {
        let stack = local_stack(&["first", "second"]);
        let mut api = FakeHostedApi::new(&stack);
        api.mismatch_build_echo = true;

        let error = deploy_artifact_stack(&api, stack, None).unwrap_err();

        assert!(error.to_string().contains("deployment plan mismatch"));
        assert_eq!(api.build_requests.borrow().len(), 1);
        assert!(!api
            .calls
            .borrow()
            .iter()
            .any(|call| matches!(call, ApiCall::GetBuild | ApiCall::Bind)));
    }

    #[test]
    fn three_alias_plan_has_stable_independent_targets_and_full_build_requests() {
        let stack = local_stack(&["first", "second-value", "third_value"]);
        let first = HostedDeploymentPlan::from_stack(&stack, None).unwrap();
        let second = HostedDeploymentPlan::from_stack(&stack, None).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.targets[0].spec_name, "HostedComposition");
        assert_ne!(first.targets[1].spec_name, first.targets[2].spec_name);
        assert_eq!(
            first
                .targets
                .iter()
                .map(|target| target.spec_name.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );

        for (index, target) in first.targets.iter().enumerate() {
            let request = artifact_build_request(
                &stack,
                target,
                10 + index as i32,
                None,
                "8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a",
                &format!("sha256:{}", "a".repeat(64)),
            );
            assert_eq!(request.target_live_alias, target.alias);
            assert_eq!(request.live_specs.len(), 3);
            assert_eq!(
                request
                    .live_specs
                    .iter()
                    .map(|live| live.alias.as_str())
                    .collect::<Vec<_>>(),
                vec!["first", "second-value", "third_value"]
            );
        }
    }

    #[test]
    fn partial_failure_never_produces_a_bind_request() {
        let stack = local_stack(&["first", "second", "third"]);
        let plan = HostedDeploymentPlan::from_stack(&stack, None).unwrap();
        let mut orchestration = HostedOrchestration::new(
            plan,
            "8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a".into(),
            format!("sha256:{}", "a".repeat(64)),
        );
        orchestration.record_success("first", 1, 11).unwrap();
        orchestration.record_failure("second").unwrap();
        assert!(orchestration.composition_request().is_none());
        assert!(orchestration.next_target().is_none());
    }

    #[test]
    fn composition_response_rejects_alias_hash_deployment_and_order_mismatches() {
        let stack = local_stack(&["first", "second", "third"]);
        let orchestration = completed_orchestration(&stack);
        let valid = bind_response(&orchestration);
        validate_composition_response(&orchestration, &valid).unwrap();

        let mut alias = valid.clone();
        alias.live_specs[0].alias = "other".into();
        assert!(validate_composition_response(&orchestration, &alias).is_err());

        let mut hash = valid.clone();
        hash.live_specs[1].live_spec_hash = "other-hash".into();
        assert!(validate_composition_response(&orchestration, &hash).is_err());

        let mut deployment = valid.clone();
        deployment.live_specs[2].deployment_id = 999;
        assert!(validate_composition_response(&orchestration, &deployment).is_err());

        let mut order = valid;
        order.live_specs.swap(0, 1);
        assert!(validate_composition_response(&orchestration, &order).is_err());

        let valid = bind_response(&orchestration);
        let mut plan_id = valid.clone();
        plan_id.deployment_plan_id = uuid::Uuid::new_v4().to_string();
        assert!(validate_composition_response(&orchestration, &plan_id).is_err());

        let mut digest = valid;
        digest.selection_digest = format!("sha256:{}", "b".repeat(64));
        assert!(validate_composition_response(&orchestration, &digest).is_err());
    }

    #[test]
    fn repeated_live_hashes_still_get_independent_alias_targets() {
        let stack = local_stack(&["primary", "replica", "archive"]);
        let plan = HostedDeploymentPlan::from_stack(&stack, None).unwrap();
        assert!(plan
            .targets
            .iter()
            .all(|target| target.live_spec_hash == plan.targets[0].live_spec_hash));
        assert_eq!(
            plan.targets
                .iter()
                .map(|target| target.spec_name.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
        let orchestration = completed_orchestration(&stack);
        assert_eq!(
            orchestration
                .composition_request()
                .unwrap()
                .deployments
                .len(),
            3
        );
    }

    #[test]
    fn single_live_keeps_manifest_name_anchor_and_compatibility_shape() {
        let stack = local_stack(&[arete_artifacts::DEFAULT_LIVE_ALIAS]);
        let plan = HostedDeploymentPlan::from_stack(&stack, None).unwrap();
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].spec_name, "HostedComposition");
        let request = artifact_build_request(
            &stack,
            &plan.targets[0],
            9,
            None,
            "8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a",
            &format!("sha256:{}", "a".repeat(64)),
        );
        assert_eq!(request.live_specs.len(), 1);
        assert_eq!(
            request.target_live_alias,
            arete_artifacts::DEFAULT_LIVE_ALIAS
        );
    }

    #[test]
    fn program_only_hosted_plan_is_rejected() {
        let mut stack = local_stack(&["live"]);
        stack.live_specs.clear();
        let error = HostedDeploymentPlan::from_stack(&stack, None).unwrap_err();
        assert!(error.to_string().contains("program-only"));
        assert!(error.to_string().contains("Program Read"));
    }
}
