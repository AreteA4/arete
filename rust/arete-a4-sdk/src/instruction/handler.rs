//! Data-driven instruction handler.
//!
//! Generated stack code produces [`InstructionHandler`] values; `build`
//! serializes args via the schema-driven serializer and resolves accounts from
//! a merged params object, so no imperative per-instruction code is required.

use std::collections::{BTreeMap, HashSet};
use std::str::FromStr;

use serde_json::{Map, Value};
use solana_pubkey::Pubkey;

use super::resolver::resolve_accounts;
use super::serializer::serialize_instruction_data;
use super::types::{
    json_kind, AccountMeta, ArgSchema, BuildOptions, BuiltAccountMeta, BuiltInstruction,
    ErrorMetadata, InstructionError,
};

/// Instruction definition consumed by the builder: discriminator, ordered
/// account metadata, argument schema, and IDL error definitions.
///
/// Building is pure — no network access. The resulting [`BuiltInstruction`]
/// is the unit of composition for transaction assembly and batching.
#[derive(Debug, Clone)]
pub struct InstructionHandler {
    /// Program ID (base58). Also the fallback program for PDA derivation.
    pub program_id: String,
    /// Instruction discriminator bytes (8 for Anchor, 1 for Steel, etc.).
    pub discriminator: Vec<u8>,
    /// Ordered account metadata.
    pub accounts: Vec<AccountMeta>,
    /// Ordered argument schema for Borsh serialization.
    pub args: Vec<ArgSchema>,
    /// Error definitions from the IDL.
    pub errors: Vec<ErrorMetadata>,
}

impl InstructionHandler {
    /// Builds the instruction from a merged params object (args plus account
    /// address overrides) with default options.
    pub fn build(&self, params: Value) -> Result<BuiltInstruction, InstructionError> {
        self.build_with(params, &BuildOptions::default())
    }

    /// Builds the instruction from a merged params object and explicit options.
    ///
    /// Params keys matching a declared argument name are serialized args; keys
    /// matching a declared account name (with a string value) are account
    /// address overrides — including signer slots, allowing explicit signer
    /// addresses to override the payer fallback. A `resolve` key carries
    /// helper-only PDA seed inputs. Anything else errors — a typo'd key
    /// silently dropped here would otherwise change the built instruction.
    /// `options.accounts` remains an unvalidated escape hatch that wins over
    /// param-derived overrides.
    pub fn build_with(
        &self,
        params: Value,
        options: &BuildOptions,
    ) -> Result<BuiltInstruction, InstructionError> {
        let Value::Object(params) = params else {
            return Err(InstructionError::InvalidValue {
                context: "params".to_string(),
                message: format!("expected a JSON object, got {}", json_kind(&params)),
            });
        };

        let (args, mut overrides, resolve) = self.split_params(params)?;
        for (name, address) in &options.accounts {
            overrides.insert(name.clone(), address.clone());
        }

        let program_id = (!self.program_id.is_empty()).then_some(self.program_id.as_str());
        let resolution = resolve_accounts(
            &self.accounts,
            &args,
            &overrides,
            resolve.as_ref(),
            options.payer.as_deref(),
            program_id,
        )?;
        if !resolution.missing.is_empty() {
            return Err(InstructionError::MissingAccounts(resolution.missing));
        }

        let data = serialize_instruction_data(&self.discriminator, &args, &self.args)?;
        let mut accounts = resolution
            .accounts
            .iter()
            .map(|account| {
                Ok(BuiltAccountMeta {
                    pubkey: parse_pubkey(&account.address)?,
                    is_signer: account.is_signer,
                    is_writable: account.is_writable,
                })
            })
            .collect::<Result<Vec<_>, InstructionError>>()?;
        accounts.extend(options.remaining_accounts.iter().cloned());

        Ok(BuiltInstruction {
            program_id: parse_pubkey(&self.program_id)?,
            accounts,
            data,
        })
    }

    /// Looks up an IDL error definition by code.
    pub fn error_for_code(&self, code: u32) -> Option<&ErrorMetadata> {
        self.errors.iter().find(|error| error.code == code)
    }

    /// Splits a merged params object into serialized args, account-address
    /// overrides, and the helper-only `resolve` map.
    #[allow(clippy::type_complexity)]
    fn split_params(
        &self,
        params: Map<String, Value>,
    ) -> Result<
        (
            Map<String, Value>,
            BTreeMap<String, String>,
            Option<Map<String, Value>>,
        ),
        InstructionError,
    > {
        let arg_names: HashSet<&str> = self.args.iter().map(|arg| arg.name.as_str()).collect();
        let account_names: HashSet<&str> = self
            .accounts
            .iter()
            .map(|account| account.name.as_str())
            .collect();

        let mut args = Map::new();
        let mut overrides = BTreeMap::new();
        let mut resolve = None;
        for (key, value) in params {
            if arg_names.contains(key.as_str()) {
                args.insert(key, value);
            } else if key == "resolve" && !account_names.contains("resolve") {
                let Value::Object(map) = value else {
                    return Err(InstructionError::InvalidValue {
                        context: "resolve".to_string(),
                        message: "Parameter \"resolve\" must be an object when provided"
                            .to_string(),
                    });
                };
                resolve = Some(map);
            } else if account_names.contains(key.as_str()) {
                let Value::String(address) = value else {
                    // Non-string values are not valid account addresses.
                    return Err(InstructionError::InvalidValue {
                        context: key.clone(),
                        message: format!(
                            "Parameter \"{key}\" is not a known argument and is not a base58 account address"
                        ),
                    });
                };
                overrides.insert(key, address);
            } else {
                return Err(InstructionError::UnknownParameter {
                    name: key,
                    args: self.args.iter().map(|arg| arg.name.clone()).collect(),
                    accounts: self
                        .accounts
                        .iter()
                        .map(|account| account.name.clone())
                        .collect(),
                });
            }
        }
        Ok((args, overrides, resolve))
    }
}

fn parse_pubkey(address: &str) -> Result<Pubkey, InstructionError> {
    Pubkey::from_str(address).map_err(|_| InstructionError::InvalidPubkey(address.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruction::{
        derive_program_address, AccountResolution, ArgType, PdaConfig, PdaSeed,
    };
    use serde_json::json;

    const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
    const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
    const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

    fn make_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: TOKEN_PROGRAM.to_string(),
            discriminator: vec![1],
            accounts: vec![
                AccountMeta {
                    name: "authority".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "mint".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "state".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig {
                        program_id: None,
                        seeds: vec![
                            PdaSeed::Literal("state".to_string()),
                            PdaSeed::AccountRef("authority".to_string()),
                        ],
                    }),
                    is_optional: false,
                },
            ],
            args: vec![ArgSchema {
                name: "amount".to_string(),
                ty: ArgType::U64,
            }],
            errors: vec![ErrorMetadata {
                code: 6000,
                name: "Boom".to_string(),
                msg: "boom".to_string(),
            }],
        }
    }

    fn payer_options() -> BuildOptions {
        BuildOptions {
            payer: Some(WSOL_MINT.to_string()),
            ..BuildOptions::default()
        }
    }

    fn state_pda(authority: &str) -> Pubkey {
        let seeds = vec![
            b"state".to_vec(),
            bs58::decode(authority).into_vec().unwrap(),
        ];
        derive_program_address(&seeds, TOKEN_PROGRAM).unwrap().0
    }

    #[test]
    fn splits_merged_params_into_args_and_account_overrides() {
        let built = make_handler()
            .build_with(
                json!({ "amount": 100, "mint": SYSTEM_PROGRAM }),
                &payer_options(),
            )
            .unwrap();

        assert_eq!(built.program_id, Pubkey::from_str(TOKEN_PROGRAM).unwrap());
        let pubkeys: Vec<Pubkey> = built.accounts.iter().map(|a| a.pubkey).collect();
        assert_eq!(
            pubkeys,
            vec![
                Pubkey::from_str(WSOL_MINT).unwrap(),      // authority (signer)
                Pubkey::from_str(SYSTEM_PROGRAM).unwrap(), // mint (user-provided)
                state_pda(WSOL_MINT),                      // state (PDA from authority)
            ]
        );
        // discriminator [1] + u64 100 little-endian.
        assert_eq!(built.data, vec![1, 100, 0, 0, 0, 0, 0, 0, 0]);
        assert!(built.accounts[0].is_signer);
        assert!(built.accounts[0].is_writable);
        assert!(!built.accounts[1].is_signer);
    }

    #[test]
    fn lets_merged_params_override_a_signer_slot_explicitly() {
        let built = make_handler()
            .build_with(
                json!({ "amount": 7, "authority": TOKEN_PROGRAM, "mint": SYSTEM_PROGRAM }),
                &payer_options(),
            )
            .unwrap();
        let pubkeys: Vec<Pubkey> = built.accounts.iter().map(|a| a.pubkey).collect();
        assert_eq!(
            pubkeys,
            vec![
                Pubkey::from_str(TOKEN_PROGRAM).unwrap(),
                Pubkey::from_str(SYSTEM_PROGRAM).unwrap(),
                state_pda(TOKEN_PROGRAM),
            ]
        );
    }

    #[test]
    fn options_account_overrides_win_over_params() {
        let options = BuildOptions {
            payer: Some(WSOL_MINT.to_string()),
            accounts: [("mint".to_string(), TOKEN_PROGRAM.to_string())].into(),
            ..BuildOptions::default()
        };
        let built = make_handler()
            .build_with(json!({ "amount": 1, "mint": SYSTEM_PROGRAM }), &options)
            .unwrap();
        assert_eq!(
            built.accounts[1].pubkey,
            Pubkey::from_str(TOKEN_PROGRAM).unwrap()
        );
    }

    #[test]
    fn rejects_non_string_account_params() {
        let err = make_handler()
            .build_with(json!({ "amount": 1, "mint": 42 }), &payer_options())
            .unwrap_err();
        assert!(
            err.to_string().contains("not a known argument"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn accepts_helper_only_resolve_inputs_for_pda_derivation() {
        let handler = InstructionHandler {
            program_id: TOKEN_PROGRAM.to_string(),
            discriminator: vec![2],
            accounts: vec![
                AccountMeta {
                    name: "authority".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "proposal".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig {
                        program_id: None,
                        seeds: vec![
                            PdaSeed::Literal("proposal".to_string()),
                            PdaSeed::ArgRef {
                                arg: "transactionIndex".to_string(),
                                arg_type: Some("u64".to_string()),
                            },
                        ],
                    }),
                    is_optional: false,
                },
            ],
            args: vec![ArgSchema {
                name: "amount".to_string(),
                ty: ArgType::U64,
            }],
            errors: Vec::new(),
        };

        let built = handler
            .build_with(
                json!({ "amount": 5, "resolve": { "transactionIndex": 11 } }),
                &payer_options(),
            )
            .unwrap();
        let expected = derive_program_address(
            &[b"proposal".to_vec(), 11u64.to_le_bytes().to_vec()],
            TOKEN_PROGRAM,
        )
        .unwrap()
        .0;
        let pubkeys: Vec<Pubkey> = built.accounts.iter().map(|a| a.pubkey).collect();
        assert_eq!(
            pubkeys,
            vec![Pubkey::from_str(WSOL_MINT).unwrap(), expected]
        );
        assert_eq!(built.data, vec![2, 5, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn rejects_non_object_resolve_inputs() {
        for resolve in [json!(1), json!(null), json!([1])] {
            let err = make_handler()
                .build_with(
                    json!({ "amount": 1, "mint": SYSTEM_PROGRAM, "resolve": resolve }),
                    &payer_options(),
                )
                .unwrap_err();
            assert!(
                err.to_string().contains("resolve"),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn rejects_unknown_parameter_names() {
        let err = make_handler()
            .build_with(
                json!({ "amount": 1, "mint": SYSTEM_PROGRAM, "mnit": TOKEN_PROGRAM }),
                &payer_options(),
            )
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Unknown parameter \"mnit\". Expected one of args [amount] \
             or accounts [authority, mint, state]"
        );
    }

    #[test]
    fn rejects_missing_required_args_instead_of_encoding_zeros() {
        let err = make_handler()
            .build_with(json!({ "mint": SYSTEM_PROGRAM }), &payer_options())
            .unwrap_err();
        assert_eq!(
            err,
            InstructionError::MissingArgument {
                name: "amount".to_string()
            }
        );
    }

    #[test]
    fn reports_missing_required_accounts() {
        let err = make_handler()
            .build_with(json!({ "amount": 1 }), &payer_options())
            .unwrap_err();
        assert_eq!(
            err,
            InstructionError::MissingAccounts(vec!["mint".to_string()])
        );
        assert_eq!(err.to_string(), "Missing required accounts: mint");
    }

    #[test]
    fn rejects_non_object_params() {
        let err = make_handler().build(json!([1, 2])).unwrap_err();
        assert!(
            err.to_string().contains("expected a JSON object"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn appends_remaining_accounts_after_the_declared_accounts() {
        let extra = BuiltAccountMeta {
            pubkey: Pubkey::from_str(TOKEN_PROGRAM).unwrap(),
            is_signer: false,
            is_writable: true,
        };
        let options = BuildOptions {
            payer: Some(WSOL_MINT.to_string()),
            remaining_accounts: vec![extra.clone()],
            ..BuildOptions::default()
        };
        let built = make_handler()
            .build_with(json!({ "amount": 1, "mint": SYSTEM_PROGRAM }), &options)
            .unwrap();
        assert_eq!(built.accounts.len(), 4); // 3 declared + 1 remaining
        assert_eq!(built.accounts[3], extra);
    }

    #[test]
    fn looks_up_idl_errors_by_code() {
        let handler = make_handler();
        assert_eq!(
            handler.error_for_code(6000).map(|e| e.name.as_str()),
            Some("Boom")
        );
        assert_eq!(handler.error_for_code(6001), None);
    }
}
