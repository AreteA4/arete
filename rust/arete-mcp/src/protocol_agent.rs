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
use crate::protocol_api::{PrepareRequest, Prepared, ProtocolApi, TRANSACTION_SCOPES};
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
}

impl ProtocolAgent {
    pub fn new(signer: Option<LocalSigner>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
            registry: RegistryClient::new(),
            signer: signer.map(Arc::new),
            hosted: None,
            executions: Arc::new(DashMap::new()),
            instances: Arc::new(DashMap::new()),
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
        let dev_project = match (&args.project, args.dev) {
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
            .map(|url| Rpc::new(self.http.clone(), url));
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
        self.submit_and_wait(
            &target,
            &deployment,
            execution_id,
            &signed,
            remembered.as_ref(),
            wait,
            timeout,
        )
        .await
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
        let submitted = target.api.submit(&token, execution_id, signed).await?;
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
            .context("The RPC is unreachable")?
            .json()
            .await
            .context("The RPC returned a body that is not JSON")?;
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
    /// submit and wait.
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
        if let Err(refusal) = signer.check_target(&deployment, target.cluster) {
            return Ok(refused(&refusal, &deployment, target.cluster, None, None));
        }
        let wallet = signer.pubkey();
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
        let remembered = self
            .executions
            .get(&execution_id)
            .map(|entry| entry.clone());
        let mut result = self
            .submit_and_wait(
                &target,
                &deployment,
                &execution_id,
                &signed,
                remembered.as_ref(),
                wait,
                timeout,
            )
            .await?;
        result["summary"] = summary;
        Ok(result)
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

#[cfg(test)]
mod tests;
