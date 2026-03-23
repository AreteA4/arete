//! Search utilities for IDL specs with fuzzy matching

use crate::error::IdlSearchError;
use crate::types::IdlSpec;
use crate::types::{IdlAccount, IdlInstruction, IdlTypeDef};
use strsim::levenshtein;

/// A fuzzy match suggestion with candidate name and edit distance.
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub candidate: String,
    pub distance: usize,
}

/// Which section of the IDL a search result came from.
#[derive(Debug, Clone)]
pub enum IdlSection {
    Instruction,
    Account,
    Type,
    Error,
    Event,
    Constant,
}

/// How a search result was matched.
#[derive(Debug, Clone)]
pub enum MatchType {
    Exact,
    CaseInsensitive,
    Contains,
    Fuzzy(usize),
}

/// A single search result from `search_idl`.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub name: String,
    pub section: IdlSection,
    pub match_type: MatchType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstructionFieldKind {
    Account,
    Arg,
}

#[derive(Debug, Clone, Copy)]
pub struct InstructionFieldLookup<'a> {
    pub instruction: &'a IdlInstruction,
    pub kind: InstructionFieldKind,
}

fn build_not_found_error(input: &str, section: String, available: Vec<String>) -> IdlSearchError {
    let candidate_refs: Vec<&str> = available.iter().map(String::as_str).collect();
    let suggestions = suggest_similar(input, &candidate_refs, 3);
    IdlSearchError::NotFound {
        input: input.to_string(),
        section,
        suggestions,
        available,
    }
}

pub fn lookup_instruction<'a>(
    idl: &'a IdlSpec,
    instruction_name: &str,
) -> Result<&'a IdlInstruction, IdlSearchError> {
    let available: Vec<String> = idl.instructions.iter().map(|ix| ix.name.clone()).collect();
    idl.instructions
        .iter()
        .find(|ix| ix.name == instruction_name)
        .ok_or_else(|| {
            build_not_found_error(instruction_name, "instructions".to_string(), available)
        })
}

pub fn lookup_account<'a>(
    idl: &'a IdlSpec,
    account_name: &str,
) -> Result<&'a IdlAccount, IdlSearchError> {
    let available: Vec<String> = idl
        .accounts
        .iter()
        .map(|account| account.name.clone())
        .collect();
    idl.accounts
        .iter()
        .find(|account| account.name == account_name)
        .ok_or_else(|| build_not_found_error(account_name, "accounts".to_string(), available))
}

pub fn lookup_type<'a>(
    idl: &'a IdlSpec,
    type_name: &str,
) -> Result<&'a IdlTypeDef, IdlSearchError> {
    let available: Vec<String> = idl.types.iter().map(|ty| ty.name.clone()).collect();
    idl.types
        .iter()
        .find(|ty| ty.name == type_name)
        .ok_or_else(|| build_not_found_error(type_name, "types".to_string(), available))
}

pub fn lookup_instruction_field<'a>(
    idl: &'a IdlSpec,
    instruction_name: &str,
    field_name: &str,
) -> Result<InstructionFieldLookup<'a>, IdlSearchError> {
    let instruction = lookup_instruction(idl, instruction_name)?;
    if instruction
        .accounts
        .iter()
        .any(|account| account.name == field_name)
    {
        return Ok(InstructionFieldLookup {
            instruction,
            kind: InstructionFieldKind::Account,
        });
    }

    if instruction.args.iter().any(|arg| arg.name == field_name) {
        return Ok(InstructionFieldLookup {
            instruction,
            kind: InstructionFieldKind::Arg,
        });
    }

    let mut available: Vec<String> = instruction
        .accounts
        .iter()
        .map(|acc| acc.name.clone())
        .collect();
    available.extend(instruction.args.iter().map(|arg| arg.name.clone()));
    Err(build_not_found_error(
        field_name,
        format!("instruction fields for '{}'", instruction.name),
        available,
    ))
}

/// Suggest similar names from a list of candidates using fuzzy matching.
///
/// Returns candidates sorted by edit distance (closest first).
/// Exact matches are excluded. Case-insensitive matches get distance 0,
/// substring matches get distance 1, and Levenshtein matches use their
/// actual edit distance.
pub fn suggest_similar(name: &str, candidates: &[&str], max_distance: usize) -> Vec<Suggestion> {
    let name_lower = name.to_lowercase();
    let mut suggestions: Vec<Suggestion> = candidates
        .iter()
        .filter_map(|&candidate| {
            // Skip exact matches
            if candidate == name {
                return None;
            }
            let candidate_lower = candidate.to_lowercase();
            // Case-insensitive match
            if candidate_lower == name_lower {
                return Some(Suggestion {
                    candidate: candidate.to_string(),
                    distance: 0,
                });
            }
            // Substring match
            if candidate_lower.contains(&name_lower) || name_lower.contains(&candidate_lower) {
                return Some(Suggestion {
                    candidate: candidate.to_string(),
                    distance: 1,
                });
            }
            // Levenshtein distance
            let dist = levenshtein(name, candidate);
            if dist <= max_distance {
                Some(Suggestion {
                    candidate: candidate.to_string(),
                    distance: dist,
                })
            } else {
                None
            }
        })
        .collect();
    suggestions.sort_by_key(|s| s.distance);
    suggestions
}

/// Search across all sections of an IDL spec for names matching the query.
///
/// Performs case-insensitive substring matching against instruction names,
/// account names, type names, error names, event names, and constant names.
pub fn search_idl(idl: &IdlSpec, query: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let q = query.to_lowercase();

    for ix in &idl.instructions {
        if ix.name.to_lowercase().contains(&q) {
            results.push(SearchResult {
                name: ix.name.clone(),
                section: IdlSection::Instruction,
                match_type: MatchType::Contains,
            });
        }
    }
    for acc in &idl.accounts {
        if acc.name.to_lowercase().contains(&q) {
            results.push(SearchResult {
                name: acc.name.clone(),
                section: IdlSection::Account,
                match_type: MatchType::Contains,
            });
        }
    }
    for ty in &idl.types {
        if ty.name.to_lowercase().contains(&q) {
            results.push(SearchResult {
                name: ty.name.clone(),
                section: IdlSection::Type,
                match_type: MatchType::Contains,
            });
        }
    }
    for err in &idl.errors {
        if err.name.to_lowercase().contains(&q) {
            results.push(SearchResult {
                name: err.name.clone(),
                section: IdlSection::Error,
                match_type: MatchType::Contains,
            });
        }
    }
    for ev in &idl.events {
        if ev.name.to_lowercase().contains(&q) {
            results.push(SearchResult {
                name: ev.name.clone(),
                section: IdlSection::Event,
                match_type: MatchType::Contains,
            });
        }
    }
    for c in &idl.constants {
        if c.name.to_lowercase().contains(&q) {
            results.push(SearchResult {
                name: c.name.clone(),
                section: IdlSection::Constant,
                match_type: MatchType::Contains,
            });
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzzy_suggestions() {
        let candidates = ["initialize", "close", "deposit"];
        let suggestions = suggest_similar("initlize", &candidates, 3);
        assert!(!suggestions.is_empty());
        assert_eq!(suggestions[0].candidate, "initialize");
    }

    #[test]
    fn test_fuzzy_case_insensitive() {
        let candidates = ["Initialize", "close"];
        let suggestions = suggest_similar("initialize", &candidates, 3);
        assert!(!suggestions.is_empty());
        assert_eq!(suggestions[0].candidate, "Initialize");
        assert_eq!(suggestions[0].distance, 0);
    }

    #[test]
    fn test_fuzzy_no_exact_match() {
        let candidates = ["initialize"];
        let suggestions = suggest_similar("initialize", &candidates, 3);
        assert!(suggestions.is_empty(), "exact matches should be excluded");
    }

    #[test]
    fn test_fuzzy_substring() {
        let candidates = ["swap_exact_in", "close"];
        let suggestions = suggest_similar("swap", &candidates, 3);
        assert!(!suggestions.is_empty());
        assert_eq!(suggestions[0].candidate, "swap_exact_in");
    }

    #[test]
    fn test_search_idl() {
        use crate::parse::parse_idl_file;
        use std::path::PathBuf;
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meteora_dlmm.json");
        let idl = parse_idl_file(&path).expect("should parse");
        let results = search_idl(&idl, "swap");
        assert!(!results.is_empty(), "should find results for 'swap'");
    }

    #[test]
    fn test_lookup_instruction_with_suggestion() {
        use crate::parse::parse_idl_file;
        use std::path::PathBuf;

        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pump.json");
        let idl = parse_idl_file(&path).expect("should parse");

        let error = lookup_instruction(&idl, "initialise").expect_err("lookup should fail");
        match error {
            IdlSearchError::NotFound { suggestions, .. } => {
                assert_eq!(suggestions[0].candidate, "initialize");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn test_lookup_instruction_field_with_suggestion() {
        use crate::parse::parse_idl_file;
        use std::path::PathBuf;

        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pump.json");
        let idl = parse_idl_file(&path).expect("should parse");

        let error = lookup_instruction_field(&idl, "buy", "usr").expect_err("lookup should fail");
        match error {
            IdlSearchError::NotFound { suggestions, .. } => {
                assert_eq!(suggestions[0].candidate, "user");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn test_lookup_account_success() {
        use crate::parse::parse_idl_file;
        use std::path::PathBuf;

        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pump.json");
        let idl = parse_idl_file(&path).expect("should parse");

        let account = lookup_account(&idl, "BondingCurve").expect("account should exist");
        assert_eq!(account.name, "BondingCurve");
    }

    #[test]
    fn test_lookup_instruction_case_mismatch_gets_suggestion() {
        use crate::parse::parse_idl_file;
        use std::path::PathBuf;

        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pump.json");
        let idl = parse_idl_file(&path).expect("should parse");

        let error = lookup_instruction(&idl, "Buy").expect_err("lookup should fail");
        match error {
            IdlSearchError::NotFound { suggestions, .. } => {
                assert_eq!(suggestions[0].candidate, "buy");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn test_lookup_account_case_mismatch_gets_suggestion() {
        use crate::parse::parse_idl_file;
        use std::path::PathBuf;

        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pump.json");
        let idl = parse_idl_file(&path).expect("should parse");

        let error = lookup_account(&idl, "bondingCurve").expect_err("lookup should fail");
        match error {
            IdlSearchError::NotFound { suggestions, .. } => {
                assert_eq!(suggestions[0].candidate, "BondingCurve");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }
}
