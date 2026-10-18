//! Running steps and dev tools from the UI, through the code paths
//! `a4 dev step`, `a4 dev wallet` and `a4 dev time-travel` use
//! ([`DevClient`]). The server signs with dev wallets; no key ever reaches
//! the browser.
//!
//! - `GET /dev/v1/wallets`, `POST /dev/v1/wallets {name}`: list dev wallets
//!   with balances, create one (funded like `a4 dev wallet`).
//! - `POST /dev/v1/airdrop {wallet | pubkey, sol?}`: add environment SOL to
//!   a dev wallet or any address (the environment's `fund`).
//! - `POST /dev/v1/time-travel {to}`: move the environment's clock forward.
//! - `POST /dev/v1/steps/run {step, instance?, args, wallet}`: prepare, sign,
//!   submit and settle in one go (`a4 dev step`).
//! - `POST /dev/v1/steps/prepare {step, instance?, args, wallet}` then
//!   `POST /dev/v1/steps/submit {executionId}`: the same in two halves, so
//!   the UI can show the prepared transaction (accounts, instructions,
//!   anatomy, simulation) before it is signed. Prepared steps wait here (at
//!   most [`PENDING_LIMIT`], for [`PENDING_TTL`]).
//!
//! Every step run here is recorded in `.arete/dev/history.jsonl`, as
//! `a4 dev step` records its own.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arete_mcp::dev::{
    describe_transaction, list_wallets, DevClient, DevWallet, NotSubmitted, Prepared, PreparedStep,
    StepRequest, TimeTarget,
};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{client, error_response, UiState};

/// Prepared steps kept for a later submit.
pub const PENDING_LIMIT: usize = 64;
/// How long a prepared step waits (its intent expires on its own anyway).
pub const PENDING_TTL: Duration = Duration::from_secs(10 * 60);
const LAMPORTS_PER_SOL: u64 = 1_000_000_000;
const MAX_AIRDROP_SOL: f64 = 1_000.0;

/// Who signs a prepared step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signer {
    /// A dev wallet, by name: the server signs.
    Dev(String),
    /// A browser wallet, by public key: the browser signs.
    Browser(String),
}

pub struct PendingStep {
    pub request: StepRequest,
    pub signer: Signer,
    pub prepared: Prepared,
    pub at: Instant,
}

/// Prepared steps by execution id.
#[derive(Default)]
pub struct PendingSteps {
    steps: std::sync::Mutex<HashMap<String, PendingStep>>,
}

impl PendingSteps {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, PendingStep>> {
        self.steps
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn insert(&self, step: PendingStep) {
        let mut steps = self.lock();
        steps.retain(|_, pending| pending.at.elapsed() < PENDING_TTL);
        while steps.len() >= PENDING_LIMIT {
            let oldest = steps
                .iter()
                .min_by_key(|(_, pending)| pending.at)
                .map(|(id, _)| id.clone());
            match oldest {
                Some(id) => steps.remove(&id),
                None => break,
            };
        }
        steps.insert(step.prepared.execution_id.clone(), step);
    }

    /// Take the prepared step `execution_id` if `signer` prepared it.
    pub fn take(
        &self,
        execution_id: &str,
        signer: impl Fn(&Signer) -> bool,
    ) -> Option<PendingStep> {
        let mut steps = self.lock();
        let fresh = steps
            .get(execution_id)
            .is_some_and(|pending| pending.at.elapsed() < PENDING_TTL && signer(&pending.signer));
        if fresh {
            steps.remove(execution_id)
        } else {
            None
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    #[cfg(test)]
    pub fn contains(&self, execution_id: &str) -> bool {
        self.lock().contains_key(execution_id)
    }
}

pub fn routes() -> Router<Arc<UiState>> {
    Router::new()
        .route("/dev/v1/wallets", get(wallets).post(create_wallet))
        .route("/dev/v1/airdrop", post(airdrop))
        .route("/dev/v1/time-travel", post(time_travel))
        .route("/dev/v1/steps/run", post(run))
        .route("/dev/v1/steps/prepare", post(prepare))
        .route("/dev/v1/steps/submit", post(submit))
        .merge(super::wallet::routes())
}

pub(super) fn bad_request(error: impl std::fmt::Display) -> Response {
    error_response(StatusCode::BAD_REQUEST, "dev-ui-bad-request", error)
}

pub(super) fn failed(error: anyhow::Error) -> Response {
    error_response(StatusCode::BAD_GATEWAY, "dev-ui-step-failed", error)
}

async fn balances(client: &DevClient, pubkeys: &[String]) -> Vec<Value> {
    if pubkeys.is_empty() {
        return Vec::new();
    }
    match client
        .rpc(
            "getMultipleAccounts",
            json!([pubkeys, { "encoding": "base64", "dataSlice": { "offset": 0, "length": 0 } }]),
        )
        .await
    {
        Ok(result) => result["value"]
            .as_array()
            .map(|accounts| {
                accounts
                    .iter()
                    .map(|account| {
                        account["lamports"]
                            .as_u64()
                            .map(Value::from)
                            .unwrap_or(json!(0))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        Err(_) => vec![Value::Null; pubkeys.len()],
    }
}

async fn wallets(State(state): State<Arc<UiState>>) -> Response {
    let listed = list_wallets(&state.project.wallets_dir());
    let pubkeys: Vec<String> = listed.iter().map(|(_, pubkey)| pubkey.clone()).collect();
    let lamports = match client(&state) {
        Ok(client) => balances(&client, &pubkeys).await,
        Err(_) => vec![Value::Null; pubkeys.len()],
    };
    let wallets: Vec<Value> = listed
        .into_iter()
        .zip(lamports.into_iter().chain(std::iter::repeat(Value::Null)))
        .map(|((name, pubkey), lamports)| json!({ "name": name, "pubkey": pubkey, "lamports": lamports }))
        .collect();
    Json(json!({ "wallets": wallets })).into_response()
}

#[derive(Deserialize)]
struct WalletBody {
    name: String,
}

async fn create_wallet(
    State(state): State<Arc<UiState>>,
    Json(body): Json<WalletBody>,
) -> Response {
    if let Err(error) = arete_mcp::dev::validate_wallet_name(&body.name) {
        return bad_request(error);
    }
    let client = session!(state);
    match client.wallet(&body.name).await {
        Ok(wallet) => {
            let lamports = balances(&client, &[wallet.pubkey()]).await;
            Json(json!({ "name": wallet.name, "pubkey": wallet.pubkey(), "lamports": lamports.first() }))
                .into_response()
        }
        Err(error) => failed(error),
    }
}

#[derive(Deserialize)]
struct AirdropBody {
    wallet: Option<String>,
    pubkey: Option<String>,
    sol: Option<f64>,
}

/// The lamports an airdrop request asks for (default 10 SOL).
pub fn airdrop_lamports(sol: Option<f64>) -> Result<u64, String> {
    let sol = sol.unwrap_or(10.0);
    if !sol.is_finite() || sol <= 0.0 || sol > MAX_AIRDROP_SOL {
        return Err(format!(
            "sol must be more than 0 and at most {MAX_AIRDROP_SOL}"
        ));
    }
    Ok((sol * LAMPORTS_PER_SOL as f64).round() as u64)
}

/// Whether `text` is a base58 32-byte public key.
pub fn is_pubkey(text: &str) -> bool {
    bs58::decode(text)
        .into_vec()
        .is_ok_and(|bytes| bytes.len() == 32)
}

async fn airdrop(State(state): State<Arc<UiState>>, Json(body): Json<AirdropBody>) -> Response {
    let lamports = match airdrop_lamports(body.sol) {
        Ok(lamports) => lamports,
        Err(error) => return bad_request(error),
    };
    let pubkey = match (&body.wallet, &body.pubkey) {
        (Some(name), None) => match DevWallet::load_or_create(&state.project.wallets_dir(), name) {
            Ok((wallet, _)) => wallet.pubkey(),
            Err(error) => return bad_request(error),
        },
        (None, Some(pubkey)) if is_pubkey(pubkey) => pubkey.clone(),
        _ => return bad_request("Pass a dev wallet name as wallet or a public key as pubkey"),
    };
    let client = session!(state);
    // `fund` sets a balance; an airdrop adds to it.
    let current = match client
        .rpc("getBalance", json!([pubkey, { "commitment": "processed" }]))
        .await
    {
        Ok(result) => result["value"].as_u64().unwrap_or(0),
        Err(error) => return failed(error),
    };
    let target = current
        .saturating_add(lamports)
        .min(arete_mcp::dev_env::MAX_FUND_LAMPORTS);
    if let Err(error) = client.env.fund(&pubkey, target).await {
        return failed(error);
    }
    let balance = balances(&client, std::slice::from_ref(&pubkey)).await;
    state.hub.publish(
        "chain.airdrop",
        json!({ "pubkey": pubkey, "lamports": lamports }),
    );
    Json(json!({ "pubkey": pubkey, "airdropped": lamports, "lamports": balance.first() }))
        .into_response()
}

#[derive(Deserialize)]
struct TimeTravelBody {
    to: String,
}

async fn time_travel(
    State(state): State<Arc<UiState>>,
    Json(body): Json<TimeTravelBody>,
) -> Response {
    let target = match TimeTarget::parse(&body.to) {
        Ok(target) => target,
        Err(error) => return bad_request(error),
    };
    let client = session!(state);
    match client.time_travel(target).await {
        Ok(result) => Json(result).into_response(),
        Err(error) => failed(error),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepBody {
    pub step: String,
    #[serde(default)]
    pub instance: Option<String>,
    #[serde(default)]
    pub args: Option<Value>,
    #[serde(default)]
    pub wallet: Option<String>,
}

impl StepBody {
    pub fn request(&self, wallet: String) -> Result<StepRequest, Box<Response>> {
        let args = self.args.clone().unwrap_or_else(|| json!({}));
        if !args.is_object() {
            return Err(Box::new(bad_request("args must be a JSON object")));
        }
        Ok(StepRequest {
            step: self.step.clone(),
            instance: self
                .instance
                .clone()
                .filter(|instance| !instance.is_empty()),
            args,
            wallet,
            result: None,
        })
    }
}

async fn run(State(state): State<Arc<UiState>>, Json(body): Json<StepBody>) -> Response {
    let request = match body.request(body.wallet.clone().unwrap_or_else(|| "default".into())) {
        Ok(request) => request,
        Err(response) => return *response,
    };
    let client = session!(state);
    match client.step(&request).await {
        Ok(result) => Json(result).into_response(),
        Err(error) => failed(error),
    }
}

/// The preview of a prepared step: each transaction decoded, with the
/// runtime's anatomy and simulation of it.
pub fn preview(prepared: &Prepared, wallet: Value) -> Value {
    let response = &prepared.response;
    let transactions: Vec<Value> = prepared
        .transactions
        .iter()
        .enumerate()
        .map(|(index, bytes)| {
            let mut described = describe_transaction(bytes)
                .unwrap_or_else(|error| json!({ "error": format!("{error:#}") }));
            let pick = |list: &Value| {
                list.as_array()
                    .and_then(|items| {
                        items.iter().find(|item| {
                            item["index"].as_u64() == Some(index as u64)
                                || item["transaction"].as_u64() == Some(index as u64)
                        })
                    })
                    .cloned()
                    .or_else(|| list.get(index).cloned())
                    .unwrap_or(Value::Null)
            };
            described["anatomy"] = pick(&response["anatomy"]);
            described["simulation"] = pick(&response["simulation"]);
            described["transactionBase64"] =
                json!(base64::engine::general_purpose::STANDARD.encode(bytes));
            described
        })
        .collect();
    json!({
        "state": "prepared",
        "executionId": prepared.execution_id,
        "instanceId": prepared.instance_id,
        "wallet": wallet,
        "intent": response["intent"],
        "transactions": transactions,
        "simulation": response["simulation"],
        "anatomy": response["anatomy"],
    })
}

async fn prepare(State(state): State<Arc<UiState>>, Json(body): Json<StepBody>) -> Response {
    let name = body.wallet.clone().unwrap_or_else(|| "default".into());
    let request = match body.request(name.clone()) {
        Ok(request) => request,
        Err(response) => return *response,
    };
    let client = session!(state);
    let wallet = match client.wallet(&name).await {
        Ok(wallet) => wallet,
        Err(error) => return failed(error),
    };
    let identity = json!({ "name": wallet.name, "pubkey": wallet.pubkey() });
    match client.prepare_step(&request, &wallet.pubkey()).await {
        Ok(PreparedStep::Rejected(rejection)) => {
            let result = rejection.result(&request.step, identity);
            client.record_step(&request, &result);
            Json(result).into_response()
        }
        Ok(PreparedStep::Ready(prepared)) => {
            let body = preview(&prepared, identity);
            state.pending.insert(PendingStep {
                request,
                signer: Signer::Dev(name),
                prepared: *prepared,
                at: Instant::now(),
            });
            Json(body).into_response()
        }
        Err(error) => failed(error),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitBody {
    execution_id: String,
}

/// Whether `error`, from [`DevClient::submit_prepared`], came before the
/// Arete API took the signed transactions: the prepared step can still
/// be submitted, so it goes back to [`PendingSteps`].
pub(super) fn not_submitted(error: &anyhow::Error) -> bool {
    error.downcast_ref::<NotSubmitted>().is_some()
}

/// A prepared step stays prepared until it is submitted: every failure
/// before the runtime takes it (no session yet, a signing error, a refused
/// or unreachable submit) puts it back, so the UI can retry.
async fn submit(State(state): State<Arc<UiState>>, Json(body): Json<SubmitBody>) -> Response {
    let Some(pending) = state.pending.take(&body.execution_id, |signer| {
        matches!(signer, Signer::Dev(_))
    }) else {
        return error_response(
            StatusCode::NOT_FOUND,
            "dev-ui-not-prepared",
            "No prepared step with that execution id is waiting here; prepare it again",
        );
    };
    let Signer::Dev(name) = pending.signer.clone() else {
        state.pending.insert(pending);
        return bad_request("This step waits for a browser wallet's signature");
    };
    let client = match client(&state) {
        Ok(client) => client,
        Err(response) => {
            state.pending.insert(pending);
            return *response;
        }
    };
    let wallet = match DevWallet::load_or_create(&state.project.wallets_dir(), &name) {
        Ok((wallet, _)) => wallet,
        Err(error) => {
            state.pending.insert(pending);
            return failed(error);
        }
    };
    let mut signed = Vec::new();
    for bytes in &pending.prepared.transactions {
        match wallet.sign_transaction(bytes) {
            Ok(bytes) => signed.push(base64::engine::general_purpose::STANDARD.encode(bytes)),
            Err(error) => {
                state.pending.insert(pending);
                return failed(error);
            }
        }
    }
    let identity = json!({ "name": wallet.name, "pubkey": wallet.pubkey() });
    match client
        .submit_prepared(&pending.prepared, &pending.request.step, signed, identity)
        .await
    {
        Ok(result) => {
            client.record_step(&pending.request, &result);
            Json(result).into_response()
        }
        Err(error) => {
            if not_submitted(&error) {
                state.pending.insert(pending);
            }
            failed(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(id: &str, signer: Signer) -> PendingStep {
        PendingStep {
            request: StepRequest {
                step: "open".into(),
                instance: None,
                args: json!({}),
                wallet: "default".into(),
                result: None,
            },
            signer,
            prepared: Prepared {
                execution_id: id.into(),
                instance_id: None,
                token: "t".into(),
                transactions: vec![],
                response: json!({}),
            },
            at: Instant::now(),
        }
    }

    #[test]
    fn pending_steps_are_taken_once_by_their_signer_kind() {
        let steps = PendingSteps::default();
        steps.insert(pending("e1", Signer::Dev("alice".into())));
        steps.insert(pending("e2", Signer::Browser("Pk".into())));
        assert!(steps
            .take("e1", |signer| matches!(signer, Signer::Browser(_)))
            .is_none());
        assert!(steps
            .take("e1", |signer| matches!(signer, Signer::Dev(_)))
            .is_some());
        assert!(steps.take("e1", |_| true).is_none());
        for index in 0..PENDING_LIMIT + 5 {
            steps.insert(pending(&format!("x{index}"), Signer::Dev("a".into())));
        }
        assert_eq!(steps.len(), PENDING_LIMIT);
    }

    #[test]
    fn validates_airdrops_and_public_keys() {
        assert_eq!(airdrop_lamports(None), Ok(10 * LAMPORTS_PER_SOL));
        assert_eq!(airdrop_lamports(Some(0.5)), Ok(LAMPORTS_PER_SOL / 2));
        assert!(airdrop_lamports(Some(0.0)).is_err());
        assert!(airdrop_lamports(Some(5_000.0)).is_err());
        assert!(airdrop_lamports(Some(f64::NAN)).is_err());
        assert!(is_pubkey("11111111111111111111111111111111"));
        assert!(!is_pubkey("not a key"));
    }

    #[test]
    fn step_bodies_need_object_args() {
        let body: StepBody =
            serde_json::from_value(json!({ "step": "close", "instance": "", "args": [1] }))
                .unwrap();
        assert!(body.request("w".into()).is_err());
        let body: StepBody =
            serde_json::from_value(json!({ "step": "close", "instance": "" })).unwrap();
        let request = body.request("w".into()).unwrap();
        assert!(request.instance.is_none());
        assert_eq!(request.args, json!({}));
    }
}
