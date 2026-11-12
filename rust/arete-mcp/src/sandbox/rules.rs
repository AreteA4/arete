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
//!
//! Version 3 is the only format. An earlier document is refused with one
//! line ([`unsupported_version`]) rather than checked field by field.
//!
//! What `GET /rules` answers (`a4 sandbox rules show`, MCP `sandbox_rules`)
//! is not such a document but the agent's view of the rules
//! ([`is_agent_view`]): the enabled programs with their instructions'
//! arguments and accounts, the baseline, and the signed document itself as
//! `signedRules`. Its numbers are `rulesFormat` (the format, 3) and
//! `revision` (which signing of the sandbox's rules is active, 1 for the
//! first). Earlier answers called the revision `version`, so an agent view's
//! `version` is never read as a format.

use std::collections::HashSet;

use serde::Serialize;
use serde_json::{Map, Value};

pub const RULES_VERSION: u64 = 3;

/// The refusal code for rules in a format before version 3.
pub const UNSUPPORTED_VERSION_CODE: &str = "sandbox-rules-version-unsupported";

/// The code the API gives, under `unavailable`, for active rules in a format
/// it no longer supports.
pub const API_UNSUPPORTED_VERSION_CODE: &str = "sandbox-rules-unsupported-version";

/// The one-line refusal for an answer of `GET /rules` whose active rules
/// are in a format before version 3, or `None`. Only the format counts: an
/// agent view's `revision` (its `version`, from an earlier API) is a count
/// of signings, so revision 1 of version 3 rules is never refused.
pub fn unsupported_answer(body: &Value) -> Option<String> {
    if let Some(message) = unsupported_version(find_document(body)) {
        return Some(message);
    }
    if !is_agent_view(body) {
        return None;
    }
    let old_format = format(body).is_some_and(|format| format < RULES_VERSION);
    let refused = body["unavailable"]["code"].as_str() == Some(API_UNSUPPORTED_VERSION_CODE);
    (old_format || refused).then(|| {
        "The sandbox's active rules use a format before version 3, which is no longer \
         supported. Build and sign v3 rules in the dashboard (Sandboxes → Rules)."
            .to_string()
    })
}

/// The one-line refusal for a rules document in a format before version 3,
/// or `None` for any other document (version 3, a version this check does
/// not know, or none at all: [`validate`] reports those).
pub fn unsupported_version(document: &Value) -> Option<String> {
    if is_agent_view(document) {
        return None;
    }
    let version = document.get("version")?.as_u64()?;
    (version < RULES_VERSION).then(|| {
        format!(
            "Rules version {version} is no longer supported. Build and sign v3 rules in the \
             dashboard (Sandboxes → Rules)."
        )
    })
}

const ROOT_KEYS: &[&str] = &["version", "programs", "transferAllowlist"];
const PROGRAM_KEYS: &[&str] = &[
    "programId",
    "name",
    "enabled",
    "instructions",
    "functions",
    "spendLimit",
    "holdOnUpgrade",
];

/// `view` of the agent's view of the rules.
pub const AGENT_VIEW: &str = "agent";

/// Where the agent's view of the rules carries the signed document.
pub const SIGNED_RULES: &str = "signedRules";

/// Whether `document` is the agent's view of a sandbox's rules (what
/// `GET /rules` answers) rather than a rules document: it says so
/// (`"view": "agent"`), or, from an API before that, it lists the
/// `baseline` programs a rules document never has.
pub fn is_agent_view(document: &Value) -> bool {
    document.get("view").and_then(Value::as_str) == Some(AGENT_VIEW)
        || document.get("baseline").is_some()
}

/// The rules revision an answer names: which signing of the sandbox's
/// rules is active (1 for the first). An agent view from an API before
/// `revision` called it `version`.
pub fn revision(body: &Value) -> Option<u64> {
    body.get("revision")
        .or_else(|| body.get("rulesRevision"))
        .and_then(Value::as_u64)
        .or_else(|| {
            is_agent_view(body)
                .then(|| body.get("version").and_then(Value::as_u64))
                .flatten()
        })
}

/// The rules format an answer names: `rulesFormat` on an agent view, else a
/// document's `version`.
pub fn format(body: &Value) -> Option<u64> {
    if is_agent_view(body) {
        body.get("rulesFormat").and_then(Value::as_u64)
    } else {
        body.get("version").and_then(Value::as_u64)
    }
}

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
    /// The member checked when the input was the agent's view of the rules
    /// (`signedRules`); the issues' paths start with it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<String>,
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

/// The refusal for the agent's view of the rules without the signed
/// document in it.
pub fn agent_view_message() -> String {
    "this is the agent's view of a sandbox's rules (`a4 sandbox rules show`), not a Rules v3 \
     document; its numbers are a revision, not a format. Validate the signed document: \
     `a4 sandbox rules show --json` includes it as `signedRules`, and the owner can copy it \
     from the dashboard (Sandboxes → Rules → View or paste JSON)."
        .to_string()
}

/// Check a Rules v3 document. The agent's view of the rules
/// ([`is_agent_view`]) is checked through the signed document it carries
/// (`signedRules`), or refused when it carries none: its programs are
/// resolved entries, never a document's, so checking them as one would
/// report a working sandbox as broken.
pub fn validate(document: &Value) -> RulesReport {
    if is_agent_view(document) {
        return match document.get(SIGNED_RULES) {
            Some(signed) if signed.is_object() => {
                let prefix = |issues: Vec<RulesIssue>| {
                    issues
                        .into_iter()
                        .map(|issue| RulesIssue {
                            path: match issue.path.as_str() {
                                "" => SIGNED_RULES.to_string(),
                                path => join(SIGNED_RULES, path),
                            },
                            message: issue.message,
                        })
                        .collect()
                };
                let report = validate(signed);
                RulesReport {
                    valid: report.valid,
                    errors: prefix(report.errors),
                    warnings: prefix(report.warnings),
                    enabled_programs: report.enabled_programs,
                    checked: Some(SIGNED_RULES.to_string()),
                }
            }
            _ => RulesReport {
                valid: false,
                errors: vec![RulesIssue {
                    path: String::new(),
                    message: agent_view_message(),
                }],
                ..Default::default()
            },
        };
    }
    let mut check = Checker {
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    let mut enabled_programs = Vec::new();
    let Some(root) = document.as_object() else {
        check.error("", "the rules must be a JSON object");
        return finish(check, enabled_programs);
    };
    if let Some(message) = unsupported_version(document) {
        check.error("version", message);
        return finish(check, enabled_programs);
    }

    match root.get("version") {
        Some(version) if version.as_u64() == Some(RULES_VERSION) => {}
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

    match object.get("functions") {
        None | Some(Value::Null) => {}
        Some(Value::Array(functions)) => {
            for (index, function) in functions.iter().enumerate() {
                if !function
                    .as_str()
                    .is_some_and(|name| !name.trim().is_empty())
                {
                    check.error(
                        format!("{}[{index}]", join(path, "functions")),
                        "must be a function name",
                    );
                }
            }
        }
        Some(_) => check.error(
            join(path, "functions"),
            "must be a list of function names or left out",
        ),
    }
    match object.get("holdOnUpgrade") {
        None | Some(Value::Null) | Some(Value::Bool(_)) => {}
        Some(_) => check.error(join(path, "holdOnUpgrade"), "must be true or false"),
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
        checked: None,
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

    const NO_LONGER: &str = "Rules version 2 is no longer supported. Build and sign v3 rules \
                             in the dashboard (Sandboxes → Rules).";

    #[test]
    fn a_version_2_document_is_refused_with_one_line() {
        // A version 2 document: programs keyed by id, instruction objects.
        let document = json!({
            "version": 2,
            "programs": {
                PUMP: { "instructions": [{ "name": "buy" }] }
            }
        });
        assert_eq!(unsupported_version(&document).as_deref(), Some(NO_LONGER));
        let report = validate(&document);
        assert!(!report.valid);
        assert!(report.warnings.is_empty(), "{report:?}");
        assert_eq!(
            report.errors,
            vec![RulesIssue {
                path: "version".into(),
                message: NO_LONGER.into()
            }]
        );
    }

    #[test]
    fn other_versions_are_checked_as_usual() {
        assert_eq!(unsupported_version(&valid()), None);
        assert_eq!(unsupported_version(&json!({ "programs": [] })), None);
        let mut document = valid();
        document["version"] = json!("2");
        assert_eq!(unsupported_version(&document), None);
        assert_eq!(paths(&validate(&document).errors), vec!["version"]);
        document["version"] = json!(4);
        assert_eq!(
            validate(&document).errors[0].message,
            "must be 3, not 4".to_string()
        );
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

    /// What `GET /rules` answered before it named its numbers: the
    /// revision as `version`, resolved programs, the baseline.
    fn old_agent_view() -> Value {
        json!({
            "version": 1,
            "requestId": "req-1",
            "baseline": [{ "name": "System" }],
            "programs": [{
                "programId": PUMP, "name": "pump", "specHash": "ab", "allowed": ["buy"],
                "instructions": [{ "name": "buy", "args": [], "accounts": [] }]
            }]
        })
    }

    fn agent_view(signed: Option<Value>) -> Value {
        let mut view = json!({
            "view": "agent",
            "rulesFormat": 3,
            "revision": 2,
            "programs": [{ "programId": PUMP, "name": "Pump.fun", "allowed": ["buy", "sell"], "instructions": [] }],
            "transferAllowlist": [FRIEND],
            "baseline": [{ "name": "System" }]
        });
        if let Some(signed) = signed {
            view["signedRules"] = signed;
        }
        view
    }

    #[test]
    fn an_agent_view_is_never_read_as_an_old_format() {
        // Revision 1 of version 3 rules is not "rules version 1".
        for view in [old_agent_view(), agent_view(None)] {
            assert!(is_agent_view(&view));
            assert_eq!(unsupported_version(&view), None);
            assert_eq!(unsupported_answer(&view), None);
        }
        assert_eq!(revision(&old_agent_view()), Some(1));
        assert_eq!(format(&old_agent_view()), None);
        assert_eq!(revision(&agent_view(None)), Some(2));
        assert_eq!(format(&agent_view(None)), Some(3));
        // A document is still a document.
        assert!(!is_agent_view(&valid()));
        assert_eq!(format(&valid()), Some(3));
        assert_eq!(revision(&valid()), None);
    }

    #[test]
    fn an_agent_view_without_the_signed_rules_is_refused_not_checked() {
        for view in [old_agent_view(), agent_view(None)] {
            let report = validate(&view);
            assert!(!report.valid);
            // One line saying what the input is, not a field-by-field
            // report that the sandbox has every program turned off.
            assert_eq!(
                report.errors,
                vec![RulesIssue {
                    path: String::new(),
                    message: agent_view_message()
                }]
            );
            assert!(report.warnings.is_empty(), "{report:?}");
            assert!(report.enabled_programs.is_empty());
        }
        assert!(agent_view_message().contains("signedRules"));
    }

    #[test]
    fn an_agent_view_is_checked_through_its_signed_rules() {
        let report = validate(&agent_view(Some(valid())));
        assert!(report.valid, "{report:?}");
        assert_eq!(report.checked.as_deref(), Some(SIGNED_RULES));
        assert_eq!(report.enabled_programs, vec!["Pump.fun"]);
        let shown = serde_json::to_value(&report).unwrap();
        assert_eq!(shown["checked"], "signedRules");

        let mut bad = valid();
        bad["programs"][0]["spendLimit"]["lamports"] = json!(0);
        bad["transferAllowList"] = json!([]);
        let report = validate(&agent_view(Some(bad)));
        assert!(!report.valid);
        assert_eq!(
            paths(&report.errors),
            vec![
                "signedRules.transferAllowList",
                "signedRules.programs[0].spendLimit.lamports"
            ]
        );
        // A plain document reports no `checked`.
        let shown = serde_json::to_value(validate(&valid())).unwrap();
        assert!(shown.get("checked").is_none(), "{shown}");
    }

    #[test]
    fn signed_documents_may_name_functions_and_hold_on_upgrade() {
        let mut document = valid();
        document["programs"][0]["functions"] = json!(["transactions.buy"]);
        document["programs"][0]["holdOnUpgrade"] = json!(true);
        let report = validate(&document);
        assert!(report.valid, "{report:?}");
        document["programs"][0]["functions"] = json!("transactions.buy");
        document["programs"][0]["holdOnUpgrade"] = json!("yes");
        assert_eq!(
            paths(&validate(&document).errors),
            vec!["programs[0].functions", "programs[0].holdOnUpgrade"]
        );
    }

    #[test]
    fn only_an_old_format_refuses_an_answer() {
        let mut view = agent_view(None);
        view["rulesFormat"] = Value::Null;
        view["programs"] = json!([]);
        view["unavailable"] = json!({
            "code": API_UNSUPPORTED_VERSION_CODE,
            "message": "the sandbox's active rules use a format Arete no longer supports"
        });
        assert!(unsupported_answer(&view).is_some());
        view["unavailable"] = json!({ "code": "sandbox-rules-unsigned", "message": "x" });
        assert_eq!(unsupported_answer(&view), None);
        view["rulesFormat"] = json!(2);
        assert!(unsupported_answer(&view).is_some());
        let mut document = valid();
        document["version"] = json!(2);
        assert!(unsupported_answer(&document).is_some());
        assert_eq!(unsupported_answer(&valid()), None);
    }

    #[test]
    fn the_document_is_found_inside_an_answer() {
        let answer = json!({ "rules": valid(), "signedAt": "x" });
        assert_eq!(find_document(&answer), &valid());
        assert_eq!(find_document(&valid()), &valid());
    }
}
