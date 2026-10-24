//! The protocol API calls shared by `a4 dev` and the hosted protocol tools.
//!
//! A [`ProtocolApi`] talks to one protocol API (`<origin>/api`): Arete's, or
//! the local runtime of an `a4 dev` session. It mints protocol sessions for
//! a deployment from an API key through the session endpoint and reads and
//! writes the protocol routes with them:
//!
//! - wallet actions, instance lists and instance reads;
//! - prepare (a guard, access or state rejection is an outcome, not an error);
//! - submit and the execution intent, with a bounded wait for it to settle.
//!
//! The key and the minted session tokens are only ever sent as bearer
//! credentials; nothing here returns or logs them.

use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

/// Scopes for reads.
pub const READ_SCOPES: [&str; 1] = ["protocol:read"];
/// Scopes for prepare and submit (and the reads around them).
pub const TRANSACTION_SCOPES: [&str; 3] =
    ["protocol:read", "protocol:submit", "protocol:prepare:*"];
/// Intent states after which nothing changes.
pub const FINAL_INTENT_STATES: [&str; 4] = ["completed", "chain_failed", "expired", "rejected"];
/// The access rejection codes a session mint or a prepare may answer with.
const ACCESS_CODES: [&str; 3] = [
    "step_requires_secret_key",
    "step_not_in_session_scope",
    "step_not_client_callable",
];

/// One protocol API and the key that mints its sessions.
#[derive(Clone)]
pub struct ProtocolApi {
    http: reqwest::Client,
    /// `<origin>/api`.
    api_base: String,
    session_endpoint: String,
    key: String,
}

impl std::fmt::Debug for ProtocolApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtocolApi")
            .field("api_base", &self.api_base)
            .field("session_endpoint", &self.session_endpoint)
            .finish_non_exhaustive()
    }
}

/// What a prepare produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Prepared {
    /// The prepare body: `intent`, `transactions`, `anatomy`, `simulation`.
    Intent(Value),
    /// A guard, access or state rejection: `{rejected, step, guard?, detail?}`,
    /// plus a `hint` for state rejections.
    Rejected(Value),
}

/// An execution intent after a bounded wait.
#[derive(Debug, Clone, PartialEq)]
pub struct Settled {
    pub state: Value,
    /// False when the wait ended before the intent reached a final state.
    pub settled: bool,
}

/// One prepare request.
#[derive(Debug, Clone)]
pub struct PrepareRequest<'a> {
    pub deployment: &'a str,
    /// The instance for instance and participant steps; `None` for create steps.
    pub instance: Option<&'a str>,
    pub step: &'a str,
    pub args: &'a Value,
    pub wallet: &'a str,
    /// `v0` or `v1`.
    pub transaction_format: &'a str,
}

impl ProtocolApi {
    /// `api_base` is the protocol API (`<origin>/api`); `key` mints sessions
    /// at `session_endpoint`.
    pub fn new(http: reqwest::Client, api_base: &str, session_endpoint: &str, key: &str) -> Self {
        Self {
            http,
            api_base: api_base.trim_end_matches('/').to_string(),
            session_endpoint: session_endpoint.to_string(),
            key: key.to_string(),
        }
    }

    pub fn api_base(&self) -> &str {
        &self.api_base
    }

    /// A protocol session token for `deployment` with `scopes`. An access
    /// refusal of the mint carries its code (see [`access_code`]).
    pub async fn token(&self, deployment: &str, scopes: &[&str]) -> Result<String> {
        let response = self
            .http
            .post(&self.session_endpoint)
            .bearer_auth(&self.key)
            .json(&json!({
                "targetKind": "protocol-deployment",
                "targetId": deployment,
                "scopes": scopes,
            }))
            .send()
            .await
            .with_context(|| {
                format!(
                    "The session endpoint {} is unreachable",
                    self.session_endpoint
                )
            })?;
        let status = response.status();
        let code = response
            .headers()
            .get("x-error-code")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let code = code
                .or_else(|| body.get("code").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            return Err(anyhow!(MintRefused {
                status: status.as_u16(),
                code,
                body,
            }));
        }
        body.get("token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("The session endpoint returned no token"))
    }

    /// One call to `<api_base><path>` with `token`.
    pub async fn api_json(
        &self,
        method: reqwest::Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> Result<(reqwest::StatusCode, Value)> {
        let url = format!("{}{path}", self.api_base);
        let mut request = self.http.request(method, &url).bearer_auth(token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("The protocol API is unreachable at {url}"))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        Ok((status, value))
    }

    /// A read with a read-only session for `deployment`; non-2xx is an error.
    pub async fn read(&self, deployment: &str, path: &str, what: &str) -> Result<Value> {
        let token = self.token(deployment, &READ_SCOPES).await?;
        let (status, body) = self
            .api_json(reqwest::Method::GET, path, &token, None)
            .await?;
        if !status.is_success() {
            bail!("{what} failed ({status}): {body}");
        }
        Ok(body)
    }

    /// What `wallet` can do now on `deployment`: create steps and the
    /// instances it relates to, each with its steps' availability.
    pub async fn wallet_actions(
        &self,
        deployment: &str,
        wallet: &str,
        instance: Option<&str>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Value> {
        let mut query = Vec::new();
        if let Some(instance) = instance {
            query.push(format!("instance={}", query_value(instance)));
        }
        if let Some(cursor) = cursor {
            query.push(format!("cursor={}", query_value(cursor)));
        }
        if let Some(limit) = limit {
            query.push(format!("limit={limit}"));
        }
        let path = format!(
            "/protocol-deployments/{}/wallets/{}/actions{}",
            path_segment(deployment)?,
            path_segment(wallet)?,
            query_string(&query)
        );
        self.read(deployment, &path, "Reading wallet actions").await
    }

    /// One page of `deployment`'s instances, newest first.
    pub async fn instances(
        &self,
        deployment: &str,
        state: Option<&str>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Value> {
        let mut query = Vec::new();
        if let Some(state) = state {
            query.push(format!("state={}", query_value(state)));
        }
        if let Some(cursor) = cursor {
            query.push(format!("cursor={}", query_value(cursor)));
        }
        if let Some(limit) = limit {
            query.push(format!("limit={limit}"));
        }
        let path = format!(
            "/protocol-deployments/{}/instances{}",
            path_segment(deployment)?,
            query_string(&query)
        );
        self.read(deployment, &path, "Listing instances").await
    }

    /// One instance; with `wallet`, its `allowedSteps` are evaluated for it.
    pub async fn instance(
        &self,
        deployment: &str,
        instance: &str,
        wallet: Option<&str>,
    ) -> Result<Value> {
        let query = match wallet {
            Some(wallet) => format!("?wallet={}", query_value(wallet)),
            None => String::new(),
        };
        let path = format!("/protocol-instances/{}{query}", path_segment(instance)?);
        self.read(deployment, &path, &format!("Reading instance {instance}"))
            .await
    }

    /// Prepare one step. Guard, access and state rejections are
    /// [`Prepared::Rejected`]; other failures are errors.
    pub async fn prepare(&self, request: &PrepareRequest<'_>) -> Result<Prepared> {
        let token = match self.token(request.deployment, &TRANSACTION_SCOPES).await {
            Ok(token) => token,
            Err(error) => {
                return match access_code(&error) {
                    Some(code) => Ok(Prepared::Rejected(json!({
                        "rejected": code,
                        "step": request.step,
                    }))),
                    None => Err(error),
                }
            }
        };
        self.prepare_with(&token, request).await
    }

    /// [`Self::prepare`] with an already minted transaction session.
    pub async fn prepare_with(
        &self,
        token: &str,
        request: &PrepareRequest<'_>,
    ) -> Result<Prepared> {
        let path = match request.instance.filter(|id| !id.is_empty()) {
            Some(instance) => format!(
                "/protocol-instances/{}/steps/{}/prepare",
                path_segment(instance)?,
                path_segment(request.step)?
            ),
            None => format!(
                "/protocol-deployments/{}/steps/{}/prepare",
                path_segment(request.deployment)?,
                path_segment(request.step)?
            ),
        };
        let args = match request.args {
            Value::Null => json!({}),
            Value::Object(_) => request.args.clone(),
            _ => bail!("step args must be a JSON object"),
        };
        let (status, prepared) = self
            .api_json(
                reqwest::Method::POST,
                &path,
                token,
                Some(json!({
                    "idempotencyKey": uuid::Uuid::new_v4().to_string(),
                    "wallet": request.wallet,
                    "args": args,
                    "transactionFormat": request.transaction_format,
                })),
            )
            .await?;
        if status.as_u16() == 422 || status.as_u16() == 403 {
            if let Some(code) = prepared
                .get("rejected")
                .or_else(|| prepared.get("code"))
                .and_then(Value::as_str)
            {
                let mut rejection = json!({
                    "rejected": code,
                    "step": prepared.get("step").and_then(Value::as_str).unwrap_or(request.step),
                });
                for field in ["guard", "detail"] {
                    if let Some(value) = prepared.get(field).filter(|value| !value.is_null()) {
                        rejection[field] = value.clone();
                    }
                }
                if let Some(hint) = state_rejection_hint(code) {
                    rejection["hint"] = json!(hint);
                }
                return Ok(Prepared::Rejected(rejection));
            }
        }
        if !status.is_success() {
            bail!(
                "Prepare of '{}' failed ({status}): {prepared}",
                request.step
            );
        }
        if prepared["intent"]["executionId"].as_str().is_none() {
            bail!("Prepare returned no intent.executionId: {prepared}");
        }
        if prepared["transactions"]
            .as_array()
            .is_none_or(|list| list.is_empty())
        {
            bail!("Prepare returned no transactions");
        }
        Ok(Prepared::Intent(prepared))
    }

    /// Submit the signed transactions of a prepared intent.
    pub async fn submit(
        &self,
        token: &str,
        execution_id: &str,
        signed_transactions: &[String],
    ) -> Result<Value> {
        let (status, submitted) = self
            .api_json(
                reqwest::Method::POST,
                &format!("/execution-intents/{}/submit", path_segment(execution_id)?),
                token,
                Some(json!({ "signedTransactions": signed_transactions })),
            )
            .await?;
        if !status.is_success() {
            bail!("Submit of execution {execution_id} failed ({status}): {submitted}");
        }
        Ok(submitted)
    }

    /// The execution intent's current state.
    pub async fn intent(&self, token: &str, execution_id: &str) -> Result<Value> {
        let (status, body) = self
            .api_json(
                reqwest::Method::GET,
                &format!("/execution-intents/{}", path_segment(execution_id)?),
                token,
                None,
            )
            .await?;
        if !status.is_success() {
            bail!("Reading execution {execution_id} failed ({status}): {body}");
        }
        Ok(body)
    }

    /// Poll the intent from `state` until it settles or `timeout` passes.
    pub async fn wait_intent(
        &self,
        token: &str,
        execution_id: &str,
        mut state: Value,
        timeout: Duration,
    ) -> Result<Settled> {
        let deadline = Instant::now() + timeout;
        loop {
            if is_final(&state) {
                return Ok(Settled {
                    state,
                    settled: true,
                });
            }
            if Instant::now() >= deadline {
                return Ok(Settled {
                    state,
                    settled: false,
                });
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            let (status, body) = self
                .api_json(
                    reqwest::Method::GET,
                    &format!("/execution-intents/{}", path_segment(execution_id)?),
                    token,
                    None,
                )
                .await?;
            if status.is_success() {
                state = body;
            }
        }
    }
}

/// The Squads onboarding routes of a deployment
/// (`/protocol-deployments/{d}/squads/*`), read with a read-only session.
impl ProtocolApi {
    async fn squads(
        &self,
        deployment: &str,
        method: reqwest::Method,
        route: &str,
        body: Option<Value>,
        what: &str,
    ) -> Result<Value> {
        let token = self.token(deployment, &READ_SCOPES).await?;
        let path = format!(
            "/protocol-deployments/{}/squads/{route}",
            path_segment(deployment)?
        );
        let url = format!("{}{path}", self.api_base);
        let mut request = self.http.request(method, &url).bearer_auth(&token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("The protocol API is unreachable at {url}"))?;
        let status = response.status();
        let header_code = response
            .headers()
            .get("x-error-code")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let text = response.text().await.unwrap_or_default();
        let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
        if !status.is_success() {
            let code = header_code
                .filter(|code| !code.trim().is_empty())
                .or_else(|| body.get("code").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            return Err(anyhow!(SquadsRefused {
                what: what.to_string(),
                status: status.as_u16(),
                code,
                body,
            }));
        }
        Ok(body)
    }

    /// The calling account's Squads proposer for `deployment`:
    /// `{pubkey, permissions: "initiate", rootKeyId}`.
    pub async fn squads_proposer(&self, deployment: &str) -> Result<Value> {
        self.squads(
            deployment,
            reqwest::Method::GET,
            "proposer",
            None,
            "Reading the Squads proposer",
        )
        .await
    }

    /// Whether the proposer is an Initiate-only member of `multisig`:
    /// `{ready, member, mask, threshold, multisig, proposer, problem?}`.
    pub async fn squads_status(&self, deployment: &str, multisig: &str) -> Result<Value> {
        self.squads(
            deployment,
            reqwest::Method::GET,
            &format!("status?multisig={}", query_value(multisig)),
            None,
            "Reading the Squads proposer status",
        )
        .await
    }

    /// An unsigned transaction creating a config proposal that adds the
    /// proposer to `multisig` with Initiate only, for `member` to sign,
    /// send and pay.
    pub async fn squads_add_proposer(
        &self,
        deployment: &str,
        multisig: &str,
        member: &str,
        transaction_format: Option<&str>,
    ) -> Result<Value> {
        let mut body = json!({ "multisig": multisig, "member": member });
        if let Some(format) = transaction_format {
            body["transactionFormat"] = json!(format);
        }
        self.squads(
            deployment,
            reqwest::Method::POST,
            "add-proposer",
            Some(body),
            "Preparing the add-proposer proposal",
        )
        .await
    }
}

/// A Squads onboarding route refused the request.
#[derive(Debug)]
pub struct SquadsRefused {
    pub what: String,
    pub status: u16,
    pub code: String,
    pub body: Value,
}

impl SquadsRefused {
    /// The code, lowercase with `_` separators.
    pub fn normalized_code(&self) -> String {
        self.code.trim().to_ascii_lowercase().replace('-', "_")
    }
}

impl std::fmt::Display for SquadsRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = self.normalized_code();
        if code == "protocol_squads_setup_required" {
            return f.write_str(&squads_setup_required_text(&self.body));
        }
        match squads_refusal_hint(&code) {
            Some(hint) => write!(
                f,
                "{} refused ({}, {}): {hint}",
                self.what, self.status, self.code
            ),
            None => {
                let message = self
                    .body
                    .get("error")
                    .or_else(|| self.body.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| self.body.to_string());
                write!(f, "{} failed ({}): {message}", self.what, self.status)
            }
        }
    }
}

impl std::error::Error for SquadsRefused {}

/// What to do about a Squads onboarding refusal `code` (normalized by
/// [`SquadsRefused::normalized_code`]); `None` for other codes.
pub fn squads_refusal_hint(code: &str) -> Option<&'static str> {
    Some(match code {
        "member_cannot_initiate" => {
            "`member` is not a member of this multisig with the Initiate permission. Pass a member who can initiate proposals: it signs, sends and pays for the add-proposer proposal."
        }
        "proposer_already_member" => {
            "The proposer is already a member of this multisig. Check its status; there is nothing to add."
        }
        "proposer_permissions_invalid" => {
            "The proposer is a member of this multisig with permissions other than Initiate only. Remove it from the multisig with a config proposal in Squads, then add it again."
        }
        "squads_multisig_controlled" => {
            "This multisig has a config authority, so members cannot change it by proposal. Its config authority must add the proposer (Initiate only) directly."
        }
        "squads_multisig_missing" => {
            "No Squads v4 multisig exists at that address. Pass the multisig account, not one of its vaults."
        }
        "protocol_squads_not_used" => {
            "This deployment has no step whose authority is a Squads proposer, so it needs no proposer."
        }
        "squads_multisig_invalid" => {
            "That account is not a Squads v4 multisig. Pass the multisig account, not one of its vaults."
        }
        "protocol_squads_multisig_not_found" => {
            "This multisig is not attached to your account. Attach it first (squads_attach, `a4 squads attach <multisig>`, or https://app.arete.run/dashboard/squads)."
        }
        "protocol_squads_multisig_active" => {
            "This multisig is active (Arete's proposer is a member), so it cannot be forgotten. Remove the proposer first, then forget it."
        }
        "protocol_squads_transaction_modified" => {
            "The signed transaction is not the one prepared (a wallet may have added instructions). Sign the prepared bytes as they are, or prepare a new transaction."
        }
        _ => return None,
    })
}

/// The text of a `protocol-squads-setup-required` refusal (a production
/// deployment whose Squads multisig is not an active attachment of the
/// account): the server's message, then the dashboard link and the
/// commands.
pub fn squads_setup_required_text(body: &Value) -> String {
    let details = &body["details"];
    let message = body
        .get("message")
        .or_else(|| body.get("error"))
        .and_then(Value::as_str)
        .unwrap_or("A Squads step's multisig is not set up for your account.");
    let multisig = details["multisig"].as_str();
    let setup_url = details["setupUrl"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| arete_sdk::protocol::squads_dashboard_url(multisig));
    let mut text = format!("{message}\n  Set it up: {setup_url}");
    if let Some(proposer) = details["proposer"].as_str() {
        text.push_str(&format!(
            "\n  Or in the Squads app: Members → Add member → {proposer} → permission Initiate only → approve and execute"
        ));
    }
    if let Some(command) = details["commands"]["addProposer"].as_str() {
        text.push_str(&format!("\n  Hot wallet: {command} --keypair <file>"));
    }
    if let Some(command) = details["commands"]["status"].as_str() {
        text.push_str(&format!("\n  Check: {command}"));
    }
    text
}

/// Whether an intent state is final.
pub fn is_final(state: &Value) -> bool {
    state["state"]
        .as_str()
        .is_some_and(|value| FINAL_INTENT_STATES.contains(&value))
}

/// A session mint the server refused.
#[derive(Debug)]
pub struct MintRefused {
    pub status: u16,
    pub code: String,
    pub body: Value,
}

impl std::fmt::Display for MintRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Minting a protocol session failed ({}): {}",
            self.status, self.body
        )
    }
}

impl std::error::Error for MintRefused {}

/// The access rejection code of a refused session mint, if that is what
/// `error` is.
pub fn access_code(error: &anyhow::Error) -> Option<String> {
    let refused = error.downcast_ref::<MintRefused>()?;
    let code = refused.code.trim().to_ascii_lowercase().replace('-', "_");
    ACCESS_CODES.contains(&code.as_str()).then_some(code)
}

/// What an agent should do after a state rejection (one of
/// [`crate::protocol::STATE_REJECTION_CODES`]); `None` for any other code.
pub fn state_rejection_hint(code: &str) -> Option<&'static str> {
    match code.trim().to_ascii_lowercase().as_str() {
        "instance_terminal" => Some(
            "The instance has finished (detail.state is its terminal state): no instance or \
             participant step can run on it. Do not retry; re-check protocol_actions for what \
             the wallet can do now.",
        ),
        "participant_terminal" => Some(
            "This wallet's participant has finished (detail.state is its terminal state): its \
             participant steps cannot run. Do not retry; re-check protocol_actions for what \
             the wallet can do now.",
        ),
        _ => None,
    }
}

/// `value` when it is a safe id for a path segment.
pub fn path_segment(value: &str) -> Result<String> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        bail!("'{value}' is not a valid id");
    }
    Ok(value.to_string())
}

fn query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn query_string(parts: &[String]) -> String {
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_queries_and_refuses_unsafe_segments() {
        assert_eq!(query_value("a b/c"), "a%20b%2Fc");
        assert_eq!(query_string(&[]), "");
        assert_eq!(query_string(&["a=1".into(), "b=2".into()]), "?a=1&b=2");
        assert!(path_segment("../x").is_err());
        assert!(path_segment("").is_err());
        assert_eq!(path_segment("d-1").unwrap(), "d-1");
    }

    #[test]
    fn mint_refusals_carry_access_codes() {
        let error = anyhow!(MintRefused {
            status: 403,
            code: "STEP-NOT-IN-SESSION-SCOPE".into(),
            body: Value::Null,
        });
        assert_eq!(
            access_code(&error).as_deref(),
            Some("step_not_in_session_scope")
        );
        let other = anyhow!(MintRefused {
            status: 401,
            code: "token-invalid".into(),
            body: Value::Null,
        });
        assert_eq!(access_code(&other), None);
        assert_eq!(access_code(&anyhow!("plain")), None);
    }

    #[test]
    fn state_rejections_carry_a_hint() {
        for code in crate::protocol::STATE_REJECTION_CODES {
            let hint = state_rejection_hint(code).unwrap();
            assert!(hint.contains("protocol_actions"), "{hint}");
            assert!(hint.contains("detail.state"), "{hint}");
            // A state code is never a mint refusal.
            let refused = anyhow!(MintRefused {
                status: 403,
                code: code.into(),
                body: Value::Null,
            });
            assert_eq!(access_code(&refused), None);
        }
        assert!(state_rejection_hint("participant_terminal")
            .unwrap()
            .contains("participant"));
        assert_eq!(state_rejection_hint("deadline_passed"), None);
        assert_eq!(state_rejection_hint("step_requires_secret_key"), None);
    }
}
