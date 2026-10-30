//! The Python client of a protocol package.
//!
//! `a4 install protocol <ref> --python` writes a Python package (or, in
//! module mode, a module directory) for the protocol:
//!
//! - `__init__.py`: the protocol definition (`<NAME>_PROTOCOL`) with its
//!   generated types: params, per-step args `TypedDict`s and guard-code
//!   `Literal`s, machine states, metadata, event names and the webhook
//!   payload; plus a publishable client (`<Name>Client`, `public` steps only)
//!   and a secret client (`<Name>SecretClient`, every client-callable step);
//! - `stacks/<stack>/`: each stack the protocol hoists, generated as
//!   `a4 install stack --python` generates it, in module mode;
//! - `programs/<program>/`: each program the protocol declares that none of
//!   its stacks bundles, generated as `a4 install program --python` does;
//! - `protocol-spec.json`, `README.md`, `sdk-manifest.json` and
//!   `sdk-provenance.json` (plus `pyproject.toml` in package mode).
//!
//! The runtime (`arete.protocol.ProtocolClient`) lives in `arete-sdk`.
//! Nothing here carries a credential.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use arete_hash::protocol::{
    FieldSchemaV1, FieldType, MetadataSchemaV1, MetadataWriter, ProtocolSpecV1, StepAccess,
    StepScope, StepV1, StringFormat,
};
use arete_interpreter::identifiers::python as py_ident;
use arete_interpreter::identifiers::{typescript as ts_ident, IdentifierCase};
use arete_interpreter::python::python_module_name;

use super::{
    build_sdk_provenance_manifest_from_artifacts, ensure_file_stem,
    generate_project_registry_dependency, program_spec_artifact_from_registry,
    write_sdk_provenance_manifest_file, ExtensionsInputKind, ProjectGenerationOptions,
    ResolvedExtensionsInputPin, SDK_MANIFEST_FILE, SDK_PROVENANCE_FILE,
};
use crate::config::to_kebab_case;
use crate::project::manifest::InstallTarget;
use crate::project::protocol::{
    hoist_protocol_programs, protocol_spec, unbundled_protocol_programs, HoistedProgramSource,
};
use crate::project::resolver::{ResolvedProtocol, ResolvedRegistryDependency};

const PROTOCOL_SPEC_FILE: &str = "protocol-spec.json";
const README_FILE: &str = "README.md";

pub(crate) fn generate_project_protocol_python(
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
    if alias != options.alias {
        bail!(
            "Resolved protocol alias '{alias}' does not match planned alias '{}'",
            options.alias
        );
    }
    ensure_file_stem(alias)?;
    let spec = protocol_spec(protocol)?;
    let hoisted = hoist_protocol_programs(
        package,
        protocol,
        &spec,
        stacks,
        programs,
        unpinned_programs,
    )?;
    let unbundled = unbundled_protocol_programs(protocol, unpinned_programs);
    let package_name = python_package_name(alias);
    let module_dir = if options.python_module {
        options.output.to_path_buf()
    } else {
        options.output.join(&package_name)
    };
    fs::create_dir_all(&module_dir)
        .with_context(|| format!("Failed to create protocol output {}", module_dir.display()))?;

    // Each hoisted stack, as a stack install generates it, as a subpackage.
    let mut stack_modules = BTreeMap::new();
    let mut ordered_stacks = stacks.iter().collect::<Vec<_>>();
    ordered_stacks.sort_by(|left, right| left.alias().cmp(right.alias()));
    for stack in ordered_stacks {
        let key = stack.alias().to_string();
        let directory = to_kebab_case(&key);
        ensure_file_stem(&directory)?;
        let module = py_ident::snake_case(&key);
        let output = module_dir.join("stacks").join(&module);
        let mut nested = stack.clone();
        if let ResolvedRegistryDependency::Stack { alias, .. } = &mut nested {
            alias.clone_from(&directory);
        }
        generate_project_registry_dependency(
            &nested,
            ProjectGenerationOptions {
                alias: &directory,
                target: InstallTarget::Python,
                output: &output,
                typescript_package: options.typescript_package,
                rust_module: false,
                python_module: true,
                stack_endpoints: None,
                reference: None,
                program_install: None,
                extensions: None,
            },
        )
        .with_context(|| format!("Failed to generate stack '{key}' of protocol '{alias}'"))?;
        let init = fs::read_to_string(output.join("__init__.py"))
            .with_context(|| format!("Failed to read the generated stack '{key}'"))?;
        let constant = stack_constant(&init).ok_or_else(|| {
            anyhow::anyhow!(
                "Stack '{key}' of protocol '{alias}' has no single StackDef; a multi-live stack cannot be hoisted into a Python protocol client yet"
            )
        })?;
        stack_modules.insert(
            key,
            PythonImport {
                module: module.clone(),
                binding: format!("_{module}_stack"),
                constant,
            },
        );
    }
    if !stack_modules.is_empty() {
        fs::write(module_dir.join("stacks").join("__init__.py"), "")?;
    }

    // Each program no stack bundles, as a program install generates it; the
    // rest are the bundling stack's programs.
    let mut program_imports = Vec::new();
    let mut program_members = Vec::new();
    for program in &hoisted {
        match program.source {
            HoistedProgramSource::Stack {
                stack,
                program: install,
            } => {
                let artifact = program_spec_artifact_from_registry(install)?;
                let program_key = python_module_name(&artifact.payload.idl_snapshot.snapshot.name);
                let module = stack_modules.get(stack).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Hoisted program '{}' names unknown stack '{stack}'",
                        program.key
                    )
                })?;
                program_members.push((
                    program.key.clone(),
                    format!(
                        "{}.{}.programs[{}]",
                        module.binding,
                        module.constant,
                        py_str(&program_key)
                    ),
                ));
            }
            HoistedProgramSource::Direct(install) => {
                let directory = to_kebab_case(&program.key);
                ensure_file_stem(&directory)?;
                let module = py_ident::snake_case(&program.key);
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
                        target: InstallTarget::Python,
                        output: &module_dir.join("programs").join(&module),
                        typescript_package: options.typescript_package,
                        rust_module: false,
                        python_module: true,
                        stack_endpoints: None,
                        reference: None,
                        program_install: None,
                        extensions: None,
                    },
                )
                .with_context(|| {
                    format!(
                        "Failed to generate program '{}' of protocol '{alias}'",
                        program.key
                    )
                })?;
                let artifact = program_spec_artifact_from_registry(install)?;
                let program_key = python_module_name(&artifact.payload.idl_snapshot.snapshot.name);
                let binding = format!("_{module}_program");
                program_members.push((
                    program.key.clone(),
                    format!("{binding}.PROGRAMS[{}]", py_str(&program_key)),
                ));
                program_imports.push((module, binding));
            }
        }
    }
    if !program_imports.is_empty() {
        fs::write(module_dir.join("programs").join("__init__.py"), "")?;
    }

    let context = PythonProtocolContext {
        package,
        version,
        package_release_hash,
        protocol,
        spec: &spec,
        stacks: &stack_modules,
        program_imports: &program_imports,
        program_members: &program_members,
    };
    fs::write(
        module_dir.join("__init__.py"),
        render_python_protocol_module(&context)?,
    )?;
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
            render_python_protocol_readme(alias, &package_name, options.python_module, &context),
            unbundled,
            InstallTarget::Python,
        ),
    )?;
    if !options.python_module {
        fs::write(
            options.output.join("pyproject.toml"),
            render_pyproject(alias, &package_name),
        )?;
    }

    // The protocol output's provenance owns every nested file; nested SDKs
    // keep their `sdk-manifest.json` but not a provenance of their own.
    for directory in ["stacks", "programs"] {
        let parent = module_dir.join(directory);
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

/// `token-raise` -> `token_raise_protocol`.
fn python_package_name(alias: &str) -> String {
    python_module_name(&format!("{alias}-protocol"))
}

/// The `<NAME>_STACK` a generated stack module defines.
fn stack_constant(init: &str) -> Option<String> {
    let mut found = init.lines().filter_map(|line| {
        line.strip_suffix(": StackDef = StackDef(")
            .filter(|name| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '_')
            })
            .map(str::to_string)
    });
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// Every file under `output`, except its own root provenance and manifest.
fn collect_payload_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeSet<String>,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("Failed to inspect generated output {}", directory.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "__pycache__") {
                continue;
            }
            collect_payload_files(root, &path, files)?;
            continue;
        }
        if path == root.join(SDK_PROVENANCE_FILE) || path == root.join(SDK_MANIFEST_FILE) {
            continue;
        }
        files.insert(
            path.strip_prefix(root)
                .context("Generated output escaped its staging root")?
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    Ok(())
}

struct PythonImport {
    /// Subpackage name under `stacks/`.
    module: String,
    /// The local name the module is imported as.
    binding: String,
    /// Its `StackDef` constant.
    constant: String,
}

pub(super) struct PythonProtocolContext<'a> {
    package: &'a str,
    version: &'a str,
    package_release_hash: &'a str,
    protocol: &'a ResolvedProtocol,
    spec: &'a ProtocolSpecV1,
    stacks: &'a BTreeMap<String, PythonImport>,
    /// `(subpackage under programs/, local binding)`.
    program_imports: &'a [(String, String)],
    /// Spec program key -> the expression that names its `ProgramDef`.
    program_members: &'a [(String, String)],
}

impl PythonProtocolContext<'_> {
    fn pascal(&self) -> String {
        ts_ident::identifier_stem(&self.spec.name, IdentifierCase::Pascal)
    }

    fn screaming(&self) -> String {
        ts_ident::identifier_stem(&self.spec.name, IdentifierCase::ScreamingSnake)
    }

    fn client_steps(&self) -> impl Iterator<Item = (&String, &StepV1)> {
        self.spec
            .steps
            .iter()
            .filter(|(_, step)| step.authority.is_client_callable())
    }
}

struct StepTypes<'a> {
    name: &'a str,
    step: &'a StepV1,
    method: String,
    args: String,
    guard: String,
    result: String,
    integers: Vec<String>,
}

pub(super) fn render_python_protocol_module(context: &PythonProtocolContext<'_>) -> Result<String> {
    let spec = context.spec;
    let pascal = context.pascal();
    let screaming = context.screaming();
    let mut out = String::new();
    writeln!(
        out,
        "\"\"\"Generated by a4 from protocol package `{}` {} (arete.protocol-spec/v1 `{}`). Do not edit.",
        py_ident::docstring_text(context.package),
        py_ident::docstring_text(context.version),
        py_ident::docstring_text(&spec.name)
    )?;
    if let Some(description) = &spec.description {
        writeln!(out, "\n{}", py_ident::docstring_text(description))?;
    }
    out.push_str("\"\"\"\n\nfrom __future__ import annotations\n\n");
    out.push_str(
        "from typing import Any, Dict, List, Literal, Mapping, NoReturn, Optional, Sequence, TypedDict, Union\n\n",
    );
    out.push_str("import httpx\n\n");
    out.push_str("from arete.auth import AuthConfig\nfrom arete.errors import AreteError\n");
    out.push_str(
        "from arete.protocol import (\n    ProtocolAccessRejectionCode,\n    ProtocolClient,\n    ProtocolDefinition,\n    ProtocolDeployment,\n    ProtocolInstanceHandle,\n    ProtocolInteger,\n    ProtocolLiveStack,\n    ProtocolMetadataFieldDefinition,\n    ProtocolPrepared,\n    ProtocolRejection,\n    ProtocolStateRejectionCode,\n    ProtocolStepDefinition,\n    ProtocolTransactionFormat,\n    VerifiedWebhook,\n    decimal_args,\n    verify_webhook,\n)\n",
    );
    for module in context.stacks.values() {
        writeln!(
            out,
            "from .stacks import {} as {}",
            module.module, module.binding
        )?;
    }
    for (module, binding) in context.program_imports {
        writeln!(out, "from .programs import {module} as {binding}")?;
    }
    out.push('\n');

    // Machine states and events.
    writeln!(
        out,
        "{pascal}InstanceState = {}",
        literal(spec.machines.instance.states.keys())
    )?;
    writeln!(
        out,
        "{pascal}ParticipantState = {}",
        spec.machines
            .participant
            .as_ref()
            .map(|machine| literal(machine.states.keys()))
            .unwrap_or_else(|| "NoReturn".into())
    )?;
    writeln!(out, "{pascal}Event = {}", literal(spec.events.keys()))?;
    out.push_str("\n\n");

    // Params.
    let mut definitions = Vec::new();
    typed_dict(
        &mut definitions,
        &format!("{pascal}Params"),
        &spec.params.properties,
        &spec.params.required,
        false,
    );

    // Client-callable steps: args, guard codes and results.
    let mut steps = Vec::new();
    let mut methods = BTreeSet::new();
    for (name, step) in context.client_steps() {
        let step_pascal = ts_ident::identifier_stem(name, IdentifierCase::Pascal);
        let method = py_ident::snake_case(name);
        if !methods.insert(method.clone()) {
            bail!("Protocol steps collide as Python method '{method}'");
        }
        let args = format!("{pascal}{step_pascal}Args");
        typed_dict(
            &mut definitions,
            &args,
            &step.args.properties,
            &step.args.required,
            false,
        );
        let guard = format!("{pascal}{step_pascal}GuardCode");
        let codes = step
            .guards
            .iter()
            .map(|guard| guard.code.as_str())
            .collect::<BTreeSet<_>>();
        definitions.push(format!("{guard} = {}\n", literal(codes)));
        let result = format!("{pascal}{step_pascal}Result");
        definitions.push(format!(
            "{result} = Union[ProtocolPrepared, ProtocolRejection[Union[{guard}, ProtocolAccessRejectionCode, ProtocolStateRejectionCode]]]\n"
        ));
        let integers = step
            .args
            .properties
            .iter()
            .filter(|(_, field)| is_integer_field(field))
            .map(|(name, _)| name.clone())
            .collect();
        steps.push(StepTypes {
            name,
            step,
            method,
            args,
            guard,
            result,
            integers,
        });
    }

    // Metadata documents hold only the fields written so far.
    for (machine, schema) in [
        ("Instance", spec.metadata.instance.as_ref()),
        ("Participant", spec.metadata.participant.as_ref()),
    ] {
        metadata_typed_dict(
            &mut definitions,
            &format!("{pascal}{machine}Metadata"),
            schema,
        );
    }
    for definition in &definitions {
        out.push_str(definition.trim_end_matches('\n'));
        out.push_str("\n\n\n");
    }

    // Webhook payload.
    writeln!(out, "class {pascal}WebhookPayload(TypedDict):")?;
    writeln!(
        out,
        "    \"\"\"The JSON body of a `{}` webhook delivery.\"\"\"\n",
        py_ident::docstring_text(&spec.name)
    )?;
    out.push_str("    id: str\n");
    writeln!(out, "    event: {pascal}Event")?;
    out.push_str("    deploymentId: str\n    instanceId: str\n    participantId: Optional[str]\n");
    writeln!(
        out,
        "    state: Union[{pascal}InstanceState, {pascal}ParticipantState]"
    )?;
    out.push_str("    occurredAt: str\n\n\n");

    // Hoisted members.
    if context.stacks.is_empty() {
        out.push_str("STACKS: Dict[str, Any] = {}\n");
    } else {
        out.push_str("STACKS: Dict[str, Any] = {\n");
        for (key, module) in context.stacks {
            writeln!(
                out,
                "    {}: {}.{},",
                py_str(key),
                module.binding,
                module.constant
            )?;
        }
        out.push_str("}\n");
    }
    if context.program_members.is_empty() {
        out.push_str("PROGRAMS: Dict[str, Any] = {}\n\n");
    } else {
        out.push_str("PROGRAMS: Dict[str, Any] = {\n");
        for (key, value) in context.program_members {
            writeln!(out, "    {}: {value},", py_str(key))?;
        }
        out.push_str("}\n\n");
    }

    // The definition.
    let deployment = &context.protocol.deployment;
    writeln!(
        out,
        "{screaming}_PROTOCOL: ProtocolDefinition = ProtocolDefinition("
    )?;
    writeln!(out, "    name={},", py_str(&spec.name))?;
    writeln!(out, "    version={},", py_str(&spec.version))?;
    writeln!(out, "    package={},", py_str(context.package))?;
    writeln!(
        out,
        "    package_release_hash={},",
        py_str(context.package_release_hash)
    )?;
    writeln!(
        out,
        "    protocol_spec_hash={},",
        py_str(&context.protocol.protocol_spec_hash)
    )?;
    writeln!(
        out,
        "    protocol_release_hash={},",
        py_str(&context.protocol.protocol_release_hash)
    )?;
    writeln!(
        out,
        "    deployment=ProtocolDeployment(\n        deployment_id={},\n        access={},\n        api_url={},\n        session_endpoint={},\n    ),",
        py_str(&deployment.deployment_id),
        py_str(deployment.access.as_str()),
        py_str(&deployment.api_url),
        py_str(&deployment.session_endpoint),
    )?;
    writeln!(
        out,
        "    instance_visibility={},",
        py_str(match spec.instance_visibility {
            arete_hash::protocol::InstanceVisibility::Shared => "shared",
            arete_hash::protocol::InstanceVisibility::Account => "account",
        })
    )?;
    if steps.is_empty() {
        out.push_str("    steps={},\n");
    } else {
        out.push_str("    steps={\n");
        for step in &steps {
            writeln!(
                out,
                "        {}: ProtocolStepDefinition(scope={}, access={}, guards={}, args={}),",
                py_str(step.name),
                py_str(scope_name(step.step.scope)),
                py_str(access_name(step_access(step.step))),
                py_tuple(step.step.guards.iter().map(|guard| guard.code.as_str())),
                py_tuple(step.step.args.properties.keys().map(String::as_str)),
            )?;
        }
        out.push_str("    },\n");
    }
    if spec.events.is_empty() {
        out.push_str("    events={},\n");
    } else {
        out.push_str("    events={\n");
        for (name, event) in &spec.events {
            writeln!(
                out,
                "        {}: {},",
                py_str(name),
                py_str(event.machine.as_str())
            )?;
        }
        out.push_str("    },\n");
    }
    for (keyword, schema) in [
        ("instance_metadata", spec.metadata.instance.as_ref()),
        ("participant_metadata", spec.metadata.participant.as_ref()),
    ] {
        let Some(schema) = schema.filter(|schema| !schema.fields.is_empty()) else {
            continue;
        };
        writeln!(out, "    {keyword}={{")?;
        for (name, field) in &schema.fields {
            writeln!(
                out,
                "        {}: ProtocolMetadataFieldDefinition(writer={}, filterable={}),",
                py_str(name),
                py_str(match field.writer {
                    MetadataWriter::Creator => "creator",
                    MetadataWriter::Participant => "participant",
                }),
                if field.filterable { "True" } else { "False" }
            )?;
        }
        out.push_str("    },\n");
    }
    if let Some(live) = &context.protocol.live_stack {
        writeln!(
            out,
            "    live_stack=ProtocolLiveStack(stack={}, view={}, key_account={}),",
            py_str(&live.stack),
            py_str(&live.view),
            py_str(&live.key_account)
        )?;
    }
    out.push_str("    stacks=STACKS,\n    programs=PROGRAMS,\n)\n\n\n");

    // Step clients: `public` steps on the publishable client, every
    // client-callable step on the secret client.
    for (class, access, base) in [
        (format!("{pascal}PublicSteps"), StepAccess::Public, None),
        (
            format!("{pascal}SecretSteps"),
            StepAccess::Secret,
            Some(format!("{pascal}PublicSteps")),
        ),
    ] {
        let members = steps
            .iter()
            .filter(|step| step_access(step.step) == access)
            .collect::<Vec<_>>();
        writeln!(
            out,
            "class {class}{}:",
            base.as_deref()
                .map(|base| format!("({base})"))
                .unwrap_or_default()
        )?;
        if access == StepAccess::Public {
            out.push_str("    \"\"\"Steps a publishable key may prepare.\"\"\"\n\n");
            out.push_str("    def __init__(self, client: ProtocolClient) -> None:\n        self._client = client\n");
        } else {
            out.push_str(
                "    \"\"\"Every client-callable step; `secret` steps need a secret key.\"\"\"\n",
            );
            if members.is_empty() {
                out.push_str("\n    pass\n");
            }
        }
        for step in members {
            render_step_method(&mut out, step)?;
        }
        out.push_str("\n\n");
    }

    // Clients.
    let client_args = "        *,\n        auth: Optional[AuthConfig] = None,\n        http_client: Optional[httpx.AsyncClient] = None,\n        api_url: Optional[str] = None,\n        session_endpoint: Optional[str] = None,\n        deployment_id: Optional[str] = None,\n        stacks: Optional[Mapping[str, Any]] = None,\n        programs: Optional[Mapping[str, Any]] = None,\n";
    let forwarded = "            auth=auth,\n            http_client=http_client,\n            api_url=api_url,\n            session_endpoint=session_endpoint,\n            deployment_id=deployment_id,\n            stacks=stacks,\n            programs=programs,\n";
    let live_doc = match &context.protocol.live_stack {
        Some(live) => format!(
            "\n\n        Live fields come from the `{}` view of the `{}` stack, keyed\n        by the instance's `{}` account, when the client has that stack\n        connected (`from_session`, or `stacks=`).",
            py_ident::docstring_text(&live.view),
            py_ident::docstring_text(&live.stack),
            py_ident::docstring_text(&live.key_account)
        ),
        None => "\n\n        This protocol publishes no live stack view.".to_string(),
    };
    let instance_handle = format!(
        "    def instance_handle(\n        self, instance_id: str, *, wallet: Optional[str] = None\n    ) -> ProtocolInstanceHandle:\n        \"\"\"A live instance, kept current from its event stream; use it with\n        `async with` or `await` it.{live_doc}\n        \"\"\"\n        return super().instance_handle(instance_id, wallet=wallet)\n"
    );
    writeln!(out, "class {pascal}Client(ProtocolClient):")?;
    writeln!(
        out,
        "    \"\"\"`{}` with a publishable key (`AuthConfig(publishable_key=...)`): only\n    `public` steps are on `steps`.\n\n    `prepare_steps` limits which steps the session may prepare (default:\n    every `public` step).\n    \"\"\"\n",
        py_ident::docstring_text(&spec.name)
    )?;
    writeln!(out, "    steps: {pascal}PublicSteps\n")?;
    writeln!(
        out,
        "    def __init__(\n        self,\n{client_args}        key_class: Optional[str] = None,\n        prepare_steps: Optional[Sequence[str]] = None,\n    ) -> None:"
    )?;
    writeln!(
        out,
        "        super().__init__(\n            {screaming}_PROTOCOL,\n{forwarded}            key_class=key_class,\n            steps=prepare_steps,\n        )"
    )?;
    writeln!(out, "        self.steps = {pascal}PublicSteps(self)\n")?;
    writeln!(
        out,
        "    @classmethod\n    def from_session(\n        cls, session: Any, *, prepare_steps: Optional[Sequence[str]] = None\n    ) -> \"{pascal}Client\":\n        \"\"\"A client on `session`'s auth and HTTP client whose instance handles\n        follow the session's connected stacks.\"\"\"\n        return cls(**session.protocol_client_options({screaming}_PROTOCOL), prepare_steps=prepare_steps)\n"
    )?;
    writeln!(out, "{instance_handle}\n")?;

    writeln!(out, "class {pascal}SecretClient(ProtocolClient):")?;
    writeln!(
        out,
        "    \"\"\"`{}` with a secret key (`AuthConfig(secret_key=...)`, or `ARETE_API_KEY`):\n    every client-callable step is on `steps`. Raises when the configured\n    credential is a publishable key.\n    \"\"\"\n",
        py_ident::docstring_text(&spec.name)
    )?;
    writeln!(out, "    steps: {pascal}SecretSteps\n")?;
    writeln!(
        out,
        "    def __init__(\n        self,\n{client_args}    ) -> None:"
    )?;
    writeln!(
        out,
        "        super().__init__(\n            {screaming}_PROTOCOL,\n{forwarded}            key_class=\"secret\",\n        )"
    )?;
    writeln!(
        out,
        "        if self.key_class != \"secret\":\n            raise AreteError(\n                \"{pascal}SecretClient requires a secret key; use {pascal}Client with a publishable key\",\n                \"INVALID_CONFIG\",\n            )"
    )?;
    writeln!(out, "        self.steps = {pascal}SecretSteps(self)\n")?;
    writeln!(
        out,
        "    @classmethod\n    def from_session(cls, session: Any) -> \"{pascal}SecretClient\":\n        \"\"\"A client on `session`'s auth and HTTP client whose instance handles\n        follow the session's connected stacks.\"\"\"\n        options = session.protocol_client_options({screaming}_PROTOCOL)\n        options.pop(\"key_class\", None)\n        return cls(**options)\n"
    )?;
    writeln!(out, "{instance_handle}\n")?;

    // Webhooks.
    let snake = py_ident::snake_case(&spec.name);
    writeln!(
        out,
        "def verify_{snake}_webhook(\n    secret: Union[str, bytes],\n    body: Union[str, bytes],\n    headers: Any,\n    *,\n    now: Optional[int] = None,\n    tolerance_seconds: int = 300,\n) -> VerifiedWebhook[{pascal}WebhookPayload]:"
    )?;
    out.push_str("    \"\"\"Verify a signed webhook delivery for this protocol's events.\"\"\"\n");
    out.push_str(
        "    return verify_webhook(secret, body, headers, now=now, tolerance_seconds=tolerance_seconds)\n\n\n",
    );

    // Exports.
    let mut exports = vec![
        format!("{screaming}_PROTOCOL"),
        format!("{pascal}Client"),
        format!("{pascal}SecretClient"),
        format!("{pascal}PublicSteps"),
        format!("{pascal}SecretSteps"),
        format!("{pascal}InstanceState"),
        format!("{pascal}ParticipantState"),
        format!("{pascal}Event"),
        format!("{pascal}Params"),
        format!("{pascal}InstanceMetadata"),
        format!("{pascal}ParticipantMetadata"),
        format!("{pascal}WebhookPayload"),
        "STACKS".into(),
        "PROGRAMS".into(),
        format!("verify_{snake}_webhook"),
    ];
    for step in &steps {
        exports.extend([step.args.clone(), step.guard.clone(), step.result.clone()]);
    }
    out.push_str("__all__ = [\n");
    for export in exports {
        writeln!(out, "    {},", py_str(&export))?;
    }
    out.push_str("]\n");
    Ok(out)
}

fn render_step_method(out: &mut String, step: &StepTypes<'_>) -> Result<()> {
    let args_optional = step.step.args.required.is_empty();
    writeln!(out, "\n    async def {}(", step.method)?;
    out.push_str("        self,\n        *,\n");
    if step.step.scope != StepScope::Create {
        out.push_str("        instance: str,\n");
    }
    out.push_str("        wallet: str,\n");
    if args_optional {
        writeln!(out, "        args: Optional[{}] = None,", step.args)?;
    } else {
        writeln!(out, "        args: {},", step.args)?;
    }
    out.push_str(
        "        transaction_format: ProtocolTransactionFormat = \"v1\",\n        idempotency_key: Optional[str] = None,\n        client_context: Optional[Mapping[str, Any]] = None,\n",
    );
    writeln!(out, "    ) -> {}:", step.result)?;
    let title = step
        .step
        .title
        .as_deref()
        .or(step.step.description.as_deref())
        .map(|text| format!("{}. ", text.trim_end_matches('.')))
        .unwrap_or_default();
    writeln!(
        out,
        "        \"\"\"{}Prepare `{}` ({} step, {} access).\"\"\"",
        py_ident::docstring_text(&title.replace('\n', " ")),
        py_ident::docstring_text(step.name),
        scope_name(step.step.scope),
        access_name(step_access(step.step))
    )?;
    let args_expr = if step.integers.is_empty() {
        "args".to_string()
    } else {
        format!(
            "decimal_args(args, {})",
            py_tuple(step.integers.iter().map(String::as_str))
        )
    };
    writeln!(
        out,
        "        return await self._client.prepare(\n            {},\n            wallet=wallet,{}\n            args={args_expr},\n            transaction_format=transaction_format,\n            idempotency_key=idempotency_key,\n            client_context=client_context,\n        )",
        py_str(step.name),
        if step.step.scope == StepScope::Create {
            ""
        } else {
            "\n            instance=instance,"
        }
    )?;
    Ok(())
}

fn render_python_protocol_readme(
    alias: &str,
    package_name: &str,
    module_mode: bool,
    context: &PythonProtocolContext<'_>,
) -> String {
    let spec = context.spec;
    let pascal = context.pascal();
    let screaming = context.screaming();
    let deployment = &context.protocol.deployment;
    let mut out = String::new();
    let _ = writeln!(out, "# `{alias}` protocol client (Python)\n");
    let _ = writeln!(
        out,
        "Generated from protocol package `{}` {} (spec `{}` {}), installed as `{alias}`. `a4 install` regenerates this folder, so do not edit it.\n",
        context.package, context.version, spec.name, spec.version
    );
    if let Some(description) = &spec.description {
        let _ = writeln!(out, "{description}\n");
    }
    let import = if module_mode {
        "<this module>".to_string()
    } else {
        package_name.to_string()
    };
    let _ = writeln!(out, "## Import\n");
    let _ = writeln!(
        out,
        "```python\nfrom arete import AuthConfig\nfrom {import} import {pascal}Client, {pascal}SecretClient, {screaming}_PROTOCOL\n\n# A publishable key prepares `public` steps only.\nasync with {pascal}Client(auth=AuthConfig(publishable_key=key)) as client:\n    result = await client.steps.<step>(wallet=wallet, ...)\n    if result.ok:\n        state = await client.submit(result.intent, signed_transactions)\n\n# A secret key (servers) prepares every client-callable step.\nclient = {pascal}SecretClient(auth=AuthConfig(secret_key=key))\n```\n"
    );
    let _ = writeln!(
        out,
        "`{screaming}_PROTOCOL.stacks` and `.programs` are the protocol's stacks and programs, ready for `arete.create_session(stacks=..., programs=...)`.\n"
    );
    let _ = writeln!(out, "## Deployment\n");
    if deployment.deployment_id.is_empty() {
        let _ = writeln!(
            out,
            "Not deployed to production yet: run the app in dev mode (`a4 dev run -- <command>`), which calls your dev environment's deployment, until `a4 protocol deploy` deploys it. This folder holds no credential.\n"
        );
    } else {
        let _ = writeln!(
            out,
            "Calls the {} deployment `{}`. A protocol session is minted from your own key through the session endpoint; this folder holds no credential.\n",
            deployment.access.as_str(),
            deployment.deployment_id
        );
    }
    let _ = writeln!(out, "## Steps\n");
    let _ = writeln!(out, "| Step | Method | Scope | Access | Rejection codes |");
    let _ = writeln!(out, "|---|---|---|---|---|");
    for (name, step) in &spec.steps {
        let (method, access) = if step.authority.is_client_callable() {
            (
                format!("`steps.{}`", py_ident::snake_case(name)),
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
            "| `{name}` | {method} | {} | {access} | {} |",
            scope_name(step.scope),
            if codes.is_empty() { "-".into() } else { codes }
        );
    }
    let _ = writeln!(
        out,
        "\nEvery prepare can also be rejected with `step_requires_secret_key`, `step_not_in_session_scope` or `step_not_client_callable`. Instance and participant steps can also be rejected with `instance_terminal` (the instance is in a terminal state) or `participant_terminal` (the wallet's participant is in a terminal state), each with `detail.state`.\n"
    );
    if !spec.events.is_empty() {
        let _ = writeln!(out, "## Events\n");
        for (name, event) in &spec.events {
            let _ = writeln!(
                out,
                "- `{name}` ({}){}",
                event.machine.as_str(),
                event
                    .description
                    .as_deref()
                    .map(|description| format!(": {description}"))
                    .unwrap_or_default()
            );
        }
        out.push('\n');
        let _ = writeln!(
            out,
            "Verify webhook deliveries with `verify_{}_webhook(secret, raw_body, headers)`.\n",
            py_ident::snake_case(&spec.name)
        );
    }
    if !context.stacks.is_empty() || !context.program_members.is_empty() {
        let _ = writeln!(out, "## Stacks and programs\n");
        for key in context.stacks.keys() {
            let _ = writeln!(out, "- `STACKS[\"{key}\"]`");
        }
        for (key, _) in context.program_members {
            let _ = writeln!(out, "- `PROGRAMS[\"{key}\"]`");
        }
    }
    out
}

fn render_pyproject(alias: &str, package_name: &str) -> String {
    format!(
        "[project]\nname = \"{}\"\nversion = \"0.1.0\"\nrequires-python = \">=3.9\"\ndependencies = [\"arete-sdk>={}\"]\n\n[build-system]\nrequires = [\"setuptools>=61.0\"]\nbuild-backend = \"setuptools.build_meta\"\n\n[tool.setuptools.packages.find]\ninclude = [\"{package_name}*\"]\n",
        to_kebab_case(&format!("{alias}-protocol")),
        arete_interpreter::python::GENERATED_PYTHON_SDK_VERSION,
    )
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

fn step_access(step: &StepV1) -> StepAccess {
    // A validated spec gives every client-callable step an access.
    step.access.unwrap_or(StepAccess::Secret)
}

fn access_name(access: StepAccess) -> &'static str {
    match access {
        StepAccess::Public => "public",
        StepAccess::Secret => "secret",
    }
}

fn scope_name(scope: StepScope) -> &'static str {
    match scope {
        StepScope::Create => "create",
        StepScope::Instance => "instance",
        StepScope::Participant => "participant",
    }
}

fn is_integer_format(format: Option<StringFormat>) -> bool {
    matches!(
        format,
        Some(
            StringFormat::U8
                | StringFormat::U16
                | StringFormat::U32
                | StringFormat::U64
                | StringFormat::U128
                | StringFormat::I64
                | StringFormat::UnixTimestamp
        )
    )
}

/// An integer sent as a decimal string, or an array of them.
fn is_integer_field(field: &FieldSchemaV1) -> bool {
    match field.field_type {
        FieldType::String => is_integer_format(field.format),
        FieldType::Array => field.items.as_deref().is_some_and(is_integer_field),
        _ => false,
    }
}

fn is_python_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
        && !py_ident::is_keyword(value)
}

/// The Python type of one field; nested objects become `TypedDict`s named
/// `<parent><Field>`, pushed to `definitions` before their parent.
fn field_type(
    definitions: &mut Vec<String>,
    parent: &str,
    name: &str,
    field: &FieldSchemaV1,
) -> String {
    match field.field_type {
        FieldType::String if is_integer_format(field.format) => "ProtocolInteger".into(),
        FieldType::String => match &field.enum_values {
            Some(values) if !values.is_empty() => literal(values.iter().collect::<BTreeSet<_>>()),
            _ => "str".into(),
        },
        FieldType::Boolean => "bool".into(),
        FieldType::Integer => "int".into(),
        FieldType::Array => {
            let item = field
                .items
                .as_deref()
                .map(|item| field_type(definitions, parent, name, item))
                .unwrap_or_else(|| "Any".into());
            format!("List[{item}]")
        }
        FieldType::Object => {
            let properties = field.properties.clone().unwrap_or_default();
            if properties.is_empty() {
                return "Dict[str, Any]".into();
            }
            let nested = format!(
                "{parent}{}",
                ts_ident::identifier_stem(name, IdentifierCase::Pascal)
            );
            typed_dict(
                definitions,
                &nested,
                &properties,
                field.required.as_deref().unwrap_or_default(),
                false,
            );
            nested
        }
    }
}

/// A `TypedDict`. Class syntax when every key is an identifier (required
/// keys on a `total=True` base, the rest on the `total=False` class);
/// otherwise the functional form, with every key optional.
fn typed_dict(
    definitions: &mut Vec<String>,
    name: &str,
    properties: &BTreeMap<String, FieldSchemaV1>,
    required: &[String],
    all_optional: bool,
) {
    let required = required.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let fields = properties
        .iter()
        .map(|(key, field)| {
            (
                key.as_str(),
                field_type(definitions, name, key, field),
                !all_optional && required.contains(key.as_str()),
            )
        })
        .collect::<Vec<_>>();
    let mut out = String::new();
    if !fields.iter().all(|(key, _, _)| is_python_identifier(key)) {
        let _ = writeln!(out, "{name} = TypedDict(\"{name}\", {{");
        for (key, ty, _) in &fields {
            let _ = writeln!(out, "    {}: {ty},", py_str(key));
        }
        out.push_str("}, total=False)\n");
        definitions.push(out);
        return;
    }
    let required_fields = fields.iter().filter(|field| field.2).collect::<Vec<_>>();
    let optional_fields = fields.iter().filter(|field| !field.2).collect::<Vec<_>>();
    if fields.is_empty() {
        let _ = writeln!(out, "class {name}(TypedDict):\n    pass\n");
    } else if optional_fields.is_empty() {
        let _ = writeln!(out, "class {name}(TypedDict):");
        for (key, ty, _) in &required_fields {
            let _ = writeln!(out, "    {key}: {ty}");
        }
        out.push('\n');
    } else if required_fields.is_empty() {
        let _ = writeln!(out, "class {name}(TypedDict, total=False):");
        for (key, ty, _) in &optional_fields {
            let _ = writeln!(out, "    {key}: {ty}");
        }
        out.push('\n');
    } else {
        let _ = writeln!(out, "class _{name}Required(TypedDict):");
        for (key, ty, _) in &required_fields {
            let _ = writeln!(out, "    {key}: {ty}");
        }
        let _ = writeln!(out, "\n\nclass {name}(_{name}Required, total=False):");
        for (key, ty, _) in &optional_fields {
            let _ = writeln!(out, "    {key}: {ty}");
        }
        out.push('\n');
    }
    definitions.push(out);
}

fn metadata_typed_dict(
    definitions: &mut Vec<String>,
    name: &str,
    schema: Option<&MetadataSchemaV1>,
) {
    let properties = schema
        .map(|schema| {
            schema
                .fields
                .iter()
                .map(|(key, field)| (key.clone(), field.schema.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    typed_dict(definitions, name, &properties, &[], true);
}

fn literal<S: AsRef<str>>(values: impl IntoIterator<Item = S>) -> String {
    let values = values
        .into_iter()
        .map(|value| py_str(value.as_ref()))
        .collect::<Vec<_>>();
    if values.is_empty() {
        "NoReturn".into()
    } else {
        format!("Literal[{}]", values.join(", "))
    }
}

fn py_tuple<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    let values = values.into_iter().map(py_str).collect::<Vec<_>>();
    match values.len() {
        0 => "()".into(),
        1 => format!("({},)", values[0]),
        _ => format!("({})", values.join(", ")),
    }
}

/// A double-quoted Python string literal.
fn py_str(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                let _ = write!(out, "\\x{:02x}", character as u32);
            }
            character => out.push(character),
        }
    }
    out.push('"');
    out
}

/// Compare a generated Python protocol output with its golden, with the
/// `arete-sdk` release pin replaced by a placeholder, after checking every
/// Python file compiles.
#[cfg(test)]
pub(crate) fn compare_python_protocol_golden(
    name: &str,
    root: &Path,
    golden_dir: &Path,
    generated: BTreeMap<String, String>,
) {
    super::stack_name_golden::assert_syntax(name, root, &generated);
    let normalized = super::stack_name_golden::without_release_version(
        generated
            .into_iter()
            .map(|(path, contents)| (format!("python/{path}"), contents))
            .collect(),
    )
    .into_iter()
    .map(|(path, contents)| (path["python/".len()..].to_string(), contents))
    .collect();
    super::stack_name_golden::compare_with_golden(name, golden_dir, normalized);
}

#[cfg(test)]
mod tests {
    //! The generated module for each protocol spec test vector, byte for
    //! byte, with no hoisted stacks or programs. The full install, with
    //! hoisting, is `protocol_install_python_golden` in the installer tests.
    //! Regenerate with `A4_UPDATE_GOLDEN=1 cargo test -p a4-cli protocol_python`.

    use super::*;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("cli crate lives in the repo root")
            .to_path_buf()
    }

    fn render(vector: &str) -> String {
        let path = repo_root().join(format!("test-vectors/protocol/{vector}.protocol-spec.json"));
        let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let mut resolved: ResolvedProtocol = serde_json::from_value(
            crate::project::protocol::test_support::protocol(&value, "public"),
        )
        .unwrap();
        if vector == "token-raise" {
            // Token raise publishes its presale row as a live stack view.
            resolved.live_stack = Some(crate::project::resolver::ResolvedProtocolLiveStack {
                stack: "meteoraPresale".into(),
                view: "Presale/state".into(),
                key_account: "presale".into(),
            });
        }
        let spec = protocol_spec(&resolved).expect("valid spec vector");
        let context = PythonProtocolContext {
            package: vector,
            version: &spec.version,
            package_release_hash: &format!("arete:h1:package-release:sha256:{}", "9".repeat(64)),
            protocol: &resolved,
            spec: &spec,
            stacks: &BTreeMap::new(),
            program_imports: &[],
            program_members: &[],
        };
        render_python_protocol_module(&context).expect("render")
    }

    #[test]
    fn protocol_python_module_golden() {
        let mut generated = BTreeMap::new();
        for vector in ["token-raise", "escrow-resolution"] {
            generated.insert(
                format!("{}/__init__.py", python_module_name(vector)),
                render(vector),
            );
        }
        let temp = tempfile::tempdir().unwrap();
        for (relative, contents) in &generated {
            let path = temp.path().join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        super::super::stack_name_golden::assert_syntax("protocol", temp.path(), &generated);
        crate::commands::sdk::compare_golden_tree(
            "python protocol modules",
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/golden/installed-protocol-python/modules"),
            generated,
        );
    }

    #[test]
    fn secret_steps_are_only_on_the_secret_client() {
        let module = render("token-raise");
        let public = module
            .split("class TokenRaisePublicSteps:")
            .nth(1)
            .and_then(|rest| rest.split("class TokenRaiseSecretSteps").next())
            .expect("public steps class");
        assert!(public.contains("async def contribute("), "{public}");
        assert!(!public.contains("async def create("), "{public}");
        let secret = module
            .split("class TokenRaiseSecretSteps(TokenRaisePublicSteps):")
            .nth(1)
            .and_then(|rest| rest.split("class TokenRaiseClient").next())
            .expect("secret steps class");
        assert!(secret.contains("async def create("), "{secret}");
        // Squads proposals are never client-callable.
        assert!(!module.contains("async def finalize("));
        assert!(module.contains("if self.key_class != \"secret\":"));
        assert!(module.contains(
            "live_stack=ProtocolLiveStack(stack=\"meteoraPresale\", view=\"Presale/state\", key_account=\"presale\"),"
        ));
        assert_eq!(module.matches("def from_session(").count(), 2);
        assert_eq!(module.matches("def instance_handle(").count(), 2);
    }

    #[test]
    fn stack_constant_reads_the_single_stack_def() {
        assert_eq!(
            stack_constant("x\nORE_STREAM_STACK: StackDef = StackDef(\n    name=\"ore\",\n"),
            Some("ORE_STREAM_STACK".into())
        );
        assert_eq!(stack_constant("from . import live  # noqa: F401\n"), None);
    }
}
