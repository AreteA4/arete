//! Instruction account resolution.
//!
//! Resolution order:
//! 1. Non-PDA accounts (signer, known, user-provided) resolve first.
//! 2. PDA accounts resolve in dependency order (accounts they reference via
//!    `accountRef` seeds come first).
//!
//! Output preserves the original account order. Omitted optional accounts that
//! precede a resolved account get the program ID as a placeholder (Anchor's
//! convention); trailing omitted optionals are dropped.

use std::collections::{BTreeMap, HashSet};

use serde_json::{Map, Value};

use super::seed::{derive_program_address, serialize_seed_value};
use super::types::{AccountMeta, AccountResolution, InstructionError, PdaConfig, PdaSeed};

/// A single account with its resolved base58 address.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAccount {
    /// Account name.
    pub name: String,
    /// Resolved base58 address.
    pub address: String,
    /// Whether this account must sign.
    pub is_signer: bool,
    /// Whether this account is writable.
    pub is_writable: bool,
}

/// Result of resolving an instruction's accounts.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountResolutionResult {
    /// Resolved accounts in the original declaration order.
    pub accounts: Vec<ResolvedAccount>,
    /// Names of required accounts that could not be resolved.
    pub missing: Vec<String>,
}

/// Resolves instruction accounts against args, overrides, and a fallback payer.
///
/// `overrides` are explicit account-address overrides (including signer slots);
/// `resolve` carries helper-only PDA seed inputs that are not serialized
/// on-chain; `program_id` is the fallback program for PDA derivation and the
/// placeholder for omitted non-trailing optional accounts.
pub fn resolve_accounts(
    metas: &[AccountMeta],
    args: &Map<String, Value>,
    overrides: &BTreeMap<String, String>,
    resolve: Option<&Map<String, Value>>,
    payer: Option<&str>,
    program_id: Option<&str>,
) -> Result<AccountResolutionResult, InstructionError> {
    let sorted = sort_by_dependency(metas)?;

    let mut resolved: BTreeMap<String, ResolvedAccount> = BTreeMap::new();
    let mut missing = Vec::new();
    for meta in sorted {
        match resolve_single(meta, args, overrides, resolve, payer, program_id, &resolved)? {
            Some(account) => {
                resolved.insert(meta.name.clone(), account);
            }
            None if !meta.is_optional => missing.push(meta.name.clone()),
            None => {}
        }
    }

    // Return accounts in original order. Omitted optional accounts that
    // precede a resolved account cannot simply be dropped — that would shift
    // every later account into the wrong slot — so they get the program ID as
    // a placeholder; trailing omitted optionals are dropped as usual.
    let last_resolved = metas
        .iter()
        .rposition(|meta| resolved.contains_key(&meta.name));
    let mut accounts = Vec::new();
    for (index, meta) in metas.iter().enumerate() {
        if let Some(account) = resolved.get(&meta.name) {
            accounts.push(account.clone());
        } else if meta.is_optional && last_resolved.is_some_and(|last| index < last) {
            let Some(placeholder) = program_id.filter(|id| !id.is_empty()) else {
                return Err(InstructionError::Pda(format!(
                    "Omitted optional account \"{}\" precedes other accounts and needs the \
                     program ID as a placeholder, but no program ID was provided",
                    meta.name
                )));
            };
            accounts.push(ResolvedAccount {
                name: meta.name.clone(),
                address: placeholder.to_string(),
                is_signer: false,
                is_writable: false,
            });
        }
    }

    Ok(AccountResolutionResult { accounts, missing })
}

/// Topologically sorts accounts so that `accountRef` dependencies resolve
/// first: non-PDA accounts, then PDAs in dependency order.
fn sort_by_dependency(metas: &[AccountMeta]) -> Result<Vec<&AccountMeta>, InstructionError> {
    let mut sorted: Vec<&AccountMeta> = Vec::with_capacity(metas.len());
    let mut pdas: Vec<&AccountMeta> = Vec::new();
    for meta in metas {
        if matches!(meta.resolution, AccountResolution::Pda(_)) {
            pdas.push(meta);
        } else {
            sorted.push(meta);
        }
    }

    let mut visited = HashSet::new();
    let mut visiting = HashSet::new();
    for meta in &pdas {
        visit(&meta.name, &pdas, &mut visited, &mut visiting, &mut sorted)?;
    }
    Ok(sorted)
}

fn visit<'a>(
    name: &str,
    pdas: &[&'a AccountMeta],
    visited: &mut HashSet<String>,
    visiting: &mut HashSet<String>,
    out: &mut Vec<&'a AccountMeta>,
) -> Result<(), InstructionError> {
    if visited.contains(name) {
        return Ok(());
    }
    if visiting.contains(name) {
        return Err(InstructionError::CircularPdaDependency(name.to_string()));
    }
    let Some(meta) = pdas.iter().copied().find(|meta| meta.name == name) else {
        return Ok(()); // Not a PDA; resolved in the non-PDA pass.
    };
    visiting.insert(name.to_string());
    if let AccountResolution::Pda(config) = &meta.resolution {
        for seed in &config.seeds {
            if let PdaSeed::AccountRef(dep) = seed {
                if pdas.iter().any(|candidate| candidate.name == *dep) {
                    visit(dep, pdas, visited, visiting, out)?;
                }
            }
        }
    }
    visiting.remove(name);
    visited.insert(name.to_string());
    out.push(meta);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn resolve_single(
    meta: &AccountMeta,
    args: &Map<String, Value>,
    overrides: &BTreeMap<String, String>,
    resolve: Option<&Map<String, Value>>,
    payer: Option<&str>,
    program_id: Option<&str>,
    resolved: &BTreeMap<String, ResolvedAccount>,
) -> Result<Option<ResolvedAccount>, InstructionError> {
    match &meta.resolution {
        AccountResolution::Signer => {
            let address = overrides.get(&meta.name).map(String::as_str).or(payer);
            Ok(address.map(|address| ResolvedAccount {
                name: meta.name.clone(),
                address: address.to_string(),
                is_signer: true,
                is_writable: meta.is_writable,
            }))
        }
        AccountResolution::Known(address) => Ok(Some(ResolvedAccount {
            name: meta.name.clone(),
            address: address.clone(),
            is_signer: meta.is_signer,
            is_writable: meta.is_writable,
        })),
        AccountResolution::UserProvided => {
            Ok(overrides.get(&meta.name).map(|address| ResolvedAccount {
                name: meta.name.clone(),
                address: address.clone(),
                is_signer: meta.is_signer,
                is_writable: meta.is_writable,
            }))
        }
        AccountResolution::Pda(config) => {
            resolve_pda(meta, config, args, resolve, resolved, program_id).map(Some)
        }
    }
}

fn resolve_pda(
    meta: &AccountMeta,
    config: &PdaConfig,
    args: &Map<String, Value>,
    resolve: Option<&Map<String, Value>>,
    resolved: &BTreeMap<String, ResolvedAccount>,
    fallback_program_id: Option<&str>,
) -> Result<ResolvedAccount, InstructionError> {
    let program_id = config
        .program_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .or(fallback_program_id)
        .ok_or_else(|| InstructionError::MissingProgramId(meta.name.clone()))?;

    let mut seeds = Vec::with_capacity(config.seeds.len());
    for seed in &config.seeds {
        match seed {
            PdaSeed::Literal(text) => seeds.push(text.as_bytes().to_vec()),
            PdaSeed::Bytes(bytes) => seeds.push(bytes.clone()),
            PdaSeed::ArgRef { arg, arg_type } => {
                let value =
                    get_value_by_path(Some(args), arg).or_else(|| get_value_by_path(resolve, arg));
                let Some(value) = value else {
                    return Err(InstructionError::Pda(format!(
                        "PDA seed references missing argument: {arg} (for account \"{}\")",
                        meta.name
                    )));
                };
                seeds.push(serialize_seed_value(value, arg_type.as_deref())?);
            }
            PdaSeed::AccountRef(account_name) => {
                let Some(referenced) = resolved.get(account_name) else {
                    return Err(InstructionError::Pda(format!(
                        "PDA seed references unresolved account: {account_name} (for account \"{}\")",
                        meta.name
                    )));
                };
                let bytes = bs58::decode(&referenced.address)
                    .into_vec()
                    .map_err(|_| InstructionError::InvalidPubkey(referenced.address.clone()))?;
                seeds.push(bytes);
            }
        }
    }

    let (address, _bump) = derive_program_address(&seeds, program_id)?;
    Ok(ResolvedAccount {
        name: meta.name.clone(),
        address: address.to_string(),
        is_signer: meta.is_signer,
        is_writable: meta.is_writable,
    })
}

/// Dot-path lookup into a JSON object: `"a.b.c"` walks nested objects, a plain
/// key is a direct lookup. JSON null counts as missing (mirroring the `??`
/// fallthrough in the TypeScript resolver).
fn get_value_by_path<'a>(source: Option<&'a Map<String, Value>>, path: &str) -> Option<&'a Value> {
    let source = source?;
    let value = if let Some(direct) = source.get(path) {
        direct
    } else {
        let mut segments = path.split('.');
        let mut current = source.get(segments.next()?)?;
        for segment in segments {
            current = current.as_object()?.get(segment)?;
        }
        current
    };
    (!value.is_null()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
    const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
    const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

    fn meta(name: &str, resolution: AccountResolution) -> AccountMeta {
        AccountMeta {
            name: name.to_string(),
            is_signer: false,
            is_writable: false,
            resolution,
            is_optional: false,
        }
    }

    fn signer(name: &str) -> AccountMeta {
        AccountMeta {
            is_signer: true,
            is_writable: true,
            ..meta(name, AccountResolution::Signer)
        }
    }

    fn overrides(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn resolve_ok(
        metas: &[AccountMeta],
        args: Value,
        overrides: &BTreeMap<String, String>,
        payer: Option<&str>,
        program_id: Option<&str>,
    ) -> AccountResolutionResult {
        let args = args.as_object().cloned().unwrap();
        let result = resolve_accounts(metas, &args, overrides, None, payer, program_id).unwrap();
        assert!(
            result.missing.is_empty(),
            "unexpected missing: {:?}",
            result.missing
        );
        result
    }

    fn expected_pda(seeds: &[Vec<u8>], program_id: &str) -> String {
        derive_program_address(seeds, program_id)
            .unwrap()
            .0
            .to_string()
    }

    #[test]
    fn resolves_signer_known_and_user_provided() {
        let metas = [
            signer("authority"),
            meta(
                "systemProgram",
                AccountResolution::Known(SYSTEM_PROGRAM.to_string()),
            ),
            meta("mint", AccountResolution::UserProvided),
        ];
        let result = resolve_ok(
            &metas,
            json!({}),
            &overrides(&[("mint", TOKEN_PROGRAM)]),
            Some(WSOL_MINT),
            None,
        );
        let names: Vec<&str> = result.accounts.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["authority", "systemProgram", "mint"]);
        assert_eq!(result.accounts[0].address, WSOL_MINT);
        assert!(result.accounts[0].is_signer);
        assert_eq!(result.accounts[1].address, SYSTEM_PROGRAM);
        assert_eq!(result.accounts[2].address, TOKEN_PROGRAM);
    }

    #[test]
    fn prefers_explicit_signer_overrides_over_the_payer() {
        let metas = [
            signer("authority"),
            meta("mint", AccountResolution::UserProvided),
        ];
        let result = resolve_ok(
            &metas,
            json!({}),
            &overrides(&[("authority", TOKEN_PROGRAM), ("mint", WSOL_MINT)]),
            Some(WSOL_MINT),
            None,
        );
        let addresses: Vec<&str> = result.accounts.iter().map(|a| a.address.as_str()).collect();
        assert_eq!(addresses, [TOKEN_PROGRAM, WSOL_MINT]);
    }

    #[test]
    fn derives_a_pda_referencing_a_signer_and_keeps_original_order() {
        let metas = [
            signer("authority"),
            AccountMeta {
                is_writable: true,
                ..meta(
                    "state",
                    AccountResolution::Pda(PdaConfig {
                        program_id: Some(TOKEN_PROGRAM.to_string()),
                        seeds: vec![
                            PdaSeed::Literal("state".to_string()),
                            PdaSeed::AccountRef("authority".to_string()),
                        ],
                    }),
                )
            },
        ];
        let result = resolve_ok(&metas, json!({}), &BTreeMap::new(), Some(WSOL_MINT), None);
        let names: Vec<&str> = result.accounts.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["authority", "state"]);

        let expected = expected_pda(
            &[
                b"state".to_vec(),
                bs58::decode(WSOL_MINT).into_vec().unwrap(),
            ],
            TOKEN_PROGRAM,
        );
        assert_eq!(result.accounts[1].address, expected);
    }

    #[test]
    fn derives_a_pda_from_raw_byte_seeds() {
        let raw = vec![1, 2, 255];
        let metas = [meta(
            "config",
            AccountResolution::Pda(PdaConfig {
                program_id: Some(TOKEN_PROGRAM.to_string()),
                seeds: vec![PdaSeed::Bytes(raw.clone())],
            }),
        )];
        let result = resolve_ok(&metas, json!({}), &BTreeMap::new(), None, None);
        assert_eq!(
            result.accounts[0].address,
            expected_pda(&[raw], TOKEN_PROGRAM)
        );
    }

    #[test]
    fn derives_a_pda_from_a_nested_arg_path() {
        let metas = [meta(
            "proposal",
            AccountResolution::Pda(PdaConfig {
                program_id: Some(TOKEN_PROGRAM.to_string()),
                seeds: vec![
                    PdaSeed::Literal("proposal".to_string()),
                    PdaSeed::ArgRef {
                        arg: "args.transactionIndex".to_string(),
                        arg_type: Some("u64".to_string()),
                    },
                ],
            }),
        )];
        let result = resolve_ok(
            &metas,
            json!({ "args": { "transactionIndex": 7 } }),
            &BTreeMap::new(),
            None,
            None,
        );
        let expected = expected_pda(
            &[b"proposal".to_vec(), 7u64.to_le_bytes().to_vec()],
            TOKEN_PROGRAM,
        );
        assert_eq!(result.accounts[0].address, expected);
    }

    #[test]
    fn derives_a_pda_from_helper_only_resolve_inputs() {
        let metas = [meta(
            "proposal",
            AccountResolution::Pda(PdaConfig {
                program_id: Some(TOKEN_PROGRAM.to_string()),
                seeds: vec![
                    PdaSeed::Literal("proposal".to_string()),
                    PdaSeed::ArgRef {
                        arg: "transactionIndex".to_string(),
                        arg_type: Some("u64".to_string()),
                    },
                ],
            }),
        )];
        let resolve = json!({ "transactionIndex": 9 });
        let result = resolve_accounts(
            &metas,
            &Map::new(),
            &BTreeMap::new(),
            resolve.as_object(),
            None,
            None,
        )
        .unwrap();
        let expected = expected_pda(
            &[b"proposal".to_vec(), 9u64.to_le_bytes().to_vec()],
            TOKEN_PROGRAM,
        );
        assert_eq!(result.accounts[0].address, expected);
    }

    #[test]
    fn errors_when_a_pda_seed_argument_is_missing() {
        let metas = [meta(
            "proposal",
            AccountResolution::Pda(PdaConfig {
                program_id: Some(TOKEN_PROGRAM.to_string()),
                seeds: vec![PdaSeed::ArgRef {
                    arg: "transactionIndex".to_string(),
                    arg_type: None,
                }],
            }),
        )];
        let err =
            resolve_accounts(&metas, &Map::new(), &BTreeMap::new(), None, None, None).unwrap_err();
        assert!(
            err.to_string()
                .contains("missing argument: transactionIndex"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn resolves_pdas_that_depend_on_other_pdas_in_topological_order() {
        // "outer" is declared before "inner" but depends on it via accountRef.
        let metas = [
            meta(
                "outer",
                AccountResolution::Pda(PdaConfig {
                    program_id: Some(TOKEN_PROGRAM.to_string()),
                    seeds: vec![
                        PdaSeed::Literal("outer".to_string()),
                        PdaSeed::AccountRef("inner".to_string()),
                    ],
                }),
            ),
            meta(
                "inner",
                AccountResolution::Pda(PdaConfig {
                    program_id: Some(TOKEN_PROGRAM.to_string()),
                    seeds: vec![PdaSeed::Literal("inner".to_string())],
                }),
            ),
        ];
        let result = resolve_ok(&metas, json!({}), &BTreeMap::new(), None, None);
        let names: Vec<&str> = result.accounts.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["outer", "inner"]);

        let inner = expected_pda(&[b"inner".to_vec()], TOKEN_PROGRAM);
        assert_eq!(result.accounts[1].address, inner);
        let outer = expected_pda(
            &[b"outer".to_vec(), bs58::decode(&inner).into_vec().unwrap()],
            TOKEN_PROGRAM,
        );
        assert_eq!(result.accounts[0].address, outer);
    }

    #[test]
    fn rejects_circular_pda_dependencies() {
        let pda_ref = |name: &str, dep: &str| {
            meta(
                name,
                AccountResolution::Pda(PdaConfig {
                    program_id: Some(TOKEN_PROGRAM.to_string()),
                    seeds: vec![PdaSeed::AccountRef(dep.to_string())],
                }),
            )
        };
        let metas = [pda_ref("a", "b"), pda_ref("b", "a")];
        let err =
            resolve_accounts(&metas, &Map::new(), &BTreeMap::new(), None, None, None).unwrap_err();
        assert!(matches!(err, InstructionError::CircularPdaDependency(_)));
    }

    #[test]
    fn reports_missing_required_user_provided_accounts() {
        let metas = [meta("mint", AccountResolution::UserProvided)];
        let result =
            resolve_accounts(&metas, &Map::new(), &BTreeMap::new(), None, None, None).unwrap();
        assert_eq!(result.missing, ["mint"]);
        assert!(result.accounts.is_empty());
    }

    #[test]
    fn errors_when_a_pda_has_no_program_id() {
        let metas = [meta(
            "state",
            AccountResolution::Pda(PdaConfig {
                program_id: None,
                seeds: vec![PdaSeed::Literal("state".to_string())],
            }),
        )];
        let err =
            resolve_accounts(&metas, &Map::new(), &BTreeMap::new(), None, None, None).unwrap_err();
        assert_eq!(err, InstructionError::MissingProgramId("state".to_string()));
    }

    #[test]
    fn substitutes_the_program_id_for_omitted_non_trailing_optional_accounts() {
        let metas = [
            signer("authority"),
            AccountMeta {
                is_optional: true,
                ..meta("referrer", AccountResolution::UserProvided)
            },
            meta("mint", AccountResolution::UserProvided),
        ];
        let result = resolve_ok(
            &metas,
            json!({}),
            &overrides(&[("mint", TOKEN_PROGRAM)]),
            Some(WSOL_MINT),
            Some(SYSTEM_PROGRAM),
        );
        let names: Vec<&str> = result.accounts.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["authority", "referrer", "mint"]);
        assert_eq!(result.accounts[1].address, SYSTEM_PROGRAM);
        assert!(!result.accounts[1].is_signer);
        assert!(!result.accounts[1].is_writable);
    }

    #[test]
    fn drops_omitted_trailing_optional_accounts() {
        let metas = [
            signer("authority"),
            AccountMeta {
                is_optional: true,
                ..meta("referrer", AccountResolution::UserProvided)
            },
        ];
        let result = resolve_ok(
            &metas,
            json!({}),
            &BTreeMap::new(),
            Some(WSOL_MINT),
            Some(SYSTEM_PROGRAM),
        );
        let names: Vec<&str> = result.accounts.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["authority"]);
    }

    #[test]
    fn resolves_provided_optional_accounts_normally() {
        let metas = [
            signer("authority"),
            AccountMeta {
                is_optional: true,
                ..meta("referrer", AccountResolution::UserProvided)
            },
            meta("mint", AccountResolution::UserProvided),
        ];
        let result = resolve_ok(
            &metas,
            json!({}),
            &overrides(&[("referrer", WSOL_MINT), ("mint", TOKEN_PROGRAM)]),
            Some(WSOL_MINT),
            Some(SYSTEM_PROGRAM),
        );
        let addresses: Vec<&str> = result.accounts.iter().map(|a| a.address.as_str()).collect();
        assert_eq!(addresses, [WSOL_MINT, WSOL_MINT, TOKEN_PROGRAM]);
    }
}
