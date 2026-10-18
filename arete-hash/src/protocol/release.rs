//! Release-side contracts: the generated stack source, the release, its
//! review signature, the build receipt and published resources.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::error::{ProtocolContractError, ProtocolResult};
use super::shape::*;
use crate::{
    hash_jcs, Compiler, HashError, HashId, ProgramRelease, ProgramSpec, ProtocolHandlerBundle,
    ProtocolRelease, ProtocolReleaseReview, ProtocolResource, ProtocolSpec, ProtocolStackSource,
    SdkExtension, StackManifest,
};

pub const PROTOCOL_STACK_SOURCE_SCHEMA_V1: &str = "arete.protocol-stack-source/v1";
pub const PROTOCOL_RELEASE_SCHEMA_V1: &str = "arete.protocol-release/v1";
pub const PROTOCOL_RELEASE_REVIEW_SCHEMA_V1: &str = "arete.protocol-release-review/v1";
pub const PROTOCOL_BUILD_RECEIPT_SCHEMA_V1: &str = "arete.protocol-build-receipt/v1";
pub const PROTOCOL_RESOURCE_SCHEMA_V1: &str = "arete.protocol-resource/v1";
pub const STACK_SOURCE_SCHEMA_V1: &str = "arete.stack-source/v1";

const MAX_DEPENDENCIES: usize = 64;

fn model<T: serde::de::DeserializeOwned>(value: &Value) -> ProtocolResult<T> {
    serde_json::from_value(value.clone()).map_err(|error| {
        ProtocolContractError::new("invalid-value", "", format!("model mismatch: {error}"))
    })
}

// ---------------------------------------------------------------------------
// Stack source
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolStackSourceV1 {
    pub schema: String,
    pub generator: GeneratorV1,
    pub profile: StackProfile,
    /// The generated `arete.stack-source/v1` document, which a stack
    /// compiler takes unchanged.
    pub stack_source: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeneratorV1 {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StackProfile {
    /// Reads only the underlying programs: no memo decoding or `via` labels.
    Interim,
    /// Memo-decoding, attestation-gated, scoped rows.
    Full,
}

impl ProtocolStackSourceV1 {
    pub fn hash(&self) -> Result<HashId<ProtocolStackSource>, HashError> {
        hash_jcs::<ProtocolStackSource, _>(self)
    }
}

pub fn validate_protocol_stack_source_value(
    value: &Value,
) -> ProtocolResult<ProtocolStackSourceV1> {
    reject_floats(value, "")?;
    let root = Obj::new(
        value,
        "",
        &["schema", "generator", "profile", "stackSource"],
    )?;
    schema_tag(
        root.req("schema")?,
        "/schema",
        PROTOCOL_STACK_SOURCE_SCHEMA_V1,
    )?;
    shape_generator(root.req("generator")?, "/generator")?;
    one_of(root.req("profile")?, "/profile", &["interim", "full"])?;
    let source = root.req("stackSource")?;
    let source_obj = source
        .as_object()
        .ok_or_else(|| err("invalid-type", "/stackSource", "expected an object"))?;
    match source_obj.get("schema") {
        Some(tag) => schema_tag(tag, "/stackSource/schema", STACK_SOURCE_SCHEMA_V1)?,
        None => {
            return Err(err(
                "missing-field",
                "/stackSource/schema",
                "missing field 'schema'",
            ))
        }
    }
    model(value)
}

fn shape_generator(value: &Value, path: &str) -> ProtocolResult<()> {
    let generator = Obj::new(value, path, &["name", "version"])?;
    let name = bounded_str(generator.req("name")?, &generator.path_of("name"), 128)?;
    if name.is_empty() {
        return Err(err(
            "invalid-value",
            &generator.path_of("name"),
            "name must not be empty",
        ));
    }
    semver(generator.req("version")?, &generator.path_of("version"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Release
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolReleaseV1 {
    pub schema: String,
    pub protocol: String,
    pub version: String,
    pub spec_hash: HashId<ProtocolSpec>,
    /// `null` for template-only protocols.
    pub handler_bundle_hash: Option<HashId<ProtocolHandlerBundle>>,
    pub stack_source_hash: HashId<ProtocolStackSource>,
    pub compiler_hash: HashId<Compiler>,
    pub dependencies: ReleaseDependenciesV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseDependenciesV1 {
    pub programs: Vec<ProgramDependencyV1>,
    pub stacks: Vec<StackDependencyV1>,
    pub sdk_extensions: Vec<SdkExtensionDependencyV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramDependencyV1 {
    pub alias: String,
    pub program_id: String,
    pub program_spec_hash: HashId<ProgramSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_release_hash: Option<HashId<ProgramRelease>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackDependencyV1 {
    pub alias: String,
    pub stack_manifest_hash: HashId<StackManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SdkExtensionDependencyV1 {
    pub alias: String,
    pub sdk_extension_hash: HashId<SdkExtension>,
}

impl ProtocolReleaseV1 {
    pub fn hash(&self) -> Result<HashId<ProtocolRelease>, HashError> {
        hash_jcs::<ProtocolRelease, _>(self)
    }
}

pub fn validate_protocol_release_value(value: &Value) -> ProtocolResult<ProtocolReleaseV1> {
    reject_floats(value, "")?;
    let root = Obj::new(
        value,
        "",
        &[
            "schema",
            "protocol",
            "version",
            "specHash",
            "handlerBundleHash",
            "stackSourceHash",
            "compilerHash",
            "dependencies",
        ],
    )?;
    schema_tag(root.req("schema")?, "/schema", PROTOCOL_RELEASE_SCHEMA_V1)?;
    protocol_name(as_str(root.req("protocol")?, "/protocol")?, "/protocol")?;
    semver(root.req("version")?, "/version")?;
    hash_id::<ProtocolSpec>(root.req("specHash")?, "/specHash")?;
    if let Some(value) = root.nullable("handlerBundleHash")? {
        hash_id::<ProtocolHandlerBundle>(value, "/handlerBundleHash")?;
    }
    hash_id::<ProtocolStackSource>(root.req("stackSourceHash")?, "/stackSourceHash")?;
    hash_id::<Compiler>(root.req("compilerHash")?, "/compilerHash")?;
    let deps = Obj::new(
        root.req("dependencies")?,
        "/dependencies",
        &["programs", "stacks", "sdkExtensions"],
    )?;
    sorted_entries(
        deps.req("programs")?,
        "/dependencies/programs",
        |entry, path| {
            let entry = Obj::new(
                entry,
                path,
                &[
                    "alias",
                    "programId",
                    "programSpecHash",
                    "programReleaseHash",
                ],
            )?;
            let alias = identifier_value(entry.req("alias")?, &entry.path_of("alias"))?;
            pubkey(entry.req("programId")?, &entry.path_of("programId"))?;
            hash_id::<ProgramSpec>(
                entry.req("programSpecHash")?,
                &entry.path_of("programSpecHash"),
            )?;
            if let Some(value) = entry.opt("programReleaseHash")? {
                hash_id::<ProgramRelease>(value, &entry.path_of("programReleaseHash"))?;
            }
            Ok(alias.to_string())
        },
    )?;
    sorted_entries(
        deps.req("stacks")?,
        "/dependencies/stacks",
        |entry, path| {
            let entry = Obj::new(entry, path, &["alias", "stackManifestHash"])?;
            let alias = identifier_value(entry.req("alias")?, &entry.path_of("alias"))?;
            hash_id::<StackManifest>(
                entry.req("stackManifestHash")?,
                &entry.path_of("stackManifestHash"),
            )?;
            Ok(alias.to_string())
        },
    )?;
    sorted_entries(
        deps.req("sdkExtensions")?,
        "/dependencies/sdkExtensions",
        |entry, path| {
            let entry = Obj::new(entry, path, &["alias", "sdkExtensionHash"])?;
            let alias = identifier_value(entry.req("alias")?, &entry.path_of("alias"))?;
            hash_id::<SdkExtension>(
                entry.req("sdkExtensionHash")?,
                &entry.path_of("sdkExtensionHash"),
            )?;
            Ok(alias.to_string())
        },
    )?;
    model(value)
}

/// Validate an array whose entries carry a key that must be strictly
/// increasing (sorted and unique).
pub(crate) fn sorted_entries(
    value: &Value,
    path: &str,
    entry: impl Fn(&Value, &str) -> ProtocolResult<String>,
) -> ProtocolResult<()> {
    let values = as_array(value, path, MAX_DEPENDENCIES)?;
    let mut previous: Option<String> = None;
    for (i, item) in values.iter().enumerate() {
        let item_path = index(path, i);
        let key = entry(item, &item_path)?;
        if let Some(previous) = &previous {
            if key == *previous {
                return Err(err(
                    "duplicate-entry",
                    &item_path,
                    format!("'{key}' is listed twice"),
                ));
            }
            if key < *previous {
                return Err(err(
                    "unsorted-entries",
                    &item_path,
                    "entries must be sorted",
                ));
            }
        }
        previous = Some(key);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Review
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolReleaseReviewV1 {
    pub schema: String,
    pub release_hash: HashId<ProtocolRelease>,
    pub review_key_id: String,
    pub trust_tier: TrustTier,
    /// Base64 (standard, padded) Ed25519 signature over
    /// [`review_signing_message`].
    pub signature: String,
}

/// The trust tier a deployment runs at.
///
/// - `reviewed`: a reviewer signed the release (handlers may run).
/// - `unreviewed`: no review. Only template-only releases on private
///   deployments may run at this tier, so no author code ever runs
///   unreviewed.
///
/// A review document always attests `reviewed`: `unreviewed` is the absence
/// of a review, so [`validate_protocol_release_review_value`] rejects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrustTier {
    Reviewed,
    Unreviewed,
}

impl TrustTier {
    pub const ALL: [&'static str; 2] = ["reviewed", "unreviewed"];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reviewed => "reviewed",
            Self::Unreviewed => "unreviewed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "reviewed" => Some(Self::Reviewed),
            "unreviewed" => Some(Self::Unreviewed),
            _ => None,
        }
    }
}

impl ProtocolReleaseReviewV1 {
    pub fn hash(&self) -> Result<HashId<ProtocolReleaseReview>, HashError> {
        hash_jcs::<ProtocolReleaseReview, _>(self)
    }

    pub fn signing_message(&self) -> Vec<u8> {
        review_signing_message(&self.release_hash, &self.review_key_id, self.trust_tier)
    }

    /// The 64 raw signature bytes. Validation already checked the encoding.
    pub fn signature_bytes(&self) -> [u8; 64] {
        let mut out = [0u8; 64];
        if let Some(bytes) = decode_base64(&self.signature) {
            if bytes.len() == 64 {
                out.copy_from_slice(&bytes);
            }
        }
        out
    }
}

/// The exact bytes a reviewer signs:
/// `arete.protocol-release-review/v1\n<keyId>\n<trustTier>\n<releaseHash>`.
/// Hosts and the API verify against these bytes with the same public keys.
pub fn review_signing_message(
    release_hash: &HashId<ProtocolRelease>,
    review_key_id: &str,
    trust_tier: TrustTier,
) -> Vec<u8> {
    format!(
        "{PROTOCOL_RELEASE_REVIEW_SCHEMA_V1}\n{review_key_id}\n{}\n{release_hash}",
        trust_tier.as_str()
    )
    .into_bytes()
}

pub fn validate_protocol_release_review_value(
    value: &Value,
) -> ProtocolResult<ProtocolReleaseReviewV1> {
    reject_floats(value, "")?;
    let root = Obj::new(
        value,
        "",
        &[
            "schema",
            "releaseHash",
            "reviewKeyId",
            "trustTier",
            "signature",
        ],
    )?;
    schema_tag(
        root.req("schema")?,
        "/schema",
        PROTOCOL_RELEASE_REVIEW_SCHEMA_V1,
    )?;
    hash_id::<ProtocolRelease>(root.req("releaseHash")?, "/releaseHash")?;
    key_id(root.req("reviewKeyId")?, "/reviewKeyId")?;
    // A review attests the reviewed tier; `unreviewed` is the absence of one.
    one_of(root.req("trustTier")?, "/trustTier", &["reviewed"])?;
    let signature = as_str(root.req("signature")?, "/signature")?;
    match decode_base64(signature) {
        Some(bytes) if bytes.len() == 64 => {}
        _ => {
            return Err(err(
                "invalid-signature",
                "/signature",
                "expected a base64 64-byte Ed25519 signature",
            ))
        }
    }
    model(value)
}

/// `^[a-z0-9][a-z0-9-]{0,63}$`.
pub(crate) fn key_id<'a>(value: &'a Value, path: &str) -> ProtocolResult<&'a str> {
    let text = as_str(value, path)?;
    let bytes = text.as_bytes();
    let valid = !bytes.is_empty()
        && bytes.len() <= 64
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-');
    if valid {
        Ok(text)
    } else {
        Err(err(
            "invalid-identifier",
            path,
            "key ids match ^[a-z0-9][a-z0-9-]{0,63}$",
        ))
    }
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Strict standard base64 with padding. Returns `None` for any
/// non-canonical encoding.
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (chunk_index, chunk) in bytes.chunks(4).enumerate() {
        let last = chunk_index == bytes.len() / 4 - 1;
        let mut values = [0u8; 4];
        let mut padding = 0;
        for (i, byte) in chunk.iter().enumerate() {
            if *byte == b'=' {
                if !last || i < 2 {
                    return None;
                }
                padding += 1;
                values[i] = 0;
            } else {
                if padding > 0 {
                    return None;
                }
                values[i] = BASE64_ALPHABET.iter().position(|c| c == byte)? as u8;
            }
        }
        let n = (u32::from(values[0]) << 18)
            | (u32::from(values[1]) << 12)
            | (u32::from(values[2]) << 6)
            | u32::from(values[3]);
        out.push((n >> 16) as u8);
        if padding < 2 {
            out.push((n >> 8) as u8);
        }
        if padding < 1 {
            out.push(n as u8);
        }
        // Reject non-zero bits hidden in the padding.
        if (padding == 1 && values[2] & 0b11 != 0) || (padding == 2 && values[1] & 0b1111 != 0) {
            return None;
        }
    }
    Some(out)
}

pub fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(BASE64_ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(BASE64_ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            BASE64_ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64_ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Build receipt
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolBuildReceiptV1 {
    pub schema: String,
    pub source_tree_sha256: String,
    pub lockfile_sha256: String,
    pub dependencies: Vec<BuildDependencyV1>,
    pub compiler: BuildCompilerV1,
    pub outputs: BuildOutputsV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BuildDependencyV1 {
    pub name: String,
    pub version: String,
    pub integrity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BuildCompilerV1 {
    pub name: String,
    pub version: String,
    pub esbuild: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BuildOutputsV1 {
    pub spec_hash: HashId<ProtocolSpec>,
    pub handler_bundle_hash: Option<HashId<ProtocolHandlerBundle>>,
    pub stack_source_hash: HashId<ProtocolStackSource>,
    pub release_hash: HashId<ProtocolRelease>,
}

pub fn validate_protocol_build_receipt_value(
    value: &Value,
) -> ProtocolResult<ProtocolBuildReceiptV1> {
    reject_floats(value, "")?;
    let root = Obj::new(
        value,
        "",
        &[
            "schema",
            "sourceTreeSha256",
            "lockfileSha256",
            "dependencies",
            "compiler",
            "outputs",
        ],
    )?;
    schema_tag(
        root.req("schema")?,
        "/schema",
        PROTOCOL_BUILD_RECEIPT_SCHEMA_V1,
    )?;
    sha256_hex(root.req("sourceTreeSha256")?, "/sourceTreeSha256")?;
    sha256_hex(root.req("lockfileSha256")?, "/lockfileSha256")?;
    sorted_entries(root.req("dependencies")?, "/dependencies", |entry, path| {
        let entry = Obj::new(entry, path, &["name", "version", "integrity"])?;
        let name = bounded_str(entry.req("name")?, &entry.path_of("name"), 214)?;
        bounded_str(entry.req("version")?, &entry.path_of("version"), 64)?;
        bounded_str(entry.req("integrity")?, &entry.path_of("integrity"), 256)?;
        Ok(name.to_string())
    })?;
    let compiler = Obj::new(
        root.req("compiler")?,
        "/compiler",
        &["name", "version", "esbuild"],
    )?;
    bounded_str(compiler.req("name")?, "/compiler/name", 128)?;
    semver(compiler.req("version")?, "/compiler/version")?;
    semver(compiler.req("esbuild")?, "/compiler/esbuild")?;
    let outputs = Obj::new(
        root.req("outputs")?,
        "/outputs",
        &[
            "specHash",
            "handlerBundleHash",
            "stackSourceHash",
            "releaseHash",
        ],
    )?;
    hash_id::<ProtocolSpec>(outputs.req("specHash")?, "/outputs/specHash")?;
    if let Some(value) = outputs.nullable("handlerBundleHash")? {
        hash_id::<ProtocolHandlerBundle>(value, "/outputs/handlerBundleHash")?;
    }
    hash_id::<ProtocolStackSource>(outputs.req("stackSourceHash")?, "/outputs/stackSourceHash")?;
    hash_id::<ProtocolRelease>(outputs.req("releaseHash")?, "/outputs/releaseHash")?;
    model(value)
}

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolResourceV1 {
    pub schema: String,
    pub name: String,
    /// Lowercase hex SHA-256 of the canonical JSON of the spec's resource
    /// field schema.
    pub schema_sha256: String,
    pub value: Value,
}

impl ProtocolResourceV1 {
    pub fn hash(&self) -> Result<HashId<ProtocolResource>, HashError> {
        hash_jcs::<ProtocolResource, _>(self)
    }
}

pub fn validate_protocol_resource_value(value: &Value) -> ProtocolResult<ProtocolResourceV1> {
    reject_floats(value, "")?;
    let root = Obj::new(value, "", &["schema", "name", "schemaSha256", "value"])?;
    schema_tag(root.req("schema")?, "/schema", PROTOCOL_RESOURCE_SCHEMA_V1)?;
    identifier_value(root.req("name")?, "/name")?;
    sha256_hex(root.req("schemaSha256")?, "/schemaSha256")?;
    root.req("value")?;
    model(value)
}
