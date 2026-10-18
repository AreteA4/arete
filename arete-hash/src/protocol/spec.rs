//! Validation for `arete.protocol-spec/v1`.
//!
//! Validation runs in two passes:
//!
//! 1. **Shape**: every field, type, bound and identifier, in document order.
//! 2. **Semantics**: references between sections and the static checks on
//!    machines, steps, roles, reactions and events.
//!
//! Expression sources are only bounded here. The platform's expression
//! checker parses and type-checks them, and template resolution against
//! ProgramSpecs lives in [`super::template`].

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::error::{ProtocolContractError, ProtocolResult};
use super::model::*;
use super::shape::*;
use crate::{hash_jcs, HashId, ProgramRelease, ProgramSpec, StackManifest};

const MAX_PROGRAMS: usize = 32;
const MAX_STACKS: usize = 16;
const MAX_ROLES: usize = 16;
const MAX_RESOURCES: usize = 32;
const MAX_READS: usize = 64;
const MAX_STATES: usize = 64;
const MAX_DERIVE_RULES: usize = 64;
const MAX_STEPS: usize = 64;
const MAX_GUARDS: usize = 32;
const MAX_EPHEMERAL: usize = 8;
const MAX_TRANSITIONS: usize = 16;
const MAX_BIND_ACCOUNTS: usize = 32;
const MAX_ATAS: usize = 8;
const MAX_TEMPLATE_INSTRUCTIONS: usize = 64;
const MAX_TEMPLATE_FIELDS: usize = 64;
const MAX_REMAINING_ACCOUNTS: usize = 32;
const MAX_REACTIONS: usize = 128;
const MAX_EVENTS: usize = 64;
const MAX_SCHEMA_PROPERTIES: usize = 64;
const MAX_SCHEMA_DEPTH: usize = 4;
const MAX_ENUM_VALUES: usize = 64;
const MAX_METADATA_FIELDS: usize = 64;
const MAX_STACK_FIELDS: usize = 64;
const MAX_ACTIVITY: usize = 64;
const MAX_TITLE_BYTES: usize = 128;

/// Parse and fully validate a ProtocolSpec from JSON bytes.
pub fn parse_protocol_spec_v1(bytes: &[u8]) -> ProtocolResult<ProtocolSpecV1> {
    let value = super::parse_contract_json(bytes)?;
    validate_protocol_spec_value(&value)
}

/// Validate a ProtocolSpec JSON value and return the typed model.
/// Rejection codes a prepare returns for access refusals and terminal
/// lifecycle states. A guard may not use them, so a client can always tell
/// a guard failure from these.
pub const RESERVED_GUARD_CODES: [&str; 5] = [
    "step_requires_secret_key",
    "step_not_in_session_scope",
    "step_not_client_callable",
    "instance_terminal",
    "participant_terminal",
];

pub fn validate_protocol_spec_value(value: &Value) -> ProtocolResult<ProtocolSpecV1> {
    reject_floats(value, "")?;
    shape_spec(value)?;
    let spec: ProtocolSpecV1 = serde_json::from_value(value.clone()).map_err(|error| {
        ProtocolContractError::new("invalid-value", "", format!("model mismatch: {error}"))
    })?;
    check_semantics(&spec)?;
    Ok(spec)
}

impl ProtocolSpecV1 {
    pub fn hash(&self) -> Result<HashId<crate::ProtocolSpec>, crate::HashError> {
        hash_jcs::<crate::ProtocolSpec, _>(self)
    }

    pub fn has_handlers(&self) -> bool {
        self.steps
            .values()
            .any(|step| matches!(step.build, BuildV1::Handler { .. }))
    }

    pub fn create_step(&self) -> Option<(&String, &StepV1)> {
        self.steps
            .iter()
            .find(|(_, step)| step.scope == StepScope::Create)
    }

    /// The step's effective access for an integrator that narrowed it, or
    /// `None` for steps no client can call.
    pub fn effective_access(&self, step: &str, narrowed_to_secret: bool) -> Option<StepAccess> {
        let access = self.steps.get(step)?.access?;
        Some(if narrowed_to_secret {
            StepAccess::Secret
        } else {
            access
        })
    }

    pub fn machine(&self, kind: MachineKind) -> Option<&MachineV1> {
        match kind {
            MachineKind::Instance => Some(&self.machines.instance),
            MachineKind::Participant => self.machines.participant.as_ref(),
        }
    }
}

// ---------------------------------------------------------------------------
// Shape
// ---------------------------------------------------------------------------

const SPEC_KEYS: &[&str] = &[
    "schema",
    "name",
    "version",
    "description",
    "handlerAbi",
    "ctxVersion",
    "programs",
    "stacks",
    "params",
    "roles",
    "resources",
    "instanceVisibility",
    "reads",
    "machines",
    "steps",
    "reactions",
    "events",
    "metadata",
    "stack",
];

fn shape_spec(value: &Value) -> ProtocolResult<()> {
    let root = Obj::new(value, "", SPEC_KEYS)?;
    schema_tag(
        root.req("schema")?,
        &root.path_of("schema"),
        PROTOCOL_SPEC_SCHEMA_V1,
    )?;
    protocol_name(
        as_str(root.req("name")?, &root.path_of("name"))?,
        &root.path_of("name"),
    )?;
    semver(root.req("version")?, &root.path_of("version"))?;
    if let Some(value) = root.opt("description")? {
        description(value, &root.path_of("description"))?;
    }
    if let Some(value) = root.opt("handlerAbi")? {
        one_of(
            value,
            &root.path_of("handlerAbi"),
            &[PROTOCOL_HANDLER_ABI_V1],
        )?;
    }
    if let Some(value) = root.opt("ctxVersion")? {
        one_of(value, &root.path_of("ctxVersion"), &[PROTOCOL_CTX_V1])?;
    }
    shape_programs(root.req("programs")?, &root.path_of("programs"))?;
    shape_stacks(root.req("stacks")?, &root.path_of("stacks"))?;
    shape_object_schema(root.req("params")?, &root.path_of("params"))?;
    for (_, role, path) in as_map(
        root.req("roles")?,
        &root.path_of("roles"),
        MAX_ROLES,
        key_identifier,
    )? {
        let role = Obj::new(role, &path, &["description"])?;
        if let Some(value) = role.opt("description")? {
            description(value, &role.path_of("description"))?;
        }
    }
    for (_, resource, path) in as_map(
        root.req("resources")?,
        &root.path_of("resources"),
        MAX_RESOURCES,
        key_identifier,
    )? {
        let resource = Obj::new(resource, &path, &["schema", "description"])?;
        shape_field_schema(resource.req("schema")?, &resource.path_of("schema"), 1)?;
        if let Some(value) = resource.opt("description")? {
            description(value, &resource.path_of("description"))?;
        }
    }
    one_of(
        root.req("instanceVisibility")?,
        &root.path_of("instanceVisibility"),
        &["shared", "account"],
    )?;
    for (_, read, path) in as_map(
        root.req("reads")?,
        &root.path_of("reads"),
        MAX_READS,
        key_identifier,
    )? {
        shape_read(read, &path)?;
    }
    let machines = Obj::new(
        root.req("machines")?,
        &root.path_of("machines"),
        &["instance", "participant"],
    )?;
    shape_machine(machines.req("instance")?, &machines.path_of("instance"))?;
    if let Some(value) = machines.opt("participant")? {
        shape_machine(value, &machines.path_of("participant"))?;
    }
    for (_, step, path) in as_map(
        root.req("steps")?,
        &root.path_of("steps"),
        MAX_STEPS,
        key_identifier,
    )? {
        shape_step(step, &path)?;
    }
    let reactions_path = root.path_of("reactions");
    for (i, reaction) in as_array(root.req("reactions")?, &reactions_path, MAX_REACTIONS)?
        .iter()
        .enumerate()
    {
        shape_reaction(reaction, &index(&reactions_path, i))?;
    }
    for (_, event, path) in as_map(
        root.req("events")?,
        &root.path_of("events"),
        MAX_EVENTS,
        key_event,
    )? {
        let event = Obj::new(event, &path, &["machine", "description"])?;
        one_of(
            event.req("machine")?,
            &event.path_of("machine"),
            &["instance", "participant"],
        )?;
        if let Some(value) = event.opt("description")? {
            description(value, &event.path_of("description"))?;
        }
    }
    shape_metadata(root.req("metadata")?, &root.path_of("metadata"))?;
    shape_stack_surface(root.req("stack")?, &root.path_of("stack"))?;
    Ok(())
}

/// Read a `kind` tag, then open the object with the keys that kind allows.
pub(crate) fn tagged<'a>(
    value: &'a Value,
    path: &str,
    kinds: &[(&'a str, &[&str])],
) -> ProtocolResult<(&'a str, Obj<'a>)> {
    let map = value
        .as_object()
        .ok_or_else(|| err("invalid-type", path, "expected an object"))?;
    let kind_path = child(path, "kind");
    let kind = match map.get("kind") {
        None => return Err(err("missing-field", &kind_path, "missing field 'kind'")),
        Some(Value::Null) => {
            return Err(err(
                "null-not-allowed",
                &kind_path,
                "field 'kind' must not be null",
            ))
        }
        Some(kind) => as_str(kind, &kind_path)?,
    };
    let Some((name, keys)) = kinds.iter().find(|(name, _)| *name == kind) else {
        let names: Vec<&str> = kinds.iter().map(|(name, _)| *name).collect();
        return Err(err(
            "invalid-value",
            &kind_path,
            format!("expected one of {}", names.join(", ")),
        ));
    };
    let mut allowed = vec!["kind"];
    allowed.extend_from_slice(keys);
    Ok((name, Obj::new(value, path, &allowed)?))
}

fn shape_programs(value: &Value, path: &str) -> ProtocolResult<()> {
    let mut program_ids: BTreeMap<String, String> = BTreeMap::new();
    for (alias, program, program_path) in as_map(value, path, MAX_PROGRAMS, key_identifier)? {
        let (kind, program) = tagged(
            program,
            &program_path,
            &[
                (
                    "programSpec",
                    &["programId", "programSpecHash", "programReleaseHash"],
                ),
                ("builtin", &["builtin"]),
            ],
        )?;
        let (id, id_path) = if kind == "programSpec" {
            let id = pubkey(program.req("programId")?, &program.path_of("programId"))?.to_string();
            hash_id::<ProgramSpec>(
                program.req("programSpecHash")?,
                &program.path_of("programSpecHash"),
            )?;
            if let Some(value) = program.opt("programReleaseHash")? {
                hash_id::<ProgramRelease>(value, &program.path_of("programReleaseHash"))?;
            }
            (id, program.path_of("programId"))
        } else {
            let builtin = one_of(
                program.req("builtin")?,
                &program.path_of("builtin"),
                &BuiltinProgram::ALL,
            )?;
            let builtin: BuiltinProgram =
                serde_json::from_value(Value::String(builtin.to_string())).map_err(|_| {
                    err(
                        "invalid-value",
                        &program.path_of("builtin"),
                        "unknown builtin",
                    )
                })?;
            (builtin.program_id().to_string(), program.path_of("builtin"))
        };
        if let Some(previous) = program_ids.insert(id.clone(), alias.clone()) {
            return Err(err(
                "duplicate-entry",
                &id_path,
                format!("program {id} is already declared as '{previous}'"),
            ));
        }
    }
    Ok(())
}

fn shape_stacks(value: &Value, path: &str) -> ProtocolResult<()> {
    for (_, stack, stack_path) in as_map(value, path, MAX_STACKS, key_identifier)? {
        let stack = Obj::new(stack, &stack_path, &["stackManifestHash", "package"])?;
        hash_id::<StackManifest>(
            stack.req("stackManifestHash")?,
            &stack.path_of("stackManifestHash"),
        )?;
        if let Some(value) = stack.opt("package")? {
            bounded_str(value, &stack.path_of("package"), MAX_TITLE_BYTES)?;
        }
    }
    Ok(())
}

const FIELD_SCHEMA_KEYS: &[&str] = &[
    "type",
    "format",
    "enum",
    "maxLength",
    "minimum",
    "maximum",
    "items",
    "maxItems",
    "properties",
    "required",
    "additionalProperties",
    "description",
];

fn shape_object_schema(value: &Value, path: &str) -> ProtocolResult<()> {
    let schema = Obj::new(
        value,
        path,
        &["type", "properties", "required", "additionalProperties"],
    )?;
    shape_object_body(&schema, 1)
}

fn shape_object_body(schema: &Obj<'_>, depth: usize) -> ProtocolResult<()> {
    let type_path = schema.path_of("type");
    if as_str(schema.req("type")?, &type_path)? != "object" {
        return Err(err("invalid-schema", &type_path, "expected type 'object'"));
    }
    let properties = as_map(
        schema.req("properties")?,
        &schema.path_of("properties"),
        MAX_SCHEMA_PROPERTIES,
        key_identifier,
    )?;
    for (_, property, property_path) in &properties {
        shape_field_schema(property, property_path, depth + 1)?;
    }
    let names: BTreeSet<&str> = properties
        .iter()
        .map(|(name, _, _)| name.as_str())
        .collect();
    let required_path = schema.path_of("required");
    let required = unique_strings(
        schema.req("required")?,
        &required_path,
        MAX_SCHEMA_PROPERTIES,
        identifier_value,
    )?;
    for (i, name) in required.iter().enumerate() {
        if !names.contains(name) {
            return Err(err(
                "invalid-schema",
                &index(&required_path, i),
                format!("required property '{name}' is not declared"),
            ));
        }
    }
    let additional_path = schema.path_of("additionalProperties");
    if as_bool(schema.req("additionalProperties")?, &additional_path)? {
        return Err(err(
            "invalid-schema",
            &additional_path,
            "additionalProperties must be false",
        ));
    }
    Ok(())
}

fn shape_field_schema(value: &Value, path: &str, depth: usize) -> ProtocolResult<()> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(err(
            "invalid-schema",
            path,
            format!("schemas nest at most {MAX_SCHEMA_DEPTH} levels"),
        ));
    }
    let schema = Obj::new(value, path, FIELD_SCHEMA_KEYS)?;
    let field_type = one_of(
        schema.req("type")?,
        &schema.path_of("type"),
        &["string", "boolean", "integer", "array", "object"],
    )?;
    let allowed: &[&str] = match field_type {
        "string" => &["type", "format", "enum", "maxLength", "description"],
        "boolean" => &["type", "description"],
        "integer" => &["type", "minimum", "maximum", "description"],
        "array" => &["type", "items", "maxItems", "description"],
        _ => &[
            "type",
            "properties",
            "required",
            "additionalProperties",
            "description",
        ],
    };
    for key in FIELD_SCHEMA_KEYS {
        if schema.has(key) && !allowed.contains(key) {
            return Err(err(
                "invalid-schema",
                &schema.path_of(key),
                format!("'{key}' is not allowed on type '{field_type}'"),
            ));
        }
    }
    match field_type {
        "string" => {
            if let Some(value) = schema.opt("format")? {
                one_of(value, &schema.path_of("format"), &StringFormat::ALL)?;
            }
            if let Some(value) = schema.opt("enum")? {
                let enum_path = schema.path_of("enum");
                if schema.has("format") {
                    return Err(err(
                        "invalid-schema",
                        &enum_path,
                        "'enum' cannot be combined with 'format'",
                    ));
                }
                let values = unique_strings(value, &enum_path, MAX_ENUM_VALUES, |value, path| {
                    let text = bounded_str(value, path, MAX_IDENTIFIER_BYTES)?;
                    if text.is_empty() {
                        return Err(err("invalid-value", path, "enum values must not be empty"));
                    }
                    Ok(text)
                })?;
                if values.is_empty() {
                    return Err(err(
                        "invalid-schema",
                        &enum_path,
                        "'enum' must list at least one value",
                    ));
                }
            }
            if let Some(value) = schema.opt("maxLength")? {
                as_integer(value, &schema.path_of("maxLength"), 1, 4096)?;
            }
        }
        "integer" => {
            let minimum = match schema.opt("minimum")? {
                Some(value) => Some(as_integer(
                    value,
                    &schema.path_of("minimum"),
                    -MAX_SAFE,
                    MAX_SAFE,
                )?),
                None => None,
            };
            let maximum = match schema.opt("maximum")? {
                Some(value) => Some(as_integer(
                    value,
                    &schema.path_of("maximum"),
                    -MAX_SAFE,
                    MAX_SAFE,
                )?),
                None => None,
            };
            if let (Some(minimum), Some(maximum)) = (minimum, maximum) {
                if minimum > maximum {
                    return Err(err(
                        "invalid-schema",
                        &schema.path_of("maximum"),
                        "maximum is below minimum",
                    ));
                }
            }
        }
        "array" => {
            shape_field_schema(schema.req("items")?, &schema.path_of("items"), depth + 1)?;
            as_integer(schema.req("maxItems")?, &schema.path_of("maxItems"), 1, 256)?;
        }
        "object" => shape_object_body(&schema, depth)?,
        _ => {}
    }
    if let Some(value) = schema.opt("description")? {
        description(value, &schema.path_of("description"))?;
    }
    Ok(())
}

const MAX_SAFE: i64 = 9_007_199_254_740_991;

fn shape_read(value: &Value, path: &str) -> ProtocolResult<()> {
    let (kind, read) = tagged(
        value,
        path,
        &[
            ("account", &["program", "account", "address", "optional"]),
            ("tokenAccount", &["address", "optional"]),
            ("lamports", &["address"]),
        ],
    )?;
    if kind == "account" {
        identifier_value(read.req("program")?, &read.path_of("program"))?;
        identifier_value(read.req("account")?, &read.path_of("account"))?;
    }
    expression(read.req("address")?, &read.path_of("address"))?;
    if kind != "lamports" {
        as_bool(read.req("optional")?, &read.path_of("optional"))?;
    }
    Ok(())
}

fn shape_machine(value: &Value, path: &str) -> ProtocolResult<()> {
    let machine = Obj::new(value, path, &["initial", "states", "derive", "reads"])?;
    snake_value(machine.req("initial")?, &machine.path_of("initial"))?;
    let states = as_map(
        machine.req("states")?,
        &machine.path_of("states"),
        MAX_STATES,
        key_snake,
    )?;
    if states.is_empty() {
        return Err(err(
            "invalid-value",
            &machine.path_of("states"),
            "a machine needs at least one state",
        ));
    }
    for (_, state, state_path) in states {
        let state = Obj::new(state, &state_path, &["kind", "terminal", "description"])?;
        one_of(
            state.req("kind")?,
            &state.path_of("kind"),
            &["recorded", "derived"],
        )?;
        as_bool(state.req("terminal")?, &state.path_of("terminal"))?;
        if let Some(value) = state.opt("description")? {
            description(value, &state.path_of("description"))?;
        }
    }
    let derive_path = machine.path_of("derive");
    for (i, rule) in as_array(machine.req("derive")?, &derive_path, MAX_DERIVE_RULES)?
        .iter()
        .enumerate()
    {
        let rule = Obj::new(rule, &index(&derive_path, i), &["from", "to", "when", "at"])?;
        let from = unique_strings(
            rule.req("from")?,
            &rule.path_of("from"),
            MAX_STATES,
            snake_value,
        )?;
        if from.is_empty() {
            return Err(err(
                "invalid-value",
                &rule.path_of("from"),
                "a derivation needs at least one source state",
            ));
        }
        snake_value(rule.req("to")?, &rule.path_of("to"))?;
        expression(rule.req("when")?, &rule.path_of("when"))?;
        if let Some(value) = rule.opt("at")? {
            expression(value, &rule.path_of("at"))?;
        }
    }
    unique_strings(
        machine.req("reads")?,
        &machine.path_of("reads"),
        MAX_READS,
        identifier_value,
    )?;
    Ok(())
}

const STEP_KEYS: &[&str] = &[
    "scope",
    "title",
    "description",
    "authority",
    "access",
    "args",
    "guards",
    "reads",
    "ephemeral",
    "fee",
    "transitions",
    "bind",
    "glue",
    "build",
];

fn shape_step(value: &Value, path: &str) -> ProtocolResult<()> {
    let step = Obj::new(value, path, STEP_KEYS)?;
    one_of(
        step.req("scope")?,
        &step.path_of("scope"),
        &["create", "instance", "participant"],
    )?;
    if let Some(value) = step.opt("title")? {
        bounded_str(value, &step.path_of("title"), MAX_TITLE_BYTES)?;
    }
    if let Some(value) = step.opt("description")? {
        description(value, &step.path_of("description"))?;
    }
    let (kind, authority) = tagged(
        step.req("authority")?,
        &step.path_of("authority"),
        &[
            ("creator", &[]),
            ("participant", &[]),
            ("role", &["role"]),
            ("keeper", &[]),
            ("squadsProposer", &["multisig"]),
        ],
    )?;
    match kind {
        "role" => {
            identifier_value(authority.req("role")?, &authority.path_of("role"))?;
        }
        "squadsProposer" => {
            expression(authority.req("multisig")?, &authority.path_of("multisig"))?;
        }
        _ => {}
    }
    if let Some(value) = step.opt("access")? {
        one_of(value, &step.path_of("access"), &["public", "secret"])?;
    }
    shape_object_schema(step.req("args")?, &step.path_of("args"))?;
    let guards_path = step.path_of("guards");
    for (i, guard) in as_array(step.req("guards")?, &guards_path, MAX_GUARDS)?
        .iter()
        .enumerate()
    {
        let guard = Obj::new(guard, &index(&guards_path, i), &["code", "expr", "message"])?;
        snake_value(guard.req("code")?, &guard.path_of("code"))?;
        expression(guard.req("expr")?, &guard.path_of("expr"))?;
        if let Some(value) = guard.opt("message")? {
            description(value, &guard.path_of("message"))?;
        }
    }
    unique_strings(
        step.req("reads")?,
        &step.path_of("reads"),
        MAX_READS,
        identifier_value,
    )?;
    unique_strings(
        step.req("ephemeral")?,
        &step.path_of("ephemeral"),
        MAX_EPHEMERAL,
        identifier_value,
    )?;
    shape_fee(step.req("fee")?, &step.path_of("fee"))?;
    let transitions_path = step.path_of("transitions");
    for (i, transition) in as_array(step.req("transitions")?, &transitions_path, MAX_TRANSITIONS)?
        .iter()
        .enumerate()
    {
        let transition = Obj::new(
            transition,
            &index(&transitions_path, i),
            &["machine", "from", "to"],
        )?;
        one_of(
            transition.req("machine")?,
            &transition.path_of("machine"),
            &["instance", "participant"],
        )?;
        unique_strings(
            transition.req("from")?,
            &transition.path_of("from"),
            MAX_STATES,
            snake_value,
        )?;
        snake_value(transition.req("to")?, &transition.path_of("to"))?;
    }
    if let Some(value) = step.opt("bind")? {
        let bind = Obj::new(value, &step.path_of("bind"), &["accounts", "roles"])?;
        for (_, expr, expr_path) in as_map(
            bind.req("accounts")?,
            &bind.path_of("accounts"),
            MAX_BIND_ACCOUNTS,
            key_identifier,
        )? {
            expression(expr, &expr_path)?;
        }
        for (_, expr, expr_path) in as_map(
            bind.req("roles")?,
            &bind.path_of("roles"),
            MAX_ROLES,
            key_identifier,
        )? {
            expression(expr, &expr_path)?;
        }
    }
    let glue = Obj::new(
        step.req("glue")?,
        &step.path_of("glue"),
        &["atas", "wrapSol", "unwrapSol"],
    )?;
    let atas_path = glue.path_of("atas");
    for (i, ata) in as_array(glue.req("atas")?, &atas_path, MAX_ATAS)?
        .iter()
        .enumerate()
    {
        let ata = Obj::new(ata, &index(&atas_path, i), &["owner", "mint"])?;
        expression(ata.req("owner")?, &ata.path_of("owner"))?;
        expression(ata.req("mint")?, &ata.path_of("mint"))?;
    }
    if let Some(value) = glue.opt("wrapSol")? {
        let wrap = Obj::new(value, &glue.path_of("wrapSol"), &["amount"])?;
        expression(wrap.req("amount")?, &wrap.path_of("amount"))?;
    }
    as_bool(glue.req("unwrapSol")?, &glue.path_of("unwrapSol"))?;
    let (kind, build) = tagged(
        step.req("build")?,
        &step.path_of("build"),
        &[
            ("template", &["instructions"]),
            ("handler", &["export", "programs"]),
        ],
    )?;
    if kind == "template" {
        let instructions_path = build.path_of("instructions");
        let instructions = as_array(
            build.req("instructions")?,
            &instructions_path,
            MAX_TEMPLATE_INSTRUCTIONS,
        )?;
        if instructions.is_empty() {
            return Err(err(
                "invalid-value",
                &instructions_path,
                "a template needs at least one instruction",
            ));
        }
        for (i, instruction) in instructions.iter().enumerate() {
            let instruction = Obj::new(
                instruction,
                &index(&instructions_path, i),
                &[
                    "program",
                    "instruction",
                    "args",
                    "accounts",
                    "remainingAccounts",
                ],
            )?;
            identifier_value(instruction.req("program")?, &instruction.path_of("program"))?;
            identifier_value(
                instruction.req("instruction")?,
                &instruction.path_of("instruction"),
            )?;
            for key in ["args", "accounts"] {
                for (_, expr, expr_path) in as_map(
                    instruction.req(key)?,
                    &instruction.path_of(key),
                    MAX_TEMPLATE_FIELDS,
                    key_identifier,
                )? {
                    expression(expr, &expr_path)?;
                }
            }
            if let Some(value) = instruction.opt("remainingAccounts")? {
                shape_remaining_accounts(value, &instruction.path_of("remainingAccounts"))?;
            }
        }
    } else {
        identifier_value(build.req("export")?, &build.path_of("export"))?;
        unique_strings(
            build.req("programs")?,
            &build.path_of("programs"),
            MAX_PROGRAMS,
            identifier_value,
        )?;
    }
    Ok(())
}

/// `remainingAccounts`: a non-empty list (omitted when there are none) of
/// `{ pubkey, isWritable, isSigner }`. Remaining accounts never sign.
fn shape_remaining_accounts(value: &Value, path: &str) -> ProtocolResult<()> {
    let accounts = as_array(value, path, MAX_REMAINING_ACCOUNTS)?;
    if accounts.is_empty() {
        return Err(err(
            "invalid-value",
            path,
            "omit remainingAccounts instead of giving an empty list",
        ));
    }
    for (i, account) in accounts.iter().enumerate() {
        let account = Obj::new(
            account,
            &index(path, i),
            &["pubkey", "isWritable", "isSigner"],
        )?;
        expression(account.req("pubkey")?, &account.path_of("pubkey"))?;
        as_bool(account.req("isWritable")?, &account.path_of("isWritable"))?;
        if as_bool(account.req("isSigner")?, &account.path_of("isSigner"))? {
            return Err(err(
                "remaining-account-signer",
                &account.path_of("isSigner"),
                "remaining accounts cannot sign; name signers as IDL accounts",
            ));
        }
    }
    Ok(())
}

fn shape_fee(value: &Value, path: &str) -> ProtocolResult<()> {
    let (kind, fee) = tagged(
        value,
        path,
        &[
            ("none", &[]),
            ("input", &["amountArg", "asset"]),
            ("fixed", &[]),
        ],
    )?;
    if kind == "input" {
        identifier_value(fee.req("amountArg")?, &fee.path_of("amountArg"))?;
        let (asset_kind, asset) = tagged(
            fee.req("asset")?,
            &fee.path_of("asset"),
            &[("native", &[]), ("spl", &["mint", "sourceArg"])],
        )?;
        if asset_kind == "spl" {
            expression(asset.req("mint")?, &asset.path_of("mint"))?;
            identifier_value(asset.req("sourceArg")?, &asset.path_of("sourceArg"))?;
        }
    }
    Ok(())
}

fn shape_reaction(value: &Value, path: &str) -> ProtocolResult<()> {
    let reaction = Obj::new(value, path, &["id", "trigger", "action"])?;
    identifier_value(reaction.req("id")?, &reaction.path_of("id"))?;
    let (kind, trigger) = tagged(
        reaction.req("trigger")?,
        &reaction.path_of("trigger"),
        &[
            ("at", &["machine", "in", "time"]),
            ("when", &["machine", "in", "condition"]),
            ("onTransition", &["machine", "from", "to"]),
            ("onConfirmed", &["step"]),
        ],
    )?;
    match kind {
        "at" | "when" => {
            one_of(
                trigger.req("machine")?,
                &trigger.path_of("machine"),
                &["instance", "participant"],
            )?;
            let states = unique_strings(
                trigger.req("in")?,
                &trigger.path_of("in"),
                MAX_STATES,
                snake_value,
            )?;
            if states.is_empty() {
                return Err(err(
                    "invalid-value",
                    &trigger.path_of("in"),
                    "list at least one state",
                ));
            }
            let key = if kind == "at" { "time" } else { "condition" };
            expression(trigger.req(key)?, &trigger.path_of(key))?;
        }
        "onTransition" => {
            one_of(
                trigger.req("machine")?,
                &trigger.path_of("machine"),
                &["instance", "participant"],
            )?;
            unique_strings(
                trigger.req("from")?,
                &trigger.path_of("from"),
                MAX_STATES,
                snake_value,
            )?;
            let to = unique_strings(
                trigger.req("to")?,
                &trigger.path_of("to"),
                MAX_STATES,
                snake_value,
            )?;
            if to.is_empty() {
                return Err(err(
                    "invalid-value",
                    &trigger.path_of("to"),
                    "list at least one state",
                ));
            }
        }
        _ => {
            identifier_value(trigger.req("step")?, &trigger.path_of("step"))?;
        }
    }
    let (kind, action) = tagged(
        reaction.req("action")?,
        &reaction.path_of("action"),
        &[
            ("runKeeperStep", &["step"]),
            ("proposeSquads", &["step"]),
            ("emit", &["event"]),
        ],
    )?;
    if kind == "emit" {
        let event_path = action.path_of("event");
        event_name(as_str(action.req("event")?, &event_path)?, &event_path)?;
    } else {
        identifier_value(action.req("step")?, &action.path_of("step"))?;
    }
    Ok(())
}

fn shape_metadata(value: &Value, path: &str) -> ProtocolResult<()> {
    let metadata = Obj::new(value, path, &["instance", "participant"])?;
    for machine in ["instance", "participant"] {
        let Some(value) = metadata.opt(machine)? else {
            continue;
        };
        let schema = Obj::new(value, &metadata.path_of(machine), &["fields", "maxBytes"])?;
        for (_, field, field_path) in as_map(
            schema.req("fields")?,
            &schema.path_of("fields"),
            MAX_METADATA_FIELDS,
            key_identifier,
        )? {
            let field = Obj::new(field, &field_path, &["schema", "writer", "filterable"])?;
            shape_field_schema(field.req("schema")?, &field.path_of("schema"), 1)?;
            let writers: &[&str] = if machine == "instance" {
                &["creator"]
            } else {
                &["creator", "participant"]
            };
            one_of(field.req("writer")?, &field.path_of("writer"), writers)?;
            as_bool(field.req("filterable")?, &field.path_of("filterable"))?;
        }
        as_integer(
            schema.req("maxBytes")?,
            &schema.path_of("maxBytes"),
            1,
            65_536,
        )?;
    }
    Ok(())
}

fn field_path_value<'a>(value: &'a Value, path: &str) -> ProtocolResult<&'a str> {
    let text = bounded_str(value, path, 128)?;
    let valid = !text.is_empty()
        && text.split('.').all(|segment| {
            let bytes = segment.as_bytes();
            !bytes.is_empty()
                && (bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
                && bytes[1..]
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
        });
    if valid {
        Ok(text)
    } else {
        Err(err(
            "invalid-identifier",
            path,
            format!("'{text}' must be a dotted field path"),
        ))
    }
}

fn shape_stack_surface(value: &Value, path: &str) -> ProtocolResult<()> {
    let stack = Obj::new(
        value,
        path,
        &["instanceAccounts", "instanceFields", "activity"],
    )?;
    for (_, account, account_path) in as_map(
        stack.req("instanceAccounts")?,
        &stack.path_of("instanceAccounts"),
        MAX_BIND_ACCOUNTS,
        key_identifier,
    )? {
        let account = Obj::new(account, &account_path, &["program", "account"])?;
        identifier_value(account.req("program")?, &account.path_of("program"))?;
        identifier_value(account.req("account")?, &account.path_of("account"))?;
    }
    for (_, field, field_path) in as_map(
        stack.req("instanceFields")?,
        &stack.path_of("instanceFields"),
        MAX_STACK_FIELDS,
        key_identifier,
    )? {
        let field = Obj::new(field, &field_path, &["account", "field"])?;
        identifier_value(field.req("account")?, &field.path_of("account"))?;
        field_path_value(field.req("field")?, &field.path_of("field"))?;
    }
    let activity_path = stack.path_of("activity");
    for (i, activity) in as_array(stack.req("activity")?, &activity_path, MAX_ACTIVITY)?
        .iter()
        .enumerate()
    {
        let activity = Obj::new(
            activity,
            &index(&activity_path, i),
            &[
                "step",
                "program",
                "instruction",
                "instanceAccount",
                "walletAccount",
                "amountArg",
            ],
        )?;
        for key in [
            "step",
            "program",
            "instruction",
            "instanceAccount",
            "walletAccount",
        ] {
            identifier_value(activity.req(key)?, &activity.path_of(key))?;
        }
        if let Some(value) = activity.opt("amountArg")? {
            identifier_value(value, &activity.path_of("amountArg"))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Semantics
// ---------------------------------------------------------------------------

fn check_semantics(spec: &ProtocolSpecV1) -> ProtocolResult<()> {
    check_handler_abi(spec)?;
    check_reads(spec)?;
    check_machine(spec, MachineKind::Instance)?;
    check_participant_machine_present(spec)?;
    if spec.machines.participant.is_some() {
        check_machine(spec, MachineKind::Participant)?;
    }
    check_steps(spec)?;
    check_stack_surface(spec)?;
    check_reachability(spec, MachineKind::Instance)?;
    if spec.machines.participant.is_some() {
        check_reachability(spec, MachineKind::Participant)?;
    }
    check_reactions(spec)?;
    check_events(spec)?;
    Ok(())
}

fn check_handler_abi(spec: &ProtocolSpecV1) -> ProtocolResult<()> {
    let handlers = spec.has_handlers();
    match (handlers, spec.handler_abi.is_some()) {
        (true, false) => {
            return Err(err(
                "handler-abi-required",
                "/handlerAbi",
                "handler steps need handlerAbi",
            ))
        }
        (false, true) => {
            return Err(err(
                "handler-abi-unused",
                "/handlerAbi",
                "no step has a handler",
            ))
        }
        _ => {}
    }
    match (handlers, spec.ctx_version.is_some()) {
        (true, false) => Err(err(
            "handler-abi-required",
            "/ctxVersion",
            "handler steps need ctxVersion",
        )),
        (false, true) => Err(err(
            "handler-abi-unused",
            "/ctxVersion",
            "no step has a handler",
        )),
        _ => Ok(()),
    }
}

fn spec_program<'a>(
    spec: &'a ProtocolSpecV1,
    alias: &str,
    path: &str,
) -> ProtocolResult<&'a ProtocolProgramV1> {
    spec.programs.get(alias).ok_or_else(|| {
        err(
            "unknown-program",
            path,
            format!("program '{alias}' is not declared"),
        )
    })
}

fn require_program_spec(spec: &ProtocolSpecV1, alias: &str, path: &str) -> ProtocolResult<()> {
    match spec_program(spec, alias, path)? {
        ProtocolProgramV1::ProgramSpec { .. } => Ok(()),
        ProtocolProgramV1::Builtin { .. } => Err(err(
            "unknown-program",
            path,
            format!("program '{alias}' is a builtin and has no ProgramSpec"),
        )),
    }
}

fn check_reads(spec: &ProtocolSpecV1) -> ProtocolResult<()> {
    for (name, read) in &spec.reads {
        if let ReadV1::Account { program, .. } = read {
            require_program_spec(spec, program, &format!("/reads/{name}/program"))?;
        }
    }
    Ok(())
}

fn require_read(spec: &ProtocolSpecV1, name: &str, path: &str) -> ProtocolResult<()> {
    if spec.reads.contains_key(name) {
        Ok(())
    } else {
        Err(err(
            "unknown-read",
            path,
            format!("read '{name}' is not declared"),
        ))
    }
}

fn require_state(machine: &MachineV1, state: &str, path: &str) -> ProtocolResult<()> {
    if machine.states.contains_key(state) {
        Ok(())
    } else {
        Err(err(
            "unknown-state",
            path,
            format!("state '{state}' is not declared"),
        ))
    }
}

fn check_machine(spec: &ProtocolSpecV1, kind: MachineKind) -> ProtocolResult<()> {
    let machine = spec
        .machine(kind)
        .expect("caller checked the machine exists");
    let base = format!("/machines/{}", kind.as_str());
    require_state(machine, &machine.initial, &format!("{base}/initial"))?;
    for (i, rule) in machine.derive.iter().enumerate() {
        let rule_path = format!("{base}/derive/{i}");
        for (j, from) in rule.from.iter().enumerate() {
            require_state(machine, from, &format!("{rule_path}/from/{j}"))?;
        }
        require_state(machine, &rule.to, &format!("{rule_path}/to"))?;
        if machine.states[&rule.to].kind != StateKind::Derived {
            return Err(err(
                "derive-targets-recorded-state",
                &format!("{rule_path}/to"),
                format!(
                    "'{}' is a recorded state; derivations may only target derived states",
                    rule.to
                ),
            ));
        }
    }
    for (i, read) in machine.reads.iter().enumerate() {
        require_read(spec, read, &format!("{base}/reads/{i}"))?;
    }
    for (name, state) in &machine.states {
        if state.kind == StateKind::Derived
            && *name != machine.initial
            && !machine.derive.iter().any(|rule| rule.to == *name)
        {
            return Err(err(
                "derived-state-without-rule",
                &format!("{base}/states/{name}"),
                format!("no derivation targets derived state '{name}'"),
            ));
        }
    }
    Ok(())
}

fn uses_participant_machine(spec: &ProtocolSpecV1) -> Option<String> {
    for (name, step) in &spec.steps {
        if step.scope == StepScope::Participant {
            return Some(format!("/steps/{name}/scope"));
        }
        for (i, transition) in step.transitions.iter().enumerate() {
            if transition.machine == MachineKind::Participant {
                return Some(format!("/steps/{name}/transitions/{i}/machine"));
            }
        }
    }
    for (i, reaction) in spec.reactions.iter().enumerate() {
        let machine = match &reaction.trigger {
            TriggerV1::At { machine, .. }
            | TriggerV1::When { machine, .. }
            | TriggerV1::OnTransition { machine, .. } => Some(*machine),
            TriggerV1::OnConfirmed { .. } => None,
        };
        if machine == Some(MachineKind::Participant) {
            return Some(format!("/reactions/{i}/trigger/machine"));
        }
    }
    for (name, event) in &spec.events {
        if event.machine == MachineKind::Participant {
            return Some(format!("/events/{name}/machine"));
        }
    }
    if spec.metadata.participant.is_some() {
        return Some("/metadata/participant".to_string());
    }
    None
}

fn check_participant_machine_present(spec: &ProtocolSpecV1) -> ProtocolResult<()> {
    if spec.machines.participant.is_none() {
        if let Some(path) = uses_participant_machine(spec) {
            return Err(err(
                "missing-participant-machine",
                &path,
                "participant scope needs machines.participant",
            ));
        }
    }
    Ok(())
}

fn role_guard_present(step: &StepV1, role: &str) -> bool {
    let compact = |text: &str| {
        text.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
    };
    let forward = format!("wallet==instance.roles.{role}");
    let reverse = format!("instance.roles.{role}==wallet");
    step.guards.iter().any(|guard| {
        let expr = compact(&guard.expr);
        expr == forward || expr == reverse
    })
}

fn arg_schema<'a>(step: &'a StepV1, name: &str) -> Option<&'a FieldSchemaV1> {
    step.args.properties.get(name)
}

fn check_steps(spec: &ProtocolSpecV1) -> ProtocolResult<()> {
    let creates: Vec<&String> = spec
        .steps
        .iter()
        .filter(|(_, step)| step.scope == StepScope::Create)
        .map(|(name, _)| name)
        .collect();
    match creates.as_slice() {
        [] => {
            return Err(err(
                "create-step-count",
                "/steps",
                "exactly one create step is required",
            ))
        }
        [_] => {}
        [_, second, ..] => {
            return Err(err(
                "create-step-count",
                &format!("/steps/{second}/scope"),
                "exactly one create step is allowed",
            ))
        }
    }
    for (name, step) in &spec.steps {
        let base = format!("/steps/{name}");
        let authority_path = format!("{base}/authority");
        let authority_ok = match (&step.authority, step.scope) {
            (AuthorityV1::Creator, StepScope::Create) => true,
            (AuthorityV1::Participant, StepScope::Participant) => true,
            (AuthorityV1::Role { role }, StepScope::Instance | StepScope::Participant) => {
                if !spec.roles.contains_key(role) {
                    return Err(err(
                        "unknown-role",
                        &format!("{authority_path}/role"),
                        format!("role '{role}' is not declared"),
                    ));
                }
                true
            }
            (AuthorityV1::Keeper, StepScope::Instance | StepScope::Participant) => true,
            (AuthorityV1::SquadsProposer { .. }, StepScope::Instance) => true,
            _ => false,
        };
        if !authority_ok {
            return Err(err(
                "invalid-authority",
                &authority_path,
                "the authority is not allowed for this step scope",
            ));
        }
        match (step.authority.is_client_callable(), step.access.is_some()) {
            (true, false) => {
                return Err(err(
                    "missing-access",
                    &format!("{base}/access"),
                    "client-callable steps declare access 'public' or 'secret'",
                ))
            }
            (false, true) => {
                return Err(err(
                    "forbidden-access",
                    &format!("{base}/access"),
                    "keeper and admin steps are never client-callable",
                ))
            }
            _ => {}
        }
        let mut codes = BTreeSet::new();
        for (i, guard) in step.guards.iter().enumerate() {
            if RESERVED_GUARD_CODES.contains(&guard.code.as_str()) {
                return Err(err(
                    "reserved-guard-code",
                    &format!("{base}/guards/{i}/code"),
                    format!(
                        "guard code '{}' is reserved: prepares return it for access or lifecycle rejections",
                        guard.code
                    ),
                ));
            }
            if !codes.insert(guard.code.as_str()) {
                return Err(err(
                    "duplicate-guard-code",
                    &format!("{base}/guards/{i}/code"),
                    format!("guard code '{}' is used twice", guard.code),
                ));
            }
        }
        if let AuthorityV1::Role { role } = &step.authority {
            if !role_guard_present(step, role) {
                return Err(err(
                    "missing-role-guard",
                    &format!("{base}/guards"),
                    format!("role steps need the guard 'wallet == instance.roles.{role}'"),
                ));
            }
        }
        for (i, read) in step.reads.iter().enumerate() {
            require_read(spec, read, &format!("{base}/reads/{i}"))?;
        }
        check_fee(step, &base)?;
        check_transitions(spec, name, step, &base)?;
        check_bind(spec, step, &base)?;
        match &step.build {
            BuildV1::Template { instructions } => {
                for (i, instruction) in instructions.iter().enumerate() {
                    spec_program(
                        spec,
                        &instruction.program,
                        &format!("{base}/build/instructions/{i}/program"),
                    )?;
                }
            }
            BuildV1::Handler { programs, .. } => {
                for (i, program) in programs.iter().enumerate() {
                    spec_program(spec, program, &format!("{base}/build/programs/{i}"))?;
                }
            }
        }
    }
    Ok(())
}

fn check_fee(step: &StepV1, base: &str) -> ProtocolResult<()> {
    let fee_path = format!("{base}/fee");
    match &step.fee {
        FeeBasisV1::None => Ok(()),
        _ if !step.authority.is_client_callable() => Err(err(
            "invalid-fee-basis",
            &fee_path,
            "only wallet steps carry fees",
        )),
        FeeBasisV1::Fixed => Ok(()),
        FeeBasisV1::Input { amount_arg, asset } => {
            let amount_path = format!("{fee_path}/amountArg");
            let schema = arg_schema(step, amount_arg).ok_or_else(|| {
                err(
                    "unknown-arg",
                    &amount_path,
                    format!("arg '{amount_arg}' is not declared"),
                )
            })?;
            if schema.field_type != FieldType::String || schema.format != Some(StringFormat::U64) {
                return Err(err(
                    "invalid-fee-basis",
                    &amount_path,
                    "the amount arg must be a string with format 'u64'",
                ));
            }
            if let FeeAssetV1::Spl { source_arg, .. } = asset {
                let source_path = format!("{fee_path}/asset/sourceArg");
                let schema = arg_schema(step, source_arg).ok_or_else(|| {
                    err(
                        "unknown-arg",
                        &source_path,
                        format!("arg '{source_arg}' is not declared"),
                    )
                })?;
                if schema.field_type != FieldType::String
                    || schema.format != Some(StringFormat::Pubkey)
                {
                    return Err(err(
                        "invalid-fee-basis",
                        &source_path,
                        "the source arg must be a string with format 'pubkey'",
                    ));
                }
            }
            Ok(())
        }
    }
}

fn check_transitions(
    spec: &ProtocolSpecV1,
    _name: &str,
    step: &StepV1,
    base: &str,
) -> ProtocolResult<()> {
    if step.scope == StepScope::Create {
        if !step.transitions.is_empty() {
            return Err(err(
                "create-step-transition",
                &format!("{base}/transitions/0"),
                "a create step starts the instance in its initial state and declares no transitions",
            ));
        }
        return Ok(());
    }
    for (i, transition) in step.transitions.iter().enumerate() {
        let path = format!("{base}/transitions/{i}");
        if transition.machine == MachineKind::Participant && step.scope != StepScope::Participant {
            return Err(err(
                "invalid-value",
                &format!("{path}/machine"),
                "only participant steps move the participant machine",
            ));
        }
        let machine = spec
            .machine(transition.machine)
            .expect("participant machine checked");
        for (j, from) in transition.from.iter().enumerate() {
            require_state(machine, from, &format!("{path}/from/{j}"))?;
        }
        require_state(machine, &transition.to, &format!("{path}/to"))?;
        if machine.states[&transition.to].kind != StateKind::Recorded {
            return Err(err(
                "transition-targets-derived-state",
                &format!("{path}/to"),
                format!(
                    "'{}' is derived; steps may only move to recorded states",
                    transition.to
                ),
            ));
        }
    }
    Ok(())
}

fn check_bind(spec: &ProtocolSpecV1, step: &StepV1, base: &str) -> ProtocolResult<()> {
    let bind_path = format!("{base}/bind");
    match (step.scope, &step.bind) {
        (StepScope::Create, None) => Err(err(
            "missing-field",
            &bind_path,
            "the create step declares bind",
        )),
        (StepScope::Create, Some(bind)) => {
            for role in bind.roles.keys() {
                if !spec.roles.contains_key(role) {
                    return Err(err(
                        "unknown-role",
                        &format!("{bind_path}/roles/{role}"),
                        format!("role '{role}' is not declared"),
                    ));
                }
            }
            if let Some(role) = spec
                .roles
                .keys()
                .find(|role| !bind.roles.contains_key(*role))
            {
                return Err(err(
                    "role-not-bound",
                    &format!("{bind_path}/roles"),
                    format!("role '{role}' must be bound by the create step"),
                ));
            }
            Ok(())
        }
        (_, Some(_)) => Err(err(
            "bind-outside-create",
            &bind_path,
            "only the create step binds",
        )),
        (_, None) => Ok(()),
    }
}

fn check_stack_surface(spec: &ProtocolSpecV1) -> ProtocolResult<()> {
    let bound: BTreeSet<&String> = spec
        .create_step()
        .and_then(|(_, step)| step.bind.as_ref())
        .map(|bind| bind.accounts.keys().collect())
        .unwrap_or_default();
    for (name, account) in &spec.stack.instance_accounts {
        let path = format!("/stack/instanceAccounts/{name}");
        if !bound.contains(name) {
            return Err(err(
                "unknown-bound-account",
                &path,
                format!("'{name}' is not bound by the create step"),
            ));
        }
        require_program_spec(spec, &account.program, &format!("{path}/program"))?;
    }
    for (name, field) in &spec.stack.instance_fields {
        if !spec.stack.instance_accounts.contains_key(&field.account) {
            return Err(err(
                "unknown-bound-account",
                &format!("/stack/instanceFields/{name}/account"),
                format!("'{}' is not a stack instance account", field.account),
            ));
        }
    }
    for (i, activity) in spec.stack.activity.iter().enumerate() {
        let path = format!("/stack/activity/{i}");
        if !spec.steps.contains_key(&activity.step) {
            return Err(err(
                "unknown-step",
                &format!("{path}/step"),
                format!("step '{}' is not declared", activity.step),
            ));
        }
        require_program_spec(spec, &activity.program, &format!("{path}/program"))?;
    }
    Ok(())
}

/// Edges of a machine: step transitions (an empty `from` means every
/// non-terminal state) and derivation rules.
fn machine_edges(spec: &ProtocolSpecV1, kind: MachineKind) -> Vec<(Vec<String>, String)> {
    let machine = spec
        .machine(kind)
        .expect("caller checked the machine exists");
    let non_terminal: Vec<String> = machine
        .states
        .iter()
        .filter(|(_, state)| !state.terminal)
        .map(|(name, _)| name.clone())
        .collect();
    let mut edges = Vec::new();
    for step in spec.steps.values() {
        for transition in step.transitions.iter().filter(|t| t.machine == kind) {
            let from = if transition.from.is_empty() {
                non_terminal.clone()
            } else {
                transition.from.clone()
            };
            edges.push((from, transition.to.clone()));
        }
    }
    for rule in &machine.derive {
        edges.push((rule.from.clone(), rule.to.clone()));
    }
    edges
}

fn check_reachability(spec: &ProtocolSpecV1, kind: MachineKind) -> ProtocolResult<()> {
    let machine = spec
        .machine(kind)
        .expect("caller checked the machine exists");
    let base = format!("/machines/{}", kind.as_str());
    let edges = machine_edges(spec, kind);
    let mut reachable = BTreeSet::from([machine.initial.clone()]);
    loop {
        let before = reachable.len();
        for (from, to) in &edges {
            if from.iter().any(|state| reachable.contains(state)) {
                reachable.insert(to.clone());
            }
        }
        if reachable.len() == before {
            break;
        }
    }
    for (name, state) in &machine.states {
        let path = format!("{base}/states/{name}");
        if !reachable.contains(name) {
            return Err(err(
                "unreachable-state",
                &path,
                format!("'{name}' cannot be reached from '{}'", machine.initial),
            ));
        }
        let exits = edges
            .iter()
            .any(|(from, to)| from.contains(name) && to != name);
        if state.terminal && exits {
            return Err(err(
                "terminal-state-exit",
                &path,
                format!("terminal state '{name}' has an exit"),
            ));
        }
        if !state.terminal && !exits {
            return Err(err(
                "dead-end-state",
                &path,
                format!("non-terminal state '{name}' has no exit"),
            ));
        }
    }
    Ok(())
}

fn trigger_machine(spec: &ProtocolSpecV1, trigger: &TriggerV1) -> Option<MachineKind> {
    match trigger {
        TriggerV1::At { machine, .. }
        | TriggerV1::When { machine, .. }
        | TriggerV1::OnTransition { machine, .. } => Some(*machine),
        TriggerV1::OnConfirmed { step } => spec.steps.get(step).map(|step| {
            if step.scope == StepScope::Participant {
                MachineKind::Participant
            } else {
                MachineKind::Instance
            }
        }),
    }
}

fn check_reactions(spec: &ProtocolSpecV1) -> ProtocolResult<()> {
    let mut ids = BTreeSet::new();
    for (i, reaction) in spec.reactions.iter().enumerate() {
        let base = format!("/reactions/{i}");
        if !ids.insert(reaction.id.as_str()) {
            return Err(err(
                "duplicate-entry",
                &format!("{base}/id"),
                format!("reaction id '{}' is used twice", reaction.id),
            ));
        }
        let trigger_path = format!("{base}/trigger");
        match &reaction.trigger {
            TriggerV1::At {
                machine, in_states, ..
            }
            | TriggerV1::When {
                machine, in_states, ..
            } => {
                let machine = spec.machine(*machine).expect("participant machine checked");
                for (j, state) in in_states.iter().enumerate() {
                    require_state(machine, state, &format!("{trigger_path}/in/{j}"))?;
                }
            }
            TriggerV1::OnTransition { machine, from, to } => {
                let machine = spec.machine(*machine).expect("participant machine checked");
                for (j, state) in from.iter().enumerate() {
                    require_state(machine, state, &format!("{trigger_path}/from/{j}"))?;
                }
                for (j, state) in to.iter().enumerate() {
                    require_state(machine, state, &format!("{trigger_path}/to/{j}"))?;
                }
            }
            TriggerV1::OnConfirmed { step } => {
                if !spec.steps.contains_key(step) {
                    return Err(err(
                        "unknown-step",
                        &format!("{trigger_path}/step"),
                        format!("step '{step}' is not declared"),
                    ));
                }
            }
        }
        let action_path = format!("{base}/action");
        match &reaction.action {
            ActionV1::RunKeeperStep { step } | ActionV1::ProposeSquads { step } => {
                let target = spec.steps.get(step).ok_or_else(|| {
                    err(
                        "unknown-step",
                        &format!("{action_path}/step"),
                        format!("step '{step}' is not declared"),
                    )
                })?;
                let matches = match &reaction.action {
                    ActionV1::RunKeeperStep { .. } => {
                        matches!(target.authority, AuthorityV1::Keeper)
                    }
                    _ => matches!(target.authority, AuthorityV1::SquadsProposer { .. }),
                };
                if !matches {
                    return Err(err(
                        "reaction-action-mismatch",
                        &format!("{action_path}/step"),
                        "runKeeperStep targets keeper steps and proposeSquads targets squadsProposer steps",
                    ));
                }
            }
            ActionV1::Emit { event } => {
                let declared = spec.events.get(event).ok_or_else(|| {
                    err(
                        "unknown-event",
                        &format!("{action_path}/event"),
                        format!("event '{event}' is not declared"),
                    )
                })?;
                if declared.machine == MachineKind::Participant
                    && trigger_machine(spec, &reaction.trigger) != Some(MachineKind::Participant)
                {
                    return Err(err(
                        "event-machine-mismatch",
                        &format!("{action_path}/event"),
                        "participant events need a participant trigger",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn check_events(spec: &ProtocolSpecV1) -> ProtocolResult<()> {
    for name in spec.events.keys() {
        let emitted = spec
            .reactions
            .iter()
            .any(|reaction| matches!(&reaction.action, ActionV1::Emit { event } if event == name));
        if !emitted {
            return Err(err(
                "event-not-emitted",
                &format!("/events/{name}"),
                format!("no emit reaction publishes '{name}'"),
            ));
        }
    }
    Ok(())
}
