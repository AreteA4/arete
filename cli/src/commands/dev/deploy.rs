//! The deploy loop: build with `arete-protocol`, then release, deploy and
//! activate the protocol in the project's Arete cloud dev environment.
//!
//! Every deployment `a4 dev up` creates is a private deployment bound to the
//! environment (`environment` on `POST /api/protocols/:id/deployments`): its
//! chain reads, simulations and sends go to the environment's fork, its
//! stack is decoded by Arete and runs in the environment, and only the
//! account can use it. Going to mainnet is `a4 protocol deploy`, a separate
//! deployment of the same release.
//!
//! Every new deployment gets a fee policy: the `--fee-policy` file (or
//! `[protocol].fee_policy` in `arete.toml`), else a policy that charges
//! nothing. Arete fills in the platform fee destination; an API that has
//! none to fill in gets a key generated for this project under
//! `.arete/dev/fee/` (never a wallet that signs steps). Resources the spec
//! declares come from `--resource` / `[protocol.resources]`, as for
//! `a4 protocol deploy`.
//!
//! A release is content-addressed, so an unchanged build is a no-op. A new
//! release gets a **new deployment** (with its own id and short id);
//! `.arete/dev/session.json` always names the newest one. Once the new
//! deployment is active, the earlier ones are disabled
//! (`POST /api/protocol-deployments/:d/disable`), so at most one is active.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use arete_hash::protocol::{parse_contract_json, validate_protocol_spec_value, ProtocolSpecV1};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::process::Command;

use super::api::DevApi;
use super::runtime::LogSink;

/// The build directory `arete-protocol build` writes, relative to the project.
pub const BUILD_DIR: &str = ".arete/protocol";
/// The protocol entry module.
pub const ENTRY: &str = "protocol.ts";
/// The fee policy schema.
pub const FEE_POLICY_SCHEMA: &str = "arete.protocol-fee-policy/v1";
/// The dev platform fee destination, under `.arete/dev/fee/`.
const FEE_DESTINATION_WALLET: &str = "platform-destination";
/// What an API without a platform fee wallet answers a policy without one.
const DESTINATION_REQUIRED: &str = "protocol-fee-policy-platform-destination-required";

/// What the deploy loop remembers between runs (`.arete/dev/deploy.json`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployState {
    /// The environment the deployments below are bound to. A state for
    /// another environment is started over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_pubkey: Option<String>,
    /// The stored fee policy and the document it was stored from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_policy_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_policy: Option<Value>,
    /// A deployment created for a release that did not activate yet (a
    /// release with handlers waits for a review): the next deploy of the
    /// same release activates it instead of creating another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingDeployment>,
    /// Every deployment created in this environment, oldest first.
    #[serde(default)]
    pub deployments: Vec<String>,
    /// The earlier deployments that were disabled after a newer one became
    /// active.
    #[serde(default)]
    pub disabled: Vec<String>,
}

impl DeployState {
    /// Each deployment with its state: `active` (the current one),
    /// `disabled`, or `enabled` (an earlier one whose disable failed; the
    /// next deploy retries it).
    pub fn deployment_states(&self) -> Vec<(String, &'static str)> {
        self.deployments
            .iter()
            .map(|id| {
                let state = if Some(id) == self.deployment_id.as_ref() {
                    "active"
                } else if self.disabled.contains(id) {
                    "disabled"
                } else {
                    "enabled"
                };
                (id.clone(), state)
            })
            .collect()
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// The state for `environment`: the saved one when it is for that
    /// environment (the protocol id and fee policy carry over to another
    /// one; its deployments don't).
    pub fn load_for(path: &Path, environment: &str) -> Self {
        let state = Self::load(path);
        if state.environment.as_deref() == Some(environment) {
            return state;
        }
        Self {
            environment: Some(environment.to_string()),
            protocol_id: state.protocol_id,
            slug: state.slug,
            fee_policy_id: state.fee_policy_id,
            fee_policy: state.fee_policy,
            ..Self::default()
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        arete_mcp::dev::write_private(path, &serde_json::to_vec_pretty(self)?)
    }
}

/// A created deployment that is not active yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDeployment {
    pub deployment_id: String,
    pub release_hash: String,
    pub short_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_pubkey: Option<String>,
}

/// What an API answers when a release with handlers is not reviewed yet.
const REVIEW_REQUIRED: &str = "protocol-review-required";

/// `localTipJar` → `local-tip-jar`: the protocol slug.
pub fn slug_from_name(name: &str) -> String {
    let mut slug = String::new();
    let mut previous: Option<char> = None;
    for character in name.chars() {
        if character.is_ascii_uppercase()
            && previous.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit())
        {
            slug.push('-');
        }
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
        previous = Some(character);
    }
    let slug = slug.trim_matches('-').to_string();
    slug.chars()
        .take(64)
        .collect::<String>()
        .trim_end_matches('-')
        .to_string()
}

/// What `a4 dev up` takes from flags and the `[protocol]` section of
/// `arete.toml` (shared with `a4 protocol deploy`). Paths are resolved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeploySettings {
    pub slug: Option<String>,
    pub name: Option<String>,
    /// Fee policy document; `None` means a zero-fee policy.
    pub fee_policy: Option<PathBuf>,
    /// Resource name -> JSON file or inline value.
    pub resources: BTreeMap<String, crate::commands::protocol::ResourceSource>,
}

/// The fee policy a dev deployment uses when none is configured: no fees.
/// Arete fills in the platform fee destination.
pub fn default_fee_policy() -> Value {
    json!({
        "schema": FEE_POLICY_SCHEMA,
        "steps": {},
    })
}

fn read_json_document(path: &Path, what: &str) -> Result<Value> {
    let bytes =
        std::fs::read(path).with_context(|| format!("Failed to read {what} {}", path.display()))?;
    parse_contract_json(&bytes)
        .map_err(|error| anyhow::anyhow!("{what} {} is not valid JSON: {error}", path.display()))
}

/// One `arete-protocol build` output.
#[derive(Debug, Clone)]
pub struct BuildOutput {
    pub spec_json: Value,
    pub spec: ProtocolSpecV1,
    pub handlers: Option<Vec<u8>>,
}

/// `arete-protocol build protocol.ts --out .arete/protocol`, with the
/// project's own `arete-protocol` (see [`crate::commands::protocol_bin`]).
pub fn build_command(project_root: &Path) -> Result<Command> {
    let bin = crate::commands::protocol_bin::resolve(project_root)?;
    let mut command = crate::commands::protocol_bin::tokio_command(&bin);
    command
        .args(["build", ENTRY, "--out", BUILD_DIR])
        .current_dir(project_root);
    Ok(command)
}

pub async fn run_build(project_root: &Path, log: &LogSink) -> Result<()> {
    let output = build_command(project_root)?
        .output()
        .await
        .context("Failed to run `arete-protocol build`; is Node.js installed?")?;
    for line in String::from_utf8_lossy(&output.stdout)
        .lines()
        .chain(String::from_utf8_lossy(&output.stderr).lines())
    {
        log.line(line);
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("could not determine executable")
            || stderr.contains("npm error could not")
        {
            bail!(
                "arete-protocol is not installed in this project. Run `npm install --save-dev @usearete/protocol`."
            );
        }
        return Err(BuildFailed {
            diagnostics: build_diagnostics(&stderr),
            output: stderr.trim().to_string(),
        }
        .into());
    }
    Ok(())
}

/// One finding of a failed `arete-protocol build`, located in a source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuildDiagnostic {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// A failed `arete-protocol build`: its output, and the findings in it that
/// name a file and line.
#[derive(Debug)]
pub struct BuildFailed {
    pub output: String,
    pub diagnostics: Vec<BuildDiagnostic>,
}

impl std::fmt::Display for BuildFailed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "`arete-protocol build` failed:\n{}", self.output)
    }
}

impl std::error::Error for BuildFailed {}

/// The `file:line:column: message [code]` findings `arete-protocol build`
/// prints, one per line.
pub fn build_diagnostics(output: &str) -> Vec<BuildDiagnostic> {
    output.lines().filter_map(parse_diagnostic).collect()
}

fn parse_diagnostic(line: &str) -> Option<BuildDiagnostic> {
    let mut parts = line.trim().splitn(4, ':');
    let file = parts.next()?.trim();
    let line_number = parts.next()?.parse().ok()?;
    let column = parts.next()?.parse().ok()?;
    let rest = parts.next()?.strip_prefix(' ')?.trim_end();
    if file.is_empty() || file.contains(' ') || rest.is_empty() {
        return None;
    }
    let (message, code) = match rest
        .strip_suffix(']')
        .and_then(|head| head.rsplit_once(" ["))
    {
        Some((message, code)) if !code.is_empty() && !code.contains(' ') => {
            (message.to_string(), Some(code.to_string()))
        }
        _ => (rest.to_string(), None),
    };
    Some(BuildDiagnostic {
        file: file.to_string(),
        line: line_number,
        column,
        message,
        code,
    })
}

pub fn read_build(build_dir: &Path) -> Result<BuildOutput> {
    let read = |name: &str| -> Result<Vec<u8>> {
        std::fs::read(build_dir.join(name))
            .with_context(|| format!("The build has no {name} in {}", build_dir.display()))
    };
    let spec_json: Value = serde_json::from_slice(&read("protocol-spec.json")?)
        .context("protocol-spec.json is not JSON")?;
    let spec = validate_protocol_spec_value(&spec_json)
        .map_err(|error| anyhow::anyhow!("protocol-spec.json is invalid: {error}"))?;
    let handlers = std::fs::read(build_dir.join("handlers.mjs")).ok();
    Ok(BuildOutput {
        spec_json,
        spec,
        handlers,
    })
}

/// Everything one deploy needs.
pub struct Deployer {
    pub project_root: PathBuf,
    pub api: DevApi,
    /// The environment every deployment is bound to.
    pub environment: String,
    pub settings: DeploySettings,
    pub state_path: PathBuf,
    /// Where a generated platform fee destination key lives.
    pub fee_dir: PathBuf,
    pub log: LogSink,
}

/// What one deploy did.
#[derive(Debug, Clone, PartialEq)]
pub struct DeployOutcome {
    /// Whether a new release and deployment were created.
    pub changed: bool,
    pub release_hash: String,
    pub deployment_id: String,
    pub has_handlers: bool,
    /// Earlier deployments disabled by this deploy.
    pub disabled: Vec<String>,
    /// Earlier deployments that could not be disabled, with the error.
    pub disable_errors: Vec<(String, String)>,
}

impl Deployer {
    pub fn build_dir(&self) -> PathBuf {
        self.project_root.join(BUILD_DIR)
    }

    /// Deploy the build already in `.arete/protocol`.
    pub async fn deploy(&self, state: &mut DeployState) -> Result<DeployOutcome> {
        if state.environment.as_deref() != Some(self.environment.as_str()) {
            *state = DeployState::load_for(&self.state_path, &self.environment);
            state.environment = Some(self.environment.clone());
        }
        let build = read_build(&self.build_dir())?;
        let slug = state
            .slug
            .clone()
            .or_else(|| self.settings.slug.clone())
            .unwrap_or_else(|| slug_from_name(&build.spec.name));
        if slug.is_empty() {
            bail!(
                "The protocol name '{}' gives no usable slug",
                build.spec.name
            );
        }
        let protocol_id = match &state.protocol_id {
            Some(id) => id.clone(),
            None => {
                let name = self
                    .settings
                    .name
                    .clone()
                    .unwrap_or_else(|| build.spec.name.clone());
                let id = self.api.ensure_protocol(&slug, &name).await?;
                state.protocol_id = Some(id.clone());
                state.slug = Some(slug.clone());
                state.save(&self.state_path)?;
                id
            }
        };
        let release = self
            .api
            .upload_release(&protocol_id, &build.spec_json, build.handlers.as_deref())
            .await?;
        let release_hash = release["releaseHash"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let has_handlers = release
            .get("hasHandlers")
            .and_then(Value::as_bool)
            .unwrap_or(build.handlers.is_some());
        if let (Some(current), Some(deployment)) =
            (state.release_hash.clone(), state.deployment_id.clone())
        {
            if current == release_hash {
                let (disabled, disable_errors) = self.disable_previous(state, &deployment).await?;
                return Ok(DeployOutcome {
                    changed: false,
                    release_hash,
                    deployment_id: deployment,
                    has_handlers,
                    disabled,
                    disable_errors,
                });
            }
        }
        self.log.line(&format!(
            "release {release_hash} (templateCheck {}, reviewed {})",
            release["templateCheck"].as_str().unwrap_or("unknown"),
            release["reviewed"]
        ));

        let deployment = match state
            .pending
            .clone()
            .filter(|pending| pending.release_hash == release_hash)
        {
            Some(pending) => {
                self.log.line(&format!(
                    "activating deployment {} created earlier for this release",
                    pending.deployment_id
                ));
                pending
            }
            None => {
                let resources = self.resources(&build, &protocol_id, &release_hash).await?;
                let fee_policy_id = self.fee_policy(state, &protocol_id).await?;
                let created = self
                    .api
                    .create_deployment(
                        &protocol_id,
                        &release_hash,
                        &resources,
                        &fee_policy_id,
                        &self.environment,
                    )
                    .await?;
                self.log.line(&format!(
                    "deployment {} (short id {}) in environment {}",
                    created.deployment_id, created.short_id, self.environment
                ));
                let pending = PendingDeployment {
                    deployment_id: created.deployment_id,
                    release_hash: release_hash.clone(),
                    short_id: created.short_id,
                    attestation_pubkey: created.attestation_pubkey,
                };
                // Remember it before activating, so a failed activation is
                // retried (and the deployment disabled later) rather than
                // forgotten.
                state.deployments.push(pending.deployment_id.clone());
                state.pending = Some(pending.clone());
                state.save(&self.state_path)?;
                pending
            }
        };
        if let Err(error) = self.api.activate(&deployment.deployment_id).await {
            if error.to_string().contains(REVIEW_REQUIRED) {
                return Err(error.context(format!(
                    "Deployment {} of release {release_hash} waits for an Arete review of its handler code (in a dev environment too); `a4 dev up` activates it once reviewed",
                    deployment.deployment_id
                )));
            }
            return Err(error);
        }
        state.pending = None;
        state.release_hash = Some(release_hash.clone());
        state.deployment_id = Some(deployment.deployment_id.clone());
        state.short_id = Some(deployment.short_id);
        state.attestation_pubkey = deployment.attestation_pubkey;
        state.save(&self.state_path)?;
        let (disabled, disable_errors) = self
            .disable_previous(state, &deployment.deployment_id)
            .await?;
        Ok(DeployOutcome {
            changed: true,
            release_hash,
            deployment_id: deployment.deployment_id,
            has_handlers,
            disabled,
            disable_errors,
        })
    }

    /// Disable every earlier deployment that isn't disabled yet, once
    /// `active` is active. A failure is reported, not fatal: the deployment
    /// stays `enabled` in the state and the next deploy tries again.
    async fn disable_previous(
        &self,
        state: &mut DeployState,
        active: &str,
    ) -> Result<(Vec<String>, Vec<(String, String)>)> {
        let pending: Vec<String> = state
            .deployments
            .iter()
            .filter(|id| id.as_str() != active && !state.disabled.contains(id))
            .cloned()
            .collect();
        let mut disabled = Vec::new();
        let mut errors = Vec::new();
        for id in pending {
            match self.api.disable(&id).await {
                Ok(()) => {
                    self.log.line(&format!("disabled earlier deployment {id}"));
                    state.disabled.push(id.clone());
                    disabled.push(id);
                }
                Err(error) => {
                    self.log
                        .line(&format!("could not disable deployment {id}: {error:#}"));
                    errors.push((id, format!("{error:#}")));
                }
            }
        }
        if !disabled.is_empty() {
            state.save(&self.state_path)?;
        }
        Ok((disabled, errors))
    }

    /// The fee policy for a new deployment: stored once per document and
    /// reused while the document is unchanged.
    async fn fee_policy(&self, state: &mut DeployState, protocol_id: &str) -> Result<String> {
        let document = match &self.settings.fee_policy {
            Some(path) => read_json_document(path, "Fee policy")?,
            None => default_fee_policy(),
        };
        if let (Some(id), Some(stored)) = (&state.fee_policy_id, &state.fee_policy) {
            if *stored == document {
                return Ok(id.clone());
            }
        }
        let (id, stored) = match self.api.create_fee_policy(protocol_id, &document).await {
            Ok(id) => (id, document),
            // No platform fee wallet on this API: name the project's own
            // dev destination, as a policy may.
            Err(error)
                if error.to_string().contains(DESTINATION_REQUIRED)
                    && document.get("platformDestination").is_none() =>
            {
                let (wallet, _) = arete_mcp::dev::DevWallet::load_or_create(
                    &self.fee_dir,
                    FEE_DESTINATION_WALLET,
                )?;
                let mut with_destination = document.clone();
                with_destination["platformDestination"] = json!(wallet.pubkey());
                let id = self
                    .api
                    .create_fee_policy(protocol_id, &with_destination)
                    .await?;
                (id, document)
            }
            Err(error) => return Err(error),
        };
        self.log.line(&format!("fee policy {id}"));
        let document = stored;
        state.fee_policy_id = Some(id.clone());
        state.fee_policy = Some(document);
        state.save(&self.state_path)?;
        Ok(id)
    }

    /// Publish a version of every resource the spec declares, for this
    /// release.
    async fn resources(
        &self,
        build: &BuildOutput,
        protocol_id: &str,
        release_hash: &str,
    ) -> Result<BTreeMap<String, i64>> {
        let mut values = BTreeMap::new();
        for name in build.spec.resources.keys() {
            let source = self.settings.resources.get(name).ok_or_else(|| {
                anyhow::anyhow!(crate::commands::protocol::missing_resource_help(name))
            })?;
            values.insert(name.clone(), source.read(name)?);
        }
        let mut versions = BTreeMap::new();
        for (name, value) in &values {
            let version = self
                .api
                .publish_resource(protocol_id, name, value, release_hash)
                .await?;
            self.log.line(&format!("resource {name} v{version}"));
            versions.insert(name.clone(), version);
        }
        Ok(versions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path as AxumPath, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    const ENV: &str = "denv_01hzx3k8m9q2r4s5t6v7w8x9yz";

    #[test]
    fn build_failures_carry_located_findings() {
        let stderr = "warning: something minor\n\
            error: the protocol does not type-check\n  \
            protocol.ts:12:7: unknown account 'vault' [template_unknown_account]\n  \
            handlers/close.ts:3:1: Cannot find name 'x'\n\
            npm error code 1\n";
        assert_eq!(
            build_diagnostics(stderr),
            vec![
                BuildDiagnostic {
                    file: "protocol.ts".into(),
                    line: 12,
                    column: 7,
                    message: "unknown account 'vault'".into(),
                    code: Some("template_unknown_account".into()),
                },
                BuildDiagnostic {
                    file: "handlers/close.ts".into(),
                    line: 3,
                    column: 1,
                    message: "Cannot find name 'x'".into(),
                    code: None,
                },
            ]
        );
        let error: anyhow::Error = BuildFailed {
            output: stderr.trim().into(),
            diagnostics: build_diagnostics(stderr),
        }
        .into();
        assert!(format!("{error:#}").starts_with("`arete-protocol build` failed:\n"));
    }

    #[test]
    fn derives_slugs() {
        assert_eq!(slug_from_name("localTipJar"), "local-tip-jar");
        assert_eq!(slug_from_name("tokenRaise"), "token-raise");
        assert_eq!(
            slug_from_name("escrow_v2 Resolution"),
            "escrow-v2-resolution"
        );
        assert_eq!(slug_from_name("ABC"), "abc");
        assert_eq!(slug_from_name(&"x".repeat(80)).len(), 64);
    }

    #[derive(Default)]
    struct Api {
        log: Mutex<Vec<(String, Value)>>,
        activations: Mutex<u32>,
        /// Refuse a fee policy without a platform destination, as an API
        /// without a platform fee wallet does.
        no_platform_wallet: Mutex<bool>,
        /// Refuse activations with `protocol-review-required` while set.
        unreviewed: Mutex<bool>,
    }

    type Shared = State<Arc<Api>>;

    fn note(api: &Api, route: &str, body: Value) {
        api.log.lock().unwrap().push((route.to_string(), body));
    }

    fn authorized(headers: &HeaderMap) -> bool {
        headers.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer a4_sk_user")
    }

    async fn create_protocol(
        State(api): Shared,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        if !authorized(&headers) {
            return (StatusCode::UNAUTHORIZED, Json(json!({}))).into_response();
        }
        note(&api, "protocols", body);
        (
            StatusCode::CREATED,
            Json(json!({ "id": "p-1", "slug": "local-tip-jar", "name": "Local", "createdAt": "2026-10-13T00:00:00Z" })),
        )
            .into_response()
    }

    async fn release(
        State(api): Shared,
        AxumPath(id): AxumPath<String>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        assert_eq!(id, "p-1");
        let hash = format!(
            "arete:h1:protocol-release:sha256:{}",
            body["spec"]["version"].as_str().unwrap()
        );
        note(&api, "release", body.clone());
        Json(json!({
            "protocolId": "p-1",
            "releaseHash": hash,
            "hasHandlers": body.get("bundleBase64").is_some(),
            "templateCheck": "passed",
            "reviewed": body.get("bundleBase64").is_some(),
            "created": true,
        }))
    }

    async fn deployment(State(api): Shared, Json(body): Json<Value>) -> impl IntoResponse {
        let n = api
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|(r, _)| r == "deployment")
            .count()
            + 1;
        let environment = body["environment"].clone();
        note(&api, "deployment", body);
        (
            StatusCode::CREATED,
            Json(json!({
                "id": format!("d-{n}"),
                "protocolId": "p-1",
                "shortId": format!("dev{n}"),
                "status": "disabled",
                "revision": 1,
                "releaseHash": "arete:h1:protocol-release:sha256:00",
                "accessMode": "private",
                "attestationKeyId": null,
                "trustTier": "unreviewed",
                "runtimeKind": "template",
                "attestationPubkey": null,
                "environmentId": environment,
            })),
        )
    }

    async fn fee_policy(
        State(api): Shared,
        AxumPath(id): AxumPath<String>,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        assert_eq!(id, "p-1");
        let refused = *api.no_platform_wallet.lock().unwrap()
            && body["document"].get("platformDestination").is_none();
        note(&api, "fee-policy", body);
        if refused {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    json!({ "error": "The fee policy must name a platformDestination", "code": "protocol-fee-policy-platform-destination-required" }),
                ),
            );
        }
        (
            StatusCode::CREATED,
            Json(json!({ "id": "f-1", "version": 1, "documentSha256": "00" })),
        )
    }

    async fn activate(State(api): Shared, AxumPath(id): AxumPath<String>) -> impl IntoResponse {
        if *api.unreviewed.lock().unwrap() {
            note(&api, "activate-refused", json!(id));
            return (
                StatusCode::FORBIDDEN,
                Json(
                    json!({ "error": "This release has handler code, which runs only after an operator reviews it", "code": "protocol-review-required" }),
                ),
            );
        }
        let mut count = api.activations.lock().unwrap();
        *count += 1;
        note(&api, "activate", json!(id));
        if *count == 1 {
            return (
                StatusCode::ACCEPTED,
                Json(
                    json!({ "deployment": { "id": id, "status": "activating" }, "pending": "protocol-release-not-loaded" }),
                ),
            );
        }
        (
            StatusCode::OK,
            Json(json!({ "deployment": { "id": id, "status": "active" } })),
        )
    }

    async fn disable(State(api): Shared, AxumPath(id): AxumPath<String>) -> impl IntoResponse {
        note(&api, "disable", json!(id));
        (
            StatusCode::OK,
            Json(json!({ "deployment": { "id": id, "status": "disabled" } })),
        )
    }

    async fn resource_version(State(api): Shared, Json(body): Json<Value>) -> impl IntoResponse {
        note(&api, "resource-version", body);
        (StatusCode::CREATED, Json(json!({ "version": 1 })))
    }

    async fn publish_resource(State(api): Shared, Json(body): Json<Value>) -> impl IntoResponse {
        note(&api, "resource-publish", body);
        (StatusCode::OK, Json(json!({ "version": 1 })))
    }

    async fn serve(api: Arc<Api>) -> String {
        let app = Router::new()
            .route(
                "/api/protocols/:id/resources/:name/versions",
                post(resource_version),
            )
            .route(
                "/api/protocols/:id/resources/:name/versions/:version/publish",
                post(publish_resource),
            )
            .route("/api/protocols", post(create_protocol))
            .route("/api/protocols/:id/releases", post(release))
            .route("/api/protocols/:id/fee-policies", post(fee_policy))
            .route("/api/protocols/:id/deployments", post(deployment))
            .route("/api/protocol-deployments/:id/activate", post(activate))
            .route("/api/protocol-deployments/:id/disable", post(disable))
            .with_state(api);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    /// A build directory with the escrow-accounts vector as the protocol.
    fn write_build(root: &Path, version: &str) {
        let vectors =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test-vectors/protocol-stack");
        let cases: Value =
            serde_json::from_slice(&std::fs::read(vectors.join("vectors.json")).unwrap()).unwrap();
        let case = cases["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "escrow-accounts.interim")
            .unwrap()
            .clone();
        let mut spec = arete_hash::protocol::parse_contract_json(
            &std::fs::read(vectors.join(case["spec"].as_str().unwrap())).unwrap(),
        )
        .unwrap();
        for (key, value) in case["specOverrides"].as_object().unwrap() {
            spec[key] = value.clone();
        }
        spec["version"] = json!(version);
        let build = root.join(BUILD_DIR);
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(
            build.join("protocol-spec.json"),
            serde_json::to_vec(&spec).unwrap(),
        )
        .unwrap();
        std::fs::copy(
            vectors.join("escrow-accounts.interim.protocol-stack-source.json"),
            build.join("protocol-stack-source.json"),
        )
        .unwrap();
        std::fs::write(build.join("handlers.mjs"), b"export const h = 1;").unwrap();
    }

    fn deployer(origin: &str, project: &Path, settings: DeploySettings) -> Deployer {
        Deployer {
            project_root: project.to_path_buf(),
            api: DevApi::new(origin, "a4_sk_user"),
            environment: ENV.into(),
            settings,
            state_path: project.join("deploy.json"),
            fee_dir: project.join("fee"),
            log: LogSink::open(&project.join("a4.log"), "a4", false).unwrap(),
        }
    }

    #[tokio::test]
    async fn deploys_releases_into_the_environment_against_a_mock_api() {
        let api = Arc::new(Api::default());
        let origin = serve(api.clone()).await;
        let project = tempfile::tempdir().unwrap();
        write_build(project.path(), "1.0.0");
        let deployer = deployer(&origin, project.path(), DeploySettings::default());
        let mut state = DeployState::default();

        let first = deployer.deploy(&mut state).await.unwrap();
        assert!(first.changed);
        assert!(first.has_handlers);
        assert_eq!(first.deployment_id, "d-1");
        assert_eq!(state.environment.as_deref(), Some(ENV));
        assert_eq!(state.short_id.as_deref(), Some("dev1"));
        assert_eq!(state.attestation_pubkey, None);

        // Unchanged build: nothing new.
        let again = deployer.deploy(&mut state).await.unwrap();
        assert!(!again.changed);
        assert_eq!(again.deployment_id, "d-1");

        // A new release: a new deployment, the same fee policy.
        write_build(project.path(), "1.0.1");
        let second = deployer.deploy(&mut state).await.unwrap();
        assert!(second.changed);
        assert_eq!(second.deployment_id, "d-2");
        assert_eq!(state.deployments, vec!["d-1", "d-2"]);
        // The new deployment is active, so the earlier one is disabled.
        assert_eq!(second.disabled, vec!["d-1"]);
        assert!(second.disable_errors.is_empty());
        assert_eq!(
            state.deployment_states(),
            vec![
                ("d-1".to_string(), "disabled"),
                ("d-2".to_string(), "active")
            ]
        );
        let third = deployer.deploy(&mut state).await.unwrap();
        assert!(!third.changed && third.disabled.is_empty());
        assert_eq!(state.fee_policy_id.as_deref(), Some("f-1"));
        assert_eq!(
            DeployState::load(&project.path().join("deploy.json")),
            state
        );

        let log = api.log.lock().unwrap();
        let routes: Vec<&str> = log.iter().map(|(route, _)| route.as_str()).collect();
        assert_eq!(
            routes,
            vec![
                "protocols",
                "release",
                "fee-policy",
                "deployment",
                "activate",
                "activate",
                "release",
                "release",
                "deployment",
                "activate",
                "disable",
                "release"
            ]
        );
        assert_eq!(
            log[0].1,
            json!({ "slug": "escrow-resolution", "name": "escrowResolution" })
        );
        assert!(log[1].1["bundleBase64"].is_string());
        // A zero-fee policy; Arete fills in the platform destination.
        assert_eq!(log[2].1["document"], default_fee_policy());
        assert_eq!(
            log[3].1,
            json!({
                "releaseHash": "arete:h1:protocol-release:sha256:1.0.0",
                "resources": {},
                "feePolicyId": "f-1",
                "accessMode": "private",
                "environment": ENV,
            })
        );
        assert_eq!(log[10].1, json!("d-1"));
    }

    #[tokio::test]
    async fn another_environment_starts_the_deployments_over() {
        let api = Arc::new(Api::default());
        let origin = serve(api.clone()).await;
        let project = tempfile::tempdir().unwrap();
        write_build(project.path(), "1.0.0");
        let mut state = DeployState {
            environment: Some("denv_01hzx3k8m9q2r4s5t6v7w8x9y0".into()),
            protocol_id: Some("p-1".into()),
            slug: Some("escrow-resolution".into()),
            release_hash: Some("arete:h1:protocol-release:sha256:1.0.0".into()),
            deployment_id: Some("d-old".into()),
            deployments: vec!["d-old".into()],
            ..DeployState::default()
        };
        state.save(&project.path().join("deploy.json")).unwrap();
        let deployer = deployer(&origin, project.path(), DeploySettings::default());
        let outcome = deployer.deploy(&mut state).await.unwrap();
        assert!(outcome.changed, "a deployment in the new environment");
        assert_eq!(outcome.deployment_id, "d-1");
        assert_eq!(state.deployments, vec!["d-1"]);
        assert!(
            outcome.disabled.is_empty(),
            "the old environment's are left alone"
        );
        let routes: Vec<String> = api
            .log
            .lock()
            .unwrap()
            .iter()
            .map(|(route, _)| route.clone())
            .collect();
        assert!(!routes.contains(&"protocols".to_string()), "{routes:?}");
    }

    #[tokio::test]
    async fn a_configured_fee_policy_and_missing_resources() {
        let api = Arc::new(Api::default());
        let origin = serve(api.clone()).await;
        let project = tempfile::tempdir().unwrap();
        write_build(project.path(), "1.0.0");
        let policy = project.path().join("fee-policy.json");
        let document = json!({
            "schema": FEE_POLICY_SCHEMA,
            "platformDestination": "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf",
            "steps": {}
        });
        std::fs::write(&policy, serde_json::to_vec(&document).unwrap()).unwrap();
        let deployer = deployer(
            &origin,
            project.path(),
            DeploySettings {
                fee_policy: Some(policy.clone()),
                ..DeploySettings::default()
            },
        );
        let mut state = DeployState::default();
        deployer.deploy(&mut state).await.unwrap();
        let fee = api
            .log
            .lock()
            .unwrap()
            .iter()
            .find(|(route, _)| route == "fee-policy")
            .cloned()
            .unwrap();
        assert_eq!(fee.1["document"], document);

        // A declared resource without a value is an error naming the flag.
        let mut build: BuildOutput = read_build(&project.path().join(BUILD_DIR)).unwrap();
        build.spec.resources.insert(
            "limits".into(),
            serde_json::from_value(json!({ "schema": { "type": "string" } })).unwrap(),
        );
        let error = deployer
            .resources(&build, "p-1", "arete:h1:protocol-release:sha256:00")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("--resource limits=<file.json>"), "{error}");
        assert!(error.contains("[protocol.resources]"), "{error}");

        // An inline value from arete.toml is published as is.
        let mut settings = deployer.settings.clone();
        settings.resources.insert(
            "limits".into(),
            crate::commands::protocol::ResourceSource::Inline(json!("tight")),
        );
        let deployer = Deployer {
            settings,
            ..deployer
        };
        deployer
            .resources(&build, "p-1", "arete:h1:protocol-release:sha256:00")
            .await
            .unwrap();
        let draft = api
            .log
            .lock()
            .unwrap()
            .iter()
            .find(|(route, _)| route == "resource-version")
            .cloned()
            .unwrap();
        assert_eq!(draft.1["value"], json!("tight"), "{draft:?}");
    }

    #[tokio::test]
    async fn an_api_without_a_platform_fee_wallet_gets_the_projects_destination() {
        let api = Arc::new(Api::default());
        *api.no_platform_wallet.lock().unwrap() = true;
        let origin = serve(api.clone()).await;
        let project = tempfile::tempdir().unwrap();
        write_build(project.path(), "1.0.0");
        let deployer = deployer(&origin, project.path(), DeploySettings::default());
        let mut state = DeployState::default();
        deployer.deploy(&mut state).await.unwrap();
        let policies: Vec<Value> = api
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|(route, _)| route == "fee-policy")
            .map(|(_, body)| body["document"].clone())
            .collect();
        assert_eq!(policies.len(), 2);
        assert!(policies[0].get("platformDestination").is_none());
        let (wallet, created) = arete_mcp::dev::DevWallet::load_or_create(
            &project.path().join("fee"),
            FEE_DESTINATION_WALLET,
        )
        .unwrap();
        assert!(!created);
        assert_eq!(policies[1]["platformDestination"], wallet.pubkey());
        // The document as configured is what is remembered.
        assert_eq!(state.fee_policy, Some(default_fee_policy()));
    }

    #[tokio::test]
    async fn an_unreviewed_release_is_activated_by_a_later_deploy() {
        let api = Arc::new(Api::default());
        *api.unreviewed.lock().unwrap() = true;
        let origin = serve(api.clone()).await;
        let project = tempfile::tempdir().unwrap();
        write_build(project.path(), "1.0.0");
        let deployer = deployer(&origin, project.path(), DeploySettings::default());
        let mut state = DeployState::default();
        let error = format!("{:#}", deployer.deploy(&mut state).await.unwrap_err());
        assert!(error.contains("waits for an Arete review"), "{error}");
        assert_eq!(state.pending.as_ref().unwrap().deployment_id, "d-1");
        assert_eq!(state.deployment_id, None);

        // Reviewed meanwhile: the same deployment activates, none is created.
        *api.unreviewed.lock().unwrap() = false;
        let outcome = deployer.deploy(&mut state).await.unwrap();
        assert!(outcome.changed);
        assert_eq!(outcome.deployment_id, "d-1");
        assert_eq!(state.pending, None);
        assert_eq!(state.deployments, vec!["d-1"]);
        let created = api
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|(route, _)| route == "deployment")
            .count();
        assert_eq!(created, 1);
    }
}
