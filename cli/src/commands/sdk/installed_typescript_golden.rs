//! Golden TypeScript SDKs as `a4 install --ts` writes them for registry
//! packages: a stack whose program is a program package with its own
//! extension and release identity, plus a stack extension, and the same
//! program package installed on its own.
//!
//! The extensions are written the way published ones are: they keep helper
//! types unexported and pass optional values through. The program has
//! amount-aware instructions, so its core resolves UI amounts. CI type-checks
//! these goldens, with declaration emit, under the options `tsc --init` writes
//! (`cli/tests/golden/tsconfig.strict.json`): `exactOptionalPropertyTypes`,
//! `noUncheckedIndexedAccess`, `verbatimModuleSyntax` with `module: nodenext`,
//! and `declaration`.
//!
//! Regenerate with `A4_UPDATE_GOLDEN=1 cargo test -p a4-cli installed_typescript_golden`.

use super::stack_name_golden::{collect_files, compare_with_golden, entity, local_stack};
use super::*;
use crate::project::resolver::ResolvedRegistryDependency;
use crate::project::GENERATOR_CONTRACT;
use arete_artifacts::{live_spec_v2, ProgramSpecArtifact};
use serde_json::{json, Value};

const VAULT_PROGRAM_ID: &str = "2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM";

fn golden_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/installed-typescript")
}

/// A program whose `u64` amounts take UI amounts, with decimals read from a
/// mint account, from a mint argument, from a decimals argument, or fixed.
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
              "args": [
                {{ "name": "amount", "type": "u64", "amountHint": {{ "decimalsSource": {{ "kind": "knownAccount", "accountName": "mint" }} }} }}
              ]
            }},
            {{
              "name": "withdraw",
              "discriminant": {{ "type": "u8", "value": 1 }},
              "accounts": [
                {{ "name": "authority", "isMut": true, "isSigner": true }},
                {{ "name": "vault", "isMut": true, "isSigner": false }}
              ],
              "args": [
                {{ "name": "amount", "type": "u64", "amountHint": {{ "decimalsSource": {{ "kind": "argMint", "argName": "mint" }} }} }},
                {{ "name": "mint", "type": "publicKey" }}
              ]
            }},
            {{
              "name": "payFee",
              "discriminant": {{ "type": "u8", "value": 2 }},
              "accounts": [
                {{ "name": "authority", "isMut": true, "isSigner": true }}
              ],
              "args": [
                {{ "name": "fee", "type": "u64", "amountHint": {{ "decimalsSource": {{ "kind": "constant", "decimals": 6 }} }} }},
                {{ "name": "tip", "type": "u64", "amountHint": {{ "decimalsSource": {{ "kind": "argDecimals", "argName": "tipDecimals" }} }} }},
                {{ "name": "tipDecimals", "type": "u8" }}
              ]
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
                  {{ "name": "balance", "type": "u64" }},
                  {{ "name": "memo", "type": {{ "option": "string" }} }}
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

/// The program package's extension. Its input types stay unexported, it adds
/// addresses, and it passes optional values on as they are, to the SDK and to
/// the generated semantic parameters.
const PROGRAM_EXTENSION: &str = r#"import {
  buildInstruction,
  createPreparedInstruction,
  defineProgramExtensions,
  instructionOperation,
  type AmountInput,
} from '@usearete/sdk';
import { depositInstruction, type DepositSemanticParams, type VAULT } from './vault-core.js';

interface DepositToTreasuryInput {
  authority: string;
  mint: string;
  amount: bigint;
  signers?: readonly string[];
}

interface TreasuryDepositInput {
  authority: string;
  mint: string;
  amount: AmountInput;
  decimals?: number;
}

const TREASURY_ADDRESS = 'Treasury11111111111111111111111111111111111';

export default defineProgramExtensions<typeof VAULT>()({
  addresses: {
    treasury: () => TREASURY_ADDRESS,
  },
  defaults: {
    treasuryDeposit: (input: TreasuryDepositInput): DepositSemanticParams => ({
      authority: input.authority,
      vault: TREASURY_ADDRESS,
      mint: input.mint,
      amount: input.amount,
      amountDecimals: input.decimals,
    }),
  },
  createOperations: () => ({
    instructions: {
      treasury: {
        deposit: instructionOperation(async (input: DepositToTreasuryInput) => {
          const instruction = buildInstruction(depositInstruction, {
            authority: input.authority,
            vault: TREASURY_ADDRESS,
            mint: input.mint,
            amount: input.amount,
          });
          return createPreparedInstruction({
            name: 'treasury.deposit',
            instruction,
            artifacts: { instruction },
            signers: input.signers,
          });
        }),
      },
    },
  }),
});
"#;

/// The stack's extension, with an unexported return type.
const STACK_EXTENSION: &str = r#"import { defineStackExtensions } from '@usearete/sdk';
import type { VAULT_STREAM_STACK_CORE } from './vault-core.js';

interface VaultLimits {
  maxDeposit: bigint;
}

export default defineStackExtensions<typeof VAULT_STREAM_STACK_CORE>()({
  defaults: {
    limits(): VaultLimits {
      return { maxDeposit: 1_000_000n };
    },
  },
});
"#;

fn hash(marker: char) -> String {
    marker.to_string().repeat(64)
}

fn package_release_hash(marker: char) -> String {
    format!("arete:registry-package-release:v2:sha256:{}", hash(marker))
}

/// One TypeScript SDK extension as the registry resolver returns it.
fn sdk_extension(marker: char, input: (&str, &str), entry: &str, contents: &str) -> Value {
    json!({
        "target": "typescript",
        "contentHash": hash(marker),
        "artifact": {
            "artifactHash": hash(marker),
            "manifest": {
                "entry": entry,
                "files": [entry],
                "inputKind": input.0,
                "inputHash": input.1,
                "sdkRange": null
            },
            "files": { entry: contents },
            "createdAt": "2026-09-28T00:00:00Z"
        }
    })
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

/// The `vault` program package release as a stack or a program install
/// references it, with its extension.
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
        "sdkExtensions": [sdk_extension(
            'e',
            ("program-spec", &spec_hash),
            "vault-extensions.ts",
            PROGRAM_EXTENSION,
        )]
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
        "sdkExtensions": [sdk_extension(
            'd',
            ("stack-manifest", &manifest_hash),
            "vault-stack-extensions.ts",
            STACK_EXTENSION,
        )],
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

fn generate(dependency: Value, output: &Path) {
    let dependency: ResolvedRegistryDependency =
        serde_json::from_value(dependency).expect("resolved registry dependency");
    generate_project_registry_dependency(
        &dependency,
        ProjectGenerationOptions {
            alias: "vault",
            target: crate::project::manifest::InstallTarget::TypeScript,
            output,
            typescript_package: "@usearete/sdk",
            rust_module: false,
            python_module: false,
            stack_endpoints: None,
        },
    )
    .unwrap_or_else(|error| panic!("generate {}: {error:#}", output.display()));
}

#[test]
fn installed_typescript_golden() {
    let program = vault_program();
    let temp = tempfile::tempdir().unwrap();
    generate(
        stack_dependency(&program),
        &temp.path().join("stacks/vault"),
    );
    generate(
        program_dependency(&program),
        &temp.path().join("programs/vault"),
    );

    let generated = collect_files(temp.path());
    for (entry, export) in [
        ("stacks/vault/vault.ts", "VAULT_STREAM_STACK"),
        (
            "stacks/vault/programs/vault/__arete-program.ts",
            "VAULT_PROGRAM",
        ),
        ("programs/vault/vault.ts", "VAULT_PROGRAM"),
    ] {
        // Annotated, so declaration emit names the extensions' types
        // through their modules.
        assert!(
            generated[entry].contains(&format!("export const {export}: ")),
            "{entry}:\n{}",
            generated[entry]
        );
    }
    compare_with_golden("installed-typescript", &golden_root(), generated);
}
