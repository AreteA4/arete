//! Golden Rust and Python SDKs as `a4 install --rust|--python` writes them
//! for registry packages: a stack whose program is a program package with its
//! own Rust and Python extensions and release identity, plus a stack
//! extension, and the same program package installed on its own.
//!
//! The same program bundle is staged into both: at the root of the standalone
//! program crate or package, and into the stack's program module (Rust) or
//! `program_sdks` subpackage (Python). Every Rust file must parse and every
//! Python file must compile here; `scripts/check-generated-rust-crates.sh`
//! compiles the Rust crates against the in-repo SDK in CI.
//!
//! Regenerate with `A4_UPDATE_GOLDEN=1 cargo test -p a4-cli installed_rust_python_golden`.

use super::stack_name_golden::{
    assert_syntax, collect_files, compare_with_golden, entity, local_stack, without_release_version,
};
use super::*;
use crate::project::manifest::InstallTarget;
use crate::project::resolver::ResolvedRegistryDependency;
use crate::project::GENERATOR_CONTRACT;
use arete_artifacts::{live_spec_v2, ProgramSpecArtifact};
use serde_json::{json, Value};

const VAULT_PROGRAM_ID: &str = "2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM";

fn golden_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/installed-rust-python")
}

fn vault_program() -> ProgramSpecArtifact {
    let idl = format!(
        r#"{{
          "version": "0.1.0",
          "name": "vault",
          "address": "{VAULT_PROGRAM_ID}",
          "metadata": {{ "address": "{VAULT_PROGRAM_ID}", "name": "vault", "version": "0.1.0", "spec": "0.1.0" }},
          "instructions": [
            {{
              "name": "deposit",
              "discriminant": {{ "type": "u8", "value": 0 }},
              "accounts": [
                {{ "name": "authority", "isMut": true, "isSigner": true }},
                {{ "name": "vault", "isMut": true, "isSigner": false }},
                {{ "name": "mint", "isMut": false, "isSigner": false }}
              ],
              "args": [{{ "name": "amount", "type": "u64" }}]
            }}
          ],
          "accounts": [
            {{
              "name": "Vault",
              "discriminator": [1, 0, 0, 0, 0, 0, 0, 0],
              "type": {{
                "kind": "struct",
                "fields": [
                  {{ "name": "authority", "type": "publicKey" }},
                  {{ "name": "balance", "type": "u64" }}
                ]
              }}
            }}
          ],
          "types": [],
          "events": [],
          "errors": [{{ "code": 0, "name": "AmountTooSmall", "msg": "Amount too small" }}]
        }}"#
    );
    let spec = arete_hash::build_program_spec_v1_from_bytes(idl.as_bytes(), None)
        .expect("golden ProgramSpecV1");
    ProgramSpecArtifact::new(spec).expect("golden ProgramSpec artifact")
}

/// The program package's Rust bundle: pure `constants`/`addresses`/`math`
/// namespaces, and context namespaces of async free functions taking a
/// `ProgramContext`. Generated items come through `super::generated`, the
/// helper file through `super::vault_math`, so the same files compile at a
/// standalone crate's root and inside a stack's `programs::vault` module.
const RUST_PROGRAM_HELPER: &str = r#"//! Pure vault helpers.

/// Decimals of the vault's amounts.
pub const VAULT_DECIMALS: u8 = 6;

/// A whole-unit amount in raw units.
pub fn to_raw(ui: u64) -> u64 {
    ui * 10u64.pow(VAULT_DECIMALS as u32)
}
"#;

const RUST_PROGRAM_EXTENSION: &str = r#"//! Vault program package extension (Rust).

pub use super::vault_math as math;

pub mod constants {
    pub const TREASURY: &str = "Treasury11111111111111111111111111111111111";
    pub const VAULT_DECIMALS: u8 = super::super::vault_math::VAULT_DECIMALS;
}

pub mod addresses {
    use arete_sdk::instruction::InstructionError;

    pub fn treasury() -> Result<String, InstructionError> {
        Ok(super::constants::TREASURY.to_string())
    }

    pub fn program() -> &'static str {
        super::super::generated::PROGRAM_ID
    }
}

pub mod instructions {
    pub mod treasury {
        use arete_sdk::operations::{create_prepared_instruction, PreparedInstruction};
        use arete_sdk::{AreteError, ProgramContext};
        use serde::Deserialize;

        use super::super::super::generated::{DepositParams, VaultProgram};

        #[derive(Debug, Clone, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct DepositInput {
            #[serde(default)]
            pub authority: Option<String>,
            pub mint: String,
            pub amount: u64,
        }

        /// Deposit into the treasury vault, signed by the context's wallet
        /// unless `authority` names another signer.
        pub async fn deposit(
            ctx: &ProgramContext<'_, VaultProgram>,
            input: DepositInput,
        ) -> Result<PreparedInstruction, AreteError> {
            let authority = input.authority.or_else(|| ctx.public_key()).ok_or_else(|| {
                AreteError::invalid_input("deposit needs an authority or a wallet")
            })?;
            let instruction = ctx
                .program()
                .deposit(DepositParams {
                    authority: Some(authority.clone()),
                    vault: super::super::constants::TREASURY.to_string(),
                    mint: input.mint,
                    amount: input.amount,
                })?;
            Ok(create_prepared_instruction(
                "treasury.deposit",
                instruction,
                serde_json::json!({ "authority": authority }),
                None,
                None,
            ))
        }
    }
}

pub mod read {
    use arete_sdk::{AreteError, ProgramContext};

    use super::super::generated::{Vault, VaultProgram};

    /// The cluster slot, through the client's chain reader.
    pub async fn slot(ctx: &ProgramContext<'_, VaultProgram>) -> Result<u64, AreteError> {
        ctx.chain()
            .clock()
            .await
            .map(|clock| clock.slot)
            .map_err(|error| AreteError::ConnectionFailed(error.to_string()))
    }

    /// The `Vault` account at `address`, decoded into the generated model
    /// (`None` when it does not exist).
    pub async fn vault(
        ctx: &ProgramContext<'_, VaultProgram>,
        address: &str,
    ) -> Result<Option<Vault>, AreteError> {
        ctx.program()
            .vault_accounts()?
            .fetch(address)
            .await
            .map_err(|error| AreteError::ConnectionFailed(error.to_string()))
    }

    /// The raw balance of the `Vault` account at `address`.
    pub async fn balance(
        ctx: &ProgramContext<'_, VaultProgram>,
        address: &str,
    ) -> Result<Option<u64>, AreteError> {
        Ok(vault(ctx, address).await?.and_then(|vault| vault.balance))
    }
}
"#;

/// The stack's Rust bundle: its `read` functions take the connected client.
const RUST_STACK_EXTENSION: &str = r#"//! Vault stack extension (Rust).

pub mod defaults {
    pub struct VaultLimits {
        pub max_deposit: u64,
    }

    pub fn limits() -> VaultLimits {
        VaultLimits {
            max_deposit: 1_000_000,
        }
    }
}

pub mod read {
    use arete_sdk::Arete;

    use super::super::generated::{Vault, VaultStreamStack};

    /// The streamed vault under `key`, once its view has data.
    pub async fn vault(a4: &Arete<VaultStreamStack>, key: &str) -> Option<Vault> {
        a4.views.vault.state().get(key).await
    }

    /// The vault program's account reader, as the stack binds it.
    pub fn program_id(a4: &Arete<VaultStreamStack>) -> &'static str {
        let _ = &a4.programs.vault;
        super::super::generated::programs::vault::PROGRAM_ID
    }
}
"#;

const PYTHON_PROGRAM_HELPER: &str = r#""""Pure vault helpers."""

VAULT_DECIMALS = 6


def to_raw(ui: int) -> int:
    return ui * 10**VAULT_DECIMALS
"#;

/// The program package's Python bundle: the entry exports
/// `PROGRAM_EXTENSIONS`, which the generated package applies.
const PYTHON_PROGRAM_EXTENSION: &str = r#""""Vault program package extension (Python)."""

from arete import create_prepared_instruction, instruction_operation

from . import vault_math
from .models import Vault, vault_from_wire
from .programs import VAULT_PROGRAM_ID

TREASURY = "Treasury11111111111111111111111111111111111"


def create_operations(ctx):
    async def deposit(*, mint, amount, authority=None):
        authority = authority or ctx.wallet.public_key
        instruction = ctx.program.raw.deposit.build(
            authority=authority, vault=TREASURY, mint=mint, amount=amount
        )
        return create_prepared_instruction(
            name="treasury.deposit",
            instruction=instruction,
            artifacts={"authority": authority},
        )

    return {"instructions": {"treasury": {"deposit": instruction_operation(deposit)}}}


def vault_balance(payload) -> int:
    """The raw balance of a wire `Vault` account payload."""
    vault: Vault = vault_from_wire(payload)
    return vault.balance


def create_read(ctx):
    async def slot():
        return (await ctx.chain.clock()).slot

    async def vault(address):
        return await ctx.program.accounts.vault.fetch(address)

    async def balance(address):
        account = await vault(address)
        return None if account is None else account.balance

    return {"slot": slot, "vault": vault, "balance": balance}


PROGRAM_EXTENSIONS = {
    "addresses": {"treasury": lambda: TREASURY, "program": lambda: VAULT_PROGRAM_ID},
    "constants": {"treasury": TREASURY, "vault_decimals": vault_math.VAULT_DECIMALS},
    "math": {"to_raw": vault_math.to_raw, "vault_balance": vault_balance},
    "create_operations": create_operations,
    "create_read": create_read,
}
"#;

const PYTHON_STACK_EXTENSION: &str = r#""""Vault stack extension (Python)."""


def create_read(client):
    async def vault(key):
        return await client.views.vault.state.get(key)

    return {"vault": vault}


STACK_EXTENSIONS = {
    "defaults": {"limits": lambda: {"max_deposit": 1_000_000}},
    "read_arg_counts": {"vault": 1},
    "create_read": create_read,
}
"#;

fn hash(marker: char) -> String {
    marker.to_string().repeat(64)
}

fn package_release_hash(marker: char) -> String {
    format!("arete:registry-package-release:v2:sha256:{}", hash(marker))
}

/// One Rust or Python SDK extension as the registry resolver returns it,
/// under its real content hash.
fn sdk_extension(target: &str, input: (&str, &str), files: &[(&str, &str)]) -> Value {
    let entry = match target {
        "rust" => "extensions.rs",
        _ => "extensions.py",
    };
    let files = files
        .iter()
        .map(|(path, contents)| (path.to_string(), contents.to_string()))
        .collect::<BTreeMap<_, _>>();
    let paths = files.keys().cloned().collect::<Vec<_>>();
    let content_hash = sdk_extension_content_hash(
        entry,
        &paths,
        &files,
        Some(input.0),
        Some(input.1),
        None,
        Some(target),
    );
    json!({
        "target": target,
        "contentHash": content_hash,
        "artifact": {
            "artifactHash": content_hash,
            "manifest": {
                "entry": entry,
                "files": paths,
                "inputKind": input.0,
                "inputHash": input.1,
                "sdkRange": null,
                "language": target
            },
            "files": files,
            "createdAt": "2026-09-30T00:00:00Z"
        }
    })
}

fn program_extensions(spec_hash: &str) -> Vec<Value> {
    vec![
        sdk_extension(
            "rust",
            ("program-spec", spec_hash),
            &[
                ("extensions.rs", RUST_PROGRAM_EXTENSION),
                ("vault_math.rs", RUST_PROGRAM_HELPER),
            ],
        ),
        sdk_extension(
            "python",
            ("program-spec", spec_hash),
            &[
                ("extensions.py", PYTHON_PROGRAM_EXTENSION),
                ("vault_math.py", PYTHON_PROGRAM_HELPER),
            ],
        ),
    ]
}

fn gateway_binding(scopes: &[&str], transaction_entitlement_required: bool) -> Value {
    json!({
        "endpoint": "https://solana.example.test/gateway/",
        "authPolicy": "signed_session",
        "solanaGatewayBindingId": "sgb_00000000000000000000000000000001",
        "cluster": "mainnet-beta",
        "region": "us-west-1",
        "auth": {
            "required": true,
            "mode": "signed_session",
            "sessionEndpoint": "https://api.example.test/ws/sessions",
            "jwksUrl": "https://api.example.test/.well-known/jwks.json",
            "tokenTransport": "bearer",
            "audience": "arete:solana-gateway",
            "targetKind": "solana-gateway-binding",
            "targetId": "sgb_00000000000000000000000000000001",
            "scopes": scopes,
            "acceptedKeyClasses": ["publishable", "secret"],
            "transactionEntitlementRequired": transaction_entitlement_required,
        }
    })
}

fn program_install(program: &ProgramSpecArtifact) -> Value {
    let spec_hash = program.artifact_hash.to_string();
    json!({
        "installName": "vault",
        "displayName": "vault",
        "definition": {
            "programId": program.payload.program_id,
            "programSpecHash": spec_hash,
            "idlContentHash": program.payload.idl_content_hash.to_string(),
            "normalizedIdlHash": program.payload.normalized_idl_hash.to_string(),
            "idlPayload": serde_json::to_value(&program.payload.idl_snapshot).unwrap(),
            "programSpec": serde_json::to_value(program).unwrap(),
            "extensions": null
        },
        "release": {
            "programReleaseHash": format!("arete:h1:program-release:sha256:{}", hash('a')),
            "programSpecHash": spec_hash
        },
        "transport": {
            "kind": "hosted-binding",
            "binding": {
                "endpoint": "https://reads.example.test/vault/",
                "programReadBindingId": "prb_00000000000000000000000000000001",
                "auth": {
                    "required": true,
                    "mode": "signed_session",
                    "sessionEndpoint": "https://api.example.test/ws/sessions",
                    "targetKind": "program-read-binding",
                    "targetId": "prb_00000000000000000000000000000001"
                }
            }
        },
        "chainBinding": gateway_binding(&["read"], false),
        "transactionBinding": gateway_binding(&["transaction:inspect", "transaction:send"], true),
        "programPackage": {
            "package": "vault",
            "version": "1.0.0",
            "packageReleaseHash": package_release_hash('7')
        },
        "sdkExtensions": program_extensions(&spec_hash)
    })
}

fn stack_dependency(program: &ProgramSpecArtifact) -> Value {
    let live = live_spec_v2(
        std::slice::from_ref(program),
        vec![entity("Vault")],
        Vec::new(),
    )
    .expect("golden LiveSpec");
    let stack = local_stack(
        "VaultStream",
        vec![program.clone()],
        vec![("live".to_string(), live.clone())],
    );
    let manifest_hash = stack.stack_manifest.artifact_hash.to_string();
    json!({
        "kind": "stack",
        "alias": "vault",
        "package": "vault",
        "version": "1.0.0",
        "packageReleaseHash": package_release_hash('5'),
        "generatorContract": GENERATOR_CONTRACT,
        "stackManifestHash": manifest_hash,
        "stackManifest": serde_json::to_value(&stack.stack_manifest).unwrap(),
        "liveSpecs": [{
            "alias": "live",
            "artifactHash": live.artifact_hash.to_string(),
            "artifact": serde_json::to_value(&live).unwrap()
        }],
        "programs": [program_install(program)],
        "sdkExtensions": [
            sdk_extension(
                "rust",
                ("stack-manifest", &manifest_hash),
                &[("extensions.rs", RUST_STACK_EXTENSION)],
            ),
            sdk_extension(
                "python",
                ("stack-manifest", &manifest_hash),
                &[("extensions.py", PYTHON_STACK_EXTENSION)],
            ),
        ],
        "delivery": {
            "mode": "hosted",
            "deploymentReleaseHash": format!("arete:h1:deployment-release:sha256:{}", hash('d')),
            "liveBindings": [{
                "alias": "live",
                "liveSpecHash": live.artifact_hash.to_string(),
                "binding": {
                    "deploymentId": 7,
                    "websocketEndpoint": "wss://vault.stack.example.test",
                    "queryEndpoint": "https://vault.stack.example.test",
                    "websocketAuthPolicy": "signed_session",
                    "queryAuthPolicy": "signed_session",
                    "observedGeneration": 1
                }
            }],
            "chainBinding": gateway_binding(&["read"], false),
            "transactionBinding": gateway_binding(&["transaction:inspect", "transaction:send"], true)
        }
    })
}

fn program_dependency(program: &ProgramSpecArtifact) -> Value {
    let mut install = program_install(program);
    let extensions = install["sdkExtensions"].take();
    let install = install.as_object_mut().unwrap();
    install.remove("programPackage");
    install.remove("sdkExtensions");
    json!({
        "kind": "program",
        "alias": "vault",
        "package": "vault",
        "version": "1.0.0",
        "packageReleaseHash": package_release_hash('7'),
        "generatorContract": GENERATOR_CONTRACT,
        "install": install,
        "sdkExtensions": extensions
    })
}

fn generate(dependency: &Value, target: InstallTarget, output: &Path) {
    let dependency: ResolvedRegistryDependency =
        serde_json::from_value(dependency.clone()).expect("resolved registry dependency");
    generate_project_registry_dependency(
        &dependency,
        ProjectGenerationOptions {
            alias: "vault",
            target,
            output,
            typescript_package: "@usearete/sdk",
            rust_module: false,
            python_module: false,
            stack_endpoints: None,
            reference: None,
        },
    )
    .unwrap_or_else(|error| panic!("generate {}: {error:#}", output.display()));
}

#[test]
fn installed_rust_python_golden() {
    let program = vault_program();
    let temp = tempfile::tempdir().unwrap();
    let stack = stack_dependency(&program);
    let standalone = program_dependency(&program);
    for (target, directory) in [
        (InstallTarget::Rust, "rust"),
        (InstallTarget::Python, "python"),
    ] {
        generate(
            &stack,
            target,
            &temp.path().join(directory).join("stacks/vault"),
        );
        generate(
            &standalone,
            target,
            &temp.path().join(directory).join("programs/vault"),
        );
    }

    let generated = collect_files(temp.path());
    assert_syntax("installed-rust-python", temp.path(), &generated);

    // The same bundle bytes sit at the standalone SDK root and in the stack.
    for (standalone, embedded) in [
        (
            "rust/programs/vault/src/extensions.rs",
            "rust/stacks/vault/src/programs/vault/extensions.rs",
        ),
        (
            "rust/programs/vault/src/vault_math.rs",
            "rust/stacks/vault/src/programs/vault/vault_math.rs",
        ),
        (
            "python/programs/vault/vault_program/extensions.py",
            "python/stacks/vault/vault_stack/program_sdks/vault/extensions.py",
        ),
    ] {
        assert_eq!(generated[standalone], generated[embedded], "{embedded}");
    }
    assert_python_bindings_resolve(temp.path());
    compare_with_golden(
        "installed-rust-python",
        &golden_root(),
        without_release_version(generated),
    );
}

/// Import the standalone program package and the stack package against the
/// in-repo Python SDK, and resolve the program's generated and extension
/// bindings in both: the `accounts.vault` reader, the bundle's `read`
/// functions reading it, and the account model under its standalone name
/// (`models.Vault`), which the stack's `models.py` declares as `VaultVault`.
/// (`httpx` is stubbed: program definitions are pure and CI's Rust jobs do
/// not install Python's network dependencies.)
fn assert_python_bindings_resolve(root: &Path) {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the CLI crate lives in the repository root");
    let stubs = root.join("python-stubs");
    std::fs::create_dir_all(&stubs).unwrap();
    std::fs::write(stubs.join("httpx.py"), "# Import-only stub.\n").unwrap();
    let python_path = std::env::join_paths([
        root.join("python/programs/vault"),
        root.join("python/stacks/vault"),
        stubs,
        repo_root.join("python/arete-sdk"),
    ])
    .unwrap();
    let script = r#"
import vault_program
import vault_stack
from vault_program import models as standalone_models
from vault_stack import models as stack_models
from vault_stack.program_sdks.vault import models as embedded_models
from arete.stack import ConnectedProgram


class Client:
    wallet = None
    chain = None


payload = {"authority": "Authority111", "balance": "18446744073709551615"}
contexts = [
    ("standalone", vault_program.PROGRAMS, standalone_models),
    ("embedded", vault_stack.PROGRAMS, embedded_models),
]
for label, programs, models in contexts:
    program = ConnectedProgram("vault", programs["vault"], Client(), None)
    assert program.accounts.vault.account == "Vault", label
    assert {"slot", "vault", "balance"} <= set(program.read), label
    assert program.math.vault_balance(payload) == 2**64 - 1, label
    vault = models.vault_from_wire(payload)
    assert isinstance(vault, models.Vault), label
    assert vault.authority == "Authority111", label
    assert programs["vault"].accounts["vault"].parser is models.vault_from_wire, label
# The stack's own `Vault` is its entity; the program's account model is
# `VaultVault` there, and `Vault` only inside the program's subpackage.
assert stack_models.Vault is not embedded_models.Vault
assert embedded_models.Vault is stack_models.VaultVault
"#;
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    let output = std::process::Command::new(python)
        .args(["-c", script])
        .env("PYTHONPATH", python_path)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("Python must be available for generated binding checks");
    assert!(
        output.status.success(),
        "generated Python bindings did not resolve:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Write a registry SDK extension (`sdk_extension`) as the local bundle
/// directory it was published from: its files and `extensions.json`.
fn write_local_bundle(extension: &Value, directory: &Path) {
    std::fs::create_dir_all(directory).unwrap();
    let artifact = &extension["artifact"];
    for (path, contents) in artifact["files"].as_object().unwrap() {
        std::fs::write(directory.join(path), contents.as_str().unwrap()).unwrap();
    }
    std::fs::write(
        directory.join("extensions.json"),
        serde_json::to_vec_pretty(&artifact["manifest"]).unwrap(),
    )
    .unwrap();
}

fn extension_for<'a>(extensions: &'a Value, target: &str) -> &'a Value {
    extensions
        .as_array()
        .unwrap()
        .iter()
        .find(|extension| extension["target"] == target)
        .unwrap()
}

/// The vault stack's artifacts as local files, for offline generation.
fn write_local_artifacts(program: &ProgramSpecArtifact, directory: &Path) -> PathBuf {
    std::fs::create_dir_all(directory).unwrap();
    let live = live_spec_v2(
        std::slice::from_ref(program),
        vec![entity("Vault")],
        Vec::new(),
    )
    .unwrap();
    let local = local_stack(
        "VaultStream",
        vec![program.clone()],
        vec![("live".to_string(), live.clone())],
    );
    std::fs::write(
        directory.join("vault.program-spec.json"),
        serde_json::to_vec_pretty(program).unwrap(),
    )
    .unwrap();
    std::fs::write(
        directory.join("live.live-spec.json"),
        serde_json::to_vec_pretty(&live).unwrap(),
    )
    .unwrap();
    let manifest_path = directory.join("VaultStream.stack-manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&local.stack_manifest).unwrap(),
    )
    .unwrap();
    manifest_path
}

/// `a4 sdk create --rust|--python --manifest <m> --extensions <stack bundle>
/// --program-extensions vault=<program bundle>`.
fn create_offline_stack(
    target: InstallTarget,
    manifest_path: &Path,
    output: &Path,
    stack_bundle: Option<&Path>,
    program_extensions: Vec<String>,
) -> Result<()> {
    create(
        "arete.toml",
        None,
        target == InstallTarget::TypeScript,
        target == InstallTarget::Rust,
        target == InstallTarget::Python,
        Some(output.display().to_string()),
        Some("vault-stack".to_string()),
        Some("vault-stack".to_string()),
        false,
        None,
        stack_bundle.map(|bundle| bundle.display().to_string()),
        None,
        None,
        Some(manifest_path.display().to_string()),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        program_extensions,
        false,
    )
}

/// The registry-install files an offline generation cannot reproduce: they
/// bind the hosted deployment (endpoints, served release, hosted read
/// descriptors, gateway, the package release identity stamp).
const HOSTED_BINDING_FILES: &[&str] = &[
    "src/entity.rs",
    "src/programs.rs",
    "vault_stack/__init__.py",
    "vault_stack/programs.py",
    "vault_stack/program_sdks/vault/__init__.py",
];

/// `a4 sdk create --rust|--python --manifest` with `--extensions` and
/// `--program-extensions`, offline from the same artifacts and bundles,
/// embeds the vault program's bundle exactly as `a4 install` of the hosted
/// stack does: the same files, byte for byte, except those that bind the
/// hosted deployment, whose program extension wiring is still identical.
#[test]
fn offline_stack_sdk_embeds_program_bundles_as_the_registry_install() {
    let program = vault_program();
    let temp = tempfile::tempdir().unwrap();
    let stack = stack_dependency(&program);
    let manifest_path = write_local_artifacts(&program, &temp.path().join("artifacts"));

    for (target, directory) in [
        (InstallTarget::Rust, "rust"),
        (InstallTarget::Python, "python"),
    ] {
        let registry = temp.path().join("registry").join(directory);
        generate(&stack, target, &registry);

        let bundles = temp.path().join("bundles").join(directory);
        let program_bundle = bundles.join("program");
        let stack_bundle = bundles.join("stack");
        write_local_bundle(
            extension_for(&stack["programs"][0]["sdkExtensions"], directory),
            &program_bundle,
        );
        write_local_bundle(
            extension_for(&stack["sdkExtensions"], directory),
            &stack_bundle,
        );
        let offline = temp.path().join("offline").join(directory);
        create_offline_stack(
            target,
            &manifest_path,
            &offline,
            Some(&stack_bundle),
            vec![format!("vault={}", program_bundle.display())],
        )
        .unwrap_or_else(|error| panic!("offline {directory} stack SDK: {error:#}"));

        let registry_files = collect_files(&registry);
        let offline_files = collect_files(&offline);
        assert_eq!(
            registry_files.keys().collect::<Vec<_>>(),
            offline_files.keys().collect::<Vec<_>>(),
            "{directory}: the same files"
        );
        for (path, contents) in &registry_files {
            let offline = &offline_files[path];
            if path == "sdk-manifest.json" {
                // Content-addressed over every file, hosted bindings included.
                let strip = |manifest: &str| {
                    let mut manifest: Value = serde_json::from_str(manifest).unwrap();
                    manifest
                        .as_object_mut()
                        .unwrap()
                        .remove("sdkOutputTreeHash");
                    manifest
                };
                assert_eq!(strip(contents), strip(offline), "{directory}: {path}");
            } else if path == "src/programs.rs" {
                let wiring = |programs: &str| {
                    let start = programs
                        .find("    // Hand-authored program package extension")
                        .expect("the vault module embeds its bundle");
                    programs[start..].to_string()
                };
                assert_eq!(wiring(contents), wiring(offline), "{directory}: {path}");
            } else if path == "vault_stack/program_sdks/vault/__init__.py" {
                // A hosted install stamps the package release identity after
                // the extension; an offline stack has none.
                let unstamped = contents
                    .replace(
                        "from arete import with_program_identity as _with_program_identity  # noqa: E402\n",
                        "",
                    )
                    .replace(
                        "VAULT_PROGRAM = _with_program_identity(\n    VAULT_PROGRAM,\n    package_release_hash=programs.VAULT_PACKAGE_RELEASE_HASH,\n)\n",
                        "",
                    );
                assert_eq!(&unstamped, offline, "{directory}: {path}");
            } else if !HOSTED_BINDING_FILES.contains(&path.as_str()) {
                assert_eq!(contents, offline, "{directory}: {path}");
            }
        }
    }
}

/// `--program-extensions` fails closed: every bundle must name a program of
/// the stack, declare the target language and pin that program's
/// ProgramSpec, and the flag is for offline Rust and Python stack SDKs only.
#[test]
fn offline_program_extensions_fail_closed() {
    let program = vault_program();
    let temp = tempfile::tempdir().unwrap();
    let stack = stack_dependency(&program);
    let manifest_path = write_local_artifacts(&program, &temp.path().join("artifacts"));
    let program_extensions = &stack["programs"][0]["sdkExtensions"];
    let rust_bundle = temp.path().join("bundles/rust");
    let python_bundle = temp.path().join("bundles/python");
    write_local_bundle(extension_for(program_extensions, "rust"), &rust_bundle);
    write_local_bundle(extension_for(program_extensions, "python"), &python_bundle);
    let output = temp.path().join("out");
    let error = |target, program_extensions: Vec<String>| {
        let _ = std::fs::remove_dir_all(&output);
        format!(
            "{:#}",
            create_offline_stack(target, &manifest_path, &output, None, program_extensions)
                .expect_err("generation must fail")
        )
    };

    let unknown = error(
        InstallTarget::Rust,
        vec![format!("treasury={}", rust_bundle.display())],
    );
    assert!(
        unknown.contains("names program 'treasury', which stack 'VaultStream' does not contain (its programs: vault)"),
        "{unknown}"
    );
    let language = error(
        InstallTarget::Python,
        vec![format!("vault={}", rust_bundle.display())],
    );
    assert!(
        language.contains("declares language 'rust'; a python stack SDK embeds only"),
        "{language}"
    );

    let mut repinned = extension_for(program_extensions, "rust").clone();
    repinned["artifact"]["manifest"]["inputHash"] =
        json!(format!("arete:h1:program-spec:sha256:{}", hash('0')));
    let repinned_bundle = temp.path().join("bundles/repinned");
    write_local_bundle(&repinned, &repinned_bundle);
    let pin = error(
        InstallTarget::Rust,
        vec![format!("vault={}", repinned_bundle.display())],
    );
    assert!(
        pin.contains(&format!(
            "must pin inputKind \"program-spec\" and inputHash \"{}\"",
            program.artifact_hash
        )),
        "{pin}"
    );

    let typescript = error(
        InstallTarget::TypeScript,
        vec![format!("vault={}", rust_bundle.display())],
    );
    assert!(typescript.contains("--program-module"), "{typescript}");
    let twice = error(
        InstallTarget::Rust,
        vec![
            format!("vault={}", rust_bundle.display()),
            format!("vault={}", rust_bundle.display()),
        ],
    );
    assert!(
        twice.contains("names program 'vault' more than once"),
        "{twice}"
    );
    assert!(
        !output.join("src/programs.rs").exists(),
        "nothing is generated when a bundle is refused"
    );

    // By program ID, too.
    create_offline_stack(
        InstallTarget::Rust,
        &manifest_path,
        &output,
        None,
        vec![format!("{VAULT_PROGRAM_ID}={}", rust_bundle.display())],
    )
    .unwrap();
    assert!(output.join("src/programs/vault/extensions.rs").is_file());
}

/// `extension` pinned to `input_hash` under its new content hash: a bundle
/// the registry could serve, for another input.
fn repinned(extension: &Value, input_hash: &str) -> Value {
    let files = extension["artifact"]["files"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(path, contents)| (path.clone(), contents.as_str().unwrap().to_string()))
        .collect::<Vec<_>>();
    sdk_extension(
        extension["target"].as_str().unwrap(),
        (
            extension["artifact"]["manifest"]["inputKind"]
                .as_str()
                .unwrap(),
            input_hash,
        ),
        &files
            .iter()
            .map(|(path, contents)| (path.as_str(), contents.as_str()))
            .collect::<Vec<_>>(),
    )
}

/// `extension` with its entry changed after the registry hashed it.
fn tampered(extension: &Value) -> Value {
    let mut extension = extension.clone();
    let entry = extension["artifact"]["manifest"]["entry"]
        .as_str()
        .unwrap()
        .to_string();
    let contents = &mut extension["artifact"]["files"][&entry];
    *contents = json!(format!("{}\n", contents.as_str().unwrap()));
    extension
}

/// `dependency` with the `target` entry of its `sdkExtensions` array at
/// `pointer` replaced.
fn with_extension(dependency: &Value, pointer: &str, target: &str, replacement: Value) -> Value {
    let mut dependency = dependency.clone();
    let slot = dependency
        .pointer_mut(pointer)
        .and_then(Value::as_array_mut)
        .unwrap()
        .iter_mut()
        .find(|extension| extension["target"] == target)
        .unwrap();
    *slot = replacement;
    dependency
}

/// Every file under `root`, provenance included; empty when `root` is absent.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    if root.exists() {
        walk(root, root, &mut files);
    }
    files
}

/// A generation run with an accepted bundle and with a refused one.
enum Generation {
    /// `a4 install` of a registry dependency.
    Hosted { accepted: Value, refused: Value },
    /// `a4 sdk create --manifest` of the vault stack with a stack bundle.
    OfflineStack { accepted: PathBuf, refused: PathBuf },
    /// `a4 sdk create --program-spec` of the vault program with a bundle.
    OfflineProgram { accepted: PathBuf, refused: PathBuf },
}

/// A Rust or Python generation checks every bundle before it writes anything:
/// a refused bundle (pinned to another input, tampered with after the
/// registry hashed it, or built for another extension API) leaves no output
/// where there was none, and an earlier SDK at the output exactly as it was.
/// For stack and standalone program SDKs, from registry installs and offline.
#[test]
fn a_refused_bundle_leaves_the_output_untouched() {
    let program = vault_program();
    let temp = tempfile::tempdir().unwrap();
    let stack = stack_dependency(&program);
    let standalone = program_dependency(&program);
    let manifest_path = write_local_artifacts(&program, &temp.path().join("artifacts"));
    let local_program = load_local_program_source(
        Some(
            temp.path()
                .join("artifacts/vault.program-spec.json")
                .to_str()
                .unwrap(),
        ),
        None,
    )
    .unwrap()
    .expect("a ProgramSpec source");
    let other_program = format!("arete:h1:program-spec:sha256:{}", hash('0'));
    let other_stack = format!("arete:h1:stack-manifest:sha256:{}", hash('0'));
    // A path `arete-a4-sdk` that provides extension API 1, around the outputs
    // of the extension API case.
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(workspace.join("sdk")).unwrap();
    std::fs::write(
        workspace.join("sdk/Cargo.toml"),
        "[package]\nname = \"arete-a4-sdk\"\nversion = \"0.23.0\"\n\n[package.metadata.arete]\nextension-api = 1\n",
    )
    .unwrap();
    std::fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\narete-sdk = { package = \"arete-a4-sdk\", path = \"sdk\" }\n",
    )
    .unwrap();

    for (target, language) in [
        (InstallTarget::Rust, "rust"),
        (InstallTarget::Python, "python"),
    ] {
        let program_extension = extension_for(&stack["programs"][0]["sdkExtensions"], language);
        let stack_extension = extension_for(&stack["sdkExtensions"], language);
        let standalone_extension = extension_for(&standalone["sdkExtensions"], language);
        let bundles = temp.path().join("bundles").join(language);
        let bundle = |name: &str, extension: &Value| {
            let directory = bundles.join(name);
            write_local_bundle(extension, &directory);
            directory
        };
        let program_bundle = bundle("program", program_extension);
        let stack_bundle = bundle("stack", stack_extension);

        let mut cases = vec![
            (
                "hosted stack, program extension pinned to another program",
                Generation::Hosted {
                    accepted: stack.clone(),
                    refused: with_extension(
                        &stack,
                        "/programs/0/sdkExtensions",
                        language,
                        repinned(program_extension, &other_program),
                    ),
                },
                "extensions input hash mismatch",
            ),
            (
                "hosted stack, program extension tampered with",
                Generation::Hosted {
                    accepted: stack.clone(),
                    refused: with_extension(
                        &stack,
                        "/programs/0/sdkExtensions",
                        language,
                        tampered(program_extension),
                    ),
                },
                "does not hash to its content hash",
            ),
            (
                "hosted stack, stack extension pinned to another stack",
                Generation::Hosted {
                    accepted: stack.clone(),
                    refused: with_extension(
                        &stack,
                        "/sdkExtensions",
                        language,
                        repinned(stack_extension, &other_stack),
                    ),
                },
                "extensions input hash mismatch",
            ),
            (
                "hosted program, extension pinned to another program",
                Generation::Hosted {
                    accepted: standalone.clone(),
                    refused: with_extension(
                        &standalone,
                        "/sdkExtensions",
                        language,
                        repinned(standalone_extension, &other_program),
                    ),
                },
                "extensions input hash mismatch",
            ),
            (
                "offline stack, --extensions bundle pinned to another stack",
                Generation::OfflineStack {
                    accepted: stack_bundle.clone(),
                    refused: bundle("stack-repinned", &repinned(stack_extension, &other_stack)),
                },
                "extensions input hash mismatch",
            ),
            (
                "offline program, --extensions bundle pinned to another program",
                Generation::OfflineProgram {
                    accepted: program_bundle.clone(),
                    refused: bundle(
                        "program-repinned",
                        &repinned(program_extension, &other_program),
                    ),
                },
                "extensions input hash mismatch",
            ),
        ];
        if target == InstallTarget::Rust {
            let mut newer = standalone_extension.clone();
            newer["artifact"]["manifest"]["extensionApi"] = json!(2);
            cases.push((
                "hosted program, extension built for another extension API",
                Generation::Hosted {
                    accepted: standalone.clone(),
                    refused: with_extension(&standalone, "/sdkExtensions", language, newer),
                },
                "provides extension API 1",
            ));
        }

        let generate = |generation: &Generation, output: &Path, refuse: bool| -> Result<()> {
            match generation {
                Generation::Hosted { accepted, refused } => {
                    let dependency: ResolvedRegistryDependency =
                        serde_json::from_value(if refuse { refused } else { accepted }.clone())
                            .unwrap();
                    generate_project_registry_dependency(
                        &dependency,
                        ProjectGenerationOptions {
                            alias: "vault",
                            target,
                            output,
                            typescript_package: "@usearete/sdk",
                            rust_module: false,
                            python_module: false,
                            stack_endpoints: None,
                            reference: None,
                        },
                    )
                }
                Generation::OfflineStack { accepted, refused } => create_offline_stack(
                    target,
                    &manifest_path,
                    output,
                    Some(if refuse { refused } else { accepted }.as_path()),
                    vec![format!("vault={}", program_bundle.display())],
                ),
                Generation::OfflineProgram { accepted, refused } => create_local_program_sdk(
                    &local_program,
                    target,
                    Some(output.display().to_string()),
                    Some("vault-program".to_string()),
                    false,
                    Some(
                        if refuse { refused } else { accepted }
                            .display()
                            .to_string(),
                    ),
                ),
            }
        };
        for (index, (case, generation, refusal)) in cases.iter().enumerate() {
            let root = if case.contains("extension API") {
                &workspace
            } else {
                temp.path()
            }
            .join(format!("{language}-{index}"));
            let absent = root.join("absent");
            let error = generate(generation, &absent, true)
                .expect_err(&format!("{language}: {case}: refused"));
            let error = format!("{error:#}");
            assert!(error.contains(refusal), "{language}: {case}: {error}");
            assert!(
                !absent.exists(),
                "{language}: {case}: a refused bundle left {}",
                absent.display()
            );

            let existing = root.join("existing");
            generate(generation, &existing, false)
                .unwrap_or_else(|error| panic!("{language}: {case}: accepted: {error:#}"));
            let before = snapshot(&existing);
            assert!(!before.is_empty());
            generate(generation, &existing, true)
                .expect_err(&format!("{language}: {case}: refused"));
            assert!(
                snapshot(&existing) == before,
                "{language}: {case}: a refused bundle changed {}",
                existing.display()
            );
        }
    }
}
