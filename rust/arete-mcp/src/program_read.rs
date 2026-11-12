//! Program Read: one account, decoded for a registry program's release.
//!
//! Shared by `a4 program account decode` and the MCP `program_read_account`
//! tool. A read takes three steps:
//!
//! 1. The program's install descriptor
//!    (`GET /api/registry/programs/<name|id>/install`) names its Program Read
//!    binding (`programReadBindingId`, `endpoint`, release) and the managed
//!    Solana gateway.
//! 2. Without an explicit program or account type, the account's raw state
//!    is read through the gateway (`POST <gateway>/chain/v1/account`) to find
//!    its owner and the IDL account its discriminator matches.
//! 3. A read session is minted for the binding and the decoded account is
//!    read from `GET <endpoint>/v1/releases/<release>/accounts/<Type>/<address>`.
//!
//! The session in step 3 comes from one of two places ([`ReadAuth`]): the
//! account's API key (`POST /ws/sessions`), or a sandbox
//! (`POST /api/sandbox/v1/sessions` with the sandbox token), which mints it
//! only for programs in the sandbox's rules. Keys are sent only to the API
//! origin they belong to.

use base64::Engine as _;
use serde_json::{json, Value};

use crate::sandbox::{ReadTarget, SandboxClient, SandboxRefusal};

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// One account's raw state: its owner and data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawAccount {
    pub owner: String,
    pub data: Vec<u8>,
}

/// A hosted binding's session endpoint and target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainBinding {
    pub endpoint: String,
    pub session_endpoint: String,
    pub target_kind: String,
    pub target_id: String,
}

/// A program's Program Read binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadBinding {
    pub binding_id: String,
    pub endpoint: String,
    pub session_endpoint: String,
}

/// What a read needs from a program install descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct Install {
    pub install_name: String,
    pub program_id: String,
    pub release: String,
    pub idl: Value,
    pub read: Option<ReadBinding>,
    pub chain: Option<ChainBinding>,
}

impl Install {
    /// From a `GET /api/registry/programs/<name>/install` body.
    pub fn parse(body: &Value) -> Result<Self, String> {
        let text = |pointer: &str| {
            body.pointer(pointer)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("The program install descriptor has no {pointer}"))
        };
        let install_name = text("/installName")?;
        let binding = body.pointer("/transport/binding");
        let read = binding.and_then(|binding| {
            Some(ReadBinding {
                binding_id: binding.get("programReadBindingId")?.as_str()?.to_string(),
                endpoint: binding.get("endpoint")?.as_str()?.to_string(),
                session_endpoint: binding
                    .pointer("/auth/sessionEndpoint")?
                    .as_str()?
                    .to_string(),
            })
        });
        let chain = body.get("chainBinding").and_then(|chain| {
            Some(ChainBinding {
                endpoint: chain.get("endpoint")?.as_str()?.to_string(),
                session_endpoint: chain
                    .pointer("/auth/sessionEndpoint")?
                    .as_str()?
                    .to_string(),
                target_kind: chain.pointer("/auth/targetKind")?.as_str()?.to_string(),
                target_id: chain.pointer("/auth/targetId")?.as_str()?.to_string(),
            })
        });
        Ok(Self {
            program_id: text("/definition/programId")?,
            release: text("/release/programReleaseHash")?,
            idl: body
                .pointer("/definition/idlPayload")
                .cloned()
                .unwrap_or(Value::Null),
            install_name,
            read,
            chain,
        })
    }

    pub fn read_binding(&self) -> Result<&ReadBinding, String> {
        self.read
            .as_ref()
            .ok_or_else(|| format!("{} has no hosted Program Read binding", self.install_name))
    }
}

/// The account type to decode as: `requested` when the IDL names it, else
/// the IDL account whose discriminator starts the data, else the IDL's only
/// account.
pub fn account_type(
    install_name: &str,
    idl: &Value,
    data: &[u8],
    requested: Option<&str>,
) -> Result<String, String> {
    let accounts = idl_accounts(idl);
    let name = |account: &Value| account["name"].as_str().map(str::to_string);
    if let Some(requested) = requested {
        return accounts
            .iter()
            .filter_map(|account| name(account))
            .find(|candidate| candidate == requested)
            .ok_or_else(|| {
                format!(
                    "{install_name} has no account type '{requested}'; it has: {}",
                    account_names(&accounts)
                )
            });
    }
    match matching_accounts(&accounts, data).as_slice() {
        [only] => Ok(only.clone()),
        [] if accounts.len() == 1 => {
            name(accounts[0]).ok_or_else(|| format!("{install_name}'s IDL account has no name"))
        }
        [] => Err(format!(
            "No account type of {install_name} matches the account's data; pass the account type (one of: {})",
            account_names(&accounts)
        )),
        several => Err(format!(
            "Several account types of {install_name} match the account's data ({}); pass the account type",
            several.join(", ")
        )),
    }
}

/// The IDL's account definitions.
pub fn idl_accounts(idl: &Value) -> Vec<&Value> {
    idl["accounts"]
        .as_array()
        .map(|accounts| accounts.iter().collect())
        .unwrap_or_default()
}

/// The names of the IDL accounts whose discriminator starts `data`.
pub fn matching_accounts(accounts: &[&Value], data: &[u8]) -> Vec<String> {
    accounts
        .iter()
        .filter(|account| {
            discriminator(account)
                .is_some_and(|prefix| !prefix.is_empty() && data.starts_with(&prefix))
        })
        .filter_map(|account| account["name"].as_str().map(str::to_string))
        .collect()
}

fn discriminator(account: &Value) -> Option<Vec<u8>> {
    account["discriminator"]
        .as_array()?
        .iter()
        .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
        .collect()
}

pub fn account_names(accounts: &[&Value]) -> String {
    let names: Vec<&str> = accounts
        .iter()
        .filter_map(|account| account["name"].as_str())
        .collect();
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

/// A `/chain/v1/account` answer: `{value: {ownerProgram, data (base64)}}`
/// or `{value: null}`.
pub fn raw_from_chain(body: &Value) -> Result<Option<RawAccount>, String> {
    let value = &body["value"];
    if value.is_null() {
        return Ok(None);
    }
    let owner = value["ownerProgram"]
        .as_str()
        .ok_or("The chain read answered without an owner")?;
    let data = value["data"]
        .as_str()
        .ok_or("The chain read answered without data")?;
    Ok(Some(RawAccount {
        owner: owner.to_string(),
        data: base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| "The chain read's data is not base64".to_string())?,
    }))
}

/// `<endpoint>/v1/releases/<release>/accounts/<Account>/<address>`, each
/// part one path segment.
pub fn decode_url(
    endpoint: &str,
    release: &str,
    account: &str,
    address: &str,
) -> Result<String, String> {
    let invalid = || format!("Invalid Program Read endpoint {endpoint}");
    let mut url = reqwest::Url::parse(endpoint).map_err(|_| invalid())?;
    url.path_segments_mut()
        .map_err(|()| invalid())?
        .pop_if_empty()
        .extend(["v1", "releases", release, "accounts", account, address]);
    Ok(url.to_string())
}

/// Why a read failed.
#[derive(Debug)]
pub enum ReadError {
    /// The sandbox refused the read session.
    Sandbox(SandboxRefusal),
    /// Anything else, with a message for the agent.
    Failed(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sandbox(refusal) => write!(f, "{refusal}"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<String> for ReadError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

impl From<SandboxRefusal> for ReadError {
    fn from(refusal: SandboxRefusal) -> Self {
        Self::Sandbox(refusal)
    }
}

/// The account's API key at its API origin.
#[derive(Clone)]
pub struct AccountKey {
    pub api_base: String,
    pub key: String,
}

impl AccountKey {
    /// The key for `ARETE_API_URL` (else the default API), from
    /// `ARETE_API_KEY` or the a4 login for that API.
    pub fn from_env() -> Option<Self> {
        let api_base = crate::credentials::normalize_api_url(
            &std::env::var("ARETE_API_URL")
                .unwrap_or_else(|_| crate::credentials::DEFAULT_API_URL.to_string()),
        );
        let key = crate::credentials::resolve_key_for_api(&api_base)
            .ok()
            .flatten()?;
        Some(Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            key: key.key,
        })
    }

    /// Mint a session at `session_endpoint`, which must be on this key's
    /// API origin: the key goes nowhere else. A key a sandbox grant binds is
    /// refused with [`crate::sandbox::READ_REQUIRED`], kept as a
    /// [`ReadError::Sandbox`] so the caller can read through the sandbox.
    pub async fn mint(
        &self,
        http: &reqwest::Client,
        session_endpoint: &str,
        body: &Value,
    ) -> Result<String, ReadError> {
        let origin = |url: &str| {
            reqwest::Url::parse(url.trim())
                .ok()
                .map(|url| url.origin().ascii_serialization())
                .filter(|origin| origin != "null")
        };
        if origin(session_endpoint).is_none() || origin(session_endpoint) != origin(&self.api_base)
        {
            return Err(ReadError::Failed(format!(
                "Refusing to send the API key to the session endpoint {session_endpoint}: it is not on {}. Nothing was sent.",
                self.api_base
            )));
        }
        let minted = send_request(
            http.post(session_endpoint)
                .bearer_auth(&self.key)
                .json(body),
            "session token",
        )
        .await
        .map_err(|refused| {
            match SandboxRefusal::read_required(refused.status, refused.body) {
                Some(refusal) => ReadError::Sandbox(refusal),
                None => ReadError::Failed(refused.message),
            }
        })?;
        minted["token"]
            .as_str()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(str::to_string)
            .ok_or_else(|| ReadError::Failed("The session endpoint returned no token".to_string()))
    }
}

/// Where a Program Read session comes from.
#[derive(Clone)]
pub enum ReadAuth {
    /// The account's API key, at `POST /ws/sessions`.
    Key(AccountKey),
    /// A sandbox, at `POST /api/sandbox/v1/sessions`.
    Sandbox(SandboxClient),
}

/// The raw account at `address`, read through `chain` with a gateway
/// session minted by `key`.
pub async fn read_raw(
    http: &reqwest::Client,
    key: &AccountKey,
    chain: &ChainBinding,
    address: &str,
) -> Result<Option<RawAccount>, String> {
    let token = key
        .mint(
            http,
            &chain.session_endpoint,
            &json!({
                "scopes": ["read"],
                "targetKind": chain.target_kind,
                "targetId": chain.target_id,
            }),
        )
        .await
        .map_err(|error| error.to_string())?;
    let url = format!("{}/chain/v1/account", chain.endpoint.trim_end_matches('/'));
    let body = send(
        http.post(&url)
            .bearer_auth(token)
            .json(&json!({"address": address, "options": {"commitment": "confirmed"}})),
        "chain read",
    )
    .await?;
    raw_from_chain(&body)
}

/// A read session for `install`'s Program Read binding.
pub async fn read_session(
    http: &reqwest::Client,
    auth: &ReadAuth,
    install: &Install,
) -> Result<String, ReadError> {
    let binding = install.read_binding()?;
    match auth {
        ReadAuth::Key(key) => Ok(key
            .mint(
                http,
                &binding.session_endpoint,
                &json!({
                    "scopes": ["read"],
                    "targetKind": "program-read-binding",
                    "targetId": binding.binding_id,
                    "programReleaseHash": install.release,
                }),
            )
            .await?),
        ReadAuth::Sandbox(sandbox) => Ok(sandbox
            .read_session_cached(&program_target(install)?)
            .await?
            .token),
    }
}

/// The sandbox read target for `install`'s Program Read binding.
fn program_target(install: &Install) -> Result<ReadTarget, String> {
    Ok(ReadTarget::ProgramRead {
        binding_id: install.read_binding()?.binding_id.clone(),
        program_release_hash: install.release.clone(),
    })
}

/// The decoded `account` at `address`: a read session from `auth`, then the
/// read. Through a sandbox the session is reused while it is fresh; when
/// Program Read refuses it as expired or invalid, it is minted again once.
pub async fn read_account(
    http: &reqwest::Client,
    auth: &ReadAuth,
    install: &Install,
    account: &str,
    address: &str,
) -> Result<Value, ReadError> {
    let token = read_session(http, auth, install).await?;
    match fetch_decoded(http, install, &token, account, address).await {
        Err(refused) if refused.token_rejected() => match auth {
            ReadAuth::Sandbox(sandbox) => {
                sandbox.forget_read_session(&program_target(install)?).await;
                let token = read_session(http, auth, install).await?;
                Ok(read_decoded(http, install, &token, account, address).await?)
            }
            ReadAuth::Key(_) => Err(ReadError::Failed(refused.message)),
        },
        Err(refused) => Err(ReadError::Failed(refused.message)),
        Ok(value) => Ok(value),
    }
}

/// The decoded `account` at `address`, read with `token`.
pub async fn read_decoded(
    http: &reqwest::Client,
    install: &Install,
    token: &str,
    account: &str,
    address: &str,
) -> Result<Value, String> {
    fetch_decoded(http, install, token, account, address)
        .await
        .map_err(|refused| refused.message)
}

async fn fetch_decoded(
    http: &reqwest::Client,
    install: &Install,
    token: &str,
    account: &str,
    address: &str,
) -> Result<Value, Refused> {
    let binding = install.read_binding().map_err(Refused::local)?;
    let url = decode_url(&binding.endpoint, &install.release, account, address)
        .map_err(Refused::local)?;
    let value = send_request(http.get(url).bearer_auth(token), "Program Read").await?;
    if value.is_null() {
        return Err(Refused::local(format!(
            "Program Read found no {account} at {address}"
        )));
    }
    Ok(value)
}

/// The HTTP client for reads.
pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("arete-mcp/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Send `request`; an error status keeps the server's code and message.
async fn send(request: reqwest::RequestBuilder, what: &str) -> Result<Value, String> {
    send_request(request, what)
        .await
        .map_err(|refused| refused.message)
}

/// A request that failed: the status and body when the server answered
/// (0 and null when it did not), and a message for the agent.
struct Refused {
    status: u16,
    body: Value,
    message: String,
}

impl Refused {
    fn local(message: String) -> Self {
        Self {
            status: 0,
            body: Value::Null,
            message,
        }
    }

    /// Whether the server refused the token itself (expired or invalid), so
    /// a newly minted one may pass.
    fn token_rejected(&self) -> bool {
        let code = self
            .body
            .pointer("/error/code")
            .or_else(|| self.body.get("code"))
            .and_then(Value::as_str);
        self.status == 401 || code == Some("token-expired")
    }
}

async fn send_request(request: reqwest::RequestBuilder, what: &str) -> Result<Value, Refused> {
    let response = request.send().await.map_err(|error| {
        Refused::local(format!(
            "The {what} request failed: {}",
            error.without_url()
        ))
    })?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(body);
    }
    Err(Refused {
        status: status.as_u16(),
        message: refusal_message(what, status.as_u16(), &body, &text),
        body,
    })
}

/// `<what> → HTTP <status> <code>: <message>` from an error body.
pub fn refusal_message(what: &str, status: u16, body: &Value, text: &str) -> String {
    let code = body
        .pointer("/error/code")
        .or_else(|| body.get("code"))
        .and_then(Value::as_str);
    let message = body
        .pointer("/error/message")
        .or_else(|| body.get("message"))
        .or_else(|| body.get("error"))
        .and_then(Value::as_str)
        .unwrap_or(text.trim());
    match code {
        Some(code) => format!("{what} → HTTP {status} {code}: {message}"),
        None => format!("{what} → HTTP {status}: {message}"),
    }
}

/// The programs a `GET /api/sandbox/v1/reads` answer lists: `(programId,
/// name)`.
pub fn readable_programs(reads: &Value) -> Vec<(String, Option<String>)> {
    reads["programs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|program| {
            let id = program["programId"].as_str()?.to_string();
            let name = program["name"].as_str().map(str::to_string);
            Some((id, name))
        })
        .collect()
}

/// A local refusal for a program the sandbox's rules leave out, in the
/// API's shape, so it reads like the refusal the mint would have returned.
pub fn not_allowlisted(program_id: &str, install_name: &str, reads: &Value) -> SandboxRefusal {
    let allowed: Vec<String> = readable_programs(reads)
        .into_iter()
        .map(|(id, name)| match name {
            Some(name) => format!("{name} ({id})"),
            None => id,
        })
        .collect();
    SandboxRefusal {
        status: 403,
        body: json!({
            "code": "sandbox-program-not-allowlisted",
            "error": format!(
                "{install_name} ({program_id}) is not in the sandbox's rules, so the sandbox cannot read it"
            ),
            "retryable": false,
            "programs": [program_id],
            "readable": allowed,
        }),
        kind: crate::sandbox::CallKind::Read,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> Value {
        json!({
            "installName": "klend",
            "definition": {
                "programId": "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD",
                "idlPayload": {"accounts": [
                    {"name": "LendingMarket", "discriminator": [1, 2, 3, 4, 5, 6, 7, 8]},
                    {"name": "Reserve", "discriminator": [9, 9, 9, 9, 9, 9, 9, 9]},
                ]},
            },
            "release": {"programReleaseHash": "arete:h1:program-release:sha256:aa"},
            "transport": {"kind": "hosted-binding", "binding": {
                "endpoint": "https://reads.example.test/prefix/",
                "programReadBindingId": "prb_1",
                "auth": {"sessionEndpoint": "https://api.example.test/ws/sessions"},
            }},
            "chainBinding": {
                "endpoint": "https://gw.example.test",
                "auth": {
                    "sessionEndpoint": "https://api.example.test/ws/sessions",
                    "targetKind": "solana-gateway-binding",
                    "targetId": "sgb_1",
                },
            },
        })
    }

    #[test]
    fn an_install_descriptor_names_its_bindings() {
        let install = Install::parse(&descriptor()).unwrap();
        assert_eq!(install.install_name, "klend");
        assert_eq!(install.release, "arete:h1:program-release:sha256:aa");
        assert_eq!(install.read_binding().unwrap().binding_id, "prb_1");
        assert_eq!(install.chain.as_ref().unwrap().target_id, "sgb_1");
        let mut bare = descriptor();
        bare.as_object_mut().unwrap().remove("transport");
        let error = Install::parse(&bare).unwrap().read_binding().unwrap_err();
        assert!(error.contains("no hosted Program Read binding"), "{error}");
    }

    #[test]
    fn the_account_type_follows_the_discriminator() {
        let install = Install::parse(&descriptor()).unwrap();
        let reserve = [9, 9, 9, 9, 9, 9, 9, 9, 42];
        let of = |data: &[u8], requested| account_type("klend", &install.idl, data, requested);
        assert_eq!(of(&reserve, None).unwrap(), "Reserve");
        assert_eq!(
            of(&reserve, Some("LendingMarket")).unwrap(),
            "LendingMarket"
        );
        assert!(of(&reserve, Some("Vault"))
            .unwrap_err()
            .contains("LendingMarket, Reserve"));
        assert!(of(&[0; 16], None)
            .unwrap_err()
            .contains("pass the account type"));
    }

    #[test]
    fn decode_urls_keep_each_part_one_segment() {
        assert_eq!(
            decode_url(
                "https://reads.example.test/prefix/",
                "rel",
                "Reserve",
                "Addr"
            )
            .unwrap(),
            "https://reads.example.test/prefix/v1/releases/rel/accounts/Reserve/Addr"
        );
        assert_eq!(
            decode_url("https://r.example.test", "a/b", "T", "x").unwrap(),
            "https://r.example.test/v1/releases/a%2Fb/accounts/T/x"
        );
    }

    #[test]
    fn a_key_is_sent_only_to_its_own_api() {
        let key = AccountKey {
            api_base: "https://api.example.test".into(),
            key: "a4_sk_secret".into(),
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let error = runtime
            .block_on(key.mint(&http(), "https://evil.example/ws/sessions", &json!({})))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Nothing was sent"), "{error}");
        assert!(!error.contains("a4_sk_secret"), "{error}");
    }

    #[test]
    fn a_program_outside_the_rules_is_refused_with_what_can_be_read() {
        let reads = json!({"programs": [
            {"programId": "JUP6", "name": "Jupiter"},
            {"programId": "Other"},
        ]});
        let refusal = not_allowlisted("KLend", "klend", &reads).to_json();
        assert_eq!(refusal["code"], "sandbox-program-not-allowlisted");
        assert_eq!(refusal["programs"], json!(["KLend"]));
        assert_eq!(refusal["readable"], json!(["Jupiter (JUP6)", "Other"]));
        assert_eq!(refusal["retryable"], false);
    }
}
