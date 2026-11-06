//! Signed registry release content (`arete.registry-release-content/v1`).
//!
//! A release's `packageReleaseHash` is an identity a4 cannot recompute: it
//! commits to registry-side state. A signature over it alone therefore says
//! nothing about the artifacts a response carries. Each package release
//! entry also carries a *release content descriptor*, `releaseContent`:
//!
//! ```json
//! {
//!   "schema": "arete.registry-release-content/v1",
//!   "kind": "stack",
//!   "owner": "public",
//!   "package": "ore",
//!   "version": "1.1.0",
//!   "packageReleaseHash": "arete:registry-package-release:v2:sha256:…",
//!   "artifacts": [
//!     { "role": "stack-manifest", "hash": "arete:h1:stack-manifest:sha256:…" },
//!     { "role": "program-spec", "name": "<program id>", "hash": "…" }
//!   ]
//! }
//! ```
//!
//! The descriptor is encoded as canonical UTF-8 text, one `\n`-terminated
//! line per field, artifacts sorted bytewise by `(role, name)`:
//!
//! ```text
//! arete.registry-release-content/v1
//! kind <kind>
//! owner <owner>
//! package <package>
//! version <version>
//! release <packageReleaseHash>
//! artifact <role> <name or -> <hash>
//! ```
//!
//! Its digest, `arete:registry-release-content:v1:sha256:<hex sha256 of the
//! text>`, is what each registry key signs, as the message
//! `arete.registry-package-release/v2\n<keyId>\n<digest>`. Signatures sit
//! beside the descriptor as `contentSignatures: [{ keyId, signature }]`.
//!
//! a4 recomputes the digest from the descriptor it receives, verifies the
//! signatures over it, then checks the descriptor against the response:
//! kind, package, version and release must match the entry, every artifact
//! hash the response would install must be the one the descriptor names for
//! that role, and every artifact of a known role the descriptor names must
//! be in the response (only SDK extensions may be a subset: a response
//! carries just the requested targets). The SDK extension bundles' bytes are re-hashed here;
//! the other artifacts (StackManifest, LiveSpecs, ProgramSpecs, protocol
//! spec) are re-hashed by the install path before anything is generated.
//!
//! Roles:
//!
//! | kind     | role               | name          | hash                         |
//! |----------|--------------------|---------------|------------------------------|
//! | stack    | `stack-manifest`   | –             | StackManifest                |
//! | stack    | `program-spec`     | program id    | ProgramSpec                  |
//! | stack    | `program-release`  | program id    | Program Release              |
//! | stack    | `program-package`  | program id    | embedded program package release |
//! | program  | `program-spec`     | –             | ProgramSpec                  |
//! | program  | `program-release`  | –             | Program Release              |
//! | protocol | `protocol-release` | –             | Protocol Release             |
//! | protocol | `protocol-spec`    | –             | protocol spec                |
//! | protocol | `deployment`       | –             | deployment id                |
//! | protocol | `stack`            | spec stack name | hoisted stack package release |
//! | protocol | `stack-manifest`   | spec stack name | its StackManifest          |
//! | protocol | `program-spec`     | program id    | ProgramSpec (pinned or unpinned) |
//! | protocol | `program-release`  | program id    | Program Release (pinned)     |
//! | protocol | `program-package`  | program id    | program package release (pinned) |
//! | stack, program | `sdk-extension` | target (`typescript`, `rust`, `python`) | extension content hash |
//!
//! `owner` is `public` for shared catalog releases and `private` for
//! owner-private ones (the release identity itself commits to the exact
//! owner). Unknown roles are hashed like any other and not cross-checked, so
//! a registry can add roles without breaking older a4 builds.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// `schema` of a release content descriptor.
pub const RELEASE_CONTENT_SCHEMA: &str = "arete.registry-release-content/v1";
/// Prefix of a release content digest.
pub const RELEASE_CONTENT_PREFIX: &str = "arete:registry-release-content:v1:sha256:";
/// First line of every message signing a release content digest.
pub const CONTENT_SIGNATURE_DOMAIN: &str = "arete.registry-package-release/v2";
/// The canonical text's name for an artifact without one.
const NO_NAME: &str = "-";
const KINDS: [&str; 3] = ["program", "protocol", "stack"];
const OWNERS: [&str; 2] = ["private", "public"];
/// Roles a4 observes whenever the response carries what they name: every
/// one the descriptor names must be in the response. Only `sdk-extension`
/// may be a subset (a response carries just the requested targets), and
/// unknown roles are not cross-checked.
const REQUIRED_ROLES: [&str; 8] = [
    "deployment",
    "program-package",
    "program-release",
    "program-spec",
    "protocol-release",
    "protocol-spec",
    "stack",
    "stack-manifest",
];
const MAX_TOKEN_LEN: usize = 256;
const MAX_LABEL_LEN: usize = 128;

/// A release content descriptor as the registry serves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseContent {
    pub schema: String,
    pub kind: String,
    pub owner: String,
    pub package: String,
    pub version: String,
    pub package_release_hash: String,
    pub artifacts: Vec<ReleaseArtifact>,
}

/// One artifact a release is made of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseArtifact {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub hash: String,
}

/// Why a descriptor has no canonical encoding. [`Self::code`] is the stable
/// name the shared test vectors use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentError {
    Schema,
    Kind,
    Owner,
    Field(&'static str),
    Role,
    Name,
    Duplicate { role: String, name: String },
}

impl ContentError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Schema => "schema",
            Self::Kind => "kind",
            Self::Owner => "owner",
            Self::Field(_) => "field",
            Self::Role => "role",
            Self::Name => "name",
            Self::Duplicate { .. } => "duplicate",
        }
    }
}

impl std::fmt::Display for ContentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Schema => write!(
                formatter,
                "release content schema is not '{RELEASE_CONTENT_SCHEMA}'"
            ),
            Self::Kind => write!(
                formatter,
                "release content kind is not program, protocol or stack"
            ),
            Self::Owner => write!(formatter, "release content owner is not public or private"),
            Self::Field(field) => write!(
                formatter,
                "release content {field} is empty, too long or not printable ASCII without spaces"
            ),
            Self::Role => write!(formatter, "a release content artifact role is invalid"),
            Self::Name => write!(
                formatter,
                "a release content artifact name is invalid (omit it instead of '-')"
            ),
            Self::Duplicate { role, name } => {
                write!(
                    formatter,
                    "release content names artifact {role} {name} twice"
                )
            }
        }
    }
}

impl std::error::Error for ContentError {}

/// Printable ASCII without spaces.
fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TOKEN_LEN
        && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

/// `[A-Za-z0-9_.-]+`.
fn is_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_LABEL_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

impl ReleaseContent {
    /// The canonical text the digest is taken over.
    pub fn canonical_text(&self) -> Result<String, ContentError> {
        if self.schema != RELEASE_CONTENT_SCHEMA {
            return Err(ContentError::Schema);
        }
        if !KINDS.contains(&self.kind.as_str()) {
            return Err(ContentError::Kind);
        }
        if !OWNERS.contains(&self.owner.as_str()) {
            return Err(ContentError::Owner);
        }
        for (field, value) in [
            ("package", &self.package),
            ("version", &self.version),
            ("packageReleaseHash", &self.package_release_hash),
        ] {
            if !is_token(value) {
                return Err(ContentError::Field(field));
            }
        }
        let mut artifacts = BTreeMap::new();
        for artifact in &self.artifacts {
            if !is_label(&artifact.role) {
                return Err(ContentError::Role);
            }
            let name = match artifact.name.as_deref() {
                None => NO_NAME,
                Some(name) if name != NO_NAME && is_label(name) => name,
                Some(_) => return Err(ContentError::Name),
            };
            if !is_token(&artifact.hash) {
                return Err(ContentError::Field("artifact hash"));
            }
            if artifacts
                .insert((artifact.role.as_str(), name), artifact.hash.as_str())
                .is_some()
            {
                return Err(ContentError::Duplicate {
                    role: artifact.role.clone(),
                    name: name.to_string(),
                });
            }
        }
        let mut text = format!(
            "{RELEASE_CONTENT_SCHEMA}\nkind {}\nowner {}\npackage {}\nversion {}\nrelease {}\n",
            self.kind, self.owner, self.package, self.version, self.package_release_hash
        );
        for ((role, name), hash) in artifacts {
            text.push_str(&format!("artifact {role} {name} {hash}\n"));
        }
        Ok(text)
    }

    /// `arete:registry-release-content:v1:sha256:<hex>` of the canonical text.
    pub fn digest(&self) -> Result<String, ContentError> {
        let text = self.canonical_text()?;
        Ok(format!(
            "{RELEASE_CONTENT_PREFIX}{:x}",
            Sha256::digest(text.as_bytes())
        ))
    }

    fn attested(&self) -> BTreeMap<(&str, &str), &str> {
        self.artifacts
            .iter()
            .map(|artifact| {
                (
                    (
                        artifact.role.as_str(),
                        artifact.name.as_deref().unwrap_or(NO_NAME),
                    ),
                    artifact.hash.as_str(),
                )
            })
            .collect()
    }
}

/// The message key `key_id` signs for the release content `digest`.
pub fn signed_message(key_id: &str, digest: &str) -> String {
    format!("{CONTENT_SIGNATURE_DOMAIN}\n{key_id}\n{digest}")
}

/// What a response would install for one package release entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Observed {
    /// The kind the entry is installed as.
    pub kind: Option<String>,
    /// `(role, name, hash)`, in response order; a role may repeat when the
    /// response carries the same artifact twice.
    pub artifacts: Vec<(String, String, String)>,
    /// Content that fails on its own: bytes that do not hash to their
    /// claimed hash, or an extension no release can attest.
    pub problems: Vec<String>,
}

impl Observed {
    fn push(&mut self, role: &str, name: &str, hash: Option<&str>) {
        if let Some(hash) = hash {
            self.artifacts
                .push((role.to_string(), name.to_string(), hash.to_string()));
        }
    }

    /// Check `content` against what the response installs.
    pub fn check(&self, content: &ReleaseContent) -> Vec<String> {
        let mut problems = self.problems.clone();
        if self.kind.as_deref() != Some(content.kind.as_str()) {
            problems.push(format!(
                "the signed release content is a {} release, but the response installs it as {}",
                content.kind,
                self.kind.as_deref().unwrap_or("an unknown kind")
            ));
        }
        let attested = content.attested();
        let mut seen = BTreeSet::new();
        for (role, name, hash) in &self.artifacts {
            seen.insert((role.as_str(), name.as_str()));
            match attested.get(&(role.as_str(), name.as_str())) {
                Some(signed) if signed == hash => {}
                Some(signed) => problems.push(format!(
                    "{role} {name} is {hash} in the response, but the signed release content names {signed}"
                )),
                None => problems.push(format!(
                    "{role} {name} ({hash}) is not part of the signed release content"
                )),
            }
        }
        for (role, name) in attested.keys() {
            if REQUIRED_ROLES.contains(role) && !seen.contains(&(*role, *name)) {
                problems.push(format!(
                    "the response omits {role} {name}, which the signed release content names"
                ));
            }
        }
        problems
    }
}

fn text<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}

fn items<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// Re-hash one served SDK extension bundle against its claimed hash.
fn check_extension_bytes(artifact: &Value, claimed: &str, label: &str, out: &mut Observed) {
    match serde_json::from_value::<crate::api_client::RegistrySdkExtensionArtifact>(
        artifact.clone(),
    ) {
        Ok(artifact) => {
            let actual = crate::commands::sdk::registry_extension_content_hash(&artifact);
            if actual != claimed {
                out.problems.push(format!(
                    "the {label} SDK extension's files hash to {actual}, not its content hash {claimed}"
                ));
            }
        }
        Err(error) => out
            .problems
            .push(format!("the {label} SDK extension is malformed: {error}")),
    }
}

/// Whether `holder` carries any SDK extension.
fn has_extensions(holder: &Value) -> bool {
    items(holder, "sdkExtensions").next().is_some()
        || holder
            .pointer("/definition/extensions")
            .is_some_and(|extensions| !extensions.is_null())
}

/// The SDK extensions `holder` carries: its per-target `sdkExtensions` and
/// a program's legacy single `definition.extensions`.
fn observe_extensions(holder: &Value, out: &mut Observed) {
    for extension in items(holder, "sdkExtensions") {
        let target = text(extension, "/target").unwrap_or("unknown");
        let Some(hash) = text(extension, "/contentHash") else {
            continue;
        };
        out.push("sdk-extension", target, Some(hash));
        match extension.get("artifact") {
            Some(artifact) => check_extension_bytes(artifact, hash, target, out),
            None => out
                .problems
                .push(format!("the {target} SDK extension has no artifact")),
        }
    }
    if let Some(artifact) = holder
        .pointer("/definition/extensions")
        .filter(|extensions| !extensions.is_null())
    {
        let target = text(artifact, "/manifest/language").unwrap_or("typescript");
        let Some(hash) = text(artifact, "/artifactHash") else {
            out.problems
                .push(format!("the {target} SDK extension has no artifact hash"));
            return;
        };
        out.push("sdk-extension", target, Some(hash));
        check_extension_bytes(artifact, hash, target, out);
    }
}

/// A program descriptor's exact program identities, under `name`.
fn observe_program(program: &Value, name: &str, out: &mut Observed) {
    out.push(
        "program-release",
        name,
        text(program, "/release/programReleaseHash"),
    );
    out.push(
        "program-spec",
        name,
        text(program, "/definition/programSpecHash"),
    );
}

/// A stack or protocol member program: its identities under its program id
/// and the program package it embeds. Its extensions belong to that
/// package's release (its `programPackage` entry checks them), so one
/// without a package cannot carry any.
fn observe_member_program(program: &Value, out: &mut Observed) {
    let program_id = text(program, "/definition/programId").unwrap_or("unknown");
    observe_program(program, program_id, out);
    let package = text(program, "/programPackage/packageReleaseHash");
    out.push("program-package", program_id, package);
    if package.is_none() && has_extensions(program) {
        out.problems.push(format!(
            "program {program_id} carries an SDK extension but names no program package release to attest it"
        ));
    }
}

/// What a resolved dependency (or protocol install descriptor) installs
/// for its own release.
pub fn observe_dependency(dependency: &Value) -> Observed {
    let mut out = Observed {
        kind: text(dependency, "/kind").map(str::to_owned),
        ..Observed::default()
    };
    match out.kind.as_deref() {
        Some("stack") => {
            out.push(
                "stack-manifest",
                NO_NAME,
                text(dependency, "/stackManifestHash"),
            );
            for program in items(dependency, "programs") {
                observe_member_program(program, &mut out);
            }
            observe_extensions(dependency, &mut out);
        }
        Some("program") => {
            if let Some(install) = dependency.get("install") {
                observe_program(install, NO_NAME, &mut out);
                observe_extensions(install, &mut out);
            }
            observe_extensions(dependency, &mut out);
        }
        Some("protocol") => {
            out.push(
                "protocol-release",
                NO_NAME,
                text(dependency, "/protocol/protocolReleaseHash"),
            );
            out.push(
                "protocol-spec",
                NO_NAME,
                text(dependency, "/protocol/protocolSpecHash"),
            );
            out.push(
                "deployment",
                NO_NAME,
                text(dependency, "/protocol/deployment/deploymentId"),
            );
            for stack in items(dependency, "stacks") {
                let name = text(stack, "/alias").unwrap_or("unknown");
                out.push("stack", name, text(stack, "/packageReleaseHash"));
                out.push("stack-manifest", name, text(stack, "/stackManifestHash"));
            }
            for program in items(dependency, "programs") {
                observe_member_program(program, &mut out);
            }
            for program in items(dependency, "unpinnedPrograms") {
                out.push(
                    "program-spec",
                    text(program, "/programId").unwrap_or("unknown"),
                    text(program, "/programSpecHash"),
                );
            }
        }
        _ => {}
    }
    out
}

/// What a program descriptor installs for the program package release its
/// `programPackage` names: the program's identities and its SDK
/// extensions.
pub fn observe_program_package(program: &Value) -> Observed {
    let mut out = Observed {
        kind: Some("program".into()),
        ..Observed::default()
    };
    observe_program(program, NO_NAME, &mut out);
    observe_extensions(program, &mut out);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    const VECTORS: &str = include_str!("../../../test-vectors/registry-release-content-v1.json");

    fn content(value: Value) -> ReleaseContent {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn shared_vectors_hash_and_sign_as_published() {
        use base64::Engine;
        use ed25519_dalek::{Signer, SigningKey};
        let vectors: Value = serde_json::from_str(VECTORS).unwrap();
        assert_eq!(
            vectors["schema"],
            "arete.registry-release-content-vectors/v1"
        );
        let seed_hex = vectors["signingKey"]["seedHex"].as_str().unwrap();
        let seed: [u8; 32] = (0..32)
            .map(|index| u8::from_str_radix(&seed_hex[index * 2..index * 2 + 2], 16).unwrap())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let key = SigningKey::from_bytes(&seed);
        let key_id = vectors["signingKey"]["keyId"].as_str().unwrap();
        assert_eq!(
            vectors["signingKey"]["publicKey"],
            bs58::encode(key.verifying_key().to_bytes()).into_string()
        );
        let cases = vectors["cases"].as_array().unwrap();
        assert!(cases.len() >= 8);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let parsed = serde_json::from_value::<ReleaseContent>(case["content"].clone())
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            if let Some(code) = case.get("error") {
                let error = parsed.digest().expect_err(name);
                assert_eq!(error.code(), code.as_str().unwrap(), "{name}");
                continue;
            }
            let expected = &case["expected"];
            assert_eq!(
                parsed.canonical_text().unwrap(),
                expected["canonicalText"].as_str().unwrap(),
                "{name}"
            );
            let digest = parsed.digest().unwrap();
            assert_eq!(digest, expected["digest"].as_str().unwrap(), "{name}");
            let message = signed_message(key_id, &digest);
            assert_eq!(
                message,
                expected["signedMessage"].as_str().unwrap(),
                "{name}"
            );
            assert_eq!(
                base64::engine::general_purpose::STANDARD
                    .encode(key.sign(message.as_bytes()).to_bytes()),
                expected["signature"].as_str().unwrap(),
                "{name}"
            );
        }
    }

    fn h(kind: &str, byte: char) -> String {
        format!("arete:h1:{kind}:sha256:{}", byte.to_string().repeat(64))
    }

    fn release(byte: char) -> String {
        format!(
            "arete:registry-package-release:v2:sha256:{}",
            byte.to_string().repeat(64)
        )
    }

    /// The descriptors behind `test-vectors/registry-release-content-v1.json`.
    fn vector_cases() -> Vec<(&'static str, Value, Option<&'static str>)> {
        let program_id = "11111111111111111111111111111111";
        let stack = json!({
            "schema": RELEASE_CONTENT_SCHEMA,
            "kind": "stack",
            "owner": "public",
            "package": "ore",
            "version": "1.1.0",
            "packageReleaseHash": release('a'),
            "artifacts": [
                { "role": "stack-manifest", "hash": h("stack-manifest", '1') },
                { "role": "program-spec", "name": program_id, "hash": h("program-spec", '2') },
                { "role": "program-release", "name": program_id, "hash": h("program-release", '3') },
                { "role": "program-package", "name": program_id, "hash": release('b') },
                { "role": "sdk-extension", "name": "typescript", "hash": "4".repeat(64) },
                { "role": "sdk-extension", "name": "rust", "hash": "5".repeat(64) },
            ],
        });
        let mut unsorted = stack.clone();
        unsorted["artifacts"].as_array_mut().unwrap().reverse();
        let program = json!({
            "schema": RELEASE_CONTENT_SCHEMA,
            "kind": "program",
            "owner": "private",
            "package": "ore-program",
            "version": "1.0.0",
            "packageReleaseHash": release('b'),
            "artifacts": [
                { "role": "program-release", "hash": h("program-release", '3') },
                { "role": "program-spec", "hash": h("program-spec", '2') },
                { "role": "sdk-extension", "name": "python", "hash": "6".repeat(64) },
            ],
        });
        let protocol = json!({
            "schema": RELEASE_CONTENT_SCHEMA,
            "kind": "protocol",
            "owner": "public",
            "package": "tip-jar",
            "version": "2.0.0-rc.1",
            "packageReleaseHash": release('c'),
            "artifacts": [
                { "role": "protocol-release", "hash": h("protocol-release", '7') },
                { "role": "protocol-spec", "hash": h("protocol-spec", '8') },
                { "role": "deployment", "hash": "0f8e2d6c-3b1a-4e5f-9a7b-1c2d3e4f5a6b" },
                { "role": "stack", "name": "tips", "hash": release('a') },
                { "role": "stack-manifest", "name": "tips", "hash": h("stack-manifest", '1') },
                { "role": "program-spec", "name": program_id, "hash": h("program-spec", '2') },
                { "role": "program-release", "name": program_id, "hash": h("program-release", '3') },
                { "role": "program-package", "name": program_id, "hash": release('b') },
                { "role": "program-spec", "name": "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo", "hash": h("program-spec", '9') },
            ],
        });
        let legacy = json!({
            "schema": RELEASE_CONTENT_SCHEMA,
            "kind": "program",
            "owner": "public",
            "package": "legacy",
            "version": "0.1.0",
            "packageReleaseHash": format!("arete:registry-package-release:v1:sha256:{}", "d".repeat(64)),
            "artifacts": [],
        });
        let with = |base: &Value, pointer: &str, value: Value| {
            let mut changed = base.clone();
            let mut target = &mut changed;
            for segment in pointer.trim_start_matches('/').split('/') {
                target = match segment.parse::<usize>() {
                    Ok(index) => &mut target[index],
                    Err(_) => &mut target[segment],
                };
            }
            *target = value;
            changed
        };
        let mut duplicate = program.clone();
        duplicate["artifacts"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "role": "program-spec", "hash": h("program-spec", 'e') }));
        vec![
            ("stack", stack.clone(), None),
            ("stack-artifacts-in-any-order", unsorted, None),
            ("program-owner-private", program.clone(), None),
            ("protocol", protocol, None),
            ("v1-release-identity-without-artifacts", legacy, None),
            (
                "error-schema",
                with(
                    &program,
                    "/schema",
                    json!("arete.registry-release-content/v2"),
                ),
                Some("schema"),
            ),
            (
                "error-kind",
                with(&program, "/kind", json!("bundle")),
                Some("kind"),
            ),
            (
                "error-owner",
                with(&program, "/owner", json!("user:17")),
                Some("owner"),
            ),
            (
                "error-space-in-version",
                with(&program, "/version", json!("1.0.0 beta")),
                Some("field"),
            ),
            (
                "error-newline-in-hash",
                with(&program, "/artifacts/0/hash", json!("a\nb")),
                Some("field"),
            ),
            (
                "error-role",
                with(&program, "/artifacts/0/role", json!("program release")),
                Some("role"),
            ),
            (
                "error-explicit-dash-name",
                with(&program, "/artifacts/0/name", json!("-")),
                Some("name"),
            ),
            ("error-duplicate-artifact", duplicate, Some("duplicate")),
        ]
    }

    /// Rewrites the shared vectors from [`vector_cases`]:
    /// `UPDATE_RELEASE_CONTENT_VECTORS=1 cargo test -p a4-cli release_content_vectors`.
    #[test]
    fn release_content_vectors_are_current() {
        use base64::Engine;
        use ed25519_dalek::{Signer, SigningKey};
        // RFC 8032 test 1.
        let seed_hex = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
        let seed: [u8; 32] = (0..32)
            .map(|index| u8::from_str_radix(&seed_hex[index * 2..index * 2 + 2], 16).unwrap())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let key = SigningKey::from_bytes(&seed);
        let key_id = "registry-test-1";
        let cases = vector_cases()
            .into_iter()
            .map(|(name, content, error)| {
                let mut case = json!({ "name": name, "content": content });
                match error {
                    Some(code) => case["error"] = json!(code),
                    None => {
                        let parsed: ReleaseContent = serde_json::from_value(content).unwrap();
                        let digest = parsed.digest().unwrap();
                        let message = signed_message(key_id, &digest);
                        case["expected"] = json!({
                            "canonicalText": parsed.canonical_text().unwrap(),
                            "digest": digest,
                            "signedMessage": message,
                            "signature": base64::engine::general_purpose::STANDARD
                                .encode(key.sign(message.as_bytes()).to_bytes()),
                        });
                    }
                }
                case
            })
            .collect::<Vec<_>>();
        let document = json!({
            "schema": "arete.registry-release-content-vectors/v1",
            "description": "Registry release content descriptors (arete.registry-release-content/v1). The canonical text is the schema line, then `kind`, `owner`, `package`, `version` and `release` lines, then one `artifact <role> <name> <hash>` line per artifact sorted bytewise by (role, name), `-` standing for an absent name; every line ends with \\n. The digest is arete:registry-release-content:v1:sha256:<lowercase hex SHA-256 of the canonical text>. Each registry key signs the UTF-8 message `arete.registry-package-release/v2\\n<keyId>\\n<digest>` with Ed25519 (64 bytes, padded standard base64). kind is program, protocol or stack; owner is public or private; package, version, packageReleaseHash and hash are printable ASCII without spaces (1-256 bytes); role and name match [A-Za-z0-9_.-]{1,128} and name is omitted rather than '-'. Error cases name the rule they break: schema, kind, owner, field, role, name or duplicate (a repeated role and name).",
            "signingKey": {
                "keyId": key_id,
                "seedHex": seed_hex,
                "publicKey": bs58::encode(key.verifying_key().to_bytes()).into_string(),
            },
            "cases": cases,
        });
        let text = format!("{}\n", serde_json::to_string_pretty(&document).unwrap());
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../test-vectors/registry-release-content-v1.json"
        );
        if std::env::var_os("UPDATE_RELEASE_CONTENT_VECTORS").is_some() {
            std::fs::write(path, &text).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(path).unwrap_or_default(),
            text,
            "the shared vectors are stale: rerun with UPDATE_RELEASE_CONTENT_VECTORS=1"
        );
    }

    #[test]
    fn artifact_order_does_not_change_the_digest_but_every_field_does() {
        let base = json!({
            "schema": RELEASE_CONTENT_SCHEMA,
            "kind": "program",
            "owner": "public",
            "package": "ore-program",
            "version": "1.0.0",
            "packageReleaseHash": "arete:registry-package-release:v2:sha256:p",
            "artifacts": [
                { "role": "program-spec", "hash": "spec" },
                { "role": "program-release", "hash": "release" },
            ],
        });
        let digest = content(base.clone()).digest().unwrap();
        let mut reordered = base.clone();
        reordered["artifacts"] = json!([
            { "role": "program-release", "hash": "release" },
            { "role": "program-spec", "hash": "spec" },
        ]);
        assert_eq!(content(reordered).digest().unwrap(), digest);
        for (field, value) in [
            ("kind", json!("stack")),
            ("owner", json!("private")),
            ("package", json!("other")),
            ("version", json!("1.0.1")),
            ("packageReleaseHash", json!("other")),
        ] {
            let mut changed = base.clone();
            changed[field] = value;
            assert_ne!(content(changed).digest().unwrap(), digest, "{field}");
        }
        for (field, value) in [("hash", "other"), ("name", "x"), ("role", "other")] {
            let mut changed = base.clone();
            changed["artifacts"][0][field] = json!(value);
            assert_ne!(content(changed).digest().unwrap(), digest, "{field}");
        }
    }

    #[test]
    fn swapped_or_unsigned_artifacts_are_reported() {
        let signed = content(json!({
            "schema": RELEASE_CONTENT_SCHEMA,
            "kind": "stack",
            "owner": "public",
            "package": "ore",
            "version": "1.0.0",
            "packageReleaseHash": "r",
            "artifacts": [
                { "role": "stack-manifest", "hash": "m" },
                { "role": "program-spec", "name": "P1", "hash": "s" },
            ],
        }));
        let mut observed = Observed {
            kind: Some("stack".into()),
            ..Observed::default()
        };
        observed.push("stack-manifest", NO_NAME, Some("m"));
        observed.push("program-spec", "P1", Some("s"));
        assert!(observed.check(&signed).is_empty());

        let mut swapped = observed.clone();
        swapped.artifacts[0].2 = "m2".into();
        assert!(swapped.check(&signed)[0].contains("names m"));

        let mut extra = observed.clone();
        extra.push("sdk-extension", "typescript", Some("x"));
        assert!(extra.check(&signed)[0].contains("not part of the signed"));

        let mut missing = observed.clone();
        missing.artifacts.remove(0);
        assert!(missing.check(&signed)[0].contains("omits stack-manifest -"));

        // A named member artifact cannot disappear either.
        let mut member = observed.clone();
        member
            .artifacts
            .retain(|(role, _, _)| role != "program-spec");
        assert!(member.check(&signed)[0].contains("omits program-spec P1"));

        let mut kind = observed;
        kind.kind = Some("program".into());
        assert!(kind.check(&signed)[0].contains("installs it as program"));
    }
}
