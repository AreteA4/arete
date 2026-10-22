//! The TypeScript client of a protocol package.
//!
//! `a4 install protocol <ref> --ts` writes `protocols/<alias>/`:
//!
//! - `<alias>.ts`: the protocol definition (`<NAME>_PROTOCOL`) with its
//!   generated types: params, per-step args and guard-code unions, machine
//!   states, metadata, event names and the webhook payload;
//! - `protocol-spec.json`: the exact `arete.protocol-spec/v1`;
//! - `stacks/<stack>/`: each stack the protocol hoists, generated exactly as
//!   `a4 install stack` generates it (with its own `programs/`);
//! - `programs/<program>/`: each program the protocol declares that none of
//!   its stacks bundles, generated as `a4 install program` generates it (a
//!   program the package leaves unpinned has no SDK to generate; the README
//!   lists it for the app to install);
//! - `README.md`, `sdk-manifest.json` and `sdk-provenance.json`.
//!
//! The runtime (`createProtocolClient`, `session.protocols`) lives in
//! `@usearete/sdk`. Nothing here carries a credential: the deployment is
//! named by id and URL only, and the app's own key supplies identity.
//!
//! `--rust` generates the Rust client instead; see [`super::protocol_rust`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use arete_hash::protocol::{
    AuthorityV1, FieldSchemaV1, FieldType, MetadataSchemaV1, MetadataWriter, ObjectSchemaV1,
    ProtocolSpecV1, StepAccess, StepScope, StringFormat,
};
use arete_interpreter::identifiers::{typescript as ts_ident, IdentifierCase};
use arete_interpreter::typescript::typescript_property_key;

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
use crate::project::resolver::{
    ResolvedProtocol, ResolvedRegistryDependency, ResolvedUnpinnedProgram,
};

pub(super) const PROTOCOL_SPEC_FILE: &str = "protocol-spec.json";
pub(super) const README_FILE: &str = "README.md";

pub(crate) fn generate_project_protocol(
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
    match options.target {
        InstallTarget::TypeScript => {}
        InstallTarget::Rust => {
            return super::protocol_rust::generate_rust_protocol(dependency, options)
        }
        InstallTarget::Python => {
            return super::protocol_python::generate_project_protocol_python(dependency, options)
        }
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
    fs::create_dir_all(options.output).with_context(|| {
        format!(
            "Failed to create protocol output {}",
            options.output.display()
        )
    })?;

    // Each hoisted stack, exactly as a stack install generates it.
    let mut stack_modules = BTreeMap::new();
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
                target: InstallTarget::TypeScript,
                output: &options.output.join("stacks").join(&directory),
                typescript_package: options.typescript_package,
                rust_module: false,
                python_module: false,
                stack_endpoints: None,
                reference: None,
                program_install: None,
            },
        )
        .with_context(|| format!("Failed to generate stack '{key}' of protocol '{alias}'"))?;
        stack_modules.insert(
            key.clone(),
            ModuleImport {
                binding: format!(
                    "{}_STACK",
                    ts_ident::identifier_stem(&key, IdentifierCase::ScreamingSnake)
                ),
                path: format!("./stacks/{directory}/{directory}.js"),
                package_release_hash: Some(stack.package_release_hash().to_string()),
            },
        );
    }

    // Each program no stack bundles, as a program install generates it; the
    // rest are the bundling stack's program modules.
    let mut program_imports = Vec::new();
    let mut program_members = Vec::new();
    for program in &hoisted {
        match program.source {
            HoistedProgramSource::Stack {
                stack,
                program: install,
            } => {
                let artifact = program_spec_artifact_from_registry(install)?;
                let stack_key = arete_interpreter::typescript::program_key(
                    &artifact.payload.idl_snapshot.snapshot.name,
                );
                let module = stack_modules.get(stack).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Hoisted program '{}' names unknown stack '{stack}'",
                        program.key
                    )
                })?;
                program_members.push((
                    program.key.clone(),
                    format!(
                        "{}.programs{}",
                        module.binding,
                        typescript_member_access(&stack_key)
                    ),
                ));
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
                        target: InstallTarget::TypeScript,
                        output: &options.output.join("programs").join(&directory),
                        typescript_package: options.typescript_package,
                        rust_module: false,
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
                let binding = format!(
                    "{}_PROGRAM",
                    ts_ident::identifier_stem(&program.key, IdentifierCase::ScreamingSnake)
                );
                program_imports.push(ModuleImport {
                    binding: binding.clone(),
                    path: format!("./programs/{directory}/{directory}.js"),
                    package_release_hash: None,
                });
                program_members.push((program.key.clone(), binding));
            }
        }
    }

    let context = ProtocolModuleContext {
        alias,
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
        options.output.join(format!("{alias}.ts")),
        render_protocol_module(&context)?,
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
        with_unbundled_programs(
            render_protocol_readme(&context),
            unbundled,
            InstallTarget::TypeScript,
        ),
    )?;

    // The protocol output's provenance owns every nested file. Nested SDKs
    // keep their `sdk-manifest.json` (input and content identity), but not a
    // provenance of their own, whose generator identity would make the
    // protocol's content hash change with the generator.
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

/// Every file under `output`, except its own root provenance and manifest.
pub(super) fn collect_payload_files(
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

struct ModuleImport {
    binding: String,
    path: String,
    /// The package release a hoisted stack was installed from: its identity
    /// when a session meets the same stack from another source.
    package_release_hash: Option<String>,
}

struct ProtocolModuleContext<'a> {
    alias: &'a str,
    package: &'a str,
    version: &'a str,
    package_release_hash: &'a str,
    protocol: &'a ResolvedProtocol,
    spec: &'a ProtocolSpecV1,
    stacks: &'a BTreeMap<String, ModuleImport>,
    program_imports: &'a [ModuleImport],
    /// Spec program key -> the expression that names its program SDK.
    program_members: &'a [(String, String)],
}

impl ProtocolModuleContext<'_> {
    fn pascal(&self) -> String {
        ts_ident::identifier_stem(&self.spec.name, IdentifierCase::Pascal)
    }

    fn screaming(&self) -> String {
        ts_ident::identifier_stem(&self.spec.name, IdentifierCase::ScreamingSnake)
    }

    /// Steps an app can prepare: wallet steps, in key order.
    fn client_steps(&self) -> impl Iterator<Item = (&String, &arete_hash::protocol::StepV1)> {
        self.spec
            .steps
            .iter()
            .filter(|(_, step)| step.authority.is_client_callable())
    }
}

fn render_protocol_module(context: &ProtocolModuleContext<'_>) -> Result<String> {
    let spec = context.spec;
    let pascal = context.pascal();
    let screaming = context.screaming();
    let mut out = String::new();
    writeln!(
        out,
        "// Generated by a4 from protocol package `{}` {} (arete.protocol-spec/v1 `{}`). Do not edit.",
        context.package, context.version, spec.name
    )?;
    out.push_str(
        "import {\n  verifyWebhook as verifyProtocolWebhook,\n  type ProtocolDefinition,\n",
    );
    if uses_integer(spec) {
        out.push_str("  type ProtocolInteger,\n");
    }
    out.push_str(
        "  type ProtocolWebhookPayload,\n  type VerifiedWebhook,\n  type VerifyWebhookOptions,\n  type WebhookHeaders,\n} from '@usearete/sdk';\n",
    );
    for module in context.stacks.values().chain(context.program_imports) {
        writeln!(out, "import {} from '{}';", module.binding, module.path)?;
    }
    out.push('\n');

    // Machine states.
    let instance_states = string_union(spec.machines.instance.states.keys());
    writeln!(
        out,
        "export type {pascal}InstanceState = {instance_states};"
    )?;
    let participant_states = spec
        .machines
        .participant
        .as_ref()
        .map(|machine| string_union(machine.states.keys()))
        .unwrap_or_else(|| "never".into());
    writeln!(
        out,
        "export type {pascal}ParticipantState = {participant_states};"
    )?;
    out.push('\n');

    // Params.
    write_object_interface(&mut out, &format!("{pascal}Params"), &spec.params)?;

    // Client-callable steps: args and guard codes.
    let mut step_types = Vec::new();
    for (name, step) in context.client_steps() {
        let step_pascal = ts_ident::identifier_stem(name, IdentifierCase::Pascal);
        let args = format!("{pascal}{step_pascal}Args");
        let guard = format!("{pascal}{step_pascal}GuardCode");
        write_object_interface(&mut out, &args, &step.args)?;
        let codes = step
            .guards
            .iter()
            .map(|guard| guard.code.as_str())
            .collect::<BTreeSet<_>>();
        writeln!(
            out,
            "export type {guard} = {};",
            if codes.is_empty() {
                "never".to_string()
            } else {
                string_union(codes)
            }
        )?;
        step_types.push((name.clone(), step, args, guard));
    }
    out.push('\n');

    // Metadata.
    write_metadata_interface(
        &mut out,
        &format!("{pascal}InstanceMetadata"),
        spec.metadata.instance.as_ref(),
    )?;
    write_metadata_interface(
        &mut out,
        &format!("{pascal}ParticipantMetadata"),
        spec.metadata.participant.as_ref(),
    )?;
    let events = if spec.events.is_empty() {
        "never".to_string()
    } else {
        string_union(spec.events.keys())
    };
    writeln!(out, "export type {pascal}Event = {events};")?;
    out.push('\n');

    // The type map.
    writeln!(out, "export interface {pascal}ProtocolTypes {{")?;
    writeln!(out, "  readonly params: {pascal}Params;")?;
    writeln!(out, "  readonly instanceState: {pascal}InstanceState;")?;
    writeln!(
        out,
        "  readonly participantState: {pascal}ParticipantState;"
    )?;
    writeln!(
        out,
        "  readonly instanceMetadata: {pascal}InstanceMetadata;"
    )?;
    writeln!(
        out,
        "  readonly participantMetadata: {pascal}ParticipantMetadata;"
    )?;
    if step_types.is_empty() {
        out.push_str("  readonly steps: {};\n");
    } else {
        out.push_str("  readonly steps: {\n");
        for (name, step, args, guard) in &step_types {
            writeln!(
                out,
                "    readonly {}: {{ readonly scope: '{}'; readonly access: '{}'; readonly args: {args}; readonly guardCode: {guard} }};",
                typescript_property_key(name),
                scope_name(step.scope),
                access_name(step_access(step)),
            )?;
        }
        out.push_str("  };\n");
    }
    if spec.events.is_empty() {
        out.push_str("  readonly events: {};\n");
    } else {
        out.push_str("  readonly events: {\n");
        for (name, event) in &spec.events {
            writeln!(
                out,
                "    readonly {}: {{ readonly machine: '{}' }};",
                typescript_property_key(name),
                machine_name(event.machine),
            )?;
        }
        out.push_str("  };\n");
    }
    out.push_str("}\n\n");

    // Hoisted members.
    out.push_str("const STACKS = {");
    if context.stacks.is_empty() {
        out.push_str("} as const;\n");
    } else {
        out.push('\n');
        for (key, module) in context.stacks {
            match &module.package_release_hash {
                Some(release) => writeln!(
                    out,
                    "  {}: {{ ...{}, packageReleaseHash: {} }},",
                    typescript_property_key(key),
                    module.binding,
                    ts_string(release)
                )?,
                None => writeln!(
                    out,
                    "  {}: {},",
                    typescript_property_key(key),
                    module.binding
                )?,
            }
        }
        out.push_str("} as const;\n");
    }
    out.push_str("const PROGRAMS = {");
    if context.program_members.is_empty() {
        out.push_str("} as const;\n\n");
    } else {
        out.push('\n');
        for (key, value) in context.program_members {
            writeln!(out, "  {}: {value},", typescript_property_key(key))?;
        }
        out.push_str("} as const;\n\n");
    }

    // The definition.
    let deployment = &context.protocol.deployment;
    writeln!(
        out,
        "export const {screaming}_PROTOCOL: ProtocolDefinition<{pascal}ProtocolTypes, typeof STACKS, typeof PROGRAMS> = {{"
    )?;
    out.push_str("  kind: 'protocol',\n");
    writeln!(out, "  name: {},", ts_string(&spec.name))?;
    writeln!(out, "  version: {},", ts_string(&spec.version))?;
    writeln!(out, "  package: {},", ts_string(context.package))?;
    writeln!(
        out,
        "  packageReleaseHash: {},",
        ts_string(context.package_release_hash)
    )?;
    writeln!(
        out,
        "  protocolSpecHash: {},",
        ts_string(&context.protocol.protocol_spec_hash)
    )?;
    writeln!(
        out,
        "  protocolReleaseHash: {},",
        ts_string(&context.protocol.protocol_release_hash)
    )?;
    // A client `a4 dev` generates calls the deployment in the project's dev
    // environment, a fork of mainnet, not a published deployment. The
    // assertion keeps an app on an SDK without the field compiling.
    let (environment, assertion) = if context.protocol.local_dev {
        (
            ", environment: 'dev'",
            " as ProtocolDefinition['deployment']",
        )
    } else {
        ("", "")
    };
    writeln!(
        out,
        "  deployment: {{ deploymentId: {}, access: '{}', apiUrl: {}, sessionEndpoint: {}{environment} }}{assertion},",
        ts_string(&deployment.deployment_id),
        deployment.access.as_str(),
        ts_string(&deployment.api_url),
        ts_string(&deployment.session_endpoint),
    )?;
    writeln!(
        out,
        "  instanceVisibility: '{}',",
        match spec.instance_visibility {
            arete_hash::protocol::InstanceVisibility::Shared => "shared",
            arete_hash::protocol::InstanceVisibility::Account => "account",
        }
    )?;
    if step_types.is_empty() {
        out.push_str("  steps: {},\n");
    } else {
        out.push_str("  steps: {\n");
        for (name, step, _, _) in &step_types {
            let guards = step
                .guards
                .iter()
                .map(|guard| ts_string(&guard.code))
                .collect::<Vec<_>>()
                .join(", ");
            let args = step
                .args
                .properties
                .keys()
                .map(|arg| ts_string(arg))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(
                out,
                "    {}: {{ scope: '{}', access: '{}', guards: [{guards}], args: [{args}] }},",
                typescript_property_key(name),
                scope_name(step.scope),
                access_name(step_access(step)),
            )?;
        }
        out.push_str("  },\n");
    }
    if spec.events.is_empty() {
        out.push_str("  events: {},\n");
    } else {
        out.push_str("  events: { ");
        out.push_str(
            &spec
                .events
                .iter()
                .map(|(name, event)| {
                    format!(
                        "{}: {{ machine: '{}' }}",
                        typescript_property_key(name),
                        machine_name(event.machine)
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push_str(" },\n");
    }
    let metadata = [
        ("instance", spec.metadata.instance.as_ref()),
        ("participant", spec.metadata.participant.as_ref()),
    ]
    .into_iter()
    .filter_map(|(machine, schema)| {
        schema.map(|schema| {
            let fields = schema
                .fields
                .iter()
                .map(|(name, field)| {
                    format!(
                        "{}: {{ writer: '{}', filterable: {} }}",
                        typescript_property_key(name),
                        match field.writer {
                            MetadataWriter::Creator => "creator",
                            MetadataWriter::Participant => "participant",
                        },
                        field.filterable
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("{machine}: {{ {fields} }}")
        })
    })
    .collect::<Vec<_>>();
    if metadata.is_empty() {
        out.push_str("  metadata: {},\n");
    } else {
        writeln!(out, "  metadata: {{ {} }},", metadata.join(", "))?;
    }
    if let Some(live) = &context.protocol.live_stack {
        writeln!(
            out,
            "  liveStack: {{ stack: {}, view: {}, keyAccount: {} }},",
            ts_string(&live.stack),
            ts_string(&live.view),
            ts_string(&live.key_account)
        )?;
    }
    out.push_str("  stacks: STACKS,\n  programs: PROGRAMS,\n};\n\n");

    // Webhooks.
    writeln!(
        out,
        "export type {pascal}WebhookPayload = ProtocolWebhookPayload<{pascal}ProtocolTypes>;"
    )?;
    out.push_str("/** Verify a signed webhook delivery for this protocol's events. */\n");
    writeln!(
        out,
        "export function verify{pascal}Webhook(secret: string, body: string | Uint8Array, headers: WebhookHeaders, options?: VerifyWebhookOptions): VerifiedWebhook<{pascal}WebhookPayload> {{"
    )?;
    writeln!(
        out,
        "  return verifyProtocolWebhook<{pascal}WebhookPayload>(secret, body, headers, options);"
    )?;
    out.push_str("}\n");
    out.push_str("export { verifyWebhook } from '@usearete/sdk';\n");
    writeln!(out, "export default {screaming}_PROTOCOL;")?;
    Ok(out)
}

fn render_protocol_readme(context: &ProtocolModuleContext<'_>) -> String {
    let spec = context.spec;
    let screaming = context.screaming();
    let deployment = &context.protocol.deployment;
    let mut out = String::new();
    let _ = writeln!(out, "# `{}` protocol client\n", context.alias);
    let _ = writeln!(
        out,
        "Generated from protocol package `{}` {} (spec `{}` {}), installed as `{}`. `a4 install` regenerates this folder, so do not edit it.\n",
        context.package, context.version, spec.name, spec.version, context.alias
    );
    if let Some(description) = &spec.description {
        let _ = writeln!(out, "{description}\n");
    }
    let _ = writeln!(out, "## Import\n");
    let _ = writeln!(
        out,
        "```ts\nimport {{ createProtocolClient }} from '@usearete/sdk';\nimport {{ {screaming}_PROTOCOL }} from './generated/typescript/protocols/{alias}/{alias}.js'; // from the project root\n\n// Browser: a publishable key prepares `public` steps only.\nconst client = createProtocolClient({screaming}_PROTOCOL, {{ auth: {{ publishableKey }} }});\n```\n",
        alias = context.alias
    );
    let _ = writeln!(
        out,
        "Or register it in a session: `createSession({{ protocols: {{ {}: {screaming}_PROTOCOL }} }})` gives `session.protocols.{}`, and promotes the protocol's stacks and programs into `session.stacks` and `session.programs`.\n",
        typescript_property_key(&spec.name),
        typescript_property_key(&spec.name)
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
        "| Step | Scope | Authority | Access | Rejection codes |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|");
    for (name, step) in &spec.steps {
        let access = if step.authority.is_client_callable() {
            access_name(step_access(step)).to_string()
        } else {
            "not client-callable".to_string()
        };
        let codes = step
            .guards
            .iter()
            .map(|guard| format!("`{}`", guard.code))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "| `{name}` | {} | {} | {access} | {} |",
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
        for (name, event) in &spec.events {
            let _ = writeln!(
                out,
                "- `{name}` ({}){}",
                machine_name(event.machine),
                event
                    .description
                    .as_deref()
                    .map(|description| format!(": {description}"))
                    .unwrap_or_default()
            );
        }
        out.push('\n');
    }
    if !context.stacks.is_empty() || !context.program_members.is_empty() {
        let _ = writeln!(out, "## Stacks and programs\n");
        for key in context.stacks.keys() {
            let _ = writeln!(out, "- `stacks.{key}`");
        }
        for (key, _) in context.program_members {
            let _ = writeln!(out, "- `programs.{key}`");
        }
    }
    out
}

/// `readme` with the section for the programs a protocol client does not
/// bundle (the package leaves them unpinned: installed from a local IDL, with
/// no published program package) appended; unchanged when there are none.
pub(super) fn with_unbundled_programs(
    readme: String,
    unbundled: &[ResolvedUnpinnedProgram],
    target: InstallTarget,
) -> String {
    if unbundled.is_empty() {
        return readme;
    }
    let mut out = readme.trim_end().to_string();
    out.push_str("\n\n");
    let _ = writeln!(
        out,
        "## Programs this client does not bundle
"
    );
    let _ = writeln!(
        out,
        "The protocol pins these programs by their exact ProgramSpec only: they were installed from a local IDL, so there is no published program package and this client carries no SDK for them. Preparing and submitting steps does not need one.
"
    );
    let _ = writeln!(out, "| Key | Program id | ProgramSpec |");
    let _ = writeln!(out, "|---|---|---|");
    for program in unbundled {
        let _ = writeln!(
            out,
            "| `{}` | `{}` | `{}` |",
            program.key, program.program_id, program.program_spec_hash
        );
    }
    let (directory, usage) = match target {
        InstallTarget::TypeScript => (
            "typescript",
            "To share it in a session, pass it under the protocol's key: `createSession({ programs: { <key>: <PROGRAM> }, protocols: { … } })`.",
        ),
        InstallTarget::Rust => ("rust", "Use it next to this client."),
        InstallTarget::Python => (
            "python",
            "To share it in a session, pass it under the protocol's key: `arete.create_session(programs={\"<key>\": <PROGRAM>, …})`.",
        ),
    };
    let _ = writeln!(
        out,
        "\nIf the app reads or calls one of them directly, install it in this project from its IDL (`a4 install program <idl.json> --as <alias>`) and import it from `generated/{directory}/programs/<alias>/`. A program with the same ProgramSpec hash is the same program; `a4 install` names the project's own install when it finds one. {usage}"
    );
    out
}

pub(super) fn step_access(step: &arete_hash::protocol::StepV1) -> StepAccess {
    // A validated spec gives every client-callable step an access.
    step.access.unwrap_or(StepAccess::Secret)
}

pub(super) fn access_name(access: StepAccess) -> &'static str {
    match access {
        StepAccess::Public => "public",
        StepAccess::Secret => "secret",
    }
}

pub(super) fn scope_name(scope: StepScope) -> &'static str {
    match scope {
        StepScope::Create => "create",
        StepScope::Instance => "instance",
        StepScope::Participant => "participant",
    }
}

pub(super) fn machine_name(machine: arete_hash::protocol::MachineKind) -> &'static str {
    match machine {
        arete_hash::protocol::MachineKind::Instance => "instance",
        arete_hash::protocol::MachineKind::Participant => "participant",
    }
}

pub(super) fn authority_name(authority: &AuthorityV1) -> String {
    match authority {
        AuthorityV1::Creator => "creator".into(),
        AuthorityV1::Participant => "participant".into(),
        AuthorityV1::Role { role } => format!("role `{role}`"),
        AuthorityV1::Keeper => "keeper".into(),
        AuthorityV1::SquadsProposer { .. } => "Squads proposal".into(),
    }
}

pub(super) fn is_integer_format(format: Option<StringFormat>) -> bool {
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

fn field_uses_integer(field: &FieldSchemaV1) -> bool {
    (field.field_type == FieldType::String && is_integer_format(field.format))
        || field.items.as_deref().is_some_and(field_uses_integer)
        || field
            .properties
            .iter()
            .flat_map(BTreeMap::values)
            .any(field_uses_integer)
}

fn uses_integer(spec: &ProtocolSpecV1) -> bool {
    let object = |schema: &ObjectSchemaV1| schema.properties.values().any(field_uses_integer);
    object(&spec.params)
        || spec
            .steps
            .values()
            .filter(|step| step.authority.is_client_callable())
            .any(|step| object(&step.args))
        || [&spec.metadata.instance, &spec.metadata.participant]
            .into_iter()
            .flatten()
            .any(|schema| {
                schema
                    .fields
                    .values()
                    .any(|field| field_uses_integer(&field.schema))
            })
}

/// The TypeScript type of one field: integers as `ProtocolInteger`
/// (`string | bigint`), public keys and other strings as `string`.
fn field_type(field: &FieldSchemaV1, indent: usize) -> String {
    match field.field_type {
        FieldType::String if is_integer_format(field.format) => "ProtocolInteger".into(),
        FieldType::String => match &field.enum_values {
            Some(values) if !values.is_empty() => {
                string_union(values.iter().collect::<BTreeSet<_>>())
            }
            _ => "string".into(),
        },
        FieldType::Boolean => "boolean".into(),
        FieldType::Integer => "number".into(),
        FieldType::Array => {
            let item = field
                .items
                .as_deref()
                .map(|item| field_type(item, indent))
                .unwrap_or_else(|| "unknown".into());
            if item.contains(' ') {
                format!("readonly ({item})[]")
            } else {
                format!("readonly {item}[]")
            }
        }
        FieldType::Object => {
            let properties = field.properties.clone().unwrap_or_default();
            if properties.is_empty() {
                return "Record<string, never>".into();
            }
            let required = field
                .required
                .iter()
                .flatten()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            let pad = "  ".repeat(indent + 1);
            let mut out = String::from("{\n");
            for (name, property) in &properties {
                let _ = writeln!(
                    out,
                    "{pad}{}{}: {};",
                    typescript_property_key(name),
                    if required.contains(name.as_str()) {
                        ""
                    } else {
                        "?"
                    },
                    field_type(property, indent + 1)
                );
            }
            out.push_str(&"  ".repeat(indent));
            out.push('}');
            out
        }
    }
}

fn write_object_interface(out: &mut String, name: &str, schema: &ObjectSchemaV1) -> Result<()> {
    if schema.properties.is_empty() {
        writeln!(out, "export interface {name} {{}}")?;
        return Ok(());
    }
    let required = schema
        .required
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    writeln!(out, "export interface {name} {{")?;
    for (property, field) in &schema.properties {
        writeln!(
            out,
            "  {}{}: {};",
            typescript_property_key(property),
            if required.contains(property.as_str()) {
                ""
            } else {
                "?"
            },
            field_type(field, 1)
        )?;
    }
    out.push_str("}\n");
    Ok(())
}

/// A metadata document: every field optional, since a document holds only
/// the fields written so far.
fn write_metadata_interface(
    out: &mut String,
    name: &str,
    schema: Option<&MetadataSchemaV1>,
) -> Result<()> {
    let Some(schema) = schema.filter(|schema| !schema.fields.is_empty()) else {
        writeln!(out, "export interface {name} {{}}")?;
        return Ok(());
    };
    writeln!(out, "export interface {name} {{")?;
    for (field, declared) in &schema.fields {
        writeln!(
            out,
            "  {}?: {};",
            typescript_property_key(field),
            field_type(&declared.schema, 1)
        )?;
    }
    out.push_str("}\n");
    Ok(())
}

fn string_union<S: AsRef<str>>(values: impl IntoIterator<Item = S>) -> String {
    let values = values
        .into_iter()
        .map(|value| ts_string(value.as_ref()))
        .collect::<Vec<_>>();
    if values.is_empty() {
        "never".into()
    } else {
        values.join(" | ")
    }
}

/// A single-quoted TypeScript string literal.
fn ts_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            character => out.push(character),
        }
    }
    out.push('\'');
    out
}

/// `.key` for identifiers, `['key']` otherwise.
fn typescript_member_access(key: &str) -> String {
    let property = typescript_property_key(key);
    if property == key {
        format!(".{key}")
    } else {
        format!("[{property}]")
    }
}
