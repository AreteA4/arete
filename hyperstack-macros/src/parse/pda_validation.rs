use std::collections::{HashMap, HashSet};

use syn::Error;

use crate::diagnostic::suggestion_or_available_suffix;

use super::idl::IdlSpec;
use super::pdas::{ParsedSeedKind, PdasBlock, ProgramPdas};

pub struct PdaValidationContext<'a> {
    pub idls: &'a HashMap<String, IdlSpec>,
}

impl<'a> PdaValidationContext<'a> {
    pub fn new(idls: &'a HashMap<String, IdlSpec>) -> Self {
        Self { idls }
    }

    pub fn validate(&self, block: &PdasBlock) -> Result<(), Error> {
        for program in &block.programs {
            self.validate_program(program)?;
        }
        Ok(())
    }

    fn validate_program(&self, program: &ProgramPdas) -> Result<(), Error> {
        let idl = self.idls.get(&program.program_name).ok_or_else(|| {
            let available: Vec<String> = self.idls.keys().cloned().collect();
            Error::new(
                program.program_name_span,
                format!(
                    "unknown program '{}' in pdas! block{}",
                    program.program_name,
                    suggestion_or_available_suffix(
                        &program.program_name,
                        &available,
                        "Available programs",
                    )
                ),
            )
        })?;

        let mut seen_names = HashSet::new();
        for pda in &program.pdas {
            if !seen_names.insert(&pda.name) {
                return Err(Error::new(
                    pda.name_span,
                    format!(
                        "duplicate PDA name '{}' in program '{}'",
                        pda.name, program.program_name
                    ),
                ));
            }

            for seed in &pda.seeds {
                self.validate_seed(seed, idl, &program.program_name)?;
            }
        }

        Ok(())
    }

    fn validate_seed(
        &self,
        seed: &super::pdas::ParsedSeed,
        idl: &IdlSpec,
        program_name: &str,
    ) -> Result<(), Error> {
        match &seed.kind {
            ParsedSeedKind::Literal(_) => Ok(()),

            ParsedSeedKind::Account(account_name) => {
                let account_exists = idl.instructions.iter().any(|ix| {
                    ix.accounts
                        .iter()
                        .any(|acc| &acc.name == account_name || acc.name == *account_name)
                });

                if !account_exists {
                    let available = self.collect_account_names(idl);
                    return Err(Error::new(
                        seed.span,
                        format!(
                            "unknown account '{}' in program '{}'{}",
                            account_name,
                            program_name,
                            suggestion_or_available_suffix(
                                account_name,
                                &available,
                                "Available accounts",
                            )
                        ),
                    ));
                }
                Ok(())
            }

            ParsedSeedKind::Arg { name, arg_type } => {
                let found_arg = idl
                    .instructions
                    .iter()
                    .find_map(|ix| ix.args.iter().find(|arg| &arg.name == name));

                match found_arg {
                    None => {
                        let available = self.collect_arg_names(idl);
                        Err(Error::new(
                            seed.span,
                            format!(
                                "unknown instruction argument '{}' in program '{}'{}",
                                name,
                                program_name,
                                suggestion_or_available_suffix(
                                    name,
                                    &available,
                                    "Available instruction arguments",
                                )
                            ),
                        ))
                    }
                    Some(idl_arg) => {
                        let actual_type = crate::parse::idl::to_rust_type_string(&idl_arg.type_);
                        if arg_type != &actual_type {
                            Err(Error::new(
                                seed.span,
                                format!(
                                    "invalid instruction argument type for '{}'. Expected '{}', found '{}'.",
                                    name, actual_type, arg_type
                                ),
                            ))
                        } else {
                            Ok(())
                        }
                    }
                }
            }
        }
    }

    fn collect_account_names(&self, idl: &IdlSpec) -> Vec<String> {
        let mut names: HashSet<String> = HashSet::new();
        for ix in &idl.instructions {
            for acc in &ix.accounts {
                names.insert(acc.name.clone());
            }
        }
        let mut result: Vec<_> = names.into_iter().collect();
        result.sort();
        result
    }

    fn collect_arg_names(&self, idl: &IdlSpec) -> Vec<String> {
        let mut names: HashSet<String> = HashSet::new();
        for ix in &idl.instructions {
            for arg in &ix.args {
                names.insert(arg.name.clone());
            }
        }
        let mut result: Vec<_> = names.into_iter().collect();
        result.sort();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_idl() -> IdlSpec {
        use crate::parse::idl::*;

        IdlSpec {
            name: Some("test".to_string()),
            address: Some("TestProgram111111111111111111111111111111".to_string()),
            version: Some("0.1.0".to_string()),
            metadata: None,
            accounts: vec![],
            constants: vec![],
            instructions: vec![IdlInstruction {
                name: "create".to_string(),
                discriminator: vec![0; 8],
                discriminant: None,
                docs: vec![],
                accounts: vec![
                    IdlAccountArg {
                        name: "authority".to_string(),
                        is_signer: true,
                        is_mut: false,
                        address: None,
                        pda: None,
                        optional: false,
                        docs: vec![],
                    },
                    IdlAccountArg {
                        name: "miner".to_string(),
                        is_signer: false,
                        is_mut: true,
                        address: None,
                        pda: None,
                        optional: false,
                        docs: vec![],
                    },
                ],
                args: vec![IdlField {
                    name: "roundId".to_string(),
                    type_: IdlType::Simple("u64".to_string()),
                }],
            }],
            types: vec![],
            events: vec![],
            errors: vec![],
        }
    }

    #[test]
    fn test_validate_valid_pda() {
        let idl = make_test_idl();
        let mut idls = HashMap::new();
        idls.insert("test".to_string(), idl);

        let ctx = PdaValidationContext::new(&idls);

        let tokens: proc_macro2::TokenStream = quote::quote! {
            test {
                miner_pda = [literal("miner"), account("authority")];
            }
        };
        let block: PdasBlock = syn::parse2(tokens).unwrap();

        assert!(ctx.validate(&block).is_ok());
    }

    #[test]
    fn test_validate_unknown_program() {
        let idls = HashMap::new();
        let ctx = PdaValidationContext::new(&idls);

        let tokens: proc_macro2::TokenStream = quote::quote! {
            unknown {
                pda = [literal("test")];
            }
        };
        let block: PdasBlock = syn::parse2(tokens).unwrap();

        let result = ctx.validate(&block);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown program"));
    }

    #[test]
    fn test_validate_unknown_account() {
        let idl = make_test_idl();
        let mut idls = HashMap::new();
        idls.insert("test".to_string(), idl);

        let ctx = PdaValidationContext::new(&idls);

        let tokens: proc_macro2::TokenStream = quote::quote! {
            test {
                pda = [literal("test"), account("nonexistent")];
            }
        };
        let block: PdasBlock = syn::parse2(tokens).unwrap();

        let result = ctx.validate(&block);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown account"));
    }

    #[test]
    fn test_validate_arg_type_mismatch() {
        let idl = make_test_idl();
        let mut idls = HashMap::new();
        idls.insert("test".to_string(), idl);

        let ctx = PdaValidationContext::new(&idls);

        let tokens: proc_macro2::TokenStream = quote::quote! {
            test {
                pda = [literal("round"), arg("roundId", u128)];
            }
        };
        let block: PdasBlock = syn::parse2(tokens).unwrap();

        let result = ctx.validate(&block);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("invalid instruction argument type"));
    }

    #[test]
    fn test_validate_duplicate_pda_name() {
        let idl = make_test_idl();
        let mut idls = HashMap::new();
        idls.insert("test".to_string(), idl);

        let ctx = PdaValidationContext::new(&idls);

        let tokens: proc_macro2::TokenStream = quote::quote! {
            test {
                miner = [literal("miner")];
                miner = [literal("other")];
            }
        };
        let block: PdasBlock = syn::parse2(tokens).unwrap();

        let result = ctx.validate(&block);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("duplicate"));
    }
}
