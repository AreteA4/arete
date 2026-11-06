//! Checks on the text SDK generation copies from a stack into source code.
//!
//! A stack spec, its LiveSpec and its programs' IDLs are written by whoever
//! published them, and the TypeScript, Rust and Python generators copy their
//! text into the SDKs they emit: names become identifiers, string literals
//! and comments, and docs become doc comments. The generators quote and
//! comment that text as it is, so a name such as `x"); run(); ("` or one
//! holding a line break could end the literal or comment it is placed in and
//! add code to the SDK, which then runs when the consumer imports or builds
//! it. Every generator entry point calls [`validate_stack_spec`] (or
//! [`validate_entity_spec`] / [`validate_text`]) first and refuses input that
//! breaks these rules, instead of emitting the SDK:
//!
//! 1. **Inert text.** Every string in the spec, and every map key, may not
//!    contain a quote (`'`, `"`, `` ` ``), a backslash, a brace, a comment
//!    delimiter (`/*`, `*/`), a control character, a Unicode line or paragraph
//!    separator or a bidirectional formatting character. None of these occur
//!    in names, types, paths or identifiers of a valid stack, and without
//!    them no text can end a string literal, template literal or comment in
//!    any of the three languages, or open a template substitution.
//! 2. **Identifiers.** The names the generators also use to build
//!    identifiers must be identifiers (`[A-Za-z_][A-Za-z0-9_]*`): an IDL's
//!    instruction, argument, account, type, field, variant, event and error
//!    names, PDA names, and an entity's name and section names. IDL account, type and
//!    event names may be path-qualified (`module::Name`, identifiers joined
//!    by `::`), instruction account names may also be written `$(Name)` (the
//!    form IDLs use for sysvar accounts), field names and field paths are
//!    identifiers joined by `.`, and view ids are `/`-separated segments of
//!    letters, digits, `_`, `.` and `-`.
//!
//! Free text is exempt: `docs`, error `msg` and `description` entries may
//! hold any text (but not a PDA or field mapping that happens to be named
//! `docs`), and so is data only the runtime reads, such as a field
//! mapping's constant. The generators render it through escaping helpers instead
//! ([`comment_text`] and the per-language doc comment and string literal
//! renderers), so it stays inside its comment or literal.

use crate::ast::{
    EntitySection, IdlEnumVariantFieldSnapshot, IdlFieldSnapshot, IdlSnapshot,
    IdlTypeDefKindSnapshot, InstructionDef, SerializableStackSpec, SerializableStreamSpec,
};
use serde_json::Value;

/// Fields holding free text, exempt from the inert-text rule: `docs` (a
/// list of lines) and an error's `msg` or a field's `description` (a string).
const FREE_TEXT_KEYS: &[&str] = &["docs", "msg", "description"];

/// Spec fields whose keys are names chosen by the stack or IDL author rather
/// than field names of the spec format. Below them a key such as `docs` is a
/// name like any other, so nothing there is treated as free text.
const AUTHOR_KEYED_MAPS: &[&str] = &["pdas", "field_mappings", "required_fields"];

/// Entity spec data that only the runtime reads, never copied into a
/// generated SDK: resolver, hook and computed-field specs, view pipelines,
/// handler conditions, and the constants and defaults a field mapping
/// supplies (the generators only infer a type from a constant). Paths are
/// relative to an entity spec; `None` matches any list index. They hold
/// literal values such as `"don't"` that need no restriction.
const RUNTIME_ONLY_ENTITY_PATHS: &[&[Option<&str>]] = &[
    &[Some("resolver_specs")],
    &[Some("resolver_hooks")],
    &[Some("instruction_hooks")],
    &[Some("computed_field_specs")],
    &[Some("views"), None, Some("pipeline")],
    &[Some("handlers"), None, Some("conditions")],
    &[
        Some("handlers"),
        None,
        Some("mappings"),
        None,
        Some("condition"),
    ],
    &[
        Some("handlers"),
        None,
        Some("mappings"),
        None,
        Some("source"),
        Some("Constant"),
    ],
    &[
        Some("handlers"),
        None,
        Some("mappings"),
        None,
        Some("source"),
        Some("FromSource"),
        Some("default"),
    ],
];

/// Refuse a stack spec whose text could escape the code the SDK generators
/// place it in; see the module documentation for the rules.
pub fn validate_stack_spec(spec: &SerializableStackSpec) -> Result<(), String> {
    let value = serde_json::to_value(spec)
        .map_err(|error| format!("stack spec cannot be checked for SDK generation: {error}"))?;
    check_inert_value(&value, "stack spec", &mut Vec::new())?;
    for (index, idl) in spec.idls.iter().enumerate() {
        check_idl_names(idl, &format!("IDL #{index}"))?;
    }
    for (index, program) in spec.program_specs.iter().enumerate() {
        check_idl_names(
            &program.idl_snapshot.clone().into_legacy_snapshot(),
            &format!("program spec #{index}"),
        )?;
    }
    for instruction in &spec.instructions {
        check_instruction_def(instruction)?;
    }
    for (program, pdas) in &spec.pdas {
        for (name, pda) in pdas {
            require_identifier(&format!("program '{program}' PDA"), name)?;
            require_identifier(&format!("program '{program}' PDA"), &pda.name)?;
        }
    }
    for entity in &spec.entities {
        check_entity_names(entity)?;
    }
    Ok(())
}

/// [`validate_stack_spec`] for a single entity spec.
pub fn validate_entity_spec(spec: &SerializableStreamSpec) -> Result<(), String> {
    let value = serde_json::to_value(spec)
        .map_err(|error| format!("entity spec cannot be checked for SDK generation: {error}"))?;
    check_inert_value(&value, "entity spec", &mut Vec::new())?;
    check_entity_names(spec)
}

/// Refuse a single name (a composition name or LiveSpec alias, say) that is
/// not inert text; `what` describes it in the error.
pub fn validate_text(what: &str, value: &str) -> Result<(), String> {
    match inert_violation(value) {
        Some(problem) => Err(format!(
            "{what} {value:?} contains {problem}; names in a stack, LiveSpec or IDL may not contain quotes, backticks, backslashes, braces, comment delimiters or control characters"
        )),
        None => Ok(()),
    }
}

/// `text` (a doc line or other free text) made safe to place in a line or
/// block comment of any of the generated languages: control characters and
/// Unicode line separators become spaces (a line break would end a line
/// comment) and `*/` becomes `*\/` (it would end a block comment). Text with
/// none of these is returned unchanged.
pub(crate) fn comment_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous = '\0';
    for character in text.chars() {
        if character == '/' && previous == '*' {
            out.push('\\');
        }
        if is_line_breaking(character) {
            out.push(' ');
        } else {
            out.push(character);
        }
        previous = character;
    }
    out
}

fn is_line_breaking(character: char) -> bool {
    (character.is_control() && character != '\t') || matches!(character, '\u{2028}' | '\u{2029}')
}

fn is_bidi_format(character: char) -> bool {
    matches!(
        character,
        '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// What makes `value` unsafe to quote or comment, if anything.
fn inert_violation(value: &str) -> Option<String> {
    if let Some(character) = value.chars().find(|character| {
        matches!(character, '\'' | '"' | '`' | '\\' | '{' | '}')
            || character.is_control()
            || matches!(character, '\u{2028}' | '\u{2029}')
            || is_bidi_format(*character)
    }) {
        return Some(format!("the character {character:?}"));
    }
    ["/*", "*/"]
        .into_iter()
        .find(|delimiter| value.contains(delimiter))
        .map(|delimiter| format!("the comment delimiter `{delimiter}`"))
}

/// One step from the root of the checked value, for error messages.
enum Step<'a> {
    Key(&'a str),
    Index(usize),
}

fn render_path(root: &str, path: &[Step<'_>]) -> String {
    let mut rendered = root.to_string();
    for step in path {
        match step {
            Step::Key(key) => {
                rendered.push('.');
                rendered.push_str(key);
            }
            Step::Index(index) => rendered.push_str(&format!("[{index}]")),
        }
    }
    rendered
}

/// Whether `path` (from the root of a stack spec, or of an entity spec) is
/// one of [`RUNTIME_ONLY_ENTITY_PATHS`]. Every step of these paths is a
/// field of the spec format, so no author-chosen name can match one.
fn is_runtime_only(path: &[Step<'_>]) -> bool {
    let relative = match path {
        [Step::Key("entities"), Step::Index(_), rest @ ..] => rest,
        _ => path,
    };
    RUNTIME_ONLY_ENTITY_PATHS.iter().any(|pattern| {
        pattern.len() == relative.len()
            && pattern
                .iter()
                .zip(relative)
                .all(|(expected, step)| match (expected, step) {
                    (Some(field), Step::Key(key)) => field == key,
                    (None, Step::Index(_)) => true,
                    _ => false,
                })
    })
}

/// Whether `item`, found under `key`, is free text: one of
/// [`FREE_TEXT_KEYS`] with the shape of free text, outside any
/// [`AUTHOR_KEYED_MAPS`].
fn is_free_text(key: &str, item: &Value, path: &[Step<'_>]) -> bool {
    let shaped = match key {
        "docs" => item
            .as_array()
            .is_some_and(|lines| lines.iter().all(Value::is_string)),
        "msg" | "description" => item.is_string() || item.is_null(),
        _ => false,
    };
    FREE_TEXT_KEYS.contains(&key)
        && shaped
        && !path
            .iter()
            .any(|step| matches!(step, Step::Key(key) if AUTHOR_KEYED_MAPS.contains(key)))
}

/// Check every string and key under `value`; the path of an offending entry
/// is only rendered when there is one.
fn check_inert_value<'a>(
    value: &'a Value,
    root: &str,
    path: &mut Vec<Step<'a>>,
) -> Result<(), String> {
    match value {
        Value::String(text) => {
            if inert_violation(text).is_some() {
                return validate_text(&render_path(root, path), text);
            }
            Ok(())
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                path.push(Step::Index(index));
                check_inert_value(item, root, path)?;
                path.pop();
            }
            Ok(())
        }
        Value::Object(entries) => {
            for (key, item) in entries {
                if inert_violation(key).is_some() {
                    return validate_text(&format!("key of {}", render_path(root, path)), key);
                }
                if is_free_text(key, item, path) {
                    continue;
                }
                path.push(Step::Key(key));
                if !is_runtime_only(path) {
                    check_inert_value(item, root, path)?;
                }
                path.pop();
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
}

fn is_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn require(valid: bool, what: &str, value: &str, expected: &str) -> Result<(), String> {
    if valid {
        Ok(())
    } else {
        Err(format!(
            "{what} {value:?} cannot be used in a generated SDK; it must be {expected}"
        ))
    }
}

fn require_identifier(what: &str, value: &str) -> Result<(), String> {
    require(
        is_identifier(value),
        what,
        value,
        "an identifier (ASCII letters, digits and `_`, not starting with a digit)",
    )
}

/// A type-like name, optionally path-qualified (`module::Name`): the
/// generators turn it into one PascalCase identifier.
fn require_qualified(what: &str, value: &str) -> Result<(), String> {
    require(
        value.split("::").all(is_identifier),
        what,
        value,
        "an identifier, or identifiers joined by `::`",
    )
}

fn require_account_name(what: &str, value: &str) -> Result<(), String> {
    let sysvar = value
        .strip_prefix("$(")
        .and_then(|rest| rest.strip_suffix(')'))
        .is_some_and(is_identifier);
    require(
        sysvar || is_identifier(value),
        what,
        value,
        "an identifier (ASCII letters, digits and `_`, not starting with a digit) or `$(Identifier)`",
    )
}

fn check_fields(fields: &[IdlFieldSnapshot], owner: &str) -> Result<(), String> {
    fields
        .iter()
        .try_for_each(|field| require_identifier(&format!("{owner} field"), &field.name))
}

fn check_idl_names(idl: &IdlSnapshot, idl_label: &str) -> Result<(), String> {
    let label = format!("{idl_label} ('{}')", idl.name);
    for account in &idl.accounts {
        require_qualified(&format!("{label} account"), &account.name)?;
        check_fields(
            &account.fields,
            &format!("{label} account '{}'", account.name),
        )?;
        if let Some(type_def) = &account.type_def {
            check_fields(
                &type_def.fields,
                &format!("{label} account '{}'", account.name),
            )?;
        }
    }
    for instruction in &idl.instructions {
        require_identifier(&format!("{label} instruction"), &instruction.name)?;
        let owner = format!("{label} instruction '{}'", instruction.name);
        for account in &instruction.accounts {
            require_account_name(&format!("{owner} account"), &account.name)?;
        }
        for arg in &instruction.args {
            require_identifier(&format!("{owner} argument"), &arg.name)?;
        }
    }
    for type_def in &idl.types {
        require_qualified(&format!("{label} type"), &type_def.name)?;
        let owner = format!("{label} type '{}'", type_def.name);
        match &type_def.type_def {
            IdlTypeDefKindSnapshot::Struct { fields, .. } => check_fields(fields, &owner)?,
            IdlTypeDefKindSnapshot::TupleStruct { .. } => {}
            IdlTypeDefKindSnapshot::Enum { variants, .. } => {
                for variant in variants {
                    require_identifier(&format!("{owner} variant"), &variant.name)?;
                    for field in &variant.fields {
                        if let IdlEnumVariantFieldSnapshot::Named(field) = field {
                            require_identifier(
                                &format!("{owner} variant '{}' field", variant.name),
                                &field.name,
                            )?;
                        }
                    }
                }
            }
        }
    }
    for event in &idl.events {
        require_qualified(&format!("{label} event"), &event.name)?;
        check_fields(&event.fields, &format!("{label} event '{}'", event.name))?;
    }
    for error in &idl.errors {
        require_identifier(&format!("{label} error"), &error.name)?;
    }
    Ok(())
}

fn check_instruction_def(instruction: &InstructionDef) -> Result<(), String> {
    require_identifier("instruction", &instruction.name)?;
    let owner = format!("instruction '{}'", instruction.name);
    for account in &instruction.accounts {
        require_account_name(&format!("{owner} account"), &account.name)?;
    }
    for arg in &instruction.args {
        require_identifier(&format!("{owner} argument"), &arg.name)?;
    }
    Ok(())
}

fn is_field_path(value: &str) -> bool {
    value.split('.').all(is_identifier)
}

fn is_view_id(value: &str) -> bool {
    value.split('/').all(|segment| {
        !segment.is_empty()
            && segment.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
            })
    })
}

fn check_section(section: &EntitySection, owner: &str) -> Result<(), String> {
    require_identifier(&format!("{owner} section"), &section.name)?;
    for field in &section.fields {
        require(
            is_field_path(&field.field_name),
            &format!("{owner} section '{}' field", section.name),
            &field.field_name,
            "identifiers joined by `.`",
        )?;
    }
    Ok(())
}

fn check_entity_names(entity: &SerializableStreamSpec) -> Result<(), String> {
    require_identifier("entity", &entity.state_name)?;
    let owner = format!("entity '{}'", entity.state_name);
    if let Some(idl) = &entity.idl {
        check_idl_names(idl, &format!("{owner} IDL"))?;
    }
    for section in &entity.sections {
        check_section(section, &owner)?;
    }
    for path in entity.field_mappings.keys() {
        require(
            is_field_path(path),
            &format!("{owner} field path"),
            path,
            "identifiers joined by `.`",
        )?;
    }
    for view in &entity.views {
        require(
            is_view_id(&view.id),
            &format!("{owner} view id"),
            &view.id,
            "`/`-separated segments of ASCII letters, digits, `_`, `.` and `-`",
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inert_text_rejects_every_breakout_character() {
        for hostile in [
            "a'b",
            "a\"b",
            "a`b",
            "a\\b",
            "a{b",
            "a}b",
            "${x}",
            "a*/b",
            "a/*b",
            "a\nb",
            "a\rb",
            "a\u{0}b",
            "a\u{85}b",
            "a\u{2028}b",
            "a\u{2029}b",
            "a\u{202E}b",
            "a\u{2066}b",
            "\"\"\"",
            "'''",
        ] {
            assert!(validate_text("name", hostile).is_err(), "{hostile:?}");
        }
        for fine in [
            "OreRound/latest",
            "results.expires_at_slot_hash",
            "Option < Vec < f64 > >",
            "[u8; 32]",
            "ore::TreasuryState",
            "$(SysVarRentPubkey)",
            "token-balances",
            "/seed?samples=",
            "https://example.com/var/",
            "value != ZERO_32",
            "a / b * c",
        ] {
            assert!(validate_text("name", fine).is_ok(), "{fine:?}");
        }
    }

    #[test]
    fn inert_errors_name_the_entry_and_skip_free_text() {
        let value = serde_json::json!({
            "entities": [{"docs": ["free \" text */"], "views": [{"id": "a'b"}]}]
        });
        let error = check_inert_value(&value, "stack spec", &mut Vec::new()).unwrap_err();
        assert!(
            error.starts_with("stack spec.entities[0].views[0].id \"a'b\" contains"),
            "{error}"
        );
    }

    #[test]
    fn author_named_map_entries_are_never_free_text() {
        let seed = serde_json::json!({"seeds": [{"Literal": {"value": "' + run() + '"}}]});
        for value in [
            serde_json::json!({"pdas": {"ore": {"docs": seed.clone()}}}),
            serde_json::json!({"pdas": {"ore": {"msg": "' + run() + '"}}}),
            serde_json::json!({"entities": [{"field_mappings": {"docs": {"field_name": "a'b"}}}]}),
            serde_json::json!({"entities": [{"docs": {"nested": "a'b"}}]}),
        ] {
            assert!(
                check_inert_value(&value, "stack spec", &mut Vec::new()).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn runtime_only_data_is_not_restricted() {
        let constant = serde_json::json!({"mappings": [{
            "target_path": "status.label",
            "source": {"Constant": "don't"}
        }]});
        let value = serde_json::json!({"entities": [{
            "handlers": [constant.clone()],
            "resolver_specs": [{"input_value": "it's"}],
            "views": [{"id": "E/list", "pipeline": [{"Filter": {"Literal": "a'b"}}]}]
        }]});
        assert!(check_inert_value(&value, "stack spec", &mut Vec::new()).is_ok());
        let entity = serde_json::json!({"handlers": [constant]});
        assert!(check_inert_value(&entity, "entity spec", &mut Vec::new()).is_ok());
        // The same value where the generators copy it into code is refused.
        let bad = serde_json::json!({"entities": [{"handlers": [{"mappings": [{
            "target_path": "status.label'",
            "source": {"Constant": "fine"}
        }]}]}]});
        assert!(check_inert_value(&bad, "stack spec", &mut Vec::new()).is_err());
    }

    #[test]
    fn name_rules() {
        assert!(is_identifier("_padding0"));
        assert!(!is_identifier("9lives"));
        assert!(!is_identifier("a-b"));
        assert!(!is_identifier(""));
        assert!(require_account_name("a", "$(SysVarRentPubkey)").is_ok());
        assert!(require_account_name("a", "$(x); y").is_err());
        assert!(is_field_path("id.round_id"));
        assert!(require_qualified("t", "sb_on_demand::pull_feed::Submission").is_ok());
        assert!(require_qualified("t", "a:::b").is_err());
        assert!(require_qualified("t", "::b").is_err());
        assert!(!is_field_path("id..round_id"));
        assert!(is_view_id("OreRound/latest"));
        assert!(is_view_id("token-balances/by.slot"));
        assert!(!is_view_id("OreRound//latest"));
        assert!(!is_view_id("OreRound/a b"));
    }

    #[test]
    fn comment_text_neutralizes_comment_terminators() {
        assert_eq!(comment_text("plain text"), "plain text");
        assert_eq!(comment_text("a */ b"), "a *\\/ b");
        assert_eq!(comment_text("a\nb\r\nc\u{2028}d"), "a b  c d");
        assert_eq!(comment_text("tab\tkept"), "tab\tkept");
    }
}
