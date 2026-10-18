//! Protocol packages: checking a protocol resolution against its spec, and
//! hoisting the stacks and programs it brings.
//!
//! A protocol resolution carries the `arete.protocol-spec/v1` document (or
//! its hash, read from `GET /api/registry/artifacts/protocol-spec/{hash}`),
//! the deployment its client calls, one stack resolution per stack the spec
//! declares and one exact program release per program the spec declares.
//! Its stacks are hoisted as they are, and its programs exactly as a stack
//! hoists its programs: one program SDK per spec program, served by the
//! first stack that bundles that release, or generated on its own when no
//! stack bundles it.
//!
//! An owner-private package may leave a program **unpinned** (it was
//! installed from a local IDL, so no program package exists): the resolution
//! lists it under `unpinnedPrograms` with its exact ProgramSpec, nothing is
//! generated for it, and the client's README says the app installs it.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};
use arete_hash::protocol::{ProtocolProgramV1, ProtocolSpecV1};

use super::resolver::{
    RegistryDependencyRequest, ResolvedProtocol, ResolvedRegistryDependency,
    ResolvedUnpinnedProgram,
};
use crate::api_client::{ApiClient, RegistryProgramInstallResponse};

/// Input properties `steps.<step>.prepare` takes beside a step's args. A
/// spec that uses one as an arg name cannot get a typed client.
pub(crate) const RESERVED_PREPARE_INPUTS: [&str; 2] = ["instance", "wallet"];

/// The stable code the core SDK raises for one program key with two
/// identities; the CLI raises it for a protocol's own programs.
pub(crate) const PROGRAM_KEY_CONFLICT: &str = "PROGRAM_KEY_CONFLICT";

/// Reads the spec from the artifact route when the resolution names only its
/// hash. A resolution that is not a protocol, or already carries its spec, is
/// left alone.
pub(crate) fn fetch_missing_protocol_spec(resolved: &mut ResolvedRegistryDependency) -> Result<()> {
    let ResolvedRegistryDependency::Protocol {
        package, protocol, ..
    } = resolved
    else {
        return Ok(());
    };
    if protocol.protocol_spec.is_some() {
        return Ok(());
    }
    let spec = ApiClient::new()?
        .get_registry_artifact("protocol-spec", &protocol.protocol_spec_hash)
        .with_context(|| {
            format!(
                "Failed to read the protocol spec {} of '{package}'",
                protocol.protocol_spec_hash
            )
        })?;
    protocol.protocol_spec = Some(spec);
    Ok(())
}

/// The resolution's spec, validated and checked against its pinned hash.
pub(crate) fn protocol_spec(protocol: &ResolvedProtocol) -> Result<ProtocolSpecV1> {
    let value = protocol.protocol_spec.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "Registry returned protocol spec {} without its document",
            protocol.protocol_spec_hash
        )
    })?;
    let spec = arete_hash::protocol::validate_protocol_spec_value(value)
        .map_err(|error| anyhow::anyhow!("Registry returned an invalid protocol spec: {error}"))?;
    let hash = spec
        .hash()
        .context("Failed to hash the resolved protocol spec")?
        .to_string();
    if hash != protocol.protocol_spec_hash {
        bail!(
            "Registry returned a protocol spec that hashes to {hash}, not its pinned {}",
            protocol.protocol_spec_hash
        );
    }
    Ok(spec)
}

/// Checks a protocol resolution: its release and spec identities, the
/// deployment it names, that its stacks are exactly the spec's stacks, and
/// that every program the spec declares resolves to one program SDK.
pub(crate) fn verify_resolved_protocol(resolved: &ResolvedRegistryDependency) -> Result<()> {
    let ResolvedRegistryDependency::Protocol {
        package,
        protocol,
        stacks,
        programs,
        unpinned_programs,
        ..
    } = resolved
    else {
        return Ok(());
    };
    expect_hash_kind(
        &protocol.protocol_release_hash,
        arete_hash::HashKindName::ProtocolRelease,
        "protocol release",
    )?;
    let spec = protocol_spec(protocol)?;
    let deployment = &protocol.deployment;
    if deployment.deployment_id.trim().is_empty()
        || !deployment
            .deployment_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("Registry returned protocol '{package}' with an invalid deployment id");
    }
    for (url, field) in [
        (&deployment.api_url, "apiUrl"),
        (&deployment.session_endpoint, "sessionEndpoint"),
    ] {
        if !is_secure_or_loopback_url(url) {
            bail!("Registry returned protocol '{package}' with an invalid deployment {field}");
        }
    }

    let mut resolved_stacks = BTreeMap::new();
    for stack in stacks {
        let ResolvedRegistryDependency::Stack {
            alias,
            stack_manifest_hash,
            ..
        } = stack
        else {
            bail!("Registry returned a non-stack entry among the stacks of protocol '{package}'");
        };
        let declared = spec.stacks.get(alias).ok_or_else(|| {
            anyhow::anyhow!(
                "Registry returned stack '{alias}' for protocol '{package}', whose spec declares no such stack"
            )
        })?;
        if declared.stack_manifest_hash.to_string() != *stack_manifest_hash {
            bail!(
                "Registry returned stack '{alias}' for protocol '{package}' at StackManifest {stack_manifest_hash}, but the spec pins {}",
                declared.stack_manifest_hash
            );
        }
        if resolved_stacks.insert(alias.as_str(), stack).is_some() {
            bail!("Registry returned stack '{alias}' twice for protocol '{package}'");
        }
    }
    if let Some(missing) = spec
        .stacks
        .keys()
        .find(|alias| !resolved_stacks.contains_key(alias.as_str()))
    {
        bail!("Registry returned protocol '{package}' without its stack '{missing}'");
    }
    if let Some(live) = &protocol.live_stack {
        if !resolved_stacks.contains_key(live.stack.as_str())
            || !live.view.contains('/')
            || live.key_account.trim().is_empty()
        {
            bail!(
                "Registry returned protocol '{package}' whose live stack '{}' is not one of its stacks",
                live.stack
            );
        }
    }

    for (step, declared) in &spec.steps {
        if !declared.authority.is_client_callable() {
            continue;
        }
        if let Some(reserved) = RESERVED_PREPARE_INPUTS
            .iter()
            .find(|name| declared.args.properties.contains_key(**name))
        {
            bail!(
                "Protocol '{package}' step '{step}' has an arg named '{reserved}', which the generated client reserves for prepare's input"
            );
        }
    }

    hoist_programs(package, &spec, stacks, programs, unpinned_programs)?;
    Ok(())
}

/// A protocol's locked stacks pin the stack releases its immutable protocol
/// release implies; resolving the same protocol release to other stack
/// releases means they moved underneath the lock.
pub(crate) fn verify_locked_protocol_stacks(
    request: &RegistryDependencyRequest,
    response: &ResolvedRegistryDependency,
) -> Result<()> {
    let ResolvedRegistryDependency::Protocol { stacks, .. } = response else {
        return Ok(());
    };
    for locked in &request.locked_stacks {
        let resolved = stacks
            .iter()
            .find(|stack| stack.alias() == locked.alias)
            .map(ResolvedRegistryDependency::package_release_hash);
        if resolved != Some(locked.package_release_hash.as_str()) {
            bail!(
                "arete.lock integrity failure for protocol '{}': its locked release pins stack '{}' to release {}, but the registry now returns {}. Nothing was changed; run `a4 update protocol {}` only if you intend to advance",
                request.alias,
                locked.alias,
                locked.package_release_hash,
                resolved.unwrap_or("no such stack"),
                request.alias
            );
        }
    }
    Ok(())
}

/// Where a hoisted program SDK comes from.
#[derive(Debug, Clone, Copy)]
pub(crate) enum HoistedProgramSource<'a> {
    /// A stack of the protocol bundles it: the protocol reuses that stack's
    /// program module.
    Stack {
        stack: &'a str,
        program: &'a RegistryProgramInstallResponse,
    },
    /// No stack bundles it: the protocol generates it under `programs/`.
    Direct(&'a RegistryProgramInstallResponse),
}

impl<'a> HoistedProgramSource<'a> {
    pub(crate) fn program(&self) -> &'a RegistryProgramInstallResponse {
        match self {
            Self::Stack { program, .. } | Self::Direct(program) => program,
        }
    }
}

/// One program the protocol hoists, under the spec's key for it.
#[derive(Debug, Clone)]
pub(crate) struct HoistedProgram<'a> {
    pub key: String,
    pub source: HoistedProgramSource<'a>,
}

/// The program SDK identity two copies of one program are compared by: the
/// program package release when there is one, else the program release.
fn program_sdk_identity(program: &RegistryProgramInstallResponse) -> String {
    program
        .program_package
        .as_ref()
        .map(|package| package.package_release_hash.clone())
        .unwrap_or_else(|| program.release.program_release_hash.clone())
}

fn same_program(
    program: &RegistryProgramInstallResponse,
    program_id: &str,
    spec_hash: &str,
) -> bool {
    program.definition.program_id == program_id && program.definition.program_spec_hash == spec_hash
}

/// [`hoist_programs`] for a resolution; a local development resolution
/// (`a4 dev sdk`) hoists nothing.
pub(crate) fn hoist_protocol_programs<'a>(
    package: &str,
    protocol: &ResolvedProtocol,
    spec: &ProtocolSpecV1,
    stacks: &'a [ResolvedRegistryDependency],
    programs: &'a [RegistryProgramInstallResponse],
    unpinned: &[ResolvedUnpinnedProgram],
) -> Result<Vec<HoistedProgram<'a>>> {
    if protocol.local_dev {
        return Ok(Vec::new());
    }
    hoist_programs(package, spec, stacks, programs, unpinned)
}

/// The programs a protocol client does not bundle, for its README: the
/// resolution's unpinned programs (none for a local development build).
pub(crate) fn unbundled_protocol_programs<'a>(
    protocol: &ResolvedProtocol,
    unpinned: &'a [ResolvedUnpinnedProgram],
) -> &'a [ResolvedUnpinnedProgram] {
    if protocol.local_dev {
        &[]
    } else {
        unpinned
    }
}

/// The protocol's programs, one per `programSpec` program the spec declares,
/// in key order. A program several stacks bundle is hoisted once, from the
/// first stack by key; one no stack bundles is generated on its own. Two
/// different program SDKs for one key fail with `PROGRAM_KEY_CONFLICT`.
///
/// A program the package leaves unpinned (`unpinned`) is skipped: it must be
/// the spec's exact program, and nothing else may resolve it.
pub(crate) fn hoist_programs<'a>(
    package: &str,
    spec: &ProtocolSpecV1,
    stacks: &'a [ResolvedRegistryDependency],
    programs: &'a [RegistryProgramInstallResponse],
    unpinned: &[ResolvedUnpinnedProgram],
) -> Result<Vec<HoistedProgram<'a>>> {
    let mut unpinned_keys = BTreeMap::new();
    for entry in unpinned {
        let Some(ProtocolProgramV1::ProgramSpec {
            program_id,
            program_spec_hash,
            ..
        }) = spec.programs.get(&entry.key)
        else {
            bail!(
                "Registry returned unpinned program '{}' for protocol '{package}', whose spec declares no such program",
                entry.key
            );
        };
        if *program_id != entry.program_id
            || program_spec_hash.to_string() != entry.program_spec_hash
        {
            bail!(
                "Registry returned unpinned program '{}' for protocol '{package}' as {} ({}), but the spec pins {program_id} ({program_spec_hash})",
                entry.key,
                entry.program_id,
                entry.program_spec_hash
            );
        }
        if unpinned_keys.insert(entry.key.as_str(), entry).is_some() {
            bail!(
                "Registry returned unpinned program '{}' twice for protocol '{package}'",
                entry.key
            );
        }
    }

    let mut stacks_by_alias = stacks
        .iter()
        .filter_map(|stack| match stack {
            ResolvedRegistryDependency::Stack {
                alias, programs, ..
            } => Some((alias.as_str(), programs)),
            _ => None,
        })
        .collect::<Vec<_>>();
    stacks_by_alias.sort_by_key(|(alias, _)| *alias);

    let mut used_direct = BTreeSet::new();
    let mut hoisted = Vec::new();
    for (key, declared) in &spec.programs {
        let ProtocolProgramV1::ProgramSpec {
            program_id,
            program_spec_hash,
            ..
        } = declared
        else {
            continue;
        };
        let spec_hash = program_spec_hash.to_string();
        let mut candidates = Vec::new();
        for (stack, stack_programs) in &stacks_by_alias {
            if let Some(program) = stack_programs
                .iter()
                .find(|program| same_program(program, program_id, &spec_hash))
            {
                candidates.push(HoistedProgramSource::Stack { stack, program });
            }
        }
        for (index, program) in programs.iter().enumerate() {
            if same_program(program, program_id, &spec_hash) {
                used_direct.insert(index);
                candidates.push(HoistedProgramSource::Direct(program));
            }
        }
        if unpinned_keys.contains_key(key.as_str()) {
            if !candidates.is_empty() {
                bail!(
                    "Registry returned program '{key}' of protocol '{package}' both resolved and unpinned"
                );
            }
            continue;
        }
        let Some(first) = candidates.first().copied() else {
            bail!(
                "Registry returned protocol '{package}' without an exact release of its program '{key}' ({program_id})"
            );
        };
        let identities = candidates
            .iter()
            .map(|candidate| program_sdk_identity(candidate.program()))
            .collect::<BTreeSet<_>>();
        if identities.len() > 1 {
            bail!(
                "{PROGRAM_KEY_CONFLICT}: protocol '{package}' program '{key}' resolves to different program SDKs ({}); its stacks must bundle one program SDK release",
                identities.into_iter().collect::<Vec<_>>().join(", ")
            );
        }
        hoisted.push(HoistedProgram {
            key: key.clone(),
            source: first,
        });
    }
    if let Some((_, extra)) = programs
        .iter()
        .enumerate()
        .find(|(index, _)| !used_direct.contains(index))
    {
        bail!(
            "Registry returned program {} for protocol '{package}', whose spec declares no such program",
            extra.definition.program_id
        );
    }
    Ok(hoisted)
}

fn expect_hash_kind(value: &str, expected: arete_hash::HashKindName, what: &str) -> Result<()> {
    let actual = value
        .parse::<arete_hash::AnyHashId>()
        .with_context(|| format!("Registry returned an invalid {what} identity '{value}'"))?
        .kind();
    if actual != expected {
        bail!("Registry returned a {actual} hash where a {what} identity belongs: '{value}'");
    }
    Ok(())
}

fn is_secure_or_loopback_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| match url.scheme() {
        "https" => url.host_str().is_some(),
        "http" => matches!(
            url.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        ),
        _ => false,
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    //! The token-raise fixture spec, re-pointed at test programs and stacks.

    use serde_json::{json, Value};

    /// `test-vectors/protocol/token-raise.protocol-spec.json` with its
    /// `meteoraPresale` and `squadsV4` programs set to `(programId,
    /// programSpecHash)` pairs and its stacks replaced by `(key,
    /// stackManifestHash, package)`.
    pub(crate) fn token_raise_spec(
        meteora_presale: (&str, &str),
        squads_v4: (&str, &str),
        stacks: &[(&str, &str, &str)],
    ) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("cli crate lives in the repo root")
            .join("test-vectors/protocol/token-raise.protocol-spec.json");
        let mut spec: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for (key, (program_id, hash)) in
            [("meteoraPresale", meteora_presale), ("squadsV4", squads_v4)]
        {
            spec["programs"][key] = json!({
                "kind": "programSpec",
                "programId": program_id,
                "programSpecHash": hash,
            });
        }
        spec["stacks"] = stacks
            .iter()
            .map(|(key, hash, package)| {
                (
                    key.to_string(),
                    json!({ "stackManifestHash": hash, "package": package }),
                )
            })
            .collect::<serde_json::Map<_, _>>()
            .into();
        spec
    }

    /// The spec's hash, as a resolution pins it.
    pub(crate) fn spec_hash(spec: &Value) -> String {
        arete_hash::protocol::validate_protocol_spec_value(spec)
            .expect("valid fixture spec")
            .hash()
            .unwrap()
            .to_string()
    }

    pub(crate) const DEPLOYMENT_ID: &str = "7d0c5a52-2f8e-4c1b-9a37-1b6f3c2a9e10";

    /// The protocol half of a resolution for `spec`.
    pub(crate) fn protocol(spec: &Value, access: &str) -> Value {
        json!({
            "protocolReleaseHash": format!("arete:h1:protocol-release:sha256:{}", "f".repeat(64)),
            "protocolSpecHash": spec_hash(spec),
            "protocolSpec": spec,
            "deployment": {
                "deploymentId": DEPLOYMENT_ID,
                "access": access,
                "apiUrl": "https://api.example.test/api",
                "sessionEndpoint": "https://api.example.test/ws/sessions"
            }
        })
    }
}
