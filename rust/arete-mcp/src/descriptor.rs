//! Compact, client-side views of registry install descriptors.
//!
//! A full program or stack install descriptor carries everything `a4 install`
//! consumes: the IDL, the ProgramSpec, every LiveSpec and the SDK extension
//! sources. For a real program that is hundreds of kilobytes, almost none of
//! which an agent needs to answer "how do I call this operation?" or "which
//! views can I subscribe to?". The functions here cut those answers out of the
//! full descriptor on the client, so the registry contract does not change.
//! The MCP explore tools and `a4 explore` both use them, so they report the
//! same shapes.
//!
//! Descriptors are read tolerantly: artifacts embedded in a descriptor may use
//! snake_case or camelCase keys, unknown fields are ignored, and anything the
//! descriptor does not carry is left out of the output rather than guessed.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Sections accepted by `explore_program { sections }` and
/// `a4 explore program --section`.
pub const PROGRAM_SECTIONS: [&str; 5] =
    ["accounts", "events", "instructions", "operations", "types"];

/// Surface kinds that build a transaction, in listing order.
const BUILD_KINDS: [&str; 4] = ["transaction", "flow", "instruction", "raw-instruction"];

/// Every surface kind, in listing order: the highest-level operations first.
const KIND_ORDER: [&str; 9] = [
    "transaction",
    "flow",
    "instruction",
    "raw-instruction",
    "read",
    "account-fetch",
    "pda",
    "math",
    "constant",
];

/// Surface kinds whose paths a summary lists by name; the rest are counted.
const SUMMARY_LISTED_KINDS: [&str; 4] = ["transaction", "flow", "instruction", "read"];

/// How many near matches an unknown operation or view error suggests.
const MAX_SUGGESTIONS: usize = 8;

// ---------------------------------------------------------------------------
// Tolerant accessors
// ---------------------------------------------------------------------------

fn get<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|key| value.get(*key).filter(|found| !found.is_null()))
}

fn get_str<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
}

fn get_array<'a>(value: &'a Value, keys: &[&str]) -> &'a [Value] {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_array))
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn get_bool(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
}

fn first_bool(value: &Value, keys: &[&str]) -> bool {
    get_bool(value, keys).unwrap_or(false)
}

fn put(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(value) = value.filter(|value| !value.is_null()) {
        map.insert(key.to_string(), value);
    }
}

fn put_str(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    put(
        map,
        key,
        value.map(|value| Value::String(value.to_string())),
    );
}

fn string_values(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

/// Lowercased with `_`, `-` and spaces removed, so `claim_sol`, `claimSol`
/// and `ClaimSol` compare equal.
fn loose(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | ' '))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Split comma-separated entries, trim them, and drop empties, keeping order
/// and the first of any duplicates.
pub fn split_list<S: AsRef<str>>(values: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for value in values {
        for part in value.as_ref().split(',') {
            let part = part.trim();
            if !part.is_empty() && !out.iter().any(|seen| seen == part) {
                out.push(part.to_string());
            }
        }
    }
    out
}

/// Validate program section names. `label` names the argument in the error
/// (`sections` for MCP, `--section` for the CLI).
pub fn parse_sections<S: AsRef<str>>(values: &[S], label: &str) -> Result<Vec<String>> {
    let sections = split_list(values)
        .into_iter()
        .map(|section| section.to_ascii_lowercase())
        .collect::<Vec<_>>();
    for section in &sections {
        if !PROGRAM_SECTIONS.contains(&section.as_str()) {
            bail!(
                "`{label}` must be one of: {}. Got `{section}`.",
                PROGRAM_SECTIONS.join(", ")
            );
        }
    }
    Ok(sections)
}

// ---------------------------------------------------------------------------
// IDL summaries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NamedTypeSummary {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discriminator: Option<Value>,
    pub fields: Vec<NamedTypeSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstructionAccountSummary {
    pub name: String,
    pub writable: bool,
    pub signer: bool,
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstructionSummary {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discriminator: Option<Value>,
    pub arguments: Vec<NamedTypeSummary>,
    pub accounts: Vec<InstructionAccountSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventSummary {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discriminator: Option<Value>,
    pub fields: Vec<NamedTypeSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TypeSummary {
    pub name: String,
    pub kind: String,
    pub fields: Vec<NamedTypeSummary>,
    pub variants: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ErrorSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Value>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

fn name_of(value: &Value) -> String {
    get_str(value, &["name"]).unwrap_or("unknown").to_string()
}

fn named_types(value: &Value, key: &str) -> Vec<NamedTypeSummary> {
    get_array(value, &[key])
        .iter()
        .filter_map(|field| {
            Some(NamedTypeSummary {
                name: field.get("name")?.as_str()?.into(),
                field_type: field.get("type").cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}

fn discriminator(value: &Value) -> Option<Value> {
    get(value, &["discriminator", "discriminant"]).cloned()
}

fn account_summary(account: &Value) -> AccountSummary {
    let fields = if account.get("fields").is_some() {
        named_types(account, "fields")
    } else {
        account
            .get("type")
            .map(|definition| named_types(definition, "fields"))
            .unwrap_or_default()
    };
    AccountSummary {
        name: name_of(account),
        discriminator: discriminator(account),
        fields,
    }
}

/// Flatten an instruction's accounts, qualifying nested account groups as
/// `group.account`.
fn flatten_accounts<'a>(values: &'a [Value], prefix: &str, output: &mut Vec<(String, &'a Value)>) {
    for account in values {
        let name = get_str(account, &["name"]).unwrap_or("unknown");
        let qualified = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        };
        if let Some(children) = account.get("accounts").and_then(Value::as_array) {
            flatten_accounts(children, &qualified, output);
            continue;
        }
        output.push((qualified, account));
    }
}

fn is_writable(account: &Value) -> bool {
    first_bool(
        account,
        &["writable", "isMut", "isWritable", "is_writable", "is_mut"],
    )
}

fn is_signer(account: &Value) -> bool {
    first_bool(account, &["signer", "isSigner", "is_signer"])
}

fn is_optional(account: &Value) -> bool {
    first_bool(account, &["optional", "isOptional", "is_optional"])
}

fn instruction_summary(instruction: &Value) -> InstructionSummary {
    let mut flattened = Vec::new();
    flatten_accounts(get_array(instruction, &["accounts"]), "", &mut flattened);
    InstructionSummary {
        name: name_of(instruction),
        discriminator: discriminator(instruction),
        arguments: named_types(instruction, "args"),
        accounts: flattened
            .into_iter()
            .map(|(name, account)| InstructionAccountSummary {
                name,
                writable: is_writable(account),
                signer: is_signer(account),
                optional: is_optional(account),
            })
            .collect(),
    }
}

fn named_type_fields(idl: &Value, name: &str) -> Option<Vec<NamedTypeSummary>> {
    get_array(idl, &["types"])
        .iter()
        .find(|user_type| {
            get_str(user_type, &["name"])
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        })
        .map(|user_type| named_types(user_type.get("type").unwrap_or(user_type), "fields"))
}

fn event_summary(idl: &Value, event: &Value) -> EventSummary {
    let inline_fields = named_types(event, "fields");
    let fields = if inline_fields.is_empty() {
        event
            .get("data")
            .and_then(|data| data.get("name"))
            .and_then(Value::as_str)
            .and_then(|name| named_type_fields(idl, name))
            .or_else(|| {
                event
                    .get("type")
                    .map(|definition| named_types(definition, "fields"))
            })
            .or_else(|| get_str(event, &["name"]).and_then(|name| named_type_fields(idl, name)))
            .unwrap_or_default()
    } else {
        inline_fields
    };
    EventSummary {
        name: name_of(event),
        discriminator: discriminator(event),
        fields,
    }
}

fn type_summary(user_type: &Value) -> TypeSummary {
    let definition = user_type.get("type").unwrap_or(user_type);
    TypeSummary {
        name: name_of(user_type),
        kind: get_str(definition, &["kind"]).unwrap_or("unknown").into(),
        fields: named_types(definition, "fields"),
        variants: get_array(definition, &["variants"])
            .iter()
            .filter_map(|variant| get_str(variant, &["name"]))
            .map(str::to_string)
            .collect(),
    }
}

fn error_summary(error: &Value) -> ErrorSummary {
    ErrorSummary {
        code: get(error, &["code"]).cloned(),
        name: name_of(error),
        message: get_str(error, &["msg", "message"]).map(str::to_string),
    }
}

/// Account types declared by an IDL.
pub fn idl_accounts(idl: &Value) -> Vec<AccountSummary> {
    get_array(idl, &["accounts"])
        .iter()
        .map(account_summary)
        .collect()
}

/// Instructions declared by an IDL, with nested account groups flattened.
pub fn idl_instructions(idl: &Value) -> Vec<InstructionSummary> {
    get_array(idl, &["instructions"])
        .iter()
        .map(instruction_summary)
        .collect()
}

/// Events declared by an IDL, with fields resolved through named types.
pub fn idl_events(idl: &Value) -> Vec<EventSummary> {
    get_array(idl, &["events"])
        .iter()
        .map(|event| event_summary(idl, event))
        .collect()
}

/// User-defined types declared by an IDL.
pub fn idl_types(idl: &Value) -> Vec<TypeSummary> {
    get_array(idl, &["types"])
        .iter()
        .map(type_summary)
        .collect()
}

/// Program errors declared by an IDL.
pub fn idl_errors(idl: &Value) -> Vec<ErrorSummary> {
    get_array(idl, &["errors"])
        .iter()
        .map(error_summary)
        .collect()
}

// ---------------------------------------------------------------------------
// Transports and auth
// ---------------------------------------------------------------------------

/// The requirement fields of one auth descriptor, read from either casing.
fn compact_auth(auth: &Value) -> Value {
    let mut out = Map::new();
    put(
        &mut out,
        "required",
        get_bool(auth, &["required"]).map(Value::Bool),
    );
    put_str(&mut out, "mode", get_str(auth, &["mode"]));
    let classes = get_array(auth, &["acceptedKeyClasses", "accepted_key_classes"]);
    if !classes.is_empty() {
        out.insert("acceptedKeyClasses".into(), json!(string_values(classes)));
    }
    let scopes = get_array(auth, &["scopes"]);
    if !scopes.is_empty() {
        out.insert("scopes".into(), json!(string_values(scopes)));
    }
    put(
        &mut out,
        "transactionEntitlementRequired",
        get_bool(
            auth,
            &[
                "transactionEntitlementRequired",
                "transaction_entitlement_required",
            ],
        )
        .map(Value::Bool),
    );
    Value::Object(out)
}

/// Endpoint and auth of a chain or transaction binding.
fn binding_transport(binding: Option<&Value>) -> Option<Value> {
    let binding = binding.filter(|binding| binding.is_object())?;
    let mut out = Map::new();
    put_str(&mut out, "endpoint", get_str(binding, &["endpoint"]));
    put_str(&mut out, "cluster", get_str(binding, &["cluster"]));
    if let Some(auth) = get(binding, &["auth"]) {
        out.insert("auth".into(), compact_auth(auth));
    }
    Some(Value::Object(out))
}

/// Endpoint and auth of a program descriptor's Program Read binding.
fn program_read_transport(program: &Value) -> Option<Value> {
    binding_transport(program.pointer("/transport/binding"))
}

fn program_install_name(program: &Value) -> &str {
    get_str(program, &["installName", "install_name"]).unwrap_or("unknown")
}

/// Accepted key classes, scopes, and whether a transaction entitlement is
/// required, for every surface a stack or program descriptor exposes. Read
/// only from fields the descriptor already carries.
pub fn auth_requirements(descriptor: &Value) -> Value {
    let mut out = Map::new();
    let mut classes: Vec<String> = Vec::new();
    let mut entitlement = false;
    let mut add = |key: &str, auth: Option<&Value>, out: &mut Map<String, Value>| {
        let Some(auth) = auth.filter(|auth| auth.is_object()) else {
            return;
        };
        let compact = compact_auth(auth);
        classes.extend(string_values(get_array(&compact, &["acceptedKeyClasses"])));
        entitlement |= first_bool(&compact, &["transactionEntitlementRequired"]);
        out.insert(key.to_string(), compact);
    };
    add("stream", get(descriptor, &["websocketAuth"]), &mut out);
    add("query", get(descriptor, &["httpAuth"]), &mut out);
    add("chain", descriptor.pointer("/chainBinding/auth"), &mut out);
    add(
        "transaction",
        descriptor.pointer("/transactionBinding/auth"),
        &mut out,
    );
    // A program descriptor carries its own Program Read binding; a stack
    // descriptor carries one per embedded program.
    add(
        "programRead",
        descriptor.pointer("/transport/binding/auth"),
        &mut out,
    );
    let mut program_reads = Vec::new();
    for program in get_array(descriptor, &["programs"]) {
        if let Some(auth) = program
            .pointer("/transport/binding/auth")
            .filter(|auth| auth.is_object())
        {
            let compact = compact_auth(auth);
            classes.extend(string_values(get_array(&compact, &["acceptedKeyClasses"])));
            let mut entry = Map::new();
            entry.insert("program".into(), json!(program_install_name(program)));
            if let Value::Object(fields) = compact {
                entry.extend(fields);
            }
            program_reads.push(Value::Object(entry));
        }
    }
    if !program_reads.is_empty() {
        out.insert("programReads".into(), Value::Array(program_reads));
    }
    if classes.iter().any(|class| class == "publishable") {
        out.insert(
            "browser".into(),
            json!({
                "keyClass": "publishable",
                "originBound": true,
                "originsPerKey": 1,
                "create": "a4 auth keys create-publishable --origin <origin>",
            }),
        );
    }
    out.insert("transactionEntitlementRequired".into(), json!(entitlement));
    Value::Object(out)
}

fn extension_summary(extension: Option<&Value>) -> Option<Value> {
    let extension = extension.filter(|extension| extension.is_object())?;
    let manifest = extension.get("manifest").unwrap_or(&Value::Null);
    let mut out = Map::new();
    put_str(&mut out, "entry", get_str(manifest, &["entry"]));
    put(&mut out, "files", get(manifest, &["files"]).cloned());
    put_str(
        &mut out,
        "sdkRange",
        get_str(manifest, &["sdkRange", "sdk_range"]),
    );
    put_str(
        &mut out,
        "sdkExtensionHash",
        get_str(extension, &["sdkExtensionHash", "sdk_extension_hash"]),
    );
    Some(Value::Object(out))
}

// ---------------------------------------------------------------------------
// Programs
// ---------------------------------------------------------------------------

fn program_idl(program: &Value) -> &Value {
    program
        .pointer("/definition/idlPayload")
        .unwrap_or(&Value::Null)
}

fn program_id(program: &Value) -> Option<&str> {
    program
        .pointer("/definition/programId")
        .and_then(Value::as_str)
}

/// Install name, display name, program id and release identity.
pub fn program_identity(program: &Value) -> Value {
    let mut out = Map::new();
    put_str(&mut out, "installName", get_str(program, &["installName"]));
    put_str(&mut out, "displayName", get_str(program, &["displayName"]));
    put_str(&mut out, "programId", program_id(program));
    put_str(
        &mut out,
        "programSpecHash",
        program
            .pointer("/definition/programSpecHash")
            .and_then(Value::as_str),
    );
    put_str(
        &mut out,
        "programReleaseHash",
        program
            .pointer("/release/programReleaseHash")
            .and_then(Value::as_str),
    );
    Value::Object(out)
}

/// The knowledge layer's SDK surface for one program: every generated and
/// extension operation with its generated path, input type and bindings.
#[derive(Debug, Clone, Default)]
pub struct ProgramSurface {
    entries: Vec<Value>,
}

impl ProgramSurface {
    /// Read a `section=surface` knowledge response. A response that reports a
    /// different program id is refused, so a slug collision can never attach
    /// another program's operations.
    pub fn from_knowledge(response: &Value, program_id: &str) -> Result<Self> {
        if let Some(reported) = get_str(response, &["program_id", "programId"]) {
            if reported != program_id {
                bail!("the knowledge surface describes program {reported}, not {program_id}");
            }
        }
        let mut entries = Vec::new();
        for artifact in get_array(response, &["artifacts"]) {
            for entry in get_array(artifact, &["entries"]) {
                if get_str(entry, &["programId", "program_id"])
                    .is_some_and(|entry_program| entry_program != program_id)
                {
                    continue;
                }
                entries.push(entry.clone());
            }
        }
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[Value] {
        &self.entries
    }
}

/// A program's surface, or why it could not be loaded.
pub type SurfaceState<'a> = std::result::Result<&'a ProgramSurface, &'a str>;

fn entry_kind(entry: &Value) -> &str {
    get_str(entry, &["kind"]).unwrap_or("unknown")
}

fn entry_path(entry: &Value) -> &str {
    get_str(entry, &["path"]).unwrap_or("")
}

fn kind_rank(kind: &str) -> usize {
    KIND_ORDER
        .iter()
        .position(|candidate| *candidate == kind)
        .unwrap_or(KIND_ORDER.len())
}

fn is_build(entry: &Value) -> bool {
    get_str(entry, &["mode"]) == Some("build") || BUILD_KINDS.contains(&entry_kind(entry))
}

fn sorted_entries(surface: &ProgramSurface) -> Vec<&Value> {
    let mut entries = surface.entries.iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        kind_rank(entry_kind(left))
            .cmp(&kind_rank(entry_kind(right)))
            .then_with(|| entry_path(left).cmp(entry_path(right)))
    });
    entries
}

/// One line per surface entry: path, kind and title.
fn operation_list(surface: &ProgramSurface) -> Value {
    Value::Array(
        sorted_entries(surface)
            .into_iter()
            .map(|entry| {
                let mut out = Map::new();
                put_str(&mut out, "path", get_str(entry, &["path"]));
                put_str(&mut out, "kind", get_str(entry, &["kind"]));
                put_str(&mut out, "title", get_str(entry, &["title"]));
                Value::Object(out)
            })
            .collect(),
    )
}

/// Names only: identity, what the IDL declares, the semantic operations the
/// surface lists, transports and the SDK extension.
pub fn program_summary(program: &Value, surface: SurfaceState<'_>) -> Value {
    let idl = program_idl(program);
    let names = |key: &str| {
        Value::Array(
            get_array(idl, &[key])
                .iter()
                .filter_map(|item| get_str(item, &["name"]))
                .map(|name| json!(name))
                .collect(),
        )
    };
    let mut out = Map::new();
    out.insert("kind".into(), json!("program-summary"));
    out.insert("program".into(), program_identity(program));
    out.insert("accounts".into(), names("accounts"));
    out.insert("instructions".into(), names("instructions"));
    out.insert("events".into(), names("events"));
    out.insert("types".into(), names("types"));
    out.insert(
        "errorCount".into(),
        json!(get_array(idl, &["errors"]).len()),
    );
    match surface {
        Ok(surface) => {
            let mut listed = Map::new();
            let mut counts = Map::new();
            for entry in sorted_entries(surface) {
                let kind = entry_kind(entry);
                let count = counts.get(kind).and_then(Value::as_u64).unwrap_or(0) + 1;
                counts.insert(kind.to_string(), json!(count));
                if SUMMARY_LISTED_KINDS.contains(&kind) {
                    if let Value::Array(paths) =
                        listed.entry(kind.to_string()).or_insert_with(|| json!([]))
                    {
                        paths.push(json!(entry_path(entry)));
                    }
                }
            }
            out.insert("operations".into(), Value::Object(listed));
            out.insert("operationCounts".into(), Value::Object(counts));
        }
        Err(reason) => {
            out.insert("operationsUnavailable".into(), json!(reason));
        }
    }
    put(&mut out, "programRead", program_read_transport(program));
    put(
        &mut out,
        "transaction",
        binding_transport(program.get("transactionBinding")),
    );
    put(
        &mut out,
        "sdkExtension",
        extension_summary(program.pointer("/definition/extensions")),
    );
    Value::Object(out)
}

/// The requested sections in full detail. `instructions` also carries the
/// program's error codes.
pub fn program_sections(program: &Value, surface: SurfaceState<'_>, sections: &[String]) -> Value {
    let idl = program_idl(program);
    let mut out = Map::new();
    out.insert("kind".into(), json!("program-sections"));
    out.insert("program".into(), program_identity(program));
    for section in sections {
        match section.as_str() {
            "accounts" => {
                out.insert("accounts".into(), json!(idl_accounts(idl)));
            }
            "instructions" => {
                out.insert("instructions".into(), json!(idl_instructions(idl)));
                out.insert("errors".into(), json!(idl_errors(idl)));
            }
            "events" => {
                out.insert("events".into(), json!(idl_events(idl)));
            }
            "types" => {
                out.insert("types".into(), json!(idl_types(idl)));
            }
            "operations" => match surface {
                Ok(surface) => {
                    out.insert("operations".into(), operation_list(surface));
                }
                Err(reason) => {
                    out.insert("operationsUnavailable".into(), json!(reason));
                }
            },
            _ => {}
        }
    }
    Value::Object(out)
}

fn entry_matches_exactly(entry: &Value, id: &str) -> bool {
    get_str(entry, &["path"]) == Some(id)
        || get_str(entry, &["operationId", "operation_id"]) == Some(id)
        || entry
            .get("targetBindings")
            .and_then(Value::as_object)
            .is_some_and(|bindings| {
                bindings
                    .values()
                    .any(|binding| binding.as_str() == Some(id))
            })
}

fn raw_instruction_name(entry: &Value) -> Option<&str> {
    get_str(entry, &["idlInstruction", "idl_instruction"])
}

fn unique<'a>(candidates: Vec<&'a Value>, id: &str) -> Result<Option<&'a Value>> {
    match candidates.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(one)),
        many => {
            let exact_path = many
                .iter()
                .filter(|entry| get_str(entry, &["path"]) == Some(id))
                .collect::<Vec<_>>();
            if let [one] = exact_path.as_slice() {
                return Ok(Some(one));
            }
            bail!(
                "operation `{id}` is ambiguous; use one of: {}",
                many.iter()
                    .map(|entry| entry_path(entry))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

/// Look `id` up among the program's operations: a surface path, operation
/// id or generated binding first, then a raw IDL instruction name. `Ok(None)`
/// means no match.
pub fn find_program_operation(
    program: &Value,
    surface: SurfaceState<'_>,
    id: &str,
) -> Result<Option<Value>> {
    let id = id.trim();
    if id.is_empty() {
        bail!("operation id must not be empty");
    }
    if let Ok(surface) = surface {
        let exact = surface
            .entries
            .iter()
            .filter(|entry| entry_matches_exactly(entry, id))
            .collect::<Vec<_>>();
        let found = match unique(exact, id)? {
            Some(entry) => Some(entry),
            None => unique(
                surface
                    .entries
                    .iter()
                    .filter(|entry| {
                        raw_instruction_name(entry).is_some_and(|name| loose(name) == loose(id))
                    })
                    .collect(),
                id,
            )?,
        };
        if let Some(entry) = found {
            return Ok(Some(operation_output(
                program,
                surface_operation(program, entry),
            )));
        }
    }
    let idl = program_idl(program);
    let instructions = get_array(idl, &["instructions"])
        .iter()
        .filter(|instruction| {
            get_str(instruction, &["name"]).is_some_and(|name| loose(name) == loose(id))
        })
        .collect::<Vec<_>>();
    if let [instruction] = instructions.as_slice() {
        return Ok(Some(operation_output(
            program,
            idl_operation(program, instruction, surface.err()),
        )));
    }
    Ok(None)
}

/// [`find_program_operation`], with a not-found error that lists near matches.
pub fn program_operation(program: &Value, surface: SurfaceState<'_>, id: &str) -> Result<Value> {
    if let Some(operation) = find_program_operation(program, surface, id)? {
        return Ok(operation);
    }
    bail!("{}", operation_not_found(program, surface, id.trim()))
}

fn operation_not_found(program: &Value, surface: SurfaceState<'_>, id: &str) -> String {
    let name = program_install_name(program);
    let needle = loose(id.rsplit('.').next().unwrap_or(id));
    let mut candidates: Vec<String> = match surface {
        Ok(surface) => sorted_entries(surface)
            .into_iter()
            .map(|entry| entry_path(entry).to_string())
            .collect(),
        Err(_) => Vec::new(),
    };
    candidates.extend(
        get_array(program_idl(program), &["instructions"])
            .iter()
            .filter_map(|instruction| get_str(instruction, &["name"]))
            .map(str::to_string),
    );
    let mut near = candidates
        .iter()
        .filter(|candidate| !needle.is_empty() && loose(candidate).contains(&needle))
        .take(MAX_SUGGESTIONS)
        .cloned()
        .collect::<Vec<_>>();
    if near.is_empty() {
        near = candidates.into_iter().take(MAX_SUGGESTIONS).collect();
    }
    let mut message = format!("No operation `{id}` in program `{name}`.");
    if !near.is_empty() {
        message.push_str(&format!(" Try one of: {}.", near.join(", ")));
    }
    match surface {
        Ok(_) => message.push_str(" List every operation with the `operations` section."),
        Err(reason) => message.push_str(&format!(
            " Semantic operations (transactions.*, instructions.*, read.*) come from the knowledge surface, which is unavailable: {reason}. Raw IDL instruction names still resolve."
        )),
    }
    message
}

fn operation_output(program: &Value, operation: Map<String, Value>) -> Value {
    json!({
        "kind": "program-operation",
        "program": program_identity(program),
        "operation": Value::Object(operation),
    })
}

fn is_pubkey_type(field_type: &str) -> bool {
    matches!(
        field_type.to_ascii_lowercase().as_str(),
        "pubkey" | "publickey" | "address"
    )
}

/// Surface input with empty docs dropped; unit metadata passes through.
fn compact_input(input: &Value) -> Option<Value> {
    let input = input.as_object()?;
    let mut out = Map::new();
    put_str(
        &mut out,
        "typeName",
        input.get("typeName").and_then(Value::as_str),
    );
    let fields = input
        .get("fields")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    out.insert(
        "fields".into(),
        Value::Array(
            fields
                .iter()
                .map(|field| {
                    let mut compact = Map::new();
                    put_str(&mut compact, "name", get_str(field, &["name"]));
                    put(&mut compact, "type", get(field, &["type"]).cloned());
                    compact.insert(
                        "optional".into(),
                        json!(first_bool(
                            field,
                            &["optional", "isOptional", "is_optional"]
                        )),
                    );
                    if let Some(doc) = get_str(field, &["doc"]).filter(|doc| !doc.trim().is_empty())
                    {
                        compact.insert("doc".into(), json!(doc));
                    }
                    for key in ["unit", "units", "decimals"] {
                        put(&mut compact, key, get(field, &[key]).cloned());
                    }
                    Value::Object(compact)
                })
                .collect(),
        ),
    );
    Some(Value::Object(out))
}

fn input_fields(input: Option<&Value>) -> &[Value] {
    input
        .and_then(|input| input.get("fields"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn has_unit_metadata(fields: &[Value]) -> bool {
    fields.iter().any(|field| {
        ["unit", "units", "decimals"]
            .iter()
            .any(|key| get(field, &[key]).is_some())
    })
}

/// Required and derived accounts of a raw instruction, from the ProgramSpec's
/// account resolutions when present and the IDL flags otherwise.
fn raw_accounts(program: &Value, instruction_name: &str) -> Option<(Value, Vec<String>)> {
    let spec = program
        .pointer("/definition/programSpec/payload/instructions")
        .and_then(Value::as_array)
        .and_then(|instructions| {
            instructions.iter().find(|instruction| {
                get_str(instruction, &["name"])
                    .is_some_and(|name| loose(name) == loose(instruction_name))
            })
        });
    let idl = get_array(program_idl(program), &["instructions"])
        .iter()
        .find(|instruction| {
            get_str(instruction, &["name"])
                .is_some_and(|name| loose(name) == loose(instruction_name))
        });
    let source = spec.or(idl)?;
    let mut flattened = Vec::new();
    flatten_accounts(get_array(source, &["accounts"]), "", &mut flattened);

    let mut required = Vec::new();
    let mut optional = Vec::new();
    let mut derived = Vec::new();
    let mut signers = Vec::new();
    for (name, account) in flattened {
        let resolution = account.get("resolution").unwrap_or(&Value::Null);
        let category = get_str(resolution, &["category", "kind"]);
        let address = get_str(account, &["address"]).or_else(|| get_str(resolution, &["address"]));
        let signer = is_signer(account) || category == Some("signer");
        if signer {
            signers.push(name.clone());
        }
        let brief = json!({ "name": name, "writable": is_writable(account) });
        if let Some(address) = address.filter(|_| !signer) {
            derived
                .push(json!({ "name": name, "derivation": "fixed-address", "address": address }));
        } else if !signer && (category == Some("pda") || resolution.get("seeds").is_some()) {
            let mut entry = json!({ "name": name, "derivation": "pda" });
            if let Some(seeds) = resolution.get("seeds") {
                entry["seeds"] = seeds.clone();
            }
            derived.push(entry);
        } else if let Some(other) = category.filter(|category| {
            !signer
                && !matches!(
                    *category,
                    "signer" | "userProvided" | "user_provided" | "user-provided"
                )
        }) {
            derived.push(json!({ "name": name, "derivation": other }));
        } else if is_optional(account) {
            optional.push(brief);
        } else {
            required.push(brief);
        }
    }
    Some((
        json!({ "required": required, "optional": optional, "derived": derived }),
        signers,
    ))
}

/// Required and derived accounts of a semantic operation, from its input:
/// required address inputs must be passed; optional ones are resolved by the
/// operation when omitted.
fn semantic_accounts(fields: &[Value]) -> (Value, Vec<String>) {
    let mut required = Vec::new();
    let mut derived = Vec::new();
    let mut signers = Vec::new();
    for field in fields {
        let Some(name) = get_str(field, &["name"]) else {
            continue;
        };
        if !get_str(field, &["type"]).is_some_and(is_pubkey_type) {
            continue;
        }
        if name
            .rsplit('.')
            .next()
            .is_some_and(|last| last.eq_ignore_ascii_case("signer"))
        {
            signers.push(name.to_string());
        }
        if first_bool(field, &["optional", "isOptional", "is_optional"]) {
            derived.push(json!({ "name": name, "derivation": "resolved-when-omitted" }));
        } else {
            required.push(json!({ "name": name }));
        }
    }
    (json!({ "required": required, "derived": derived }), signers)
}

/// Transactions an operation's `prepare` returns, from the SDK contract:
/// instruction and transaction operations prepare exactly one; a flow
/// decides its count while preparing.
fn transaction_count(kind: &str) -> Option<u64> {
    matches!(kind, "transaction" | "instruction" | "raw-instruction").then_some(1)
}

fn transport_for(program: &Value, kind: &str, build: bool) -> Option<Value> {
    if build {
        binding_transport(program.get("transactionBinding"))
    } else if matches!(kind, "read" | "account-fetch") {
        program_read_transport(program)
    } else {
        None
    }
}

/// The TypeScript program key: the install name in lower camel case, as the
/// generated `session.programs.<key>` uses it.
fn program_key(install_name: &str) -> String {
    let mut key = String::new();
    for (index, part) in install_name
        .split(['-', '_', ' ', '.'])
        .filter(|part| !part.is_empty())
        .enumerate()
    {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            if index == 0 {
                key.extend(first.to_lowercase());
            } else {
                key.extend(first.to_uppercase());
            }
            key.push_str(chars.as_str());
        }
    }
    key
}

/// Top-level required input names, for a minimal call.
fn required_input_names(fields: &[Value]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for field in fields {
        if first_bool(field, &["optional", "isOptional", "is_optional"]) {
            continue;
        }
        if let Some(name) = get_str(field, &["name"]) {
            let top = name.split('.').next().unwrap_or(name).to_string();
            if !names.contains(&top) {
                names.push(top);
            }
        }
    }
    names
}

fn usage(program: &Value, kind: &str, ts_path: &str, fields: &[Value]) -> Option<Value> {
    let key = program_key(program_install_name(program));
    let names = required_input_names(fields);
    let args = if names.is_empty() {
        "{}".to_string()
    } else {
        format!("{{ {} }}", names.join(", "))
    };
    match kind {
        "transaction" | "instruction" | "flow" => Some(json!({
            "typescript": format!("const prepared = await session.programs.{key}.{ts_path}.prepare({args});"),
            "react": format!("const mutation = arete.programs.{key}.{ts_path}.useMutation();"),
        })),
        "raw-instruction" => Some(json!({
            "typescript": format!("const instruction = session.programs.{key}.{ts_path}.build({args});"),
        })),
        "account-fetch" => Some(json!({
            "typescript": format!("const account = await session.programs.{key}.{ts_path}.fetch(address);"),
        })),
        _ => None,
    }
}

/// The keys an operation of this shape could carry; those missing from the
/// output are reported as `notReported` so an agent knows they were not
/// available rather than dropped.
fn expected_keys(kind: &str, build: bool) -> &'static [&'static str] {
    if kind == "raw-instruction" {
        // Raw builders run offline: nothing is read while building.
        &[
            "generatedPaths",
            "input",
            "output",
            "units",
            "accounts",
            "signers",
            "transactionCount",
            "errors",
            "transport",
            "usage",
        ]
    } else if build {
        &[
            "generatedPaths",
            "input",
            "output",
            "units",
            "accounts",
            "signers",
            "readsDuringPrepare",
            "transactionCount",
            "errors",
            "transport",
            "usage",
        ]
    } else if kind == "account-fetch" {
        &["generatedPaths", "output", "transport", "usage"]
    } else if kind == "read" {
        &["generatedPaths", "input", "output", "units", "transport"]
    } else {
        &["generatedPaths"]
    }
}

fn finish(mut out: Map<String, Value>, expected: &[&str]) -> Map<String, Value> {
    let missing = expected
        .iter()
        .filter(|key| !out.contains_key(**key))
        .map(|key| json!(key))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        out.insert("notReported".into(), Value::Array(missing));
    }
    out
}

fn surface_operation(program: &Value, entry: &Value) -> Map<String, Value> {
    let kind = entry_kind(entry).to_string();
    let build = is_build(entry);
    let mut out = Map::new();
    put_str(&mut out, "path", get_str(entry, &["path"]));
    put_str(
        &mut out,
        "operationId",
        get_str(entry, &["operationId", "operation_id"]),
    );
    out.insert("kind".into(), json!(kind));
    put_str(&mut out, "mode", get_str(entry, &["mode"]));
    put_str(&mut out, "title", get_str(entry, &["title"]));
    put_str(
        &mut out,
        "doc",
        get_str(entry, &["doc"]).filter(|doc| !doc.is_empty()),
    );
    put_str(
        &mut out,
        "docDetails",
        get_str(entry, &["docDetails"]).filter(|doc| !doc.is_empty()),
    );
    put_str(&mut out, "origin", get_str(entry, &["origin"]));
    let concepts = get_array(entry, &["concepts"]);
    if !concepts.is_empty() {
        out.insert("concepts".into(), json!(string_values(concepts)));
    }
    let bindings = entry
        .get("targetBindings")
        .filter(|bindings| bindings.as_object().is_some_and(|map| !map.is_empty()));
    put(&mut out, "generatedPaths", bindings.cloned());
    let input = entry.get("input");
    put(&mut out, "input", input.and_then(compact_input));
    let fields = input_fields(input);
    if has_unit_metadata(fields) {
        out.insert("units".into(), json!("see input.fields"));
    }
    put(
        &mut out,
        "output",
        get(entry, &["output", "returns", "outputType"]).cloned(),
    );
    for key in ["account", "value", "valueType"] {
        put(&mut out, key, get(entry, &[key]).cloned());
    }

    if kind == "raw-instruction" {
        let instruction = raw_instruction_name(entry)
            .map(str::to_string)
            .or_else(|| entry_path(entry).rsplit('.').next().map(str::to_string));
        if let Some((accounts, signers)) = instruction
            .as_deref()
            .and_then(|name| raw_accounts(program, name))
        {
            out.insert("accounts".into(), accounts);
            out.insert("signers".into(), json!(signers));
        }
    } else if build {
        let (accounts, signers) = semantic_accounts(fields);
        out.insert("accounts".into(), accounts);
        out.insert("signers".into(), json!(signers));
    }
    put(
        &mut out,
        "readsDuringPrepare",
        get(entry, &["readsDuringPrepare", "prepareReads", "reads"]).cloned(),
    );
    if build {
        put(
            &mut out,
            "transactionCount",
            transaction_count(&kind).map(|count| json!(count)),
        );
        let mut errors = idl_errors(program_idl(program))
            .into_iter()
            .map(|error| json!(error))
            .collect::<Vec<_>>();
        errors.extend(get_array(entry, &["errors"]).iter().cloned());
        if !errors.is_empty() {
            out.insert("errors".into(), Value::Array(errors));
        }
    }
    put(&mut out, "transport", transport_for(program, &kind, build));
    let ts_path = bindings
        .and_then(|bindings| bindings.get("typescript"))
        .and_then(Value::as_str);
    put(
        &mut out,
        "usage",
        ts_path.and_then(|ts_path| usage(program, &kind, ts_path, fields)),
    );
    finish(out, expected_keys(&kind, build))
}

/// An operation known only from the IDL: the raw instruction's arguments and
/// accounts. Generated paths come from the knowledge surface, so without it
/// they are reported as unavailable rather than guessed.
fn idl_operation(
    program: &Value,
    instruction: &Value,
    surface_reason: Option<&str>,
) -> Map<String, Value> {
    let name = name_of(instruction);
    let mut out = Map::new();
    out.insert("name".into(), json!(name));
    out.insert("kind".into(), json!("raw-instruction"));
    out.insert("mode".into(), json!("build"));
    out.insert("source".into(), json!("idl"));
    let docs = get_array(instruction, &["docs"]);
    if !docs.is_empty() {
        out.insert("doc".into(), json!(string_values(docs).join("\n")));
    }
    out.insert("arguments".into(), json!(named_types(instruction, "args")));
    if let Some((accounts, signers)) = raw_accounts(program, &name) {
        out.insert("accounts".into(), accounts);
        out.insert("signers".into(), json!(signers));
    }
    out.insert("transactionCount".into(), json!(1));
    let errors = idl_errors(program_idl(program));
    if !errors.is_empty() {
        out.insert("errors".into(), json!(errors));
    }
    put(
        &mut out,
        "transport",
        transport_for(program, "raw-instruction", true),
    );
    if let Some(reason) = surface_reason {
        out.insert("surfaceUnavailable".into(), json!(reason));
    }
    // The raw arguments stand in for the generated input type.
    let expected = expected_keys("raw-instruction", true)
        .iter()
        .copied()
        .filter(|key| *key != "input")
        .collect::<Vec<_>>();
    finish(out, &expected)
}

// ---------------------------------------------------------------------------
// Stacks
// ---------------------------------------------------------------------------

/// One field of a LiveSpec entity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntityField {
    pub section: String,
    pub path: String,
    pub rust_type: String,
    pub nullable: bool,
}

/// An entity's state name (`stateName` or `state_name`).
pub fn entity_name(entity: &Value) -> Option<&str> {
    get_str(entity, &["stateName", "state_name"])
}

/// An entity's program id (`programId` or `program_id`).
pub fn entity_program_id(entity: &Value) -> Option<&str> {
    get_str(entity, &["programId", "program_id"])
}

/// An entity's primary key paths.
pub fn entity_primary_keys(entity: &Value) -> Vec<String> {
    entity
        .get("identity")
        .map(|identity| string_values(get_array(identity, &["primaryKeys", "primary_keys"])))
        .unwrap_or_default()
}

/// An entity's emitted fields, as `section.field` paths.
pub fn entity_fields(entity: &Value) -> Vec<EntityField> {
    let mut fields = Vec::new();
    for section in get_array(entity, &["sections"]) {
        let section_name = get_str(section, &["name"]).unwrap_or("fields");
        for field in get_array(section, &["fields"]) {
            if get_bool(field, &["emit"]) == Some(false) {
                continue;
            }
            let name = get_str(field, &["fieldName", "field_name"]).unwrap_or("unknown");
            fields.push(EntityField {
                section: section_name.into(),
                path: format!("{section_name}.{name}"),
                rust_type: get_str(field, &["rustTypeName", "rust_type_name"])
                    .unwrap_or("unknown")
                    .into(),
                nullable: first_bool(field, &["isOptional", "is_optional"]),
            });
        }
    }
    fields
}

/// The entities of one LiveSpec artifact.
pub fn live_entities(artifact: &Value) -> &[Value] {
    artifact
        .pointer("/payload/entities")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// The entity and view with `view_id` in a LiveSpec artifact.
pub fn find_view<'a>(artifact: &'a Value, view_id: &str) -> Option<(&'a Value, &'a Value)> {
    live_entities(artifact).iter().find_map(|entity| {
        get_array(entity, &["views"])
            .iter()
            .find(|view| get_str(view, &["id"]) == Some(view_id))
            .map(|view| (entity, view))
    })
}

/// `Collection`, `{"Keyed": …}` and similar view outputs, as a lowercase
/// label.
fn output_label(output: Option<&Value>) -> Option<String> {
    match output? {
        Value::String(label) => Some(label.to_ascii_lowercase()),
        Value::Object(map) => map.keys().next().map(|key| key.to_ascii_lowercase()),
        _ => None,
    }
}

fn live_specs(descriptor: &Value) -> &[Value] {
    get_array(descriptor, &["liveSpecs"])
}

/// A selected view: its LiveSpec alias and view id, as the StackManifest
/// lists them.
fn selected_view_refs(descriptor: &Value) -> Vec<(String, String)> {
    let lives = live_specs(descriptor);
    descriptor
        .pointer("/stackManifest/payload/selectedViews")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|entry| {
            let view_id = get_str(entry, &["viewId", "view_id"])?;
            let alias = get_str(entry, &["liveAlias", "live_alias"])
                .map(str::to_string)
                .or_else(|| {
                    let hash = get_str(entry, &["liveSpecHash", "live_spec_hash"])?;
                    lives
                        .iter()
                        .find(|live| get_str(live, &["liveSpecHash"]) == Some(hash))
                        .and_then(|live| get_str(live, &["alias"]))
                        .map(str::to_string)
                })
                .or_else(|| {
                    (lives.len() == 1)
                        .then(|| get_str(&lives[0], &["alias"]).map(str::to_string))
                        .flatten()
                })?;
            Some((alias, view_id.to_string()))
        })
        .collect()
}

fn live_by_alias<'a>(descriptor: &'a Value, alias: &str) -> Option<&'a Value> {
    live_specs(descriptor)
        .iter()
        .find(|live| get_str(live, &["alias"]) == Some(alias))
}

fn live_endpoint(live: &Value) -> Value {
    let binding = live.get("binding").unwrap_or(&Value::Null);
    let mut out = Map::new();
    put_str(&mut out, "alias", get_str(live, &["alias"]));
    put_str(&mut out, "liveSpecHash", get_str(live, &["liveSpecHash"]));
    put_str(
        &mut out,
        "websocket",
        get_str(binding, &["websocketEndpoint", "websocket_endpoint"]),
    );
    put_str(
        &mut out,
        "query",
        get_str(binding, &["queryEndpoint", "query_endpoint"]),
    );
    Value::Object(out)
}

fn stack_header(descriptor: &Value, kind: &str) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("kind".into(), json!(kind));
    put_str(&mut out, "name", get_str(descriptor, &["name"]));
    put_str(&mut out, "stack", get_str(descriptor, &["stack"]));
    put_str(
        &mut out,
        "description",
        get_str(descriptor, &["description"]),
    );
    put_str(&mut out, "visibility", get_str(descriptor, &["visibility"]));
    put_str(
        &mut out,
        "stackManifestHash",
        get_str(descriptor, &["stackManifestHash"]),
    );
    out
}

/// Entities with their selected views, program SDKs, endpoints and auth
/// requirements. No artifact bodies.
pub fn stack_summary(descriptor: &Value) -> Value {
    let mut out = stack_header(descriptor, "stack-summary");
    let selected = selected_view_refs(descriptor);

    let mut entities = Vec::new();
    for live in live_specs(descriptor) {
        let alias = get_str(live, &["alias"]).unwrap_or("unknown");
        let artifact = live.get("artifact").unwrap_or(&Value::Null);
        for entity in live_entities(artifact) {
            let views = get_array(entity, &["views"])
                .iter()
                .filter_map(|view| {
                    let id = get_str(view, &["id"])?;
                    selected
                        .iter()
                        .any(|(selected_alias, selected_id)| {
                            selected_alias == alias && selected_id == id
                        })
                        .then(|| {
                            let mut entry = Map::new();
                            entry.insert("id".into(), json!(id));
                            put(
                                &mut entry,
                                "output",
                                output_label(view.get("output")).map(Value::String),
                            );
                            Value::Object(entry)
                        })
                })
                .collect::<Vec<_>>();
            let mut summary = Map::new();
            summary.insert("liveAlias".into(), json!(alias));
            put_str(&mut summary, "name", entity_name(entity));
            summary.insert("primaryKeys".into(), json!(entity_primary_keys(entity)));
            summary.insert("fieldCount".into(), json!(entity_fields(entity).len()));
            summary.insert("views".into(), Value::Array(views));
            entities.push(Value::Object(summary));
        }
    }
    out.insert("entities".into(), Value::Array(entities));

    let programs = get_array(descriptor, &["programs"])
        .iter()
        .map(|program| {
            let mut summary = match program_identity(program) {
                Value::Object(map) => map,
                _ => Map::new(),
            };
            put(
                &mut summary,
                "sdkExtension",
                extension_summary(program.pointer("/definition/extensions")),
            );
            Value::Object(summary)
        })
        .collect::<Vec<_>>();
    out.insert("programs".into(), Value::Array(programs));
    put(
        &mut out,
        "stackExtension",
        extension_summary(descriptor.get("extensions")),
    );

    let mut endpoints = Map::new();
    endpoints.insert(
        "liveSpecs".into(),
        Value::Array(live_specs(descriptor).iter().map(live_endpoint).collect()),
    );
    put_str(
        &mut endpoints,
        "chain",
        descriptor
            .pointer("/chainBinding/endpoint")
            .and_then(Value::as_str),
    );
    put_str(
        &mut endpoints,
        "transaction",
        descriptor
            .pointer("/transactionBinding/endpoint")
            .and_then(Value::as_str),
    );
    let program_reads = get_array(descriptor, &["programs"])
        .iter()
        .filter_map(|program| {
            let endpoint = program
                .pointer("/transport/binding/endpoint")
                .and_then(Value::as_str)?;
            Some(json!({ "program": program_install_name(program), "endpoint": endpoint }))
        })
        .collect::<Vec<_>>();
    if !program_reads.is_empty() {
        endpoints.insert("programReads".into(), Value::Array(program_reads));
    }
    out.insert("endpoints".into(), Value::Object(endpoints));
    out.insert("auth".into(), auth_requirements(descriptor));
    Value::Object(out)
}

/// Only the requested selected views, each with its entity schema. A view is
/// `Entity/view`, or `alias:Entity/view` when several LiveSpecs select the
/// same id.
pub fn stack_views<S: AsRef<str>>(descriptor: &Value, views: &[S]) -> Result<Value> {
    let requested = split_list(views);
    if requested.is_empty() {
        bail!("`views` must name at least one view, e.g. `OreRound/latest`");
    }
    let selected = selected_view_refs(descriptor);
    let available = || {
        selected
            .iter()
            .map(|(alias, id)| {
                if selected.iter().filter(|(_, other)| other == id).count() > 1 {
                    format!("{alias}:{id}")
                } else {
                    id.clone()
                }
            })
            .collect::<Vec<_>>()
    };
    let mut out_views = Vec::new();
    for request in &requested {
        let (alias, id) = match request.split_once(':') {
            Some((alias, id)) => (Some(alias.trim()), id.trim()),
            None => (None, request.as_str()),
        };
        let mut matches = selected
            .iter()
            .filter(|(candidate_alias, candidate_id)| {
                candidate_id == id && alias.is_none_or(|alias| alias == candidate_alias)
            })
            .collect::<Vec<_>>();
        if matches.is_empty() {
            matches = selected
                .iter()
                .filter(|(candidate_alias, candidate_id)| {
                    candidate_id.eq_ignore_ascii_case(id)
                        && alias.is_none_or(|alias| alias.eq_ignore_ascii_case(candidate_alias))
                })
                .collect();
        }
        let (live_alias, view_id) = match matches.as_slice() {
            [one] => *one,
            [] => bail!(
                "view `{request}` is not selected by this stack. Selected views: {}",
                available().join(", ")
            ),
            many => bail!(
                "view `{request}` is selected under several LiveSpecs; prefix the alias: {}",
                many.iter()
                    .map(|(alias, id)| format!("{alias}:{id}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let live = live_by_alias(descriptor, live_alias);
        let artifact = live
            .and_then(|live| live.get("artifact"))
            .unwrap_or(&Value::Null);
        let mut entry = Map::new();
        entry.insert("liveAlias".into(), json!(live_alias));
        entry.insert("id".into(), json!(view_id));
        match find_view(artifact, view_id) {
            Some((entity, view)) => {
                put_str(&mut entry, "entity", entity_name(entity));
                put(&mut entry, "output", view.get("output").cloned());
                put(&mut entry, "pipeline", view.get("pipeline").cloned());
                entry.insert("primaryKeys".into(), json!(entity_primary_keys(entity)));
                entry.insert("fields".into(), json!(entity_fields(entity)));
            }
            None => {
                entry.insert(
                    "schemaUnavailable".into(),
                    json!("the view is selected but absent from its LiveSpec artifact"),
                );
            }
        }
        if let Some(live) = live {
            let endpoint = live_endpoint(live);
            let mut compact = Map::new();
            put(
                &mut compact,
                "websocket",
                endpoint.get("websocket").cloned(),
            );
            put(&mut compact, "query", endpoint.get("query").cloned());
            entry.insert("endpoint".into(), Value::Object(compact));
        }
        out_views.push(Value::Object(entry));
    }
    let mut out = stack_header(descriptor, "stack-views");
    out.insert("views".into(), Value::Array(out_views));
    Ok(Value::Object(out))
}

/// Merge a stack's chain and transaction bindings into one of its program
/// descriptors, so an operation looked up through the stack reports the
/// transport the stack's SDK uses.
pub fn program_in_stack(stack: &Value, program: &Value) -> Value {
    let mut program = program.clone();
    if let Value::Object(map) = &mut program {
        for key in ["chainBinding", "transactionBinding"] {
            if map.get(key).is_none_or(Value::is_null) {
                if let Some(binding) = stack.get(key).filter(|binding| !binding.is_null()) {
                    map.insert(key.to_string(), binding.clone());
                }
            }
        }
    }
    program
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program() -> Value {
        json!({
            "installName": "demo-program",
            "displayName": "Demo",
            "definition": {
                "programId": "Demo111",
                "programSpecHash": "spec-exact",
                "idlPayload": {
                    "accounts": [{"name": "Vault", "discriminator": [1, 2], "fields": [{"name": "amount", "type": "u64"}]}],
                    "instructions": [{
                        "name": "set_value",
                        "docs": ["Sets the value."],
                        "args": [{"name": "value", "type": "u64"}],
                        "accounts": [
                            {"name": "vault", "writable": true},
                            {"name": "authority", "signer": true},
                            {"name": "systemProgram", "address": "11111111111111111111111111111111"},
                            {"name": "audit", "optional": true}
                        ]
                    }],
                    "events": [{"name": "ValueSet", "fields": [{"name": "value", "type": "u64"}]}],
                    "types": [{"name": "Mode", "type": {"kind": "enum", "variants": [{"name": "On"}]}}],
                    "errors": [{"code": 0, "name": "TooSmall", "msg": "Amount too small"}]
                },
                "programSpec": {"payload": {"instructions": [{
                    "name": "set_value",
                    "accounts": [
                        {"name": "vault", "is_writable": true, "resolution": {"category": "userProvided"}},
                        {"name": "authority", "is_signer": true, "resolution": {"category": "signer"}},
                        {"name": "systemProgram", "resolution": {"category": "known", "address": "11111111111111111111111111111111"}},
                        {"name": "config", "resolution": {"category": "pda", "seeds": ["config"]}},
                        {"name": "audit", "is_optional": true, "resolution": {"category": "userProvided"}}
                    ]
                }]}},
                "extensions": {"manifest": {"entry": "demo.ts", "files": ["demo.ts"], "sdkRange": "^0.23.0"}, "files": {"demo.ts": "export {}"}}
            },
            "release": {"programReleaseHash": "release-exact"},
            "transport": {"kind": "hosted-binding", "binding": {"endpoint": "https://read.test", "auth": {"required": true, "scopes": ["read"], "acceptedKeyClasses": ["anonymous", "publishable", "secret"]}}},
            "transactionBinding": {"endpoint": "https://tx.test", "auth": {"required": true, "scopes": ["transaction:inspect", "transaction:send"], "acceptedKeyClasses": ["publishable", "secret"], "transactionEntitlementRequired": true}}
        })
    }

    fn surface() -> ProgramSurface {
        ProgramSurface::from_knowledge(
            &json!({
                "program_id": "Demo111",
                "artifacts": [{"entries": [
                    {
                        "kind": "transaction", "mode": "build", "path": "transactions.vault.setWithCheck",
                        "operationId": "program/Demo111/transaction/transactions.vault.setWithCheck",
                        "title": "Set with check", "doc": "Sets after checking.",
                        "targetBindings": {"typescript": "transactions.vault.setWithCheck"},
                        "input": {"typeName": "SetInput", "fields": [
                            {"name": "signer", "type": "pubkey", "optional": false, "doc": ""},
                            {"name": "vault", "type": "pubkey", "optional": true, "doc": ""},
                            {"name": "amount", "type": "amount", "optional": false, "doc": "How much."},
                            {"name": "check.signer", "type": "pubkey", "optional": true, "doc": ""}
                        ]},
                        "programId": "Demo111"
                    },
                    {
                        "kind": "raw-instruction", "mode": "build", "path": "rawInstructions.setValue",
                        "idlInstruction": "set_value",
                        "targetBindings": {"typescript": "raw.setValue", "rust": "set_value"},
                        "input": {"typeName": "SetValueParams", "fields": [{"name": "value", "type": "u64", "optional": false}]},
                        "programId": "Demo111"
                    },
                    {"kind": "read", "mode": "read", "path": "read.vault", "title": "Vault", "targetBindings": {"typescript": "read.vault"}, "programId": "Demo111"},
                    {"kind": "constant", "mode": "read", "path": "constants.MAX", "value": 5, "programId": "Demo111"},
                    {"kind": "read", "mode": "read", "path": "read.other", "programId": "Other111"}
                ]}]
            }),
            "Demo111",
        )
        .unwrap()
    }

    #[test]
    fn surface_refuses_another_program_and_drops_foreign_entries() {
        assert!(
            ProgramSurface::from_knowledge(&json!({"program_id": "Other"}), "Demo111").is_err()
        );
        assert_eq!(surface().entries().len(), 4);
    }

    #[test]
    fn semantic_operation_reports_paths_accounts_transport_and_usage() {
        let surface = surface();
        let output =
            program_operation(&program(), Ok(&surface), "transactions.vault.setWithCheck").unwrap();
        let operation = &output["operation"];
        assert_eq!(output["program"]["programReleaseHash"], "release-exact");
        assert_eq!(
            operation["generatedPaths"]["typescript"],
            "transactions.vault.setWithCheck"
        );
        assert_eq!(
            operation["accounts"]["required"],
            json!([{"name": "signer"}])
        );
        assert_eq!(operation["accounts"]["derived"][0]["name"], "vault");
        assert_eq!(operation["signers"], json!(["signer", "check.signer"]));
        assert_eq!(operation["transactionCount"], 1);
        assert_eq!(operation["errors"][0]["name"], "TooSmall");
        assert_eq!(
            operation["transport"]["auth"]["transactionEntitlementRequired"],
            true
        );
        assert_eq!(
            operation["usage"]["typescript"],
            "const prepared = await session.programs.demoProgram.transactions.vault.setWithCheck.prepare({ signer, amount });"
        );
        assert!(operation["input"]["fields"][0].get("doc").is_none());
        assert_eq!(operation["input"]["fields"][2]["doc"], "How much.");
        assert_eq!(
            operation["notReported"],
            json!(["output", "units", "readsDuringPrepare"])
        );
        // The full operation id and the generated binding find the same entry.
        for id in [
            "program/Demo111/transaction/transactions.vault.setWithCheck",
            " transactions.vault.setWithCheck ",
        ] {
            assert_eq!(
                program_operation(&program(), Ok(&surface), id).unwrap(),
                output
            );
        }
    }

    #[test]
    fn raw_instruction_names_resolve_with_spec_account_resolutions() {
        let surface = surface();
        for id in [
            "set_value",
            "setValue",
            "raw.setValue",
            "rawInstructions.setValue",
        ] {
            let output = program_operation(&program(), Ok(&surface), id).unwrap();
            let operation = &output["operation"];
            assert_eq!(operation["path"], "rawInstructions.setValue", "{id}");
            assert_eq!(operation["signers"], json!(["authority"]));
            assert_eq!(
                operation["accounts"]["required"],
                json!([{"name": "vault", "writable": true}, {"name": "authority", "writable": false}])
            );
            assert_eq!(
                operation["accounts"]["optional"],
                json!([{"name": "audit", "writable": false}])
            );
            assert_eq!(
                operation["accounts"]["derived"][0]["derivation"],
                "fixed-address"
            );
            assert_eq!(
                operation["accounts"]["derived"][1]["seeds"],
                json!(["config"])
            );
            assert_eq!(
                operation["usage"]["typescript"],
                "const instruction = session.programs.demoProgram.raw.setValue.build({ value });"
            );
        }
    }

    #[test]
    fn without_a_surface_raw_instructions_come_from_the_idl() {
        let output = program_operation(&program(), Err("no key"), "setValue").unwrap();
        let operation = &output["operation"];
        assert_eq!(operation["source"], "idl");
        assert_eq!(operation["arguments"][0]["name"], "value");
        assert_eq!(operation["surfaceUnavailable"], "no key");
        assert_eq!(operation["transactionCount"], 1);
        assert_eq!(
            operation["notReported"],
            json!(["generatedPaths", "output", "units", "usage"])
        );
        let error = program_operation(&program(), Err("no key"), "transactions.vault.setWithCheck")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("knowledge surface, which is unavailable: no key"),
            "{error}"
        );
    }

    #[test]
    fn unknown_operations_suggest_near_matches() {
        let surface = surface();
        let error = program_operation(&program(), Ok(&surface), "vault.setWith")
            .unwrap_err()
            .to_string();
        assert!(error.contains("transactions.vault.setWithCheck"), "{error}");
        assert!(program_operation(&program(), Ok(&surface), "  ").is_err());
    }

    #[test]
    fn summary_lists_names_semantic_paths_and_no_artifact_bodies() {
        let surface = surface();
        let summary = program_summary(&program(), Ok(&surface));
        assert_eq!(summary["instructions"], json!(["set_value"]));
        assert_eq!(
            summary["operations"]["transaction"],
            json!(["transactions.vault.setWithCheck"])
        );
        assert_eq!(summary["operations"]["read"], json!(["read.vault"]));
        assert_eq!(summary["operationCounts"]["constant"], 1);
        assert_eq!(summary["sdkExtension"]["entry"], "demo.ts");
        assert!(!summary.to_string().contains("export {}"));
        let without = program_summary(&program(), Err("log in"));
        assert_eq!(without["operationsUnavailable"], "log in");
    }

    #[test]
    fn sections_are_validated_and_selected() {
        assert_eq!(
            parse_sections(&["accounts, types", "accounts"], "sections").unwrap(),
            vec!["accounts", "types"]
        );
        let error = parse_sections(&["idl"], "sections")
            .unwrap_err()
            .to_string();
        assert!(error.contains("must be one of"), "{error}");
        let surface = surface();
        let output = program_sections(
            &program(),
            Ok(&surface),
            &["instructions".into(), "operations".into()],
        );
        assert_eq!(output["instructions"][0]["accounts"][1]["signer"], true);
        assert_eq!(output["errors"][0]["message"], "Amount too small");
        assert_eq!(
            output["operations"][0]["path"],
            "transactions.vault.setWithCheck"
        );
        assert!(output.get("accounts").is_none());
    }

    fn stack() -> Value {
        json!({
            "name": "demo",
            "stack": "demo-abc",
            "visibility": "public",
            "stackManifestHash": "manifest-exact",
            "websocketAuth": {"required": true, "accepted_key_classes": ["publishable", "secret"]},
            "liveSpecs": [{
                "alias": "live",
                "liveSpecHash": "live-exact",
                "artifact": {"payload": {"entities": [{
                    "state_name": "Round",
                    "identity": {"primary_keys": ["id.round_id"]},
                    "sections": [{"name": "id", "fields": [
                        {"field_name": "round_id", "rust_type_name": "u64", "is_optional": false},
                        {"field_name": "hidden", "rust_type_name": "u8", "emit": false}
                    ]}],
                    "views": [
                        {"id": "Round/latest", "output": "Collection", "pipeline": [{"Sort": {}}]},
                        {"id": "Round/state", "output": {"Keyed": {}}},
                        {"id": "Round/unselected", "output": "Collection"}
                    ]
                }]}},
                "binding": {"websocketEndpoint": "wss://demo.test", "queryEndpoint": "https://demo.test"}
            }],
            "stackManifest": {"payload": {"selectedViews": [
                {"liveAlias": "live", "viewId": "Round/latest"},
                {"liveAlias": "live", "viewId": "Round/state"}
            ]}},
            "transactionBinding": {"endpoint": "https://tx.test", "auth": {"scopes": ["transaction:send"], "acceptedKeyClasses": ["publishable"], "transactionEntitlementRequired": true}},
            "programs": [program()]
        })
    }

    #[test]
    fn stack_summary_is_compact_and_reads_snake_case_live_specs() {
        let summary = stack_summary(&stack());
        let entity = &summary["entities"][0];
        assert_eq!(entity["name"], "Round");
        assert_eq!(entity["primaryKeys"], json!(["id.round_id"]));
        assert_eq!(entity["fieldCount"], 1);
        assert_eq!(
            entity["views"],
            json!([{"id": "Round/latest", "output": "collection"}, {"id": "Round/state", "output": "keyed"}])
        );
        assert_eq!(summary["programs"][0]["installName"], "demo-program");
        assert_eq!(
            summary["endpoints"]["liveSpecs"][0]["websocket"],
            "wss://demo.test"
        );
        assert_eq!(
            summary["endpoints"]["programReads"][0]["endpoint"],
            "https://read.test"
        );
        assert_eq!(
            summary["auth"]["stream"]["acceptedKeyClasses"],
            json!(["publishable", "secret"])
        );
        assert_eq!(summary["auth"]["browser"]["originsPerKey"], 1);
        assert_eq!(summary["auth"]["transactionEntitlementRequired"], true);
        assert!(!summary.to_string().contains("export {}"));
    }

    #[test]
    fn stack_views_return_only_the_requested_schemas() {
        let output = stack_views(&stack(), &["Round/state"]).unwrap();
        assert_eq!(output["views"].as_array().unwrap().len(), 1);
        let view = &output["views"][0];
        assert_eq!(view["entity"], "Round");
        assert_eq!(
            view["fields"],
            json!([{"section": "id", "path": "id.round_id", "rustType": "u64", "nullable": false}])
        );
        assert_eq!(view["endpoint"]["websocket"], "wss://demo.test");
        assert!(stack_views(&stack(), &["live:round/LATEST"]).is_ok());
        let error = stack_views(&stack(), &["Round/unselected"])
            .unwrap_err()
            .to_string();
        assert!(error.contains("Round/latest, Round/state"), "{error}");
        assert!(stack_views(&stack(), &[" , "]).is_err());
    }

    #[test]
    fn stack_programs_inherit_the_stack_transaction_binding() {
        let mut bare = program();
        bare.as_object_mut().unwrap().remove("transactionBinding");
        let merged = program_in_stack(&stack(), &bare);
        assert_eq!(merged["transactionBinding"]["endpoint"], "https://tx.test");
    }

    #[test]
    fn program_keys_are_lower_camel_case() {
        assert_eq!(program_key("spl-token"), "splToken");
        assert_eq!(program_key("ore"), "ore");
        assert_eq!(program_key("Meteora_cp_amm"), "meteoraCpAmm");
    }
}
