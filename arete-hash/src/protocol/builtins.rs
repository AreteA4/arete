//! ProgramSpecs for the builtin programs, so templates can call them like
//! installed programs.
//!
//! System, SPL Token, Associated Token and Memo carry the registry's
//! published ProgramSpecs (`system-program`, `spl-token`, `spl-ata` and
//! `spl-memo`), which hash the same. Compute Budget has no registry package; its
//! ProgramSpec is built from the IDL in `builtins/compute-budget.idl.json`.
//! Each is pinned by hash: [`BuiltinProgram::program_spec_hash`] is fixed,
//! and a test fails when an embedded spec no longer hashes to it.

use std::sync::OnceLock;

use crate::{
    build_program_spec_v1_from_bytes, AccountResolutionV1, InstructionDefinitionV1, PdaSeedV1,
    ProgramSpecV1,
};

use super::BuiltinProgram;

const SYSTEM: &str = include_str!("builtins/system-program-canonical.program-spec.json");
const SPL_TOKEN: &str = include_str!("builtins/spl-token.program-spec.json");
const ASSOCIATED_TOKEN: &str = include_str!("builtins/spl-ata.program-spec.json");
const MEMO: &str = include_str!("builtins/spl-memo.program-spec.json");
const COMPUTE_BUDGET_IDL: &str = include_str!("builtins/compute-budget.idl.json");

/// Other names agents and tools use for each builtin: registry package
/// names and the program SDK names generated from them.
const ALIASES: &[(&str, BuiltinProgram)] = &[
    ("system-program", BuiltinProgram::System),
    ("systemProgram", BuiltinProgram::System),
    ("spl-token", BuiltinProgram::SplToken),
    ("token", BuiltinProgram::SplToken),
    ("spl-ata", BuiltinProgram::AssociatedToken),
    ("splAta", BuiltinProgram::AssociatedToken),
    ("ata", BuiltinProgram::AssociatedToken),
    ("associated-token", BuiltinProgram::AssociatedToken),
    (
        "spl-associated-token-account",
        BuiltinProgram::AssociatedToken,
    ),
    ("compute-budget", BuiltinProgram::ComputeBudget),
    ("spl-memo", BuiltinProgram::Memo),
    ("splMemo", BuiltinProgram::Memo),
];

fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect()
}

impl BuiltinProgram {
    /// Every builtin, in [`BuiltinProgram::ALL`] order.
    pub const VALUES: [Self; 5] = [
        Self::System,
        Self::SplToken,
        Self::AssociatedToken,
        Self::ComputeBudget,
        Self::Memo,
    ];

    /// The canonical name, as specs and `builtins(...)` write it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::SplToken => "splToken",
            Self::AssociatedToken => "associatedToken",
            Self::ComputeBudget => "computeBudget",
            Self::Memo => "memo",
        }
    }

    /// The builtin with this canonical name.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::VALUES
            .into_iter()
            .find(|builtin| builtin.name() == name)
    }

    /// The builtin another name refers to (`spl-ata`, `splAta` →
    /// `associatedToken`), for error messages. `None` for a canonical name
    /// written exactly, and for names that are no builtin.
    pub fn from_alias(name: &str) -> Option<Self> {
        if Self::from_name(name).is_some() {
            return None;
        }
        let normalized = normalize(name);
        Self::VALUES
            .into_iter()
            .find(|builtin| normalize(builtin.name()) == normalized)
            .or_else(|| {
                ALIASES
                    .iter()
                    .find(|(alias, _)| normalize(alias) == normalized)
                    .map(|(_, builtin)| *builtin)
            })
    }

    /// The registry package the ProgramSpec comes from, if any.
    pub const fn registry_package(self) -> Option<&'static str> {
        match self {
            Self::System => Some("system-program"),
            Self::SplToken => Some("spl-token"),
            Self::AssociatedToken => Some("spl-ata"),
            Self::ComputeBudget => None,
            Self::Memo => Some("spl-memo"),
        }
    }

    /// The pinned hash of [`BuiltinProgram::program_spec`].
    pub const fn program_spec_hash(self) -> &'static str {
        match self {
            Self::System => {
                "arete:h1:program-spec:sha256:a416fd5d908028d96d4e6445d1da3c7a60147e55711640c281d91f6602c7fb17"
            }
            Self::SplToken => {
                "arete:h1:program-spec:sha256:69adc15288ea9f6009434e446a3d4aa2a3a171169072568754596cf13d685fa1"
            }
            Self::AssociatedToken => {
                "arete:h1:program-spec:sha256:8a2641d19db953d2945a6eb864c4135e0748f5467a5e298d5969972d44441ea4"
            }
            Self::ComputeBudget => {
                "arete:h1:program-spec:sha256:66dcfb8f7fa0f43dcb6a9d4dda212134ad9fab15292ba73665c8cfca84ec1b5b"
            }
            Self::Memo => {
                "arete:h1:program-spec:sha256:63fe09c92de2095d6f56683451058871e12c2f584f4d9ad35a53e6d85daf64a7"
            }
        }
    }

    /// The builtin's ProgramSpec, which templates resolve against.
    pub fn program_spec(self) -> &'static ProgramSpecV1 {
        static SPECS: OnceLock<[ProgramSpecV1; 5]> = OnceLock::new();
        let specs = SPECS.get_or_init(|| Self::VALUES.map(build_spec));
        &specs[self as usize]
    }
}

fn build_spec(builtin: BuiltinProgram) -> ProgramSpecV1 {
    let spec = match builtin {
        BuiltinProgram::System => serde_json::from_str(SYSTEM),
        BuiltinProgram::SplToken => serde_json::from_str(SPL_TOKEN),
        BuiltinProgram::AssociatedToken => serde_json::from_str(ASSOCIATED_TOKEN),
        BuiltinProgram::Memo => serde_json::from_str(MEMO),
        BuiltinProgram::ComputeBudget => {
            return build_program_spec_v1_from_bytes(COMPUTE_BUDGET_IDL.as_bytes(), None)
                .expect("the embedded Compute Budget IDL builds");
        }
    };
    spec.expect("an embedded builtin ProgramSpec parses")
}

/// Where the generated TypeScript builtin modules live, relative to the
/// repository root.
#[doc(hidden)]
pub const BUILTIN_MODULES_TYPESCRIPT_PATH: &str = "typescript/protocol/src/builtins.generated.ts";

/// The TypeScript module `builtins(...)` reads: each builtin's program id,
/// pinned ProgramSpec hash and instruction shapes, in the shape generated
/// program modules use. Regenerate with
/// `cargo run -p arete-hash --example generate_builtin_modules`.
#[doc(hidden)]
pub fn builtin_modules_typescript() -> String {
    let q = |text: &str| serde_json::to_string(text).expect("a string serializes");
    let mut out = String::from(
        "// Generated by `cargo run -p arete-hash --example generate_builtin_modules`\n\
         // from the builtin ProgramSpecs in arete-hash. Do not edit.\n\n\
         /** Each builtin program's id, pinned ProgramSpec hash and instruction shapes. */\n\
         export const BUILTIN_PROGRAM_MODULES = {\n",
    );
    for builtin in BuiltinProgram::VALUES {
        let spec = builtin.program_spec();
        out.push_str(&format!(
            "  {}: {{\n    builtin: {},\n    programId: {},\n    programSpecHash: {},\n    instructionShapes: {{\n",
            builtin.name(),
            q(builtin.name()),
            q(builtin.program_id()),
            q(builtin.program_spec_hash()),
        ));
        for ix in &spec.instructions {
            let args = ix.args.iter().map(|arg| q(&arg.name)).collect::<Vec<_>>();
            out.push_str(&format!(
                "      {}: {{\n        args: [{}],\n        accounts: {{",
                q(&ix.name),
                args.join(", ")
            ));
            if ix.accounts.is_empty() {
                out.push_str("},\n");
            } else {
                out.push('\n');
                for account in &ix.accounts {
                    let derive = match account_derivation(spec, ix, &account.resolution) {
                        Some(names) => format!(
                            "[{}]",
                            names
                                .iter()
                                .map(|name| q(name))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        None => "null".into(),
                    };
                    out.push_str(&format!(
                        "          {}: {{ optional: {}, derive: {} }},\n",
                        q(&account.name),
                        account.is_optional,
                        derive
                    ));
                }
                out.push_str("        },\n");
            }
            out.push_str("      },\n");
        }
        out.push_str("    },\n  },\n");
    }
    out.push_str("} as const;\n");
    out
}

/// The accounts an account's address derives from, as the template check
/// derives it: `[]` for a fixed address or a PDA of literal and argument
/// seeds, the seed accounts for a PDA over accounts, `None` when a template
/// must supply it.
fn account_derivation(
    spec: &ProgramSpecV1,
    ix: &InstructionDefinitionV1,
    resolution: &AccountResolutionV1,
) -> Option<Vec<String>> {
    let seeds = |seeds: &[PdaSeedV1], dynamic_program: bool| {
        if dynamic_program {
            return None;
        }
        let mut accounts = Vec::new();
        for seed in seeds {
            match seed {
                PdaSeedV1::Literal { .. } | PdaSeedV1::Bytes { .. } => {}
                PdaSeedV1::ArgRef { arg_name, .. } => {
                    if !ix.args.iter().any(|arg| arg.name == *arg_name) {
                        return None;
                    }
                }
                PdaSeedV1::AccountRef { account_name } => {
                    if !accounts.contains(account_name) {
                        accounts.push(account_name.clone());
                    }
                }
            }
        }
        Some(accounts)
    };
    match resolution {
        AccountResolutionV1::Known { .. } => Some(Vec::new()),
        AccountResolutionV1::PdaRef { pda_name } => spec
            .pdas
            .get(pda_name)
            .and_then(|pda| seeds(&pda.seeds, pda.program.is_some())),
        AccountResolutionV1::PdaInline {
            seeds: s, program, ..
        } => seeds(s, program.is_some()),
        AccountResolutionV1::Signer | AccountResolutionV1::UserProvided => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_specs_hash_to_their_pins() {
        for builtin in BuiltinProgram::VALUES {
            let spec = builtin.program_spec();
            assert_eq!(spec.program_id, builtin.program_id(), "{builtin:?}");
            assert_eq!(
                spec.hash().unwrap().to_string(),
                builtin.program_spec_hash(),
                "{builtin:?}"
            );
        }
    }

    #[test]
    fn generated_typescript_is_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(BUILTIN_MODULES_TYPESCRIPT_PATH);
        let Ok(current) = std::fs::read_to_string(&path) else {
            // Outside the repository (a packaged crate) there is nothing to compare.
            return;
        };
        assert!(
            current == builtin_modules_typescript(),
            "{BUILTIN_MODULES_TYPESCRIPT_PATH} is stale: run `cargo run -p arete-hash --example generate_builtin_modules`"
        );
    }

    #[test]
    fn values_match_all_and_serde() {
        for (builtin, name) in BuiltinProgram::VALUES.iter().zip(BuiltinProgram::ALL) {
            assert_eq!(builtin.name(), name);
            assert_eq!(
                serde_json::to_value(builtin).unwrap(),
                serde_json::Value::String(name.into())
            );
        }
    }

    #[test]
    fn aliases_name_the_canonical_builtin() {
        assert_eq!(
            BuiltinProgram::from_alias("spl-ata"),
            Some(BuiltinProgram::AssociatedToken)
        );
        assert_eq!(
            BuiltinProgram::from_alias("splAta"),
            Some(BuiltinProgram::AssociatedToken)
        );
        assert_eq!(
            BuiltinProgram::from_alias("associated_token"),
            Some(BuiltinProgram::AssociatedToken)
        );
        assert_eq!(
            BuiltinProgram::from_alias("spl-token"),
            Some(BuiltinProgram::SplToken)
        );
        assert_eq!(
            BuiltinProgram::from_alias("compute-budget"),
            Some(BuiltinProgram::ComputeBudget)
        );
        assert_eq!(BuiltinProgram::from_alias("associatedToken"), None);
        assert_eq!(BuiltinProgram::from_alias("vault"), None);
    }

    #[test]
    fn specs_name_the_canonical_builtin_for_an_alias() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../test-vectors/protocol/escrow-resolution.protocol-spec.json"
        ))
        .unwrap();

        let mut spec = fixture.clone();
        spec["programs"]["ata"] = serde_json::json!({ "kind": "builtin", "builtin": "spl-ata" });
        let error = super::super::validate_protocol_spec_value(&spec).unwrap_err();
        assert_eq!(error.code, "invalid-value");
        assert_eq!(error.path, "/programs/ata/builtin");
        assert_eq!(error.message, "'spl-ata' is the builtin 'associatedToken'");

        let mut spec = fixture;
        spec["steps"]["open"]["build"]["instructions"][0]["program"] = "splAta".into();
        let error = super::super::validate_protocol_spec_value(&spec).unwrap_err();
        assert_eq!(error.code, "unknown-program");
        assert!(
            error
                .message
                .contains("'splAta' is the builtin 'associatedToken'"),
            "{}",
            error.message
        );
    }

    #[test]
    fn associated_token_create_idempotent_names_its_accounts() {
        let spec = BuiltinProgram::AssociatedToken.program_spec();
        let ix = spec
            .instructions
            .iter()
            .find(|ix| ix.name == "createIdempotent")
            .unwrap();
        assert_eq!(ix.discriminator, vec![1]);
        let names: Vec<_> = ix.accounts.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "fundingAddress",
                "associatedTokenAccountAddress",
                "walletAddress",
                "tokenMintAddress",
                "systemProgram",
                "tokenProgram"
            ]
        );
    }

    #[test]
    fn compute_budget_instructions_use_one_byte_discriminators() {
        let spec = BuiltinProgram::ComputeBudget.program_spec();
        let found: Vec<_> = spec
            .instructions
            .iter()
            .map(|ix| (ix.name.as_str(), ix.discriminator.clone()))
            .collect();
        assert_eq!(
            found,
            [
                ("requestHeapFrame", vec![1]),
                ("setComputeUnitLimit", vec![2]),
                ("setComputeUnitPrice", vec![3]),
                ("setLoadedAccountsDataSizeLimit", vec![4]),
            ]
        );
    }
}
