//! The opt-in local signer for `protocol_execute`.
//!
//! By default the MCP server never signs: `protocol_prepare` returns
//! unsigned transactions and `protocol_submit` takes them back signed. A
//! signer is configured only when the user writes one, in the `[signer]`
//! table of `~/.arete/mcp.toml` or of the TOML file `ARETE_MCP_SIGNER_CONFIG`
//! names:
//!
//! ```toml
//! [signer]
//! keypair = "~/.config/solana/agent.json"   # a Solana CLI keypair file
//! rpcUrl = "https://my-rpc.example"         # simulates balance changes
//!
//! [signer.policy]
//! deployments = ["<deployment id>"]         # required allow-list
//! maxSolPerTransaction = 0.05               # required, SOL
//! programs = ["<program id>"]               # optional allow-list
//! clusters = ["devnet", "localnet"]         # optional; mainnet only if listed
//!
//! [signer.policy.maxTokenAmounts]           # base units per mint; without
//! "<mint>" = "5000000"                      # it no token may leave
//! ```
//!
//! Every transaction is checked against the policy before it is signed, and
//! a refusal names the rule. The caps hold per transaction, not per step or
//! over time. Wrapped SOL counts against `maxSolPerTransaction`, as do the
//! lamports of a token account closed to someone else. Instructions that
//! hand control of the signer's funds to someone else (token approvals,
//! authority changes, System assignments; see [`crate::authority`]) are
//! always refused. The keypair's secret is held only as a signing
//! key: no `Debug`, output, error or log line contains it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use ed25519_dalek::SigningKey;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::authority::NATIVE_MINTS;
use crate::prepared_summary::{decode_transaction, TOKEN_PROGRAMS};

/// Environment variable naming a TOML file with a `[signer]` table.
pub const ENV_SIGNER_CONFIG: &str = "ARETE_MCP_SIGNER_CONFIG";
/// The MCP server's config file under the home directory.
pub const MCP_CONFIG_FILE: &str = ".arete/mcp.toml";

/// Clusters a policy allows when it does not list any.
pub const DEFAULT_CLUSTERS: [&str; 3] = ["localnet", "devnet", "testnet"];
/// Every cluster name a policy may list.
pub const KNOWN_CLUSTERS: [&str; 4] = ["localnet", "devnet", "testnet", "mainnet"];

/// Programs the transaction compiler adds around a step's own instructions
/// (compute budget, memo, fee transfers, token accounts and wrapped SOL).
/// They are always allowed; what they move is bounded by the SOL and token
/// caps.
pub const RUNTIME_PROGRAMS: [&str; 7] = [
    "ComputeBudget111111111111111111111111111111",
    "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr",
    "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo",
    "11111111111111111111111111111111",
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
    TOKEN_PROGRAMS[0],
    TOKEN_PROGRAMS[1],
];

#[derive(Debug, Deserialize)]
struct ConfigFile {
    signer: Option<RawSigner>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawSigner {
    keypair: String,
    #[serde(default)]
    rpc_url: Option<String>,
    policy: RawPolicy,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawPolicy {
    deployments: Vec<String>,
    max_sol_per_transaction: toml::Value,
    #[serde(default)]
    programs: Option<Vec<String>>,
    #[serde(default)]
    max_token_amounts: Option<BTreeMap<String, toml::Value>>,
    #[serde(default)]
    clusters: Option<Vec<String>>,
}

/// What the signer may sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerPolicy {
    pub deployments: Vec<String>,
    pub max_lamports_per_transaction: u64,
    pub programs: Option<Vec<String>>,
    /// Base units per mint. An outflow of a mint without a cap is refused,
    /// so without this table no token may leave the signer (wrapped SOL
    /// counts as SOL instead).
    pub max_token_amounts: Option<BTreeMap<String, u128>>,
    pub clusters: Vec<String>,
    /// Whether the config listed `clusters` (rather than the default).
    pub clusters_listed: bool,
}

/// A configured signer: the keypair and its policy.
#[derive(Clone)]
pub struct LocalSigner {
    key: SigningKey,
    pub rpc_url: Option<String>,
    pub policy: SignerPolicy,
}

impl std::fmt::Debug for LocalSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalSigner")
            .field("pubkey", &self.pubkey())
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

/// A policy refusal: the rule and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub rule: &'static str,
    pub reason: String,
}

impl Refusal {
    fn new(rule: &'static str, reason: impl Into<String>) -> Self {
        Self {
            rule,
            reason: reason.into(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "rule": self.rule, "reason": self.reason })
    }
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Parse SOL (a TOML number or decimal string) into lamports.
fn sol_to_lamports(value: &toml::Value) -> Result<u64> {
    let text = match value {
        toml::Value::String(text) => text.trim().to_string(),
        toml::Value::Integer(number) => number.to_string(),
        toml::Value::Float(number) => format!("{number}"),
        _ => bail!("maxSolPerTransaction must be a number of SOL"),
    };
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 9
    {
        bail!("maxSolPerTransaction must be a non-negative number of SOL with at most 9 decimals");
    }
    let whole: u64 = if whole.is_empty() { 0 } else { whole.parse()? };
    let fraction: u64 = format!("{fraction:0<9}").parse()?;
    whole
        .checked_mul(1_000_000_000)
        .and_then(|lamports| lamports.checked_add(fraction))
        .ok_or_else(|| anyhow!("maxSolPerTransaction is too large"))
}

fn token_amount(value: &toml::Value) -> Result<u128> {
    match value {
        toml::Value::Integer(number) if *number >= 0 => Ok(*number as u128),
        toml::Value::String(text)
            if text.bytes().all(|b| b.is_ascii_digit()) && !text.is_empty() =>
        {
            Ok(text.parse()?)
        }
        _ => bail!("maxTokenAmounts values must be non-negative integers in base units"),
    }
}

fn is_pubkey(value: &str) -> bool {
    bs58::decode(value).into_vec().map(|bytes| bytes.len()) == Ok(32)
}

/// Read a Solana CLI keypair file. Errors never quote the file's contents.
fn read_keypair(path: &Path) -> Result<SigningKey> {
    let text = fs::read_to_string(path)
        .map_err(|error| anyhow!("the signer keypair file cannot be read ({})", error.kind()))?;
    let bytes: Vec<u8> = serde_json::from_str(&text).map_err(|_| {
        anyhow!("the signer keypair file is not a Solana keypair (a JSON array of 64 bytes)")
    })?;
    if bytes.len() != 64 {
        bail!("the signer keypair file is not a Solana keypair (a JSON array of 64 bytes)");
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&bytes[..32]);
    let key = SigningKey::from_bytes(&seed);
    if key.verifying_key().as_bytes() != &bytes[32..] {
        bail!("the signer keypair file's public key does not match its secret key");
    }
    Ok(key)
}

impl SignerPolicy {
    /// A policy from the fields of a `[signer.policy]` table, as JSON. The
    /// shared signer-policy test vectors use this form.
    pub fn from_json(value: Value) -> Result<Self> {
        let raw: RawPolicy = serde_json::from_value(value)
            .map_err(|error| anyhow!("invalid signer policy: {error}"))?;
        Self::from_raw(raw)
    }

    fn from_raw(policy: RawPolicy) -> Result<Self> {
        if policy.deployments.is_empty() {
            bail!("signer.policy.deployments must list at least one deployment id");
        }
        let max_lamports_per_transaction = sol_to_lamports(&policy.max_sol_per_transaction)?;
        if let Some(programs) = &policy.programs {
            if let Some(bad) = programs.iter().find(|program| !is_pubkey(program)) {
                bail!("signer.policy.programs: '{bad}' is not a program id");
            }
        }
        let max_token_amounts = match policy.max_token_amounts {
            Some(caps) => {
                let mut out = BTreeMap::new();
                for (mint, cap) in caps {
                    if !is_pubkey(&mint) {
                        bail!("signer.policy.maxTokenAmounts: '{mint}' is not a mint address");
                    }
                    out.insert(mint, token_amount(&cap)?);
                }
                Some(out)
            }
            None => None,
        };
        let clusters_listed = policy.clusters.is_some();
        let clusters = match policy.clusters {
            Some(clusters) => {
                if let Some(bad) = clusters
                    .iter()
                    .find(|cluster| !KNOWN_CLUSTERS.contains(&cluster.as_str()))
                {
                    bail!(
                        "signer.policy.clusters: '{bad}' is not one of {}",
                        KNOWN_CLUSTERS.join(", ")
                    );
                }
                clusters
            }
            None => DEFAULT_CLUSTERS.iter().map(|c| c.to_string()).collect(),
        };
        Ok(Self {
            deployments: policy.deployments,
            max_lamports_per_transaction,
            programs: policy.programs,
            max_token_amounts,
            clusters,
            clusters_listed,
        })
    }

    /// Whether the deployment and cluster are allowed, before preparing.
    pub fn check_target(&self, deployment: &str, cluster: &str) -> Result<(), Refusal> {
        if !self.deployments.iter().any(|d| d == deployment) {
            return Err(Refusal::new(
                "deployments",
                format!("deployment {deployment} is not in the signer's deployments allow-list"),
            ));
        }
        if !self.clusters.iter().any(|c| c == cluster) {
            return Err(Refusal::new(
                "clusters",
                format!(
                    "the target cluster is {cluster}, which the signer's clusters ({}) do not include{}",
                    self.clusters.join(", "),
                    if cluster == "mainnet" { "; mainnet must be listed explicitly" } else { "" }
                ),
            ));
        }
        Ok(())
    }

    /// Check a prepared step against the policy: the intent's deployment,
    /// every transaction's programs, the instructions that would hand over
    /// control of the signer's funds, and the simulated SOL and token
    /// outflows of the signer in `summary`, per transaction. `simulates`
    /// says whether the summary came from a simulation at all.
    pub fn check_prepared(
        &self,
        deployment: &str,
        prepared: &Value,
        summary: &Value,
        simulates: bool,
    ) -> Result<(), Refusal> {
        let intent_deployment = prepared["intent"]["deployment"]["deploymentId"]
            .as_str()
            .unwrap_or(deployment);
        if intent_deployment != deployment
            || !self.deployments.iter().any(|d| d == intent_deployment)
        {
            return Err(Refusal::new(
                "deployments",
                format!("the prepared intent is for deployment {intent_deployment}, which is not allowed"),
            ));
        }
        let transactions = summary["transactions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for transaction in &transactions {
            let index = &transaction["index"];
            if let Some(allowed) = &self.programs {
                for program in transaction["programs"].as_array().into_iter().flatten() {
                    let id = program["programId"].as_str().unwrap_or_default();
                    if !RUNTIME_PROGRAMS.contains(&id) && !allowed.iter().any(|p| p == id) {
                        return Err(Refusal::new(
                            "programs",
                            format!("transaction {index} invokes {id}, which is not in the signer's programs allow-list"),
                        ));
                    }
                }
            }
            if let Some(finding) = transaction["unsafeInstructions"]
                .as_array()
                .and_then(|findings| findings.first())
            {
                return Err(Refusal::new(
                    "authority",
                    format!(
                        "transaction {index} has {} `{}` instruction of {} that {}; the signer never signs those",
                        if finding["inner"] == true { "an inner" } else { "a" },
                        finding["instruction"].as_str().unwrap_or_default(),
                        finding["programId"].as_str().unwrap_or_default(),
                        finding["reason"].as_str().unwrap_or_default(),
                    ),
                ));
            }
            let effect = &transaction["signerEffect"];
            if effect["known"] != true {
                let why = if effect["omittedAccounts"].as_u64().unwrap_or(0) > 0 {
                    " (it writes more accounts than the simulation reports on)".to_string()
                } else if effect["closedAccountLamportsOut"].is_null()
                    && effect["solLamports"].is_string()
                {
                    " (it closes a token account the simulation could not measure)".to_string()
                } else if !simulates {
                    " (set signer.rpcUrl)".to_string()
                } else {
                    String::new()
                };
                return Err(Refusal::new(
                    "maxSolPerTransaction",
                    format!(
                        "the signer's balance change in transaction {index} could not be simulated{why}, so the caps cannot be checked"
                    ),
                ));
            }
            let lamports: i128 = effect["solLamports"]
                .as_str()
                .and_then(|value| value.parse().ok())
                .unwrap_or(i128::MIN / 2);
            let token_delta = |token: &Value| -> i128 {
                token["amount"]
                    .as_str()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0)
            };
            // Wrapped SOL is SOL: its delta joins the lamports, so wrapping
            // is not an outflow and spending wrapped SOL is.
            let wrapped: i128 = effect["tokens"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|token| NATIVE_MINTS.contains(&token["mint"].as_str().unwrap_or_default()))
                .map(token_delta)
                .sum();
            let closed: i128 = effect["closedAccountLamportsOut"]
                .as_str()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            let outflow = (closed - lamports - wrapped).max(0);
            if outflow > self.max_lamports_per_transaction as i128 {
                return Err(Refusal::new(
                    "maxSolPerTransaction",
                    format!(
                        "transaction {index} moves {} SOL out of the signer (wrapped SOL and closed token accounts included), over the {} SOL cap",
                        crate::prepared_summary::lamports_to_sol(outflow),
                        crate::prepared_summary::lamports_to_sol(
                            self.max_lamports_per_transaction as i128
                        )
                    ),
                ));
            }
            for token in effect["tokens"].as_array().into_iter().flatten() {
                let mint = token["mint"].as_str().unwrap_or_default();
                if NATIVE_MINTS.contains(&mint) {
                    continue;
                }
                let amount = token_delta(token);
                if amount >= 0 {
                    continue;
                }
                let outflow = amount.unsigned_abs();
                let Some(caps) = &self.max_token_amounts else {
                    return Err(Refusal::new(
                        "maxTokenAmounts",
                        format!("transaction {index} moves {outflow} base units of {mint} out of the signer, and the policy sets no maxTokenAmounts, so no token may leave"),
                    ));
                };
                match caps.get(mint) {
                    Some(cap) if outflow <= *cap => {}
                    Some(cap) => {
                        return Err(Refusal::new(
                            "maxTokenAmounts",
                            format!("transaction {index} moves {outflow} base units of {mint} out of the signer, over the cap of {cap}"),
                        ))
                    }
                    None => {
                        return Err(Refusal::new(
                            "maxTokenAmounts",
                            format!("transaction {index} moves {outflow} base units of {mint} out of the signer, and that mint has no cap"),
                        ))
                    }
                }
            }
        }
        Ok(())
    }
}

impl LocalSigner {
    /// The signer from `ARETE_MCP_SIGNER_CONFIG`, else `~/.arete/mcp.toml`.
    /// `Ok(None)` when neither has a `[signer]` table.
    pub fn from_environment() -> Result<Option<Self>> {
        let path = match std::env::var(ENV_SIGNER_CONFIG)
            .ok()
            .filter(|value| !value.trim().is_empty())
        {
            Some(path) => expand_home(path.trim()),
            None => match dirs::home_dir() {
                Some(home) => home.join(MCP_CONFIG_FILE),
                None => return Ok(None),
            },
        };
        match fs::read_to_string(&path) {
            Ok(text) => Self::from_toml(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if std::env::var(ENV_SIGNER_CONFIG).is_ok() {
                    bail!("{ENV_SIGNER_CONFIG} names a file that does not exist");
                }
                Ok(None)
            }
            Err(error) => bail!("the MCP config cannot be read ({})", error.kind()),
        }
    }

    /// Parse a config document; `Ok(None)` without a `[signer]` table.
    pub fn from_toml(text: &str) -> Result<Option<Self>> {
        let file: ConfigFile = toml::from_str(text)
            .map_err(|error| anyhow!("invalid signer config: {}", error.message()))?;
        let Some(raw) = file.signer else {
            return Ok(None);
        };
        let key = read_keypair(&expand_home(&raw.keypair))?;
        let policy = SignerPolicy::from_raw(raw.policy)?;
        Ok(Some(Self {
            key,
            rpc_url: raw.rpc_url.filter(|url| !url.trim().is_empty()),
            policy,
        }))
    }

    pub fn pubkey(&self) -> String {
        bs58::encode(self.key.verifying_key().as_bytes()).into_string()
    }

    /// The policy, without the key, for tool output.
    pub fn describe(&self) -> Value {
        json!({
            "pubkey": self.pubkey(),
            "deployments": self.policy.deployments,
            "maxSolPerTransaction": crate::prepared_summary::lamports_to_sol(self.policy.max_lamports_per_transaction as i128),
            "programs": self.policy.programs,
            "maxTokenAmounts": self.policy.max_token_amounts.as_ref().map(|caps| caps.iter().map(|(m, a)| (m.clone(), Value::String(a.to_string()))).collect::<serde_json::Map<_, _>>()),
            "clusters": self.policy.clusters,
            "simulates": self.rpc_url.is_some(),
        })
    }

    /// Whether the deployment and cluster are allowed, before preparing.
    pub fn check_target(&self, deployment: &str, cluster: &str) -> Result<(), Refusal> {
        self.policy.check_target(deployment, cluster)
    }

    /// Check a prepared step against the policy; see
    /// [`SignerPolicy::check_prepared`].
    pub fn check_prepared(
        &self,
        deployment: &str,
        prepared: &Value,
        summary: &Value,
    ) -> Result<(), Refusal> {
        self.policy
            .check_prepared(deployment, prepared, summary, self.rpc_url.is_some())
    }

    /// Sign every prepared transaction (base64 wire) in the signer's slot.
    pub fn sign_all(&self, prepared: &Value) -> Result<Vec<String>> {
        use base64::Engine as _;
        let mut out = Vec::new();
        for transaction in prepared["transactions"].as_array().into_iter().flatten() {
            let encoded = transaction["transactionBase64"]
                .as_str()
                .ok_or_else(|| anyhow!("a prepared transaction has no transactionBase64"))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| anyhow!("a prepared transaction is not base64"))?;
            decode_transaction(&bytes)?;
            let signed = crate::dev::sign_transaction(&bytes, &self.key)?;
            out.push(base64::engine::general_purpose::STANDARD.encode(signed));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keypair_file(dir: &Path, seed: u8) -> (PathBuf, SigningKey) {
        let key = SigningKey::from_bytes(&[seed; 32]);
        let mut bytes = vec![seed; 32];
        bytes.extend_from_slice(key.verifying_key().as_bytes());
        let path = dir.join("agent.json");
        fs::write(&path, serde_json::to_string(&bytes).unwrap()).unwrap();
        (path, key)
    }

    #[test]
    fn parses_a_signer_config() {
        let dir = tempfile::tempdir().unwrap();
        let (path, key) = keypair_file(dir.path(), 9);
        let mint = bs58::encode([5u8; 32]).into_string();
        let text = format!(
            "[signer]\nkeypair = {path:?}\nrpcUrl = \"http://127.0.0.1:1\"\n[signer.policy]\ndeployments = [\"d-1\"]\nmaxSolPerTransaction = 0.05\nclusters = [\"mainnet\"]\n[signer.policy.maxTokenAmounts]\n\"{mint}\" = \"1000\"\n"
        );
        let signer = LocalSigner::from_toml(&text).unwrap().unwrap();
        assert_eq!(
            signer.pubkey(),
            bs58::encode(key.verifying_key().as_bytes()).into_string()
        );
        assert_eq!(signer.policy.max_lamports_per_transaction, 50_000_000);
        assert_eq!(signer.policy.clusters, vec!["mainnet"]);
        assert_eq!(
            signer.policy.max_token_amounts.as_ref().unwrap()[&mint],
            1000
        );
        assert!(LocalSigner::from_toml("[other]\nx = 1\n")
            .unwrap()
            .is_none());
    }

    #[test]
    fn defaults_exclude_mainnet_and_bad_configs_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = keypair_file(dir.path(), 3);
        let base = format!("[signer]\nkeypair = {path:?}\n[signer.policy]\n");
        let signer = LocalSigner::from_toml(&format!(
            "{base}deployments = [\"d\"]\nmaxSolPerTransaction = \"1\"\n"
        ))
        .unwrap()
        .unwrap();
        assert!(!signer.policy.clusters.contains(&"mainnet".to_string()));
        assert_eq!(
            signer.check_target("d", "mainnet").unwrap_err().rule,
            "clusters"
        );
        assert_eq!(
            signer.check_target("other", "localnet").unwrap_err().rule,
            "deployments"
        );
        assert!(signer.check_target("d", "localnet").is_ok());
        for bad in [
            "deployments = []\nmaxSolPerTransaction = 1\n",
            "deployments = [\"d\"]\n",
            "deployments = [\"d\"]\nmaxSolPerTransaction = -1\n",
            "deployments = [\"d\"]\nmaxSolPerTransaction = 1\nclusters = [\"moon\"]\n",
            "deployments = [\"d\"]\nmaxSolPerTransaction = 1\nprograms = [\"nope\"]\n",
            "deployments = [\"d\"]\nmaxSolPerTransaction = 1\nsurprise = true\n",
        ] {
            assert!(
                LocalSigner::from_toml(&format!("{base}{bad}")).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn keypair_errors_never_quote_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        let secret = "[12,34,56,78,90,11,22,33,44,55]";
        fs::write(&path, secret).unwrap();
        let error = LocalSigner::from_toml(&format!(
            "[signer]\nkeypair = {path:?}\n[signer.policy]\ndeployments = [\"d\"]\nmaxSolPerTransaction = 1\n"
        ))
        .unwrap_err()
        .to_string();
        assert!(!error.contains("12,34"), "{error}");
        assert!(error.contains("not a Solana keypair"), "{error}");

        let (path, key) = keypair_file(dir.path(), 7);
        let signer = LocalSigner::from_toml(&format!(
            "[signer]\nkeypair = {path:?}\n[signer.policy]\ndeployments = [\"d\"]\nmaxSolPerTransaction = 1\n"
        ))
        .unwrap()
        .unwrap();
        let debug = format!("{signer:?} {}", signer.describe());
        assert!(!debug.contains(&bs58::encode(key.to_keypair_bytes()).into_string()));
        assert!(!debug.contains("[7, 7, 7"), "{debug}");
        assert!(!debug.contains("7,7,7"), "{debug}");
    }

    #[test]
    fn converts_sol_without_float_rounding() {
        assert_eq!(
            sol_to_lamports(&toml::Value::Float(0.1)).unwrap(),
            100_000_000
        );
        assert_eq!(
            sol_to_lamports(&toml::Value::String("1.000000001".into())).unwrap(),
            1_000_000_001
        );
        assert_eq!(
            sol_to_lamports(&toml::Value::Integer(2)).unwrap(),
            2_000_000_000
        );
        assert!(sol_to_lamports(&toml::Value::String("0.0000000001".into())).is_err());
        assert!(sol_to_lamports(&toml::Value::String("abc".into())).is_err());
    }
}
