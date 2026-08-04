use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use colored::Colorize;
use serde::Serialize;
use serde_json::Value;

use crate::api_client::{
    ApiClient, RegistryCapabilityInstallBinding, RegistryProgramInstallResponse,
    RegistryProgramInstallTransport, RegistryProgramItem, RegistrySdkExtensionArtifact,
    RegistryStackInstallResponse, RegistryStackItem, DEFAULT_DOMAIN_SUFFIX,
};

const EXPLORE_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExploreStackListOutput {
    schema_version: u32,
    registry: Vec<RegistryStackItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_stacks: Option<Vec<UserStackItem>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UserStackItem {
    name: String,
    entity_name: String,
    websocket_url: String,
    status: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExploreProgramListOutput {
    schema_version: u32,
    programs: Vec<RegistryProgramItem>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct StackIdentitySummary {
    stack_manifest_hash: String,
    ast_content_hash: String,
    portable_ast_hash: String,
    spec_version_id: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveSpecSummary {
    alias: String,
    live_spec_hash: String,
    deployment_id: i32,
    observed_generation: i64,
    websocket_endpoint: String,
    query_endpoint: String,
    websocket_auth_policy: String,
    query_auth_policy: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelectedViewSummary {
    live_alias: String,
    view_id: String,
    entity: String,
    source: Value,
    output: Value,
    pipeline_steps: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionSummary {
    artifact_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sdk_extension_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sdk_output_tree_hash: Option<String>,
    entry: String,
    files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sdk_range: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SdkTargetSummary {
    language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    extension: Option<ExtensionSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgramReadSummary {
    available: bool,
    endpoint: String,
    program_read_binding_id: String,
    auth: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StackProgramSummary {
    install_name: String,
    display_name: String,
    program_id: String,
    program_spec_hash: String,
    program_release_hash: String,
    program_read: ProgramReadSummary,
    sdk_targets: Vec<SdkTargetSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveAuthSummary {
    alias: String,
    websocket: String,
    query: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthenticationSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    websocket: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    http: Option<Value>,
    live_specs: Vec<LiveAuthSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chain: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transaction: Option<Value>,
    program_reads: Vec<ProgramReadSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitySummary {
    endpoint: String,
    auth_policy: String,
    cluster: String,
    region: String,
    auth: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StackExploreOutput {
    schema_version: u32,
    kind: &'static str,
    name: String,
    install_ref: String,
    description: Option<String>,
    visibility: String,
    identity: StackIdentitySummary,
    live_specs: Vec<LiveSpecSummary>,
    selected_views: Vec<SelectedViewSummary>,
    programs: Vec<StackProgramSummary>,
    sdk_targets: Vec<SdkTargetSummary>,
    authentication: AuthenticationSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    chain: Option<CapabilitySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transaction: Option<CapabilitySummary>,
    install_command: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NamedTypeSummary {
    name: String,
    #[serde(rename = "type")]
    field_type: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountSummary {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    discriminator: Option<Value>,
    fields: Vec<NamedTypeSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstructionAccountSummary {
    name: String,
    writable: bool,
    signer: bool,
    optional: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstructionSummary {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    discriminator: Option<Value>,
    arguments: Vec<NamedTypeSummary>,
    accounts: Vec<InstructionAccountSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventSummary {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    discriminator: Option<Value>,
    fields: Vec<NamedTypeSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TypeSummary {
    name: String,
    kind: String,
    fields: Vec<NamedTypeSummary>,
    variants: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ProgramIdentitySummary {
    program_id: String,
    program_spec_hash: String,
    program_release_hash: String,
    idl_content_hash: String,
    normalized_idl_hash: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgramExploreOutput {
    schema_version: u32,
    kind: &'static str,
    install_name: String,
    display_name: String,
    identity: ProgramIdentitySummary,
    accounts: Vec<AccountSummary>,
    instructions: Vec<InstructionSummary>,
    events: Vec<EventSummary>,
    types: Vec<TypeSummary>,
    program_read: ProgramReadSummary,
    sdk_targets: Vec<SdkTargetSummary>,
    install_command: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EntityFieldSummary {
    section: String,
    path: String,
    rust_type: String,
    nullable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StackEntityExploreOutput {
    schema_version: u32,
    kind: &'static str,
    stack: String,
    identity: StackIdentitySummary,
    live_alias: String,
    name: String,
    program_id: Option<String>,
    primary_keys: Vec<String>,
    fields: Vec<EntityFieldSummary>,
    views: Vec<SelectedViewSummary>,
}

#[derive(Debug, PartialEq, Eq)]
struct StackDescriptorIdentity {
    name: String,
    stack: String,
    visibility: String,
    spec_version_id: Option<i32>,
    ast_content_hash: String,
    portable_ast_hash: String,
    stack_manifest_hash: String,
    live_specs: Vec<(String, String)>,
    programs: Vec<(String, String, String)>,
}

pub fn list(json: bool) -> Result<()> {
    let client = ApiClient::new()?;
    let registry_stacks = client.list_registry()?;
    let user_stacks = client.list_specs().ok();
    let user_deployments = if user_stacks.is_some() {
        client.list_deployments(100).ok()
    } else {
        None
    };

    let user_items = user_stacks.as_ref().map(|specs| {
        let deployment_map: HashMap<i32, _> = user_deployments
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|deployment| (deployment.spec_id, deployment))
            .collect();
        specs
            .iter()
            .map(|spec| {
                let deployment = deployment_map.get(&spec.id);
                UserStackItem {
                    name: spec.name.clone(),
                    entity_name: spec.entity_name.clone(),
                    websocket_url: spec.websocket_url(DEFAULT_DOMAIN_SUFFIX),
                    status: deployment.map(|item| item.status.to_string()),
                }
            })
            .collect()
    });

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&ExploreStackListOutput {
                schema_version: EXPLORE_SCHEMA_VERSION,
                registry: registry_stacks,
                user_stacks: user_items,
            })?
        );
        return Ok(());
    }

    if !registry_stacks.is_empty() {
        println!("\n{}", "Public Registry".bold());
        println!("{}", "-".repeat(60).dimmed());
        for stack in &registry_stacks {
            println!(
                "  {}  {}",
                stack.name.green().bold(),
                stack.websocket_url.cyan()
            );
            if let Some(description) = &stack.description {
                println!("    {}", description.dimmed());
            }
            println!("    Entities: {}", stack.entities.join(", "));
            println!();
        }
    }

    if let Some(specs) = user_stacks {
        if !specs.is_empty() {
            let deployment_map: HashMap<i32, _> = user_deployments
                .unwrap_or_default()
                .into_iter()
                .map(|deployment| (deployment.spec_id, deployment))
                .collect();
            println!("{}", "Your Stacks".bold());
            println!("{}", "-".repeat(60).dimmed());
            for spec in &specs {
                let status = deployment_map
                    .get(&spec.id)
                    .map(|deployment| deployment.status.to_string())
                    .unwrap_or_else(|| "-".into());
                println!(
                    "  {}  {}  [{}]",
                    spec.name.green().bold(),
                    spec.websocket_url(DEFAULT_DOMAIN_SUFFIX).cyan(),
                    status,
                );
            }
            println!();
        }
    }

    if registry_stacks.is_empty() {
        println!("{}", "No stacks found in registry.".yellow());
    }
    println!(
        "{}",
        "Tip: Run `a4 explore stack <ref>` for deployment-pinned details".dimmed()
    );
    Ok(())
}

pub fn list_programs(json: bool) -> Result<()> {
    let programs = ApiClient::new()?.list_registry_programs()?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&ExploreProgramListOutput {
                schema_version: EXPLORE_SCHEMA_VERSION,
                programs,
            })?
        );
        return Ok(());
    }

    if programs.is_empty() {
        println!("{}", "No installable programs found in registry.".yellow());
        return Ok(());
    }
    println!("\n{}", "Installable Programs".bold());
    println!("{}", "-".repeat(72).dimmed());
    for program in programs {
        println!(
            "  {}  {}",
            program.install_name.green().bold(),
            program.display_name
        );
        println!("    Program ID: {}", program.program_id.cyan());
        println!("    Release: {}", program.program_release_hash);
        println!("    SDK targets: {}", program.sdk_targets.join(", "));
        println!();
    }
    println!(
        "{}",
        "Tip: Run `a4 explore program <ref>` for accounts and instructions".dimmed()
    );
    Ok(())
}

pub fn show_stack(reference: &str, entity: Option<&str>, json: bool) -> Result<()> {
    let client = ApiClient::new()?;
    let typescript = resolve_stack_descriptor(&client, reference, None)?;
    let rust = client
        .get_registry_stack_install(&typescript.stack, Some("rust"))
        .with_context(|| descriptor_diagnostic(&typescript.stack))?;
    validate_stack_descriptor_identity(&typescript, &rust)?;

    if let Some(entity) = entity {
        let output = build_entity_output(&typescript, entity)?;
        if json {
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            print!("{}", render_entity(&output));
        }
        return Ok(());
    }

    let output = build_stack_output(&typescript, &rust)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_stack(&output));
    }
    Ok(())
}

pub fn show_program(reference: &str, json: bool) -> Result<()> {
    let descriptor = ApiClient::new()?
        .get_registry_program_install(reference, None)
        .with_context(|| {
            format!(
                "Unable to assemble the install descriptor for program '{reference}'. Explore does not fall back to raw or latest IDL artifacts; verify that a promoted Program Release and healthy Program Read binding exist."
            )
        })?;
    let output = build_program_output(&descriptor)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_program(&output));
    }
    Ok(())
}

fn resolve_stack_descriptor(
    client: &ApiClient,
    reference: &str,
    language: Option<&str>,
) -> Result<RegistryStackInstallResponse> {
    let direct_error = match client.get_registry_stack_install(reference, language) {
        Ok(descriptor) => return Ok(descriptor),
        Err(error) => error,
    };

    // Legacy explore accepted the display name emitted by `a4 explore`. The
    // install endpoint resolves deployment references, so translate through
    // that listing and retry the descriptor endpoint. This is intentionally
    // not a schema or latest-AST fallback.
    if let Ok(stacks) = client.list_registry() {
        if let Some(item) = stacks
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(reference))
        {
            if let Some(install_ref) = install_ref_from_websocket_url(&item.websocket_url) {
                return client
                    .get_registry_stack_install(&install_ref, language)
                    .with_context(|| descriptor_diagnostic(&install_ref));
            }
        }
    }

    Err(direct_error).with_context(|| descriptor_diagnostic(reference))
}

fn descriptor_diagnostic(reference: &str) -> String {
    format!(
        "Unable to assemble the install descriptor for stack '{reference}'. Explore does not fall back to the latest AST. If you own this stack, run `a4 stack show {reference}` to inspect its deployment and publication state."
    )
}

fn install_ref_from_websocket_url(websocket_url: &str) -> Option<String> {
    url::Url::parse(websocket_url)
        .ok()?
        .host_str()?
        .split('.')
        .next()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn descriptor_identity(descriptor: &RegistryStackInstallResponse) -> StackDescriptorIdentity {
    StackDescriptorIdentity {
        name: descriptor.name.clone(),
        stack: descriptor.stack.clone(),
        visibility: descriptor.visibility.clone(),
        spec_version_id: descriptor.spec_version_id,
        ast_content_hash: descriptor.ast_content_hash.clone(),
        portable_ast_hash: descriptor.portable_ast_hash.clone(),
        stack_manifest_hash: descriptor.stack_manifest_hash.clone(),
        live_specs: descriptor
            .live_specs
            .iter()
            .map(|live| (live.alias.clone(), live.live_spec_hash.clone()))
            .collect(),
        programs: descriptor
            .programs
            .iter()
            .map(|program| {
                (
                    program.definition.program_id.clone(),
                    program.definition.program_spec_hash.clone(),
                    program.release.program_release_hash.clone(),
                )
            })
            .collect(),
    }
}

fn validate_stack_descriptor_identity(
    typescript: &RegistryStackInstallResponse,
    rust: &RegistryStackInstallResponse,
) -> Result<()> {
    if descriptor_identity(typescript) != descriptor_identity(rust) {
        anyhow::bail!(
            "Hosted stack returned different descriptor identities for TypeScript and Rust SDK targets"
        );
    }
    Ok(())
}

fn build_stack_output(
    typescript: &RegistryStackInstallResponse,
    rust: &RegistryStackInstallResponse,
) -> Result<StackExploreOutput> {
    let selected_views = selected_views(typescript)?;
    let rust_programs = rust
        .programs
        .iter()
        .map(|program| (program.definition.program_id.as_str(), program))
        .collect::<BTreeMap<_, _>>();
    let programs = typescript
        .programs
        .iter()
        .map(|program| {
            let rust_program = rust_programs
                .get(program.definition.program_id.as_str())
                .copied();
            stack_program_summary(program, rust_program)
        })
        .collect::<Result<Vec<_>>>()?;
    let program_reads = programs
        .iter()
        .map(|program| program.program_read.clone())
        .collect();
    let live_specs = typescript
        .live_specs
        .iter()
        .map(|live| LiveSpecSummary {
            alias: live.alias.clone(),
            live_spec_hash: live.live_spec_hash.clone(),
            deployment_id: live.binding.deployment_id,
            observed_generation: live.binding.observed_generation,
            websocket_endpoint: live.binding.websocket_endpoint.clone(),
            query_endpoint: live.binding.query_endpoint.clone(),
            websocket_auth_policy: live.binding.websocket_auth_policy.clone(),
            query_auth_policy: live.binding.query_auth_policy.clone(),
        })
        .collect::<Vec<_>>();
    let live_auth = live_specs
        .iter()
        .map(|live| LiveAuthSummary {
            alias: live.alias.clone(),
            websocket: live.websocket_auth_policy.clone(),
            query: live.query_auth_policy.clone(),
        })
        .collect();
    let chain = typescript
        .chain_binding
        .as_ref()
        .map(capability_summary)
        .transpose()?;
    let transaction = typescript
        .transaction_binding
        .as_ref()
        .map(capability_summary)
        .transpose()?;

    Ok(StackExploreOutput {
        schema_version: EXPLORE_SCHEMA_VERSION,
        kind: "stack",
        name: typescript.name.clone(),
        install_ref: typescript.stack.clone(),
        description: typescript.description.clone(),
        visibility: typescript.visibility.clone(),
        identity: StackIdentitySummary {
            stack_manifest_hash: typescript.stack_manifest_hash.clone(),
            ast_content_hash: typescript.ast_content_hash.clone(),
            portable_ast_hash: typescript.portable_ast_hash.clone(),
            spec_version_id: typescript.spec_version_id,
        },
        live_specs,
        selected_views,
        programs,
        sdk_targets: vec![
            sdk_target("typescript", typescript.extensions.as_ref())?,
            sdk_target("rust", rust.extensions.as_ref())?,
        ],
        authentication: AuthenticationSummary {
            websocket: typescript.websocket_auth.clone(),
            http: typescript.http_auth.clone(),
            live_specs: live_auth,
            chain: typescript
                .chain_binding
                .as_ref()
                .map(|binding| serde_json::to_value(&binding.auth))
                .transpose()?,
            transaction: typescript
                .transaction_binding
                .as_ref()
                .map(|binding| serde_json::to_value(&binding.auth))
                .transpose()?,
            program_reads,
        },
        chain,
        transaction,
        install_command: format!("a4 install {} --ts", typescript.stack),
    })
}

fn capability_summary(binding: &RegistryCapabilityInstallBinding) -> Result<CapabilitySummary> {
    Ok(CapabilitySummary {
        endpoint: binding.endpoint.clone(),
        auth_policy: binding.auth_policy.clone(),
        cluster: binding.cluster.clone(),
        region: binding.region.clone(),
        auth: serde_json::to_value(&binding.auth)?,
    })
}

fn stack_program_summary(
    program: &RegistryProgramInstallResponse,
    rust: Option<&RegistryProgramInstallResponse>,
) -> Result<StackProgramSummary> {
    if rust.is_some_and(|rust| {
        rust.definition.program_spec_hash != program.definition.program_spec_hash
            || rust.release.program_release_hash != program.release.program_release_hash
    }) {
        anyhow::bail!(
            "Hosted program '{}' changed identity between SDK targets",
            program.install_name
        );
    }
    Ok(StackProgramSummary {
        install_name: program.install_name.clone(),
        display_name: program.display_name.clone(),
        program_id: program.definition.program_id.clone(),
        program_spec_hash: program.definition.program_spec_hash.clone(),
        program_release_hash: program.release.program_release_hash.clone(),
        program_read: program_read_summary(program)?,
        sdk_targets: vec![
            sdk_target("typescript", program.definition.extensions.as_ref())?,
            sdk_target(
                "rust",
                rust.and_then(|program| program.definition.extensions.as_ref()),
            )?,
        ],
    })
}

fn sdk_target(
    language: &str,
    extension: Option<&RegistrySdkExtensionArtifact>,
) -> Result<SdkTargetSummary> {
    Ok(SdkTargetSummary {
        language: language.into(),
        extension: extension.map(extension_summary).transpose()?,
    })
}

fn extension_summary(extension: &RegistrySdkExtensionArtifact) -> Result<ExtensionSummary> {
    let input_kind = extension
        .manifest
        .input_kind
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?
        .and_then(|value| value.as_str().map(str::to_string));
    Ok(ExtensionSummary {
        artifact_hash: extension.artifact_hash.clone(),
        sdk_extension_hash: extension.sdk_extension_hash.clone(),
        sdk_output_tree_hash: extension.sdk_output_tree_hash.clone(),
        entry: extension.manifest.entry.clone(),
        files: extension.manifest.files.clone(),
        input_kind,
        input_hash: extension.manifest.input_hash.clone(),
        sdk_range: extension.manifest.sdk_range.clone(),
    })
}

fn program_read_summary(program: &RegistryProgramInstallResponse) -> Result<ProgramReadSummary> {
    let RegistryProgramInstallTransport::HostedBinding { binding } = &program.transport;
    Ok(ProgramReadSummary {
        available: true,
        endpoint: binding.endpoint.clone(),
        program_read_binding_id: binding.program_read_binding_id.clone(),
        auth: binding.auth.clone(),
    })
}

fn selected_views(descriptor: &RegistryStackInstallResponse) -> Result<Vec<SelectedViewSummary>> {
    let entries = descriptor
        .stack_manifest
        .pointer("/payload/selectedViews")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("Hosted StackManifest omitted selectedViews"))?;
    entries
        .iter()
        .map(|entry| {
            let view_id = entry.get("viewId").and_then(Value::as_str).ok_or_else(|| {
                anyhow::anyhow!("Hosted StackManifest has an invalid selected view")
            })?;
            let live_alias = entry
                .get("liveAlias")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    let hash = entry.get("liveSpecHash")?.as_str()?;
                    descriptor
                        .live_specs
                        .iter()
                        .find(|live| live.live_spec_hash == hash)
                        .map(|live| live.alias.clone())
                })
                .or_else(|| {
                    (descriptor.live_specs.len() == 1)
                        .then(|| descriptor.live_specs[0].alias.clone())
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("Selected view '{view_id}' has no LiveSpec alias")
                })?;
            let live = descriptor
                .live_specs
                .iter()
                .find(|live| live.alias == live_alias)
                .ok_or_else(|| {
                    anyhow::anyhow!("Selected view '{live_alias}:{view_id}' references no LiveSpec")
                })?;
            let (entity, view) = find_view(&live.artifact, view_id).ok_or_else(|| {
                anyhow::anyhow!(
                    "Selected view '{}:{}' is absent from its exact LiveSpec",
                    live_alias,
                    view_id
                )
            })?;
            Ok(SelectedViewSummary {
                live_alias,
                view_id: view_id.into(),
                entity: entity
                    .get("stateName")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .into(),
                source: view.get("source").cloned().unwrap_or(Value::Null),
                output: view.get("output").cloned().unwrap_or(Value::Null),
                pipeline_steps: view
                    .get("pipeline")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len),
            })
        })
        .collect()
}

fn find_view<'a>(artifact: &'a Value, view_id: &str) -> Option<(&'a Value, &'a Value)> {
    artifact
        .pointer("/payload/entities")?
        .as_array()?
        .iter()
        .find_map(|entity| {
            entity
                .get("views")?
                .as_array()?
                .iter()
                .find(|view| view.get("id").and_then(Value::as_str) == Some(view_id))
                .map(|view| (entity, view))
        })
}

fn build_program_output(
    descriptor: &RegistryProgramInstallResponse,
) -> Result<ProgramExploreOutput> {
    let idl = &descriptor.definition.idl_payload;
    Ok(ProgramExploreOutput {
        schema_version: EXPLORE_SCHEMA_VERSION,
        kind: "program",
        install_name: descriptor.install_name.clone(),
        display_name: descriptor.display_name.clone(),
        identity: ProgramIdentitySummary {
            program_id: descriptor.definition.program_id.clone(),
            program_spec_hash: descriptor.definition.program_spec_hash.clone(),
            program_release_hash: descriptor.release.program_release_hash.clone(),
            idl_content_hash: descriptor.definition.idl_content_hash.clone(),
            normalized_idl_hash: descriptor.definition.normalized_idl_hash.clone(),
        },
        accounts: value_array(idl, "accounts")
            .iter()
            .map(account_summary)
            .collect(),
        instructions: value_array(idl, "instructions")
            .iter()
            .map(instruction_summary)
            .collect(),
        events: value_array(idl, "events")
            .iter()
            .map(event_summary)
            .collect(),
        types: value_array(idl, "types").iter().map(type_summary).collect(),
        program_read: program_read_summary(descriptor)?,
        sdk_targets: vec![sdk_target(
            "typescript",
            descriptor.definition.extensions.as_ref(),
        )?],
        install_command: format!("a4 install program {} --ts", descriptor.install_name),
    })
}

fn value_array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn named_types(value: &Value, key: &str) -> Vec<NamedTypeSummary> {
    value_array(value, key)
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
    value
        .get("discriminator")
        .or_else(|| value.get("discriminant"))
        .filter(|value| !value.is_null())
        .cloned()
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
        name: account
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .into(),
        discriminator: discriminator(account),
        fields,
    }
}

fn instruction_summary(instruction: &Value) -> InstructionSummary {
    let mut accounts = Vec::new();
    summarize_instruction_accounts(value_array(instruction, "accounts"), "", &mut accounts);
    InstructionSummary {
        name: instruction
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .into(),
        discriminator: discriminator(instruction),
        arguments: named_types(instruction, "args"),
        accounts,
    }
}

fn summarize_instruction_accounts(
    values: &[Value],
    prefix: &str,
    output: &mut Vec<InstructionAccountSummary>,
) {
    for account in values {
        let name = account
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let qualified = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        };
        if let Some(children) = account.get("accounts").and_then(Value::as_array) {
            summarize_instruction_accounts(children, &qualified, output);
            continue;
        }
        output.push(InstructionAccountSummary {
            name: qualified,
            writable: first_bool(account, &["writable", "isMut", "isWritable", "is_writable"]),
            signer: first_bool(account, &["signer", "isSigner", "is_signer"]),
            optional: first_bool(account, &["optional", "isOptional", "is_optional"]),
        });
    }
}

fn first_bool(value: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .find_map(|key| value.get(key).and_then(Value::as_bool))
        .unwrap_or(false)
}

fn event_summary(event: &Value) -> EventSummary {
    let fields = if event.get("fields").is_some() {
        named_types(event, "fields")
    } else {
        event
            .get("type")
            .map(|definition| named_types(definition, "fields"))
            .unwrap_or_default()
    };
    EventSummary {
        name: event
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .into(),
        discriminator: discriminator(event),
        fields,
    }
}

fn type_summary(user_type: &Value) -> TypeSummary {
    let definition = user_type.get("type").unwrap_or(user_type);
    TypeSummary {
        name: user_type
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .into(),
        kind: definition
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .into(),
        fields: named_types(definition, "fields"),
        variants: value_array(definition, "variants")
            .iter()
            .filter_map(|variant| variant.get("name").and_then(Value::as_str))
            .map(str::to_string)
            .collect(),
    }
}

fn build_entity_output(
    descriptor: &RegistryStackInstallResponse,
    query: &str,
) -> Result<StackEntityExploreOutput> {
    let selected = selected_views(descriptor)?;
    let (requested_alias, requested_name) = query
        .split_once(':')
        .map_or((None, query), |(alias, name)| (Some(alias), name));
    let mut matches = Vec::new();
    for live in &descriptor.live_specs {
        if requested_alias.is_some_and(|alias| !live.alias.eq_ignore_ascii_case(alias)) {
            continue;
        }
        for entity in live
            .artifact
            .pointer("/payload/entities")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if entity
                .get("stateName")
                .and_then(Value::as_str)
                .is_some_and(|name| name.eq_ignore_ascii_case(requested_name))
            {
                matches.push((live.alias.as_str(), entity));
            }
        }
    }
    if matches.is_empty() {
        let available = descriptor
            .live_specs
            .iter()
            .flat_map(|live| {
                live.artifact
                    .pointer("/payload/entities")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|entity| entity.get("stateName").and_then(Value::as_str))
                    .map(|name| format!("{}:{name}", live.alias))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        anyhow::bail!(
            "Entity '{}' not found in stack '{}'. Available entities: {}",
            query,
            descriptor.stack,
            available.join(", ")
        );
    }
    if matches.len() > 1 {
        anyhow::bail!(
            "Entity '{}' exists under multiple LiveSpec aliases; use alias:entity (matches: {})",
            query,
            matches
                .iter()
                .map(|(alias, _)| *alias)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let (live_alias, entity) = matches[0];
    let primary_keys = entity
        .pointer("/identity/primaryKeys")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    let mut fields = Vec::new();
    for section in value_array(entity, "sections") {
        let section_name = section
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("fields");
        for field in value_array(section, "fields") {
            if field.get("emit").and_then(Value::as_bool) == Some(false) {
                continue;
            }
            let name = field
                .get("fieldName")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            fields.push(EntityFieldSummary {
                section: section_name.into(),
                path: format!("{section_name}.{name}"),
                rust_type: field
                    .get("rustTypeName")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .into(),
                nullable: field
                    .get("isOptional")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
        }
    }
    let views = selected
        .into_iter()
        .filter(|view| {
            view.live_alias == live_alias
                && view.entity.eq_ignore_ascii_case(
                    entity
                        .get("stateName")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )
        })
        .collect();
    Ok(StackEntityExploreOutput {
        schema_version: EXPLORE_SCHEMA_VERSION,
        kind: "stack-entity",
        stack: descriptor.stack.clone(),
        identity: StackIdentitySummary {
            stack_manifest_hash: descriptor.stack_manifest_hash.clone(),
            ast_content_hash: descriptor.ast_content_hash.clone(),
            portable_ast_hash: descriptor.portable_ast_hash.clone(),
            spec_version_id: descriptor.spec_version_id,
        },
        live_alias: live_alias.into(),
        name: entity
            .get("stateName")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .into(),
        program_id: entity
            .get("programId")
            .and_then(Value::as_str)
            .map(str::to_string),
        primary_keys,
        fields,
        views,
    })
}

fn render_stack(output: &StackExploreOutput) -> String {
    let mut text = format!(
        "\nStack: {}\n  Install reference: {}\n  Visibility: {}\n",
        output.name, output.install_ref, output.visibility
    );
    if let Some(description) = &output.description {
        text.push_str(&format!("  Description: {description}\n"));
    }
    text.push_str(&format!(
        "\nIdentities\n  StackManifest: {}\n  AST content: {}\n  Portable AST: {}\n",
        output.identity.stack_manifest_hash,
        output.identity.ast_content_hash,
        output.identity.portable_ast_hash
    ));
    text.push_str("\nLiveSpecs\n");
    for live in &output.live_specs {
        text.push_str(&format!(
            "  {}  {}\n    WebSocket: {} ({})\n    Query: {} ({})\n",
            live.alias,
            live.live_spec_hash,
            live.websocket_endpoint,
            live.websocket_auth_policy,
            live.query_endpoint,
            live.query_auth_policy
        ));
    }
    text.push_str("\nSelected views\n");
    if output.selected_views.is_empty() {
        text.push_str("  none\n");
    } else {
        for view in &output.selected_views {
            text.push_str(&format!(
                "  {}:{}  (entity {}, {} pipeline step(s))\n",
                view.live_alias, view.view_id, view.entity, view.pipeline_steps
            ));
        }
    }
    text.push_str("\nPrograms\n");
    if output.programs.is_empty() {
        text.push_str("  none\n");
    } else {
        for program in &output.programs {
            text.push_str(&format!(
                "  {}  {}\n    Program ID: {}\n    Program Release: {}\n    Program Read: {}\n",
                program.install_name,
                program.display_name,
                program.program_id,
                program.program_release_hash,
                program.program_read.endpoint
            ));
            text.push_str(&format!(
                "    SDK targets: {}\n",
                render_sdk_targets(&program.sdk_targets)
            ));
        }
    }
    text.push_str("\nSDK targets\n");
    for target in &output.sdk_targets {
        text.push_str(&format!(
            "  {}  extensions: {}\n",
            target.language,
            target
                .extension
                .as_ref()
                .map(|extension| extension.artifact_hash.as_str())
                .unwrap_or("none")
        ));
    }
    text.push_str("\nAuthentication\n");
    for live in &output.authentication.live_specs {
        text.push_str(&format!(
            "  LiveSpec {}: websocket={}, query={}\n",
            live.alias, live.websocket, live.query
        ));
    }
    if let Some(chain) = &output.chain {
        text.push_str(&format!(
            "  Chain: {} ({})\n",
            chain.endpoint,
            auth_requirement(&chain.auth)
        ));
    }
    if let Some(transaction) = &output.transaction {
        text.push_str(&format!(
            "  Transaction: {} ({})\n",
            transaction.endpoint,
            auth_requirement(&transaction.auth)
        ));
    }
    for program in &output.programs {
        text.push_str(&format!(
            "  Program Read {}: {}\n",
            program.install_name,
            auth_requirement(&program.program_read.auth)
        ));
    }
    text.push_str(&format!("\nInstall\n  {}\n", output.install_command));
    text
}

fn render_program(output: &ProgramExploreOutput) -> String {
    let mut text = format!(
        "\nProgram: {} ({})\n  Program ID: {}\n  ProgramSpec: {}\n  Program Release: {}\n",
        output.display_name,
        output.install_name,
        output.identity.program_id,
        output.identity.program_spec_hash,
        output.identity.program_release_hash
    );
    text.push_str("\nAccounts\n");
    if output.accounts.is_empty() {
        text.push_str("  none\n");
    }
    for account in &output.accounts {
        text.push_str(&format!(
            "  {}  discriminator: {}\n",
            account.name,
            account
                .discriminator
                .as_ref()
                .map(compact_json)
                .unwrap_or_else(|| "none".into())
        ));
        for field in &account.fields {
            text.push_str(&format!(
                "    {}: {}\n",
                field.name,
                compact_json(&field.field_type)
            ));
        }
    }
    text.push_str("\nInstructions\n");
    if output.instructions.is_empty() {
        text.push_str("  none\n");
    }
    for instruction in &output.instructions {
        let arguments = instruction
            .arguments
            .iter()
            .map(|argument| format!("{}: {}", argument.name, compact_json(&argument.field_type)))
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&format!("  {}({})\n", instruction.name, arguments));
        if !instruction.accounts.is_empty() {
            text.push_str(&format!(
                "    Accounts: {}\n",
                instruction
                    .accounts
                    .iter()
                    .map(|account| {
                        let mut flags = Vec::new();
                        if account.writable {
                            flags.push("writable");
                        }
                        if account.signer {
                            flags.push("signer");
                        }
                        if account.optional {
                            flags.push("optional");
                        }
                        if flags.is_empty() {
                            account.name.clone()
                        } else {
                            format!("{} [{}]", account.name, flags.join(", "))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    text.push_str(&format!(
        "\nEvents\n  {}\n",
        names_or_none(&output.events, |v| &v.name)
    ));
    text.push_str(&format!(
        "\nTypes\n  {}\n",
        names_or_none(&output.types, |v| &v.name)
    ));
    text.push_str(&format!(
        "\nProgram Read\n  {}\n  Auth: {}\n\nSDK targets\n  {}\n\nInstall\n  {}\n",
        output.program_read.endpoint,
        auth_requirement(&output.program_read.auth),
        render_sdk_targets(&output.sdk_targets),
        output.install_command
    ));
    text
}

fn names_or_none<'a, T>(values: &'a [T], name: impl Fn(&'a T) -> &'a str) -> String {
    if values.is_empty() {
        "none".into()
    } else {
        values.iter().map(name).collect::<Vec<_>>().join(", ")
    }
}

fn render_entity(output: &StackEntityExploreOutput) -> String {
    let mut text = format!(
        "\nEntity: {}\n  Stack: {}\n  LiveSpec alias: {}\n  Primary key: {}\n",
        output.name,
        output.stack,
        output.live_alias,
        output.primary_keys.join(", ")
    );
    text.push_str("\nFields\n");
    for field in &output.fields {
        text.push_str(&format!(
            "  {}  {}{}\n",
            field.path,
            field.rust_type,
            if field.nullable { "?" } else { "" }
        ));
    }
    text.push_str("\nSelected views\n");
    if output.views.is_empty() {
        text.push_str("  none\n");
    } else {
        for view in &output.views {
            text.push_str(&format!("  {}:{}\n", view.live_alias, view.view_id));
        }
    }
    text
}

fn compact_json(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

fn auth_requirement(auth: &Value) -> String {
    let required =
        auth.get("required")
            .and_then(Value::as_bool)
            .map_or("unspecified", |required| {
                if required {
                    "required"
                } else {
                    "not required"
                }
            });
    let mode = auth
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    format!("{required}, mode={mode}")
}

fn render_sdk_targets(targets: &[SdkTargetSummary]) -> String {
    targets
        .iter()
        .map(|target| {
            target.extension.as_ref().map_or_else(
                || target.language.clone(),
                |extension| {
                    format!(
                        "{} (extension {})",
                        target.language, extension.artifact_hash
                    )
                },
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stack_descriptor() -> RegistryStackInstallResponse {
        serde_json::from_value(json!({
            "name": "Multi",
            "stack": "multi-stack",
            "description": "two lives",
            "visibility": "public",
            "specVersionId": 7,
            "astContentHash": "ast-exact",
            "portableAstHash": "portable-exact",
            "astPayload": {},
            "liveSpecs": [
                {
                    "alias": "primary",
                    "liveSpecHash": "live-primary",
                    "artifact": {
                        "payload": {"entities": [{
                            "stateName": "Position",
                            "programId": "Program111",
                            "identity": {"primaryKeys": ["id.address"]},
                            "sections": [{"name": "id", "fields": [{
                                "fieldName": "address", "rustTypeName": "Pubkey",
                                "baseType": "Pubkey", "isOptional": false, "isArray": false
                            }]}],
                            "views": [{"id": "Position/state", "source": {"Entity": {"name": "Position"}}, "pipeline": [], "output": "Collection"}]
                        }]}
                    },
                    "binding": {
                        "deploymentId": 11,
                        "websocketEndpoint": "wss://primary.test",
                        "queryEndpoint": "https://primary.test",
                        "websocketAuthPolicy": "signed_session",
                        "queryAuthPolicy": "signed_session",
                        "observedGeneration": 3
                    }
                },
                {
                    "alias": "history",
                    "liveSpecHash": "live-history",
                    "artifact": {"payload": {"entities": [{
                        "stateName": "Position",
                        "identity": {"primaryKeys": ["id.address"]},
                        "sections": [],
                        "views": [{"id": "Position/list", "source": {"Entity": {"name": "Position"}}, "pipeline": [{}], "output": "Collection"}]
                    }]}},
                    "binding": {
                        "deploymentId": 12,
                        "websocketEndpoint": "wss://history.test",
                        "queryEndpoint": "https://history.test",
                        "websocketAuthPolicy": "signed_session",
                        "queryAuthPolicy": "signed_session",
                        "observedGeneration": 4
                    }
                }
            ],
            "stackManifestHash": "manifest-exact",
            "stackManifest": {"payload": {"selectedViews": [
                {"liveAlias": "primary", "viewId": "Position/state"},
                {"liveAlias": "history", "viewId": "Position/list"}
            ]}},
            "chainBinding": null,
            "transactionBinding": null,
            "extensions": null,
            "programs": []
        }))
        .unwrap()
    }

    fn program_descriptor() -> RegistryProgramInstallResponse {
        serde_json::from_value(json!({
            "installName": "demo",
            "displayName": "Demo",
            "definition": {
                "programId": "Demo111",
                "programSpecHash": "program-spec-exact",
                "idlContentHash": "idl-exact",
                "normalizedIdlHash": "normalized-exact",
                "idlPayload": {
                    "accounts": [{"name": "Vault", "discriminator": [1,2], "fields": [{"name": "amount", "type": "u64"}]}],
                    "instructions": [{"name": "setValue", "discriminator": [3,4], "args": [{"name": "value", "type": "u64"}], "accounts": [{"name": "vault", "isMut": true}, {"name": "authority", "isSigner": true}]}],
                    "events": [{"name": "ValueSet", "fields": [{"name": "value", "type": "u64"}]}],
                    "types": [{"name": "Mode", "type": {"kind": "enum", "variants": [{"name": "On"}, {"name": "Off"}]}}]
                },
                "programSpec": {},
                "extensions": null
            },
            "release": {"programReleaseHash": "release-exact", "programSpecHash": "program-spec-exact"},
            "transport": {"kind": "hosted-binding", "binding": {"endpoint": "https://read.test", "programReadBindingId": "prb_demo", "auth": {"required": true}}}
        }))
        .unwrap()
    }

    #[test]
    fn stack_explore_preserves_descriptor_identities_aliases_and_selected_views() {
        let typescript = stack_descriptor();
        let rust = stack_descriptor();
        let output = build_stack_output(&typescript, &rust).unwrap();
        assert_eq!(output.schema_version, 1);
        assert_eq!(output.identity.stack_manifest_hash, "manifest-exact");
        assert_eq!(output.identity.ast_content_hash, "ast-exact");
        assert_eq!(
            output
                .live_specs
                .iter()
                .map(|live| live.alias.as_str())
                .collect::<Vec<_>>(),
            vec!["primary", "history"]
        );
        assert_eq!(
            output
                .selected_views
                .iter()
                .map(|view| (view.live_alias.as_str(), view.view_id.as_str()))
                .collect::<Vec<_>>(),
            vec![("primary", "Position/state"), ("history", "Position/list")]
        );
        let terminal = render_stack(&output);
        assert!(terminal.contains("StackManifest: manifest-exact"));
        assert!(terminal.contains("primary:Position/state"));
    }

    #[test]
    fn target_specific_descriptors_must_keep_the_same_install_identity() {
        let typescript = stack_descriptor();
        let mut rust = stack_descriptor();
        rust.stack_manifest_hash = "drifted".into();
        assert!(validate_stack_descriptor_identity(&typescript, &rust).is_err());
    }

    #[test]
    fn single_live_stack_keeps_the_install_manifest_and_live_identity() {
        let mut typescript = stack_descriptor();
        typescript.live_specs.truncate(1);
        typescript.stack_manifest["payload"]["selectedViews"] = json!([
            {"liveAlias": "primary", "viewId": "Position/state"}
        ]);
        let rust = typescript.clone();
        let output = build_stack_output(&typescript, &rust).unwrap();
        assert_eq!(output.identity.stack_manifest_hash, "manifest-exact");
        assert_eq!(output.live_specs.len(), 1);
        assert_eq!(output.live_specs[0].live_spec_hash, "live-primary");
        assert_eq!(output.selected_views.len(), 1);
    }

    #[test]
    fn legacy_entity_drilldown_uses_exact_live_spec_and_selected_views() {
        let descriptor = stack_descriptor();
        assert!(build_entity_output(&descriptor, "Position").is_err());
        let output = build_entity_output(&descriptor, "primary:Position").unwrap();
        assert_eq!(output.identity.stack_manifest_hash, "manifest-exact");
        assert_eq!(output.primary_keys, vec!["id.address"]);
        assert_eq!(output.views.len(), 1);
        assert_eq!(output.views[0].view_id, "Position/state");
    }

    #[test]
    fn program_explore_is_bounded_but_complete_for_public_surface() {
        let output = build_program_output(&program_descriptor()).unwrap();
        assert_eq!(output.schema_version, 1);
        assert_eq!(output.identity.program_release_hash, "release-exact");
        assert_eq!(output.accounts[0].fields[0].name, "amount");
        assert_eq!(output.instructions[0].arguments[0].name, "value");
        assert!(output.instructions[0].accounts[0].writable);
        assert_eq!(output.events[0].name, "ValueSet");
        assert_eq!(output.types[0].variants, vec!["On", "Off"]);
        let json = serde_json::to_value(&output).unwrap();
        assert!(json["definition"].is_null());
        assert!(json["accounts"][0].get("docs").is_none());
        assert!(render_program(&output).contains("a4 install program demo --ts"));
    }

    #[test]
    fn legacy_stack_name_can_be_translated_from_listing_url() {
        assert_eq!(
            install_ref_from_websocket_url("wss://ore-stack-abc.stack.arete.run/ws").as_deref(),
            Some("ore-stack-abc")
        );
    }

    #[test]
    fn list_and_descriptor_diagnostic_json_contracts_are_stable() {
        let list = ExploreProgramListOutput {
            schema_version: 1,
            programs: vec![RegistryProgramItem {
                install_name: "demo".into(),
                display_name: "Demo".into(),
                program_id: "Demo111".into(),
                program_release_hash: "release-exact".into(),
                program_spec_hash: "program-spec-exact".into(),
                sdk_targets: vec!["typescript".into()],
            }],
        };
        assert_eq!(
            serde_json::to_value(list).unwrap(),
            json!({
                "schemaVersion": 1,
                "programs": [{
                    "installName": "demo",
                    "displayName": "Demo",
                    "programId": "Demo111",
                    "programReleaseHash": "release-exact",
                    "programSpecHash": "program-spec-exact",
                    "sdkTargets": ["typescript"]
                }]
            })
        );
        let diagnostic = descriptor_diagnostic("demo-stack");
        assert!(diagnostic.contains("does not fall back to the latest AST"));
        assert!(diagnostic.contains("a4 stack show demo-stack"));
    }
}
