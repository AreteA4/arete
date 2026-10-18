//! Release derivation.
//!
//! [`derive_release`] builds the `arete.protocol-release/v1` document of a
//! spec, its optional handler bundle and its generated stack source under a
//! given compiler identity. It is a port of `buildRelease` and
//! `releaseDependencies` in `@usearete/protocol`
//! (`src/build/artifacts.ts`); both run the shared vectors in
//! `test-vectors/protocol-release/` and must agree byte for byte.
//!
//! A hosted service that accepts a protocol as one file (the spec, plus the
//! handler bundle when a step has a handler) derives everything else itself
//! with [`derive_server_release`]: the interim stack source, the release and
//! a build receipt, all under the fixed [`server_compiler`] identity. A local
//! `arete-protocol build` uses its own package and esbuild version as the
//! compiler identity and the definition's stack-source options, so its
//! release hash differs from the server-derived one only through:
//!
//! - the compiler identity (`compilerHash`), and
//! - the stack source, whose generator identity is the server's and whose
//!   program sources are the generator defaults.
//!
//! `arete-protocol build --server-release` prints the server-derived hash, so
//! authors can predict it before uploading.

use sha2::{Digest, Sha256};

use super::hash_handler_bundle;
use super::model::{ProtocolProgramV1, ProtocolSpecV1};
use super::release::{
    BuildCompilerV1, BuildOutputsV1, GeneratorV1, ProgramDependencyV1, ProtocolBuildReceiptV1,
    ProtocolReleaseV1, ProtocolStackSourceV1, ReleaseDependenciesV1, StackDependencyV1,
    PROTOCOL_BUILD_RECEIPT_SCHEMA_V1, PROTOCOL_RELEASE_SCHEMA_V1,
};
use super::stack::{generate_stack_source, StackGenerationError, StackSourceOptions};
use crate::{
    canonicalize_jcs, Compiler, CompilerSourceV1, CompilerV1, HashError, HashId,
    ProtocolHandlerBundle, ProtocolRelease, ProtocolSpec, ProtocolStackSource,
};

/// The compiler and stack-generator name of server-side derivation.
pub const SERVER_COMPILER_NAME: &str = "arete-protocol-server";

/// The version of server-side derivation. It changes only when the derived
/// documents for the same inputs change, never with crate releases, so
/// re-uploading the same spec stays idempotent across server upgrades.
pub const SERVER_DERIVATION_VERSION: &str = "1.0.0";

/// The server never bundles: the uploaded bundle is used as is. The receipt
/// contract requires a semver here, so the server records `0.0.0`.
pub const SERVER_COMPILER_ESBUILD: &str = "0.0.0";

/// The SHA-256 of zero bytes: the lockfile digest of a derivation that read
/// no lockfile.
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// A spec, bundle or stack the release cannot be derived from.
#[derive(Debug, thiserror::Error)]
pub enum ReleaseDerivationError {
    #[error("the spec has handler steps but no handler bundle was supplied")]
    MissingBundle,
    #[error("a template-only spec cannot carry a handler bundle")]
    UnexpectedBundle,
    #[error("the handler bundle is empty")]
    EmptyBundle,
    #[error("the stack source cannot be generated: {0}")]
    Stack(#[from] StackGenerationError),
    #[error("a derived document cannot be hashed: {0}")]
    Hash(#[from] HashError),
}

impl ReleaseDerivationError {
    /// A stable, kebab-case code for API errors.
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingBundle | Self::UnexpectedBundle | Self::EmptyBundle => {
                "protocol-bundle-mismatch"
            }
            Self::Stack(_) => "protocol-stack-generation-failed",
            Self::Hash(_) => "protocol-invalid-document",
        }
    }
}

impl BuildCompilerV1 {
    /// The `compiler` hash (`arete.compiler/v1`, framed tuple): one source
    /// per identity field (`esbuild`, `name`, `version`), matching
    /// `compilerHash` in `@usearete/protocol`.
    pub fn compiler_hash(&self) -> Result<HashId<Compiler>, HashError> {
        CompilerV1::new([
            CompilerSourceV1::new("esbuild", self.esbuild.as_bytes()),
            CompilerSourceV1::new("name", self.name.as_bytes()),
            CompilerSourceV1::new("version", self.version.as_bytes()),
        ])?
        .hash()
    }
}

/// The compiler identity of server-side derivation.
pub fn server_compiler() -> BuildCompilerV1 {
    BuildCompilerV1 {
        name: SERVER_COMPILER_NAME.to_string(),
        version: SERVER_DERIVATION_VERSION.to_string(),
        esbuild: SERVER_COMPILER_ESBUILD.to_string(),
    }
}

/// The stack generator identity of server-side derivation.
pub fn server_stack_generator() -> GeneratorV1 {
    GeneratorV1 {
        name: SERVER_COMPILER_NAME.to_string(),
        version: SERVER_DERIVATION_VERSION.to_string(),
    }
}

/// Stack-source options of server-side derivation: the interim profile, the
/// generator defaults for program sources, and the server generator identity.
pub fn server_stack_source_options() -> StackSourceOptions {
    StackSourceOptions {
        generator: Some(server_stack_generator()),
        ..StackSourceOptions::default()
    }
}

/// Release dependencies: every ProgramSpec program and stack, sorted by
/// alias; builtins excluded.
pub fn release_dependencies(spec: &ProtocolSpecV1) -> ReleaseDependenciesV1 {
    // BTreeMap iteration is raw UTF-8 byte order, like `compareUtf8`.
    let programs = spec
        .programs
        .iter()
        .filter_map(|(alias, program)| match program {
            ProtocolProgramV1::ProgramSpec {
                program_id,
                program_spec_hash,
                program_release_hash,
            } => Some(ProgramDependencyV1 {
                alias: alias.clone(),
                program_id: program_id.clone(),
                program_spec_hash: *program_spec_hash,
                program_release_hash: *program_release_hash,
            }),
            ProtocolProgramV1::Builtin { .. } => None,
        })
        .collect();
    let stacks = spec
        .stacks
        .iter()
        .map(|(alias, stack)| StackDependencyV1 {
            alias: alias.clone(),
            stack_manifest_hash: stack.stack_manifest_hash,
        })
        .collect();
    ReleaseDependenciesV1 {
        programs,
        stacks,
        sdk_extensions: Vec::new(),
    }
}

fn bundle_hash(
    spec: &ProtocolSpecV1,
    handler_bundle: Option<&[u8]>,
) -> Result<Option<HashId<ProtocolHandlerBundle>>, ReleaseDerivationError> {
    match (spec.has_handlers(), handler_bundle) {
        (true, Some([])) => Err(ReleaseDerivationError::EmptyBundle),
        (true, Some(bytes)) => Ok(Some(hash_handler_bundle(bytes)?)),
        (true, None) => Err(ReleaseDerivationError::MissingBundle),
        (false, Some(_)) => Err(ReleaseDerivationError::UnexpectedBundle),
        (false, None) => Ok(None),
    }
}

/// The release document of a spec, its handler bundle (exactly when a step
/// has a handler) and its stack source, under `compiler`.
pub fn derive_release(
    spec: &ProtocolSpecV1,
    handler_bundle: Option<&[u8]>,
    stack_source: &ProtocolStackSourceV1,
    compiler: &BuildCompilerV1,
) -> Result<ProtocolReleaseV1, ReleaseDerivationError> {
    Ok(ProtocolReleaseV1 {
        schema: PROTOCOL_RELEASE_SCHEMA_V1.to_string(),
        protocol: spec.name.clone(),
        version: spec.version.clone(),
        spec_hash: spec.hash()?,
        handler_bundle_hash: bundle_hash(spec, handler_bundle)?,
        stack_source_hash: stack_source.hash()?,
        compiler_hash: compiler.compiler_hash()?,
        dependencies: release_dependencies(spec),
    })
}

/// Everything server-side derivation produces from one upload.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerDerivedRelease {
    pub spec_hash: HashId<ProtocolSpec>,
    pub handler_bundle_hash: Option<HashId<ProtocolHandlerBundle>>,
    pub stack_source: ProtocolStackSourceV1,
    pub stack_source_hash: HashId<ProtocolStackSource>,
    pub release: ProtocolReleaseV1,
    pub release_hash: HashId<ProtocolRelease>,
    /// A receipt for what the server derived: the source tree is the
    /// uploaded files (`handlers.mjs`, `protocol-spec.json`), there is no
    /// lockfile and no bundled dependency.
    pub receipt: ProtocolBuildReceiptV1,
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Derive the stack source, release and receipt of an uploaded spec (and
/// bundle) under [`server_compiler`]. Deterministic: identical inputs give
/// identical documents and hashes.
pub fn derive_server_release(
    spec: &ProtocolSpecV1,
    handler_bundle: Option<&[u8]>,
) -> Result<ServerDerivedRelease, ReleaseDerivationError> {
    let handler_bundle_hash = bundle_hash(spec, handler_bundle)?;
    let stack_source = generate_stack_source(spec, &server_stack_source_options())?;
    let compiler = server_compiler();
    let release = derive_release(spec, handler_bundle, &stack_source, &compiler)?;
    let release_hash = release.hash()?;
    let spec_hash = release.spec_hash;
    let stack_source_hash = release.stack_source_hash;

    // Source tree: `[path, sha256]` pairs sorted by path, as canonical JSON,
    // the same shape `arete-protocol build` hashes.
    let mut tree: Vec<(String, String)> = Vec::with_capacity(2);
    if let Some(bytes) = handler_bundle {
        tree.push(("handlers.mjs".into(), sha256_hex(bytes)));
    }
    tree.push((
        "protocol-spec.json".into(),
        sha256_hex(&canonicalize_jcs(spec)?),
    ));
    let source_tree_sha256 = sha256_hex(&canonicalize_jcs(&tree)?);

    let receipt = ProtocolBuildReceiptV1 {
        schema: PROTOCOL_BUILD_RECEIPT_SCHEMA_V1.to_string(),
        source_tree_sha256,
        lockfile_sha256: EMPTY_SHA256.to_string(),
        dependencies: Vec::new(),
        compiler,
        outputs: BuildOutputsV1 {
            spec_hash,
            handler_bundle_hash,
            stack_source_hash,
            release_hash,
        },
    };
    Ok(ServerDerivedRelease {
        spec_hash,
        handler_bundle_hash,
        stack_source,
        stack_source_hash,
        release,
        release_hash,
        receipt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_lockfile_digest_is_sha256_of_nothing() {
        assert_eq!(sha256_hex(&[]), EMPTY_SHA256);
    }

    #[test]
    fn the_server_compiler_is_a_valid_receipt_compiler() {
        let compiler = server_compiler();
        assert_eq!(compiler.name, SERVER_COMPILER_NAME);
        compiler.compiler_hash().unwrap();
    }
}
