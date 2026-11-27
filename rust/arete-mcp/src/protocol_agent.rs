//! Protocol tools for agents: what a wallet can do, prepare, submit, and
//! (with an opt-in signer) execute.
//!
//! Every tool targets one protocol API: Arete's (the default; the origin is
//! `ARETE_API_URL` or `https://api.arete.run`, and the API key resolves the
//! way `connect` resolves it, never from tool arguments), for a deployment
//! on mainnet or in the project's Arete cloud dev environment (`dev: true`
//! or `project`).
//!
//! A deployment is named directly (`deployment`), through a protocol package
//! reference (`protocol`, resolved to the deployment its install descriptor
//! names, as generated clients do), or, in a dev session, defaults to the
//! session's deployment. Instances and executions the server has seen are
//! remembered, so `protocol_instance`, `protocol_submit` and
//! `protocol_intent` need only their ids afterwards.
//!
//! Signing stays outside the server unless a [`LocalSigner`] is configured.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use dashmap::DashMap;
use serde_json::{json, Value};

use crate::credentials;
use crate::dev::{DevClient, DevProject};
use crate::prepared_summary::{self, decode_transaction, Rpc};
use crate::protocol_api::{
    PrepareRequest, Prepared, ProtocolApi, SubmitUnknown, TRANSACTION_SCOPES,
};
use crate::registry::RegistryClient;
use crate::signer::{LocalSigner, Refusal};

const DEFAULT_API_ORIGIN: &str = "https://api.arete.run";
const ENV_API_URL: &str = "ARETE_API_URL";
/// An RPC URL used to simulate prepared transactions against Arete.
pub const ENV_RPC_URL: &str = "ARETE_MCP_RPC_URL";
/// How long `protocol_submit` and `protocol_execute` wait by default.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(60);
/// The longest wait a caller may ask for.
pub const MAX_WAIT: Duration = Duration::from_secs(120);
/// How long one request to the protocol API or an RPC may take.
pub const API_TIMEOUT: Duration = Duration::from_secs(60);
/// How long simulating a prepared step may take as a whole, however many
/// transactions it has: past it the rest are summarized unsimulated.
pub const SIMULATION_BUDGET: Duration = Duration::from_secs(60);

/// An argument the caller must fix.
#[derive(Debug)]
pub struct InvalidArgs(pub String);

impl std::fmt::Display for InvalidArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvalidArgs {}

fn invalid(message: impl Into<String>) -> anyhow::Error {
    anyhow!(InvalidArgs(message.into()))
}

/// Where a tool call goes and which deployment it names.
#[derive(Debug, Clone, Default)]
pub struct TargetArgs {
    /// Use the `a4 dev` session found from the working directory.
    pub dev: bool,
    /// Use the `a4 dev` session of this project directory.
    pub project: Option<String>,
    pub deployment: Option<String>,
    /// A protocol package reference, resolved to its default deployment.
    pub protocol: Option<String>,
}

/// What the server remembers about an instance or an execution.
#[derive(Debug, Clone, Default)]
struct Remembered {
    dev_project: Option<PathBuf>,
    deployment: String,
    step: Option<String>,
    instance: Option<String>,
    wallet: Option<String>,
    /// Whether this server has sent the execution's submit.
    sent: bool,
    /// The intent's `expiresAt`, from its prepare.
    expires_at: Option<Value>,
}

/// A submit whose outcome is unknown: it failed in transit, or was answered
/// 5xx, 408 or 429, and one read did not show the API took the
/// transactions. A read that shows the execution `prepared` does not rule
/// that out, so the safe ways on are the same signed bytes again, which
/// cannot land twice, or waiting until the execution is `expired`.
#[derive(Debug)]
pub struct MaybeSubmitted {
    pub execution_id: String,
    cause: String,
    /// The intent's `expiresAt`, when known.
    pub expires_at: Option<Value>,
    /// The deployment it was submitted to, for protocol_submit after a
    /// restart.
    pub deployment_id: Option<String>,
    /// The signed transactions, when the agent did not pass them itself
    /// (`protocol_execute` signed them).
    pub signed: Option<Vec<String>>,
}

impl std::fmt::Display for MaybeSubmitted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}. Execution {} may have been submitted, even if protocol_intent shows it \
             `prepared`. Do not prepare and sign the step again: resend the same signed \
             transactions with protocol_submit{}, which cannot land twice (the chain takes \
             a signed transaction once), or wait until protocol_intent shows the execution \
             settled or `expired`{}.",
            self.cause,
            self.execution_id,
            if self.signed.is_some() {
                " (`signedTransactions` in this error's data)"
            } else {
                ""
            },
            match &self.expires_at {
                Some(at) => format!(" (expiresAt {})", shown(at)),
                None => String::new(),
            }
        )?;
        if self.signed.is_some() {
            write!(
                f,
                " Running protocol_execute again for this step with the same args resends them \
                 itself, but restarting a4 mcp clears that guard: after a restart, a run would \
                 prepare and sign a new execution."
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for MaybeSubmitted {}

/// A `protocol_execute` refused before anything was prepared or signed,
/// because the same step call is running or has not settled.
#[derive(Debug)]
pub struct StepBusy {
    /// `step-in-flight` or `step-unsettled`.
    pub code: &'static str,
    pub execution_id: Option<String>,
    message: String,
}

impl std::fmt::Display for StepBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for StepBusy {}

/// What `protocol_execute` holds for a step call (deployment, instance,
/// step and signer) in this session.
#[derive(Debug, Clone)]
enum StepRun {
    /// A call is preparing, signing or sending it: its execution, once
    /// prepared.
    InFlight(Option<String>),
    /// Its last execution has not settled: the submit's outcome was
    /// unknown, or the wait ended first.
    Unsettled(LastRun),
    /// Its last execution completed. Until `until` (its `expiresAt`, Unix
    /// seconds) the same args get `answer` again instead of a second
    /// execution, since a host that gave up on the call never saw it.
    Completed {
        args: Value,
        answer: Value,
        expires_at: Value,
        until: i64,
    },
}

#[derive(Debug, Clone)]
struct LastRun {
    execution_id: String,
    signed: Vec<String>,
    /// The step args it was prepared with.
    args: Value,
}

impl StepRun {
    fn unsettled_by(&self, execution_id: &str) -> bool {
        matches!(self, Self::Unsettled(last) if last.execution_id == execution_id)
    }
}

/// A step call reserved as [`StepRun::InFlight`]. Dropped, it leaves the
/// step as `keep` (unsettled while it holds a signed execution that may
/// have been sent, or completed), and free when that is `None`.
struct Reservation {
    steps: Arc<DashMap<String, StepRun>>,
    key: String,
    keep: Option<StepRun>,
}

impl Reservation {
    /// Name the execution the call prepared, for a call refused while
    /// this one runs.
    fn prepared(&self, execution_id: &str) {
        self.steps.insert(
            self.key.clone(),
            StepRun::InFlight(Some(execution_id.to_string())),
        );
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        match self.keep.take() {
            Some(run) => {
                self.steps.insert(self.key.clone(), run);
            }
            None => {
                self.steps.remove(&self.key);
            }
        }
    }
}

/// `expiresAt` as Unix seconds: an RFC 3339 time, or a number of seconds
/// (or milliseconds, when it is that large).
fn unix_seconds(expires_at: &Value) -> Option<i64> {
    match expires_at {
        Value::Number(number) => {
            number
                .as_i64()
                .map(|n| if n > 100_000_000_000 { n / 1000 } else { n })
        }
        Value::String(at) => chrono::DateTime::parse_from_rfc3339(at)
            .ok()
            .map(|at| at.timestamp()),
        _ => None,
    }
}

/// `expiresAt` as written in a message: a time without its quotes.
fn shown(expires_at: &Value) -> String {
    match expires_at {
        Value::String(at) => at.clone(),
        other => other.to_string(),
    }
}

/// One resolved protocol API.
pub struct Target {
    pub api: ProtocolApi,
    /// `dev` or `mainnet`.
    pub cluster: &'static str,
    rpc_url: Option<String>,
    dev: Option<DevClient>,
}

impl Target {
    fn label(&self) -> &'static str {
        if self.dev.is_some() {
            "a4 dev"
        } else {
            "arete"
        }
    }
}

/// The hosted API origin and key, for tests.
#[derive(Clone)]
struct HostedOverride {
    origin: String,
    key: String,
}

/// State shared by the protocol tools.
#[derive(Clone)]
pub struct ProtocolAgent {
    http: reqwest::Client,
    registry: RegistryClient,
    signer: Option<Arc<LocalSigner>>,
    hosted: Option<HostedOverride>,
    executions: Arc<DashMap<String, Remembered>>,
    instances: Arc<DashMap<String, Remembered>>,
    /// What `protocol_execute` holds per step call in this session: one
    /// running, or one whose last execution has not settled (its submit
    /// had an unknown outcome, or the wait ended first). Running the call
    /// again answers for that execution instead of preparing a new one.
    steps: Arc<DashMap<String, StepRun>>,
    /// How long simulating a prepared step may take as a whole.
    simulation_budget: Duration,
}

impl ProtocolAgent {
    pub fn new(signer: Option<LocalSigner>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(API_TIMEOUT)
                .build()
                .unwrap_or_default(),
            registry: RegistryClient::new(),
            signer: signer.map(Arc::new),
            hosted: None,
            executions: Arc::new(DashMap::new()),
            instances: Arc::new(DashMap::new()),
            steps: Arc::new(DashMap::new()),
            simulation_budget: SIMULATION_BUDGET,
        }
    }

    /// Talk to the API at `origin` (registry included) with `key` instead
    /// of the API and key resolved from the environment.
    pub fn with_api(mut self, origin: &str, key: &str) -> Self {
        self.registry = RegistryClient::with_base_url(origin);
        self.hosted = Some(HostedOverride {
            origin: origin.trim_end_matches('/').to_string(),
            key: key.to_string(),
        });
        self
    }

    pub fn signer(&self) -> Option<&LocalSigner> {
        self.signer.as_deref()
    }

    fn target(&self, args: &TargetArgs, remembered: Option<&Remembered>) -> Result<Target> {
        // ARETE_ENVIRONMENT=dev targets the `a4 dev` session, as `dev` does.
        let dev = args.dev || crate::dev::environment_is_dev()?;
        let dev_project = match (&args.project, dev) {
            (Some(project), _) => Some(Some(PathBuf::from(project))),
            (None, true) => Some(None),
            (None, false) => remembered.and_then(|r| r.dev_project.clone()).map(Some),
        };
        if let Some(project) = dev_project {
            let client = DevProject::discover(project.as_deref()).and_then(DevClient::open)?;
            return Ok(Target {
                api: client.api(),
                cluster: "dev",
                rpc_url: Some(client.session.rpc_url.clone()),
                dev: Some(client),
            });
        }
        let (origin, key) = match &self.hosted {
            Some(hosted) => (hosted.origin.clone(), hosted.key.clone()),
            None => {
                let origin = std::env::var(ENV_API_URL)
                    .ok()
                    .map(|url| credentials::normalize_api_url(&url))
                    .filter(|url| !url.is_empty())
                    .unwrap_or_else(|| DEFAULT_API_ORIGIN.to_string());
                let key = credentials::resolve(None, "")?.key.ok_or_else(|| {
                    anyhow!(
                        "No Arete API key found in ARETE_API_KEY or the a4 login. Run `a4 auth signup` (agent) or `a4 auth login`, or set ARETE_API_KEY."
                    )
                })?;
                (origin, key)
            }
        };
        let origin = origin
            .trim_end_matches('/')
            .trim_end_matches("/api")
            .to_string();
        if !crate::registry::is_arete_origin(&origin) {
            bail!(
                "`{origin}` is not a recognised Arete origin (or is a non-loopback host over plain HTTP), so no credential will be sent to it. Point `ARETE_API_URL` at https://api.arete.run or a loopback runtime."
            );
        }
        let rpc_url = self
            .signer
            .as_ref()
            .and_then(|signer| signer.rpc_url.clone())
            .or_else(|| std::env::var(ENV_RPC_URL).ok())
            .filter(|url| !url.trim().is_empty());
        Ok(Target {
            api: ProtocolApi::new(
                self.http.clone(),
                &format!("{origin}/api"),
                &format!("{origin}/ws/sessions"),
                &key,
            ),
            // Arete's API settles on mainnet only and reports no other
            // cluster, so a hosted origin is mainnet: the strictest label,
            // which a signer must list explicitly. A loopback origin is a
            // local runtime, a fork like a dev environment.
            cluster: if is_loopback(&origin) {
                "dev"
            } else {
                "mainnet"
            },
            rpc_url,
            dev: None,
        })
    }

    async fn deployment(
        &self,
        target: &Target,
        args: &TargetArgs,
        remembered: Option<&Remembered>,
    ) -> Result<String> {
        if let Some(deployment) = args.deployment.as_deref().filter(|d| !d.is_empty()) {
            return Ok(deployment.to_string());
        }
        if let Some(dev) = &target.dev {
            return dev.deployment().map(str::to_string);
        }
        if let Some(protocol) = args.protocol.as_deref().filter(|p| !p.is_empty()) {
            return self.protocol_deployment(protocol).await;
        }
        if let Some(remembered) = remembered {
            return Ok(remembered.deployment.clone());
        }
        Err(invalid(
            "Pass `deployment` (a deployment id) or `protocol` (a package reference such as `token-raise`), or `dev: true` for the a4 dev session.",
        ))
    }

    /// The deployment a protocol package's install descriptor names.
    pub async fn protocol_deployment(&self, protocol: &str) -> Result<String> {
        let body = self.registry.protocol_install(protocol).await?;
        let install: Value =
            serde_json::from_str(&body).context("the protocol install descriptor is not JSON")?;
        install
            .pointer("/protocol/deployment/deploymentId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                anyhow!("protocol '{protocol}' has no deployment in its install descriptor")
            })
    }

    fn remember_instance(&self, target: &Target, deployment: &str, id: &str) {
        self.instances.insert(
            id.to_string(),
            Remembered {
                dev_project: target.dev.as_ref().map(|dev| dev.project.root.clone()),
                deployment: deployment.to_string(),
                ..Remembered::default()
            },
        );
    }

    /// A wallet argument: a base58 public key, or in a dev session also a
    /// dev wallet name.
    fn wallet(&self, target: &Target, wallet: &str) -> Result<String> {
        let wallet = wallet.trim();
        if let Some(dev) = &target.dev {
            return dev
                .wallet_pubkey(wallet)
                .map_err(|error| invalid(error.to_string()));
        }
        if is_pubkey(wallet) {
            Ok(wallet.to_string())
        } else {
            Err(invalid(format!("'{wallet}' is not a base58 public key")))
        }
    }

    /// `protocol_actions`.
    pub async fn actions(
        &self,
        args: &TargetArgs,
        wallet: &str,
        instance: Option<&str>,
        cursor: Option<&str>,
    ) -> Result<Value> {
        let target = self.target(args, None)?;
        let deployment = self.deployment(&target, args, None).await?;
        let wallet = self.wallet(&target, wallet)?;
        let mut body = target
            .api
            .wallet_actions(&deployment, &wallet, instance, cursor, None)
            .await?;
        for entry in body["instances"].as_array().into_iter().flatten() {
            if let Some(id) = entry["instanceId"].as_str() {
                self.remember_instance(&target, &deployment, id);
            }
        }
        if let Some(object) = body.as_object_mut() {
            object.entry("deploymentId").or_insert(json!(deployment));
            object.insert("target".into(), json!(target.label()));
            object.insert(
                "next".into(),
                json!(if body_has_cursor(object) {
                    "Pass `cursor: nextCursor` for more instances. Prepare an allowed step with protocol_prepare (create steps without `instance`)."
                } else {
                    "Prepare an allowed step with protocol_prepare (create steps without `instance`). Steps listing `argsDependent` guards are decided by their args at prepare."
                }),
            );
        }
        Ok(body)
    }

    /// `protocol_squads_proposer`: the calling account's Squads proposer for
    /// the deployment, with its SOL balance when an RPC is available
    /// (`rpc_url`, else the target's).
    /// `protocol_check`: the server's upload checks on a built release
    /// (`<dir>/.arete/protocol/`) or an inline spec, storing nothing. Always
    /// against Arete's API: a dev session has no separate checker.
    pub async fn check(&self, dir: Option<&str>, spec: Option<Value>) -> Result<Value> {
        let (spec, bundle, source) = match (spec, dir) {
            (Some(_), Some(_)) => {
                return Err(invalid("Pass `spec` or `dir`, not both."));
            }
            (Some(spec), None) => (spec, None, "spec".to_string()),
            (None, dir) => {
                let build = Path::new(dir.unwrap_or(".")).join(".arete/protocol");
                let spec_path = build.join("protocol-spec.json");
                let text = std::fs::read_to_string(&spec_path).map_err(|error| {
                    invalid(format!(
                        "Cannot read {} ({error}). Build the protocol first: `npx arete-protocol build` in the project, or pass `spec`.",
                        spec_path.display()
                    ))
                })?;
                let spec: Value = serde_json::from_str(&text)
                    .with_context(|| format!("{} is not JSON", spec_path.display()))?;
                let bundle = match std::fs::read(build.join("handlers.mjs")) {
                    Ok(bytes) => Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error).context("Cannot read handlers.mjs"),
                };
                (spec, bundle, spec_path.display().to_string())
            }
        };
        let target = self.target(&TargetArgs::default(), None)?;
        let mut result = target.api.check_release(&spec, bundle.as_deref()).await?;
        if let Some(object) = result.as_object_mut() {
            object.insert("checked".into(), json!(source));
            let next = if object.get("passed") == Some(&json!(true)) {
                if object.get("templateCheck") == Some(&json!("pending")) {
                    "The server accepts this release, but its template check is pending until the program specs in `missingProgramSpecs` reach Arete; activation waits for them."
                } else {
                    "The server accepts this release. Deploy it with `a4 protocol deploy`."
                }
            } else {
                "The server would reject this release. Each entry of `errors` names the spec path (a JSON pointer), the code and the problem; fix them, rebuild and check again."
            };
            object.insert("next".into(), json!(next));
        }
        Ok(result)
    }

    pub async fn squads_proposer(&self, args: &TargetArgs, rpc_url: Option<&str>) -> Result<Value> {
        let target = self.target(args, None)?;
        let deployment = self.deployment(&target, args, None).await?;
        let mut body = target.api.squads_proposer(&deployment).await?;
        let proposer = body["pubkey"].as_str().unwrap_or_default().to_string();
        let balance = self.proposer_balance(&target, rpc_url, &proposer).await;
        if let Some(object) = body.as_object_mut() {
            object.insert("deploymentId".into(), json!(deployment));
            object.insert("target".into(), json!(target.label()));
            object.extend(balance);
            object.insert(
                "next".into(),
                json!(format!(
                    "Add {proposer} to the team's Squads v4 multisig as a member with exactly the Initiate permission (mask 1), and keep it funded with a little SOL: it pays the fee and rent of every proposal. {} protocol_squads_status checks it.",
                    squads_add_hint(target.dev.is_some(), None, &proposer)
                )),
            );
        }
        Ok(body)
    }

    /// `protocol_squads_status`: whether the proposer is an Initiate-only
    /// member of `multisig`, with its SOL balance when an RPC is available.
    pub async fn squads_status(
        &self,
        args: &TargetArgs,
        multisig: &str,
        rpc_url: Option<&str>,
    ) -> Result<Value> {
        let multisig = squads_multisig(multisig)?;
        let target = self.target(args, None)?;
        let deployment = self.deployment(&target, args, None).await?;
        let mut body = target.api.squads_status(&deployment, &multisig).await?;
        let proposer = body["proposer"].as_str().unwrap_or_default().to_string();
        // Newer servers report the balance and what one proposal needs;
        // older ones omit both, and the balance is read from an RPC.
        let balance = match body["proposerLamports"].as_u64() {
            Some(lamports) => balance_fields(lamports, body["proposerMinLamports"].as_u64()),
            None => self.proposer_balance(&target, rpc_url, &proposer).await,
        };
        let next = squads_status_next(&body, target.dev.is_some());
        if let Some(object) = body.as_object_mut() {
            object.insert("deploymentId".into(), json!(deployment));
            object.insert("target".into(), json!(target.label()));
            object.extend(balance);
            object.insert("next".into(), json!(next));
        }
        Ok(body)
    }

    /// The proposer's balance fields: `proposerLamports` (null when no RPC
    /// answered), `balance` (`ok`, `low` or `unavailable`) and, unless it
    /// is `ok`, `balanceNote`.
    async fn proposer_balance(
        &self,
        target: &Target,
        rpc_url: Option<&str>,
        proposer: &str,
    ) -> serde_json::Map<String, Value> {
        let lamports = if proposer.is_empty() {
            Err(anyhow!("the server named no proposer"))
        } else {
            let params = json!([proposer, { "commitment": "confirmed" }]);
            match (rpc_url.filter(|url| !url.trim().is_empty()), &target.dev) {
                (None, Some(dev)) => dev.rpc("getBalance", params).await,
                (rpc_url, _) => match rpc_url.or(target.rpc_url.as_deref()) {
                    Some(url) => rpc_call(&self.http, url, "getBalance", params).await,
                    None => Err(anyhow!(
                        "no RPC URL is configured ({ENV_RPC_URL}, or the signer's rpcUrl)"
                    )),
                },
            }
            .and_then(|result| {
                result["value"]
                    .as_u64()
                    .ok_or_else(|| anyhow!("getBalance returned no value"))
            })
        };
        match lamports {
            Ok(lamports) => balance_fields(lamports, None),
            Err(error) => {
                let mut fields = serde_json::Map::new();
                fields.insert("proposerLamports".into(), Value::Null);
                fields.insert("balance".into(), json!("unavailable"));
                fields.insert(
                    "balanceNote".into(),
                    json!(format!("The balance was not read: {error:#}")),
                );
                fields
            }
        }
    }

    /// `protocol_instances`.
    pub async fn instances(
        &self,
        args: &TargetArgs,
        state: Option<&str>,
        cursor: Option<&str>,
    ) -> Result<Value> {
        let target = self.target(args, None)?;
        let deployment = self.deployment(&target, args, None).await?;
        let mut body = target
            .api
            .instances(&deployment, state, cursor, None)
            .await?;
        for entry in body["instances"].as_array().into_iter().flatten() {
            if let Some(id) = entry["id"].as_str() {
                self.remember_instance(&target, &deployment, id);
            }
        }
        if let Some(object) = body.as_object_mut() {
            object.entry("deploymentId").or_insert(json!(deployment));
            object.insert("target".into(), json!(target.label()));
        }
        Ok(body)
    }

    /// `protocol_instance`.
    pub async fn instance(
        &self,
        args: &TargetArgs,
        instance: &str,
        wallet: Option<&str>,
    ) -> Result<Value> {
        let remembered = self.instances.get(instance).map(|entry| entry.clone());
        let target = self.target(args, remembered.as_ref())?;
        let deployment = self.deployment(&target, args, remembered.as_ref()).await?;
        let wallet = wallet
            .map(|wallet| self.wallet(&target, wallet))
            .transpose()?;
        let body = target
            .api
            .instance(&deployment, instance, wallet.as_deref())
            .await?;
        self.remember_instance(&target, &deployment, instance);
        Ok(body)
    }

    async fn prepare_inner(
        &self,
        target: &Target,
        deployment: &str,
        request: &StepCall<'_>,
        wallet: &str,
    ) -> Result<Result<(Value, Value), Value>> {
        let prepared = target
            .api
            .prepare(&PrepareRequest {
                deployment,
                instance: request.instance,
                step: request.step,
                args: request.args,
                wallet,
                transaction_format: request.transaction_format.unwrap_or("v0"),
            })
            .await?;
        let prepared = match prepared {
            Prepared::Rejected(rejection) => {
                let mut result = prepared_summary::rejection_summary(&rejection);
                result["state"] = json!("rejected");
                result["wallet"] = json!(wallet);
                result["deploymentId"] = json!(deployment);
                return Ok(Err(result));
            }
            Prepared::Intent(prepared) => prepared,
        };
        let rpc = target
            .rpc_url
            .as_deref()
            .map(|url| Rpc::new(self.http.clone(), url).within(self.simulation_budget));
        let summary = prepared_summary::summarize(&prepared, wallet, rpc.as_ref()).await?;
        let execution_id = prepared["intent"]["executionId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let instance = prepared["intent"]["instanceId"]
            .as_str()
            .map(str::to_string)
            .or(request.instance.map(str::to_string));
        self.executions.insert(
            execution_id,
            Remembered {
                dev_project: target.dev.as_ref().map(|dev| dev.project.root.clone()),
                deployment: deployment.to_string(),
                step: Some(request.step.to_string()),
                instance: instance.clone(),
                wallet: Some(wallet.to_string()),
                sent: false,
                expires_at: Some(prepared["intent"]["expiresAt"].clone())
                    .filter(|at| !at.is_null()),
            },
        );
        if let Some(instance) = &instance {
            self.remember_instance(target, deployment, instance);
        }
        Ok(Ok((prepared, summary)))
    }

    /// `protocol_prepare`: unsigned transactions and their summary, or the
    /// rejection.
    pub async fn prepare(
        &self,
        args: &TargetArgs,
        call: &StepCall<'_>,
        wallet: &str,
    ) -> Result<Value> {
        let target = self.target(args, None)?;
        let deployment = self.deployment(&target, args, None).await?;
        let wallet = self.wallet(&target, wallet)?;
        let (prepared, summary) = match self
            .prepare_inner(&target, &deployment, call, &wallet)
            .await?
        {
            Ok(prepared) => prepared,
            Err(rejection) => return Ok(rejection),
        };
        let transactions: Vec<Value> = prepared["transactions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|transaction| {
                json!({
                    "index": transaction["index"],
                    "transactionBase64": transaction["transactionBase64"],
                    "messageSha256": transaction["messageSha256"],
                    "requiredWalletSigners": transaction["requiredWalletSigners"],
                })
            })
            .collect();
        Ok(json!({
            "state": "prepared",
            "target": target.label(),
            "deploymentId": deployment,
            "executionId": prepared["intent"]["executionId"],
            "step": call.step,
            "instanceId": prepared["intent"]["instanceId"],
            "wallet": wallet,
            "expiresAt": prepared["intent"]["expiresAt"],
            "fees": prepared["intent"]["fees"],
            "transactions": transactions,
            "summary": summary,
            "next": "Check `summary`, sign every transaction with the wallet (keep the signatures already present), then call protocol_submit with `executionId` and the signed base64 transactions before `expiresAt`.",
        }))
    }

    /// `protocol_submit`: submit signed transactions and wait (bounded).
    pub async fn submit(
        &self,
        args: &TargetArgs,
        execution_id: &str,
        signed: &[String],
        wait: Option<bool>,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        if signed.is_empty() {
            return Err(invalid(
                "`signedTransactions` must hold the signed base64 transactions",
            ));
        }
        for (index, transaction) in signed.iter().enumerate() {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(transaction.trim())
                .map_err(|_| invalid(format!("signedTransactions[{index}] is not base64")))?;
            let decoded = decode_transaction(&bytes).map_err(|error| {
                invalid(format!(
                    "signedTransactions[{index}] is not a transaction: {error}"
                ))
            })?;
            let missing: Vec<String> = decoded
                .signers()
                .into_iter()
                .filter(|(_, signed)| !signed)
                .map(|(key, _)| key)
                .collect();
            if !missing.is_empty() {
                return Err(invalid(format!(
                    "signedTransactions[{index}] still lacks the signature of {}",
                    missing.join(", ")
                )));
            }
        }
        let remembered = self.executions.get(execution_id).map(|entry| entry.clone());
        let target = self.target(args, remembered.as_ref())?;
        let deployment = self.deployment(&target, args, remembered.as_ref()).await?;
        let signed: Vec<String> = signed.iter().map(|s| s.trim().to_string()).collect();
        let result = self
            .submit_and_wait(
                &target,
                &deployment,
                execution_id,
                &signed,
                remembered.as_ref(),
                wait,
                timeout,
            )
            .await?;
        // Settled: a protocol_execute step left unsettled by this execution
        // (resent here, as its error says) is free to run anew, or, when it
        // completed, answers the same args with it until its expiresAt.
        if result["settled"] == json!(true) {
            let held: Vec<(String, Value)> = self
                .steps
                .iter()
                .filter_map(|run| match run.value() {
                    StepRun::Unsettled(last) if last.execution_id == execution_id => {
                        Some((run.key().clone(), last.args.clone()))
                    }
                    _ => None,
                })
                .collect();
            for (key, args) in held {
                match self.completed(execution_id, args, &result) {
                    Some(done) => {
                        if let Some(mut run) = self.steps.get_mut(&key) {
                            if run.unsettled_by(execution_id) {
                                *run = done;
                            }
                        }
                    }
                    None => {
                        self.steps
                            .remove_if(&key, |_, run| run.unsettled_by(execution_id));
                    }
                }
            }
        }
        Ok(result)
    }

    /// What a step call keeps once its execution settled with `answer`: a
    /// completed one with a known `expiresAt`, until then.
    fn completed(&self, execution_id: &str, args: Value, answer: &Value) -> Option<StepRun> {
        if answer["state"] != "completed" {
            return None;
        }
        let expires_at = self.executions.get(execution_id)?.expires_at.clone()?;
        let until = unix_seconds(&expires_at)?;
        Some(StepRun::Completed {
            args,
            answer: answer.clone(),
            expires_at,
            until,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn submit_and_wait(
        &self,
        target: &Target,
        deployment: &str,
        execution_id: &str,
        signed: &[String],
        remembered: Option<&Remembered>,
        wait: Option<bool>,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        let token = target.api.token(deployment, &TRANSACTION_SCOPES).await?;
        let submitted = self
            .send_once(target, &token, deployment, execution_id, signed)
            .await?;
        let waited = if wait.unwrap_or(true) {
            target
                .api
                .wait_intent(
                    &token,
                    execution_id,
                    submitted,
                    timeout.unwrap_or(DEFAULT_WAIT).min(MAX_WAIT),
                )
                .await?
        } else {
            crate::protocol_api::Settled {
                settled: crate::protocol_api::is_final(&submitted),
                state: submitted,
                poll_error: None,
            }
        };
        let intent = waited.state;
        let instance = remembered
            .and_then(|r| r.instance.clone())
            .or_else(|| intent["instanceId"].as_str().map(str::to_string));
        let mut result = json!({
            "step": remembered.and_then(|r| r.step.clone()),
            "wallet": remembered.and_then(|r| r.wallet.clone()),
            "executionId": execution_id,
            "instanceId": instance,
            "state": intent["state"],
            "settled": waited.settled,
            "transactions": intent["transactions"],
        });
        if let Some(url) = intent.get("receiptUrl").filter(|url| url.is_string()) {
            result["receiptUrl"] = url.clone();
        }
        if let Some(error) = waited.poll_error {
            result["pollError"] = json!(error);
        }
        if !waited.settled {
            result["next"] =
                json!("Not settled yet: read it with protocol_intent later. Do not resubmit.");
        } else if result.get("receiptUrl").is_some() {
            result["next"] = json!(
                "Completed. Check the signed receipt of a transaction with protocol_receipt and its signature."
            );
        }
        Ok(result)
    }

    /// Submit `signed` for `execution_id` and return the intent's state,
    /// never sending an execution twice: one this server already sent is
    /// sent again only when a read shows the API never took it (still
    /// `prepared`), so the same signed bytes go again. A submit whose
    /// outcome is unknown (lost in transit, timed out, or any 5xx, 408 or
    /// 429) is settled with one read: taken, its state is returned; a 5xx
    /// whose body says `not_submitted` is a refusal; otherwise it fails
    /// with [`MaybeSubmitted`].
    async fn send_once(
        &self,
        target: &Target,
        token: &str,
        deployment: &str,
        execution_id: &str,
        signed: &[String],
    ) -> Result<Value> {
        let sent = self
            .executions
            .get(execution_id)
            .is_some_and(|entry| entry.sent);
        if sent {
            match target.api.taken_state(token, execution_id).await {
                Ok(Some(state)) => return Ok(state),
                Ok(None) => {}
                Err(error) => return Err(self.maybe_submitted(execution_id, &error)),
            }
        }
        self.executions
            .entry(execution_id.to_string())
            .or_insert_with(|| Remembered {
                dev_project: target.dev.as_ref().map(|dev| dev.project.root.clone()),
                deployment: deployment.to_string(),
                ..Remembered::default()
            })
            .sent = true;
        let error = match target.api.submit(token, execution_id, signed).await {
            Ok(submitted) => return Ok(submitted),
            Err(error) => error,
        };
        let refused = match error.downcast_ref::<SubmitUnknown>() {
            Some(answered) => answered.not_submitted,
            None if error.downcast_ref::<reqwest::Error>().is_some() => false,
            None => return Err(error),
        };
        match target.api.taken_state(token, execution_id).await {
            Ok(Some(state)) => Ok(state),
            _ if refused => Err(error),
            Ok(None) => Err(self.maybe_submitted(
                execution_id,
                &anyhow!("{error:#}; a read then showed it still `prepared`"),
            )),
            Err(read) => {
                Err(self.maybe_submitted(execution_id, &anyhow!("{error:#}; then {read:#}")))
            }
        }
    }

    /// [`MaybeSubmitted`] for `execution_id`, with its deployment and, when
    /// this server prepared it, its `expiresAt`.
    fn maybe_submitted(&self, execution_id: &str, cause: &anyhow::Error) -> anyhow::Error {
        let remembered = self.executions.get(execution_id).map(|entry| entry.clone());
        anyhow!(MaybeSubmitted {
            execution_id: execution_id.to_string(),
            cause: format!("{cause:#}"),
            deployment_id: remembered.as_ref().map(|entry| entry.deployment.clone()),
            expires_at: remembered.and_then(|entry| entry.expires_at),
            signed: None,
        })
    }

    /// `protocol_intent`.
    pub async fn intent(&self, args: &TargetArgs, execution_id: &str) -> Result<Value> {
        let remembered = self.executions.get(execution_id).map(|entry| entry.clone());
        let target = self.target(args, remembered.as_ref())?;
        let deployment = self.deployment(&target, args, remembered.as_ref()).await?;
        let token = target
            .api
            .token(&deployment, &crate::protocol_api::READ_SCOPES)
            .await?;
        target.api.intent(&token, execution_id).await
    }

    /// `protocol_receipt`: fetch the signed execution receipt of a
    /// transaction from Arete, verify its signature against the published
    /// receipt keys, and, with an RPC configured, check it against the
    /// transaction itself.
    pub async fn receipt(&self, signature: &str) -> Result<Value> {
        let signature = signature.trim();
        if signature.is_empty() {
            return Err(invalid("`signature` must be a transaction signature"));
        }
        let (origin, key) = self.hosted_origin()?;
        let segment = crate::protocol_api::path_segment(signature)?;
        let fetch = |path: String| {
            let mut request = self.http.get(format!("{origin}/api{path}"));
            if let Some(key) = &key {
                request = request.bearer_auth(key);
            }
            request
        };
        let response = fetch(format!("/protocol-receipts/{segment}"))
            .send()
            .await
            .with_context(|| format!("The protocol API is unreachable at {origin}"))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(json!({
                "signature": signature,
                "found": false,
                "code": body.get("code").cloned().unwrap_or(json!("protocol-receipt-not-found")),
                "note": "Arete issues a receipt only for a completed intent transaction whose message hash it confirmed and whose memo the revision's attestation key signed.",
            }));
        }
        if !status.is_success() {
            bail!("Reading the receipt of {signature} failed ({status}): {body}");
        }
        let response = fetch("/protocol-receipts/keys".to_string())
            .send()
            .await
            .with_context(|| format!("The protocol API is unreachable at {origin}"))?;
        let keys_status = response.status();
        let keys: Value = response.json().await.unwrap_or(Value::Null);
        if !keys_status.is_success() {
            bail!("Reading the receipt keys failed ({keys_status}): {keys}");
        }
        let signature_check = match serde_json::from_value::<
            arete_sdk::protocol::ProtocolReceiptKeySet,
        >(keys)
        {
            Ok(keys) => match arete_sdk::protocol::verify_receipt(&body, &keys) {
                Ok(verified) => json!({ "valid": true, "keyId": verified.key_id }),
                Err(error) => {
                    json!({ "valid": false, "error": error.code, "message": error.message })
                }
            },
            Err(error) => {
                json!({ "valid": false, "error": "invalid-key-set", "message": error.to_string() })
            }
        };
        let rpc_url = self
            .signer
            .as_ref()
            .and_then(|signer| signer.rpc_url.clone())
            .or_else(|| std::env::var(ENV_RPC_URL).ok())
            .filter(|url| !url.trim().is_empty());
        let on_chain = match rpc_url {
            None => json!({
                "checked": false,
                "note": format!("Set {ENV_RPC_URL} to an RPC URL to also check the receipt against the transaction."),
            }),
            Some(url) => match self.rpc_transaction(&url, signature).await {
                Err(error) => json!({ "checked": false, "error": error.to_string() }),
                Ok(Value::Null) => {
                    json!({ "checked": false, "error": "The RPC does not know this transaction." })
                }
                Ok(transaction) => {
                    match arete_sdk::protocol::verify_receipt_on_chain(&body, &transaction) {
                        Ok(()) => json!({ "checked": true, "valid": true }),
                        Err(error) => {
                            json!({ "checked": true, "valid": false, "error": error.code, "message": error.message })
                        }
                    }
                }
            },
        };
        Ok(json!({
            "signature": signature,
            "found": true,
            "receipt": body,
            "signatureCheck": signature_check,
            "onChain": on_chain,
        }))
    }

    /// `getTransaction` with base64 encoding, so the message bytes are exact.
    async fn rpc_transaction(&self, url: &str, signature: &str) -> Result<Value> {
        let response: Value = self
            .http
            .post(url)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getTransaction",
                "params": [signature, {
                    "encoding": "base64",
                    "commitment": "confirmed",
                    "maxSupportedTransactionVersion": 1,
                }],
            }))
            .send()
            .await
            .map_err(|error| anyhow!("The RPC is unreachable: {}", error.without_url()))?
            .json()
            .await
            .map_err(|error| {
                anyhow!(
                    "The RPC returned a body that is not JSON: {}",
                    error.without_url()
                )
            })?;
        if let Some(error) = response.get("error") {
            bail!("getTransaction failed: {error}");
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// The hosted origin, and the API key when one is configured (public
    /// deployments' receipts need none).
    fn hosted_origin(&self) -> Result<(String, Option<String>)> {
        let (origin, key) = match &self.hosted {
            Some(hosted) => (hosted.origin.clone(), Some(hosted.key.clone())),
            None => {
                let origin = std::env::var(ENV_API_URL)
                    .ok()
                    .map(|url| credentials::normalize_api_url(&url))
                    .filter(|url| !url.is_empty())
                    .unwrap_or_else(|| DEFAULT_API_ORIGIN.to_string());
                let key = credentials::resolve(None, "")
                    .ok()
                    .and_then(|resolved| resolved.key);
                (origin, key)
            }
        };
        let origin = origin
            .trim_end_matches('/')
            .trim_end_matches("/api")
            .to_string();
        if !crate::registry::is_arete_origin(&origin) {
            bail!(
                "`{origin}` is not a recognised Arete origin (or is a non-loopback host over plain HTTP), so no credential will be sent to it. Point `ARETE_API_URL` at https://api.arete.run or a loopback runtime."
            );
        }
        Ok((origin, key))
    }

    /// `protocol_execute`: prepare for the signer, check the policy, sign,
    /// submit and wait. The step call (deployment, instance, step, signer)
    /// is reserved before anything is funded, prepared or signed, so a
    /// second call while one runs is refused with [`StepBusy`], naming its
    /// execution once prepared. While its last execution has not settled
    /// (an unknown submit, or a wait that ended first), a call with the
    /// same args answers for that execution (the same signed bytes) instead
    /// of preparing and signing a new one, and a call with other args is
    /// refused. Once it completed, a call with the same args gets its
    /// answer again until its `expiresAt`, since a host that gave up on the
    /// first call never saw it. This lasts for this server's session only.
    pub async fn execute(
        &self,
        args: &TargetArgs,
        call: &StepCall<'_>,
        wait: Option<bool>,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        let signer = self
            .signer
            .clone()
            .ok_or_else(|| anyhow!("No signer is configured"))?;
        if let Some(project) = &args.project {
            require_under_working_dir(Path::new(project))?;
        }
        let mut target = self.target(args, None)?;
        target.rpc_url = simulation_rpc(&signer, &target);
        let deployment = self.deployment(&target, args, None).await?;
        if let Err(refusal) = dev_deployment_binding(&target, &deployment)
            .and_then(|()| signer.check_target(&deployment, target.cluster))
        {
            return Ok(refused(&refusal, &deployment, target.cluster, None, None));
        }
        let wallet = signer.pubkey();
        let call_key = [
            target.label(),
            deployment.as_str(),
            call.instance.unwrap_or_default(),
            call.step,
            wallet.as_str(),
        ]
        .join("\n");
        let step_args = match call.args {
            Value::Null => json!({}),
            other => other.clone(),
        };
        let now = crate::dev::unix_now() as i64;
        let resumed = match self.steps.entry(call_key.clone()) {
            dashmap::mapref::entry::Entry::Vacant(free) => {
                free.insert(StepRun::InFlight(None));
                None
            }
            dashmap::mapref::entry::Entry::Occupied(mut held) => match held.get().clone() {
                StepRun::InFlight(execution_id) => {
                    let message = match &execution_id {
                        Some(id) => format!(
                            "protocol_execute of step '{}' is already running for this signer \
                             in this a4 mcp session, as execution {id}; nothing was prepared or \
                             signed for this call. Its answer goes to the call that started it. \
                             If your host gave up on that call, its answer is lost: read \
                             execution {id} with protocol_intent before re-running the step.",
                            call.step
                        ),
                        None => format!(
                            "protocol_execute of step '{}' is already running for this signer \
                             in this a4 mcp session and has not prepared its execution yet; \
                             nothing was prepared or signed for this call. Its answer goes to \
                             the call that started it. If your host gave up on that call, its \
                             answer is lost: call again shortly for its `executionId`, and read \
                             it with protocol_intent before re-running the step.",
                            call.step
                        ),
                    };
                    return Err(anyhow!(StepBusy {
                        code: "step-in-flight",
                        execution_id,
                        message,
                    }));
                }
                StepRun::Unsettled(previous) if previous.args != step_args => {
                    return Err(anyhow!(StepBusy {
                        code: "step-unsettled",
                        message: format!(
                            "Step '{}' was last run with other args, and its execution {} has \
                             not settled; nothing was prepared or signed for this call. Run \
                             the step again with its previous args ({}) to answer for that \
                             execution; once that answer, or protocol_submit with its signed \
                             transactions, shows it settled, the step runs with new args. \
                             Restarting a4 mcp clears this guard, so after a restart do not \
                             run the step until protocol_intent shows execution {} settled or \
                             `expired`.",
                            call.step, previous.execution_id, previous.args, previous.execution_id
                        ),
                        execution_id: Some(previous.execution_id),
                    }))
                }
                StepRun::Unsettled(previous) => {
                    held.insert(StepRun::InFlight(Some(previous.execution_id.clone())));
                    Some(previous)
                }
                StepRun::Completed {
                    args,
                    mut answer,
                    expires_at,
                    until,
                } if args == step_args && until > now => {
                    answer["note"] = json!(format!(
                        "This answers the last protocol_execute of step '{}' with these args, \
                         which completed: nothing new was prepared or signed. Until its \
                         expiresAt ({}), running the step with the same args answers this way; \
                         other args, or the same args after then, run anew.",
                        call.step,
                        shown(&expires_at)
                    ));
                    return Ok(answer);
                }
                StepRun::Completed { .. } => {
                    held.insert(StepRun::InFlight(None));
                    None
                }
            },
        };
        let mut reservation = Reservation {
            steps: self.steps.clone(),
            key: call_key,
            keep: resumed.clone().map(StepRun::Unsettled),
        };
        if let Some(previous) = resumed {
            let remembered = self
                .executions
                .get(&previous.execution_id)
                .map(|entry| entry.clone());
            let mut result = self
                .submit_and_wait(
                    &target,
                    &deployment,
                    &previous.execution_id,
                    &previous.signed,
                    remembered.as_ref(),
                    wait,
                    timeout,
                )
                .await
                .map_err(|mut error| {
                    if let Some(unknown) = error.downcast_mut::<MaybeSubmitted>() {
                        unknown.signed = Some(previous.signed.clone());
                    }
                    error
                })?;
            let settled = result["settled"] == json!(true);
            if settled {
                reservation.keep = self.completed(&previous.execution_id, step_args, &result);
            }
            result["note"] = json!(match &reservation.keep {
                Some(StepRun::Completed { expires_at, .. }) => format!(
                    "This answers the previous protocol_execute of this step, which had not \
                     settled: nothing new was prepared or signed. Until its expiresAt ({}), \
                     running the step again with the same args answers for this execution; \
                     other args run anew.",
                    shown(expires_at)
                ),
                _ if settled => "This answers the previous protocol_execute of this step, \
                                 which had not settled: nothing new was prepared or signed. \
                                 Run the step again for a new execution."
                    .to_string(),
                _ => "This answers the previous protocol_execute of this step, which has not \
                      settled: nothing new was prepared or signed, and running the step again \
                      in this a4 mcp session answers for this execution until it settles."
                    .to_string(),
            });
            return Ok(result);
        }
        if let Some(dev) = &target.dev {
            fund_on_fork(dev, &wallet).await?;
        }
        let (prepared, summary) = match self
            .prepare_inner(&target, &deployment, call, &wallet)
            .await?
        {
            Ok(prepared) => prepared,
            Err(rejection) => return Ok(rejection),
        };
        let execution_id = prepared["intent"]["executionId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        reservation.prepared(&execution_id);
        if let Err(refusal) = signer.check_prepared(&deployment, &prepared, &summary) {
            return Ok(refused(
                &refusal,
                &deployment,
                target.cluster,
                Some(&execution_id),
                Some(summary),
            ));
        }
        let signed = signer.sign_all(&prepared)?;
        // Signed: from here on it may be sent, so the step stays unsettled
        // unless it settles or the API refuses it.
        reservation.keep = Some(StepRun::Unsettled(LastRun {
            execution_id: execution_id.clone(),
            signed: signed.clone(),
            args: step_args.clone(),
        }));
        let remembered = self
            .executions
            .get(&execution_id)
            .map(|entry| entry.clone());
        let mut result = match self
            .submit_and_wait(
                &target,
                &deployment,
                &execution_id,
                &signed,
                remembered.as_ref(),
                wait,
                timeout,
            )
            .await
        {
            Ok(result) => result,
            Err(mut error) => {
                match error.downcast_mut::<MaybeSubmitted>() {
                    Some(unknown) => unknown.signed = Some(signed),
                    // Refused, or never sent: nothing was taken.
                    None => reservation.keep = None,
                }
                return Err(error);
            }
        };
        result["summary"] = summary;
        if result["settled"] == json!(true) {
            reservation.keep = self.completed(&execution_id, step_args, &result);
        }
        Ok(result)
    }
}

/// A dev target is labelled `dev` because its deployment runs in the dev
/// environment. A deployment id passed alongside it may name any deployment,
/// a mainnet one included, so the signer only signs for the environment's
/// own deployment there.
fn dev_deployment_binding(target: &Target, deployment: &str) -> Result<(), Refusal> {
    let Some(dev) = &target.dev else {
        return Ok(());
    };
    match dev.deployment() {
        Ok(own) if own == deployment => Ok(()),
        Ok(own) => Err(Refusal::new(
            "clusters",
            format!("deployment {deployment} is not the dev environment's deployment ({own}), so it is not known to be on the dev cluster; the signer signs only for the environment's own deployment in a dev target"),
        )),
        Err(_) => Err(Refusal::new(
            "clusters",
            format!("the dev environment has no deployment, so deployment {deployment} is not known to be on the dev cluster"),
        )),
    }
}

/// The RPC `protocol_execute` simulates with: the signer's own `rpcUrl`
/// whenever it has one. In an `a4 dev` session the session's local RPC (the
/// loopback proxy to the dev environment that `a4 dev up` serves) is used
/// instead only when the signer has no `rpcUrl`, or its policy lists
/// `dev` explicitly in `clusters`, since a fork's state is not on the
/// signer's RPC.
fn simulation_rpc(signer: &LocalSigner, target: &Target) -> Option<String> {
    let session_rpc = target
        .dev
        .as_ref()
        .map(|dev| dev.session.rpc_url.clone())
        .filter(|url| crate::dev::is_loopback_url(url));
    match (&signer.rpc_url, session_rpc) {
        (Some(own), Some(session)) => {
            let dev_listed =
                signer.policy.clusters_listed && signer.policy.clusters.iter().any(|c| c == "dev");
            Some(if dev_listed { session } else { own.clone() })
        }
        (Some(own), None) => Some(own.clone()),
        (None, Some(session)) => Some(session),
        (None, None) if target.dev.is_some() => None,
        (None, None) => target.rpc_url.clone(),
    }
}

/// `protocol_execute` only signs for an `a4 dev` project at or below the
/// server's working directory, so an agent cannot point the signer at a
/// session file elsewhere on the machine.
fn require_under_working_dir(project: &Path) -> Result<()> {
    let cwd = std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .context("Failed to read the current directory")?;
    let project = if project.is_absolute() {
        project.to_path_buf()
    } else {
        cwd.join(project)
    };
    let project = std::fs::canonicalize(&project)
        .map_err(|_| invalid(format!("project `{}` does not exist", project.display())))?;
    if !project.starts_with(&cwd) {
        return Err(invalid(format!(
            "protocol_execute only uses a4 dev projects at or below the server's working directory ({}); `{}` is outside it",
            cwd.display(),
            project.display()
        )));
    }
    Ok(())
}

/// The step a prepare or execute names.
#[derive(Debug, Clone)]
pub struct StepCall<'a> {
    pub step: &'a str,
    pub instance: Option<&'a str>,
    pub args: &'a Value,
    pub transaction_format: Option<&'a str>,
}

fn refused(
    refusal: &Refusal,
    deployment: &str,
    cluster: &str,
    execution_id: Option<&str>,
    summary: Option<Value>,
) -> Value {
    let mut out = json!({
        "state": "refused",
        "rule": refusal.rule,
        "reason": refusal.reason,
        "deploymentId": deployment,
        "cluster": cluster,
        "note": "The signer's policy refused this step: nothing was signed or submitted.",
    });
    if let Some(execution_id) = execution_id {
        out["executionId"] = json!(execution_id);
    }
    if let Some(summary) = summary {
        out["summary"] = summary;
    }
    out
}

/// Fund the signer in the dev environment when it has no SOL (dev sessions
/// only).
async fn fund_on_fork(dev: &DevClient, wallet: &str) -> Result<()> {
    let balance = dev
        .rpc("getBalance", json!([wallet, { "commitment": "processed" }]))
        .await?;
    if balance.get("value").and_then(Value::as_u64) == Some(0) {
        dev.env
            .fund(wallet, crate::dev::WALLET_FUND_LAMPORTS)
            .await
            .context("Funding the signer in the dev environment failed")?;
    }
    Ok(())
}

fn body_has_cursor(object: &serde_json::Map<String, Value>) -> bool {
    object
        .get("nextCursor")
        .is_some_and(|cursor| cursor.as_str().is_some_and(|c| !c.is_empty()))
}

fn is_pubkey(value: &str) -> bool {
    bs58::decode(value).into_vec().map(|bytes| bytes.len()) == Ok(32)
}

/// Lamports below which the Squads proposer is reported low on SOL.
pub const SQUADS_PROPOSER_LOW_LAMPORTS: u64 = 50_000_000;

/// A `multisig` argument: a base58 public key.
fn squads_multisig(multisig: &str) -> Result<String> {
    let multisig = multisig.trim();
    if is_pubkey(multisig) {
        Ok(multisig.to_string())
    } else {
        Err(invalid(format!(
            "`multisig` '{multisig}' is not a base58 public key (the Squads multisig account)"
        )))
    }
}

/// The balance fields for `lamports`: low below `minimum` (what one
/// proposal needs, when the server says) or else below
/// [`SQUADS_PROPOSER_LOW_LAMPORTS`].
fn balance_fields(lamports: u64, minimum: Option<u64>) -> serde_json::Map<String, Value> {
    let mut fields = serde_json::Map::new();
    fields.insert("proposerLamports".into(), json!(lamports));
    if let Some(minimum) = minimum {
        fields.insert("proposerMinLamports".into(), json!(minimum));
    }
    let threshold = minimum.unwrap_or(SQUADS_PROPOSER_LOW_LAMPORTS);
    if lamports < threshold {
        fields.insert("balance".into(), json!("low"));
        fields.insert(
            "balanceNote".into(),
            json!(format!(
                "The proposer holds {} SOL{}. It pays the fee and rent of every proposal, so fund it or proposals will fail.",
                lamports as f64 / 1e9,
                minimum
                    .map(|minimum| format!(" and one proposal needs {} SOL", minimum as f64 / 1e9))
                    .unwrap_or_default()
            )),
        );
    } else {
        fields.insert("balance".into(), json!("ok"));
    }
    fields
}

/// How to add the proposer to a multisig. Arete builds no Squads
/// transactions: on a dev environment `a4 dev squads add-proposer` builds
/// the config proposal locally; elsewhere the team uses the dashboard or
/// the Squads app, and code the squads-v4 program SDK.
pub fn squads_add_hint(dev: bool, multisig: Option<&str>, proposer: &str) -> String {
    let multisig = multisig.unwrap_or("<multisig>");
    if dev {
        format!(
            "In `a4 dev`, `a4 dev squads add-proposer --multisig {multisig} --proposer {proposer} --wallet <a dev wallet that can initiate>` opens the config proposal; then `a4 dev squads approve` and `a4 dev squads execute` it."
        )
    } else {
        format!(
            "Send the team to {} (any browser wallet, Ledger included), or the Squads app: Members → Add member → paste the proposer → permission Initiate only → approve and execute. {SQUADS_PROGRAM_SDK_HINT}",
            if multisig == "<multisig>" {
                SQUADS_DASHBOARD_URL.to_string()
            } else {
                squads_dashboard_url(Some(multisig))
            }
        )
    }
}

/// What to do after a Squads status answer, phrased for the MCP tools.
pub fn squads_status_next(status: &Value, dev: bool) -> String {
    if status["ready"] == true {
        return "The proposer can propose to this multisig. Keep it funded: it pays the fee and rent of every proposal.".into();
    }
    let multisig = status["multisig"].as_str();
    let proposer = status["proposer"].as_str().unwrap_or("<proposer>");
    match status["problem"].as_str() {
        Some("proposer_unfunded") => format!(
            "The proposer is a valid member but holds too little SOL to pay for a proposal{}. Send SOL to {} (in `a4 dev`, `a4 dev fund {}`), then check again.",
            status["proposerMinLamports"]
                .as_u64()
                .map(|minimum| format!(" (one needs {} SOL)", minimum as f64 / 1e9))
                .unwrap_or_default(),
            status["proposer"].as_str().unwrap_or("the proposer"),
            proposer
        ),
        Some("permissions_invalid") => format!(
            "The proposer is a member with permission mask {} instead of exactly Initiate (mask 1). Remove it with a config proposal in Squads, then add it again. {}",
            status["mask"],
            squads_add_hint(dev, multisig, proposer)
        ),
        _ => format!(
            "The proposer is not a member of this multisig. {}",
            squads_add_hint(dev, multisig, proposer)
        ),
    }
}

/// One JSON-RPC call to `url`; a JSON-RPC error is an error.
async fn rpc_call(http: &reqwest::Client, url: &str, method: &str, params: Value) -> Result<Value> {
    let response: Value = http
        .post(url)
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .send()
        .await
        .map_err(|error| anyhow!("The RPC is unreachable: {}", error.without_url()))?
        .json()
        .await
        .map_err(|error| {
            anyhow!(
                "The RPC returned a body that is not JSON: {}",
                error.without_url()
            )
        })?;
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        bail!("{method} failed: {error}");
    }
    Ok(response.get("result").cloned().unwrap_or(Value::Null))
}

fn is_loopback(origin: &str) -> bool {
    let rest = origin.split_once("://").map_or(origin, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    let host = if host.starts_with('[') {
        host.split(']')
            .next()
            .unwrap_or_default()
            .trim_start_matches('[')
    } else {
        host.split(':').next().unwrap_or_default()
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1") || host.starts_with("127.")
}

/// Whether `path` is a directory holding an `a4 dev` session.
pub fn has_dev_session(path: &Path) -> bool {
    DevProject::discover(Some(path)).is_ok()
}

mod account_squads;
pub use account_squads::{
    attachment_next, squads_app_steps, squads_dashboard_url, status_next as account_squads_next,
    SQUADS_DASHBOARD_URL, SQUADS_PROGRAM_SDK_HINT,
};

#[cfg(test)]
mod tests;
