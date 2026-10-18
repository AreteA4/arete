//! The Rust client of a protocol package.
//!
//! `a4 install protocol <ref> --rust` writes a crate (or, with
//! `sdk.rust.module_mode`, a module directory):
//!
//! - `src/lib.rs` (`mod.rs` in module mode): the protocol definition
//!   (`<NAME>_PROTOCOL`), typed params, per-step args and guard-code enums,
//!   machine states, metadata documents and event names, a webhook verifier,
//!   and two clients over `arete_sdk::protocol::ProtocolClient`: one for
//!   publishable keys with only the `public` steps, one for secret keys with
//!   every client-callable step;
//! - `stacks/<stack>/`: each stack the protocol hoists, generated exactly as
//!   `a4 install stack --rust` generates it, re-exported as `stacks::<key>`;
//! - `programs/<program>/`: each program the protocol declares that none of
//!   its stacks bundles, generated as `a4 install program --rust` does;
//!   every program is re-exported as `programs::<key>`;
//! - `Cargo.toml` (crate mode), `protocol-spec.json`, `README.md`,
//!   `sdk-manifest.json` and `sdk-provenance.json`.
//!
//! Nothing here carries a credential: the deployment is named by id and URL
//! only, and the app's own key supplies identity.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;

use anyhow::{bail, Context, Result};
use arete_hash::protocol::{
    FieldSchemaV1, FieldType, MetadataSchemaV1, MetadataWriter, ObjectSchemaV1, ProtocolSpecV1,
    StepAccess, StepScope, StepV1,
};
use arete_interpreter::identifiers::{rust as rust_ident, IdentifierCase};

use super::protocol::{
    access_name, authority_name, collect_payload_files, is_integer_format, machine_name,
    scope_name, step_access, PROTOCOL_SPEC_FILE, README_FILE,
};
use super::{
    build_sdk_provenance_manifest_from_artifacts, ensure_file_stem,
    generate_project_registry_dependency, program_spec_artifact_from_registry,
    write_sdk_provenance_manifest_file, ExtensionsInputKind, ProjectGenerationOptions,
    ResolvedExtensionsInputPin, SDK_PROVENANCE_FILE,
};
use crate::config::to_kebab_case;
use crate::project::manifest::InstallTarget;
use crate::project::protocol::{
    hoist_protocol_programs, protocol_spec, unbundled_protocol_programs, HoistedProgramSource,
};
use crate::project::resolver::{
    ResolvedProtocol, ResolvedProtocolLiveStack, ResolvedRegistryDependency,
};

/// How the protocol module reaches one hoisted stack or program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HoistedRust {
    /// A sibling crate: `pub use ::<crate> as <key>;`, plus the path
    /// dependency `<package> = { path = "<path>" }`.
    Crate { package: String, path: String },
    /// A module directory: `#[path = "<dir>/mod.rs"] pub mod <key>;` inside
    /// the `stacks` or `programs` module.
    Module { directory: String },
    /// A path to re-export: `pub use <path> as <key>;`.
    Reexport { path: String },
}

pub(super) struct RustProtocolContext<'a> {
    pub alias: &'a str,
    pub package: &'a str,
    pub version: &'a str,
    pub package_release_hash: &'a str,
    pub protocol: &'a ResolvedProtocol,
    pub spec: &'a ProtocolSpecV1,
    /// Spec stack key -> how the module reaches it.
    pub stacks: BTreeMap<String, HoistedRust>,
    /// Spec program key -> how the module reaches it.
    pub programs: BTreeMap<String, HoistedRust>,
}

/// The Rust crate (package) a generated crate is named as: `<alias>-<kind>`.
fn crate_ident(package: &str) -> String {
    package.replace('-', "_")
}

pub(super) fn generate_rust_protocol(
    dependency: &ResolvedRegistryDependency,
    options: ProjectGenerationOptions<'_>,
) -> Result<()> {
    let ResolvedRegistryDependency::Protocol {
        alias,
        package,
        version,
        package_release_hash,
        generator_contract,
        protocol,
        stacks,
        programs,
        unpinned_programs,
    } = dependency
    else {
        bail!("Expected a protocol resolution");
    };
    ensure_file_stem(alias)?;
    let spec = protocol_spec(protocol)?;
    check_rust_names(&spec)?;
    let hoisted = hoist_protocol_programs(
        package,
        protocol,
        &spec,
        stacks,
        programs,
        unpinned_programs,
    )?;
    let unbundled = unbundled_protocol_programs(protocol, unpinned_programs);
    fs::create_dir_all(options.output).with_context(|| {
        format!(
            "Failed to create protocol output {}",
            options.output.display()
        )
    })?;
    let module_mode = options.rust_module;

    // Each hoisted stack, exactly as a stack install generates it.
    let mut stack_members = BTreeMap::new();
    let mut ordered_stacks = stacks.iter().collect::<Vec<_>>();
    ordered_stacks.sort_by(|left, right| left.alias().cmp(right.alias()));
    for stack in ordered_stacks {
        let key = stack.alias().to_string();
        let directory = to_kebab_case(&key);
        ensure_file_stem(&directory)?;
        let mut nested = stack.clone();
        if let ResolvedRegistryDependency::Stack { alias, .. } = &mut nested {
            alias.clone_from(&directory);
        }
        generate_project_registry_dependency(
            &nested,
            ProjectGenerationOptions {
                alias: &directory,
                target: InstallTarget::Rust,
                output: &options.output.join("stacks").join(&directory),
                typescript_package: options.typescript_package,
                rust_module: module_mode,
                python_module: false,
                stack_endpoints: None,
                reference: None,
                program_install: None,
            },
        )
        .with_context(|| format!("Failed to generate stack '{key}' of protocol '{alias}'"))?;
        stack_members.insert(
            key,
            if module_mode {
                HoistedRust::Module { directory }
            } else {
                HoistedRust::Crate {
                    package: format!("{directory}-stack"),
                    path: format!("stacks/{directory}"),
                }
            },
        );
    }

    // Each program no stack bundles, as a program install generates it; the
    // rest are the bundling stack's program modules.
    let mut program_members = BTreeMap::new();
    for program in &hoisted {
        match program.source {
            HoistedProgramSource::Stack {
                stack,
                program: install,
            } => {
                let artifact = program_spec_artifact_from_registry(install)?;
                let module = arete_interpreter::rust::rust_module_name(
                    &artifact.payload.idl_snapshot.snapshot.name,
                );
                let stack_path = match stack_members.get(stack) {
                    Some(HoistedRust::Crate { package, .. }) => {
                        format!("::{}", crate_ident(package))
                    }
                    Some(_) => format!("super::stacks::{}", rust_snake(stack)),
                    None => bail!(
                        "Hoisted program '{}' names unknown stack '{stack}'",
                        program.key
                    ),
                };
                program_members.insert(
                    program.key.clone(),
                    HoistedRust::Reexport {
                        path: format!("{stack_path}::programs::{module}"),
                    },
                );
            }
            HoistedProgramSource::Direct(install) => {
                let directory = to_kebab_case(&program.key);
                ensure_file_stem(&directory)?;
                let reference = install.program_package.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "Protocol program '{}' has no program package to generate",
                        program.key
                    )
                })?;
                let resolution = ResolvedRegistryDependency::Program {
                    alias: directory.clone(),
                    package: reference.package.clone(),
                    version: reference.version.clone(),
                    package_release_hash: reference.package_release_hash.clone(),
                    generator_contract: generator_contract.clone(),
                    install: Box::new(install.clone()),
                    sdk_extensions: install.sdk_extensions.clone().unwrap_or_default(),
                };
                generate_project_registry_dependency(
                    &resolution,
                    ProjectGenerationOptions {
                        alias: &directory,
                        target: InstallTarget::Rust,
                        output: &options.output.join("programs").join(&directory),
                        typescript_package: options.typescript_package,
                        rust_module: module_mode,
                        python_module: false,
                        stack_endpoints: None,
                        reference: None,
                        program_install: None,
                    },
                )
                .with_context(|| {
                    format!(
                        "Failed to generate program '{}' of protocol '{alias}'",
                        program.key
                    )
                })?;
                program_members.insert(
                    program.key.clone(),
                    if module_mode {
                        HoistedRust::Module { directory }
                    } else {
                        HoistedRust::Crate {
                            package: format!("{directory}-program"),
                            path: format!("programs/{directory}"),
                        }
                    },
                );
            }
        }
    }

    let context = RustProtocolContext {
        alias,
        package,
        version,
        package_release_hash,
        protocol,
        spec: &spec,
        stacks: stack_members,
        programs: program_members,
    };
    let module = render_rust_protocol_module(&context)?;
    if module_mode {
        fs::write(options.output.join("mod.rs"), module)?;
    } else {
        fs::create_dir_all(options.output.join("src"))?;
        fs::write(options.output.join("src/lib.rs"), module)?;
        fs::write(
            options.output.join("Cargo.toml"),
            render_cargo_toml(&format!("{alias}-protocol"), &context),
        )?;
    }
    let spec_value = protocol
        .protocol_spec
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Protocol '{alias}' resolution carries no spec"))?;
    fs::write(
        options.output.join(PROTOCOL_SPEC_FILE),
        format!("{}\n", serde_json::to_string_pretty(spec_value)?),
    )?;
    fs::write(
        options.output.join(README_FILE),
        super::protocol::with_unbundled_programs(
            render_rust_readme(&context, module_mode),
            unbundled,
            InstallTarget::Rust,
        ),
    )?;

    // The protocol output's provenance owns every nested file, as in the
    // TypeScript client.
    for directory in ["stacks", "programs"] {
        let parent = options.output.join(directory);
        if !parent.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&parent)? {
            let nested = entry?.path().join(SDK_PROVENANCE_FILE);
            if nested.is_file() {
                fs::remove_file(&nested)?;
            }
        }
    }
    let mut generated = BTreeSet::new();
    collect_payload_files(options.output, options.output, &mut generated)?;
    let manifest = build_sdk_provenance_manifest_from_artifacts(
        generated,
        "",
        &ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::ProtocolSpec,
            hash: protocol.protocol_spec_hash.clone(),
        },
        None,
    )?;
    write_sdk_provenance_manifest_file(options.output, &manifest)
}

pub(super) fn render_cargo_toml(crate_name: &str, context: &RustProtocolContext<'_>) -> String {
    let mut out = format!(
        "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\narete-sdk = {{ package = \"arete-a4-sdk\", version = \"{}\" }}\nserde = {{ version = \"1\", features = [\"derive\"] }}\nserde_json = \"1\"\n",
        arete_interpreter::rust::GENERATED_RUST_SDK_VERSION
    );
    for member in context.stacks.values().chain(context.programs.values()) {
        if let HoistedRust::Crate { package, path } = member {
            let _ = writeln!(out, "{package} = {{ path = \"{path}\" }}");
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

/// `snake_case` for fields, modules and methods: every non-alphanumeric
/// character and every upper-case letter starts a word; keywords get a
/// trailing `_` and a leading digit a `value_` prefix.
pub(super) fn rust_snake(raw: &str) -> String {
    let mut out = String::new();
    let mut separator = false;
    let mut previous_lower = false;
    for character in raw.chars() {
        if !character.is_ascii_alphanumeric() {
            separator = true;
            previous_lower = false;
            continue;
        }
        let upper = character.is_ascii_uppercase();
        if !out.is_empty() && (separator || (upper && previous_lower)) && !out.ends_with('_') {
            out.push('_');
        }
        separator = false;
        previous_lower = character.is_ascii_lowercase() || character.is_ascii_digit();
        out.push(character.to_ascii_lowercase());
    }
    if out.is_empty() {
        out.push_str("value");
    }
    if out.starts_with(|character: char| character.is_ascii_digit()) {
        out.insert_str(0, "value_");
    }
    if rust_ident::is_keyword(&out) {
        out.push('_');
    }
    out
}

fn pascal(raw: &str) -> String {
    rust_ident::identifier(raw, IdentifierCase::Pascal)
}

fn screaming(raw: &str) -> String {
    rust_ident::identifier(raw, IdentifierCase::ScreamingSnake)
}

/// Refuse a spec whose names collide once they are Rust identifiers.
fn check_rust_names(spec: &ProtocolSpecV1) -> Result<()> {
    fn unique<'a>(
        what: &str,
        names: impl IntoIterator<Item = &'a String>,
        convert: fn(&str) -> String,
    ) -> Result<()> {
        let mut seen = BTreeMap::new();
        for name in names {
            if let Some(previous) = seen.insert(convert(name), name) {
                bail!(
                    "Protocol {what} '{previous}' and '{name}' have the same Rust name '{}'",
                    convert(name)
                );
            }
        }
        Ok(())
    }
    unique(
        "instance states",
        spec.machines.instance.states.keys(),
        pascal,
    )?;
    if let Some(machine) = &spec.machines.participant {
        unique("participant states", machine.states.keys(), pascal)?;
    }
    unique("events", spec.events.keys(), pascal)?;
    let client_steps = spec
        .steps
        .iter()
        .filter(|(_, step)| step.authority.is_client_callable())
        .collect::<Vec<_>>();
    unique("steps", client_steps.iter().map(|(name, _)| *name), pascal)?;
    unique(
        "steps",
        client_steps.iter().map(|(name, _)| *name),
        rust_snake,
    )?;
    for (name, step) in &client_steps {
        unique(
            &format!("step '{name}' guards"),
            step.guards.iter().map(|guard| &guard.code),
            pascal,
        )?;
    }
    unique("stacks", spec.stacks.keys(), rust_snake)?;
    unique("programs", spec.programs.keys(), rust_snake)?;
    Ok(())
}

/// A Rust string literal.
fn rust_string(value: &str) -> String {
    format!("{value:?}")
}

fn doc_lines(out: &mut String, indent: &str, text: &str) {
    for line in text.lines() {
        if line.trim().is_empty() {
            let _ = writeln!(out, "{indent}///");
        } else {
            let _ = writeln!(out, "{indent}/// {}", line.trim_end());
        }
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Structs a schema needs, rendered after the struct that declared them.
struct Structs {
    rendered: Vec<String>,
    names: BTreeSet<String>,
}

impl Structs {
    fn new() -> Self {
        Self {
            rendered: Vec::new(),
            names: BTreeSet::new(),
        }
    }

    fn claim(&mut self, name: &str) -> Result<()> {
        if !self.names.insert(name.to_string()) {
            bail!("Protocol types declare '{name}' twice");
        }
        Ok(())
    }
}

/// The Rust type of one field: wide integers as `ProtocolInteger` (decimal
/// strings), public keys and other strings as `String`, nested objects as
/// their own structs.
fn field_type(
    field: &FieldSchemaV1,
    owner: &str,
    property: &str,
    structs: &mut Structs,
) -> Result<String> {
    Ok(match field.field_type {
        FieldType::String if is_integer_format(field.format) => "ProtocolInteger".into(),
        FieldType::String => "String".into(),
        FieldType::Boolean => "bool".into(),
        FieldType::Integer => "i64".into(),
        FieldType::Array => match field.items.as_deref() {
            Some(item) => format!("Vec<{}>", field_type(item, owner, property, structs)?),
            None => "Vec<serde_json::Value>".into(),
        },
        FieldType::Object => {
            let properties = field.properties.clone().unwrap_or_default();
            if properties.is_empty() {
                "serde_json::Map<String, serde_json::Value>".into()
            } else {
                let name = format!("{owner}{}", pascal(property));
                let required = field
                    .required
                    .iter()
                    .flatten()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>();
                render_struct(
                    &name,
                    field.description.as_deref(),
                    &properties,
                    |property| required.contains(property),
                    structs,
                )?;
                name
            }
        }
    })
}

fn field_doc(field: &FieldSchemaV1) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(description) = &field.description {
        parts.push(description.clone());
    }
    if let Some(format) = field.format {
        let format = serde_json::to_value(format)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default();
        parts.push(format!("Format: `{format}`."));
    }
    if let Some(values) = field
        .enum_values
        .as_ref()
        .filter(|values| !values.is_empty())
    {
        parts.push(format!(
            "One of {}.",
            values
                .iter()
                .map(|value| format!("`{value}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Render a struct of `properties` into `structs`, nested structs first.
fn render_struct(
    name: &str,
    doc: Option<&str>,
    properties: &BTreeMap<String, FieldSchemaV1>,
    is_required: impl Fn(&str) -> bool,
    structs: &mut Structs,
) -> Result<()> {
    structs.claim(name)?;
    let mut body = String::new();
    let mut fields = BTreeMap::new();
    for (property, field) in properties {
        let ident = rust_snake(property);
        if let Some(previous) = fields.insert(ident.clone(), property) {
            bail!(
                "'{name}' fields '{previous}' and '{property}' have the same Rust name '{ident}'"
            );
        }
        let ty = field_type(field, name, property, structs)?;
        if let Some(doc) = field_doc(field) {
            doc_lines(&mut body, "    ", &doc);
        }
        let required = is_required(property);
        let mut serde = Vec::new();
        if ident != *property {
            serde.push(format!("rename = {}", rust_string(property)));
        }
        if !required {
            serde.push("default".to_string());
            serde.push("skip_serializing_if = \"Option::is_none\"".to_string());
        }
        if !serde.is_empty() {
            let _ = writeln!(body, "    #[serde({})]", serde.join(", "));
        }
        if required {
            let _ = writeln!(body, "    pub {ident}: {ty},");
        } else {
            let _ = writeln!(body, "    pub {ident}: Option<{ty}>,");
        }
    }
    let mut out = String::new();
    if let Some(doc) = doc {
        doc_lines(&mut out, "", doc);
    }
    out.push_str(
        "#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]\n",
    );
    if body.is_empty() {
        let _ = writeln!(out, "pub struct {name} {{}}");
    } else {
        let _ = write!(out, "pub struct {name} {{\n{body}}}\n");
    }
    structs.rendered.push(out);
    Ok(())
}

fn render_object(
    name: &str,
    doc: &str,
    schema: &ObjectSchemaV1,
    structs: &mut Structs,
) -> Result<()> {
    let required = schema
        .required
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    render_struct(
        name,
        Some(doc),
        &schema.properties,
        |property| required.contains(property),
        structs,
    )
}

fn render_metadata(
    name: &str,
    doc: &str,
    schema: Option<&MetadataSchemaV1>,
    structs: &mut Structs,
) -> Result<()> {
    let properties = schema
        .map(|schema| {
            schema
                .fields
                .iter()
                .map(|(field, declared)| (field.clone(), declared.schema.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    // A document holds only the fields written so far: every field optional.
    render_struct(name, Some(doc), &properties, |_| false, structs)
}

/// A unit-variant enum over wire names, with `ALL`, `as_wire` and
/// `from_wire`.
fn render_wire_enum(
    out: &mut String,
    name: &str,
    doc: &str,
    variants: &[(String, Option<String>)],
    serde: bool,
) {
    doc_lines(out, "", doc);
    if serde {
        out.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]\n");
    } else {
        out.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n");
    }
    let _ = writeln!(out, "pub enum {name} {{");
    for (wire, description) in variants {
        if let Some(description) = description {
            doc_lines(out, "    ", description);
        }
        if serde {
            let _ = writeln!(out, "    #[serde(rename = {})]", rust_string(wire));
        }
        let _ = writeln!(out, "    {},", pascal(wire));
    }
    out.push_str("}\n\n");
    let _ = writeln!(out, "impl {name} {{");
    let _ = writeln!(
        out,
        "    pub const ALL: [Self; {}] = [{}];",
        variants.len(),
        variants
            .iter()
            .map(|(wire, _)| format!("Self::{}", pascal(wire)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    out.push('\n');
    out.push_str("    /// The name on the wire.\n");
    out.push_str("    pub fn as_wire(self) -> &'static str {\n");
    if variants.is_empty() {
        out.push_str("        match self {}\n");
    } else {
        out.push_str("        match self {\n");
        for (wire, _) in variants {
            let _ = writeln!(
                out,
                "            Self::{} => {},",
                pascal(wire),
                rust_string(wire)
            );
        }
        out.push_str("        }\n");
    }
    out.push_str("    }\n\n");
    out.push_str("    /// The variant named `wire`, if this release knows it.\n");
    out.push_str("    pub fn from_wire(wire: &str) -> Option<Self> {\n");
    if variants.is_empty() {
        out.push_str("        let _ = wire;\n        None\n");
    } else {
        out.push_str("        match wire {\n");
        for (wire, _) in variants {
            let _ = writeln!(
                out,
                "            {} => Some(Self::{}),",
                rust_string(wire),
                pascal(wire)
            );
        }
        out.push_str("            _ => None,\n        }\n");
    }
    out.push_str("    }\n}\n\n");
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

fn client_steps(spec: &ProtocolSpecV1) -> Vec<(&String, &StepV1)> {
    spec.steps
        .iter()
        .filter(|(_, step)| step.authority.is_client_callable())
        .collect()
}

fn guard_codes(step: &StepV1) -> Vec<String> {
    let mut seen = BTreeSet::new();
    step.guards
        .iter()
        .filter(|guard| seen.insert(guard.code.clone()))
        .map(|guard| guard.code.clone())
        .collect()
}

fn hoisted_module(
    out: &mut String,
    name: &str,
    doc: &str,
    members: &BTreeMap<String, HoistedRust>,
) {
    let _ = writeln!(out, "/// {doc}");
    if members.is_empty() {
        let _ = writeln!(out, "pub mod {name} {{}}\n");
        return;
    }
    let _ = writeln!(out, "pub mod {name} {{");
    for (key, member) in members {
        let ident = rust_snake(key);
        match member {
            HoistedRust::Crate { package, .. } => {
                let _ = writeln!(out, "    pub use ::{} as {ident};", crate_ident(package));
            }
            HoistedRust::Module { directory } => {
                let _ = writeln!(
                    out,
                    "    #[path = \"{directory}/mod.rs\"]\n    pub mod {ident};"
                );
            }
            HoistedRust::Reexport { path } => {
                let _ = writeln!(out, "    pub use {path} as {ident};");
            }
        }
    }
    out.push_str("}\n\n");
}

pub(super) fn render_rust_protocol_module(context: &RustProtocolContext<'_>) -> Result<String> {
    let spec = context.spec;
    check_rust_names(spec)?;
    let name = pascal(&spec.name);
    let constant = format!("{}_PROTOCOL", screaming(&spec.name));
    let steps = client_steps(spec);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "//! Generated by a4 from protocol package `{}` {} (arete.protocol-spec/v1 `{}`). Do not edit.",
        context.package, context.version, spec.name
    );
    if let Some(description) = &spec.description {
        out.push_str("//!\n");
        for line in description.lines() {
            let _ = writeln!(out, "//! {line}");
        }
    }
    out.push_str("//!\n//! Create a client with [`");
    let _ = write!(out, "{name}PublishableClient");
    out.push_str("`] (publishable key, `public` steps\n//! only) or [`");
    let _ = write!(out, "{name}SecretClient");
    out.push_str("`] (secret key, every client-callable step).\n\n");
    out.push_str("use arete_sdk::protocol;\n");
    out.push_str("pub use arete_sdk::protocol::{\n    verify_webhook, ProtocolClient, ProtocolClientOptions, ProtocolError, ProtocolInteger,\n    ProtocolPrepareOptions, ProtocolPrepareResult, ProtocolPrepared, ProtocolRejection,\n    ProtocolRejectionCode,\n};\n\n");

    hoisted_module(
        &mut out,
        "stacks",
        "The stacks this protocol reads, as `a4 install stack --rust` generates them.",
        &context.stacks,
    );
    hoisted_module(
        &mut out,
        "programs",
        "The programs this protocol uses, as `a4 install program --rust` generates them.",
        &context.programs,
    );

    // Machine states.
    let states = |machine: &arete_hash::protocol::MachineV1| {
        machine
            .states
            .iter()
            .map(|(state, declared)| (state.clone(), declared.description.clone()))
            .collect::<Vec<_>>()
    };
    render_wire_enum(
        &mut out,
        &format!("{name}InstanceState"),
        "States of the instance machine.",
        &states(&spec.machines.instance),
        true,
    );
    let terminal = spec
        .machines
        .instance
        .states
        .iter()
        .filter(|(_, state)| state.terminal)
        .map(|(state, _)| format!("Self::{}", pascal(state)))
        .collect::<Vec<_>>();
    let _ = writeln!(out, "impl {name}InstanceState {{");
    out.push_str("    /// The state of a read instance, if this release knows it.\n");
    out.push_str("    pub fn of(instance: &protocol::ProtocolInstanceSummary) -> Option<Self> {\n        Self::from_wire(&instance.state)\n    }\n\n");
    out.push_str("    /// No step leaves this state.\n    pub fn is_terminal(self) -> bool {\n");
    if terminal.is_empty() {
        out.push_str("        let _ = self;\n        false\n");
    } else {
        let _ = writeln!(out, "        matches!(self, {})", terminal.join(" | "));
    }
    out.push_str("    }\n}\n\n");
    if let Some(machine) = &spec.machines.participant {
        render_wire_enum(
            &mut out,
            &format!("{name}ParticipantState"),
            "States of the participant machine.",
            &states(machine),
            true,
        );
        let _ = writeln!(out, "impl {name}ParticipantState {{");
        out.push_str(
            "    /// The state of a read participant, if it has one this release knows.\n",
        );
        out.push_str("    pub fn of(participant: &protocol::ProtocolParticipantDetail) -> Option<Self> {\n        participant.state.as_deref().and_then(Self::from_wire)\n    }\n}\n\n");
    }

    // Params, args, metadata.
    let mut structs = Structs::new();
    render_object(
        &format!("{name}Params"),
        "Parameters an instance is created with.",
        &spec.params,
        &mut structs,
    )?;
    for (step_name, step) in &steps {
        render_object(
            &format!("{name}{}Args", pascal(step_name)),
            &format!("Arguments of the `{step_name}` step."),
            &step.args,
            &mut structs,
        )?;
    }
    render_metadata(
        &format!("{name}InstanceMetadata"),
        "An instance's metadata document: the fields written so far.",
        spec.metadata.instance.as_ref(),
        &mut structs,
    )?;
    render_metadata(
        &format!("{name}ParticipantMetadata"),
        "A participant's metadata document: the fields written so far.",
        spec.metadata.participant.as_ref(),
        &mut structs,
    )?;
    for rendered in &structs.rendered {
        out.push_str(rendered);
        out.push('\n');
    }
    for metadata in ["Instance", "Participant"] {
        let _ = writeln!(out, "impl {name}{metadata}Metadata {{");
        out.push_str("    /// Read a metadata document.\n");
        out.push_str("    pub fn from_map(\n        metadata: &serde_json::Map<String, serde_json::Value>,\n    ) -> Result<Self, serde_json::Error> {\n        serde_json::from_value(serde_json::Value::Object(metadata.clone()))\n    }\n\n");
        out.push_str("    /// The fields set here, as a merge patch for `update_metadata`.\n");
        out.push_str("    pub fn to_patch(&self) -> serde_json::Map<String, serde_json::Value> {\n        match serde_json::to_value(self) {\n            Ok(serde_json::Value::Object(patch)) => patch,\n            _ => serde_json::Map::new(),\n        }\n    }\n}\n\n");
    }

    // Guard codes.
    for (step_name, step) in &steps {
        let enum_name = format!("{name}{}GuardCode", pascal(step_name));
        let codes = guard_codes(step);
        render_wire_enum(
            &mut out,
            &enum_name,
            &format!("Guard codes the `{step_name}` step can reject with, in evaluation order."),
            &codes
                .iter()
                .map(|code| (code.clone(), None))
                .collect::<Vec<_>>(),
            false,
        );
        let _ = writeln!(out, "impl protocol::ProtocolGuardCode for {enum_name} {{");
        out.push_str("    fn from_wire(wire: &str) -> Option<Self> {\n        Self::from_wire(wire)\n    }\n\n");
        out.push_str("    fn as_wire(&self) -> &str {\n        (*self).as_wire()\n    }\n}\n\n");
    }

    // Events.
    let events = spec
        .events
        .iter()
        .map(|(event, declared)| (event.clone(), declared.description.clone()))
        .collect::<Vec<_>>();
    render_wire_enum(
        &mut out,
        &format!("{name}Event"),
        "Events the protocol emits, on the event stream and to webhooks.",
        &events,
        false,
    );
    let _ = writeln!(out, "impl {name}Event {{");
    out.push_str("    /// The machine the event belongs to.\n");
    out.push_str("    pub fn machine(self) -> protocol::ProtocolMachineKind {\n");
    if spec.events.is_empty() {
        out.push_str("        match self {}\n");
    } else {
        out.push_str("        match self {\n");
        for (event, declared) in &spec.events {
            let _ = writeln!(
                out,
                "            Self::{} => protocol::ProtocolMachineKind::{},",
                pascal(event),
                pascal(machine_name(declared.machine))
            );
        }
        out.push_str("        }\n");
    }
    out.push_str("    }\n\n");
    out.push_str("    /// The event a webhook payload carries, if this release knows it.\n");
    out.push_str("    pub fn of(payload: &protocol::ProtocolEventPayloadV1) -> Option<Self> {\n        Self::from_wire(&payload.event)\n    }\n}\n\n");

    // The definition.
    let deployment = &context.protocol.deployment;
    let _ = writeln!(
        out,
        "/// The `{}` protocol and the deployment this client calls.",
        spec.name
    );
    let _ = writeln!(
        out,
        "pub const {constant}: protocol::ProtocolDefinition = protocol::ProtocolDefinition {{"
    );
    let _ = writeln!(out, "    name: {},", rust_string(&spec.name));
    let _ = writeln!(out, "    version: {},", rust_string(&spec.version));
    let _ = writeln!(out, "    package: {},", rust_string(context.package));
    let _ = writeln!(
        out,
        "    package_release_hash: {},",
        rust_string(context.package_release_hash)
    );
    let _ = writeln!(
        out,
        "    protocol_spec_hash: {},",
        rust_string(&context.protocol.protocol_spec_hash)
    );
    let _ = writeln!(
        out,
        "    protocol_release_hash: {},",
        rust_string(&context.protocol.protocol_release_hash)
    );
    out.push_str("    deployment: protocol::ProtocolDeploymentDescriptor {\n");
    let _ = writeln!(
        out,
        "        deployment_id: {},",
        rust_string(&deployment.deployment_id)
    );
    let _ = writeln!(
        out,
        "        access: protocol::ProtocolDeploymentAccess::{},",
        pascal(deployment.access.as_str())
    );
    let _ = writeln!(
        out,
        "        api_url: {},",
        rust_string(&deployment.api_url)
    );
    let _ = writeln!(
        out,
        "        session_endpoint: {},",
        rust_string(&deployment.session_endpoint)
    );
    out.push_str("    },\n");
    let _ = writeln!(
        out,
        "    instance_visibility: protocol::ProtocolInstanceVisibility::{},",
        match spec.instance_visibility {
            arete_hash::protocol::InstanceVisibility::Shared => "Shared",
            arete_hash::protocol::InstanceVisibility::Account => "Account",
        }
    );
    if steps.is_empty() {
        out.push_str("    steps: &[],\n");
    } else {
        out.push_str("    steps: &[\n");
        for (step_name, step) in &steps {
            out.push_str("        protocol::ProtocolStepDefinition {\n");
            let _ = writeln!(out, "            name: {},", rust_string(step_name));
            let _ = writeln!(
                out,
                "            scope: protocol::ProtocolStepScope::{},",
                pascal(scope_name(step.scope))
            );
            let _ = writeln!(
                out,
                "            access: protocol::ProtocolStepAccess::{},",
                pascal(access_name(step_access(step)))
            );
            let _ = writeln!(
                out,
                "            guards: &[{}],",
                step.guards
                    .iter()
                    .map(|guard| rust_string(&guard.code))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let _ = writeln!(
                out,
                "            args: &[{}],",
                step.args
                    .properties
                    .keys()
                    .map(|arg| rust_string(arg))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            out.push_str("        },\n");
        }
        out.push_str("    ],\n");
    }
    if spec.events.is_empty() {
        out.push_str("    events: &[],\n");
    } else {
        out.push_str("    events: &[\n");
        for (event, declared) in &spec.events {
            let _ = writeln!(
                out,
                "        protocol::ProtocolEventDefinition {{ name: {}, machine: protocol::ProtocolMachineKind::{} }},",
                rust_string(event),
                pascal(machine_name(declared.machine))
            );
        }
        out.push_str("    ],\n");
    }
    for (field, schema) in [
        ("instance_metadata", spec.metadata.instance.as_ref()),
        ("participant_metadata", spec.metadata.participant.as_ref()),
    ] {
        match schema.filter(|schema| !schema.fields.is_empty()) {
            None => {
                let _ = writeln!(out, "    {field}: &[],");
            }
            Some(schema) => {
                let _ = writeln!(out, "    {field}: &[");
                for (name, declared) in &schema.fields {
                    let _ = writeln!(
                        out,
                        "        protocol::ProtocolMetadataFieldDefinition {{ name: {}, writer: protocol::ProtocolMetadataWriter::{}, filterable: {} }},",
                        rust_string(name),
                        match declared.writer {
                            MetadataWriter::Creator => "Creator",
                            MetadataWriter::Participant => "Participant",
                        },
                        declared.filterable
                    );
                }
                out.push_str("    ],\n");
            }
        }
    }
    match &context.protocol.live_stack {
        Some(live) => {
            let _ = writeln!(
                out,
                "    live_stack: Some(protocol::ProtocolLiveStackDescriptor {{ stack: {}, view: {}, key_account: {} }}),",
                rust_string(&live.stack),
                rust_string(&live.view),
                rust_string(&live.key_account)
            );
        }
        None => out.push_str("    live_stack: None,\n"),
    }
    out.push_str("};\n\n");

    // Clients.
    let live = context.protocol.live_stack.as_ref();
    for secret in [false, true] {
        render_client(&mut out, &name, &constant, &steps, live, secret);
    }

    // Webhooks.
    let _ = writeln!(
        out,
        "/// A webhook delivery of this protocol's events; read its event with [`{name}Event::of`]."
    );
    let _ = writeln!(
        out,
        "pub type {name}WebhookPayload = protocol::ProtocolEventPayloadV1;\n"
    );
    out.push_str("/// Verify a signed webhook delivery for this protocol's events. Pass the raw\n/// request body exactly as received.\n");
    let _ = writeln!(
        out,
        "pub fn verify_{}_webhook(\n    secret: impl AsRef<[u8]>,\n    body: impl AsRef<[u8]>,\n    headers: &(impl protocol::WebhookHeaders + ?Sized),\n    options: protocol::VerifyWebhookOptions,\n) -> Result<protocol::VerifiedWebhook<{name}WebhookPayload>, protocol::WebhookVerificationError> {{\n    protocol::verify_webhook(secret, body, headers, options)\n}}",
        rust_snake(&spec.name)
    );
    Ok(out)
}

fn render_client(
    out: &mut String,
    name: &str,
    constant: &str,
    steps: &[(&String, &StepV1)],
    live: Option<&ResolvedProtocolLiveStack>,
    secret: bool,
) {
    let (client, key_class, key_doc) = if secret {
        (
            format!("{name}SecretClient"),
            "Secret",
            "A client for a secret or agent key (`a4_sk_...`, `a4_ak_...`): it can prepare\n/// every client-callable step. Keep it on servers.",
        )
    } else {
        (
            format!("{name}PublishableClient"),
            "Publishable",
            "A client for a publishable key (`a4_pk_...`): it can prepare the `public`\n/// steps only.",
        )
    };
    let _ = writeln!(out, "/// {key_doc}");
    out.push_str("///\n/// Reads, submit, metadata and the event stream come from [`ProtocolClient`]\n/// through `Deref`.\n");
    let _ = writeln!(
        out,
        "#[derive(Debug, Clone)]\npub struct {client} {{\n    client: ProtocolClient,\n}}\n"
    );
    let _ = writeln!(out, "impl {client} {{");
    if secret {
        out.push_str("    /// Authenticate with a secret or agent key.\n");
        out.push_str("    pub fn new(secret_key: impl Into<String>) -> Result<Self, ProtocolError> {\n        Self::with_options(ProtocolClientOptions::default().with_secret_key(secret_key))\n    }\n\n");
        out.push_str("    /// Authenticate with `ARETE_API_KEY`, or else the `a4` CLI login.\n");
        out.push_str("    pub fn from_env() -> Result<Self, ProtocolError> {\n        Self::with_options(ProtocolClientOptions::default())\n    }\n\n");
    } else {
        out.push_str("    /// Authenticate with a publishable key.\n");
        out.push_str("    pub fn new(publishable_key: impl Into<String>) -> Result<Self, ProtocolError> {\n        Self::with_options(ProtocolClientOptions::default().with_publishable_key(publishable_key))\n    }\n\n");
    }
    out.push_str("    /// Full options: tokens, token providers, URL overrides, an HTTP client.\n");
    let _ = writeln!(
        out,
        "    pub fn with_options(mut options: ProtocolClientOptions) -> Result<Self, ProtocolError> {{\n        options.key_class = Some(protocol::ProtocolKeyClass::{key_class});\n        Ok(Self {{\n            client: ProtocolClient::new(&{constant}, options)?,\n        }})\n    }}\n"
    );
    out.push_str("    /// A client on `session`'s auth whose instance handles follow the\n    /// session's stack members; see `Session::protocol_client`.\n");
    let _ = writeln!(
        out,
        "    pub fn from_session(\n        session: &arete_sdk::Session,\n        mut options: ProtocolClientOptions,\n    ) -> Result<Self, ProtocolError> {{\n        options.key_class = Some(protocol::ProtocolKeyClass::{key_class});\n        Ok(Self {{\n            client: session.protocol_client(&{constant}, options)?,\n        }})\n    }}\n"
    );
    out.push_str("    /// The untyped client.\n    pub fn client(&self) -> &ProtocolClient {\n        &self.client\n    }\n\n");
    out.push_str("    /// Load an instance and keep it current from its event stream.\n");
    match live {
        Some(live) => {
            let _ = writeln!(
                out,
                "    ///\n    /// Live fields come from the `{}` view of the `{}` stack,\n    /// keyed by the instance's `{}` account, when the client has live\n    /// views: a client from [`Self::from_session`] with that stack as a\n    /// session member.",
                live.view, live.stack, live.key_account
            );
        }
        None => out.push_str("    ///\n    /// This protocol publishes no live stack view.\n"),
    }
    out.push_str("    pub async fn instance_handle(\n        &self,\n        instance: &str,\n    ) -> Result<protocol::ProtocolInstanceHandle, ProtocolError> {\n        self.client.instance_handle(instance).await\n    }\n");
    for (step_name, step) in steps {
        let access = step_access(step);
        if !secret && access != StepAccess::Public {
            continue;
        }
        let step_pascal = pascal(step_name);
        let args = format!("{name}{step_pascal}Args");
        let guard = format!("{name}{step_pascal}GuardCode");
        let has_args = !step.args.properties.is_empty();
        out.push('\n');
        let mut doc = format!(
            "Prepare `{step_name}` ({} step, {} authority)",
            scope_name(step.scope),
            authority_name(&step.authority)
        );
        if let Some(title) = &step.title {
            let _ = write!(doc, ": {title}");
        }
        doc.push('.');
        doc_lines(out, "    ", &doc);
        if let Some(description) = &step.description {
            out.push_str("    ///\n");
            doc_lines(out, "    ", description);
        }
        out.push_str("    ///\n    /// Resolves with the intent and the transactions for `wallet` to sign, or\n");
        let _ = writeln!(
            out,
            "    /// with a rejection carrying a [`{guard}`] or an access code."
        );
        let _ = write!(
            out,
            "    pub async fn prepare_{}(\n        &self,\n",
            rust_snake(step_name)
        );
        if step.scope != StepScope::Create {
            out.push_str("        instance: &str,\n");
        }
        out.push_str("        wallet: &str,\n");
        if has_args {
            let _ = writeln!(out, "        args: &{args},");
        }
        let _ = writeln!(
            out,
            "        options: ProtocolPrepareOptions,\n    ) -> Result<ProtocolPrepareResult<{guard}>, ProtocolError> {{"
        );
        let instance = if step.scope == StepScope::Create {
            "None"
        } else {
            "Some(instance)"
        };
        let args_expr = if has_args {
            "args".to_string()
        } else {
            format!("&{args}::default()")
        };
        let _ = writeln!(
            out,
            "        self.client\n            .prepare({}, {instance}, wallet, {args_expr}, options)\n            .await\n    }}",
            rust_string(step_name)
        );
    }
    out.push_str("}\n\n");
    let _ = writeln!(
        out,
        "impl std::ops::Deref for {client} {{\n    type Target = ProtocolClient;\n\n    fn deref(&self) -> &ProtocolClient {{\n        &self.client\n    }}\n}}\n"
    );
}

pub(super) fn render_rust_readme(context: &RustProtocolContext<'_>, module_mode: bool) -> String {
    let spec = context.spec;
    let name = pascal(&spec.name);
    let deployment = &context.protocol.deployment;
    let crate_name = crate_ident(&format!("{}-protocol", context.alias));
    let mut out = String::new();
    let _ = writeln!(out, "# `{}` protocol client (Rust)\n", context.alias);
    let _ = writeln!(
        out,
        "Generated from protocol package `{}` {} (spec `{}` {}), installed as `{}`. `a4 install` regenerates this folder, so do not edit it.\n",
        context.package, context.version, spec.name, spec.version, context.alias
    );
    if let Some(description) = &spec.description {
        let _ = writeln!(out, "{description}\n");
    }
    let _ = writeln!(out, "## Use\n");
    if module_mode {
        let _ = writeln!(
            out,
            "Include this folder as a module (`#[path = \"…/mod.rs\"] mod {crate_name};`); your crate needs `arete-sdk` (`arete-a4-sdk`), `serde` (with `derive`) and `serde_json`.\n"
        );
    } else {
        let _ = writeln!(
            out,
            "Add the crate as a path dependency: `{}-protocol = {{ path = \"…\" }}`.\n",
            context.alias
        );
    }
    let first_public = client_steps(spec)
        .into_iter()
        .find(|(_, step)| step_access(step) == StepAccess::Public);
    let _ = writeln!(out, "```rust,ignore\nuse {crate_name}::*;\n");
    let _ = writeln!(
        out,
        "// Servers: a secret key prepares every client-callable step.\nlet client = {name}SecretClient::new(std::env::var(\"ARETE_API_KEY\")?)?;"
    );
    if let Some((step_name, step)) = first_public {
        let instance = if step.scope == StepScope::Create {
            ""
        } else {
            "&instance_id, "
        };
        let args = if step.args.properties.is_empty() {
            String::new()
        } else {
            format!("&{name}{}Args {{ /* … */ }}, ", pascal(step_name))
        };
        let _ = writeln!(
            out,
            "match client\n    .prepare_{}({instance}&wallet, {args}ProtocolPrepareOptions::default())\n    .await?\n{{\n    ProtocolPrepareResult::Prepared(prepared) => {{\n        // Sign `prepared.transactions` with the wallet, then:\n        let state = client.submit(&prepared.intent.execution_id, signed).await?;\n    }}\n    ProtocolPrepareResult::Rejected(rejection) => println!(\"rejected: {{}}\", rejection.rejected()),\n}}",
            rust_snake(step_name)
        );
    }
    out.push_str("```\n\n");
    let _ = writeln!(
        out,
        "`{name}PublishableClient` takes a publishable key and has only the `public` steps. Both clients dereference to `arete_sdk::protocol::ProtocolClient` for reads (`instance`, `list_instances`, `participant`, `allowed_steps`, and `actions` for what a wallet can do across the deployment), `submit`, `intent`, metadata (`update_metadata`, `sign_metadata_proof`) and `events` (the instance event stream). Never submit again after `submitted_unknown`: read the intent until it settles.\n"
    );
    let _ = writeln!(out, "## Deployment\n");
    let _ = writeln!(
        out,
        "Calls the {} deployment `{}`. A protocol session is minted from your own key through the session endpoint; this folder holds no credential.\n",
        deployment.access.as_str(),
        deployment.deployment_id
    );
    let _ = writeln!(out, "## Steps\n");
    let _ = writeln!(
        out,
        "| Step | Method | Scope | Authority | Access | Rejection codes |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|");
    for (step_name, step) in &spec.steps {
        let (method, access) = if step.authority.is_client_callable() {
            (
                format!("`prepare_{}`", rust_snake(step_name)),
                access_name(step_access(step)).to_string(),
            )
        } else {
            ("-".to_string(), "not client-callable".to_string())
        };
        let codes = step
            .guards
            .iter()
            .map(|guard| format!("`{}`", guard.code))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "| `{step_name}` | {method} | {} | {} | {access} | {} |",
            scope_name(step.scope),
            authority_name(&step.authority),
            if codes.is_empty() { "-".into() } else { codes }
        );
    }
    let _ = writeln!(
        out,
        "\nEvery prepare can also be rejected with `step_requires_secret_key`, `step_not_in_session_scope` or `step_not_client_callable`. Instance and participant steps can also be rejected with `instance_terminal` (the instance is in a terminal state) or `participant_terminal` (the wallet's participant is in a terminal state), each with `detail.state`.\n"
    );
    if !spec.events.is_empty() {
        let _ = writeln!(out, "## Events\n");
        for (event, declared) in &spec.events {
            let _ = writeln!(
                out,
                "- `{event}` ({}){}",
                machine_name(declared.machine),
                declared
                    .description
                    .as_deref()
                    .map(|description| format!(": {description}"))
                    .unwrap_or_default()
            );
        }
        let _ = writeln!(
            out,
            "\nVerify webhook deliveries with `verify_{}_webhook`.\n",
            rust_snake(&spec.name)
        );
    }
    if !context.stacks.is_empty() || !context.programs.is_empty() {
        let _ = writeln!(out, "## Stacks and programs\n");
        for key in context.stacks.keys() {
            let _ = writeln!(out, "- `stacks::{}`", rust_snake(key));
        }
        for key in context.programs.keys() {
            let _ = writeln!(out, "- `programs::{}`", rust_snake(key));
        }
    }
    out
}

/// Compare a generated Rust protocol output with its golden, with the SDK
/// version and versioned tree hashes normalized. Every `.rs` file must parse.
#[cfg(test)]
pub(crate) fn compare_rust_protocol_golden(
    name: &str,
    root: &std::path::Path,
    golden_dir: &std::path::Path,
    generated: BTreeMap<String, String>,
) {
    super::stack_name_golden::assert_syntax(name, root, &generated);
    let normalized = super::stack_name_golden::without_release_version(
        generated
            .into_iter()
            .map(|(path, contents)| (format!("rust/{path}"), contents))
            .collect(),
    )
    .into_iter()
    .map(|(path, contents)| (path["rust/".len()..].to_string(), contents))
    .collect();
    super::stack_name_golden::compare_with_golden(name, golden_dir, normalized);
}

#[cfg(test)]
mod tests {
    use super::super::stack_name_golden::{assert_syntax, compare_with_golden};
    use super::*;
    use crate::project::resolver::{ProtocolDeploymentAccess, ResolvedProtocolDeployment};
    use std::path::Path;

    fn vector_spec(name: &str) -> (serde_json::Value, ProtocolSpecV1) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(format!("test-vectors/protocol/{name}.protocol-spec.json"));
        let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let spec = arete_hash::protocol::validate_protocol_spec_value(&value).unwrap();
        (value, spec)
    }

    fn resolved(spec_value: serde_json::Value, spec: &ProtocolSpecV1) -> ResolvedProtocol {
        ResolvedProtocol {
            protocol_release_hash: format!("arete:h1:protocol-release:sha256:{}", "f".repeat(64)),
            protocol_spec_hash: spec.hash().unwrap().to_string(),
            protocol_spec: Some(spec_value),
            deployment: ResolvedProtocolDeployment {
                deployment_id: "7d0c5a52-2f8e-4c1b-9a37-1b6f3c2a9e10".into(),
                access: ProtocolDeploymentAccess::Public,
                api_url: "https://api.example.test/api".into(),
                session_endpoint: "https://api.example.test/ws/sessions".into(),
            },
            live_stack: None,
            local_dev: false,
        }
    }

    /// Token raise publishes its presale row as a live stack view.
    fn presale_live_stack() -> ResolvedProtocolLiveStack {
        ResolvedProtocolLiveStack {
            stack: "meteoraPresale".into(),
            view: "Presale/state".into(),
            key_account: "presale".into(),
        }
    }

    /// The protocol module for each test-vector spec, rendered without
    /// hoisted stacks or programs so it compiles on its own:
    /// `rust/arete-a4-sdk/tests/generated_protocol.rs` builds and exercises
    /// these files against the in-repo SDK. Regenerate with
    /// `A4_UPDATE_GOLDEN=1 cargo test -p a4-cli protocol_rust`.
    #[test]
    fn vector_spec_module_goldens() {
        let mut generated = BTreeMap::new();
        for name in ["token-raise", "escrow-resolution"] {
            let (value, spec) = vector_spec(name);
            let mut protocol = resolved(value, &spec);
            if name == "token-raise" {
                protocol.live_stack = Some(presale_live_stack());
            }
            let context = RustProtocolContext {
                alias: name,
                package: name,
                version: &spec.version,
                package_release_hash: &format!(
                    "arete:registry-package-release:v2:sha256:{}",
                    "9".repeat(64)
                ),
                protocol: &protocol,
                spec: &spec,
                stacks: BTreeMap::new(),
                programs: BTreeMap::new(),
            };
            generated.insert(
                format!("{name}.rs"),
                render_rust_protocol_module(&context).unwrap(),
            );
            generated.insert(
                format!("{name}.README.md"),
                render_rust_readme(&context, false),
            );
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/protocol-rust");
        assert_syntax("protocol-rust", &root, &generated);
        let token_raise = &generated["token-raise.rs"];
        // The publishable client has the public steps only; Squads steps are
        // never client-callable.
        let publishable = token_raise
            .split("pub struct TokenRaisePublishableClient")
            .nth(1)
            .unwrap()
            .split("pub struct TokenRaiseSecretClient")
            .next()
            .unwrap();
        assert!(publishable.contains("pub async fn prepare_contribute("));
        assert!(!publishable.contains("pub async fn prepare_create("));
        assert!(token_raise.contains("pub async fn prepare_create("));
        assert!(!token_raise.contains("prepare_finalize"));
        assert!(token_raise.contains("pub amount: ProtocolInteger,"));
        assert!(token_raise.contains(
            "live_stack: Some(protocol::ProtocolLiveStackDescriptor { stack: \"meteoraPresale\", view: \"Presale/state\", key_account: \"presale\" }),"
        ));
        assert_eq!(token_raise.matches("pub fn from_session(").count(), 2);
        assert_eq!(
            token_raise.matches("pub async fn instance_handle(").count(),
            2
        );
        let escrow = &generated["escrow-resolution.rs"];
        let publishable = escrow
            .split("pub struct EscrowResolutionPublishableClient")
            .nth(1)
            .unwrap()
            .split("pub struct EscrowResolutionSecretClient")
            .next()
            .unwrap();
        assert!(
            !publishable.contains("pub async fn prepare_"),
            "{publishable}"
        );
        compare_with_golden("protocol-rust", &root, generated);
    }

    #[test]
    fn rust_names() {
        assert_eq!(rust_snake("hardCap"), "hard_cap");
        assert_eq!(rust_snake("quoteMint"), "quote_mint");
        assert_eq!(rust_snake("type"), "type_");
        assert_eq!(
            rust_snake("contribution.confirmed"),
            "contribution_confirmed"
        );
        assert_eq!(rust_snake("v2Pool"), "v2_pool");
        assert_eq!(rust_snake("9lives"), "value_9lives");
        assert_eq!(rust_snake("HTTPServer"), "httpserver");
        assert_eq!(pascal("awaiting_approval"), "AwaitingApproval");
        assert_eq!(pascal("raise.succeeded"), "RaiseSucceeded");
        assert_eq!(pascal("self"), "Self_");
        assert_eq!(screaming("tokenRaise"), "TOKEN_RAISE");
    }

    #[test]
    fn colliding_names_are_refused() {
        let (_, mut spec) = vector_spec("token-raise");
        let event = spec.events["raise.succeeded"].clone();
        spec.events.insert("raiseSucceeded".into(), event);
        let error = check_rust_names(&spec).unwrap_err().to_string();
        assert!(error.contains("same Rust name 'RaiseSucceeded'"), "{error}");
    }
}
