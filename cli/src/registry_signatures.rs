//! Signed registry package releases.
//!
//! Every package release entry of an install descriptor or resolve result
//! carries two kinds of Ed25519 signature by the registry keys:
//!
//! * `signatures: [{ keyId, signature }]` over the release identity alone:
//!   the UTF-8 message `arete.registry-package-release/v1\n<keyId>\n<packageReleaseHash>`.
//!   a4 cannot recompute that identity, so this signature does not bind the
//!   artifacts a response carries; it is only checked in the warning-only
//!   mode, against registries that predate content signatures.
//! * `contentSignatures: [{ keyId, signature }]` over the digest of the
//!   entry's `releaseContent` descriptor (kind, owner, package, version,
//!   release identity and every artifact hash): the message
//!   `arete.registry-package-release/v2\n<keyId>\n<digest>`. a4 recomputes the
//!   digest and checks the descriptor against what the response installs;
//!   see [`content`].
//!
//! Signatures are 64 bytes, padded standard base64.
//!
//! Trust anchors come from `cli/registry-keys.json` compiled into `a4`, or an
//! explicitly provisioned `ARETE_REGISTRY_TRUSTED_KEYS_FILE`. Registry key
//! responses and their cache may retire a matching pinned key; they cannot
//! introduce authorities or replace public keys. Strict verification requires
//! a current key: unsigned publication times cannot revive a retired authority.
//!
//! Signatures are required by default when trust anchors are provisioned:
//! then every release entry needs a valid content signature that matches
//! the response. Empty bootstrap builds warn until pins are installed.
//! Setting `ARETE_REQUIRE_REGISTRY_SIGNATURES=1` always requires
//! verification; `0` turns refusals into warnings.

pub mod content;

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use base64::Engine;
use colored::Colorize;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Schema of the key list (`GET /api/registry/v1/keys` and the compiled-in
/// file).
pub const KEYS_SCHEMA: &str = "arete.registry-keys/v1";
/// First line of every signed message.
pub const SIGNATURE_DOMAIN: &str = "arete.registry-package-release/v1";
/// Set to `1` to refuse releases without a valid signature.
pub const REQUIRE_SIGNATURES_ENV: &str = "ARETE_REQUIRE_REGISTRY_SIGNATURES";
/// Whether a missing or invalid signature refuses the response when
/// [`REQUIRE_SIGNATURES_ENV`] is not set.
const REQUIRE_SIGNATURES_BY_DEFAULT: bool = false;
/// How long a fetched key list is reused before it is fetched again.
pub const KEYS_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// The keys compiled into this build.
const EMBEDDED_KEYS: &str = include_str!("../registry-keys.json");
const KEYS_CACHE_SCHEMA: &str = "arete.registry-keys-cache/v1";
const MAX_KEY_ID_LEN: usize = 64;

/// The key list document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryKeysDocument {
    pub schema: String,
    pub keys: Vec<RegistryKeyEntry>,
}

/// One registry signing key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryKeyEntry {
    pub key_id: String,
    /// Base58 Ed25519 public key.
    pub public_key: String,
    /// Unix seconds after which the key signs nothing new.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired_at: Option<i64>,
}

#[cfg(test)]
thread_local! {
    /// Test hooks for this thread: whether signatures are required, and the
    /// key cache file, so tests touch neither the process environment nor
    /// the user's Arete home.
    // Mock registries are unsigned unless a test explicitly checks trust.
    // Production builds do not compile this override.
    pub(crate) static TEST_REQUIRE: std::cell::Cell<Option<bool>> =
        const { std::cell::Cell::new(Some(false)) };
    pub(crate) static TEST_CACHE_FILE: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static TEST_TRUST_FILE: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

fn trusted_keys_file() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(path) = TEST_TRUST_FILE.with(|file| file.borrow().clone()) {
        return Some(path);
    }
    std::env::var_os("ARETE_REGISTRY_TRUSTED_KEYS_FILE").map(PathBuf::from)
}

/// Whether missing or invalid signatures refuse the response.
pub fn require_signatures() -> bool {
    #[cfg(test)]
    if let Some(require) = TEST_REQUIRE.with(std::cell::Cell::get) {
        return require;
    }
    match std::env::var(REQUIRE_SIGNATURES_ENV) {
        Ok(value) => value.trim() == "1",
        Err(_) => {
            REQUIRE_SIGNATURES_BY_DEFAULT
                || !KeySet::embedded().keys.is_empty()
                || trusted_keys_file().is_some()
        }
    }
}

/// `^[a-z0-9][a-z0-9-]{0,63}$`.
pub fn is_valid_key_id(key_id: &str) -> bool {
    let bytes = key_id.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_KEY_ID_LEN
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// The message key `key_id` signs for the release `identity`.
pub fn signed_message(key_id: &str, identity: &str) -> String {
    format!("{SIGNATURE_DOMAIN}\n{key_id}\n{identity}")
}

#[derive(Debug, Clone)]
struct TrustedKey {
    key: VerifyingKey,
    retired_at: Option<i64>,
}

/// The keys signatures are checked against.
#[derive(Debug, Clone, Default)]
pub struct KeySet {
    keys: BTreeMap<String, TrustedKey>,
    /// Why the registry's keys are missing, when they are.
    pub problem: Option<String>,
    /// Whether the fetched keys came from the cache (a refresh may find
    /// newer ones).
    pub from_cache: bool,
}

impl KeySet {
    /// Parse and validate a key list document.
    pub fn from_document(document: &RegistryKeysDocument) -> Result<Self> {
        if document.schema != KEYS_SCHEMA {
            bail!(
                "registry key list has schema '{}'; this a4 reads '{KEYS_SCHEMA}'",
                document.schema
            );
        }
        let mut keys = BTreeMap::new();
        for entry in &document.keys {
            if !is_valid_key_id(&entry.key_id) {
                bail!(
                    "registry key list names an invalid key id '{}'",
                    entry.key_id
                );
            }
            let bytes = bs58::decode(&entry.public_key)
                .into_vec()
                .with_context(|| format!("registry key '{}' is not base58", entry.key_id))?;
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("registry key '{}' is not 32 bytes", entry.key_id))?;
            let key = VerifyingKey::from_bytes(&bytes).with_context(|| {
                format!("registry key '{}' is not an Ed25519 key", entry.key_id)
            })?;
            if keys
                .insert(
                    entry.key_id.clone(),
                    TrustedKey {
                        key,
                        retired_at: entry.retired_at,
                    },
                )
                .is_some()
            {
                bail!("registry key list names '{}' twice", entry.key_id);
            }
        }
        Ok(Self {
            keys,
            problem: None,
            from_cache: false,
        })
    }

    /// Parse a key list from JSON.
    pub fn from_json(value: &Value) -> Result<Self> {
        let document: RegistryKeysDocument =
            serde_json::from_value(value.clone()).context("invalid registry key list")?;
        Self::from_document(&document)
    }

    /// The keys compiled into this build.
    pub fn embedded() -> Self {
        serde_json::from_str::<Value>(EMBEDDED_KEYS)
            .map_err(anyhow::Error::from)
            .and_then(|value| Self::from_json(&value))
            .unwrap_or_else(|error| Self {
                problem: Some(format!(
                    "the compiled-in registry keys are invalid: {error}"
                )),
                ..Self::default()
            })
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Fetched/cache metadata can retire a pinned key, never introduce a new
    /// trust anchor or replace a pinned public key. Rotation adds pins via a
    /// signed CLI release or an explicit operator-managed trust file.
    fn merge(&mut self, other: KeySet) {
        for (key_id, key) in other.keys {
            if let Some(existing) = self
                .keys
                .get_mut(&key_id)
                .filter(|existing| existing.key == key.key && existing.retired_at.is_none())
            {
                existing.retired_at = key.retired_at;
            }
        }
        if self.problem.is_none() {
            self.problem = other.problem;
        }
        self.from_cache = other.from_cache;
    }
}

/// The outcome of checking one release's signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A current key signed it.
    Verified { key_id: String },
    /// Only a retired key signed it. Strict enforcement refuses it because
    /// publication timestamps are not bound by the release signature.
    VerifiedRetired { key_id: String },
    /// No signatures.
    Unsigned,
    /// Signed only by keys this CLI does not know.
    UnknownKeys { key_ids: Vec<String> },
    /// A signature is malformed or does not verify.
    Invalid { reason: String },
}

fn parse_timestamp(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.timestamp())
}

/// Check the `signatures` of the release `identity` against `keys`.
pub fn verify_release(
    identity: &str,
    signatures: Option<&Value>,
    published_at: Option<&str>,
    keys: &KeySet,
) -> Verdict {
    verify_signed(signed_message, identity, signatures, published_at, keys)
}

/// Check `signatures` over the message `message(keyId, signed)` against
/// `keys`.
fn verify_signed(
    message: fn(&str, &str) -> String,
    signed: &str,
    signatures: Option<&Value>,
    published_at: Option<&str>,
    keys: &KeySet,
) -> Verdict {
    let signatures = match signatures {
        None | Some(Value::Null) => return Verdict::Unsigned,
        Some(Value::Array(signatures)) if signatures.is_empty() => return Verdict::Unsigned,
        Some(Value::Array(signatures)) => signatures,
        Some(_) => {
            return Verdict::Invalid {
                reason: "`signatures` is not a list".into(),
            }
        }
    };
    let published_at = published_at.and_then(parse_timestamp);
    let mut unknown = Vec::new();
    let mut failures = Vec::new();
    let mut retired = None;
    for signature in signatures {
        let Some(key_id) = signature.get("keyId").and_then(Value::as_str) else {
            failures.push("a signature has no keyId".to_string());
            continue;
        };
        if !is_valid_key_id(key_id) {
            failures.push(format!("invalid key id '{key_id}'"));
            continue;
        }
        let bytes = signature
            .get("signature")
            .and_then(Value::as_str)
            .and_then(|text| base64::engine::general_purpose::STANDARD.decode(text).ok())
            .and_then(|bytes| <[u8; 64]>::try_from(bytes).ok());
        let Some(bytes) = bytes else {
            failures.push(format!(
                "the signature by key '{key_id}' is not 64 bytes of base64"
            ));
            continue;
        };
        let Some(trusted) = keys.keys.get(key_id) else {
            unknown.push(key_id.to_string());
            continue;
        };
        let message = message(key_id, signed);
        if trusted
            .key
            .verify_strict(message.as_bytes(), &Signature::from_bytes(&bytes))
            .is_err()
        {
            failures.push(format!("the signature by key '{key_id}' does not verify"));
            continue;
        }
        match (trusted.retired_at, published_at) {
            (None, _) => {
                return Verdict::Verified {
                    key_id: key_id.to_string(),
                }
            }
            (Some(retired_at), Some(published)) if published < retired_at => {
                retired = retired.or_else(|| Some(key_id.to_string()));
            }
            (Some(_), Some(_)) => failures.push(format!(
                "key '{key_id}' was retired before this release was published"
            )),
            (Some(_), None) => retired = retired.or_else(|| Some(key_id.to_string())),
        }
    }
    if let Some(key_id) = retired {
        return Verdict::VerifiedRetired { key_id };
    }
    if !failures.is_empty() {
        return Verdict::Invalid {
            reason: failures.join("; "),
        };
    }
    Verdict::UnknownKeys { key_ids: unknown }
}

/// Where package release entries sit in a registry response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseShape {
    /// `POST /api/registry/v1/resolve`: `dependencies[]`.
    ResolveResponse,
    /// One resolved dependency, or a protocol install descriptor.
    Dependency,
    /// A stack install descriptor: its programs' `programPackage`.
    StackInstall,
    /// A program install descriptor (or a stack's program): `programPackage`.
    ProgramInstall,
}

/// Called on each package release entry with what the response installs
/// for that release.
type ReleaseVisitor<'a> = dyn FnMut(&mut Map<String, Value>, content::Observed) + 'a;

fn release_object(
    object: &mut Map<String, Value>,
    observed: content::Observed,
    f: &mut ReleaseVisitor<'_>,
) {
    if object
        .get("packageReleaseHash")
        .is_some_and(Value::is_string)
    {
        f(object, observed);
    }
}

fn each_mut(value: &mut Value, key: &str, mut f: impl FnMut(&mut Value)) {
    if let Some(items) = value.get_mut(key).and_then(Value::as_array_mut) {
        for item in items {
            f(item);
        }
    }
}

/// Call `f` on every package release entry of `value`.
fn for_each_release(value: &mut Value, shape: ReleaseShape, f: &mut ReleaseVisitor<'_>) {
    match shape {
        ReleaseShape::ResolveResponse => each_mut(value, "dependencies", |dependency| {
            for_each_release(dependency, ReleaseShape::Dependency, f)
        }),
        ReleaseShape::Dependency => {
            let kind = value.get("kind").and_then(Value::as_str).map(str::to_owned);
            let observed = content::observe_dependency(value);
            if let Some(object) = value.as_object_mut() {
                release_object(object, observed, f);
            }
            match kind.as_deref() {
                Some("program") => {
                    if let Some(install) = value.get_mut("install") {
                        for_each_release(install, ReleaseShape::ProgramInstall, f);
                    }
                }
                Some("protocol") => {
                    each_mut(value, "stacks", |stack| {
                        for_each_release(stack, ReleaseShape::Dependency, f)
                    });
                    each_mut(value, "programs", |program| {
                        for_each_release(program, ReleaseShape::ProgramInstall, f)
                    });
                }
                _ => each_mut(value, "programs", |program| {
                    for_each_release(program, ReleaseShape::ProgramInstall, f)
                }),
            }
        }
        ReleaseShape::StackInstall => each_mut(value, "programs", |program| {
            for_each_release(program, ReleaseShape::ProgramInstall, f)
        }),
        ReleaseShape::ProgramInstall => {
            let observed = content::observe_program_package(value);
            if let Some(object) = value
                .get_mut("programPackage")
                .and_then(Value::as_object_mut)
            {
                release_object(object, observed, f);
            }
        }
    }
}

/// One package release entry of a response, with its signatures.
#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseEntry {
    /// `package@version` when the entry names them, else the identity.
    pub label: String,
    pub identity: String,
    pub package: Option<String>,
    pub version: Option<String>,
    pub signatures: Option<Value>,
    pub published_at: Option<String>,
    /// The entry's `releaseContent` descriptor.
    pub content: Option<Value>,
    pub content_signatures: Option<Value>,
    /// What the response installs for this release.
    pub observed: content::Observed,
}

/// The package release entries of `value`, with their `signatures`,
/// `releaseContent` and `contentSignatures` removed so the response still
/// matches the typed contract it is read into.
pub fn take_release_entries(value: &mut Value, shape: ReleaseShape) -> Vec<ReleaseEntry> {
    let mut entries = Vec::new();
    for_each_release(value, shape, &mut |object, observed| {
        let identity = object
            .get("packageReleaseHash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let package = object
            .get("package")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let version = object
            .get("version")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let label = match (&package, &version) {
            (Some(package), Some(version)) => format!("{package}@{version}"),
            (Some(package), None) => package.clone(),
            _ => identity.clone(),
        };
        entries.push(ReleaseEntry {
            label,
            identity,
            package,
            version,
            signatures: object.remove("signatures"),
            published_at: object
                .get("publishedAt")
                .and_then(Value::as_str)
                .map(str::to_owned),
            content: object.remove("releaseContent"),
            content_signatures: object.remove("contentSignatures"),
            observed,
        });
    });
    entries
}

/// Check an entry's release content signatures, then the descriptor against
/// the entry and what the response installs for it.
pub fn verify_content(entry: &ReleaseEntry, keys: &KeySet) -> Verdict {
    let present = |value: &Option<Value>| value.as_ref().is_some_and(|value| !value.is_null());
    if !present(&entry.content) && !present(&entry.content_signatures) {
        return Verdict::Unsigned;
    }
    let invalid = |reason: String| Verdict::Invalid { reason };
    let Some(value) = entry.content.as_ref().filter(|value| !value.is_null()) else {
        return invalid("content signatures without `releaseContent`".into());
    };
    let descriptor: content::ReleaseContent = match serde_json::from_value(value.clone()) {
        Ok(descriptor) => descriptor,
        Err(error) => return invalid(format!("invalid `releaseContent`: {error}")),
    };
    let digest = match descriptor.digest() {
        Ok(digest) => digest,
        Err(error) => return invalid(format!("{error} (rule: {})", error.code())),
    };
    let verdict = verify_signed(
        content::signed_message,
        &digest,
        entry.content_signatures.as_ref(),
        entry.published_at.as_deref(),
        keys,
    );
    let verdict = match verdict {
        Verdict::Unsigned => return invalid("`releaseContent` without content signatures".into()),
        Verdict::Verified { .. } | Verdict::VerifiedRetired { .. } => verdict,
        other => return other,
    };
    let mut problems = Vec::new();
    for (field, signed, served) in [
        (
            "package release",
            &descriptor.package_release_hash,
            Some(&entry.identity),
        ),
        ("package", &descriptor.package, entry.package.as_ref()),
        ("version", &descriptor.version, entry.version.as_ref()),
    ] {
        if Some(signed) != served {
            problems.push(format!(
                "the signed release content names {field} {signed}, but the response names {}",
                served.map(String::as_str).unwrap_or("none")
            ));
        }
    }
    problems.extend(entry.observed.check(&descriptor));
    if problems.is_empty() {
        verdict
    } else {
        invalid(problems.join("; "))
    }
}

/// One checked release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseCheck {
    pub label: String,
    pub identity: String,
    /// The release identity signature (`signatures`).
    pub verdict: Verdict,
    /// The release content signature (`releaseContent`,
    /// `contentSignatures`), checked against the response.
    pub content: Verdict,
}

/// Check every entry. Keys are loaded only when an entry is signed; when a
/// signature names a key the cached list lacks, the list is fetched again
/// once.
pub fn check_entries(
    entries: Vec<ReleaseEntry>,
    load_keys: &mut dyn FnMut(bool) -> KeySet,
) -> Vec<ReleaseCheck> {
    let present = |value: &Option<Value>| value.as_ref().is_some_and(|value| !value.is_null());
    let signed = entries
        .iter()
        .any(|entry| present(&entry.signatures) || present(&entry.content_signatures));
    let mut keys = if signed {
        load_keys(false)
    } else {
        KeySet::default()
    };
    let verify = |keys: &KeySet| -> Vec<ReleaseCheck> {
        entries
            .iter()
            .map(|entry| ReleaseCheck {
                label: entry.label.clone(),
                identity: entry.identity.clone(),
                verdict: verify_release(
                    &entry.identity,
                    entry.signatures.as_ref(),
                    entry.published_at.as_deref(),
                    keys,
                ),
                content: verify_content(entry, keys),
            })
            .collect()
    };
    let mut checks = verify(&keys);
    let unknown = checks.iter().any(|check| {
        matches!(check.verdict, Verdict::UnknownKeys { .. })
            || matches!(check.content, Verdict::UnknownKeys { .. })
    });
    if unknown && keys.from_cache {
        keys = load_keys(true);
        checks = verify(&keys);
    }
    if let Some(problem) = keys.problem.as_deref() {
        for check in &mut checks {
            for verdict in [&mut check.verdict, &mut check.content] {
                if let Verdict::UnknownKeys { key_ids } = verdict {
                    key_ids.push(format!("({problem})"));
                }
            }
        }
    }
    checks
}

/// The warnings for `checks`, or, when `require` is set, an error naming
/// every release without a current valid content signature that matches the
/// response. Retired keys require re-signing with an active authority, even
/// for allegedly historical releases.
///
/// Without `require`, a release whose registry sent no content signature is
/// judged by its identity signature, as before content signatures existed.
pub fn enforce(checks: &[ReleaseCheck], require: bool) -> Result<Vec<String>> {
    let mut unsigned = Vec::new();
    let mut refusals = Vec::new();
    let mut warnings = Vec::new();
    for check in checks {
        let verdict = match (&check.content, require) {
            (Verdict::Unsigned, false) => &check.verdict,
            (content, _) => content,
        };
        match verdict {
            Verdict::Verified { .. } => {}
            Verdict::VerifiedRetired { key_id } => {
                let message = format!("{} is signed only by retired registry key '{key_id}'; re-sign with a current key", check.label);
                if require {
                    refusals.push(message);
                } else {
                    warnings.push(message);
                }
            }
            Verdict::Unsigned => unsigned.push(check.label.clone()),
            Verdict::UnknownKeys { key_ids } => refusals.push(format!(
                "{} is signed only by registry keys this a4 does not know: {}",
                check.label,
                key_ids.join(", ")
            )),
            Verdict::Invalid { reason } => refusals.push(format!(
                "{} has an invalid registry signature: {reason}",
                check.label
            )),
        }
    }
    if !unsigned.is_empty() {
        let message = if require {
            format!(
                "the registry sent no signed release content for {} (this a4 verifies signatures over each release's artifacts)",
                unsigned.join(", ")
            )
        } else {
            format!("the registry sent no signature for {}", unsigned.join(", "))
        };
        refusals.insert(0, message);
    }
    if require && !refusals.is_empty() {
        bail!(
            "Refusing registry releases without a valid signature ({REQUIRE_SIGNATURES_ENV}=1):\n  - {}",
            refusals.join("\n  - ")
        );
    }
    refusals.extend(warnings);
    Ok(refusals)
}

/// Print each warning once per process, on stderr so `--json` output stays
/// clean.
pub fn emit_warnings(warnings: &[String]) {
    static PRINTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let printed = PRINTED.get_or_init(|| Mutex::new(HashSet::new()));
    let Ok(mut printed) = printed.lock() else {
        return;
    };
    for warning in warnings {
        if printed.insert(warning.clone()) {
            eprintln!(
                "{} Registry signature: {warning}",
                crate::ui::symbols::WARNING.yellow().bold()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Key cache
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeysCache {
    schema: String,
    api_url: String,
    fetched_at: u64,
    keys: Value,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

/// `~/.arete/cache/registry-keys/<sha256(api url)[..16]>.json`.
pub fn keys_cache_path(api_url: &str) -> Result<PathBuf> {
    use sha2::Digest;
    let digest = format!("{:x}", sha2::Sha256::digest(api_url.as_bytes()));
    Ok(crate::arete_home::arete_home()?
        .join("cache")
        .join("registry-keys")
        .join(format!("{}.json", &digest[..16])))
}

fn read_cache(path: &std::path::Path, api_url: &str) -> Option<KeysCache> {
    let bytes = std::fs::read(path).ok()?;
    let cache: KeysCache = serde_json::from_slice(&bytes).ok()?;
    (cache.schema == KEYS_CACHE_SCHEMA && cache.api_url == api_url).then_some(cache)
}

fn write_cache(path: &std::path::Path, api_url: &str, keys: &Value) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid key cache path"))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create {}", parent.display()))?;
    let cache = KeysCache {
        schema: KEYS_CACHE_SCHEMA.into(),
        api_url: api_url.into(),
        fetched_at: now_secs(),
        keys: keys.clone(),
    };
    let temporary = path.with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, serde_json::to_vec_pretty(&cache)?)
        .with_context(|| format!("Failed to write {}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

/// The keys to check `api_url`'s signatures against: the compiled-in keys
/// plus the registry's own list, from the cache while it is fresh (and
/// `refresh` is not set), else from `fetch`. `fetch` answers `None` when
/// the registry has no key list. A failed fetch falls back to a stale
/// cache.
pub fn load_keys(
    api_url: &str,
    refresh: bool,
    fetch: &dyn Fn() -> Result<Option<Value>>,
) -> KeySet {
    #[cfg(test)]
    if let Some(file) = TEST_CACHE_FILE.with(|file| file.borrow().clone()) {
        return load_keys_with_cache(Some(&file), api_url, refresh, fetch);
    }
    // The cache lives in the owner-only Arete home directory.
    let cache = crate::arete_home::ensure_arete_home()
        .ok()
        .and_then(|_| keys_cache_path(api_url).ok());
    load_keys_with_cache(cache.as_deref(), api_url, refresh, fetch)
}

/// [`load_keys`] with the cache file at `cache` (none: no cache).
fn load_keys_with_cache(
    cache: Option<&std::path::Path>,
    api_url: &str,
    refresh: bool,
    fetch: &dyn Fn() -> Result<Option<Value>>,
) -> KeySet {
    let mut keys = match trusted_keys_file() {
        Some(path) => match std::fs::read(path)
            .context("Cannot read configured registry trust file")
            .and_then(|bytes| serde_json::from_slice(&bytes).context("Invalid registry trust file"))
            .and_then(|value| KeySet::from_json(&value))
        {
            Ok(keys) => keys,
            Err(error) => KeySet {
                problem: Some(format!(
                    "Configured registry trust file is invalid: {error}"
                )),
                ..KeySet::default()
            },
        },
        None => KeySet::embedded(),
    };
    let cached = cache.and_then(|path| read_cache(path, api_url));
    if !refresh {
        if let Some(cache) = cached
            .as_ref()
            .filter(|cache| now_secs().saturating_sub(cache.fetched_at) < KEYS_TTL.as_secs())
        {
            if let Ok(mut fetched) = KeySet::from_json(&cache.keys) {
                fetched.from_cache = true;
                keys.merge(fetched);
                return keys;
            }
        }
    }
    let fetched = match fetch() {
        Ok(Some(value)) => match KeySet::from_json(&value) {
            Ok(fetched) => {
                // Best effort: an unwritable cache only means fetching again.
                if let Some(path) = cache {
                    let _ = write_cache(path, api_url, &value);
                }
                fetched
            }
            Err(error) => KeySet {
                problem: Some(format!("{error:#}")),
                ..KeySet::default()
            },
        },
        Ok(None) => KeySet {
            problem: Some("the registry publishes no signing keys".into()),
            ..KeySet::default()
        },
        Err(error) => {
            let problem = format!("could not fetch the registry signing keys: {error:#}");
            match cached.and_then(|cache| KeySet::from_json(&cache.keys).ok()) {
                Some(stale) => KeySet {
                    problem: Some(problem),
                    ..stale
                },
                None => KeySet {
                    problem: Some(problem),
                    ..KeySet::default()
                },
            }
        }
    };
    keys.merge(fetched);
    keys
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;

    pub(crate) fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    pub(crate) fn key_entry(key_id: &str, key: &SigningKey, retired_at: Option<i64>) -> Value {
        let mut entry = json!({
            "keyId": key_id,
            "publicKey": bs58::encode(key.verifying_key().to_bytes()).into_string(),
        });
        if let Some(retired_at) = retired_at {
            entry["retiredAt"] = json!(retired_at);
        }
        entry
    }

    pub(crate) fn keys_document(entries: Vec<Value>) -> Value {
        json!({ "schema": KEYS_SCHEMA, "keys": entries })
    }

    pub(crate) fn sign(key_id: &str, key: &SigningKey, identity: &str) -> Value {
        let signature = key.sign(signed_message(key_id, identity).as_bytes());
        json!({
            "keyId": key_id,
            "signature": base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
        })
    }

    /// A `contentSignatures` entry by `key` over the release content
    /// descriptor `content`.
    pub(crate) fn sign_content(key_id: &str, key: &SigningKey, content: &Value) -> Value {
        let digest = serde_json::from_value::<content::ReleaseContent>(content.clone())
            .unwrap()
            .digest()
            .unwrap();
        let signature = key.sign(content::signed_message(key_id, &digest).as_bytes());
        json!({
            "keyId": key_id,
            "signature": base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
        })
    }

    const IDENTITY: &str = "arete:registry-package-release:v2:sha256:abc";

    fn keys(entries: Vec<Value>) -> KeySet {
        KeySet::from_json(&keys_document(entries)).unwrap()
    }

    #[test]
    fn the_compiled_in_key_list_is_valid() {
        let embedded = KeySet::embedded();
        assert!(embedded.problem.is_none(), "{:?}", embedded.problem);
    }

    #[test]
    fn production_registry_pin_requires_signatures_by_default() {
        let pins = KeySet::embedded();
        assert!(pins.keys.contains_key("registry-prod-1"));
        TEST_REQUIRE.with(|required| required.set(None));
        let required = require_signatures();
        TEST_REQUIRE.with(|required| required.set(Some(false)));
        assert!(required);
    }

    #[test]
    fn the_signed_message_is_domain_key_and_identity_on_three_lines() {
        assert_eq!(
            signed_message("arete-2026", IDENTITY),
            format!("arete.registry-package-release/v1\narete-2026\n{IDENTITY}")
        );
    }

    #[test]
    fn key_ids_follow_the_contract_pattern() {
        for valid in ["a", "0", "arete-2026-a", &"a".repeat(64)] {
            assert!(is_valid_key_id(valid), "{valid}");
        }
        for invalid in ["", "-a", "A", "a_b", "a.b", &"a".repeat(65)] {
            assert!(!is_valid_key_id(invalid), "{invalid}");
        }
    }

    #[test]
    fn a_signature_by_a_current_key_verifies() {
        let key = signing_key(1);
        let set = keys(vec![key_entry("k1", &key, None)]);
        let signatures = json!([sign("k1", &key, IDENTITY)]);
        assert_eq!(
            verify_release(IDENTITY, Some(&signatures), None, &set),
            Verdict::Verified {
                key_id: "k1".into()
            }
        );
    }

    #[test]
    fn missing_and_empty_signatures_are_unsigned() {
        let set = KeySet::default();
        assert_eq!(
            verify_release(IDENTITY, None, None, &set),
            Verdict::Unsigned
        );
        assert_eq!(
            verify_release(IDENTITY, Some(&json!([])), None, &set),
            Verdict::Unsigned
        );
    }

    #[test]
    fn a_signature_over_another_identity_or_key_id_is_invalid() {
        let key = signing_key(1);
        let set = keys(vec![
            key_entry("k1", &key, None),
            key_entry("k2", &key, None),
        ]);
        let other = json!([sign(
            "k1",
            &key,
            "arete:registry-package-release:v2:sha256:def"
        )]);
        assert!(matches!(
            verify_release(IDENTITY, Some(&other), None, &set),
            Verdict::Invalid { .. }
        ));
        // Signed for k1, presented as k2: the key id is part of the message.
        let mut moved = sign("k1", &key, IDENTITY);
        moved["keyId"] = json!("k2");
        assert!(matches!(
            verify_release(IDENTITY, Some(&json!([moved])), None, &set),
            Verdict::Invalid { .. }
        ));
    }

    #[test]
    fn malformed_signatures_are_invalid() {
        let set = keys(vec![key_entry("k1", &signing_key(1), None)]);
        for signatures in [
            json!("nope"),
            json!([{ "signature": "AAAA" }]),
            json!([{ "keyId": "K1", "signature": "AAAA" }]),
            json!([{ "keyId": "k1", "signature": "not base64!" }]),
            json!([{ "keyId": "k1", "signature": "AAAA" }]),
        ] {
            assert!(
                matches!(
                    verify_release(IDENTITY, Some(&signatures), None, &set),
                    Verdict::Invalid { .. }
                ),
                "{signatures}"
            );
        }
    }

    #[test]
    fn a_signature_by_an_unknown_key_is_reported_as_such() {
        let key = signing_key(1);
        let signatures = json!([sign("k9", &key, IDENTITY)]);
        assert_eq!(
            verify_release(IDENTITY, Some(&signatures), None, &KeySet::default()),
            Verdict::UnknownKeys {
                key_ids: vec!["k9".into()]
            }
        );
    }

    #[test]
    fn a_retired_key_cannot_become_current_through_backdating() {
        let key = signing_key(1);
        let retired_at = 1_760_000_000;
        let set = keys(vec![key_entry("old", &key, Some(retired_at))]);
        let signatures = json!([sign("old", &key, IDENTITY)]);
        assert_eq!(
            verify_release(
                IDENTITY,
                Some(&signatures),
                Some("2025-01-01T00:00:00Z"),
                &set
            ),
            Verdict::VerifiedRetired {
                key_id: "old".into()
            }
        );
        assert!(matches!(
            verify_release(
                IDENTITY,
                Some(&signatures),
                Some("2026-01-01T00:00:00Z"),
                &set
            ),
            Verdict::Invalid { .. }
        ));
        assert_eq!(
            verify_release(IDENTITY, Some(&signatures), None, &set),
            Verdict::VerifiedRetired {
                key_id: "old".into()
            }
        );
    }

    #[test]
    fn invalid_key_lists_are_rejected() {
        for document in [
            json!({ "schema": "other", "keys": [] }),
            keys_document(vec![
                json!({ "keyId": "Bad", "publicKey": "11111111111111111111111111111111" }),
            ]),
            keys_document(vec![json!({ "keyId": "k1", "publicKey": "abc" })]),
            keys_document(vec![
                key_entry("k1", &signing_key(1), None),
                key_entry("k1", &signing_key(2), None),
            ]),
        ] {
            assert!(KeySet::from_json(&document).is_err(), "{document}");
        }
    }

    fn resolve_response(signatures: Option<Value>) -> Value {
        let mut program_package = json!({
            "package": "ore-program",
            "version": "1.0.0",
            "packageReleaseHash": "arete:registry-package-release:v2:sha256:p",
        });
        let mut stack = json!({
            "kind": "stack",
            "alias": "ore",
            "package": "ore",
            "version": "1.1.0",
            "packageReleaseHash": IDENTITY,
            "stackManifest": { "signatures": "not a release entry" },
            "programs": [ { "programPackage": program_package.clone() } ],
        });
        if let Some(signatures) = signatures {
            stack["signatures"] = signatures.clone();
            program_package["signatures"] = signatures;
            stack["programs"][0]["programPackage"] = program_package;
        }
        json!({ "resolverContract": "x", "dependencies": [stack] })
    }

    #[test]
    fn release_entries_are_found_and_their_signatures_removed() {
        let mut value = resolve_response(Some(json!([])));
        let entries = take_release_entries(&mut value, ReleaseShape::ResolveResponse);
        let labels: Vec<&str> = entries.iter().map(|entry| entry.label.as_str()).collect();
        assert_eq!(labels, ["ore@1.1.0", "ore-program@1.0.0"]);
        let stack = &value["dependencies"][0];
        assert!(stack.get("signatures").is_none());
        assert!(stack["programs"][0]["programPackage"]
            .get("signatures")
            .is_none());
        // Content that is not a release entry is left alone.
        assert_eq!(
            stack["stackManifest"]["signatures"],
            json!("not a release entry")
        );
    }

    #[test]
    fn protocol_descriptors_include_their_hoisted_stacks_and_programs() {
        let mut value = json!({
            "kind": "protocol",
            "package": "tip-jar",
            "version": "1.0.0",
            "packageReleaseHash": "arete:registry-package-release:v2:sha256:t",
            "stacks": [ { "kind": "stack", "package": "tips", "version": "1.0.0", "packageReleaseHash": "arete:registry-package-release:v2:sha256:s", "programs": [] } ],
            "programs": [ { "programPackage": { "package": "system", "version": "1.0.0", "packageReleaseHash": "arete:registry-package-release:v2:sha256:y" } } ],
        });
        let labels: Vec<String> = take_release_entries(&mut value, ReleaseShape::Dependency)
            .into_iter()
            .map(|entry| entry.label)
            .collect();
        assert_eq!(labels, ["tip-jar@1.0.0", "tips@1.0.0", "system@1.0.0"]);
    }

    #[test]
    fn unsigned_releases_are_one_warning_and_keys_are_not_loaded() {
        let mut value = resolve_response(None);
        let entries = take_release_entries(&mut value, ReleaseShape::ResolveResponse);
        let checks = check_entries(entries, &mut |_| panic!("no keys are needed"));
        let warnings = enforce(&checks, false).unwrap();
        assert_eq!(
            warnings,
            ["the registry sent no signature for ore@1.1.0, ore-program@1.0.0"]
        );
        let error = enforce(&checks, true).unwrap_err().to_string();
        assert!(error.contains(REQUIRE_SIGNATURES_ENV), "{error}");
        assert!(error.contains("ore@1.1.0"), "{error}");
    }

    #[test]
    fn signed_releases_pass_and_an_unknown_key_refreshes_the_cached_list_once() {
        let key = signing_key(3);
        let mut value = resolve_response(None);
        value["dependencies"][0]["signatures"] = json!([sign("k3", &key, IDENTITY)]);
        let entries = take_release_entries(&mut value, ReleaseShape::ResolveResponse);
        let mut loads = Vec::new();
        let checks = check_entries(entries, &mut |refresh| {
            loads.push(refresh);
            if refresh {
                keys(vec![key_entry("k3", &key, None)])
            } else {
                KeySet {
                    from_cache: true,
                    ..KeySet::default()
                }
            }
        });
        assert_eq!(loads, [false, true]);
        assert_eq!(
            checks[0].verdict,
            Verdict::Verified {
                key_id: "k3".into()
            }
        );
        // The program package is still unsigned.
        assert_eq!(checks[1].verdict, Verdict::Unsigned);
    }

    #[test]
    fn retired_keys_warn_in_legacy_mode_but_strict_verification_refuses() {
        let checks = [ReleaseCheck {
            label: "ore@1.1.0".into(),
            identity: IDENTITY.into(),
            verdict: Verdict::VerifiedRetired {
                key_id: "old".into(),
            },
            content: Verdict::VerifiedRetired {
                key_id: "old".into(),
            },
        }];
        assert!(enforce(&checks, true).is_err());
        let warnings = enforce(&checks, false).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("retired"), "{}", warnings[0]);
    }

    #[test]
    fn the_key_list_is_cached_and_refetched_when_stale_or_refreshed() {
        let anchors = KeySet::embedded().len();
        let home = tempfile::tempdir().unwrap();
        let cache_of = |api: &str| home.path().join(format!("{}.json", api.len()));
        let load = |api: &str, refresh: bool, fetch: &dyn Fn() -> Result<Option<Value>>| {
            load_keys_with_cache(Some(&cache_of(api)), api, refresh, fetch)
        };
        let api = "http://registry.test";
        let key = signing_key(4);
        let document = keys_document(vec![key_entry("k4", &key, None)]);
        let fetches = std::cell::Cell::new(0);
        let fetch = || -> Result<Option<Value>> {
            fetches.set(fetches.get() + 1);
            Ok(Some(document.clone()))
        };

        let first = load(api, false, &fetch);
        assert_eq!(
            (first.len(), first.from_cache, fetches.get()),
            (anchors, false, 1)
        );
        let second = load(api, false, &fetch);
        assert_eq!(
            (second.len(), second.from_cache, fetches.get()),
            (anchors, true, 1)
        );
        let refreshed = load(api, true, &fetch);
        assert_eq!((refreshed.from_cache, fetches.get()), (false, 2));

        // Another API has its own cache entry.
        load("http://other.test/x", false, &fetch);
        assert_eq!(fetches.get(), 3);

        // A stale cache is fetched again; a failed fetch falls back to it.
        let path = cache_of(api);
        let mut cache: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        cache["fetchedAt"] = json!(0);
        std::fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        let failing = || -> Result<Option<Value>> { bail!("offline") };
        let stale = load(api, false, &failing);
        assert_eq!(stale.len(), anchors);
        assert!(stale.problem.as_deref().unwrap().contains("offline"));

        // A registry without the route publishes no keys.
        let none = load("http://none.test/xy", false, &|| Ok(None));
        assert_eq!(none.len(), anchors);
        assert!(none.problem.as_deref().unwrap().contains("no signing keys"));

        // The cache is keyed by API URL.
        assert!(keys_cache_path(api).unwrap() != keys_cache_path("http://other.test").unwrap());
    }

    #[test]
    fn configured_trust_file_requires_signatures_and_invalid_files_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.json");
        let trusted = signing_key(4);
        let document = keys_document(vec![key_entry("trusted", &trusted, None)]);
        std::fs::write(&path, document.to_string()).unwrap();
        TEST_TRUST_FILE.with(|file| *file.borrow_mut() = Some(path.clone()));
        TEST_REQUIRE.with(|required| required.set(None));
        let required = require_signatures();
        TEST_REQUIRE.with(|required| required.set(Some(false)));
        let loaded =
            load_keys_with_cache(None, "http://test", false, &|| Ok(Some(document.clone())));
        std::fs::write(&path, "invalid").unwrap();
        let invalid =
            load_keys_with_cache(None, "http://test", false, &|| Ok(Some(document.clone())));
        TEST_TRUST_FILE.with(|file| *file.borrow_mut() = None);
        assert!(required);
        assert_eq!(loaded.len(), 1);
        assert!(invalid.is_empty());
        assert!(invalid.problem.unwrap().contains("trust file is invalid"));
    }

    #[test]
    fn registry_metadata_cannot_replace_pins_or_introduce_signing_authorities() {
        let trusted = signing_key(4);
        let attacker = signing_key(5);
        let mut pins = keys(vec![key_entry("trusted", &trusted, None)]);
        pins.merge(keys(vec![
            key_entry("trusted", &attacker, None),
            key_entry("new", &attacker, None),
        ]));
        assert_eq!(pins.len(), 1);
        assert!(matches!(
            verify_release(
                IDENTITY,
                Some(&json!([sign("trusted", &trusted, IDENTITY)])),
                None,
                &pins
            ),
            Verdict::Verified { .. }
        ));
        assert!(matches!(
            verify_release(
                IDENTITY,
                Some(&json!([sign("new", &attacker, IDENTITY)])),
                None,
                &pins
            ),
            Verdict::UnknownKeys { .. }
        ));
        assert!(matches!(
            verify_release(
                IDENTITY,
                Some(&json!([sign("trusted", &attacker, IDENTITY)])),
                None,
                &pins
            ),
            Verdict::Invalid { .. }
        ));
        let mut no_pins = KeySet::default();
        no_pins.merge(keys(vec![key_entry("new", &attacker, None)]));
        assert!(no_pins.is_empty());
    }

    // -----------------------------------------------------------------------
    // Release content
    // -----------------------------------------------------------------------

    const PROGRAM_ID: &str = "11111111111111111111111111111111";

    fn typed(kind: &str, byte: char) -> String {
        format!("arete:h1:{kind}:sha256:{}", byte.to_string().repeat(64))
    }

    fn release_hash(byte: char) -> String {
        format!(
            "arete:registry-package-release:v2:sha256:{}",
            byte.to_string().repeat(64)
        )
    }

    /// A served SDK extension bundle whose content hash is that of its bytes.
    fn extension_artifact(language: Option<&str>, source: &str) -> Value {
        let mut artifact = json!({
            "artifactHash": "",
            "manifest": { "entry": "index.ts", "files": ["index.ts"] },
            "files": { "index.ts": source },
            "createdAt": "2026-11-01T00:00:00Z",
        });
        if let Some(language) = language {
            artifact["manifest"]["language"] = json!(language);
        }
        let hash = crate::commands::sdk::registry_extension_content_hash(
            &serde_json::from_value(artifact.clone()).unwrap(),
        );
        artifact["artifactHash"] = json!(hash);
        artifact
    }

    fn resolved_extension(target: &str, source: &str) -> Value {
        let artifact = extension_artifact(None, source);
        json!({ "target": target, "contentHash": artifact["artifactHash"], "artifact": artifact })
    }

    /// A resolved stack whose program embeds a program package, both with a
    /// TypeScript SDK extension.
    fn stack_dependency() -> Value {
        let program_extension = resolved_extension("typescript", "export const p = 1;");
        json!({
            "kind": "stack",
            "alias": "ore",
            "package": "ore",
            "version": "1.1.0",
            "packageReleaseHash": release_hash('a'),
            "generatorContract": "sdk-generator-v1",
            "stackManifestHash": typed("stack-manifest", '1'),
            "stackManifest": {},
            "liveSpecs": [],
            "programs": [{
                "installName": "ore-program",
                "definition": {
                    "programId": PROGRAM_ID,
                    "programSpecHash": typed("program-spec", '2'),
                    "extensions": program_extension["artifact"],
                },
                "release": {
                    "programReleaseHash": typed("program-release", '3'),
                    "programSpecHash": typed("program-spec", '2'),
                },
                "programPackage": {
                    "package": "ore-program",
                    "version": "1.0.0",
                    "packageReleaseHash": release_hash('b'),
                },
                "sdkExtensions": [program_extension],
            }],
            "sdkExtensions": [resolved_extension("typescript", "export const s = 1;")],
        })
    }

    /// The descriptors the registry signs for [`stack_dependency`].
    fn stack_contents(dependency: &Value) -> (Value, Value) {
        let program = &dependency["programs"][0];
        let stack = json!({
            "schema": content::RELEASE_CONTENT_SCHEMA,
            "kind": "stack",
            "owner": "public",
            "package": "ore",
            "version": "1.1.0",
            "packageReleaseHash": release_hash('a'),
            "artifacts": [
                { "role": "stack-manifest", "hash": typed("stack-manifest", '1') },
                { "role": "program-spec", "name": PROGRAM_ID, "hash": typed("program-spec", '2') },
                { "role": "program-release", "name": PROGRAM_ID, "hash": typed("program-release", '3') },
                { "role": "program-package", "name": PROGRAM_ID, "hash": release_hash('b') },
                { "role": "sdk-extension", "name": "typescript", "hash": dependency["sdkExtensions"][0]["contentHash"] },
                // Published, not requested.
                { "role": "sdk-extension", "name": "rust", "hash": "5".repeat(64) },
            ],
        });
        let package = json!({
            "schema": content::RELEASE_CONTENT_SCHEMA,
            "kind": "program",
            "owner": "public",
            "package": "ore-program",
            "version": "1.0.0",
            "packageReleaseHash": release_hash('b'),
            "artifacts": [
                { "role": "program-release", "hash": typed("program-release", '3') },
                { "role": "program-spec", "hash": typed("program-spec", '2') },
                { "role": "sdk-extension", "name": "typescript", "hash": program["sdkExtensions"][0]["contentHash"] },
            ],
        });
        (stack, package)
    }

    /// `dependency` as the registry serves it: each release entry with its
    /// signed descriptor (`contents` = stack, program package).
    fn signed_response(dependency: &Value, contents: &(Value, Value)) -> Value {
        let key = signing_key(7);
        let mut dependency = dependency.clone();
        dependency["releaseContent"] = contents.0.clone();
        dependency["contentSignatures"] = json!([sign_content("k7", &key, &contents.0)]);
        let package = &mut dependency["programs"][0]["programPackage"];
        package["releaseContent"] = contents.1.clone();
        package["contentSignatures"] = json!([sign_content("k7", &key, &contents.1)]);
        json!({ "resolverContract": "x", "dependencies": [dependency] })
    }

    fn checked(mut response: Value, require: bool) -> Result<Vec<String>> {
        let entries = take_release_entries(&mut response, ReleaseShape::ResolveResponse);
        let checks = check_entries(entries, &mut |_| {
            keys(vec![key_entry("k7", &signing_key(7), None)])
        });
        enforce(&checks, require)
    }

    fn refusal(response: Value) -> String {
        format!("{:#}", checked(response, true).unwrap_err())
    }

    #[test]
    fn signed_release_content_matching_the_response_passes() {
        let dependency = stack_dependency();
        let mut response = signed_response(&dependency, &stack_contents(&dependency));
        let entries = take_release_entries(&mut response, ReleaseShape::ResolveResponse);
        assert_eq!(entries.len(), 2);
        let stack = &response["dependencies"][0];
        assert!(stack.get("releaseContent").is_none());
        assert!(stack.get("contentSignatures").is_none());
        assert!(stack["programs"][0]["programPackage"]
            .get("releaseContent")
            .is_none());
        let checks = check_entries(entries, &mut |_| {
            keys(vec![key_entry("k7", &signing_key(7), None)])
        });
        for check in &checks {
            assert_eq!(
                check.content,
                Verdict::Verified {
                    key_id: "k7".into()
                },
                "{}",
                check.label
            );
        }
        assert!(enforce(&checks, true).unwrap().is_empty());
    }

    #[test]
    fn swapping_any_installed_artifact_under_the_same_signature_is_refused() {
        let dependency = stack_dependency();
        let contents = stack_contents(&dependency);
        let other_extension = resolved_extension("typescript", "export const evil = 1;");
        type Swap = Box<dyn Fn(&mut Value)>;
        let swaps: Vec<(&str, Swap, &str)> = vec![
            (
                "StackManifest",
                Box::new(|d| d["stackManifestHash"] = json!(typed("stack-manifest", 'f'))),
                "stack-manifest - is",
            ),
            (
                "ProgramSpec",
                Box::new(|d| {
                    d["programs"][0]["definition"]["programSpecHash"] =
                        json!(typed("program-spec", 'f'))
                }),
                "program-spec",
            ),
            (
                "Program Release",
                Box::new(|d| {
                    d["programs"][0]["release"]["programReleaseHash"] =
                        json!(typed("program-release", 'f'))
                }),
                "program-release",
            ),
            (
                "program package",
                Box::new(|d| {
                    d["programs"][0]["programPackage"]["packageReleaseHash"] =
                        json!(release_hash('f'))
                }),
                "program-package",
            ),
            (
                "stack extension bytes",
                Box::new(|d| {
                    d["sdkExtensions"][0]["artifact"]["files"]["index.ts"] =
                        json!("export const evil = 1;")
                }),
                "files hash to",
            ),
            (
                "stack extension bundle",
                Box::new(move |d| d["sdkExtensions"][0] = other_extension.clone()),
                "sdk-extension typescript",
            ),
            (
                "program extension bytes",
                Box::new(|d| {
                    d["programs"][0]["definition"]["extensions"]["files"]["index.ts"] =
                        json!("export const evil = 1;")
                }),
                "files hash to",
            ),
            (
                "extension without a program package",
                Box::new(|d| {
                    d["programs"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("programPackage");
                }),
                "names no program package release",
            ),
        ];
        for (what, swap, expected) in swaps {
            let mut swapped = dependency.clone();
            swap(&mut swapped);
            let error = refusal(signed_response(&swapped, &contents));
            assert!(error.contains(expected), "{what}: {error}");
        }
    }

    #[test]
    fn a_signed_release_cannot_be_relabelled_or_its_descriptor_edited() {
        let dependency = stack_dependency();
        let contents = stack_contents(&dependency);
        for (field, value) in [
            ("package", json!("ore-fork")),
            ("version", json!("9.9.9")),
            ("packageReleaseHash", json!(release_hash('e'))),
        ] {
            let mut relabelled = dependency.clone();
            relabelled[field] = value;
            let error = refusal(signed_response(&relabelled, &contents));
            assert!(
                error.contains("the signed release content names"),
                "{field}: {error}"
            );
        }
        // Served as another kind of release.
        let mut as_program = contents.clone();
        as_program.0["kind"] = json!("program");
        let error = refusal(signed_response(&dependency, &as_program));
        assert!(error.contains("installs it as stack"), "{error}");

        // The descriptor edited after signing: the signature no longer
        // covers it.
        let mut response = signed_response(&dependency, &contents);
        response["dependencies"][0]["releaseContent"]["artifacts"][0]["hash"] =
            json!(typed("stack-manifest", 'f'));
        response["dependencies"][0]["stackManifestHash"] = json!(typed("stack-manifest", 'f'));
        let error = refusal(response);
        assert!(error.contains("does not verify"), "{error}");
    }

    #[test]
    fn identity_only_signatures_are_refused_when_required_and_judged_alone_otherwise() {
        let key = signing_key(7);
        let mut dependency = stack_dependency();
        dependency["signatures"] = json!([sign("k7", &key, &release_hash('a'))]);
        dependency["programs"][0]["programPackage"]["signatures"] =
            json!([sign("k7", &key, &release_hash('b'))]);
        let response = json!({ "resolverContract": "x", "dependencies": [dependency] });
        let error = refusal(response.clone());
        assert!(
            error.contains("no signed release content for ore@1.1.0, ore-program@1.0.0"),
            "{error}"
        );
        assert!(checked(response, false).unwrap().is_empty());

        // A descriptor without signatures, or signatures without one, is
        // invalid rather than unsigned.
        let dependency = stack_dependency();
        let mut response = signed_response(&dependency, &stack_contents(&dependency));
        response["dependencies"][0]
            .as_object_mut()
            .unwrap()
            .remove("contentSignatures");
        assert!(refusal(response).contains("without content signatures"));
    }
}
