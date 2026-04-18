use proc_macro2::Span;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{braced, bracketed, Ident, LitStr, Result, Token};

use crate::ast::{PdaDefinition, PdaSeedDef};

#[derive(Debug, Clone)]
pub struct PdasBlock {
    pub programs: Vec<ProgramPdas>,
}

#[derive(Debug, Clone)]
pub struct ProgramPdas {
    pub program_name: String,
    pub program_name_span: Span,
    pub pdas: Vec<ParsedPda>,
}

#[derive(Debug, Clone)]
pub struct ParsedPda {
    pub name: String,
    pub name_span: Span,
    pub seeds: Vec<ParsedSeed>,
}

#[derive(Debug, Clone)]
pub struct ParsedSeed {
    pub kind: ParsedSeedKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ParsedSeedKind {
    Literal(String),
    Account(String),
    Arg { name: String, arg_type: String },
}

impl Parse for PdasBlock {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut programs = Vec::new();

        while !input.is_empty() {
            let program_name: Ident = input.parse()?;
            let content;
            braced!(content in input);

            let mut pdas = Vec::new();
            while !content.is_empty() {
                let pda = parse_pda_definition(&content)?;
                pdas.push(pda);
            }

            programs.push(ProgramPdas {
                program_name: program_name.to_string(),
                program_name_span: program_name.span(),
                pdas,
            });
        }

        Ok(PdasBlock { programs })
    }
}

fn parse_pda_definition(input: ParseStream) -> Result<ParsedPda> {
    let name: Ident = input.parse()?;
    let name_span = name.span();
    input.parse::<Token![=]>()?;

    let seeds_content;
    bracketed!(seeds_content in input);

    let seeds_punctuated: Punctuated<ParsedSeed, Token![,]> =
        seeds_content.parse_terminated(parse_seed, Token![,])?;
    let seeds: Vec<ParsedSeed> = seeds_punctuated.into_iter().collect();

    input.parse::<Token![;]>()?;

    Ok(ParsedPda {
        name: name.to_string(),
        name_span,
        seeds,
    })
}

fn parse_seed(input: ParseStream) -> Result<ParsedSeed> {
    let fn_name: Ident = input.parse()?;
    let span = fn_name.span();

    let args_content;
    syn::parenthesized!(args_content in input);

    let kind = match fn_name.to_string().as_str() {
        "literal" => {
            let lit: LitStr = args_content.parse()?;
            ParsedSeedKind::Literal(lit.value())
        }
        "account" => {
            let name: LitStr = args_content.parse()?;
            ParsedSeedKind::Account(name.value())
        }
        "arg" => {
            let name: LitStr = args_content.parse()?;
            args_content.parse::<Token![,]>()?;
            let arg_type: Ident = args_content.parse()?;
            ParsedSeedKind::Arg {
                name: name.value(),
                arg_type: arg_type.to_string(),
            }
        }
        other => {
            return Err(syn::Error::new(
                span,
                format!(
                    "unknown seed type '{}'. Expected: literal, account, or arg",
                    other
                ),
            ));
        }
    };

    Ok(ParsedSeed { kind, span })
}

impl ParsedPda {
    pub fn to_pda_definition(&self) -> PdaDefinition {
        PdaDefinition {
            name: self.name.clone(),
            seeds: self
                .seeds
                .iter()
                .map(|s| match &s.kind {
                    ParsedSeedKind::Literal(v) => PdaSeedDef::Literal { value: v.clone() },
                    ParsedSeedKind::Account(name) => PdaSeedDef::AccountRef {
                        account_name: name.clone(),
                    },
                    ParsedSeedKind::Arg { name, arg_type } => PdaSeedDef::ArgRef {
                        arg_name: name.clone(),
                        arg_type: Some(arg_type.clone()),
                    },
                })
                .collect(),
            program_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_pda() {
        let tokens: proc_macro2::TokenStream = quote::quote! {
            ore {
                miner = [literal("miner"), account("authority")];
                treasury = [literal("treasury")];
            }
        };

        let block: PdasBlock = syn::parse2(tokens).unwrap();
        assert_eq!(block.programs.len(), 1);
        assert_eq!(block.programs[0].program_name, "ore");
        assert_eq!(block.programs[0].pdas.len(), 2);
        assert_eq!(block.programs[0].pdas[0].name, "miner");
        assert_eq!(block.programs[0].pdas[0].seeds.len(), 2);
    }

    #[test]
    fn test_parse_multiple_programs() {
        let tokens: proc_macro2::TokenStream = quote::quote! {
            ore {
                miner = [literal("miner"), account("authority")];
            }
            entropy {
                round = [literal("round"), arg("roundId", u64)];
            }
        };

        let block: PdasBlock = syn::parse2(tokens).unwrap();
        assert_eq!(block.programs.len(), 2);
        assert_eq!(block.programs[0].program_name, "ore");
        assert_eq!(block.programs[1].program_name, "entropy");
    }
}
