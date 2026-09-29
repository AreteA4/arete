use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{Context, Result};
use arete_mcp::descriptor::{
    self as shape, AccountSummary, EntityField, ErrorSummary, EventSummary, InstructionSummary,
    ProgramSurface, TypeSummary,
};
use arete_mcp::stack_knowledge::{KeyCase, StackKnowledge};
use colored::Colorize;
use serde::Serialize;
use serde_json::{json, Value};

use crate::api_client::{
    ApiClient, DeploymentResponse, RegistryCapabilityInstallBinding,
    RegistryProgramInstallResponse, RegistryProgramInstallTransport, RegistryProgramItem,
    RegistrySdkExtensionArtifact, RegistryStackInstallResponse, RegistryStackItem,
    CAPABILITY_TRANSACTION_INSPECT, CAPABILITY_TRANSACTION_SEND, DEFAULT_DOMAIN_SUFFIX,
};
use crate::commands::stack::deployment_selection_key;
use crate::project::manifest::{DependencySourceV1, DependencyV1, ProjectManifest};

const EXPLORE_SCHEMA_VERSION: u32 = 1;
const DEPLOYMENT_PAGE_SIZE: i64 = 100;
const MAX_DEPLOYMENT_PAGES: i64 = 100;

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
    /// The view's summary from the stack's catalog knowledge.
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    extension_api: Option<std::num::NonZeroU32>,
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
    service_class: String,
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
    /// Accepted key classes, scopes, browser-key origin binding and whether a
    /// transaction entitlement is required, per surface.
    auth_requirements: Value,
    /// Which stream endpoints the generated SDK reads: arete.toml endpoints
    /// recorded by `a4 up`, or the registry's hosted ones.
    sdk_endpoints: Value,
    /// Whether the logged-in account meets the transaction requirement.
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<Value>,
    install_command: String,
    /// The stack's catalog knowledge: its document, and each described
    /// entity's summary and field descriptions.
    #[serde(skip_serializing_if = "Option::is_none")]
    knowledge: Option<Value>,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StackEntityExploreOutput {
    schema_version: u32,
    kind: &'static str,
    stack: String,
    identity: StackIdentitySummary,
    live_alias: String,
    name: String,
    /// The entity's summary from the stack's catalog knowledge.
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    program_id: Option<String>,
    primary_keys: Vec<String>,
    fields: Vec<EntityField>,
    views: Vec<SelectedViewSummary>,
    /// The catalog knowledge document the descriptions come from.
    #[serde(skip_serializing_if = "Option::is_none")]
    knowledge: Option<Value>,
}

/// Options for `a4 explore stack <ref>`.
#[derive(Debug, Default)]
pub struct StackOptions<'a> {
    /// Legacy entity drill-down.
    pub entity: Option<&'a str>,
    /// Compact summary instead of the full exploration.
    pub summary: bool,
    /// Only these selected views, with their entity schemas.
    pub views: Vec<String>,
    /// One operation from the stack's program SDKs.
    pub operation: Option<&'a str>,
    /// arete.toml, to report the endpoints a project's SDK uses.
    pub config_path: &'a str,
}

/// Options for `a4 explore program <ref>`.
#[derive(Debug, Default)]
pub struct ProgramOptions<'a> {
    /// One operation.
    pub operation: Option<&'a str>,
    /// Only these sections (see `shape::PROGRAM_SECTIONS`).
    pub sections: Vec<String>,
}

/// A registry stack this project depends on (`[dependencies.stacks]`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectStack {
    alias: String,
    package: String,
    /// arete.toml `endpoints` recorded by `a4 up`: (LiveSpec alias, WebSocket, query).
    endpoints: Vec<(String, String, String)>,
}

#[derive(Debug, PartialEq, Eq)]
struct StackDescriptorIdentity {
    name: String,
    stack: String,
    visibility: String,
    service_class: String,
    spec_version_id: Option<i32>,
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
            println!("    Service class: {}", stack.service_class);
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

pub fn show_stack(reference: &str, options: StackOptions<'_>, json: bool) -> Result<()> {
    let client = ApiClient::new()?;
    // Inside a project, `reference` may be a dependency alias; look the stack
    // up by its registry package.
    let project = project_stack(options.config_path, reference);
    let lookup = project
        .as_ref()
        .map_or(reference, |project| project.package.as_str());

    if let Some(entity) = options.entity {
        let (_, typescript, _) = resolve_stack_descriptors(&client, lookup)?;
        let knowledge = stack_knowledge(&client, &typescript);
        let output = build_entity_output(&typescript, entity, knowledge.as_ref())?;
        if json {
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            print!("{}", render_entity(&output));
        }
        return Ok(());
    }

    let compact = options.summary || !options.views.is_empty() || options.operation.is_some();
    if !compact {
        let (install_ref, typescript, rust) = resolve_stack_descriptors(&client, lookup)?;
        let knowledge = stack_knowledge(&client, &typescript);
        let mut output = build_stack_output(&install_ref, &typescript, &rust, knowledge.as_ref())?;
        output.sdk_endpoints = sdk_endpoints(&typescript, project.as_ref());
        output.account = account_readiness(&client, &output.auth_requirements);
        if json {
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            print!("{}", render_stack(&output));
        }
        return Ok(());
    }

    // The compact forms are cut from the TypeScript descriptor alone.
    let (install_ref, typescript) = resolve_stack_descriptor(&client, lookup, None)?;
    let stack = serde_json::to_value(&typescript)?;
    if let Some(operation) = options.operation {
        return show_stack_operation(&client, &install_ref, &stack, operation, json);
    }
    let knowledge = stack_knowledge(&client, &typescript);
    let mut output = if options.views.is_empty() {
        let mut summary = shape::stack_summary(&stack, knowledge.as_ref());
        if let Some(account) = account_readiness(&client, &summary["auth"]) {
            summary["account"] = account;
        }
        summary["installCommand"] = json!(format!("a4 install stack {install_ref} --ts"));
        summary
    } else {
        shape::stack_views(&stack, &options.views, knowledge.as_ref())?
    };
    output["schemaVersion"] = json!(EXPLORE_SCHEMA_VERSION);
    output["installRef"] = json!(install_ref);
    output["sdkEndpoints"] = sdk_endpoints(&typescript, project.as_ref());
    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else if options.views.is_empty() {
        print!("{}", render_stack_summary(&output));
    } else {
        print!("{}", render_stack_views(&output));
    }
    Ok(())
}

pub fn show_program(reference: &str, options: ProgramOptions<'_>, json: bool) -> Result<()> {
    let sections = shape::parse_sections(&options.sections, "--section")?;
    let client = ApiClient::new()?;
    let descriptor = client
        .get_registry_program_install(reference, None)
        .with_context(|| {
            format!(
                "Unable to assemble the install descriptor for program '{reference}'. Explore does not fall back to raw or latest IDL artifacts; verify that a promoted Program Release and healthy Program Read binding exist."
            )
        })?;
    if options.operation.is_none() && sections.is_empty() {
        let output = build_program_output(&descriptor)?;
        if json {
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            print!("{}", render_program(&output));
        }
        return Ok(());
    }

    let program = serde_json::to_value(&descriptor)?;
    let surface = if options.operation.is_some() || sections.iter().any(|s| s == "operations") {
        program_surface(&client, &program)
    } else {
        Err("not requested".to_string())
    };
    let state = surface.as_ref().map_err(String::as_str);
    if let Some(operation) = options.operation {
        let mut output = shape::program_operation(&program, state, operation)?;
        output["schemaVersion"] = json!(EXPLORE_SCHEMA_VERSION);
        add_operation_account(&client, &mut output);
        return print_operation(&output, json);
    }
    let mut output = shape::program_sections(&program, state, &sections);
    output["schemaVersion"] = json!(EXPLORE_SCHEMA_VERSION);
    output["installCommand"] = json!(format!(
        "a4 install program {} --ts",
        descriptor.install_name
    ));
    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_program_sections(&output));
    }
    Ok(())
}

/// One operation from a stack's program SDKs: each program is searched with
/// the stack's transaction binding, and the operation must be unique.
fn show_stack_operation(
    client: &ApiClient,
    install_ref: &str,
    stack: &Value,
    id: &str,
    json: bool,
) -> Result<()> {
    let programs = value_array(stack, "programs");
    let names = programs
        .iter()
        .filter_map(|program| program.get("installName").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let mut found = Vec::new();
    let mut unavailable = None;
    for program in programs {
        let program = shape::program_in_stack(stack, program);
        let surface = program_surface(client, &program);
        if let Err(reason) = &surface {
            unavailable.get_or_insert_with(|| reason.clone());
        }
        let state = surface.as_ref().map_err(String::as_str);
        if let Some(operation) = shape::find_program_operation(&program, state, id)? {
            found.push(operation);
        }
    }
    let mut output = match found.len() {
        1 => found.remove(0),
        0 => {
            let mut message = format!(
                "No operation `{id}` in the program SDKs of stack `{install_ref}` ({}).",
                if names.is_empty() {
                    "it carries none".to_string()
                } else {
                    names.join(", ")
                }
            );
            if let Some(reason) = unavailable {
                message.push_str(&format!(" Semantic operations are unavailable: {reason}."));
            }
            anyhow::bail!(message)
        }
        _ => anyhow::bail!(
            "Operation `{id}` matches in several program SDKs of stack `{install_ref}` ({}); explore one program with `a4 explore program <ref> --operation {id}`",
            names.join(", ")
        ),
    };
    output["schemaVersion"] = json!(EXPLORE_SCHEMA_VERSION);
    output["stack"] = json!(install_ref);
    add_operation_account(client, &mut output);
    print_operation(&output, json)
}

fn print_operation(output: &Value, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(output)?);
    } else {
        print!("{}", render_operation(output));
    }
    Ok(())
}

/// The knowledge surface for a program descriptor, or why it is unavailable.
fn program_surface(
    client: &ApiClient,
    program: &Value,
) -> std::result::Result<ProgramSurface, String> {
    if !client.has_api_key() {
        return Err(
            "semantic operations come from the knowledge layer, which needs an API key: run `a4 auth login` (or `a4 auth signup`)"
                .into(),
        );
    }
    let slug = program
        .get("installName")
        .and_then(Value::as_str)
        .ok_or("the descriptor names no install name")?;
    let slug = catalog_slug(slug).map_err(|error| error.to_string())?;
    let program_id = program
        .pointer("/definition/programId")
        .and_then(Value::as_str)
        .ok_or("the descriptor names no program id")?;
    let response = client
        .knowledge_program(&slug, Some("surface"))
        .map_err(|error| format!("{error:#}"))?;
    ProgramSurface::from_knowledge(&response, program_id).map_err(|error| error.to_string())
}

/// Report whether the logged-in account can use an operation's transport.
fn add_operation_account(client: &ApiClient, output: &mut Value) {
    if let Some(account) = account_readiness(client, &output["operation"]["transport"]["auth"]) {
        output["account"] = account;
    }
}

/// Whether the logged-in account has the transaction access these auth
/// requirements call for. `None` when no transaction entitlement is required.
fn account_readiness(client: &ApiClient, requirements: &Value) -> Option<Value> {
    if requirements
        .get("transactionEntitlementRequired")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return None;
    }
    let transactions = if !client.has_api_key() {
        json!({
            "status": "unknown",
            "detail": "not logged in; run `a4 auth login` (or `a4 auth signup`) to check transaction access",
        })
    } else {
        match client.account_capabilities() {
            Ok(account) => {
                let missing =
                    account.missing(&[CAPABILITY_TRANSACTION_INSPECT, CAPABILITY_TRANSACTION_SEND]);
                if missing.is_empty() {
                    json!({ "status": "ready" })
                } else {
                    json!({
                        "status": "not-ready",
                        "missing": missing,
                        "detail": "your account doesn't have transaction access yet; reads and subscriptions are unaffected",
                    })
                }
            }
            Err(error) => json!({
                "status": "unknown",
                "detail": crate::commands::doctor::account_unavailable_reason(&error),
            }),
        }
    };
    Some(json!({ "transactions": transactions }))
}

/// The curated knowledge of a stack published in the catalog: entity and
/// view summaries and field descriptions. A catalog stack's descriptor is
/// named after its package slug, which is what the knowledge route takes;
/// the knowledge document's own slug may differ. `None`, silently, for a
/// stack without a catalog entry or document, a registry that does not
/// serve the route, knowledge published for another StackManifest, or any
/// failure: it only ever adds to the output. A stack that cannot have
/// catalog knowledge (a private stack, or one not named by a package slug)
/// is not looked up, and the lookup is abandoned after
/// [`arete_mcp::stack_knowledge::LOOKUP_TIMEOUT`].
fn stack_knowledge(
    client: &ApiClient,
    descriptor: &RegistryStackInstallResponse,
) -> Option<StackKnowledge> {
    if !arete_mcp::stack_knowledge::may_have_catalog_knowledge(
        &descriptor.name,
        Some(&descriptor.visibility),
    ) {
        return None;
    }
    let slug = catalog_slug(&descriptor.name).ok()?;
    let response = client.catalog_entry_knowledge("stack", &slug)?;
    StackKnowledge::from_response(&response)
        .filter(|knowledge| knowledge.belongs_to(&descriptor.stack_manifest_hash))
}

/// The full output's `knowledge`: the document, and every LiveSpec entity it
/// describes with its summary and described fields (view summaries are on
/// `selectedViews`).
fn full_stack_knowledge(
    descriptor: &RegistryStackInstallResponse,
    knowledge: &StackKnowledge,
) -> Value {
    let mut entities = Vec::new();
    for live in &descriptor.live_specs {
        for entity in shape::live_entities(&live.artifact) {
            let Some(name) = shape::entity_name(entity) else {
                continue;
            };
            let Some(entity_knowledge) = knowledge.entity(name) else {
                continue;
            };
            let fields = shape::entity_fields(entity);
            let mut item = json!({ "liveAlias": live.alias, "name": name });
            if let Some(summary) = &entity_knowledge.summary {
                item["summary"] = json!(summary);
            }
            let described =
                entity_knowledge.field_descriptions(fields.iter().map(|field| field.path.as_str()));
            if !described.is_empty() {
                item["fieldDescriptions"] = Value::Array(described);
            }
            entities.push(item);
        }
    }
    let mut out = knowledge.source(KeyCase::Camel);
    out["entities"] = Value::Array(entities);
    out
}

/// The stack dependency in arete.toml that `reference` names, by alias or by
/// registry package. Explore never fails because of the project manifest: an
/// absent or invalid arete.toml means there is no project context.
fn project_stack(config_path: &str, reference: &str) -> Option<ProjectStack> {
    let path = Path::new(config_path);
    if !path.is_file() {
        return None;
    }
    let manifest = ProjectManifest::load(path).ok()?;
    project_stack_in(&manifest.document.dependencies.stacks, reference)
}

fn project_stack_in(
    stacks: &BTreeMap<String, DependencyV1>,
    reference: &str,
) -> Option<ProjectStack> {
    let registry = |dependency: &DependencyV1| match &dependency.source {
        DependencySourceV1::Registry(source) => Some(source.registry.clone()),
        _ => None,
    };
    let (alias, dependency, package) = stacks
        .get_key_value(reference)
        .and_then(|(alias, dependency)| Some((alias, dependency, registry(dependency)?)))
        .or_else(|| {
            stacks.iter().find_map(|(alias, dependency)| {
                registry(dependency)
                    .filter(|package| package == reference)
                    .map(|package| (alias, dependency, package))
            })
        })?;
    Some(ProjectStack {
        alias: alias.clone(),
        package,
        endpoints: dependency
            .endpoints
            .iter()
            .map(|(live, endpoint)| {
                (
                    live.clone(),
                    endpoint.websocket.clone(),
                    endpoint.query.clone(),
                )
            })
            .collect(),
    })
}

/// The stream endpoints the generated SDK reads, labelled. arete.toml
/// `endpoints` (recorded by `a4 up`) take precedence over the registry's
/// hosted endpoints, as `a4 install` applies them.
fn sdk_endpoints(
    descriptor: &RegistryStackInstallResponse,
    project: Option<&ProjectStack>,
) -> Value {
    let hosted = descriptor
        .live_specs
        .iter()
        .map(|live| {
            json!({
                "alias": live.alias,
                "websocket": live.binding.websocket_endpoint,
                "query": live.binding.query_endpoint,
            })
        })
        .collect::<Vec<_>>();
    match project {
        Some(project) if !project.endpoints.is_empty() => json!({
            "used": "project",
            "dependency": project.alias,
            "project": {
                "source": "arete.toml endpoints recorded by `a4 up`",
                "liveSpecs": project
                    .endpoints
                    .iter()
                    .map(|(alias, websocket, query)| {
                        json!({ "alias": alias, "websocket": websocket, "query": query })
                    })
                    .collect::<Vec<_>>(),
            },
            "hosted": {
                "note": "not used: arete.toml endpoints take precedence",
                "liveSpecs": hosted,
            },
        }),
        Some(project) => json!({
            "used": "hosted",
            "dependency": project.alias,
            "hosted": { "liveSpecs": hosted },
        }),
        None => json!({
            "used": "hosted",
            "hosted": { "liveSpecs": hosted },
        }),
    }
}

/// The TypeScript and Rust descriptors of one stack, both requested with the
/// install reference that resolved. `descriptor.stack` names the deployed
/// stack's public subdomain, which is not necessarily an install reference
/// (a catalog package slug is not one), so it is never used to ask again.
fn resolve_stack_descriptors(
    client: &ApiClient,
    reference: &str,
) -> Result<(
    String,
    RegistryStackInstallResponse,
    RegistryStackInstallResponse,
)> {
    let (install_ref, typescript) = resolve_stack_descriptor(client, reference, None)?;
    let rust = client
        .get_registry_stack_install(&install_ref, Some("rust"))
        .with_context(|| descriptor_diagnostic(&install_ref))?;
    validate_stack_descriptor_identity(&typescript, &rust)?;
    Ok((install_ref, typescript, rust))
}

/// The descriptor for `reference`, and the exact install reference that
/// produced it: `reference` itself, or the one a legacy display name or an
/// owner's private stack name translated to.
fn resolve_stack_descriptor(
    client: &ApiClient,
    reference: &str,
    language: Option<&str>,
) -> Result<(String, RegistryStackInstallResponse)> {
    let direct_error = match client.get_registry_stack_install(reference, language) {
        Ok(descriptor) => return Ok((reference.to_string(), descriptor)),
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
                let descriptor = client
                    .get_registry_stack_install(&install_ref, language)
                    .with_context(|| descriptor_diagnostic(&install_ref))?;
                return Ok((install_ref, descriptor));
            }
        }
    }

    // Private and global user stacks are intentionally absent from the public
    // registry listing. Resolve their display name through the authenticated
    // deployment list and retry with the stable atom/install reference.
    if let Ok(Some(install_ref)) = paginated_deployment_install_ref(reference, |limit, offset| {
        client.list_deployments_page(limit, offset)
    }) {
        let descriptor = client
            .get_registry_stack_install(&install_ref, language)
            .with_context(|| descriptor_diagnostic(&install_ref))?;
        return Ok((install_ref, descriptor));
    }

    Err(direct_error).with_context(|| descriptor_diagnostic(reference))
}

fn matching_deployment<'a>(
    reference: &str,
    deployments: &'a [DeploymentResponse],
) -> Option<&'a DeploymentResponse> {
    deployments
        .iter()
        .filter(|deployment| {
            deployment.branch.is_none() && deployment.spec_name.eq_ignore_ascii_case(reference)
        })
        .max_by_key(|deployment| deployment_selection_key(deployment))
}

fn paginated_deployment_install_ref<F>(reference: &str, mut list_page: F) -> Result<Option<String>>
where
    F: FnMut(i64, i64) -> Result<Vec<DeploymentResponse>>,
{
    let mut selected = None;
    for page in 0..MAX_DEPLOYMENT_PAGES {
        let deployments = list_page(DEPLOYMENT_PAGE_SIZE, page * DEPLOYMENT_PAGE_SIZE)?;
        if let Some(candidate) = matching_deployment(reference, &deployments) {
            let should_replace = selected.as_ref().is_none_or(|current| {
                deployment_selection_key(candidate) > deployment_selection_key(current)
            });
            if should_replace {
                selected = Some(candidate.clone());
            }
        }
        if deployments.len() < DEPLOYMENT_PAGE_SIZE as usize {
            return Ok(selected.map(|deployment| deployment.atom_name));
        }
    }
    anyhow::bail!("Deployment lookup exceeded the bounded pagination limit")
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
        service_class: descriptor.service_class.clone(),
        spec_version_id: descriptor.spec_version_id,
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
    install_ref: &str,
    typescript: &RegistryStackInstallResponse,
    rust: &RegistryStackInstallResponse,
    knowledge: Option<&StackKnowledge>,
) -> Result<StackExploreOutput> {
    let selected_views = selected_views(typescript, knowledge)?;
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
        install_ref: install_ref.to_string(),
        description: typescript.description.clone(),
        visibility: typescript.visibility.clone(),
        service_class: typescript.service_class.clone(),
        identity: StackIdentitySummary {
            stack_manifest_hash: typescript.stack_manifest_hash.clone(),
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
        auth_requirements: shape::auth_requirements(&serde_json::to_value(typescript)?),
        sdk_endpoints: sdk_endpoints(typescript, None),
        account: None,
        install_command: format!("a4 install stack {install_ref} --ts"),
        knowledge: knowledge.map(|knowledge| full_stack_knowledge(typescript, knowledge)),
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
        extension_api: extension.manifest.extension_api,
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

fn selected_views(
    descriptor: &RegistryStackInstallResponse,
    knowledge: Option<&StackKnowledge>,
) -> Result<Vec<SelectedViewSummary>> {
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
            let (entity, view) = shape::find_view(&live.artifact, view_id).ok_or_else(|| {
                anyhow::anyhow!(
                    "Selected view '{}:{}' is absent from its exact LiveSpec",
                    live_alias,
                    view_id
                )
            })?;
            let entity_name = shape::entity_name(entity).unwrap_or("unknown");
            Ok(SelectedViewSummary {
                live_alias,
                view_id: view_id.into(),
                entity: entity_name.into(),
                summary: knowledge
                    .and_then(|knowledge| knowledge.entity(entity_name))
                    .and_then(|entity| entity.view_summary(view_id))
                    .map(str::to_string),
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
        accounts: shape::idl_accounts(idl),
        instructions: shape::idl_instructions(idl),
        events: shape::idl_events(idl),
        types: shape::idl_types(idl),
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

fn build_entity_output(
    descriptor: &RegistryStackInstallResponse,
    query: &str,
    knowledge: Option<&StackKnowledge>,
) -> Result<StackEntityExploreOutput> {
    let selected = selected_views(descriptor, knowledge)?;
    let (requested_alias, requested_name) = query
        .split_once(':')
        .map_or((None, query), |(alias, name)| (Some(alias), name));
    let mut matches = Vec::new();
    for live in &descriptor.live_specs {
        if requested_alias.is_some_and(|alias| !live.alias.eq_ignore_ascii_case(alias)) {
            continue;
        }
        for entity in shape::live_entities(&live.artifact) {
            if shape::entity_name(entity)
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
                shape::live_entities(&live.artifact)
                    .iter()
                    .filter_map(shape::entity_name)
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
    let name = shape::entity_name(entity).unwrap_or("unknown");
    let views = selected
        .into_iter()
        .filter(|view| view.live_alias == live_alias && view.entity.eq_ignore_ascii_case(name))
        .collect();
    Ok(StackEntityExploreOutput {
        schema_version: EXPLORE_SCHEMA_VERSION,
        kind: "stack-entity",
        stack: descriptor.stack.clone(),
        identity: StackIdentitySummary {
            stack_manifest_hash: descriptor.stack_manifest_hash.clone(),
            spec_version_id: descriptor.spec_version_id,
        },
        live_alias: live_alias.into(),
        name: name.into(),
        summary: knowledge
            .and_then(|knowledge| knowledge.entity(name))
            .and_then(|entity| entity.summary.clone()),
        program_id: shape::entity_program_id(entity).map(str::to_string),
        primary_keys: shape::entity_primary_keys(entity),
        fields: shape::described_entity_fields(entity, knowledge),
        views,
        knowledge: knowledge.map(|knowledge| knowledge.source(KeyCase::Camel)),
    })
}

fn render_stack(output: &StackExploreOutput) -> String {
    let mut text = format!(
        "\nStack: {}\n  Install reference: {}\n  Visibility: {}\n  Service class: {}\n",
        output.name, output.install_ref, output.visibility, output.service_class
    );
    if let Some(description) = &output.description {
        text.push_str(&format!("  Description: {description}\n"));
    }
    text.push_str(&format!(
        "\nIdentity\n  StackManifest: {}\n",
        output.identity.stack_manifest_hash
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
            if let Some(summary) = &view.summary {
                text.push_str(&format!("    {summary}\n"));
            }
        }
    }
    if let Some(knowledge) = &output.knowledge {
        text.push_str(&render_knowledge_entities(knowledge));
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
    text.push_str(&render_auth_notes(
        &output.auth_requirements,
        output.account.as_ref(),
    ));
    text.push_str(&render_sdk_endpoints(&output.sdk_endpoints));
    text.push_str(&format!("\nInstall\n  {}\n", output.install_command));
    text
}

/// Browser-key and transaction-access notes under a stack's authentication.
fn render_auth_notes(requirements: &Value, account: Option<&Value>) -> String {
    let mut text = String::new();
    if requirements.get("browser").is_some() {
        text.push_str(
            "  Browser: a publishable key, bound to exactly one origin (a4 auth keys create-publishable --origin <origin>)\n",
        );
    }
    if requirements["transactionEntitlementRequired"] == true {
        text.push_str("  Transactions: require transaction access on your account\n");
    }
    if let Some(transactions) = account.and_then(|account| account.get("transactions")) {
        let mut line = format!(
            "  Your account: transactions {}",
            transactions["status"].as_str().unwrap_or("unknown")
        );
        let missing = string_list(transactions, "missing");
        if missing != "none" {
            line.push_str(&format!(" (missing {missing})"));
        }
        if let Some(detail) = transactions["detail"].as_str() {
            line.push_str(&format!("; {detail}"));
        }
        text.push_str(&line);
        text.push('\n');
    }
    text
}

/// Which stream endpoints the generated SDK reads, with the other set labelled.
fn render_sdk_endpoints(endpoints: &Value) -> String {
    let mut text = String::from("\nSDK endpoints\n");
    if let Some(dependency) = endpoints["dependency"].as_str() {
        text.push_str(&format!("  arete.toml dependency: {dependency}\n"));
    }
    let used = endpoints["used"].as_str().unwrap_or("hosted");
    for key in ["project", "hosted"] {
        let Some(set) = endpoints.get(key) else {
            continue;
        };
        let source = if key == "project" {
            "arete.toml, from `a4 up`"
        } else {
            "hosted"
        };
        let label = if key == used {
            "used by the generated SDK"
        } else {
            "not used"
        };
        for live in value_array(set, "liveSpecs") {
            text.push_str(&format!(
                "  {}  {}  {}  [{source}; {label}]\n",
                live["alias"].as_str().unwrap_or("-"),
                live["websocket"].as_str().unwrap_or("-"),
                live["query"].as_str().unwrap_or("-"),
            ));
        }
    }
    text
}

/// One auth surface as `keys …; scopes …; entitlement required`.
fn render_auth_surface(auth: &Value) -> String {
    let mut parts = vec![format!("keys {}", string_list(auth, "acceptedKeyClasses"))];
    let scopes = string_list(auth, "scopes");
    if scopes != "none" {
        parts.push(format!("scopes {scopes}"));
    }
    if auth["transactionEntitlementRequired"] == true {
        parts.push("transaction access required".into());
    }
    parts.join("; ")
}

fn render_stack_summary(output: &Value) -> String {
    let mut text = format!(
        "\nStack: {} (install reference {})\n",
        output["name"].as_str().unwrap_or("-"),
        output["installRef"].as_str().unwrap_or("-")
    );
    if let Some(description) = output["description"].as_str() {
        text.push_str(&format!("  {description}\n"));
    }
    text.push_str(&format!(
        "  StackManifest: {}\n\nEntities\n",
        output["stackManifestHash"].as_str().unwrap_or("-")
    ));
    for entity in value_array(output, "entities") {
        let described = value_array(entity, "fieldDescriptions").len();
        text.push_str(&format!(
            "  {}:{}  key {}  {} field(s){}\n",
            entity["liveAlias"].as_str().unwrap_or("-"),
            entity["name"].as_str().unwrap_or("-"),
            string_list(entity, "primaryKeys"),
            entity["fieldCount"].as_u64().unwrap_or(0),
            if described > 0 {
                format!(", {described} described")
            } else {
                String::new()
            }
        ));
        if let Some(summary) = entity["summary"].as_str() {
            text.push_str(&format!("    {summary}\n"));
        }
        let views = value_array(entity, "views")
            .iter()
            .map(|view| {
                format!(
                    "{} ({})",
                    view["id"].as_str().unwrap_or("-"),
                    view["output"].as_str().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>();
        if !views.is_empty() {
            text.push_str(&format!("    views: {}\n", views.join(", ")));
        }
    }
    text.push_str("\nProgram SDKs\n");
    let programs = value_array(output, "programs");
    if programs.is_empty() {
        text.push_str("  none\n");
    }
    for program in programs {
        text.push_str(&format!(
            "  {}  {}  release {}{}\n",
            program["installName"].as_str().unwrap_or("-"),
            program["programId"].as_str().unwrap_or("-"),
            program["programReleaseHash"].as_str().unwrap_or("-"),
            program["sdkExtension"]["entry"]
                .as_str()
                .map(|entry| format!("  extension {entry}"))
                .unwrap_or_default()
        ));
    }
    text.push_str(&render_sdk_endpoints(&output["sdkEndpoints"]));
    let endpoints = &output["endpoints"];
    for (label, key) in [("Chain", "chain"), ("Transaction", "transaction")] {
        if let Some(endpoint) = endpoints[key].as_str() {
            text.push_str(&format!("  {label}: {endpoint}\n"));
        }
    }
    for read in value_array(endpoints, "programReads") {
        text.push_str(&format!(
            "  Program Read {}: {}\n",
            read["program"].as_str().unwrap_or("-"),
            read["endpoint"].as_str().unwrap_or("-")
        ));
    }
    text.push_str("\nAuthentication\n");
    let auth = &output["auth"];
    for key in ["stream", "query", "chain", "transaction"] {
        if let Some(surface) = auth.get(key) {
            text.push_str(&format!("  {key}: {}\n", render_auth_surface(surface)));
        }
    }
    for read in value_array(auth, "programReads") {
        text.push_str(&format!(
            "  Program Read {}: {}\n",
            read["program"].as_str().unwrap_or("-"),
            render_auth_surface(read)
        ));
    }
    text.push_str(&render_auth_notes(auth, output.get("account")));
    text.push_str(&format!(
        "\nInstall\n  {}\n\nMore: --views <Entity/view,...> for view schemas{}, --operation <id> for one program operation\n",
        output["installCommand"].as_str().unwrap_or("-"),
        if output.get("knowledge").is_some() {
            " and field descriptions"
        } else {
            ""
        }
    ));
    text
}

fn render_stack_views(output: &Value) -> String {
    let mut text = format!(
        "\nStack: {} (install reference {})\n  StackManifest: {}\n",
        output["name"].as_str().unwrap_or("-"),
        output["installRef"].as_str().unwrap_or("-"),
        output["stackManifestHash"].as_str().unwrap_or("-")
    );
    for view in value_array(output, "views") {
        text.push_str(&format!(
            "\nView {}:{}  (entity {})\n",
            view["liveAlias"].as_str().unwrap_or("-"),
            view["id"].as_str().unwrap_or("-"),
            view["entity"].as_str().unwrap_or("-")
        ));
        if let Some(summary) = view["summary"].as_str() {
            text.push_str(&format!("  {summary}\n"));
        }
        if let Some(reason) = view["schemaUnavailable"].as_str() {
            text.push_str(&format!("  Schema unavailable: {reason}\n"));
            continue;
        }
        text.push_str(&format!(
            "  Output: {}\n  Pipeline steps: {}\n  Primary key: {}\n",
            compact_json(&view["output"]),
            value_array(view, "pipeline").len(),
            string_list(view, "primaryKeys")
        ));
        if let Some(websocket) = view["endpoint"]["websocket"].as_str() {
            text.push_str(&format!("  Hosted WebSocket: {websocket}\n"));
        }
        text.push_str("  Fields\n");
        let fields: Vec<EntityField> =
            serde_json::from_value(view["fields"].clone()).unwrap_or_default();
        for field in &fields {
            text.push_str(&render_field(field, "    "));
        }
    }
    text.push_str(&render_sdk_endpoints(&output["sdkEndpoints"]));
    text
}

fn render_operation(output: &Value) -> String {
    let operation = &output["operation"];
    let program = &output["program"];
    let id = operation["path"]
        .as_str()
        .or_else(|| operation["name"].as_str())
        .unwrap_or("-");
    let mut text = format!(
        "\nOperation: {id}  ({}, {})\n  Program: {} ({})  Program Release: {}\n",
        operation["kind"].as_str().unwrap_or("-"),
        operation["mode"].as_str().unwrap_or("-"),
        program["installName"].as_str().unwrap_or("-"),
        program["programId"].as_str().unwrap_or("-"),
        program["programReleaseHash"].as_str().unwrap_or("-")
    );
    if let Some(stack) = output["stack"].as_str() {
        text.push_str(&format!("  Stack: {stack}\n"));
    }
    if let Some(title) = operation["title"].as_str() {
        text.push_str(&format!("  Title: {title}\n"));
    }
    if let Some(operation_id) = operation["operationId"].as_str() {
        text.push_str(&format!("  Operation id: {operation_id}\n"));
    }
    if let Some(doc) = operation["doc"].as_str() {
        text.push_str(&format!("\n{doc}\n"));
    }
    if let Some(paths) = operation["generatedPaths"].as_object() {
        text.push_str("\nGenerated paths\n");
        for (language, path) in paths {
            text.push_str(&format!("  {language}: {}\n", compact_json(path)));
        }
    }
    if let Some(input) = operation.get("input").filter(|input| input.is_object()) {
        text.push_str(&format!(
            "\nInput ({})\n",
            input["typeName"].as_str().unwrap_or("-")
        ));
        for field in value_array(input, "fields") {
            text.push_str(&format!(
                "  {}{}: {}{}\n",
                field["name"].as_str().unwrap_or("-"),
                if field["optional"] == true { "?" } else { "" },
                compact_json(&field["type"]),
                field["doc"]
                    .as_str()
                    .map(|doc| format!("  {doc}"))
                    .unwrap_or_default()
            ));
        }
    }
    let arguments = value_array(operation, "arguments");
    if !arguments.is_empty() {
        text.push_str("\nArguments\n");
        for argument in arguments {
            text.push_str(&format!(
                "  {}: {}\n",
                argument["name"].as_str().unwrap_or("-"),
                compact_json(&argument["type"])
            ));
        }
    }
    if let Some(accounts) = operation.get("accounts") {
        let names = |key: &str| {
            let names = value_array(accounts, key)
                .iter()
                .filter_map(|account| account["name"].as_str())
                .collect::<Vec<_>>();
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        };
        text.push_str(&format!(
            "\nAccounts\n  Required: {}\n  Derived: {}\n",
            names("required"),
            names("derived")
        ));
        if accounts.get("optional").is_some() {
            text.push_str(&format!("  Optional: {}\n", names("optional")));
        }
        text.push_str(&format!(
            "  Signers: {}\n",
            string_list(operation, "signers")
        ));
    }
    if let Some(count) = operation["transactionCount"].as_u64() {
        text.push_str(&format!("\nTransactions: {count}\n"));
    }
    let errors: Vec<ErrorSummary> =
        serde_json::from_value(operation["errors"].clone()).unwrap_or_default();
    if !errors.is_empty() {
        text.push_str("\nProgram errors\n");
        for error in errors {
            text.push_str(&format!(
                "  {} {}{}\n",
                error
                    .code
                    .as_ref()
                    .map(compact_json)
                    .unwrap_or_else(|| "-".into()),
                error.name,
                error
                    .message
                    .map(|message| format!(": {message}"))
                    .unwrap_or_default()
            ));
        }
    }
    if let Some(transport) = operation.get("transport") {
        text.push_str(&format!(
            "\nTransport\n  {}  {}\n",
            transport["endpoint"].as_str().unwrap_or("-"),
            render_auth_surface(&transport["auth"])
        ));
    }
    if let Some(transactions) = output["account"].get("transactions") {
        text.push_str(&render_auth_notes(
            &Value::Null,
            Some(&json!({ "transactions": transactions })),
        ));
    }
    if let Some(usage) = operation["usage"].as_object() {
        text.push_str("\nUsage\n");
        for line in usage.values().filter_map(Value::as_str) {
            text.push_str(&format!("  {line}\n"));
        }
    }
    if let Some(reason) = operation["surfaceUnavailable"].as_str() {
        text.push_str(&format!("\nKnowledge surface unavailable: {reason}\n"));
    }
    let not_reported = string_list(operation, "notReported");
    if not_reported != "none" {
        text.push_str(&format!("\nNot reported: {not_reported}\n"));
    }
    text
}

fn render_program_sections(output: &Value) -> String {
    let program = &output["program"];
    let mut text = format!(
        "\nProgram: {} ({})\n  Program ID: {}\n  Program Release: {}\n",
        program["displayName"].as_str().unwrap_or("-"),
        program["installName"].as_str().unwrap_or("-"),
        program["programId"].as_str().unwrap_or("-"),
        program["programReleaseHash"].as_str().unwrap_or("-")
    );
    if let Some(accounts) = output.get("accounts") {
        let accounts: Vec<AccountSummary> =
            serde_json::from_value(accounts.clone()).unwrap_or_default();
        text.push_str(&render_accounts(&accounts));
    }
    if let Some(instructions) = output.get("instructions") {
        let instructions: Vec<InstructionSummary> =
            serde_json::from_value(instructions.clone()).unwrap_or_default();
        text.push_str(&render_instructions(&instructions));
        let errors: Vec<ErrorSummary> =
            serde_json::from_value(output["errors"].clone()).unwrap_or_default();
        text.push_str("\nErrors\n");
        if errors.is_empty() {
            text.push_str("  none\n");
        }
        for error in errors {
            text.push_str(&format!(
                "  {} {}{}\n",
                error
                    .code
                    .as_ref()
                    .map(compact_json)
                    .unwrap_or_else(|| "-".into()),
                error.name,
                error
                    .message
                    .map(|message| format!(": {message}"))
                    .unwrap_or_default()
            ));
        }
    }
    if let Some(events) = output.get("events") {
        let events: Vec<EventSummary> = serde_json::from_value(events.clone()).unwrap_or_default();
        text.push_str("\nEvents\n");
        if events.is_empty() {
            text.push_str("  none\n");
        }
        for event in events {
            let fields = event
                .fields
                .iter()
                .map(|field| format!("{}: {}", field.name, compact_json(&field.field_type)))
                .collect::<Vec<_>>()
                .join(", ");
            text.push_str(&format!("  {}({fields})\n", event.name));
        }
    }
    if let Some(types) = output.get("types") {
        let types: Vec<TypeSummary> = serde_json::from_value(types.clone()).unwrap_or_default();
        text.push_str("\nTypes\n");
        if types.is_empty() {
            text.push_str("  none\n");
        }
        for user_type in types {
            let members = if user_type.variants.is_empty() {
                user_type
                    .fields
                    .iter()
                    .map(|field| format!("{}: {}", field.name, compact_json(&field.field_type)))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                user_type.variants.join(" | ")
            };
            text.push_str(&format!(
                "  {} ({})  {members}\n",
                user_type.name, user_type.kind
            ));
        }
    }
    if let Some(reason) = output["operationsUnavailable"].as_str() {
        text.push_str(&format!("\nOperations\n  unavailable: {reason}\n"));
    } else if output.get("operations").is_some() {
        text.push_str("\nOperations\n");
        for operation in value_array(output, "operations") {
            text.push_str(&format!(
                "  {:<16} {}{}\n",
                operation["kind"].as_str().unwrap_or("-"),
                operation["path"].as_str().unwrap_or("-"),
                operation["title"]
                    .as_str()
                    .map(|title| format!("  {title}"))
                    .unwrap_or_default()
            ));
        }
    }
    text.push_str(&format!(
        "\nInstall\n  {}\n",
        output["installCommand"].as_str().unwrap_or("-")
    ));
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
    text.push_str(&render_accounts(&output.accounts));
    text.push_str(&render_instructions(&output.instructions));
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

fn render_accounts(accounts: &[AccountSummary]) -> String {
    let mut text = String::from("\nAccounts\n");
    if accounts.is_empty() {
        text.push_str("  none\n");
    }
    for account in accounts {
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
    text
}

fn render_instructions(instructions: &[InstructionSummary]) -> String {
    let mut text = String::from("\nInstructions\n");
    if instructions.is_empty() {
        text.push_str("  none\n");
    }
    for instruction in instructions {
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
    if let Some(summary) = &output.summary {
        text.push_str(&format!("  {summary}\n"));
    }
    text.push_str("\nFields\n");
    for field in &output.fields {
        text.push_str(&render_field(field, "  "));
    }
    text.push_str("\nSelected views\n");
    if output.views.is_empty() {
        text.push_str("  none\n");
    } else {
        for view in &output.views {
            text.push_str(&format!(
                "  {}:{}{}\n",
                view.live_alias,
                view.view_id,
                view.summary
                    .as_deref()
                    .map(|summary| format!("  {summary}"))
                    .unwrap_or_default()
            ));
        }
    }
    text
}

/// One schema field, with its curated description on the next line.
fn render_field(field: &EntityField, indent: &str) -> String {
    let mut text = format!(
        "{indent}{}  {}{}\n",
        field.path,
        field.rust_type,
        if field.nullable { "?" } else { "" }
    );
    if let Some(description) = &field.description {
        text.push_str(&format!("{indent}    {description}\n"));
    }
    text
}

/// The described entities of a full stack exploration.
fn render_knowledge_entities(knowledge: &Value) -> String {
    let entities = value_array(knowledge, "entities");
    if entities.is_empty() {
        return String::new();
    }
    let mut text = String::from("\nEntities\n");
    for entity in entities {
        let described = value_array(entity, "fieldDescriptions").len();
        text.push_str(&format!(
            "  {}:{}{}\n",
            entity["liveAlias"].as_str().unwrap_or("-"),
            entity["name"].as_str().unwrap_or("-"),
            if described > 0 {
                format!("  {described} described field(s)")
            } else {
                String::new()
            }
        ));
        if let Some(summary) = entity["summary"].as_str() {
            text.push_str(&format!("    {summary}\n"));
        }
    }
    text.push_str("  Field descriptions: a4 explore stack <ref> <Entity>\n");
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

// ============================================================================
// Catalog: the public discovery and installation boundary
// ============================================================================

/// Search filters for `a4 explore catalog`.
#[derive(Debug, Default, Clone, Copy)]
pub struct CatalogSearchArgs<'a> {
    pub query: Option<&'a str>,
    pub concept: Option<&'a str>,
    pub category: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub target: Option<&'a str>,
    pub limit: Option<usize>,
    pub cursor: Option<&'a str>,
}

const CATALOG_KINDS: [&str; 2] = ["program", "stack"];
const CATALOG_MODES: [&str; 3] = ["build", "read", "subscribe"];
const CATALOG_TARGETS: [&str; 3] = ["typescript", "rust", "python"];

fn catalog_choice<'a>(
    value: Option<&'a str>,
    flag: &str,
    allowed: &[&str],
) -> Result<Option<&'a str>> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) if allowed.contains(&value) => Ok(Some(value)),
        Some(value) => Err(anyhow::anyhow!(
            "{flag} must be one of {}; got '{value}'",
            allowed.join(", ")
        )),
    }
}

/// A catalog slug is one bare URL path segment. It is interpolated into an
/// authenticated request path, so anything a URL parser could treat as a
/// separator, escape, or relative component is rejected before the request
/// is built (mirroring the MCP client's `path_segment`).
fn catalog_slug(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("slug must not be empty");
    }
    if let Some(bad) = trimmed
        .chars()
        .find(|c| c.is_whitespace() || c.is_control() || "/\\?#%&".contains(*c))
    {
        anyhow::bail!(
            "slug contains an invalid character {bad:?}; pass a bare package slug (e.g. `ore`), not a path or URL"
        );
    }
    if trimmed.chars().all(|c| c == '.') {
        anyhow::bail!(
            "slug must not be a relative path segment; pass a bare package slug (e.g. `ore`)"
        );
    }
    Ok(trimmed.to_string())
}

pub fn catalog_search(args: CatalogSearchArgs<'_>, json: bool) -> Result<()> {
    let non_empty = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let query = non_empty(args.query);
    let concept = non_empty(args.concept);
    let category = non_empty(args.category);
    let kind = catalog_choice(args.kind, "--kind", &CATALOG_KINDS)?;
    let mode = catalog_choice(args.mode, "--mode", &CATALOG_MODES)?;
    let target = catalog_choice(args.target, "--target", &CATALOG_TARGETS)?;
    let cursor = non_empty(args.cursor);
    if query.is_none()
        && concept.is_none()
        && category.is_none()
        && kind.is_none()
        && mode.is_none()
        && target.is_none()
    {
        anyhow::bail!(
            "Provide at least one of --query, --concept, --category, --kind, --mode, or --target. \
             Run `a4 explore catalog --vocabulary` to list concept and category slugs."
        );
    }
    let value = ApiClient::new()?.catalog_search(
        query.as_deref(),
        concept.as_deref(),
        category.as_deref(),
        kind,
        mode,
        target,
        args.limit,
        cursor.as_deref(),
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    print!("{}", render_catalog_search(&value));
    Ok(())
}

pub fn catalog_entry(kind: &str, slug: &str, json: bool) -> Result<()> {
    let kind = catalog_choice(Some(kind), "kind", &CATALOG_KINDS)?
        .ok_or_else(|| anyhow::anyhow!("kind must be program or stack"))?;
    let slug = catalog_slug(slug)?;
    let value = ApiClient::new()?.catalog_entry(kind, &slug).with_context(|| {
        format!("{kind} '{slug}' is not in the active catalog; search with `a4 explore catalog --query <intent>`")
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    print!("{}", render_catalog_entry(&value));
    Ok(())
}

pub fn catalog_vocabulary(json: bool) -> Result<()> {
    let value = ApiClient::new()?.catalog_vocabulary()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    let mut text = String::from("\nConcepts\n");
    for concept in value_array(&value, "concepts") {
        text.push_str(&format!(
            "  {}  {}\n",
            concept["slug"].as_str().unwrap_or("-"),
            concept["description"].as_str().unwrap_or("")
        ));
    }
    text.push_str("\nCategories\n");
    for category in value_array(&value, "categories") {
        text.push_str(&format!(
            "  {}  {}\n",
            category["slug"].as_str().unwrap_or("-"),
            category["description"].as_str().unwrap_or("")
        ));
    }
    print!("{text}");
    Ok(())
}

fn string_list(value: &Value, key: &str) -> String {
    let items = value_array(value, key)
        .iter()
        .filter_map(|item| {
            item.as_str()
                .or_else(|| item.get("target").and_then(Value::as_str))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        "none".into()
    } else {
        items.join(", ")
    }
}

/// The install command for one catalog entry, pinned to the exact version the
/// catalog showed (`=<version>`), so a later activation cannot silently swap
/// the release under the user. The lockfile then records the
/// `packageReleaseHash` printed beside it.
fn pinned_install_command(entry: &Value) -> String {
    let kind = entry["kind"].as_str().unwrap_or("-");
    let slug = entry["slug"].as_str().unwrap_or("-");
    match entry["version"].as_str() {
        Some(version) => format!("a4 install {kind} {slug}@={version}"),
        None => format!("a4 install {kind} {slug}"),
    }
}

fn render_catalog_search(value: &Value) -> String {
    let results = value_array(value, "results");
    let mut text = String::new();
    let matched = string_list(value, "matchedConcepts");
    if matched != "none" {
        text.push_str(&format!("\nMatched concepts: {matched}\n"));
    }
    if results.is_empty() {
        text.push_str("\nNo catalog entries match. Broaden the query or run `a4 explore catalog --vocabulary`.\n");
        return text;
    }
    text.push_str("\nCatalog entries\n");
    for result in results {
        let kind = result["kind"].as_str().unwrap_or("-");
        let slug = result["slug"].as_str().unwrap_or("-");
        let health = result
            .pointer("/delivery/health")
            .and_then(Value::as_str)
            .unwrap_or("n/a");
        text.push_str(&format!(
            "  {kind} {slug}@{}  {}\n",
            result["version"].as_str().unwrap_or("-"),
            result["name"].as_str().unwrap_or("")
        ));
        let summary = result["summary"].as_str().unwrap_or("");
        if !summary.is_empty() {
            text.push_str(&format!("    {summary}\n"));
        }
        text.push_str(&format!(
            "    modes: {}  targets: {}  delivery: {health}\n",
            string_list(result, "modes"),
            string_list(result, "sdkTargets")
        ));
        text.push_str(&format!(
            "    install: {}  ({})\n",
            pinned_install_command(result),
            result["packageReleaseHash"].as_str().unwrap_or("-")
        ));
    }
    if let Some(cursor) = value["nextCursor"].as_str() {
        text.push_str(&format!(
            "\nMore results: repeat the same search with --cursor {cursor}\n"
        ));
    }
    text
}

fn render_catalog_entry(value: &Value) -> String {
    let kind = value["kind"].as_str().unwrap_or("-");
    let slug = value["slug"].as_str().unwrap_or("-");
    let mut text = format!(
        "\n{} {}@{}\n  Install: {}\n  Package release: {}\n  Bundle: {}\n  Set: {}\n",
        kind,
        slug,
        value["version"].as_str().unwrap_or("-"),
        pinned_install_command(value),
        value["packageReleaseHash"].as_str().unwrap_or("-"),
        value["bundleHash"].as_str().unwrap_or("-"),
        value["setHash"].as_str().unwrap_or("-")
    );
    if let Some(program_id) = value["programId"].as_str() {
        text.push_str(&format!("  Program ID: {program_id}\n"));
    }
    if let Some(release) = value["programReleaseHash"].as_str() {
        text.push_str(&format!("  Program Release: {release}\n"));
    }
    if let Some(manifest) = value["stackManifestHash"].as_str() {
        text.push_str(&format!("  StackManifest: {manifest}\n"));
    }
    text.push_str(&format!(
        "  SDK targets: {}\n",
        string_list(value, "sdkTargets")
    ));
    if let Some(knowledge) = value
        .get("knowledge")
        .filter(|knowledge| knowledge.is_object())
    {
        text.push_str(&format!(
            "\nKnowledge: {}\n  {}\n",
            knowledge["name"].as_str().unwrap_or(slug),
            knowledge["summary"].as_str().unwrap_or("")
        ));
        if let Some(protocol) = knowledge["protocol"].as_str() {
            text.push_str(&format!("  Protocol: {protocol}\n"));
        }
    }
    text.push_str("\nCapabilities\n");
    let capabilities = value_array(value, "capabilities");
    if capabilities.is_empty() {
        text.push_str("  none\n");
    }
    for claim in capabilities {
        text.push_str(&format!(
            "  {} {}  {}\n",
            claim["mode"].as_str().unwrap_or("-"),
            claim["concept"].as_str().unwrap_or("-"),
            claim["operationId"].as_str().unwrap_or("-")
        ));
    }
    match value
        .get("delivery")
        .filter(|delivery| delivery.is_object())
    {
        Some(delivery) => text.push_str(&format!(
            "\nDelivery: {} {} ({})\n",
            delivery["kind"].as_str().unwrap_or("-"),
            delivery["health"].as_str().unwrap_or("-"),
            delivery["identity"].as_str().unwrap_or("-")
        )),
        None => text.push_str("\nDelivery: none advertised\n"),
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::{DeploymentLiveStatus, DeploymentPhase, DeploymentStatus};
    use serde_json::json;

    #[test]
    fn catalog_rendering_shows_install_ref_targets_capabilities_and_delivery() {
        let entry = json!({
            "kind": "program",
            "slug": "ore",
            "version": "1.0.0",
            "packageReleaseHash": "arete:registry-package-release:v2:sha256:aa",
            "bundleHash": "arete:h1:catalog-bundle:sha256:bb",
            "setHash": "arete:h1:catalog-publication-set:sha256:cc",
            "programId": "oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv",
            "programReleaseHash": "arete:h1:program-release:sha256:dd",
            "knowledge": {"name": "ore", "summary": "ORE mining.", "protocol": "ore"},
            "capabilities": [{"concept": "mining", "mode": "build", "operationId": "program/oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv/raw-instruction/deploy"}],
            "sdkTargets": [{"target": "typescript", "sdkInstallTargetHash": "arete:h1:sdk-install-target:sha256:ee"}],
            "delivery": {"kind": "program-read", "identity": "relation", "status": "active", "health": "ready"},
            "unexpectedFutureField": true
        });
        let rendered = render_catalog_entry(&entry);
        assert!(rendered.contains("Install: a4 install program ore@=1.0.0"));
        assert!(rendered.contains("SDK targets: typescript"));
        assert!(rendered.contains("build mining  program/oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv/raw-instruction/deploy"));
        assert!(rendered.contains("Delivery: program-read ready"));

        let page = json!({
            "matchedConcepts": ["mining"],
            "results": [{"kind": "program", "slug": "ore", "version": "1.0.0", "name": "ore", "summary": "ORE mining.", "modes": ["build", "read"], "sdkTargets": ["typescript"], "packageReleaseHash": "arete:registry-package-release:v2:sha256:aa", "delivery": {"health": "degraded"}}],
            "sets": ["arete:h1:catalog-publication-set:sha256:cc"],
            "nextCursor": "eyJ2IjoxfQ"
        });
        let rendered = render_catalog_search(&page);
        assert!(rendered.contains("Matched concepts: mining"));
        assert!(rendered.contains("program ore@1.0.0"));
        assert!(rendered.contains("install: a4 install program ore@=1.0.0"));
        assert!(rendered.contains("delivery: degraded"));
        assert!(rendered.contains("--cursor eyJ2IjoxfQ"));
        assert!(render_catalog_search(&json!({"results": []})).contains("No catalog entries match"));
    }

    #[test]
    fn catalog_filters_fail_closed() {
        assert!(catalog_choice(Some("bundle"), "--kind", &CATALOG_KINDS).is_err());
        assert_eq!(
            catalog_choice(Some(" stack "), "--kind", &CATALOG_KINDS).unwrap(),
            Some("stack")
        );
        assert_eq!(
            catalog_choice(None, "--mode", &CATALOG_MODES).unwrap(),
            None
        );
        assert!(catalog_choice(Some("go"), "--target", &CATALOG_TARGETS).is_err());
    }

    #[test]
    fn catalog_slugs_are_single_path_segments() {
        assert_eq!(catalog_slug(" ore ").unwrap(), "ore");
        assert_eq!(catalog_slug("spl-token").unwrap(), "spl-token");
        for bad in [
            "",
            "..",
            ".",
            "ore/x",
            "..\\..\\admin",
            "ore%2F..",
            "ore?x=1",
            "ore#frag",
            "ore&x",
            "or e",
            "ore\u{0}",
        ] {
            assert!(catalog_slug(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    fn deployment(
        id: i32,
        spec_name: &str,
        atom_name: &str,
        status: DeploymentStatus,
        phase: DeploymentPhase,
        branch: Option<&str>,
    ) -> DeploymentResponse {
        DeploymentResponse {
            id,
            spec_id: 29,
            spec_name: spec_name.into(),
            atom_name: atom_name.into(),
            branch: branch.map(str::to_string),
            current_build_id: None,
            current_spec_version_id: None,
            current_version: None,
            portable_ast_hash: None,
            deployment_release_hash: None,
            current_idl_program_ids: Vec::new(),
            current_image_tag: None,
            websocket_url: format!("wss://{atom_name}.example.test"),
            http_url: format!("https://{atom_name}.example.test"),
            websocket_auth: json!({}),
            http_auth: json!({}),
            transaction_relay_enabled: false,
            status,
            status_message: None,
            first_deployed_at: None,
            last_deployed_at: None,
            live_status: DeploymentLiveStatus {
                phase,
                desired_replicas: None,
                ready_replicas: None,
                available_replicas: None,
                updated_replicas: None,
                last_transition_time: None,
                source: "test".into(),
                error_category: None,
            },
            latest_operation: None,
        }
    }

    fn stack_descriptor() -> RegistryStackInstallResponse {
        serde_json::from_value(json!({
            "name": "Multi",
            "stack": "multi-stack",
            "description": "two lives",
            "visibility": "public",
            "specVersionId": 7,
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
        let output = build_stack_output("multi-stack", &typescript, &rust, None).unwrap();
        assert_eq!(output.schema_version, 1);
        assert_eq!(output.identity.stack_manifest_hash, "manifest-exact");
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
        let output = build_stack_output("multi-stack", &typescript, &rust, None).unwrap();
        assert_eq!(output.identity.stack_manifest_hash, "manifest-exact");
        assert_eq!(output.live_specs.len(), 1);
        assert_eq!(output.live_specs[0].live_spec_hash, "live-primary");
        assert_eq!(output.selected_views.len(), 1);
    }

    #[test]
    fn legacy_entity_drilldown_uses_exact_live_spec_and_selected_views() {
        let descriptor = stack_descriptor();
        assert!(build_entity_output(&descriptor, "Position", None).is_err());
        let output = build_entity_output(&descriptor, "primary:Position", None).unwrap();
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
    fn program_explore_resolves_event_data_type_fields() {
        let mut descriptor = program_descriptor();
        descriptor.definition.idl_payload["events"] = json!([{
            "name": "ValueSet",
            "fields": [],
            "data": {"kind": "definedTypeLinkNode", "name": "ValueSetData"}
        }]);
        descriptor.definition.idl_payload["types"] = json!([{
            "name": "ValueSetData",
            "type": {
                "kind": "struct",
                "fields": [{"name": "value", "type": "u64"}]
            }
        }]);

        let output = build_program_output(&descriptor).unwrap();

        assert_eq!(output.events[0].fields[0].name, "value");
        assert_eq!(output.events[0].fields[0].field_type, json!("u64"));
    }

    /// `ApiClient::new()` pointed at a mock registry that answers `responses`
    /// in order and records every request, with an owner credential.
    struct MockRegistry {
        _guard: std::sync::MutexGuard<'static, ()>,
        _dir: tempfile::TempDir,
        server: crate::api_client::test_support::MockServer,
    }

    impl MockRegistry {
        fn new(responses: Vec<(u16, String)>) -> Self {
            let guard = crate::api_client::test_support::ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let dir = tempfile::tempdir().unwrap();
            let server = crate::api_client::test_support::MockServer::json_sequence(responses);
            let credentials = dir.path().join("credentials.toml");
            std::fs::write(
                &credentials,
                format!(
                    "[keys]\n\"{}\" = \"a4_sk_explore_owner\"\n",
                    server.base_url()
                ),
            )
            .unwrap();
            std::env::set_var("ARETE_API_URL", server.base_url());
            std::env::set_var("ARETE_CREDENTIALS_PATH", &credentials);
            std::env::set_var("ARETE_TELEMETRY_DISABLED", "1");
            Self {
                _guard: guard,
                _dir: dir,
                server,
            }
        }

        /// The path and query of the next recorded request.
        fn next_target(&self) -> String {
            let request = self.server.request();
            request
                .request_line
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_string()
        }
    }

    impl Drop for MockRegistry {
        fn drop(&mut self) {
            std::env::remove_var("ARETE_API_URL");
            std::env::remove_var("ARETE_CREDENTIALS_PATH");
        }
    }

    fn single_live_descriptor(stack: &str) -> String {
        let mut descriptor = stack_descriptor();
        descriptor.stack = stack.into();
        descriptor.live_specs.truncate(1);
        descriptor.stack_manifest["payload"]["selectedViews"] = json!([
            {"liveAlias": "primary", "viewId": "Position/state"}
        ]);
        serde_json::to_string(&descriptor).unwrap()
    }

    fn install_target(reference: &str, language: Option<&str>) -> String {
        match language {
            Some(language) => format!(
                "/api/registry/stacks/{reference}/install?language={language}&capabilities=managed-solana-gateway-v1"
            ),
            None => format!(
                "/api/registry/stacks/{reference}/install?capabilities=managed-solana-gateway-v1"
            ),
        }
    }

    fn not_found() -> (u16, String) {
        (
            404,
            json!({"error": "Stack not found in registry"}).to_string(),
        )
    }

    #[test]
    fn explore_reuses_a_catalog_slug_even_when_the_descriptor_names_a_subdomain() {
        let registry = MockRegistry::new(vec![
            (200, single_live_descriptor("ore-abc123")),
            (200, single_live_descriptor("ore-abc123")),
        ]);
        let client = ApiClient::new().unwrap();
        let (install_ref, typescript, rust) = resolve_stack_descriptors(&client, "ore").unwrap();
        assert_eq!(install_ref, "ore");
        assert_eq!(typescript.stack, "ore-abc123");
        assert_eq!(registry.next_target(), install_target("ore", None));
        assert_eq!(registry.next_target(), install_target("ore", Some("rust")));
        let output = build_stack_output(&install_ref, &typescript, &rust, None).unwrap();
        assert_eq!(output.install_ref, "ore");
        assert_eq!(output.install_command, "a4 install stack ore --ts");
    }

    #[test]
    fn explore_reuses_the_reference_a_legacy_display_name_translated_to() {
        let listing = json!([{
            "name": "OreMining",
            "description": null,
            "websocket_url": "wss://oremining-x1y2z3.stack.arete.run",
            "entities": ["Position"]
        }]);
        let registry = MockRegistry::new(vec![
            not_found(),
            (200, listing.to_string()),
            (200, single_live_descriptor("oremining-x1y2z3")),
            (200, single_live_descriptor("oremining-x1y2z3")),
        ]);
        let client = ApiClient::new().unwrap();
        let (install_ref, _, _) = resolve_stack_descriptors(&client, "OreMining").unwrap();
        assert_eq!(install_ref, "oremining-x1y2z3");
        assert_eq!(registry.next_target(), install_target("OreMining", None));
        assert_eq!(registry.next_target(), "/api/registry");
        assert_eq!(
            registry.next_target(),
            install_target("oremining-x1y2z3", None)
        );
        assert_eq!(
            registry.next_target(),
            install_target("oremining-x1y2z3", Some("rust"))
        );
    }

    #[test]
    fn explore_reuses_the_atom_an_owner_private_name_resolved_to() {
        let deployments = serde_json::to_string(&vec![deployment(
            41,
            "Vault",
            "vault-live",
            DeploymentStatus::Active,
            DeploymentPhase::Running,
            None,
        )])
        .unwrap();
        let registry = MockRegistry::new(vec![
            not_found(),
            (200, "[]".into()),
            (200, deployments),
            (200, single_live_descriptor("vault-live")),
            (200, single_live_descriptor("vault-live")),
        ]);
        let client = ApiClient::new().unwrap();
        let (install_ref, _, _) = resolve_stack_descriptors(&client, "vault").unwrap();
        assert_eq!(install_ref, "vault-live");
        assert_eq!(registry.next_target(), install_target("vault", None));
        assert_eq!(registry.next_target(), "/api/registry");
        assert!(registry.next_target().starts_with("/api/deployments?"));
        assert_eq!(registry.next_target(), install_target("vault-live", None));
        assert_eq!(
            registry.next_target(),
            install_target("vault-live", Some("rust"))
        );
    }

    #[test]
    fn explore_still_rejects_typescript_and_rust_identity_drift() {
        let mut drifted: Value = serde_json::from_str(&single_live_descriptor("ore")).unwrap();
        drifted["stackManifestHash"] = json!("drifted");
        let registry = MockRegistry::new(vec![
            (200, single_live_descriptor("ore")),
            (200, drifted.to_string()),
        ]);
        let client = ApiClient::new().unwrap();
        let error = resolve_stack_descriptors(&client, "ore").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("different descriptor identities"),
            "{error:#}"
        );
        registry.next_target();
        assert_eq!(registry.next_target(), install_target("ore", Some("rust")));
    }

    #[test]
    fn legacy_stack_name_can_be_translated_from_listing_url() {
        assert_eq!(
            install_ref_from_websocket_url("wss://ore-stack-abc.stack.arete.run/ws").as_deref(),
            Some("ore-stack-abc")
        );
    }

    #[test]
    fn private_stack_name_resolves_to_serving_production_install_ref() {
        let deployments = vec![
            deployment(
                24,
                "Jurassic",
                "jurassic-old",
                DeploymentStatus::Stopped,
                DeploymentPhase::Missing,
                None,
            ),
            deployment(
                25,
                "jurassic",
                "jurassic-live",
                DeploymentStatus::Active,
                DeploymentPhase::Running,
                None,
            ),
            deployment(
                26,
                "jurassic",
                "jurassic-preview",
                DeploymentStatus::Active,
                DeploymentPhase::Running,
                Some("preview"),
            ),
        ];

        assert_eq!(
            matching_deployment("JURASSIC", &deployments)
                .map(|deployment| deployment.atom_name.as_str()),
            Some("jurassic-live")
        );
    }

    #[test]
    fn private_stack_name_lookup_paginates_beyond_the_first_page() {
        let first_page = (0..DEPLOYMENT_PAGE_SIZE)
            .map(|id| {
                deployment(
                    id as i32,
                    "Other",
                    "other",
                    DeploymentStatus::Active,
                    DeploymentPhase::Running,
                    None,
                )
            })
            .collect::<Vec<_>>();
        let second_page = vec![deployment(
            101,
            "Jurassic",
            "jurassic-live",
            DeploymentStatus::Active,
            DeploymentPhase::Running,
            None,
        )];
        let mut pages = vec![first_page, second_page].into_iter();
        let mut requests = Vec::new();

        let install_ref = paginated_deployment_install_ref("JURASSIC", |limit, offset| {
            requests.push((limit, offset));
            Ok(pages.next().unwrap_or_default())
        })
        .unwrap();

        assert_eq!(install_ref.as_deref(), Some("jurassic-live"));
        assert_eq!(
            requests,
            vec![(DEPLOYMENT_PAGE_SIZE, 0), (DEPLOYMENT_PAGE_SIZE, 100)]
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

    fn dependencies(toml: &str) -> BTreeMap<String, DependencyV1> {
        #[derive(serde::Deserialize)]
        struct Stacks {
            stacks: BTreeMap<String, DependencyV1>,
        }
        toml::from_str::<Stacks>(toml).unwrap().stacks
    }

    #[test]
    fn project_stacks_resolve_by_alias_or_registry_package() {
        let stacks = dependencies(
            r#"
            [stacks.mining]
            source = { registry = "ore" }
            version = "^1"
            endpoints = { live = { websocket = "wss://mine.example", query = "https://mine.example" } }

            [stacks.local]
            source = { path = "stacks/local.stack-manifest.json" }
            "#,
        );
        let by_alias = project_stack_in(&stacks, "mining").unwrap();
        assert_eq!(by_alias.package, "ore");
        assert_eq!(
            by_alias.endpoints,
            vec![(
                "live".to_string(),
                "wss://mine.example".to_string(),
                "https://mine.example".to_string()
            )]
        );
        assert_eq!(project_stack_in(&stacks, "ore"), Some(by_alias));
        assert_eq!(project_stack_in(&stacks, "local"), None);
        assert_eq!(project_stack_in(&stacks, "other"), None);
        assert_eq!(project_stack("/definitely/missing/arete.toml", "ore"), None);
    }

    #[test]
    fn sdk_endpoints_prefer_project_endpoints_and_label_the_hosted_ones() {
        let descriptor = stack_descriptor();
        let hosted = sdk_endpoints(&descriptor, None);
        assert_eq!(hosted["used"], "hosted");
        assert_eq!(
            hosted["hosted"]["liveSpecs"][0]["websocket"],
            "wss://primary.test"
        );
        assert!(hosted.get("project").is_none());

        let project = ProjectStack {
            alias: "multi".into(),
            package: "multi-stack".into(),
            endpoints: vec![(
                "primary".into(),
                "wss://mine.example".into(),
                "https://mine.example".into(),
            )],
        };
        let used = sdk_endpoints(&descriptor, Some(&project));
        assert_eq!(used["used"], "project");
        assert_eq!(used["dependency"], "multi");
        assert_eq!(
            used["project"]["liveSpecs"][0]["websocket"],
            "wss://mine.example"
        );
        assert!(used["hosted"]["note"]
            .as_str()
            .unwrap()
            .contains("not used"));
        let rendered = render_sdk_endpoints(&used);
        assert!(rendered.contains("wss://mine.example"));
        assert!(rendered.contains("[arete.toml, from `a4 up`; used by the generated SDK]"));
        assert!(rendered.contains("[hosted; not used]"));

        let dependency_without_endpoints = ProjectStack {
            endpoints: Vec::new(),
            ..project
        };
        let hosted = sdk_endpoints(&descriptor, Some(&dependency_without_endpoints));
        assert_eq!(hosted["used"], "hosted");
        assert_eq!(hosted["dependency"], "multi");
    }

    #[test]
    fn full_stack_output_reports_auth_requirements_and_resolves_entity_names() {
        let mut typescript = stack_descriptor();
        typescript.websocket_auth = Some(json!({
            "required": true,
            "accepted_key_classes": ["publishable", "secret"]
        }));
        let output =
            build_stack_output("multi-stack", &typescript, &stack_descriptor(), None).unwrap();
        assert_eq!(
            output.auth_requirements["stream"]["acceptedKeyClasses"],
            json!(["publishable", "secret"])
        );
        assert_eq!(output.auth_requirements["browser"]["originsPerKey"], 1);
        assert_eq!(output.sdk_endpoints["used"], "hosted");
        assert_eq!(output.selected_views[0].entity, "Position");
        let rendered = render_stack(&output);
        assert!(rendered.contains("bound to exactly one origin"));
        assert!(rendered.contains("SDK endpoints"));
    }

    #[test]
    fn entity_drilldown_reads_snake_case_live_specs() {
        let mut descriptor = stack_descriptor();
        descriptor.live_specs[0].artifact = json!({"payload": {"entities": [{
            "state_name": "Position",
            "program_id": "Program111",
            "identity": {"primary_keys": ["id.address"]},
            "sections": [{"name": "id", "fields": [
                {"field_name": "address", "rust_type_name": "Pubkey", "is_optional": false}
            ]}],
            "views": [{"id": "Position/state", "output": "Collection", "pipeline": []}]
        }]}});
        let output = build_entity_output(&descriptor, "primary:Position", None).unwrap();
        assert_eq!(output.name, "Position");
        assert_eq!(output.program_id.as_deref(), Some("Program111"));
        assert_eq!(output.primary_keys, vec!["id.address"]);
        assert_eq!(output.fields[0].path, "id.address");
        assert_eq!(output.fields[0].rust_type, "Pubkey");
        assert_eq!(output.views[0].view_id, "Position/state");
    }

    #[test]
    fn account_readiness_is_only_reported_when_transactions_need_an_entitlement() {
        let anonymous = ApiClient::with_base_url("http://127.0.0.1:9");
        assert_eq!(
            account_readiness(
                &anonymous,
                &json!({"transactionEntitlementRequired": false})
            ),
            None
        );
        let unknown =
            account_readiness(&anonymous, &json!({"transactionEntitlementRequired": true}))
                .unwrap();
        assert_eq!(unknown["transactions"]["status"], "unknown");

        let server = crate::api_client::test_support::MockServer::json_sequence(vec![
            (
                200,
                r#"{"accountKind":"agent","capabilities":["transaction_inspect"]}"#.into(),
            ),
            (404, r#"{"error":"not found"}"#.into()),
        ]);
        let client = ApiClient::with_base_url(server.base_url()).with_api_key("a4_sk_x".into());
        let required = json!({"transactionEntitlementRequired": true});
        let not_ready = account_readiness(&client, &required).unwrap();
        assert_eq!(not_ready["transactions"]["status"], "not-ready");
        assert_eq!(
            not_ready["transactions"]["missing"],
            json!(["transaction_send"])
        );
        let unknown = account_readiness(&client, &required).unwrap();
        assert_eq!(
            unknown["transactions"]["detail"],
            "this API does not report account capabilities yet"
        );
        assert!(
            render_auth_notes(&required, Some(&not_ready)).contains("(missing transaction_send)")
        );
    }

    #[test]
    fn program_operation_falls_back_to_the_idl_when_the_surface_is_unavailable() {
        let registry = MockRegistry::new(vec![
            (200, serde_json::to_string(&program_descriptor()).unwrap()),
            (404, json!({"error": "no knowledge"}).to_string()),
        ]);
        show_program(
            "demo",
            ProgramOptions {
                operation: Some("set_value"),
                sections: Vec::new(),
            },
            true,
        )
        .unwrap();
        assert!(registry
            .next_target()
            .starts_with("/api/registry/programs/demo/install?"));
        assert_eq!(
            registry.next_target(),
            "/api/registry/knowledge/programs/demo?section=surface"
        );

        let program = serde_json::to_value(program_descriptor()).unwrap();
        let output = shape::program_operation(&program, Err("no knowledge"), "setValue").unwrap();
        let rendered = render_operation(&output);
        assert!(rendered.contains("Operation: setValue  (raw-instruction, build)"));
        assert!(rendered.contains("Signers: authority"));
        assert!(rendered.contains("Knowledge surface unavailable: no knowledge"));
    }

    #[test]
    fn sections_reject_unknown_names_before_any_request() {
        let error = show_program(
            "demo",
            ProgramOptions {
                operation: None,
                sections: vec!["idl".into()],
            },
            true,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("`--section` must be one of"), "{error}");
    }

    /// A catalog stack `ore` whose primary LiveSpec reports snake_case field
    /// names, as published LiveSpecs do.
    fn catalog_descriptor() -> RegistryStackInstallResponse {
        let mut descriptor = stack_descriptor();
        descriptor.name = "ore".into();
        descriptor.stack = "ore-abc123".into();
        descriptor.live_specs[0].artifact["payload"]["entities"][0]["sections"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "results", "fields": [
                {"fieldName": "winning_square", "rustTypeName": "Option<u8>", "isOptional": true},
                {"fieldName": "pre_reveal_winning_square", "rustTypeName": "Option<u8>", "isOptional": true},
                {"fieldName": "rng", "rustTypeName": "Option<u64>", "isOptional": true}
            ]}));
        descriptor
    }

    /// The registry's catalog knowledge for [`catalog_descriptor`]: camelCase
    /// field paths, a document slug of its own, and keys this CLI does not
    /// know yet.
    fn catalog_knowledge() -> Value {
        json!({
            "kind": "stack",
            "package": "ore",
            "version": "1.0.0",
            "stackManifestHash": "manifest-exact",
            "schemaVersion": "arete.knowledge-stack/v1",
            "documentHash": "arete:h1:knowledge-document:sha256:aa",
            "slug": "ore-stream",
            "provenance": {"source": "manual", "reviewed": true},
            "stackName": "OreStream",
            "summary": "Live positions.",
            "entities": {"Position": {
                "summary": "One position.",
                "concepts": ["mining"],
                "views": {"state": {"summary": "One position by address."}},
                "fields": {
                    "results.preRevealWinningSquare": "The winning square before reveal; show this in a live UI.",
                    "results.winningSquare": "Only set once the next round opens.",
                    "results.retired": "A field this LiveSpec no longer emits."
                },
                "futureEntityKey": 1
            }},
            "futureKey": true
        })
    }

    #[test]
    fn stack_knowledge_is_read_by_catalog_slug_and_pinned_to_the_stack_manifest() {
        let registry = MockRegistry::new(vec![(200, catalog_knowledge().to_string())]);
        let client = ApiClient::new().unwrap();
        let knowledge = stack_knowledge(&client, &catalog_descriptor()).unwrap();
        assert_eq!(
            registry.next_target(),
            "/api/registry/v1/catalog/entries/stack/ore/knowledge",
            "the descriptor's package slug, never the document slug or subdomain"
        );
        assert_eq!(knowledge.slug, "ore-stream");
        drop(registry);

        let mut other_manifest = catalog_knowledge();
        other_manifest["stackManifestHash"] = json!("another-manifest");
        let mut unpinned = catalog_knowledge();
        unpinned
            .as_object_mut()
            .unwrap()
            .remove("stackManifestHash");
        for response in [
            (404, json!({"error": "Stack 'ore' not found in registry"}).to_string()),
            (
                404,
                json!({"error": "stack 'ore' has no published knowledge document", "code": "catalog-knowledge-missing"}).to_string(),
            ),
            (500, "upstream failure".to_string()),
            (200, "<html>not json</html>".to_string()),
            (200, other_manifest.to_string()),
            (200, unpinned.to_string()),
        ] {
            let _registry = MockRegistry::new(vec![response.clone()]);
            let client = ApiClient::new().unwrap();
            assert!(
                stack_knowledge(&client, &catalog_descriptor()).is_none(),
                "{response:?}"
            );
        }
    }

    #[test]
    fn stacks_that_cannot_have_catalog_knowledge_are_not_looked_up() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let client =
            ApiClient::with_base_url(&format!("http://{}", listener.local_addr().unwrap()));
        let mut private = catalog_descriptor();
        private.visibility = "private".into();
        let mut display_name = catalog_descriptor();
        display_name.name = "Ore Mining".into();
        for descriptor in [private, display_name] {
            assert!(stack_knowledge(&client, &descriptor).is_none());
        }
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "no request was made"
        );
    }

    #[test]
    fn a_slow_knowledge_route_costs_at_most_the_lookup_timeout() {
        let server = crate::api_client::test_support::MockServer::json_delayed(
            200,
            &catalog_knowledge().to_string(),
            std::time::Duration::from_secs(8),
        );
        let client = ApiClient::with_base_url(server.base_url());
        let started = std::time::Instant::now();
        assert!(stack_knowledge(&client, &catalog_descriptor()).is_none());
        let elapsed = started.elapsed();
        assert!(
            elapsed
                < arete_mcp::stack_knowledge::LOOKUP_TIMEOUT + std::time::Duration::from_secs(2),
            "{elapsed:?}"
        );
        assert_eq!(
            server.request().request_line,
            "GET /api/registry/v1/catalog/entries/stack/ore/knowledge HTTP/1.1"
        );
    }

    #[test]
    fn entity_drilldown_attaches_field_descriptions_and_summaries() {
        let descriptor = catalog_descriptor();
        let knowledge = StackKnowledge::from_response(&catalog_knowledge()).unwrap();
        let output =
            build_entity_output(&descriptor, "primary:Position", Some(&knowledge)).unwrap();
        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["summary"], "One position.");
        assert_eq!(
            json["fields"],
            json!([
                {"section": "id", "path": "id.address", "rustType": "Pubkey", "nullable": false},
                {"section": "results", "path": "results.winning_square", "rustType": "Option<u8>", "nullable": true,
                 "description": "Only set once the next round opens."},
                {"section": "results", "path": "results.pre_reveal_winning_square", "rustType": "Option<u8>", "nullable": true,
                 "description": "The winning square before reveal; show this in a live UI."},
                {"section": "results", "path": "results.rng", "rustType": "Option<u64>", "nullable": true}
            ])
        );
        assert_eq!(json["views"][0]["summary"], "One position by address.");
        assert_eq!(
            json["knowledge"],
            json!({"slug": "ore-stream", "documentHash": "arete:h1:knowledge-document:sha256:aa"})
        );
        let rendered = render_entity(&output);
        assert!(rendered.contains("  One position.\n"), "{rendered}");
        assert!(
            rendered.contains(
                "  results.pre_reveal_winning_square  Option<u8>?\n      The winning square before reveal; show this in a live UI.\n"
            ),
            "{rendered}"
        );
        assert!(
            rendered.contains("  results.rng  Option<u64>?\n\nSelected views"),
            "an undescribed field prints as before: {rendered}"
        );
        assert!(rendered.contains("  primary:Position/state  One position by address.\n"));

        // Without knowledge the output keeps exactly its previous keys.
        let plain = serde_json::to_value(
            build_entity_output(&descriptor, "primary:Position", None).unwrap(),
        )
        .unwrap();
        let mut keys = plain
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        assert_eq!(
            keys,
            [
                "fields",
                "identity",
                "kind",
                "liveAlias",
                "name",
                "primaryKeys",
                "programId",
                "schemaVersion",
                "stack",
                "views"
            ]
        );
        assert!(plain["fields"]
            .as_array()
            .unwrap()
            .iter()
            .all(|field| field.get("description").is_none()));
        assert!(plain["views"][0].get("summary").is_none());
        assert!(!render_entity(
            &build_entity_output(&descriptor, "primary:Position", None).unwrap()
        )
        .contains("One position"));
    }

    #[test]
    fn summary_views_and_full_outputs_carry_the_knowledge() {
        let descriptor = catalog_descriptor();
        let knowledge = StackKnowledge::from_response(&catalog_knowledge()).unwrap();
        let stack = serde_json::to_value(&descriptor).unwrap();

        let mut summary = shape::stack_summary(&stack, Some(&knowledge));
        let entity = &summary["entities"][0];
        assert_eq!(entity["summary"], "One position.");
        assert_eq!(entity["views"][0]["summary"], "One position by address.");
        assert_eq!(
            entity["fieldDescriptions"],
            json!([
                {"path": "results.winning_square", "description": "Only set once the next round opens."},
                {"path": "results.pre_reveal_winning_square", "description": "The winning square before reveal; show this in a live UI."}
            ])
        );
        let described = entity["fieldDescriptions"].clone();
        summary["installRef"] = json!("ore");
        summary["installCommand"] = json!("a4 install stack ore --ts");
        summary["sdkEndpoints"] = sdk_endpoints(&descriptor, None);
        let rendered = render_stack_summary(&summary);
        assert!(
            rendered.contains(
                "primary:Position  key id.address  4 field(s), 2 described\n    One position.\n"
            ),
            "{rendered}"
        );

        let mut views =
            shape::stack_views(&stack, &["primary:Position/state"], Some(&knowledge)).unwrap();
        assert_eq!(views["views"][0]["summary"], "One position by address.");
        assert_eq!(
            views["views"][0]["fields"][2]["description"],
            "The winning square before reveal; show this in a live UI."
        );
        views["installRef"] = json!("ore");
        views["sdkEndpoints"] = sdk_endpoints(&descriptor, None);
        let rendered = render_stack_views(&views);
        assert!(
            rendered.contains("  One position by address.\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("    results.winning_square  Option<u8>?\n        Only set once the next round opens.\n"),
            "{rendered}"
        );

        let output = build_stack_output("ore", &descriptor, &descriptor, Some(&knowledge)).unwrap();
        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(
            json["selectedViews"][0]["summary"],
            "One position by address."
        );
        assert!(json["selectedViews"][1].get("summary").is_none());
        assert_eq!(json["knowledge"]["slug"], "ore-stream");
        assert_eq!(
            json["knowledge"]["entities"][0],
            json!({
                "liveAlias": "primary",
                "name": "Position",
                "summary": "One position.",
                "fieldDescriptions": described
            })
        );
        assert_eq!(
            json["knowledge"]["entities"][1],
            json!({"liveAlias": "history", "name": "Position", "summary": "One position."})
        );
        let rendered = render_stack(&output);
        assert!(
            rendered.contains("    One position by address.\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("  primary:Position  2 described field(s)\n"),
            "{rendered}"
        );

        let plain = serde_json::to_value(
            build_stack_output("ore", &descriptor, &descriptor, None).unwrap(),
        )
        .unwrap();
        assert!(plain.get("knowledge").is_none());
        assert!(plain["selectedViews"][0].get("summary").is_none());
    }

    #[test]
    fn explore_prints_the_schema_whether_or_not_knowledge_is_served() {
        let descriptor = serde_json::to_string(&catalog_descriptor()).unwrap();
        for (knowledge, label) in [
            (
                Some((404, json!({"error": "not found"}).to_string())),
                "registry without the route",
            ),
            (
                Some((200, catalog_knowledge().to_string())),
                "published knowledge",
            ),
            (None, "knowledge request refused"),
        ] {
            let mut responses = vec![(200, descriptor.clone()), (200, descriptor.clone())];
            responses.extend(knowledge.clone());
            let registry = MockRegistry::new(responses);
            show_stack(
                "ore",
                StackOptions {
                    entity: Some("primary:Position"),
                    config_path: "/definitely/missing/arete.toml",
                    ..Default::default()
                },
                true,
            )
            .unwrap_or_else(|error| panic!("{label}: {error:#}"));
            assert_eq!(
                registry.next_target(),
                install_target("ore", None),
                "{label}"
            );
            assert_eq!(
                registry.next_target(),
                install_target("ore", Some("rust")),
                "{label}"
            );
            if knowledge.is_some() {
                assert_eq!(
                    registry.next_target(),
                    "/api/registry/v1/catalog/entries/stack/ore/knowledge",
                    "{label}"
                );
            }
        }
    }

    #[test]
    fn compact_renderers_show_the_essentials() {
        let stack = serde_json::to_value(stack_descriptor()).unwrap();
        let mut summary = shape::stack_summary(&stack, None);
        summary["installRef"] = json!("multi-stack");
        summary["installCommand"] = json!("a4 install stack multi-stack --ts");
        summary["sdkEndpoints"] = sdk_endpoints(&stack_descriptor(), None);
        let rendered = render_stack_summary(&summary);
        assert!(rendered.contains("primary:Position  key id.address  1 field(s)"));
        assert!(rendered.contains("views: Position/state (collection)"));
        assert!(rendered.contains("a4 install stack multi-stack --ts"));

        let mut views = shape::stack_views(&stack, &["primary:Position/state"], None).unwrap();
        views["installRef"] = json!("multi-stack");
        views["sdkEndpoints"] = sdk_endpoints(&stack_descriptor(), None);
        let rendered = render_stack_views(&views);
        assert!(rendered.contains("View primary:Position/state  (entity Position)"));
        assert!(rendered.contains("id.address  Pubkey"));

        let program = serde_json::to_value(program_descriptor()).unwrap();
        let mut sections = shape::program_sections(
            &program,
            Err("log in"),
            &["accounts".into(), "types".into(), "operations".into()],
        );
        sections["installCommand"] = json!("a4 install program demo --ts");
        let rendered = render_program_sections(&sections);
        assert!(rendered.contains("Vault  discriminator: [1,2]"));
        assert!(rendered.contains("Mode (enum)  On | Off"));
        assert!(rendered.contains("unavailable: log in"));
        assert!(!rendered.contains("\nInstructions\n"));
    }
}
