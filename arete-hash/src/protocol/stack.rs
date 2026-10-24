//! The generated protocol stack.
//!
//! [`generate_stack_source`] turns a spec's `stack` surface into an
//! `arete.stack-source/v1` document wrapped in
//! `arete.protocol-stack-source/v1`. It is a port of the TypeScript
//! generator in `@usearete/protocol` (`src/stack/generate.ts`), which
//! documents the two profiles and their known gaps. Both generators run the
//! shared vectors in `test-vectors/protocol-stack/` and must produce the
//! same canonical bytes for the same spec, options and generator identity.
//!
//! **`interim`** reads only the underlying programs: one `ProtocolActivity`
//! row per protocol instruction, a `ProtocolInstance` keyed by the primary
//! bound account and a `ProtocolParticipant` keyed by
//! `<instance account>:<wallet>`.
//!
//! **`full`** decodes the attested Memo v2 through the SPL Memo program and
//! keys rows by the memo's instance and participant ids. The attestation
//! key, the deployment short id and the program ids are inlined as literals.
//! It sets `lookups: strict`, so each `via` join resolves through the index it
//! names only, never through another of the entity's indexes.
//!
//! Every instruction occurrence is counted once, even when several steps
//! list the same instruction: without the memo such steps are read as one
//! group named by their step names joined with `_or_`, and in the full
//! profile a participant's per-step amounts (`<step>_amount`) and an
//! instance's attested per-step amounts (`<step>_protocol_amount`) follow the
//! step the attested memo names (through `last_<step>_signature`).
//!
//! Direct (unattested) activity is matched only on the instance's own
//! accounts, those the create step binds from its ephemeral keys
//! ([`instance_owned_accounts`]); a step whose instance account is anything
//! else (a wallet, a role, a shared account) is counted from attested memos
//! only.
//!
//! Generation is deterministic: the output depends only on the spec and the
//! options.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::model::{ProtocolProgramV1, ProtocolSpecV1, StackActivityV1};
use super::release::{
    GeneratorV1, ProtocolStackSourceV1, StackProfile, PROTOCOL_STACK_SOURCE_SCHEMA_V1,
    STACK_SOURCE_SCHEMA_V1,
};

/// The memo prefix of Memo v2.
pub const MEMO_V2_PREFIX: &str = "arete:p2:";

/// The program alias the full profile gives the SPL Memo program.
pub const MEMO_PROGRAM_ALIAS: &str = "areteMemo";

/// The default IDL file of the SPL Memo program in the full profile.
pub const DEFAULT_MEMO_PROGRAM_IDL: &str = "./idl/spl-memo.idl.json";

const MEMO_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

/// The default generator identity: this crate's name and version.
pub fn default_stack_generator() -> GeneratorV1 {
    GeneratorV1 {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Where a program's IDL comes from in the generated Stack Source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum StackProgramSource {
    /// A public registry slug with an exact (`=1.2.3`) or range (`^1.2`,
    /// `~1.2`) version requirement.
    Registry { registry: String, version: String },
    /// A local IDL file, relative to the Stack Source file.
    Idl { idl: String },
}

impl StackProgramSource {
    /// The default SPL Memo program source of the full profile.
    pub fn default_memo() -> Self {
        Self::Idl {
            idl: DEFAULT_MEMO_PROGRAM_IDL.to_string(),
        }
    }
}

/// Options of [`generate_stack_source`]. Field names match the TypeScript
/// `GenerateStackSourceOptions`, so the shared vectors deserialize directly.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct StackSourceOptions {
    /// `interim` (the default) or `full`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<StackProfile>,
    /// Hosted runtime capability; kept opt-in until runtime deployment.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub operation_boundaries: bool,
    /// Program sources by alias.
    pub programs: BTreeMap<String, StackProgramSource>,
    /// Full profile: the deployment revision's attestation public key (base58).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation: Option<String>,
    /// Full profile: the deployment's short id; memos of other deployments
    /// are ignored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_short_id: Option<String>,
    /// Full profile: the SPL Memo program source. Defaults to
    /// [`StackProgramSource::default_memo`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo_program: Option<StackProgramSource>,
    /// The identity recorded in the output. Defaults to
    /// [`default_stack_generator`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generator: Option<GeneratorV1>,
}

/// A spec or option the generator cannot turn into a stack. The messages
/// match the TypeScript generator's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct StackGenerationError {
    pub message: String,
}

impl StackGenerationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

type GenResult<T> = Result<T, StackGenerationError>;
type Fields = Map<String, Value>;

/// JavaScript's `/([a-z0-9])([A-Z])/g` → `$1-$2`, then `_` → `-`, lowercased.
fn kebab(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 4);
    let mut i = 0;
    while i < chars.len() {
        let current = chars[i];
        if (current.is_ascii_lowercase() || current.is_ascii_digit())
            && chars.get(i + 1).is_some_and(char::is_ascii_uppercase)
        {
            out.push(current);
            out.push('-');
            out.push(chars[i + 1]);
            i += 2;
        } else {
            out.push(current);
            i += 1;
        }
    }
    out.replace('_', "-").to_lowercase()
}

fn pascal(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Stack Source rejects names equal once non-alphanumerics are removed and
/// case folded. `names` is in insertion order, so the first clash reported is
/// the one the TypeScript generator reports.
fn assert_no_collisions<'a>(
    names: impl IntoIterator<Item = &'a String>,
    place: &str,
) -> GenResult<()> {
    let mut seen: BTreeMap<String, &String> = BTreeMap::new();
    for name in names {
        let folded: String = name
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase();
        if let Some(other) = seen.get(&folded) {
            if *other != name {
                return Err(StackGenerationError::new(format!(
                    "{place}: generated names '{other}' and '{name}' collide"
                )));
            }
        }
        seen.insert(folded, name);
    }
    Ok(())
}

struct StepActivity {
    step: String,
    sources: Vec<String>,
    instance_account: String,
    wallet_account: String,
    amount_arg: Option<String>,
}

/// The steps whose instructions overlap, read as one unit. Without the memo
/// an instruction occurrence cannot say which of the steps that list it it
/// belongs to, so every instruction is counted once, under its group.
struct ActivityGroup {
    /// The step name, or the sorted step names joined by `_or_`.
    step: String,
    /// The steps of the group, sorted.
    steps: Vec<StepActivity>,
    /// The instructions of every step, sorted and unique.
    sources: Vec<String>,
    instance_account: String,
    wallet_account: String,
    amount_arg: Option<String>,
}

/// The separator of a merged group's step names.
const GROUP_SEPARATOR: &str = "_or_";

/// Whether two activity rules name the same accounts and amount.
fn same_roles(step: &StepActivity, entry: &StackActivityV1) -> bool {
    step.instance_account == entry.instance_account
        && step.wallet_account == entry.wallet_account
        && step.amount_arg == entry.amount_arg
}

fn group_activity(activity: &[StackActivityV1]) -> GenResult<Vec<StepActivity>> {
    let mut steps: BTreeMap<&str, StepActivity> = BTreeMap::new();
    // The step that first listed each instruction.
    let mut owners: BTreeMap<String, &str> = BTreeMap::new();
    for entry in activity {
        let source = format!("{}.instructions.{}", entry.program, entry.instruction);
        match steps.get_mut(entry.step.as_str()) {
            None => {
                steps.insert(
                    &entry.step,
                    StepActivity {
                        step: entry.step.clone(),
                        sources: vec![source.clone()],
                        instance_account: entry.instance_account.clone(),
                        wallet_account: entry.wallet_account.clone(),
                        amount_arg: entry.amount_arg.clone(),
                    },
                );
            }
            Some(existing) => {
                if !same_roles(existing, entry) {
                    return Err(StackGenerationError::new(format!(
                        "stack.activity for step '{}': every instruction of a step must name the same instanceAccount, walletAccount and amountArg",
                        entry.step
                    )));
                }
                if existing.sources.contains(&source) {
                    return Err(StackGenerationError::new(format!(
                        "stack.activity lists {source} twice for step '{}'",
                        entry.step
                    )));
                }
                existing.sources.push(source.clone());
            }
        }
        // One instruction occurrence has one instance account, wallet and
        // amount, whichever step it is attributed to.
        match owners.get(&source) {
            None => {
                owners.insert(source, &entry.step);
            }
            Some(owner) => {
                let owner_step = &steps[owner];
                if !same_roles(owner_step, entry) {
                    return Err(StackGenerationError::new(format!(
                        "stack.activity: steps '{owner}' and '{}' both list {source}, so they must name the same instanceAccount, walletAccount and amountArg",
                        entry.step
                    )));
                }
            }
        }
    }
    // Identifiers are ASCII, so byte order is the TypeScript UTF-8 order.
    Ok(steps
        .into_values()
        .map(|mut group| {
            group.sources.sort();
            group
        })
        .collect())
}

/// Merge steps that share an instruction into one group (transitively), so
/// each instruction belongs to exactly one group. A step that shares
/// nothing is a group of its own, named by the step.
fn merge_shared_steps(steps: &[StepActivity]) -> GenResult<Vec<ActivityGroup>> {
    let mut groups: Vec<Vec<&StepActivity>> = Vec::new();
    for step in steps {
        let (overlapping, rest): (Vec<_>, Vec<_>) = groups.into_iter().partition(|group| {
            group.iter().any(|other| {
                other
                    .sources
                    .iter()
                    .any(|source| step.sources.contains(source))
            })
        });
        groups = rest;
        let mut merged: Vec<&StepActivity> = overlapping.into_iter().flatten().collect();
        merged.push(step);
        groups.push(merged);
    }
    let mut merged: Vec<ActivityGroup> = groups
        .into_iter()
        .map(|mut members| {
            members.sort_by(|a, b| a.step.cmp(&b.step));
            let first = members[0];
            let sources: BTreeSet<&String> = members
                .iter()
                .flat_map(|member| member.sources.iter())
                .collect();
            ActivityGroup {
                step: members
                    .iter()
                    .map(|member| member.step.as_str())
                    .collect::<Vec<_>>()
                    .join(GROUP_SEPARATOR),
                sources: sources.into_iter().cloned().collect(),
                instance_account: first.instance_account.clone(),
                wallet_account: first.wallet_account.clone(),
                amount_arg: first.amount_arg.clone(),
                steps: members
                    .into_iter()
                    .map(|member| StepActivity {
                        step: member.step.clone(),
                        sources: member.sources.clone(),
                        instance_account: member.instance_account.clone(),
                        wallet_account: member.wallet_account.clone(),
                        amount_arg: member.amount_arg.clone(),
                    })
                    .collect(),
            }
        })
        .collect();
    merged.sort_by(|a, b| a.step.cmp(&b.step));
    if let Some(pair) = merged.windows(2).find(|pair| pair[0].step == pair[1].step) {
        return Err(StackGenerationError::new(format!(
            "stack.activity: the merged step name '{}' is also a step",
            pair[1].step
        )));
    }
    Ok(merged)
}

fn relative(sources: &[String], suffix: &str) -> Vec<String> {
    sources
        .iter()
        .map(|source| format!("{source}.{suffix}"))
        .collect()
}

fn all_sources(groups: &[ActivityGroup]) -> Vec<String> {
    let mut all: Vec<String> = groups
        .iter()
        .flat_map(|group| group.sources.iter().cloned())
        .collect();
    all.sort();
    all
}

/// `<source>.accounts.<role account>` of every group, sorted.
fn account_refs(groups: &[ActivityGroup], role: fn(&ActivityGroup) -> &str) -> Vec<String> {
    let mut refs: Vec<String> = groups
        .iter()
        .flat_map(|group| relative(&group.sources, &format!("accounts.{}", role(group))))
        .collect();
    refs.sort();
    refs
}

fn instance_of(group: &ActivityGroup) -> &str {
    &group.instance_account
}

fn wallet_of(group: &ActivityGroup) -> &str {
    &group.wallet_account
}

/// `<source>.args.<amountArg>` of every group with an amount, sorted.
fn amount_refs(groups: &[ActivityGroup]) -> Vec<String> {
    let mut refs: Vec<String> = groups
        .iter()
        .filter_map(|group| {
            group
                .amount_arg
                .as_ref()
                .map(|arg| relative(&group.sources, &format!("args.{arg}")))
        })
        .flatten()
        .collect();
    refs.sort();
    refs
}

/// A JSON string literal for the expression language.
fn literal(text: &str) -> String {
    Value::String(text.to_string()).to_string()
}

fn object(fields: Fields) -> Value {
    Value::Object(fields)
}

/// Merge `extra`'s fields into an object value.
fn with(mut value: Value, extra: &Value) -> Value {
    if let (Some(target), Some(source)) = (value.as_object_mut(), extra.as_object()) {
        for (key, item) in source {
            target.insert(key.clone(), item.clone());
        }
    }
    value
}

// ---------------------------------------------------------------------------
// Programs
// ---------------------------------------------------------------------------

fn program_id_of(program: &ProtocolProgramV1) -> &str {
    match program {
        ProtocolProgramV1::ProgramSpec { program_id, .. } => program_id,
        ProtocolProgramV1::Builtin { builtin } => builtin.program_id(),
    }
}

fn program_declaration(
    spec: &ProtocolSpecV1,
    alias: &str,
    source: Option<&StackProgramSource>,
) -> Value {
    match source {
        None => json!({ "source": { "registry": kebab(alias) } }),
        Some(StackProgramSource::Idl { idl }) => {
            let mut declaration = json!({ "source": { "idl": idl } });
            if let Some(program) = spec.programs.get(alias) {
                declaration["program_id"] = json!(program_id_of(program));
            }
            declaration
        }
        Some(StackProgramSource::Registry { registry, version }) => {
            json!({ "source": { "registry": registry }, "version": version })
        }
    }
}

fn memo_declaration(source: &StackProgramSource) -> Value {
    match source {
        StackProgramSource::Idl { idl } => {
            json!({ "source": { "idl": idl }, "program_id": MEMO_PROGRAM_ID })
        }
        StackProgramSource::Registry { registry, version } => {
            json!({ "source": { "registry": registry }, "version": version })
        }
    }
}

// ---------------------------------------------------------------------------
// Interim profile
// ---------------------------------------------------------------------------

fn interim_activity_entity(groups: &[ActivityGroup]) -> Value {
    let all = all_sources(groups);
    let mut step = Fields::new();
    for group in groups {
        step.insert(
            group.step.clone(),
            json!({ "type": "bool", "from": group.sources, "expr": "true", "strategy": "set_once" }),
        );
    }
    let mut fields = Fields::new();
    fields.insert(
        "id".into(),
        json!({ "fields": {
            "occurrence": {
                "type": "string",
                "from": all,
                "expr": "concat(__signature, \":\", __ix_path)",
                "strategy": "set_once",
            },
            "signature": { "from": relative(&all, "__signature"), "strategy": "set_once" },
            "ix_path": { "from": relative(&all, "__ix_path"), "strategy": "set_once" },
        }}),
    );
    fields.insert("step".into(), json!({ "fields": step }));
    fields.insert(
        "instance".into(),
        json!({ "from": account_refs(groups, instance_of), "strategy": "set_once" }),
    );
    fields.insert(
        "wallet".into(),
        json!({ "from": account_refs(groups, wallet_of), "strategy": "set_once" }),
    );
    fields.insert(
        "slot".into(),
        json!({ "from": relative(&all, "__slot"), "strategy": "set_once" }),
    );
    fields.insert(
        "timestamp".into(),
        json!({ "from": relative(&all, "__timestamp"), "strategy": "set_once" }),
    );
    let amounts = amount_refs(groups);
    if !amounts.is_empty() {
        fields.insert(
            "amount".into(),
            json!({ "from": amounts, "strategy": "set_once" }),
        );
    }
    json!({
        "primary_key": "id.occurrence",
        "indexes": ["instance", "wallet"],
        "fields": fields,
        "views": {
            "byInstance": {
                "pipeline": [{ "sort": { "by": "slot", "order": "desc" } }],
                "output": "collection",
            },
        },
    })
}

fn account_ref(spec: &ProtocolSpecV1, primary: &str) -> String {
    let account = &spec.stack.instance_accounts[primary];
    format!("{}.accounts.{}", account.program, account.account)
}

fn instance_state(spec: &ProtocolSpecV1, primary: &str) -> Fields {
    let account = account_ref(spec, primary);
    let mut state = Fields::new();
    for (name, field) in &spec.stack.instance_fields {
        if field.account != primary {
            continue;
        }
        state.insert(name.clone(), json!(format!("{account}.{}", field.field)));
    }
    state
}

fn interim_instance_entity(
    spec: &ProtocolSpecV1,
    primary: &str,
    groups: &[ActivityGroup],
) -> GenResult<Value> {
    let account = account_ref(spec, primary);
    let state = instance_state(spec, primary);
    let mut activity = Fields::new();
    let mut keys = Vec::new();
    for group in groups {
        let lookup = format!("accounts.{}", group.instance_account);
        let mut put = |key: String, value: Value| {
            keys.push(key.clone());
            activity.insert(key, value);
        };
        put(
            format!("{}_count", group.step),
            json!({ "aggregate": { "from": group.sources, "op": "count", "lookup_by": lookup } }),
        );
        put(
            format!("{}_last_slot", group.step),
            json!({ "aggregate": {
                "from": group.sources, "op": "max", "field": "__slot", "lookup_by": lookup,
            }}),
        );
        if let Some(arg) = &group.amount_arg {
            put(
                format!("{}_amount", group.step),
                json!({ "aggregate": {
                    "from": group.sources, "op": "sum", "field": format!("args.{arg}"), "lookup_by": lookup,
                }}),
            );
        }
    }
    assert_no_collisions(keys.iter(), "ProtocolInstance.activity")?;
    let mut fields = Fields::new();
    fields.insert(
        "id".into(),
        json!({ "fields": { "address": {
            "from": format!("{account}.__account_address"), "strategy": "set_once",
        }}}),
    );
    if !state.is_empty() {
        fields.insert("state".into(), json!({ "fields": state }));
    }
    if !activity.is_empty() {
        fields.insert("activity".into(), json!({ "fields": activity }));
    }
    Ok(json!({
        "primary_key": "id.address",
        "fields": fields,
        "views": {
            "byId": {
                "pipeline": [{ "sort": { "by": "id.address", "order": "asc" } }],
                "output": "collection",
            },
        },
    }))
}

/// The accounts the create step binds from one of its ephemeral keys, the
/// instance's own accounts: the only accounts the full profile matches direct
/// activity on (see the TypeScript generator's "Direct activity"). A binding
/// qualifies when its expression references `ephemeral.<name>` for a key the
/// create step declares, outside a string literal.
pub fn instance_owned_accounts(spec: &ProtocolSpecV1) -> BTreeSet<String> {
    let Some((bind, ephemeral)) = spec
        .create_step()
        .and_then(|(_, step)| step.bind.as_ref().map(|bind| (bind, &step.ephemeral)))
    else {
        return BTreeSet::new();
    };
    bind.accounts
        .iter()
        .filter(|(_, expr)| {
            ephemeral_references(expr)
                .iter()
                .any(|name| ephemeral.iter().any(|key| key == name))
        })
        .map(|(name, _)| name.clone())
        .collect()
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

const EPHEMERAL_PREFIX: &str = "ephemeral.";

/// The `<name>` of every `ephemeral.<name>` in an expression, skipping
/// string literals. Byte-wise; only ASCII bytes are significant, so it reads
/// the same text as the TypeScript scanner's UTF-16 walk.
fn ephemeral_references(expr: &str) -> Vec<&str> {
    let bytes = expr.as_bytes();
    let mut names = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'"' || byte == b'\'' {
            // A string literal: skip to its closing quote, honouring escapes.
            i += 1;
            while i < bytes.len() && bytes[i] != byte {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
            continue;
        }
        let before = if i == 0 { None } else { Some(bytes[i - 1]) };
        let boundary = before.is_none_or(|b| !is_ident_byte(b) && b != b'.');
        if boundary && bytes[i..].starts_with(EPHEMERAL_PREFIX.as_bytes()) {
            let start = i + EPHEMERAL_PREFIX.len();
            let mut end = start;
            while end < bytes.len() && is_ident_byte(bytes[end]) {
                end += 1;
            }
            names.push(&expr[start..end]);
            i = end;
            continue;
        }
        i += 1;
    }
    names
}

/// The one instruction account each role is read from, across every
/// activity group.
fn shared_account(
    groups: &[ActivityGroup],
    role: fn(&ActivityGroup) -> &str,
    role_name: &str,
    place: &str,
) -> GenResult<String> {
    let names: BTreeSet<&str> = groups.iter().map(role).collect();
    if names.len() != 1 {
        let found: Vec<&str> = names.into_iter().collect();
        return Err(StackGenerationError::new(format!(
            "{place} needs every stack.activity entry to name the same {role_name}; found {}",
            found.join(", ")
        )));
    }
    Ok(names.into_iter().next().unwrap_or_default().to_string())
}

fn interim_participant_entity(groups: &[ActivityGroup]) -> GenResult<Value> {
    let all = all_sources(groups);
    // A row key is one field (generated SDKs support no composite keys), and
    // its expression reads the same account names from every instruction.
    let instance_account = shared_account(
        groups,
        instance_of,
        "instanceAccount",
        "ProtocolParticipant",
    )?;
    let wallet_account = shared_account(groups, wallet_of, "walletAccount", "ProtocolParticipant")?;
    let mut activity = Fields::new();
    let mut keys = Vec::new();
    for group in groups {
        let count = format!("{}_count", group.step);
        activity.insert(
            count.clone(),
            json!({ "type": "u64", "from": group.sources, "expr": "1", "strategy": "sum" }),
        );
        keys.push(count);
        if let Some(arg) = &group.amount_arg {
            let amount = format!("{}_amount", group.step);
            activity.insert(
                amount.clone(),
                json!({ "from": relative(&group.sources, &format!("args.{arg}")), "strategy": "sum" }),
            );
            keys.push(amount);
        }
    }
    activity.insert(
        "last_slot".into(),
        json!({ "from": relative(&all, "__slot"), "strategy": "max" }),
    );
    keys.push("last_slot".into());
    assert_no_collisions(keys.iter(), "ProtocolParticipant.activity")?;
    Ok(json!({
        "primary_key": "id.key",
        "indexes": ["id.instance"],
        "fields": {
            "id": { "fields": {
                "key": {
                    "type": "string",
                    "from": all,
                    "expr": format!("concat(accounts.{instance_account}, \":\", accounts.{wallet_account})"),
                    "strategy": "set_once",
                },
                "instance": { "from": account_refs(groups, instance_of), "strategy": "set_once" },
                "wallet": { "from": account_refs(groups, wallet_of), "strategy": "set_once" },
            }},
            "activity": { "fields": activity },
        },
        "views": {
            "byInstance": {
                "pipeline": [{ "sort": { "by": "id.instance", "order": "asc" } }],
                "output": "collection",
            },
        },
    }))
}

// ---------------------------------------------------------------------------
// Full profile
// ---------------------------------------------------------------------------

/// Memo v2 field positions, counting from 1 as `split_part` does.
#[derive(Clone, Copy)]
enum MemoField {
    Scope = 4,
    Instance = 5,
    Participant = 6,
    Step = 7,
}

fn memo_part(field: MemoField) -> String {
    format!("split_part(args.memo, \":\", {})", field as u8)
}

struct MemoContext {
    /// `<alias>.instructions.memo`.
    memo: String,
    /// Holds for an attested Memo v2 of this deployment.
    attested: String,
    scoped: bool,
}

impl MemoContext {
    /// A string field written once from an attested memo.
    fn string(&self, expr: impl Into<String>) -> Value {
        self.string_guarded(expr, &self.attested)
    }

    fn string_guarded(&self, expr: impl Into<String>, guard: &str) -> Value {
        json!({ "type": "string", "from": self.memo, "expr": expr.into(), "strategy": "set_once", "guard": guard })
    }
}

/// A step's attested amount on the instance row: the `last_<step>_signature`
/// field (hidden) and the `<step>_protocol_amount` field joined through it. An
/// instruction adds to a step's amount only in a transaction whose attested
/// memo names that step, so it counts once, under the step the memo names.
fn protocol_amount(
    step: &StepActivity,
    ctx: &MemoContext,
) -> Option<(String, Value, String, Value)> {
    let arg = step.amount_arg.as_ref()?;
    let signature = json!({
        "type": "string",
        "from": ctx.memo,
        "expr": "__signature",
        "strategy": "last_write",
        "guard": format!("{} && {} == {}", ctx.attested, memo_part(MemoField::Step), literal(&step.step)),
        "emit": false,
    });
    let amount = json!({
        "from": relative(&step.sources, &format!("args.{arg}")),
        "strategy": "sum",
        "lookup_by": "__signature",
        "via": last_step_signature(&step.step),
    });
    Some((
        last_step_signature(&step.step),
        signature,
        format!("{}_protocol_amount", step.step),
        amount,
    ))
}

fn full_instance_entity(
    spec: &ProtocolSpecV1,
    primary: Option<&str>,
    groups: &[ActivityGroup],
    ctx: &MemoContext,
) -> GenResult<Value> {
    // First-appearance order over the step-sorted groups, as the TypeScript
    // `Set` keeps it.
    let mut instance_accounts: Vec<&str> = Vec::new();
    for group in groups {
        if !instance_accounts.contains(&group.instance_account.as_str()) {
            instance_accounts.push(&group.instance_account);
        }
    }
    if instance_accounts.len() > 1 {
        return Err(StackGenerationError::new(format!(
            "the full profile routes all activity through one instance account; stack.activity names {}",
            instance_accounts.join(", ")
        )));
    }
    // Direct activity is matched only on the instance's own accounts.
    let direct = instance_accounts.len() == 1
        && instance_owned_accounts(spec).contains(instance_accounts[0]);
    let lookup = instance_accounts
        .first()
        .filter(|_| direct)
        .map(|account| format!("accounts.{account}"));
    let all = all_sources(groups);

    let registrations: Vec<Value> = groups
        .iter()
        .filter(|_| direct)
        .flat_map(|group| {
            group.sources.iter().map(|source| {
                json!({
                    "instruction": ctx.memo,
                    "sibling": source,
                    "pda_field": format!("sibling.accounts.{}", group.instance_account),
                    "seed_expr": memo_part(MemoField::Instance),
                    "guard": ctx.attested,
                    "lookup_name": "instance_accounts",
                })
            })
        })
        .collect();

    let mut id = Fields::new();
    id.insert(
        "instance".into(),
        ctx.string(memo_part(MemoField::Instance)),
    );
    if ctx.scoped {
        id.insert("scope".into(), ctx.string(memo_part(MemoField::Scope)));
    }

    let mut activity = Fields::new();
    let mut step_signatures = Fields::new();
    let mut keys = Vec::new();
    if let Some(lookup) = &lookup {
        let via = json!({ "lookup_by": lookup, "via": "instance_accounts" });
        for group in groups {
            // The instructions count once, under the group; the memo names
            // the step, so the attested part is counted per step.
            let step = &group.step;
            let total = format!("(activity.{step}_count ?? 0)");
            let parts: Vec<String> = group
                .steps
                .iter()
                .map(|member| format!("(activity.{}_protocol_count ?? 0)", member.step))
                .collect();
            let attested = if parts.len() == 1 {
                parts[0].clone()
            } else {
                format!("({})", parts.join(" + "))
            };
            let mut put = |key: String, value: Value| {
                keys.push(key.clone());
                activity.insert(key, value);
            };
            put(
                format!("{step}_count"),
                json!({ "aggregate": with(json!({ "from": group.sources, "op": "count" }), &via) }),
            );
            for member in &group.steps {
                put(
                    format!("{}_protocol_count", member.step),
                    json!({
                        "type": "u64",
                        "from": ctx.memo,
                        "expr": "1",
                        "strategy": "sum",
                        "guard": format!("{} && {} == {}", ctx.attested, memo_part(MemoField::Step), literal(&member.step)),
                    }),
                );
                if let Some((name, signature, key, amount)) = protocol_amount(member, ctx) {
                    step_signatures.insert(name, signature);
                    put(key, amount);
                }
            }
            put(
                format!("{step}_direct_count"),
                json!({
                    "type": "u64",
                    "compute": format!(
                        "if({total} >= {attested}, {total} - {attested}, 0)"
                    ),
                }),
            );
            if let Some(arg) = &group.amount_arg {
                put(
                    format!("{step}_amount"),
                    json!({ "aggregate": with(
                        json!({ "from": group.sources, "op": "sum", "field": format!("args.{arg}") }),
                        &via,
                    )}),
                );
            }
        }
        let mut put = |key: &str, value: Value| {
            keys.push(key.to_string());
            activity.insert(key.to_string(), value);
        };
        put(
            "last_signature",
            with(json!({ "from": relative(&all, "__signature") }), &via),
        );
        put(
            "last_ix_path",
            with(json!({ "from": relative(&all, "__ix_path") }), &via),
        );
        put(
            "last_slot",
            with(
                json!({ "from": relative(&all, "__slot"), "strategy": "max" }),
                &via,
            ),
        );
        put(
            "last_timestamp",
            with(
                json!({ "from": relative(&all, "__timestamp"), "strategy": "max" }),
                &via,
            ),
        );
    } else if !groups.is_empty() {
        // No account belongs to the instance alone, so only attested memos
        // reach the row: per-step protocol counts, and the last protocol
        // transaction.
        let mut put = |key: String, value: Value| {
            keys.push(key.clone());
            activity.insert(key, value);
        };
        for member in groups.iter().flat_map(|group| &group.steps) {
            put(
                format!("{}_protocol_count", member.step),
                json!({
                    "type": "u64",
                    "from": ctx.memo,
                    "expr": "1",
                    "strategy": "sum",
                    "guard": format!("{} && {} == {}", ctx.attested, memo_part(MemoField::Step), literal(&member.step)),
                }),
            );
            if let Some((name, signature, key, amount)) = protocol_amount(member, ctx) {
                step_signatures.insert(name, signature);
                put(key, amount);
            }
        }
        let memo_field = |field: &str| format!("{}.{field}", ctx.memo);
        put(
            "last_signature".into(),
            json!({ "from": memo_field("__signature"), "guard": ctx.attested }),
        );
        put(
            "last_ix_path".into(),
            json!({ "from": memo_field("__ix_path"), "guard": ctx.attested }),
        );
        put(
            "last_slot".into(),
            json!({ "from": memo_field("__slot"), "strategy": "max", "guard": ctx.attested }),
        );
        put(
            "last_timestamp".into(),
            json!({ "from": memo_field("__timestamp"), "strategy": "max", "guard": ctx.attested }),
        );
    }
    assert_no_collisions(keys.iter(), "ProtocolInstance.activity")?;

    let mut fields = Fields::new();
    fields.insert("id".into(), json!({ "fields": id }));
    let mut indexes = vec!["id.instance".to_string()];
    let mut signature_names: Vec<String> = step_signatures.keys().cloned().collect();
    signature_names.sort();
    indexes.extend(signature_names);
    fields.extend(step_signatures);
    let state = match (primary, &lookup) {
        (Some(primary), Some(_)) => instance_state(spec, primary),
        _ => Fields::new(),
    };
    if !state.is_empty() {
        fields.insert("state".into(), json!({ "fields": state }));
    }
    if !activity.is_empty() {
        fields.insert("activity".into(), json!({ "fields": activity }));
    }
    let mut entity = Fields::new();
    entity.insert("primary_key".into(), json!("id.instance"));
    if ctx.scoped {
        entity.insert("scope".into(), json!("id.scope"));
    }
    entity.insert("indexes".into(), json!(indexes));
    if !registrations.is_empty() {
        entity.insert("register_from".into(), Value::Array(registrations));
    }
    entity.insert("fields".into(), object(fields));
    entity.insert(
        "views".into(),
        json!({
            "byId": {
                "pipeline": [{ "sort": { "by": "id.instance", "order": "asc" } }],
                "output": "collection",
            },
        }),
    );
    Ok(object(entity))
}

fn full_activity_entity(groups: &[ActivityGroup], ctx: &MemoContext) -> Value {
    let mut fields = Fields::new();
    fields.insert(
        "id".into(),
        json!({ "fields": { "signature": ctx.string("__signature") } }),
    );
    fields.insert(
        "instance".into(),
        ctx.string(memo_part(MemoField::Instance)),
    );
    fields.insert(
        "participant".into(),
        ctx.string(memo_part(MemoField::Participant)),
    );
    fields.insert("step".into(), ctx.string(memo_part(MemoField::Step)));
    fields.insert(
        "attested".into(),
        json!({ "type": "bool", "from": ctx.memo, "expr": "true", "strategy": "set_once", "guard": ctx.attested }),
    );
    fields.insert("via".into(), ctx.string(literal("protocol")));
    fields.insert(
        "slot".into(),
        json!({ "from": format!("{}.__slot", ctx.memo), "strategy": "set_once", "guard": ctx.attested }),
    );
    fields.insert(
        "timestamp".into(),
        json!({ "from": format!("{}.__timestamp", ctx.memo), "strategy": "set_once", "guard": ctx.attested }),
    );
    if ctx.scoped {
        fields.insert("scope".into(), ctx.string(memo_part(MemoField::Scope)));
    }
    if !groups.is_empty() {
        let join = json!({ "lookup_by": "__signature", "via": "id.signature" });
        fields.insert(
            "wallet".into(),
            with(
                json!({ "from": account_refs(groups, wallet_of), "strategy": "set_once" }),
                &join,
            ),
        );
        let amounts = amount_refs(groups);
        if !amounts.is_empty() {
            fields.insert(
                "amount".into(),
                with(json!({ "from": amounts, "strategy": "sum" }), &join),
            );
        }
        // The account the step's activity entry names (`instanceAccount`).
        fields.insert(
            "activity_account".into(),
            with(
                json!({ "from": account_refs(groups, instance_of), "strategy": "set_once" }),
                &join,
            ),
        );
    }
    let mut entity = Fields::new();
    entity.insert("primary_key".into(), json!("id.signature"));
    if ctx.scoped {
        entity.insert("scope".into(), json!("scope"));
    }
    entity.insert("indexes".into(), json!(["id.signature", "instance"]));
    entity.insert("fields".into(), object(fields));
    entity.insert(
        "views".into(),
        json!({
            "byInstance": {
                "pipeline": [{ "sort": { "by": "slot", "order": "desc" } }],
                "output": "collection",
            },
        }),
    );
    object(entity)
}

/// The instance or participant field holding the signature of the last
/// attested memo naming `step`.
fn last_step_signature(step: &str) -> String {
    format!("last_{step}_signature")
}

fn full_participant_entity(
    steps: &[StepActivity],
    groups: &[ActivityGroup],
    ctx: &MemoContext,
) -> GenResult<Value> {
    let with_participant = format!(
        "{} && {} != \"-\"",
        ctx.attested,
        memo_part(MemoField::Participant)
    );
    let mut activity = Fields::new();
    let mut step_signatures = Fields::new();
    let mut keys = Vec::new();
    for step in steps {
        let named = format!(
            "{with_participant} && {} == {}",
            memo_part(MemoField::Step),
            literal(&step.step)
        );
        let count = format!("{}_count", step.step);
        activity.insert(
            count.clone(),
            json!({ "type": "u64", "from": ctx.memo, "expr": "1", "strategy": "sum", "guard": named }),
        );
        keys.push(count);
        if let Some(arg) = &step.amount_arg {
            // An instruction adds to a step's amount only in a transaction
            // whose attested memo names that step, so an instruction several
            // steps list counts once, under the step the memo names.
            step_signatures.insert(
                last_step_signature(&step.step),
                json!({
                    "type": "string",
                    "from": ctx.memo,
                    "expr": "__signature",
                    "strategy": "last_write",
                    "guard": named,
                    "emit": false,
                }),
            );
            let amount = format!("{}_amount", step.step);
            activity.insert(
                amount.clone(),
                json!({
                    "from": relative(&step.sources, &format!("args.{arg}")),
                    "strategy": "sum",
                    "lookup_by": "__signature",
                    "via": last_step_signature(&step.step),
                }),
            );
            keys.push(amount);
        }
    }
    activity.insert(
        "last_slot".into(),
        json!({ "from": format!("{}.__slot", ctx.memo), "strategy": "max", "guard": with_participant }),
    );
    keys.push("last_slot".into());
    assert_no_collisions(keys.iter(), "ProtocolParticipant.activity")?;
    let mut fields = Fields::new();
    fields.insert(
        "id".into(),
        json!({ "fields": { "participant": ctx.string_guarded(memo_part(MemoField::Participant), &with_participant) } }),
    );
    fields.insert(
        "instance".into(),
        ctx.string_guarded(memo_part(MemoField::Instance), &with_participant),
    );
    fields.insert(
        "last_signature".into(),
        json!({
            "type": "string",
            "from": ctx.memo,
            "expr": "__signature",
            "strategy": "last_write",
            "guard": with_participant,
            "emit": false,
        }),
    );
    let mut indexes = vec![
        "id.participant".to_string(),
        "instance".to_string(),
        "last_signature".to_string(),
    ];
    let mut signature_names: Vec<String> = step_signatures.keys().cloned().collect();
    signature_names.sort();
    indexes.extend(signature_names);
    fields.extend(step_signatures);
    fields.insert("activity".into(), json!({ "fields": activity }));
    if ctx.scoped {
        fields.insert(
            "scope".into(),
            ctx.string_guarded(memo_part(MemoField::Scope), &with_participant),
        );
    }
    if !groups.is_empty() {
        fields.insert(
            "wallet".into(),
            json!({
                "from": account_refs(groups, wallet_of),
                "strategy": "set_once",
                "lookup_by": "__signature",
                "via": "last_signature",
            }),
        );
    }
    let mut entity = Fields::new();
    entity.insert("primary_key".into(), json!("id.participant"));
    if ctx.scoped {
        entity.insert("scope".into(), json!("scope"));
    }
    entity.insert("indexes".into(), json!(indexes));
    entity.insert("fields".into(), object(fields));
    entity.insert(
        "views".into(),
        json!({
            "byInstance": {
                "pipeline": [{ "sort": { "by": "instance", "order": "asc" } }],
                "output": "collection",
            },
        }),
    );
    Ok(object(entity))
}

/// `^[A-Za-z0-9_-]{1,64}$`.
fn is_short_id(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// A base58 text of a 32-byte key, at most 44 bytes long.
fn is_pubkey(text: &str) -> bool {
    text.len() <= 44
        && bs58::decode(text)
            .into_vec()
            .is_ok_and(|bytes| bytes.len() == 32)
}

fn memo_context(spec: &ProtocolSpecV1, options: &StackSourceOptions) -> GenResult<MemoContext> {
    let attestation = match options.attestation.as_deref() {
        Some(key) if is_pubkey(key) => key,
        _ => {
            return Err(StackGenerationError::new(
                "the full profile needs options.attestation: the deployment revision's attestation public key (base58)",
            ))
        }
    };
    let prefix = match options.deployment_short_id.as_deref() {
        None => MEMO_V2_PREFIX.to_string(),
        Some(short_id) if is_short_id(short_id) => format!("{MEMO_V2_PREFIX}{short_id}:"),
        Some(_) => {
            return Err(StackGenerationError::new(
                "options.deploymentShortId must be 1-64 characters of [A-Za-z0-9_-]",
            ))
        }
    };
    Ok(MemoContext {
        memo: format!("{MEMO_PROGRAM_ALIAS}.instructions.memo"),
        attested: format!(
            "starts_with(args.memo, {}) && accounts.signer == {} && accounts.signer.is_signer",
            literal(&prefix),
            literal(attestation)
        ),
        scoped: spec.instance_visibility == super::model::InstanceVisibility::Account,
    })
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn guard_operation_activity(value: &mut Value, sources: &[String], attestation: &str) {
    match value {
        Value::Array(values) => {
            for value in values {
                guard_operation_activity(value, sources, attestation);
            }
        }
        Value::Object(object) => {
            if object.get("sibling").is_some_and(Value::is_string) {
                object.insert(
                    "sibling_where".into(),
                    json!({"__protocol_activity_attestation":attestation}),
                );
            }
            let references = match object.get("from") {
                Some(Value::String(from)) => vec![from.as_str()],
                Some(Value::Array(from)) => from.iter().filter_map(Value::as_str).collect(),
                _ => vec![],
            };
            if references.iter().any(|reference| {
                sources.iter().any(|source| {
                    *reference == source || reference.starts_with(&format!("{source}."))
                })
            }) {
                let member = format!(
                    "__protocol_activity_attestation == {}",
                    literal(attestation)
                );
                let guard =
                    if object.get("via").and_then(Value::as_str) == Some("instance_accounts") {
                        "__protocol_attestation == \"\" || __protocol_activity_attestation != \"\""
                            .to_string()
                    } else {
                        member
                    };
                let guard = match object.get("guard").and_then(Value::as_str) {
                    Some(existing) => format!("({existing}) && ({guard})"),
                    None => guard,
                };
                object.insert("guard".into(), Value::String(guard));
            }
            for value in object.values_mut() {
                guard_operation_activity(value, sources, attestation);
            }
        }
        _ => {}
    }
}

/// Generate the protocol stack source of a validated spec.
///
/// The output depends only on `spec` and `options`. With the TypeScript
/// generator's identity in `options.generator`, its canonical JSON is
/// byte-identical to the TypeScript output.
pub fn generate_stack_source(
    spec: &ProtocolSpecV1,
    options: &StackSourceOptions,
) -> Result<ProtocolStackSourceV1, StackGenerationError> {
    let profile = options.profile.unwrap_or(StackProfile::Interim);
    let primary = spec
        .stack
        .instance_accounts
        .keys()
        .next()
        .map(String::as_str);
    let steps = group_activity(&spec.stack.activity)?;
    let groups = merge_shared_steps(&steps)?;
    if primary.is_none() && groups.is_empty() {
        return Err(StackGenerationError::new(
            "the protocol stack needs stack.instanceAccounts or stack.activity; declare what the stack should track",
        ));
    }

    let mut aliases: BTreeSet<&str> = spec
        .stack
        .activity
        .iter()
        .map(|entry| entry.program.as_str())
        .collect();
    if let Some(primary) = primary {
        aliases.insert(&spec.stack.instance_accounts[primary].program);
    }
    let mut programs = Fields::new();
    for alias in &aliases {
        programs.insert(
            alias.to_string(),
            program_declaration(spec, alias, options.programs.get(*alias)),
        );
    }

    let mut entities = Fields::new();
    match profile {
        StackProfile::Interim => {
            if !groups.is_empty() {
                entities.insert("ProtocolActivity".into(), interim_activity_entity(&groups));
                entities.insert(
                    "ProtocolParticipant".into(),
                    interim_participant_entity(&groups)?,
                );
            }
            if let Some(primary) = primary {
                entities.insert(
                    "ProtocolInstance".into(),
                    interim_instance_entity(spec, primary, &groups)?,
                );
            }
        }
        StackProfile::Full => {
            if aliases.contains(MEMO_PROGRAM_ALIAS) {
                return Err(StackGenerationError::new(format!(
                    "the program alias '{MEMO_PROGRAM_ALIAS}' is reserved for the memo program"
                )));
            }
            let ctx = memo_context(spec, options)?;
            let memo = options
                .memo_program
                .clone()
                .unwrap_or_else(StackProgramSource::default_memo);
            programs.insert(MEMO_PROGRAM_ALIAS.into(), memo_declaration(&memo));
            entities.insert(
                "ProtocolActivity".into(),
                full_activity_entity(&groups, &ctx),
            );
            entities.insert(
                "ProtocolInstance".into(),
                full_instance_entity(spec, primary, &groups, &ctx)?,
            );
            entities.insert(
                "ProtocolParticipant".into(),
                full_participant_entity(&steps, &groups, &ctx)?,
            );
            if options.operation_boundaries {
                let attestation = options
                    .attestation
                    .as_deref()
                    .expect("full context validated");
                let sources = all_sources(&groups);
                for entity in entities.values_mut() {
                    guard_operation_activity(entity, &sources, attestation);
                }
            }
        }
    }

    let base = format!("{}Protocol", pascal(&spec.name));
    let name = if base.len() <= 64 {
        base
    } else {
        pascal(&spec.name)
    };
    let mut stack_source = json!({
        "schema": STACK_SOURCE_SCHEMA_V1,
        "name": name,
        "programs": programs,
        "entities": entities,
    });
    // The full profile joins by signature through per-step indexes; strict
    // lookups resolve each join through the index it names only.
    if profile == StackProfile::Full {
        stack_source["lookups"] = json!("strict");
    }
    Ok(ProtocolStackSourceV1 {
        schema: PROTOCOL_STACK_SOURCE_SCHEMA_V1.to_string(),
        generator: options
            .generator
            .clone()
            .unwrap_or_else(default_stack_generator),
        profile,
        stack_source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{parse_contract_json, validate_protocol_spec_value};

    const ATTESTATION: &str = "Attest1111111111111111111111111111111111111";

    fn token_raise() -> ProtocolSpecV1 {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../test-vectors/protocol/token-raise.protocol-spec.json");
        let value = parse_contract_json(&std::fs::read(path).unwrap()).unwrap();
        validate_protocol_spec_value(&value).unwrap()
    }

    fn full() -> StackSourceOptions {
        StackSourceOptions {
            profile: Some(StackProfile::Full),
            attestation: Some(ATTESTATION.into()),
            deployment_short_id: Some("raise1".into()),
            ..StackSourceOptions::default()
        }
    }

    fn activity(step: &str, instruction: &str) -> StackActivityV1 {
        StackActivityV1 {
            step: step.into(),
            program: "meteoraPresale".into(),
            instruction: instruction.into(),
            instance_account: "presale".into(),
            wallet_account: "owner".into(),
            amount_arg: None,
        }
    }

    fn message(spec: &ProtocolSpecV1, options: &StackSourceOptions) -> String {
        generate_stack_source(spec, options).unwrap_err().message
    }

    #[test]
    fn kebab_matches_the_javascript_regex() {
        assert_eq!(kebab("meteoraPresale"), "meteora-presale");
        assert_eq!(kebab("token_v2Vault9XProgram"), "token-v2-vault9-xprogram");
        assert_eq!(kebab("aBC"), "a-bc");
        assert_eq!(kebab("a1B2C"), "a1-b2-c");
        assert_eq!(kebab("ABC"), "abc");
    }

    #[test]
    fn rejects_colliding_generated_names() {
        let mut spec = token_raise();
        spec.stack.activity = vec![activity("a_b", "first"), activity("aB", "second")];
        assert_eq!(
            message(&spec, &StackSourceOptions::default()),
            "ProtocolParticipant.activity: generated names 'aB_count' and 'a_b_count' collide"
        );
        assert_eq!(
            message(&spec, &full()),
            "ProtocolInstance.activity: generated names 'aB_count' and 'a_b_count' collide"
        );
    }

    #[test]
    fn only_the_full_profile_declares_strict_lookups() {
        let spec = token_raise();
        let full = generate_stack_source(&spec, &full()).unwrap().stack_source;
        assert_eq!(full["lookups"], "strict");
        let interim = generate_stack_source(&spec, &StackSourceOptions::default())
            .unwrap()
            .stack_source;
        assert!(interim.get("lookups").is_none());
    }

    #[test]
    fn counts_an_instruction_several_steps_list_once() {
        let mut spec = token_raise();
        let mut first = activity("contribute", "deposit");
        first.amount_arg = Some("max_amount".into());
        let mut second = activity("refund", "deposit");
        second.amount_arg = Some("max_amount".into());
        spec.stack.activity = vec![first, second];
        let full = generate_stack_source(&spec, &full()).unwrap().stack_source;
        let entities = &full["entities"];
        assert_eq!(
            entities["ProtocolActivity"]["fields"]["amount"]["from"],
            json!(["meteoraPresale.instructions.deposit.args.max_amount"])
        );
        let participant = &entities["ProtocolParticipant"]["fields"];
        assert_eq!(
            participant["activity"]["fields"]["refund_amount"]["via"],
            "last_refund_signature"
        );
        assert!(entities["ProtocolInstance"]["fields"]["activity"]["fields"]
            ["contribute_or_refund_direct_count"]
            .is_object());
        let interim = generate_stack_source(&spec, &StackSourceOptions::default())
            .unwrap()
            .stack_source;
        assert!(
            interim["entities"]["ProtocolInstance"]["fields"]["activity"]["fields"]
                ["contribute_or_refund_amount"]
                .is_object()
        );
    }

    #[test]
    fn instance_owned_accounts_are_bound_from_ephemeral_keys() {
        let mut spec = token_raise();
        let owned: Vec<String> = instance_owned_accounts(&spec).into_iter().collect();
        assert_eq!(owned, ["mint", "presale"]);
        let bind = spec.steps.get_mut("create").unwrap().bind.as_mut().unwrap();
        bind.accounts = [
            ("creator", "wallet"),
            ("recipient", "args.recipient"),
            (
                "config",
                r#"pda("meteoraPresale", "config", params.quoteMint)"#,
            ),
            ("quoted", r#"pda("meteoraPresale", "x", "ephemeral.mint")"#),
            ("undeclared", "ephemeral.other"),
            ("nested", "instance.ephemeral.mint"),
            ("fresh", "ephemeral.mint"),
            (
                "derived",
                r#"pda("meteoraPresale", "presale", ephemeral.presaleBase)"#,
            ),
        ]
        .into_iter()
        .map(|(name, expr)| (name.to_string(), expr.to_string()))
        .collect();
        let owned: Vec<String> = instance_owned_accounts(&spec).into_iter().collect();
        assert_eq!(owned, ["derived", "fresh"]);
    }

    #[test]
    fn matches_no_direct_activity_on_a_wallet() {
        // A tip jar: each step is a bare transfer whose instance account is
        // the creator's wallet.
        let mut spec = token_raise();
        let bind = spec.steps.get_mut("create").unwrap().bind.as_mut().unwrap();
        bind.accounts.insert("creator".into(), "wallet".into());
        let mut open = activity("contribute", "deposit");
        open.instance_account = "creator".into();
        open.amount_arg = Some("max_amount".into());
        let mut close = activity("refund", "deposit");
        close.instance_account = "creator".into();
        close.amount_arg = Some("max_amount".into());
        spec.stack.activity = vec![open, close];
        let full = generate_stack_source(&spec, &full()).unwrap().stack_source;
        let instance = &full["entities"]["ProtocolInstance"];
        assert!(instance.get("register_from").is_none());
        let fields = instance["fields"]["activity"]["fields"]
            .as_object()
            .unwrap();
        let names: Vec<&str> = fields.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            [
                "contribute_protocol_amount",
                "contribute_protocol_count",
                "last_ix_path",
                "last_signature",
                "last_slot",
                "last_timestamp",
                "refund_protocol_amount",
                "refund_protocol_count",
            ]
        );
        assert_eq!(
            fields["last_signature"]["from"],
            "areteMemo.instructions.memo.__signature"
        );
        // Each transfer's amount counts once, under the step its attested
        // memo names, joined by signature rather than through the wallet.
        assert_eq!(
            fields["refund_protocol_amount"],
            json!({
                "from": ["meteoraPresale.instructions.deposit.args.max_amount"],
                "strategy": "sum",
                "lookup_by": "__signature",
                "via": "last_refund_signature",
            })
        );
        assert_eq!(instance["fields"]["last_refund_signature"]["emit"], false);
        assert_eq!(
            instance["indexes"],
            json!([
                "id.instance",
                "last_contribute_signature",
                "last_refund_signature"
            ])
        );
    }

    #[test]
    fn rejects_shared_instructions_that_disagree_or_merged_names_that_clash() {
        let mut spec = token_raise();
        let mut first = activity("contribute", "deposit");
        first.amount_arg = Some("max_amount".into());
        spec.stack.activity = vec![first, activity("refund", "deposit")];
        assert_eq!(
            message(&spec, &StackSourceOptions::default()),
            "stack.activity: steps 'contribute' and 'refund' both list meteoraPresale.instructions.deposit, so they must name the same instanceAccount, walletAccount and amountArg"
        );
        spec.stack.activity = vec![
            activity("a", "deposit"),
            activity("b", "deposit"),
            activity("a_or_b", "claim"),
        ];
        assert_eq!(
            message(&spec, &StackSourceOptions::default()),
            "stack.activity: the merged step name 'a_or_b' is also a step"
        );
    }

    #[test]
    fn checks_the_deployment_short_id_bounds() {
        let spec = token_raise();
        let with_id = |id: String| StackSourceOptions {
            deployment_short_id: Some(id),
            ..full()
        };
        assert!(generate_stack_source(&spec, &with_id("a".repeat(64))).is_ok());
        assert!(generate_stack_source(&spec, &with_id("A-z_09".into())).is_ok());
        for bad in [
            "a".repeat(65),
            String::new(),
            "raise\n".into(),
            "räise".into(),
        ] {
            assert_eq!(
                message(&spec, &with_id(bad)),
                "options.deploymentShortId must be 1-64 characters of [A-Za-z0-9_-]"
            );
        }
    }

    #[test]
    fn checks_the_attestation_key() {
        let spec = token_raise();
        let expected = "the full profile needs options.attestation: the deployment revision's attestation public key (base58)";
        for key in [None, Some("not-a-key"), Some("0OIl"), Some("")] {
            let options = StackSourceOptions {
                attestation: key.map(str::to_string),
                ..full()
            };
            assert_eq!(message(&spec, &options), expected);
        }
    }

    #[test]
    fn gives_each_attestation_key_and_deployment_its_own_stack() {
        let spec = token_raise();
        let variants = [
            full(),
            StackSourceOptions {
                attestation: Some("SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf".into()),
                ..full()
            },
            StackSourceOptions {
                deployment_short_id: Some("raise2".into()),
                ..full()
            },
            StackSourceOptions {
                deployment_short_id: None,
                ..full()
            },
        ];
        let outputs: BTreeSet<Vec<u8>> = variants
            .iter()
            .map(|options| {
                crate::canonicalize_jcs(&generate_stack_source(&spec, options).unwrap()).unwrap()
            })
            .collect();
        assert_eq!(outputs.len(), 4);
    }

    #[test]
    fn is_deterministic_and_keys_entities_by_one_field() {
        let spec = token_raise();
        for options in [StackSourceOptions::default(), full()] {
            let first = generate_stack_source(&spec, &options).unwrap();
            assert_eq!(
                first,
                generate_stack_source(&spec.clone(), &options).unwrap()
            );
            for entity in first.stack_source["entities"].as_object().unwrap().values() {
                assert!(entity["primary_key"].is_string());
            }
        }
    }

    #[test]
    fn options_parse_like_the_typescript_options() {
        let options: StackSourceOptions = serde_json::from_value(json!({
            "profile": "full",
            "programs": {
                "a": { "registry": "a", "version": "=1.0.0" },
                "b": { "idl": "./idl/b.json" },
            },
            "attestation": ATTESTATION,
            "deploymentShortId": "x",
            "memoProgram": { "registry": "spl-memo", "version": "^3" },
        }))
        .unwrap();
        assert_eq!(options.profile, Some(StackProfile::Full));
        assert_eq!(
            options.programs["b"],
            StackProgramSource::Idl {
                idl: "./idl/b.json".into()
            }
        );
        // An unknown profile cannot be expressed, so it fails at the boundary.
        assert!(
            serde_json::from_value::<StackSourceOptions>(json!({ "profile": "other" })).is_err()
        );
        assert!(serde_json::from_value::<StackSourceOptions>(json!({ "extra": 1 })).is_err());
        assert!(serde_json::from_value::<StackProgramSource>(json!({ "registry": "a" })).is_err());
    }
}
