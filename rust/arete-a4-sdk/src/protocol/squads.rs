//! Account Squads: the Squads v4 multisigs an Arete account has attached.
//!
//! Admin steps of a protocol are proposed to a multisig attached to the
//! deployment owner's account, by a proposer key that belongs to the account
//! and may only initiate. The account attaches each multisig once (or
//! creates one) and adds the proposer as an Initiate-only member; production
//! deployments refuse a Squads step whose multisig is not an `active`
//! attachment ([`SQUADS_SETUP_REQUIRED_CODE`]).
//!
//! The easiest path is the dashboard ([`SQUADS_DASHBOARD_URL`]), which signs
//! with any standard browser wallet, hardware wallets included. This client
//! is for scripts and hot wallets: every write returns an unsigned
//! transaction for one wallet to sign, and [`SquadsClient::submit`] sends
//! and confirms it. Arete never holds or asks for a member key.
//!
//! Mirror of `@usearete/sdk`'s `createSquadsClient`.

use std::sync::Arc;

use base64::Engine as _;
use reqwest::header::{HeaderValue, AUTHORIZATION};
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::client::{ProtocolError, ARETE_API_URL_ENV};
use super::types::ProtocolTransactionFormat;
use crate::auth::{classify_api_key, ApiKeyClass, AuthConfig, TokenProvider};
use crate::error::AreteError;
use crate::http::HEADER_ERROR_CODE;

/// The dashboard page that sets up and manages the account's multisigs.
pub const SQUADS_DASHBOARD_URL: &str = "https://app.arete.run/dashboard/squads";

/// The wire code of a deployment create or activation refused because a
/// Squads step's multisig is not an `active` attachment of the account.
pub const SQUADS_SETUP_REQUIRED_CODE: &str = "protocol-squads-setup-required";

const DEFAULT_API_URL: &str = "https://api.arete.run/api";

/// The dashboard page for the account's multisigs, or for one of them.
pub fn squads_dashboard_url(multisig: Option<&str>) -> String {
    match multisig.map(str::trim).filter(|m| !m.is_empty()) {
        Some(multisig) => format!("{SQUADS_DASHBOARD_URL}/{multisig}"),
        None => SQUADS_DASHBOARD_URL.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// `notStarted`: nothing attached; `ready`: one is `active`;
/// `needsAttention`: attachments exist and none is `active`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SquadsSetup {
    NotStarted,
    Ready,
    NeedsAttention,
}

/// `active` while the proposer is an Initiate-only member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SquadsAttachmentState {
    Pending,
    Active,
    Archived,
}

/// The account's proposer. It proposes; it never votes or executes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsAccountProposer {
    pub pubkey: String,
    /// Always `initiate`.
    pub permissions: String,
    /// The balance, for information; `None` when the chain did not answer.
    #[serde(default)]
    pub lamports: Option<u64>,
}

/// A multisig member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsMember {
    pub pubkey: String,
    /// The Squads permission mask (Initiate 1, Vote 2, Execute 4).
    pub mask: u8,
    /// `initiate`, `vote`, `execute`.
    #[serde(default)]
    pub permissions: Vec<String>,
}

/// A production deployment whose Squads steps propose to the multisig (as
/// the deploy gate last resolved them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsAttachmentDeployment {
    pub id: String,
    pub protocol_id: String,
    #[serde(default)]
    pub protocol_name: String,
    #[serde(default)]
    pub protocol_slug: String,
    pub status: String,
    /// The Squads steps that propose to it.
    #[serde(default)]
    pub steps: Vec<String>,
}

/// A deployment named by a [`SquadsNotification`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsNotificationDeployment {
    pub id: String,
    #[serde(default)]
    pub protocol_name: Option<String>,
    #[serde(default)]
    pub steps: Vec<String>,
}

/// An `archived` attachment that was `active` (the proposer was removed or
/// its permissions changed), with the deployments that used it then. It
/// goes away once the attachment is active again or forgotten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsNotification {
    pub multisig: String,
    #[serde(default)]
    pub problem: Option<String>,
    #[serde(default)]
    pub problem_description: Option<String>,
    #[serde(default)]
    pub deployments: Vec<SquadsNotificationDeployment>,
    /// RFC 3339.
    pub since: String,
    pub dashboard_url: String,
}

/// A multisig attached to the account, refreshed from the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsAttachment {
    pub address: String,
    #[serde(default)]
    pub label: Option<String>,
    pub state: SquadsAttachmentState,
    /// Why it is not `active`: `not_member`, `permissions_invalid`,
    /// `squads_multisig_missing`, `squads_multisig_invalid` or
    /// `create_unconfirmed`.
    #[serde(default)]
    pub problem: Option<String>,
    /// The problem as a plain sentence.
    #[serde(default)]
    pub problem_description: Option<String>,
    /// `page` (attached by address), `created` or `deployment`.
    pub created_via: String,
    /// RFC 3339.
    pub state_changed_at: String,
    /// RFC 3339.
    pub created_at: String,
    /// `None` when the account could not be read.
    #[serde(default)]
    pub threshold: Option<u16>,
    #[serde(default)]
    pub members: Option<Vec<SquadsMember>>,
    /// The proposer's mask, when it is a member.
    #[serde(default)]
    pub proposer_mask: Option<u8>,
    /// Vault 0.
    pub vault: String,
    /// The multisig in the Squads app.
    pub squads_url: String,
    /// The multisig's dashboard page.
    pub dashboard_url: String,
    /// The deployment-config snippet that names this multisig.
    pub deployment_config_snippet: String,
    #[serde(default)]
    pub deployments: Vec<SquadsAttachmentDeployment>,
}

/// A config proposal Arete built for the multisig (adding or removing the
/// proposer), with its on-chain status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsConfigProposal {
    /// `addProposer` or `removeProposer`.
    pub kind: String,
    pub transaction_index: u64,
    pub proposal: String,
    /// `draft`, `active`, `approved`, `executed`, `rejected`, `cancelled`,
    /// `executing` or `missing`.
    pub status: String,
    #[serde(default)]
    pub approvals: Vec<String>,
    #[serde(default)]
    pub threshold: Option<u16>,
    pub squads_url: String,
}

/// A proposal the account's proposer made for a deployment's admin step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsStepProposal {
    pub deployment_id: String,
    pub step: String,
    #[serde(default)]
    pub instance_id: Option<String>,
    pub status: String,
    pub transaction_index: u64,
    pub proposal: String,
    pub created_at: String,
    pub squads_url: String,
}

/// One attachment with the proposals Arete made for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsAttachmentDetail {
    #[serde(flatten)]
    pub attachment: SquadsAttachment,
    /// The account's proposer pubkey.
    #[serde(default)]
    pub proposer: String,
    #[serde(default)]
    pub config_proposals: Vec<SquadsConfigProposal>,
    #[serde(default)]
    pub proposals: Vec<SquadsStepProposal>,
}

/// The account's Squads setup (`GET /account/squads`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsAccountStatus {
    pub setup: SquadsSetup,
    pub proposer: SquadsAccountProposer,
    pub dashboard_url: String,
    #[serde(default)]
    pub multisigs: Vec<SquadsAttachment>,
}

/// An unsigned transaction for one wallet (`signer`, which also pays) to
/// sign. Pass the signed bytes and `transaction_id` to
/// [`SquadsClient::submit`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsPreparedTransaction {
    pub transaction_id: String,
    /// `createMultisig`, `addProposer`, `approve`, `execute` or
    /// `removeProposer`.
    pub kind: String,
    /// Wire bytes; the signer's signature slot is zeroed.
    pub transaction_base64: String,
    pub transaction_format: ProtocolTransactionFormat,
    pub signer: String,
    pub last_valid_block_height: u64,
    /// The multisig (for `createMultisig`, the new address).
    pub multisig: String,
    #[serde(default)]
    pub transaction_index: Option<u64>,
    #[serde(default)]
    pub proposal: Option<String>,
    /// What to do once it confirms.
    #[serde(default)]
    pub next: String,
}

impl SquadsPreparedTransaction {
    /// The wire bytes to sign.
    pub fn transaction_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        base64::engine::general_purpose::STANDARD
            .decode(self.transaction_base64.trim())
            .map_err(|error| {
                ProtocolError::UnexpectedResponse(format!(
                    "the prepared transaction is not base64: {error}"
                ))
            })
    }
}

/// `confirmed`, `failed`, `pending` (sent, not confirmed yet: submit the
/// same body again) or `expired` (prepare a new one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SquadsSubmitStatus {
    Confirmed,
    Failed,
    Pending,
    Expired,
}

/// What `POST /account/squads/transactions` answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsSubmitResult {
    pub transaction_id: String,
    #[serde(default)]
    pub signature: Option<String>,
    pub status: SquadsSubmitStatus,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default)]
    pub multisig: Option<SquadsAttachment>,
}

/// A member of a multisig to create.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsCreateMember {
    pub pubkey: String,
    /// Any of `initiate`, `vote`, `execute`.
    pub permissions: Vec<String>,
}

impl SquadsCreateMember {
    pub fn new<I, S>(pubkey: impl Into<String>, permissions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            pubkey: pubkey.into(),
            permissions: permissions.into_iter().map(Into::into).collect(),
        }
    }
}

/// Body of `POST /account/squads/multisigs/create`. The proposer is added
/// as an Initiate-only member by the server and must not be listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsCreateRequest {
    pub members: Vec<SquadsCreateMember>,
    pub threshold: u16,
    /// Signs and pays (creation fee and rent); must be a member.
    pub creator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_format: Option<ProtocolTransactionFormat>,
}

/// The 409 [`SQUADS_SETUP_REQUIRED_CODE`] `details`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsSetupRequiredDetails {
    pub step: String,
    pub multisig: String,
    pub state: SquadsAttachmentState,
    #[serde(default)]
    pub problem: Option<String>,
    pub proposer: String,
    pub setup_url: String,
    #[serde(default)]
    pub commands: Option<SquadsSetupCommands>,
}

/// The CLI commands a setup refusal suggests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadsSetupCommands {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub add_proposer: Option<String>,
}

/// A deployment refused because a Squads step's multisig is not set up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquadsSetupRequired {
    pub message: String,
    pub details: SquadsSetupRequiredDetails,
}

/// The setup refusal a failed deployment create or activation carries, if
/// it is one: a [`ProtocolError`] (or raw response body) with code
/// [`SQUADS_SETUP_REQUIRED_CODE`].
pub fn squads_setup_required(body: &Value) -> Option<SquadsSetupRequired> {
    let code = body.get("code").and_then(Value::as_str)?;
    if !code.trim().eq_ignore_ascii_case(SQUADS_SETUP_REQUIRED_CODE) {
        return None;
    }
    let details = serde_json::from_value(body.get("details")?.clone()).ok()?;
    let message = body
        .get("message")
        .or_else(|| body.get("error"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(SquadsSetupRequired { message, details })
}

impl ProtocolError {
    /// The Squads setup refusal this error carries, if it is one.
    pub fn squads_setup_required(&self) -> Option<SquadsSetupRequired> {
        match self {
            Self::Request {
                body: Some(body), ..
            } => squads_setup_required(body),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// How a [`SquadsClient`] authenticates and where it calls.
#[derive(Clone, Default)]
pub struct SquadsClientOptions {
    /// A secret or agent key (`a4_sk_...`, `a4_ak_...`), a token, or a token
    /// provider. When unset, `ARETE_API_KEY`, then the active `a4` login,
    /// supplies the key. Publishable keys are refused.
    pub auth: Option<AuthConfig>,
    /// The API, `<origin>/api`. Defaults to [`ARETE_API_URL_ENV`] (the
    /// origin) plus `/api`, then `https://api.arete.run/api`.
    pub api_url: Option<String>,
    pub http_client: Option<reqwest::Client>,
}

impl std::fmt::Debug for SquadsClientOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SquadsClientOptions")
            .field("auth", &self.auth)
            .field("api_url", &self.api_url)
            .finish_non_exhaustive()
    }
}

impl SquadsClientOptions {
    pub fn with_auth(mut self, auth: AuthConfig) -> Self {
        self.auth = Some(auth);
        self
    }

    pub fn with_api_url(mut self, api_url: impl Into<String>) -> Self {
        self.api_url = Some(api_url.into());
        self
    }

    pub fn with_http_client(mut self, client: reqwest::Client) -> Self {
        self.http_client = Some(client);
        self
    }
}

#[derive(Clone)]
enum Bearer {
    Static(String),
    Provider(Arc<TokenProvider>),
}

/// The account's Squads multisigs (`<api>/account/squads`).
///
/// The bearer is the key or token itself; the routes take a secret key and
/// never a protocol session.
#[derive(Clone)]
pub struct SquadsClient {
    http: reqwest::Client,
    base: String,
    bearer: Bearer,
}

impl std::fmt::Debug for SquadsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SquadsClient")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

fn invalid_auth(message: &str) -> ProtocolError {
    ProtocolError::Auth(AreteError::InvalidConfig(message.into()))
}

/// The API base for `options`: the option, then the environment origin,
/// then Arete's.
fn resolve_api_url(options: &SquadsClientOptions, env: impl Fn(&str) -> Option<String>) -> String {
    if let Some(url) = options
        .api_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        return url.trim_end_matches('/').to_string();
    }
    if let Some(origin) = env(ARETE_API_URL_ENV)
        .map(|origin| origin.trim().trim_end_matches('/').to_string())
        .filter(|origin| !origin.is_empty())
    {
        let origin = origin.trim_end_matches("/api");
        return format!("{origin}/api");
    }
    DEFAULT_API_URL.to_string()
}

impl SquadsClient {
    pub fn new(options: SquadsClientOptions) -> Result<Self, ProtocolError> {
        let base = format!(
            "{}/account/squads",
            resolve_api_url(&options, |name| std::env::var(name).ok())
        );
        let auth = crate::auth::resolve_auth_config(options.auth)?;
        let Some(auth) = auth else {
            return Err(invalid_auth(
                "account Squads needs a secret key (a4_sk_...) or agent key (a4_ak_...): pass \
                 one, set ARETE_API_KEY, or run `a4 auth login`",
            ));
        };
        let bearer = if let Some(token) = auth.token.clone() {
            Bearer::Static(token)
        } else if let Some(provider) = auth.get_token.clone() {
            Bearer::Provider(provider)
        } else if let Some(key) = auth.api_key_for(&base) {
            if classify_api_key(key) == ApiKeyClass::Publishable {
                return Err(invalid_auth(
                    "account Squads needs a secret key (a4_sk_...) or agent key (a4_ak_...); a \
                     publishable key cannot manage the account's multisigs",
                ));
            }
            Bearer::Static(key.to_string())
        } else if auth.secret_key.is_some() {
            return Err(invalid_auth(
                "the a4 login key is only sent to the Arete API; pass a secret key for this API",
            ));
        } else {
            return Err(invalid_auth(
                "account Squads needs a secret key (a4_sk_...) or agent key (a4_ak_...), not a \
                 publishable key or a token endpoint",
            ));
        };
        Ok(Self {
            http: options.http_client.unwrap_or_default(),
            base,
            bearer,
        })
    }

    /// `<api>/account/squads`.
    pub fn base_url(&self) -> &str {
        &self.base
    }

    async fn bearer(&self) -> Result<String, ProtocolError> {
        match &self.bearer {
            Bearer::Static(token) => Ok(token.clone()),
            Bearer::Provider(provider) => Ok(provider().await?.token),
        }
    }

    async fn call(
        &self,
        what: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Option<Vec<u8>>, ProtocolError> {
        let url = format!("{}{path}", self.base);
        let token = self.bearer().await?;
        let header = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| ProtocolError::InvalidInput("the key is not a valid header".into()))?;
        let mut request = self
            .http
            .request(method, &url)
            .header(AUTHORIZATION, header);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|error| ProtocolError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        let header_code = response
            .headers()
            .get(HEADER_ERROR_CODE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim().to_string());
        let bytes = response
            .bytes()
            .await
            .map_err(|error| ProtocolError::Transport(error.to_string()))?
            .to_vec();
        if !(200..300).contains(&status) {
            return Err(super::client::response_error(
                what,
                status,
                header_code.as_deref(),
                super::client::parse_body(&bytes),
            ));
        }
        Ok((!bytes.is_empty()).then_some(bytes))
    }

    async fn json<T: DeserializeOwned>(
        &self,
        what: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<T, ProtocolError> {
        let bytes = self
            .call(what, method, path, body)
            .await?
            .unwrap_or_default();
        serde_json::from_slice(&bytes).map_err(|error| {
            ProtocolError::UnexpectedResponse(format!(
                "{what} returned an unexpected body: {error}"
            ))
        })
    }

    fn multisig_path(multisig: &str) -> Result<String, ProtocolError> {
        Ok(format!(
            "/multisigs/{}",
            super::client::segment(multisig.trim(), "multisig")?
        ))
    }

    fn member_body(member: &str, format: Option<ProtocolTransactionFormat>) -> Value {
        let mut body = serde_json::json!({ "member": member });
        if let Some(format) = format {
            body["transactionFormat"] = serde_json::to_value(format).unwrap_or(Value::Null);
        }
        body
    }

    /// The account's setup, its proposer and every attachment, refreshed
    /// from the chain.
    pub async fn status(&self) -> Result<SquadsAccountStatus, ProtocolError> {
        self.json("Squads status", Method::GET, "", None).await
    }

    /// The account notifications: attachments archived after being active.
    /// No chain read.
    pub async fn notifications(&self) -> Result<Vec<SquadsNotification>, ProtocolError> {
        self.json("Squads notifications", Method::GET, "/notifications", None)
            .await
    }

    /// One attachment, refreshed, with the proposals Arete made for it.
    pub async fn get(&self, multisig: &str) -> Result<SquadsAttachmentDetail, ProtocolError> {
        let path = Self::multisig_path(multisig)?;
        self.json("Squads multisig read", Method::GET, &path, None)
            .await
    }

    /// Attach an existing Squads v4 multisig by address: `active` when the
    /// proposer is already an Initiate-only member, else `pending`.
    pub async fn attach(
        &self,
        address: &str,
        label: Option<&str>,
    ) -> Result<SquadsAttachment, ProtocolError> {
        if address.trim().is_empty() {
            return Err(ProtocolError::InvalidInput(
                "attach requires a multisig address".into(),
            ));
        }
        let mut body = serde_json::json!({ "address": address.trim() });
        if let Some(label) = label {
            body["label"] = Value::String(label.to_string());
        }
        self.json("Squads attach", Method::POST, "/multisigs", Some(body))
            .await
    }

    /// An unsigned `multisigCreateV2` (already signed by its throwaway
    /// create key) with the proposer an Initiate-only member, for `creator`
    /// to sign and pay.
    pub async fn create(
        &self,
        request: &SquadsCreateRequest,
    ) -> Result<SquadsPreparedTransaction, ProtocolError> {
        if request.members.is_empty() || request.creator.trim().is_empty() {
            return Err(ProtocolError::InvalidInput(
                "create requires members and a creator".into(),
            ));
        }
        let body = serde_json::to_value(request)
            .map_err(|error| ProtocolError::InvalidInput(error.to_string()))?;
        self.json(
            "Squads create",
            Method::POST,
            "/multisigs/create",
            Some(body),
        )
        .await
    }

    /// An unsigned config proposal adding the proposer (Initiate only), for
    /// `member` (who can initiate) to sign and pay.
    pub async fn add_proposer(
        &self,
        multisig: &str,
        member: &str,
        format: Option<ProtocolTransactionFormat>,
    ) -> Result<SquadsPreparedTransaction, ProtocolError> {
        self.member_write(
            "Squads add-proposer",
            multisig,
            "/add-proposer",
            member,
            format,
        )
        .await
    }

    /// An unsigned approval of config proposal `transaction_index`, for
    /// `member` (who can vote).
    pub async fn approve(
        &self,
        multisig: &str,
        transaction_index: u64,
        member: &str,
        format: Option<ProtocolTransactionFormat>,
    ) -> Result<SquadsPreparedTransaction, ProtocolError> {
        let route = format!("/proposals/{transaction_index}/approve");
        self.member_write("Squads approve", multisig, &route, member, format)
            .await
    }

    /// An unsigned execution of approved config proposal
    /// `transaction_index`, for `member` (who can execute).
    pub async fn execute(
        &self,
        multisig: &str,
        transaction_index: u64,
        member: &str,
        format: Option<ProtocolTransactionFormat>,
    ) -> Result<SquadsPreparedTransaction, ProtocolError> {
        let route = format!("/proposals/{transaction_index}/execute");
        self.member_write("Squads execute", multisig, &route, member, format)
            .await
    }

    /// An unsigned config proposal removing the proposer.
    pub async fn remove_proposer(
        &self,
        multisig: &str,
        member: &str,
        format: Option<ProtocolTransactionFormat>,
    ) -> Result<SquadsPreparedTransaction, ProtocolError> {
        self.member_write(
            "Squads remove-proposer",
            multisig,
            "/remove-proposer",
            member,
            format,
        )
        .await
    }

    async fn member_write(
        &self,
        what: &str,
        multisig: &str,
        route: &str,
        member: &str,
        format: Option<ProtocolTransactionFormat>,
    ) -> Result<SquadsPreparedTransaction, ProtocolError> {
        if member.trim().is_empty() {
            return Err(ProtocolError::InvalidInput(format!(
                "{what} requires a member"
            )));
        }
        let path = format!("{}{route}", Self::multisig_path(multisig)?);
        self.json(
            what,
            Method::POST,
            &path,
            Some(Self::member_body(member.trim(), format)),
        )
        .await
    }

    /// Forget a `pending` or `archived` attachment. An `active` one is
    /// refused (`protocol-squads-multisig-active`): remove the proposer
    /// first.
    pub async fn forget(&self, multisig: &str) -> Result<(), ProtocolError> {
        let path = Self::multisig_path(multisig)?;
        self.call("Squads forget", Method::DELETE, &path, None)
            .await
            .map(|_| ())
    }

    /// Send a transaction one of the routes above prepared, signed by its
    /// `signer`, and wait (about 30 seconds) for it to confirm. A `pending`
    /// answer is re-checked by submitting the same body again; it is never
    /// sent twice.
    pub async fn submit(
        &self,
        transaction_id: &str,
        signed_transaction: &[u8],
    ) -> Result<SquadsSubmitResult, ProtocolError> {
        if transaction_id.trim().is_empty() {
            return Err(ProtocolError::InvalidInput(
                "submit requires the prepared transactionId".into(),
            ));
        }
        let body = serde_json::json!({
            "transactionId": transaction_id.trim(),
            "signedTransactionBase64":
                base64::engine::general_purpose::STANDARD.encode(signed_transaction),
        });
        self.json("Squads submit", Method::POST, "/transactions", Some(body))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_url_prefers_the_option_then_the_environment_origin() {
        let options = SquadsClientOptions::default().with_api_url("http://127.0.0.1:9/api/");
        assert_eq!(
            resolve_api_url(&options, |_| None),
            "http://127.0.0.1:9/api"
        );
        let env = |_: &str| Some("https://api.example.test/".to_string());
        assert_eq!(
            resolve_api_url(&SquadsClientOptions::default(), env),
            "https://api.example.test/api"
        );
        assert_eq!(
            resolve_api_url(&SquadsClientOptions::default(), |_| None),
            DEFAULT_API_URL
        );
    }

    #[test]
    fn dashboard_urls_name_the_multisig() {
        assert_eq!(squads_dashboard_url(None), SQUADS_DASHBOARD_URL);
        assert_eq!(
            squads_dashboard_url(Some(" Ms1 ")),
            "https://app.arete.run/dashboard/squads/Ms1"
        );
    }

    #[test]
    fn setup_refusals_are_recognised() {
        let body = serde_json::json!({
            "code": "protocol-squads-setup-required",
            "message": "Step 'finalize' proposes to Squads multisig Ms1, which is not set up",
            "details": {
                "step": "finalize", "multisig": "Ms1", "state": "pending",
                "problem": "not_member", "proposer": "Prop1",
                "setupUrl": "https://app.arete.run/dashboard/squads/Ms1",
                "commands": { "status": "a4 squads status Ms1" }
            }
        });
        let refusal = squads_setup_required(&body).unwrap();
        assert_eq!(refusal.details.step, "finalize");
        assert_eq!(refusal.details.state, SquadsAttachmentState::Pending);
        assert_eq!(
            refusal.details.setup_url,
            "https://app.arete.run/dashboard/squads/Ms1"
        );
        assert!(refusal.message.starts_with("Step 'finalize'"));
        assert!(squads_setup_required(&serde_json::json!({ "code": "other" })).is_none());
        let error = ProtocolError::Request {
            what: "Deployment create".into(),
            status: 409,
            wire_code: Some(SQUADS_SETUP_REQUIRED_CODE.into()),
            message: "x".into(),
            body: Some(body),
        };
        assert_eq!(
            error.squads_setup_required().unwrap().details.multisig,
            "Ms1"
        );
    }

    #[test]
    fn publishable_keys_are_refused() {
        let error = SquadsClient::new(
            SquadsClientOptions::default()
                .with_api_url("http://127.0.0.1:9/api")
                .with_auth(AuthConfig::default().with_publishable_key("a4_pk_test")),
        )
        .unwrap_err();
        assert!(error.to_string().contains("secret key"), "{error}");
    }
}
