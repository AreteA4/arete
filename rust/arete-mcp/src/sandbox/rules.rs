//! Rules v3: the document a sandbox's owner signs to say which programs the
//! sandbox wallet may use and where funds may go.
//!
//! ```json
//! {
//!   "version": 3,
//!   "programs": [{
//!     "programId": "<base58>", "name": "Pump.fun", "enabled": true,
//!     "instructions": ["buy", "sell"],
//!     "spendLimit": { "lamports": "1000000000", "windowSeconds": 86400 }
//!   }],
//!   "transferAllowlist": ["<base58>"]
//! }
//! ```
//!
//! `instructions` is optional (absent: the registry's safe set for the
//! program) and so is `spendLimit` (absent: no per-program limit). Funds may
//! leave the enabled programs only to the owner's wallet (always allowed),
//! an address in `transferAllowlist`, or the sandbox's own accounts.
//!
//! [`validate`] checks a document locally, before the owner signs it. The
//! Arete API checks it again: passing here does not mean the API will accept
//! every program (it must also be one the registry supports).

use std::collections::HashSet;

use serde::Serialize;
use serde_json::{Map, Value};

pub const RULES_VERSION: u64 = 3;

const ROOT_KEYS: &[&str] = &["version", "programs", "transferAllowlist"];
const PROGRAM_KEYS: &[&str] = &["programId", "name", "enabled", "instructions", "spendLimit"];
const SPEND_LIMIT_KEYS: &[&str] = &["lamports", "windowSeconds"];

/// One problem, at a JSON path such as `programs[1].spendLimit.lamports`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RulesIssue {
    pub path: String,
    pub message: String,
}

/// What [`validate`] found. `valid` is false when there is any error;
/// warnings are worth reading but do not stop the document being signed.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RulesReport {
    pub valid: bool,
    pub errors: Vec<RulesIssue>,
    pub warnings: Vec<RulesIssue>,
    /// The enabled programs, by name, when the document could be read.
    #[serde(rename = "enabledPrograms")]
    pub enabled_programs: Vec<String>,
}

struct Checker {
    errors: Vec<RulesIssue>,
    warnings: Vec<RulesIssue>,
}

impl Checker {
    fn error(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.errors.push(RulesIssue {
            path: path.into(),
            message: message.into(),
        });
    }

    fn warn(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(RulesIssue {
            path: path.into(),
            message: message.into(),
        });
    }

    /// Keys outside `known` are errors: a misspelt key (`spend_limit`,
    /// `transferAllowList`) would otherwise be signed and silently ignored.
    fn known_keys(&mut self, path: &str, object: &Map<String, Value>, known: &[&str]) {
        for key in object.keys() {
            if known.contains(&key.as_str()) {
                continue;
            }
            let suggestion = known
                .iter()
                .find(|candidate| normalize(candidate) == normalize(key))
                .map(|candidate| format!(" (did you mean `{candidate}`?)"))
                .unwrap_or_default();
            self.error(
                join(path, key),
                format!(
                    "unknown key{suggestion}; allowed here: {}",
                    known.join(", ")
                ),
            );
        }
    }
}

fn normalize(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

/// A base58 Solana public key (32 bytes).
pub fn is_pubkey(value: &str) -> bool {
    bs58::decode(value)
        .into_vec()
        .is_ok_and(|bytes| bytes.len() == 32)
}

/// A u64 given as a JSON integer or a decimal string, as the Arete API takes
/// amounts.
fn as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            text.parse().ok()
        }
        _ => None,
    }
}

/// Check a Rules v3 document.
pub fn validate(document: &Value) -> RulesReport {
    let mut check = Checker {
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    let mut enabled_programs = Vec::new();
    let Some(root) = document.as_object() else {
        check.error("", "the rules must be a JSON object");
        return finish(check, enabled_programs);
    };

    match root.get("version") {
        Some(version) if version.as_u64() == Some(RULES_VERSION) => {}
        Some(version) if matches!(version.as_u64(), Some(1 | 2)) => check.error(
            "version",
            format!(
                "version {version} is an earlier rules format; rebuild the rules as version 3 \
                 (programs with toggles and optional spend limits, plus transferAllowlist)"
            ),
        ),
        Some(version) => check.error("version", format!("must be {RULES_VERSION}, not {version}")),
        None => check.error("version", format!("missing; must be {RULES_VERSION}")),
    }
    check.known_keys("", root, ROOT_KEYS);

    match root.get("programs") {
        Some(Value::Array(programs)) => {
            let mut seen = HashSet::new();
            for (index, program) in programs.iter().enumerate() {
                let path = format!("programs[{index}]");
                if let Some(name) = check_program(&mut check, &path, program, &mut seen) {
                    enabled_programs.push(name);
                }
            }
            if programs.is_empty() {
                check.warn(
                    "programs",
                    "no programs: the sandbox can only hold, wrap and withdraw funds",
                );
            } else if enabled_programs.is_empty() {
                check.warn("programs", "every program is turned off");
            }
        }
        Some(_) => check.error("programs", "must be an array"),
        None => check.error(
            "programs",
            "missing; list the programs (an empty list is allowed)",
        ),
    }

    match root.get("transferAllowlist") {
        None | Some(Value::Null) => {}
        Some(Value::Array(addresses)) => {
            let mut seen = HashSet::new();
            for (index, address) in addresses.iter().enumerate() {
                let path = format!("transferAllowlist[{index}]");
                match address.as_str() {
                    Some(address) if is_pubkey(address) => {
                        if !seen.insert(address) {
                            check.warn(path, format!("{address} is listed twice"));
                        }
                    }
                    Some(address) => {
                        check.error(path, format!("{address:?} is not a base58 Solana address"))
                    }
                    None => check.error(path, "must be a base58 address string"),
                }
            }
        }
        Some(_) => check.error("transferAllowlist", "must be an array of addresses"),
    }

    finish(check, enabled_programs)
}

/// Check one `programs[]` entry; its name when it is enabled.
fn check_program<'a>(
    check: &mut Checker,
    path: &str,
    program: &'a Value,
    seen: &mut HashSet<&'a str>,
) -> Option<String> {
    let Some(object) = program.as_object() else {
        check.error(path, "must be an object");
        return None;
    };
    check.known_keys(path, object, PROGRAM_KEYS);

    match object.get("programId").and_then(Value::as_str) {
        Some(id) if is_pubkey(id) => {
            if !seen.insert(id) {
                check.error(
                    join(path, "programId"),
                    format!("{id} is listed more than once"),
                );
            }
        }
        Some(id) => check.error(
            join(path, "programId"),
            format!("{id:?} is not a base58 program id"),
        ),
        None => check.error(join(path, "programId"), "missing; the program's address"),
    }
    let name = match object.get("name").and_then(Value::as_str).map(str::trim) {
        Some(name) if !name.is_empty() => Some(name.to_string()),
        _ => {
            check.error(
                join(path, "name"),
                "missing; a display name for the program",
            );
            None
        }
    };
    let enabled = match object.get("enabled") {
        Some(Value::Bool(enabled)) => *enabled,
        Some(_) => {
            check.error(join(path, "enabled"), "must be true or false");
            false
        }
        None => {
            check.error(join(path, "enabled"), "missing; true or false");
            false
        }
    };

    match object.get("instructions") {
        None | Some(Value::Null) => {}
        Some(Value::Array(instructions)) => {
            let mut names = HashSet::new();
            for (index, instruction) in instructions.iter().enumerate() {
                let at = format!("{}[{index}]", join(path, "instructions"));
                match instruction.as_str().map(str::trim) {
                    Some(name) if !name.is_empty() => {
                        if !names.insert(name) {
                            check.warn(at, format!("`{name}` is listed twice"));
                        }
                    }
                    _ => check.error(at, "must be an instruction name"),
                }
            }
            if instructions.is_empty() && enabled {
                check.warn(
                    join(path, "instructions"),
                    "an empty list allows no instructions; leave it out to use the registry's safe set",
                );
            }
        }
        Some(_) => check.error(
            join(path, "instructions"),
            "must be a list of instruction names (or left out for the registry's safe set)",
        ),
    }

    match object.get("spendLimit") {
        None | Some(Value::Null) => {}
        Some(Value::Object(limit)) => {
            let at = join(path, "spendLimit");
            check.known_keys(&at, limit, SPEND_LIMIT_KEYS);
            match limit.get("lamports").map(as_u64) {
                Some(Some(0)) => check.error(
                    join(&at, "lamports"),
                    "must be above 0 (turn the program off to stop it spending)",
                ),
                Some(Some(_)) => {}
                Some(None) => check.error(
                    join(&at, "lamports"),
                    "must be a whole number of lamports (a JSON integer or a decimal string)",
                ),
                None => check.error(join(&at, "lamports"), "missing"),
            }
            match limit.get("windowSeconds") {
                Some(value) if value.as_u64().is_some_and(|seconds| seconds > 0) => {}
                Some(_) => check.error(
                    join(&at, "windowSeconds"),
                    "must be a whole number of seconds above 0",
                ),
                None => check.error(join(&at, "windowSeconds"), "missing"),
            }
        }
        Some(_) => check.error(
            join(path, "spendLimit"),
            "must be { lamports, windowSeconds } or left out",
        ),
    }

    if enabled {
        name
    } else {
        None
    }
}

fn finish(check: Checker, enabled_programs: Vec<String>) -> RulesReport {
    RulesReport {
        valid: check.errors.is_empty(),
        errors: check.errors,
        warnings: check.warnings,
        enabled_programs,
    }
}

/// The rules document inside an API answer: the answer itself, or its
/// `rules` member.
pub fn find_document(body: &Value) -> &Value {
    match body.get("rules") {
        Some(rules) if rules.is_object() => rules,
        _ => body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PUMP: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
    const DLMM: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
    const FRIEND: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";

    fn valid() -> Value {
        json!({
            "version": 3,
            "programs": [
                {
                    "programId": PUMP, "name": "Pump.fun", "enabled": true,
                    "instructions": ["buy", "sell"],
                    "spendLimit": { "lamports": "1000000000", "windowSeconds": 86400 }
                },
                { "programId": DLMM, "name": "Meteora DLMM", "enabled": false }
            ],
            "transferAllowlist": [FRIEND]
        })
    }

    fn paths(issues: &[RulesIssue]) -> Vec<&str> {
        issues.iter().map(|issue| issue.path.as_str()).collect()
    }

    #[test]
    fn a_builder_document_is_valid() {
        let report = validate(&valid());
        assert!(report.valid, "{report:?}");
        assert!(report.warnings.is_empty(), "{report:?}");
        assert_eq!(report.enabled_programs, vec!["Pump.fun"]);
    }

    #[test]
    fn options_may_be_left_out_and_lamports_may_be_a_number() {
        let report = validate(&json!({
            "version": 3,
            "programs": [{
                "programId": PUMP, "name": "Pump.fun", "enabled": true,
                "spendLimit": { "lamports": 5000, "windowSeconds": 60 }
            }]
        }));
        assert!(report.valid, "{report:?}");
    }

    #[test]
    fn earlier_versions_are_named_as_such() {
        let mut document = valid();
        document["version"] = json!(2);
        let report = validate(&document);
        assert!(!report.valid);
        assert!(report.errors[0].message.contains("earlier rules format"));
        document["version"] = json!("3");
        assert_eq!(paths(&validate(&document).errors), vec!["version"]);
    }

    #[test]
    fn misspelt_keys_are_errors_with_a_suggestion() {
        let mut document = valid();
        document["transferAllowList"] = json!([]);
        document["programs"][0]["spend_limit"] = json!({});
        let report = validate(&document);
        assert_eq!(
            paths(&report.errors),
            vec!["transferAllowList", "programs[0].spend_limit"]
        );
        assert!(report.errors[0]
            .message
            .contains("did you mean `transferAllowlist`"));
        assert!(report.errors[1]
            .message
            .contains("did you mean `spendLimit`"));
    }

    #[test]
    fn program_entries_are_checked() {
        let report = validate(&json!({
            "version": 3,
            "programs": [
                { "programId": "not-a-key", "name": "", "enabled": "yes", "instructions": "buy" },
                { "programId": PUMP, "name": "A", "enabled": true },
                { "programId": PUMP, "name": "B", "enabled": true,
                  "spendLimit": { "lamports": "0", "windowSeconds": 0, "mint": "x" } },
                { "programId": DLMM, "name": "C", "enabled": true,
                  "spendLimit": { "lamports": "-1" } },
            ]
        }));
        assert_eq!(
            paths(&report.errors),
            vec![
                "programs[0].programId",
                "programs[0].name",
                "programs[0].enabled",
                "programs[0].instructions",
                "programs[2].programId",
                "programs[2].spendLimit.mint",
                "programs[2].spendLimit.lamports",
                "programs[2].spendLimit.windowSeconds",
                "programs[3].spendLimit.lamports",
                "programs[3].spendLimit.windowSeconds",
            ],
            "{report:#?}"
        );
    }

    #[test]
    fn suspicious_but_signable_documents_warn() {
        let report = validate(&json!({
            "version": 3,
            "programs": [{ "programId": PUMP, "name": "Pump.fun", "enabled": true,
                           "instructions": [] }],
            "transferAllowlist": [FRIEND, FRIEND]
        }));
        assert!(report.valid, "{report:?}");
        assert_eq!(
            paths(&report.warnings),
            vec!["programs[0].instructions", "transferAllowlist[1]"]
        );
        let off = validate(&json!({
            "version": 3,
            "programs": [{ "programId": PUMP, "name": "Pump.fun", "enabled": false }]
        }));
        assert_eq!(paths(&off.warnings), vec!["programs"]);
    }

    #[test]
    fn allowlist_entries_must_be_addresses() {
        let report = validate(&json!({
            "version": 3, "programs": [], "transferAllowlist": ["0x1234", 7]
        }));
        assert_eq!(
            paths(&report.errors),
            vec!["transferAllowlist[0]", "transferAllowlist[1]"]
        );
    }

    #[test]
    fn non_objects_are_refused() {
        assert!(!validate(&json!([])).valid);
        assert!(!validate(&json!({ "version": 3, "programs": {} })).valid);
    }

    #[test]
    fn the_document_is_found_inside_an_answer() {
        let answer = json!({ "rules": valid(), "signedAt": "x" });
        assert_eq!(find_document(&answer), &valid());
        assert_eq!(find_document(&valid()), &valid());
    }
}
