//! `a4 protocol`: self-serve protocol releases and deployments.
//!
//! `a4 protocol deploy` builds the project with `arete-protocol build`
//! (unless `--no-build`, which uploads the existing output after checking
//! that the sources have not changed since it was built), uploads the spec,
//! plus the handler bundle when a step has a handler, and takes it to a
//! private deployment: it ensures the protocol exists, uploads the release,
//! reuses or creates a deployment (fee policy and resources included) and
//! activates it. The server derives everything else from the spec, so the
//! release hash is predicted locally with `arete-hash` and checked against
//! the server's answer.
//!
//! Programs the project installed from a local IDL are pushed to the account
//! first ([`local_programs`]): the release's templates, stack and package
//! resolve only programs Arete has. arete.lock records each admitted release
//! and the project is rebuilt with it.
//!
//! When the project installs its own protocol (`[dependencies.protocols]`
//! with this slug, which `a4 dev` adds), an active deployment then updates
//! that client: `a4 update protocol <alias>` from the registry, or, while
//! the registry has no package release for this deployment yet, the client
//! is generated from the deployed build with the new deployment id and the
//! lock records it as a local build.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use arete_hash::protocol::{
    derive_server_release, encode_base64, parse_contract_json, validate_protocol_spec_value,
    ProtocolSpecV1,
};
use clap::{Args, Subcommand};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api_client::{
    api_error_details, ApiClient, ProtocolDeployment, ProtocolRecord, ProtocolReleaseUpload,
};
use crate::ui;

/// Where `arete-protocol build` writes its output, relative to the project.
pub const PROTOCOL_BUILD_DIR: &str = ".arete/protocol";
const SPEC_FILE: &str = "protocol-spec.json";
const BUNDLE_FILE: &str = "handlers.mjs";
const RECEIPT_FILE: &str = "build-receipt.json";
const SOURCES_FILE: &str = "build-sources.json";
const SOURCES_SCHEMA: &str = "arete.protocol-build-sources/v1";
/// Changed files named in a stale-build error before the rest are counted.
const MAX_NAMED_CHANGES: usize = 10;
const FEE_POLICY_SCHEMA: &str = "arete.protocol-fee-policy/v1";
const MAX_SLUG_LEN: usize = 63;

pub mod init;
pub mod listing;
pub mod local_programs;
pub mod squads;
pub mod webhook;

#[derive(Subcommand, Debug)]
pub enum ProtocolCommands {
    /// Scaffold a protocol project: a template-only tip jar with the System
    /// Program installed from the registry (`--local-idl` explicitly opts out)
    Init(init::InitArgs),
    /// Upload a built protocol and deploy it privately
    Deploy(DeployArgs),
    /// What a wallet can do now on a deployment: create steps and the
    /// instances it relates to, with each step's availability and args
    Actions(ActionsArgs),
    /// Search the callable protocol packages listed on Arete (trust tier,
    /// attested executions, steps, package)
    Search(listing::SearchArgs),
    /// Show one listed protocol package and its install command
    Show(listing::ShowArgs),
    /// Ask for your deployment to be listed in the public registry
    RequestListing(listing::RequestListingArgs),
    /// Show your deployment's listing state
    ListingStatus(listing::ListingArgs),
    /// Manage the webhook endpoints that receive your deployment's events
    #[command(subcommand)]
    Webhook(webhook::WebhookCommands),
    /// Bring your own Squads v4 multisig: the account's proposer, whether
    /// it can propose to a multisig, and the transaction that adds it
    #[command(subcommand)]
    Squads(squads::SquadsCommands),
}

/// Options for `a4 protocol actions`.
#[derive(Args, Debug, Clone, Default)]
pub struct ActionsArgs {
    /// Protocol package reference (e.g. `token-raise`) or deployment id;
    /// optional with `--dev`, which defaults to the session's deployment
    pub target: Option<String>,

    /// The wallet's public key (with `--dev` also a dev wallet name)
    #[arg(long)]
    pub wallet: String,

    /// Only this instance
    #[arg(long)]
    pub instance: Option<String>,

    /// `nextCursor` of the previous page
    #[arg(long)]
    pub cursor: Option<String>,

    /// Ask the local `a4 dev` session instead of Arete
    #[arg(long)]
    pub dev: bool,

    /// The `a4 dev` project directory (implies `--dev`)
    #[arg(long, value_name = "DIR")]
    pub project: Option<String>,
}

/// Options for `a4 protocol deploy`.
#[derive(Args, Debug, Clone, Default)]
pub struct DeployArgs {
    /// Protocol project directory (holds `.arete/protocol/` and `arete.toml`)
    #[arg(default_value = ".")]
    pub dir: PathBuf,

    /// Upload the existing `.arete/protocol/` output instead of building
    /// first; refused when the sources changed since that build
    #[arg(long)]
    pub no_build: bool,

    /// Print the plan without changing anything on the server
    #[arg(long)]
    pub dry_run: bool,

    /// Protocol slug (default: [protocol].slug, else derived from the spec name)
    #[arg(long)]
    pub slug: Option<String>,

    /// Protocol display name (default: [protocol].name, else the spec name)
    #[arg(long)]
    pub name: Option<String>,

    /// Fee policy document (JSON, `arete.protocol-fee-policy/v1`) for a new deployment
    #[arg(long, value_name = "FILE")]
    pub fee_policy: Option<PathBuf>,

    /// Value of a resource the spec declares, as NAME=FILE (JSON); repeatable
    #[arg(long = "resource", value_name = "NAME=FILE")]
    pub resources: Vec<String>,

    /// Seconds to keep re-trying activation while the server answers that it
    /// is still preparing the deployment (0 = activate once), and to wait for
    /// each pushed program's admission
    #[arg(long, default_value_t = 300)]
    pub wait_secs: u64,

    /// Create or reuse the deployment but do not activate it
    #[arg(long)]
    pub no_activate: bool,
}

pub fn run(command: ProtocolCommands, json: bool) -> Result<()> {
    match command {
        ProtocolCommands::Init(args) => init::init(&args, json),
        ProtocolCommands::Deploy(args) => deploy(&args, json),
        ProtocolCommands::Actions(args) => actions(&args, json),
        ProtocolCommands::Search(args) => listing::search(&args, json),
        ProtocolCommands::Show(args) => listing::show(&args, json),
        ProtocolCommands::RequestListing(args) => listing::request_listing(&args, json),
        ProtocolCommands::ListingStatus(args) => listing::listing_status(&args, json),
        ProtocolCommands::Webhook(command) => webhook::run(command, json),
        ProtocolCommands::Squads(command) => squads::run(command, json),
    }
}

/// The `commands` an activation refusal names when a step's Squads proposer
/// cannot propose to its multisig yet (`commands.addProposer`,
/// `commands.status`), as one line to print.
fn squads_activation_commands(error: &anyhow::Error) -> Option<String> {
    let api_error = error.downcast_ref::<crate::api_client::ApiClientError>()?;
    // The account setup refusal already prints its link and commands.
    if api_error.problem.code.as_deref() == Some(crate::api_client::SQUADS_SETUP_REQUIRED) {
        return None;
    }
    let commands = api_error.problem.extra.get("commands")?;
    let add = commands["addProposer"].as_str();
    let status = commands["status"].as_str();
    let mut parts = Vec::new();
    if let Some(add) = add {
        parts.push(format!("Add the Squads proposer: `{add}`"));
    }
    if let Some(status) = status {
        parts.push(format!("check it: `{status}`"));
    }
    (!parts.is_empty()).then(|| format!("{}.", parts.join("; then ")))
}

/// Whether `target` names a deployment (an id) rather than a protocol
/// package.
fn is_deployment_id(target: &str) -> bool {
    uuid::Uuid::parse_str(target).is_ok()
}

/// The tool target of `a4 protocol actions`.
pub fn actions_target(args: &ActionsArgs) -> Result<arete_mcp::protocol_agent::TargetArgs> {
    protocol_target(args.target.as_deref(), args.dev, args.project.as_deref())
}

/// A deployment target as `a4 protocol actions` and `a4 protocol squads`
/// take it: a deployment id or a protocol package reference, or, with
/// `--dev` / `--project`, by default the `a4 dev` session's deployment.
pub fn protocol_target(
    target: Option<&str>,
    dev: bool,
    project: Option<&str>,
) -> Result<arete_mcp::protocol_agent::TargetArgs> {
    let dev = dev || project.is_some();
    let (deployment, protocol) = match target.map(str::trim) {
        Some(target) if !target.is_empty() => {
            if is_deployment_id(target) {
                (Some(target.to_string()), None)
            } else {
                (None, Some(target.to_string()))
            }
        }
        _ if dev => (None, None),
        _ => bail!("Name a protocol package (e.g. token-raise) or a deployment id, or pass --dev"),
    };
    Ok(arete_mcp::protocol_agent::TargetArgs {
        dev,
        project: project.map(str::to_string),
        deployment,
        protocol,
    })
}

/// `a4 protocol actions`.
pub fn actions(args: &ActionsArgs, json: bool) -> Result<()> {
    let result = fetch_actions(&arete_mcp::protocol_agent::ProtocolAgent::new(None), args)?;
    if json {
        println!("{result}");
    } else {
        print!("{}", render_actions(&result));
    }
    Ok(())
}

/// Read the wallet's actions through `agent`.
pub fn fetch_actions(
    agent: &arete_mcp::protocol_agent::ProtocolAgent,
    args: &ActionsArgs,
) -> Result<Value> {
    let target = actions_target(args)?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the async runtime")?
        .block_on(agent.actions(
            &target,
            &args.wallet,
            args.instance.as_deref(),
            args.cursor.as_deref(),
        ))
}

fn render_step(step: &Value) -> String {
    let name = step["step"].as_str().unwrap_or("?");
    let mut line = if step["allowed"] == true {
        format!("    {} {name}", "✓".green())
    } else {
        format!(
            "    {} {name} ({})",
            "✗".red(),
            step["reason"].as_str().unwrap_or("not allowed")
        )
    };
    let args: Vec<String> = step["args"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|arg| {
            format!(
                "{}{}: {}",
                arg["name"].as_str().unwrap_or("?"),
                if arg["optional"] == true { "?" } else { "" },
                arg_type(arg)
            )
        })
        .collect();
    if !args.is_empty() {
        line.push_str(&format!("  args {{ {} }}", args.join(", ")));
    }
    let dependent: Vec<&str> = step["argsDependent"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !dependent.is_empty() {
        line.push_str(&format!("  decided at prepare: {}", dependent.join(", ")));
    }
    line
}

/// An arg's type as a protocol author writes it: the field schema's
/// `format` (`u64`, `pubkey`, `unixTimestamp`) when it has one, else its
/// JSON `type`, with `[]` for arrays.
fn arg_type(arg: &Value) -> String {
    let base = arg["format"]
        .as_str()
        .or_else(|| arg["type"].as_str())
        .unwrap_or("?");
    if base == "array" {
        let items = arg_type(&arg["items"]);
        return format!("{items}[]");
    }
    base.to_string()
}

/// The human rendering of a wallet-actions answer.
pub fn render_actions(result: &Value) -> String {
    let mut out = format!(
        "Wallet {} on deployment {}\n",
        result["wallet"].as_str().unwrap_or("?"),
        result["deploymentId"].as_str().unwrap_or("?")
    );
    let create: Vec<&Value> = result["createSteps"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    out.push_str("\n  Create steps\n");
    if create.is_empty() {
        out.push_str("    (none)\n");
    }
    for step in create {
        out.push_str(&render_step(step));
        out.push('\n');
    }
    let instances: Vec<&Value> = result["instances"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    if instances.is_empty() {
        out.push_str("\n  No instances relate to this wallet.\n");
    }
    for instance in instances {
        let relation: Vec<&str> = instance["relation"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        out.push_str(&format!(
            "\n  Instance {} [{}] {}{}\n",
            instance["instanceId"].as_str().unwrap_or("?"),
            instance["state"].as_str().unwrap_or("?"),
            relation.join(", "),
            instance["participantState"]
                .as_str()
                .map(|state| format!(" (participant: {state})"))
                .unwrap_or_default()
        ));
        for step in instance["steps"].as_array().into_iter().flatten() {
            out.push_str(&render_step(step));
            out.push('\n');
        }
    }
    if let Some(cursor) = result["nextCursor"].as_str().filter(|c| !c.is_empty()) {
        out.push_str(&format!("\n  More instances: --cursor {cursor}\n"));
    }
    out
}

/// `a4 protocol deploy`.
pub fn deploy(args: &DeployArgs, json: bool) -> Result<()> {
    let client = ApiClient::new()?;
    let outcome = execute(&client, args, json)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&outcome)?);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// The `[protocol]` table of the project's `arete.toml`. Paths are relative
/// to the project directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProtocolConfig {
    pub slug: Option<String>,
    pub name: Option<String>,
    pub fee_policy: Option<PathBuf>,
    pub resources: BTreeMap<String, ResourceSource>,
    pub access: Option<String>,
}

/// Where a resource value comes from: a JSON file, or the value itself
/// written inline in `[protocol.resources]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceSource {
    File(PathBuf),
    Inline(Value),
}

impl ResourceSource {
    /// Resolve a relative file path against `base`; inline values are kept.
    pub fn relative_to(&self, base: &Path) -> Self {
        match self {
            Self::File(path) => Self::File(base.join(path)),
            Self::Inline(value) => Self::Inline(value.clone()),
        }
    }

    /// The resource value, checked as contract JSON.
    pub fn read(&self, name: &str) -> Result<Value> {
        match self {
            Self::File(path) => read_contract_document(path, &format!("Resource {name}")),
            Self::Inline(value) => Ok(value.clone()),
        }
    }
}

/// `--resource NAME=FILE` entries.
pub fn parse_resource_flags(entries: &[String]) -> Result<Vec<(String, ResourceSource)>> {
    entries
        .iter()
        .map(|entry| {
            let (name, path) = entry
                .split_once('=')
                .filter(|(name, path)| !name.is_empty() && !path.is_empty())
                .ok_or_else(|| anyhow::anyhow!("--resource expects NAME=FILE, got '{entry}'"))?;
            Ok((name.to_string(), ResourceSource::File(PathBuf::from(path))))
        })
        .collect()
}

/// The error for a resource the spec declares but nothing supplies.
pub fn missing_resource_help(name: &str) -> String {
    format!(
        "The spec declares resource '{name}' but no value was given. Pass --resource {name}=<file.json>, \
         or set it under [protocol.resources] in arete.toml, either as a path to a JSON file relative \
         to the project ({name} = \"resources/{name}.json\") or as an inline TOML value \
         ({name} = {{ someField = 1 }}). The value must match the resource's schema in the spec"
    )
}

/// Read `[protocol]` from `<dir>/arete.toml`; absent file or table means
/// defaults.
pub fn load_protocol_config(dir: &Path) -> Result<ProtocolConfig> {
    let path = dir.join("arete.toml");
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ProtocolConfig::default())
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read {}", path.display()))
        }
    };
    // Errors print only their outermost message, so the cause goes into it.
    parse_protocol_config(&source)
        .map_err(|error| anyhow::anyhow!("Invalid {}: {error:#}", path.display()))
}

/// `arete.toml` as this command reads it: only the `[protocol]` table.
#[derive(Deserialize)]
struct ProtocolManifest {
    #[serde(default)]
    protocol: Option<RawProtocolConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProtocolConfig {
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    fee_policy: Option<PathBuf>,
    #[serde(default)]
    resources: BTreeMap<String, toml::Value>,
    #[serde(default)]
    access: Option<String>,
}

fn parse_protocol_config(source: &str) -> Result<ProtocolConfig> {
    let manifest: ProtocolManifest = toml::from_str(source).map_err(|error| {
        anyhow::anyhow!(
            "{}\n[protocol] accepts the keys slug, name, fee_policy, resources and access",
            error.to_string().trim_end()
        )
    })?;
    let Some(raw) = manifest.protocol else {
        return Ok(ProtocolConfig::default());
    };
    let mut resources = BTreeMap::new();
    for (name, value) in raw.resources {
        let source = resource_source(&value).with_context(|| {
            format!("[protocol.resources].{name} is not a valid resource value")
        })?;
        resources.insert(name, source);
    }
    if let Some(access) = raw.access.as_deref() {
        if access != "private" {
            bail!(
                "[protocol].access = \"{access}\" is not supported: self-serve deployments are private. \
                 Public deployments are created by an operator."
            );
        }
    }
    Ok(ProtocolConfig {
        slug: raw.slug,
        name: raw.name,
        fee_policy: raw.fee_policy,
        resources,
        access: raw.access,
    })
}

/// A string is a path to a JSON file; any other TOML value is the resource
/// value itself.
fn resource_source(value: &toml::Value) -> Result<ResourceSource> {
    if let toml::Value::String(path) = value {
        if path.is_empty() {
            bail!("the file path is empty");
        }
        return Ok(ResourceSource::File(PathBuf::from(path)));
    }
    // Inline values follow the same JSON rules as a resource file.
    let bytes = serde_json::to_vec(&toml_to_json(value)?)?;
    Ok(ResourceSource::Inline(parse_contract_json(&bytes)?))
}

fn toml_to_json(value: &toml::Value) -> Result<Value> {
    Ok(match value {
        toml::Value::String(text) => Value::String(text.clone()),
        toml::Value::Integer(number) => Value::from(*number),
        toml::Value::Float(number) => serde_json::Number::from_f64(*number)
            .map(Value::Number)
            .ok_or_else(|| anyhow::anyhow!("{number} is not a finite number"))?,
        toml::Value::Boolean(flag) => Value::Bool(*flag),
        toml::Value::Datetime(datetime) => {
            bail!("TOML dates are not JSON values; write it as a string (\"{datetime}\")")
        }
        toml::Value::Array(items) => {
            Value::Array(items.iter().map(toml_to_json).collect::<Result<_>>()?)
        }
        toml::Value::Table(table) => Value::Object(
            table
                .iter()
                .map(|(key, item)| Ok((key.clone(), toml_to_json(item)?)))
                .collect::<Result<_>>()?,
        ),
    })
}

/// Flags over config; config paths resolve against the project directory,
/// flag paths against the working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Settings {
    slug: String,
    name: String,
    fee_policy: Option<PathBuf>,
    resources: BTreeMap<String, ResourceSource>,
}

fn resolve_settings(
    args: &DeployArgs,
    config: &ProtocolConfig,
    spec: &ProtocolSpecV1,
) -> Result<Settings> {
    let slug = match args.slug.as_deref().or(config.slug.as_deref()) {
        Some(slug) => {
            validate_slug(slug)?;
            slug.to_string()
        }
        None => derive_slug(&spec.name).with_context(|| {
            format!(
                "Cannot derive a protocol slug from spec name '{}'; pass --slug",
                spec.name
            )
        })?,
    };
    let name = args
        .name
        .clone()
        .or_else(|| config.name.clone())
        .unwrap_or_else(|| spec.name.clone());
    if name.trim().is_empty() {
        bail!("The protocol name must not be empty");
    }
    let fee_policy = args
        .fee_policy
        .clone()
        .or_else(|| config.fee_policy.as_ref().map(|path| args.dir.join(path)));
    let mut resources: BTreeMap<String, ResourceSource> = config
        .resources
        .iter()
        .map(|(name, source)| (name.clone(), source.relative_to(&args.dir)))
        .collect();
    resources.extend(parse_resource_flags(&args.resources)?);
    for name in resources.keys() {
        if !spec.resources.contains_key(name) {
            let declared = spec.resources.keys().cloned().collect::<Vec<_>>();
            bail!(
                "Resource '{name}' is not declared by the spec (declared: {})",
                if declared.is_empty() {
                    "none".to_string()
                } else {
                    declared.join(", ")
                }
            );
        }
    }
    Ok(Settings {
        slug,
        name,
        fee_policy,
        resources,
    })
}

/// A protocol slug: `^[a-z0-9][a-z0-9-]{0,62}$`.
pub fn validate_slug(slug: &str) -> Result<()> {
    let valid = !slug.is_empty()
        && slug.len() <= MAX_SLUG_LEN
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !slug.starts_with('-');
    if !valid {
        bail!(
            "Invalid protocol slug '{slug}': use 1-63 lowercase letters, digits and '-', starting with a letter or digit"
        );
    }
    Ok(())
}

/// A slug from a spec name: camelCase words split with `-`, lowercased,
/// every other character replaced by `-`, runs collapsed and trimmed.
pub fn derive_slug(name: &str) -> Result<String> {
    let mut slug = String::new();
    let mut previous: Option<char> = None;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            let boundary = character.is_ascii_uppercase()
                && previous.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit());
            if boundary && !slug.ends_with('-') {
                slug.push('-');
            }
            slug.push(character.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        previous = Some(character);
    }
    slug.truncate(MAX_SLUG_LEN);
    let slug = slug.trim_matches('-').to_string();
    validate_slug(&slug)?;
    Ok(slug)
}

// ---------------------------------------------------------------------------
// Local build output
// ---------------------------------------------------------------------------

/// The built protocol: its validated spec, optional handler bundle and the
/// release hash the server is expected to derive.
#[derive(Debug, Clone)]
pub struct LocalRelease {
    pub spec_value: Value,
    pub spec: ProtocolSpecV1,
    pub spec_path: PathBuf,
    pub bundle: Option<Vec<u8>>,
    pub bundle_path: Option<PathBuf>,
    pub expected_release_hash: String,
}

pub fn load_local_release(dir: &Path) -> Result<LocalRelease> {
    let build_dir = dir.join(PROTOCOL_BUILD_DIR);
    let spec_path = build_dir.join(SPEC_FILE);
    let bytes = match std::fs::read(&spec_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!(
            "No built protocol at {}. Run `a4 protocol deploy` without --no-build, or `npx arete-protocol build` in the project.",
            spec_path.display()
        ),
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read {}", spec_path.display()))
        }
    };
    let spec_value = parse_contract_json(&bytes)
        .with_context(|| format!("{} is not valid contract JSON", spec_path.display()))?;
    let spec = validate_protocol_spec_value(&spec_value)
        .with_context(|| format!("{} is not a valid protocol spec", spec_path.display()))?;

    let candidate = build_dir.join(BUNDLE_FILE);
    let bundle = match std::fs::read(&candidate) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read {}", candidate.display()))
        }
    };
    match (spec.has_handlers(), bundle.is_some()) {
        (true, false) => bail!(
            "The spec has handler steps but {} is missing. Rebuild with `npx arete-protocol build`.",
            candidate.display()
        ),
        (false, true) => bail!(
            "The spec has no handler steps but {} exists (a stale build output?). Rebuild, or delete it.",
            candidate.display()
        ),
        _ => {}
    }
    let derived = derive_server_release(&spec, bundle.as_deref()).map_err(|error| {
        anyhow::anyhow!(
            "The release cannot be derived from this spec ({}): {error}",
            error.code()
        )
    })?;
    let bundle_path = bundle.as_ref().map(|_| candidate);
    Ok(LocalRelease {
        spec_value,
        spec,
        spec_path,
        bundle,
        bundle_path,
        expected_release_hash: derived.release_hash.to_string(),
    })
}

/// `build-sources.json`: the sources behind the receipt's source-tree hash.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuildSources {
    schema: String,
    source_tree_sha256: String,
    files: Vec<BuildSource>,
}

#[derive(Debug, Deserialize)]
struct BuildSource {
    path: String,
    sha256: String,
}

/// One source that differs from the build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedSource {
    pub path: String,
    /// `changed` or `deleted`.
    pub change: &'static str,
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

/// The source-tree hash `arete-protocol build` records: SHA-256 of the JCS
/// of `[[path, sha256], ...]` in build order.
fn source_tree_sha256(files: &[BuildSource]) -> Result<String> {
    let entries: Vec<[&str; 2]> = files
        .iter()
        .map(|file| [file.path.as_str(), file.sha256.as_str()])
        .collect();
    let bytes = arete_hash::canonicalize_jcs(&entries)
        .map_err(|error| anyhow::anyhow!("Failed to canonicalize the source list: {error}"))?;
    Ok(sha256_hex(&bytes))
}

/// The project sources (relative to `dir`) that differ from the build in
/// `dir/.arete/protocol`, read from its `build-sources.json` and checked
/// against the receipt's `sourceTreeSha256`.
pub fn changed_sources(dir: &Path) -> Result<Vec<ChangedSource>> {
    let build_dir = dir.join(PROTOCOL_BUILD_DIR);
    let read = |name: &str| -> Result<Option<Vec<u8>>> {
        let path = build_dir.join(name);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("Failed to read {}", path.display())),
        }
    };
    let rebuild = "Run `a4 protocol deploy` without --no-build to build and deploy";
    let Some(receipt) = read(RECEIPT_FILE)? else {
        bail!(
            "{} has no {RECEIPT_FILE}, so --no-build cannot tell whether it is current. {rebuild}.",
            build_dir.display()
        );
    };
    let receipt: Value = serde_json::from_slice(&receipt)
        .with_context(|| format!("{RECEIPT_FILE} in {} is not JSON", build_dir.display()))?;
    let recorded = receipt["sourceTreeSha256"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("{RECEIPT_FILE} has no sourceTreeSha256"))?;
    let Some(sources) = read(SOURCES_FILE)? else {
        bail!(
            "{} has no {SOURCES_FILE} (an older arete-protocol built it), so --no-build cannot tell whether it is current. {rebuild}.",
            build_dir.display()
        );
    };
    let sources: BuildSources = serde_json::from_slice(&sources)
        .with_context(|| format!("{SOURCES_FILE} in {} is invalid", build_dir.display()))?;
    if sources.schema != SOURCES_SCHEMA {
        bail!(
            "{SOURCES_FILE} has schema '{}'; this a4 reads '{SOURCES_SCHEMA}'. {rebuild}.",
            sources.schema
        );
    }
    if sources.source_tree_sha256 != recorded || source_tree_sha256(&sources.files)? != recorded {
        bail!(
            "{SOURCES_FILE} does not match the source tree in {RECEIPT_FILE}: the build output was edited or mixed from two builds. {rebuild}."
        );
    }
    let mut changed = Vec::new();
    for file in &sources.files {
        let relative = Path::new(&file.path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            bail!("{SOURCES_FILE} names '{}', outside the project", file.path);
        }
        match std::fs::read(dir.join(relative)) {
            Ok(bytes) if sha256_hex(&bytes) == file.sha256 => {}
            Ok(_) => changed.push(ChangedSource {
                path: file.path.clone(),
                change: "changed",
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                changed.push(ChangedSource {
                    path: file.path.clone(),
                    change: "deleted",
                })
            }
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to read {}", file.path))
            }
        }
    }
    Ok(changed)
}

/// `--no-build`: refuse a build whose sources changed since it was made.
fn check_build_is_current(dir: &Path) -> Result<()> {
    let changed = changed_sources(dir)?;
    if changed.is_empty() {
        return Ok(());
    }
    let mut named: Vec<String> = changed
        .iter()
        .take(MAX_NAMED_CHANGES)
        .map(|source| format!("{} ({})", source.path, source.change))
        .collect();
    if changed.len() > MAX_NAMED_CHANGES {
        named.push(format!("and {} more", changed.len() - MAX_NAMED_CHANGES));
    }
    bail!(
        "The build in {} is stale: {} source file(s) changed since it was built: {}. Run `a4 protocol deploy` without --no-build to rebuild, or `npx arete-protocol build` first.",
        dir.join(PROTOCOL_BUILD_DIR).display(),
        changed.len(),
        named.join(", ")
    )
}

fn read_contract_document(path: &Path, what: &str) -> Result<Value> {
    let bytes =
        std::fs::read(path).with_context(|| format!("Failed to read {what} {}", path.display()))?;
    parse_contract_json(&bytes)
        .with_context(|| format!("{what} {} is not valid contract JSON", path.display()))
}

fn run_build(dir: &Path, json: bool) -> Result<()> {
    let bin = super::protocol_bin::resolve(dir)?;
    if !json {
        ui::print_step("Running arete-protocol build");
    }
    let mut command = super::protocol_bin::command(&bin);
    command.arg("build").current_dir(dir).stdin(Stdio::null());
    // Keep stdout clean for the JSON result.
    if json {
        command.stdout(Stdio::from(std::io::stderr()));
    }
    let status = command
        .status()
        .context("Failed to run arete-protocol build")?;
    if !status.success() {
        bail!("arete-protocol build failed ({status})");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployOutcome {
    /// `active`, `activating`, `disabled`, `review-required` or `dry-run`.
    pub status: String,
    pub dry_run: bool,
    pub protocol: ProtocolOutcome,
    pub release: ReleaseOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment: Option<DeploymentOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<Value>,
    /// Dry run only: what a real run would do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanOutcome>,
    pub files: Vec<String>,
    pub warnings: Vec<String>,
    /// The project's installed client of this protocol, updated to the new
    /// deployment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<ClientUpdate>,
    /// The activation's owner-private registry package: `status`
    /// (`published`, `existing` or `skipped` with `code` and `message`),
    /// `package`, `version`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_package: Option<Value>,
    /// The programs installed from a local IDL, and how each reached the
    /// platform.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub programs: Vec<local_programs::LocalProgramOutcome>,
}

/// How `a4 protocol deploy` updated the project's own protocol client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientUpdate {
    pub alias: String,
    /// `reinstalled` (the published package release of this deployment) or
    /// `local-build` (generated from the deployed build: no package release
    /// yet).
    pub mode: String,
    pub deployment_id: String,
    pub outputs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolOutcome {
    pub slug: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Whether this run created the protocol (`null` in a dry run).
    pub created: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseOutcome {
    pub expected_release_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_hash: Option<String>,
    pub has_handlers: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_check: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentOutcome {
    #[serde(flatten)]
    pub deployment: ProtocolDeployment,
    /// Whether this run created the deployment.
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanOutcome {
    /// Whether the server was consulted (a secret key was available).
    pub checked_remote: bool,
    /// `create`, `exists` or `unknown`.
    pub protocol: String,
    /// `create`, `reuse` or `unknown`.
    pub deployment: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reuse_deployment_id: Option<String>,
    pub activation: String,
    /// Inputs a new deployment needs but that are not configured.
    pub missing_inputs: Vec<String>,
}

// ---------------------------------------------------------------------------
// Deploy
// ---------------------------------------------------------------------------

struct Reporter {
    json: bool,
    warnings: Vec<String>,
}

impl local_programs::Progress for Reporter {
    fn step(&self, message: &str) {
        Reporter::step(self, message)
    }
    fn success(&self, message: &str) {
        Reporter::success(self, message)
    }
    fn info(&self, message: &str) {
        Reporter::info(self, message)
    }
}

impl Reporter {
    fn step(&self, message: &str) {
        if !self.json {
            ui::print_step(message);
        }
    }
    fn success(&self, message: &str) {
        if !self.json {
            ui::print_success(message);
        }
    }
    fn info(&self, message: &str) {
        if !self.json {
            ui::print_info(message);
        }
    }
    fn warn(&mut self, message: String) {
        if !self.json {
            ui::print_warning(&message);
        }
        self.warnings.push(message);
    }
}

/// Run the deploy flow against `client`. Prints progress unless `json`.
pub fn execute(client: &ApiClient, args: &DeployArgs, json: bool) -> Result<DeployOutcome> {
    if args.no_build {
        check_build_is_current(&args.dir)?;
    } else {
        run_build(&args.dir, json)?;
    }
    let config = load_protocol_config(&args.dir)?;
    let mut local = load_local_release(&args.dir)?;
    let settings = resolve_settings(args, &config, &local.spec)?;
    let mut reporter = Reporter {
        json,
        warnings: Vec::new(),
    };

    // 0. Programs installed from a local IDL.
    if !args.dry_run {
        require_secret_key(client)?;
    }
    let published = local_programs::publish(
        client,
        &args.dir,
        &local_programs::PublishOptions {
            slug: &settings.slug,
            dry_run: args.dry_run,
            record: !args.no_build,
            wait: std::time::Duration::from_secs(args.wait_secs),
        },
        &reporter,
    )?;
    if published.lock_changed {
        // The generated programs now pin their releases, so the spec does.
        run_build(&args.dir, json)?;
        local = load_local_release(&args.dir)?;
    }

    let has_handlers = local.spec.has_handlers();
    let mut files = vec![local.spec_path.display().to_string()];
    if let Some(path) = &local.bundle_path {
        files.push(path.display().to_string());
    }
    reporter.step(&format!(
        "Protocol {} {} ({})",
        local.spec.name.bold(),
        local.spec.version,
        if has_handlers {
            "with handler code"
        } else {
            "template-only"
        }
    ));
    for file in &files {
        reporter.info(&format!("upload {file}"));
    }
    reporter.info(&format!("expected release {}", local.expected_release_hash));

    let mut outcome = DeployOutcome {
        status: String::new(),
        dry_run: args.dry_run,
        protocol: ProtocolOutcome {
            slug: settings.slug.clone(),
            name: settings.name.clone(),
            id: None,
            created: None,
        },
        release: ReleaseOutcome {
            expected_release_hash: local.expected_release_hash.clone(),
            release_hash: None,
            has_handlers,
            created: None,
            template_check: None,
            reviewed: None,
        },
        deployment: None,
        pending: None,
        plan: None,
        files,
        warnings: Vec::new(),
        client: None,
        registry_package: None,
        programs: published.programs,
    };

    if args.dry_run {
        let plan = dry_run_plan(client, &local, &settings, args, &mut reporter);
        print_plan(&reporter, &settings, &plan);
        outcome.status = "dry-run".into();
        outcome.plan = Some(plan);
        outcome.warnings = reporter.warnings;
        return Ok(outcome);
    }

    // 1. Protocol.
    let (protocol, created) = ensure_protocol(client, &settings)?;
    if created {
        reporter.success(&format!(
            "Created protocol {} ({})",
            protocol.slug, protocol.id
        ));
    } else {
        reporter.success(&format!(
            "Using protocol {} ({})",
            protocol.slug, protocol.id
        ));
    }
    outcome.protocol.id = Some(protocol.id.clone());
    outcome.protocol.created = Some(created);
    outcome.protocol.name = protocol.name.clone();

    // 2. Release.
    let bundle_base64 = local.bundle.as_deref().map(encode_base64);
    let upload: ProtocolReleaseUpload = client.upload_protocol_release(
        &protocol.id,
        &local.spec_value,
        bundle_base64.as_deref(),
    )?;
    reporter.success(&format!(
        "Release {} {}",
        upload.release_hash,
        if upload.created {
            "uploaded"
        } else {
            "already registered"
        }
    ));
    if upload.release_hash != local.expected_release_hash {
        reporter.warn(format!(
            "The server derived release {} but this CLI expected {}; the server may use a newer derivation. Continuing with the server's release.",
            upload.release_hash, local.expected_release_hash
        ));
    }
    outcome.release.release_hash = Some(upload.release_hash.clone());
    outcome.release.created = Some(upload.created);
    outcome.release.template_check = Some(upload.template_check.clone());
    outcome.release.reviewed = Some(upload.reviewed);
    outcome.release.has_handlers = upload.has_handlers;
    match upload.template_check.as_str() {
        "failed" => bail!(
            "The server's template check failed for release {}: a template does not match the program specs it targets. Fix the templates, rebuild and deploy again.",
            upload.release_hash
        ),
        "pending" => reporter.warn(
            "The template check is pending: activation waits until the server has the program specs the templates target.".into(),
        ),
        _ => {}
    }

    // 3. Deployment.
    let existing = client.list_protocol_deployments(&protocol.id)?;
    let (deployment, deployment_created) =
        match select_reusable_deployment(&existing, &upload.release_hash) {
            Some(deployment) => {
                reporter.success(&format!(
                    "Reusing deployment {} ({})",
                    deployment.short_id, deployment.status
                ));
                if settings.fee_policy.is_some() || !settings.resources.is_empty() {
                    reporter.info(
                        "fee policy and resource inputs apply only when a deployment is created",
                    );
                }
                (deployment.clone(), false)
            }
            None => {
                let deployment =
                    create_deployment(client, &protocol.id, &upload, &local, &settings, &reporter)?;
                reporter.success(&format!("Created deployment {}", deployment.short_id));
                (deployment, true)
            }
        };

    // 4. Activation.
    let mut deployment = deployment;
    let mut status = deployment.status.clone();
    if deployment.status == "active" && args.no_activate {
        reporter.success("Deployment is active");
    } else if deployment.status == "active" {
        // Already active: activating again is idempotent and (re)publishes
        // the release's registry package, so its status is reported too.
        match client.activate_protocol_deployment(&deployment.id) {
            Ok(transition) => outcome.registry_package = transition.registry_package,
            Err(error) => reporter.warn(format!(
                "Could not read the registry package of the active deployment: {error:#}"
            )),
        }
        reporter.success("Deployment is active");
        report_registry_package(&reporter, outcome.registry_package.as_ref());
    } else if args.no_activate {
        reporter.info("not activated (--no-activate)");
    } else {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(args.wait_secs);
        let mut activation = client.activate_protocol_deployment(&deployment.id);
        // 202: the server is still preparing the deployment (loading handler
        // code, deploying the stack). Activating again is idempotent, so poll
        // until it is active or the wait runs out.
        while let Ok(transition) = &activation {
            if transition.deployment.status == "active"
                || transition.pending.is_none()
                || std::time::Instant::now() >= deadline
            {
                break;
            }
            if let Some(pending) = &transition.pending {
                reporter.info(&format!("waiting: {pending}"));
            }
            std::thread::sleep(std::time::Duration::from_secs(3));
            activation = client.activate_protocol_deployment(&deployment.id);
        }
        match activation {
            Ok(transition) => {
                deployment = transition.deployment;
                status = deployment.status.clone();
                outcome.pending = transition.pending;
                outcome.registry_package = transition.registry_package;
                if status == "active" {
                    reporter.success("Deployment is active");
                    report_registry_package(&reporter, outcome.registry_package.as_ref());
                } else {
                    reporter.success(&format!("Deployment is {status}"));
                    if let Some(pending) = &outcome.pending {
                        reporter.info(&format!("pending: {pending}"));
                    }
                }
            }
            Err(error) if api_error_code(&error) == Some("protocol-review-required") => {
                status = "review-required".into();
                reporter.warn(format!(
                    "Release {} has handler code, which runs only after an operator reviews it. \
                     Share the release hash and deployment id with the operator, who reviews the \
                     release and activates the deployment.",
                    upload.release_hash
                ));
            }
            Err(error) if api_error_code(&error) == Some("protocol-public-requires-admin") => {
                return Err(error.context(
                    "Public deployments are created by an operator; self-serve deployments are private",
                ));
            }
            Err(error) => {
                let context = match squads_activation_commands(&error) {
                    Some(commands) => format!("Failed to activate the deployment. {commands}"),
                    None => "Failed to activate the deployment".to_string(),
                };
                return Err(error.context(context));
            }
        }
    }

    print_deployment(&reporter, &deployment, &status);
    if status == "active" {
        let release_hash = outcome
            .release
            .release_hash
            .clone()
            .unwrap_or_else(|| local.expected_release_hash.clone());
        outcome.client = update_installed_client(
            client,
            &args.dir,
            &settings.slug,
            &deployment,
            &local,
            &release_hash,
            &mut reporter,
        );
    }
    outcome.status = status;
    outcome.deployment = Some(DeploymentOutcome {
        deployment,
        created: deployment_created,
    });
    outcome.warnings = reporter.warnings;
    Ok(outcome)
}

/// One line on the activation's registry package, when the server reported
/// one.
fn report_registry_package(reporter: &Reporter, package: Option<&Value>) {
    let Some(package) = package else {
        return;
    };
    let name = format!(
        "{}@{}",
        package["package"].as_str().unwrap_or("?"),
        package["version"].as_str().unwrap_or("?")
    );
    match package["status"].as_str() {
        Some("published") => reporter.success(&format!(
            "Published the private registry package {name} (`a4 install protocol` resolves it for this account)"
        )),
        Some("existing") => reporter.info(&format!("Registry package {name} (already published)")),
        Some("skipped") => reporter.info(&format!(
            "No registry package published: {} ({})",
            package["message"].as_str().unwrap_or("skipped"),
            package["code"].as_str().unwrap_or("skipped")
        )),
        _ => {}
    }
}

/// After an activation: point the project's installed client of this
/// protocol (if it installs one) at `deployment`. The registry reinstall is
/// tried first; until the registry has a package release for this
/// deployment the client is generated from the deployed build instead.
/// Never fails the deploy: problems become warnings.
fn update_installed_client(
    client: &ApiClient,
    dir: &Path,
    slug: &str,
    deployment: &ProtocolDeployment,
    local: &LocalRelease,
    release_hash: &str,
    reporter: &mut Reporter,
) -> Option<ClientUpdate> {
    use crate::project::installer::{
        install_local_protocol_build, install_project, own_protocol_dependency,
        require_own_protocol_version, InstallOptions, LocalBuildDeployment, LocalProtocolBuild,
        UpdateSelection,
    };
    use crate::project::manifest::DependencyKind;
    use crate::project::{ProjectLock, ProjectManifest};

    let manifest_path = dir.join("arete.toml");
    if !manifest_path.is_file() {
        return None;
    }
    let manifest = match ProjectManifest::load(&manifest_path) {
        Ok(manifest) => manifest,
        Err(error) => {
            reporter.warn(format!("The installed client was not updated: {error:#}"));
            return None;
        }
    };
    let Some(alias) = own_protocol_dependency(&manifest, slug) else {
        reporter.info(&format!(
            "This project installs no {slug} client; `a4 install protocol {slug} --ts` adds one \
             (`a4 dev up` does too)"
        ));
        return None;
    };
    let lock_path = dir.join("arete.lock");
    let locked_deployment = |path: &Path| -> Option<(String, bool)> {
        let lock = ProjectLock::load_optional(path).ok()??;
        let entry = lock
            .dependencies
            .into_iter()
            .find(|entry| entry.kind == DependencyKind::Protocol && entry.alias == alias)?;
        Some((entry.protocol?.deployment_id, entry.local_build.is_none()))
    };
    let lock_was_fresh = ProjectLock::load_optional(&lock_path)
        .ok()
        .flatten()
        .is_some_and(|lock| lock.is_fresh(&manifest.manifest_hash));
    // A version bump since the dependency was written: accept the deployed
    // version, or the reinstall resolves the previous package.
    let deployed_version = local.spec_value["version"].as_str().unwrap_or_default();
    let manifest = match require_own_protocol_version(&manifest_path, &alias, deployed_version)
        .and_then(|changed| Ok((changed, ProjectManifest::load(&manifest_path)?)))
    {
        Ok((Some(requirement), manifest)) => {
            reporter.info(&format!(
                "[dependencies.protocols.{alias}] version is now \"{requirement}\" (the deployed {deployed_version})"
            ));
            manifest
        }
        Ok((None, manifest)) => manifest,
        Err(error) => {
            reporter.warn(format!("The installed client was not updated: {error:#}"));
            return None;
        }
    };
    reporter.step(&format!(
        "Updating the installed {alias} client to deployment {}",
        deployment.short_id
    ));
    let reinstalled = install_project(
        &manifest_path,
        InstallOptions {
            update: Some(UpdateSelection {
                kind: Some(DependencyKind::Protocol),
                alias: Some(&alias),
            }),
            quiet: true,
            ..InstallOptions::default()
        },
    );
    let outputs = || -> Vec<String> {
        crate::project::InstallPlan::build(&manifest, false)
            .map(|plan| {
                plan.for_dependency(DependencyKind::Protocol, &alias)
                    .map(|output| output.path.display().to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    let why = match reinstalled {
        Ok(()) => match locked_deployment(&lock_path) {
            Some((id, true)) if id == deployment.id => {
                reporter.success(&format!(
                    "Reinstalled {alias} from its published package; the client calls {}",
                    deployment.id
                ));
                let outputs = outputs();
                return Some(ClientUpdate {
                    alias,
                    mode: "reinstalled".into(),
                    deployment_id: deployment.id.clone(),
                    outputs,
                    note: None,
                });
            }
            _ => "the registry's newest package release is for another deployment".to_string(),
        },
        Err(error) => format!("{error:#}"),
    };
    let build = LocalProtocolBuild {
        spec_json: local.spec_value.clone(),
        release_hash: release_hash.to_string(),
        deployment: LocalBuildDeployment::Deployed {
            id: deployment.id.clone(),
            access: deployment.access_mode.clone(),
        },
        api_base: client.api_base_url().to_string(),
    };
    match install_local_protocol_build(&manifest_path, &alias, &build, lock_was_fresh) {
        Ok(install) => {
            let note = format!(
                "No published package release of {alias} for this deployment yet ({why}). The client was generated from the deployed build and calls {}; arete.lock marks it as a local build, so `a4 install --check` fails until `a4 install` installs the published release.",
                deployment.id
            );
            reporter.warn(note.clone());
            Some(ClientUpdate {
                alias,
                mode: "local-build".into(),
                deployment_id: deployment.id.clone(),
                outputs: install
                    .outputs
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                note: Some(note),
            })
        }
        Err(error) => {
            reporter.warn(format!(
                "The installed {alias} client was not updated ({why}; then: {error:#}). Run `a4 install` once the package is published."
            ));
            None
        }
    }
}

fn api_error_code(error: &anyhow::Error) -> Option<&str> {
    api_error_details(error).and_then(|details| details.code)
}

fn require_secret_key(client: &ApiClient) -> Result<()> {
    if !client.has_api_key() {
        bail!(
            "`a4 protocol deploy` needs a secret API key. Run `a4 auth login`, or set ARETE_API_KEY."
        );
    }
    if client.api_key_is_publishable() {
        bail!(
            "`a4 protocol deploy` needs a secret API key; the configured key is a publishable (browser) key."
        );
    }
    Ok(())
}

/// Find the protocol by slug, creating it when absent (a concurrent create
/// shows up as `protocol-slug-taken`, after which it is listed again).
fn ensure_protocol(client: &ApiClient, settings: &Settings) -> Result<(ProtocolRecord, bool)> {
    let find = |protocols: Vec<ProtocolRecord>| {
        protocols
            .into_iter()
            .find(|protocol| protocol.slug == settings.slug)
    };
    if let Some(protocol) = find(client.list_protocols()?) {
        return Ok((protocol, false));
    }
    match client.create_protocol(&settings.slug, &settings.name) {
        Ok(protocol) => Ok((protocol, true)),
        Err(error) if api_error_code(&error) == Some("protocol-slug-taken") => {
            match find(client.list_protocols()?) {
                Some(protocol) => Ok((protocol, false)),
                None => Err(error.context(format!(
                    "The slug '{}' belongs to another account; pass a different --slug",
                    settings.slug
                ))),
            }
        }
        Err(error) => Err(error),
    }
}

/// A private deployment of `release_hash`, preferring active, then
/// activating, then disabled.
fn select_reusable_deployment<'a>(
    deployments: &'a [ProtocolDeployment],
    release_hash: &str,
) -> Option<&'a ProtocolDeployment> {
    let rank = |status: &str| match status {
        "active" => 0,
        "activating" => 1,
        "disabled" => 2,
        _ => 3,
    };
    deployments
        .iter()
        .filter(|deployment| {
            deployment.release_hash == release_hash
                && deployment.environment_id.is_none()
                && deployment.access_mode == "private"
                && rank(&deployment.status) < 3
        })
        .min_by_key(|deployment| rank(&deployment.status))
}

fn fee_policy_help() -> String {
    format!(
        "A new deployment needs a fee policy: pass --fee-policy <file> or set [protocol].fee_policy in arete.toml. \
         The file is a JSON document like {{\"schema\":\"{FEE_POLICY_SCHEMA}\",\"steps\":{{}}}}. \
         Arete fills in the platform fee destination, so platformDestination is optional"
    )
}

fn missing_inputs(local: &LocalRelease, settings: &Settings) -> Vec<String> {
    let mut missing = Vec::new();
    if settings.fee_policy.is_none() {
        missing.push("fee policy".to_string());
    }
    for name in local.spec.resources.keys() {
        if !settings.resources.contains_key(name) {
            missing.push(format!("resource {name}"));
        }
    }
    missing
}

fn create_deployment(
    client: &ApiClient,
    protocol_id: &str,
    upload: &ProtocolReleaseUpload,
    local: &LocalRelease,
    settings: &Settings,
    reporter: &Reporter,
) -> Result<ProtocolDeployment> {
    // Read every input before changing anything on the server.
    let fee_policy_path = settings
        .fee_policy
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!(fee_policy_help()))?;
    let fee_policy = read_contract_document(fee_policy_path, "Fee policy")?;
    let mut values = BTreeMap::new();
    for name in local.spec.resources.keys() {
        let source = settings
            .resources
            .get(name)
            .ok_or_else(|| anyhow::anyhow!(missing_resource_help(name)))?;
        values.insert(name.clone(), source.read(name)?);
    }

    let policy = client.create_protocol_fee_policy(protocol_id, &fee_policy)?;
    reporter.success(&format!("Stored fee policy {}", policy.id));
    let mut versions = BTreeMap::new();
    for (name, value) in &values {
        let draft = client.create_protocol_resource_version(
            protocol_id,
            name,
            value,
            &upload.release_hash,
        )?;
        let published = client.publish_protocol_resource_version(
            protocol_id,
            name,
            draft.version,
            &upload.release_hash,
        )?;
        reporter.success(&format!("Published resource {name} v{}", published.version));
        versions.insert(name.clone(), published.version);
    }
    client.create_protocol_deployment(protocol_id, &upload.release_hash, &versions, &policy.id)
}

fn dry_run_plan(
    client: &ApiClient,
    local: &LocalRelease,
    settings: &Settings,
    args: &DeployArgs,
    reporter: &mut Reporter,
) -> PlanOutcome {
    let has_handlers = local.spec.has_handlers();
    let mut plan = PlanOutcome {
        checked_remote: false,
        protocol: "unknown".into(),
        deployment: "unknown".into(),
        reuse_deployment_id: None,
        activation: String::new(),
        missing_inputs: missing_inputs(local, settings),
    };
    let mut reused: Option<ProtocolDeployment> = None;
    if client.has_api_key() && !client.api_key_is_publishable() {
        match remote_state(client, settings, &local.expected_release_hash) {
            Ok((exists, deployment)) => {
                plan.checked_remote = true;
                plan.protocol = if exists { "exists" } else { "create" }.into();
                plan.deployment = if deployment.is_some() {
                    "reuse"
                } else {
                    "create"
                }
                .into();
                plan.reuse_deployment_id = deployment.as_ref().map(|d| d.id.clone());
                reused = deployment;
            }
            Err(error) => reporter.warn(format!("Skipped server checks: {error}")),
        }
    } else {
        reporter.info("no secret key: server checks skipped");
    }
    if reused.is_some() {
        plan.missing_inputs.clear();
    }
    plan.activation = match (&reused, args.no_activate) {
        (Some(deployment), _) if deployment.status == "active" => "already active".into(),
        (_, true) => "skipped (--no-activate)".into(),
        _ if has_handlers => {
            "requires an operator review of the handler code before it can activate".into()
        }
        _ => "activates as a private deployment at trust tier unreviewed".into(),
    };
    plan
}

fn remote_state(
    client: &ApiClient,
    settings: &Settings,
    release_hash: &str,
) -> Result<(bool, Option<ProtocolDeployment>)> {
    let Some(protocol) = client
        .list_protocols()?
        .into_iter()
        .find(|protocol| protocol.slug == settings.slug)
    else {
        return Ok((false, None));
    };
    let deployments = client.list_protocol_deployments(&protocol.id)?;
    Ok((
        true,
        select_reusable_deployment(&deployments, release_hash).cloned(),
    ))
}

fn print_plan(reporter: &Reporter, settings: &Settings, plan: &PlanOutcome) {
    if reporter.json {
        return;
    }
    println!();
    println!("{}", "Plan (dry run)".bold());
    println!("  Protocol:   {} ({})", settings.slug.cyan(), plan.protocol);
    match &plan.reuse_deployment_id {
        Some(id) => println!("  Deployment: reuse {id}"),
        None => println!("  Deployment: {} (private)", plan.deployment),
    }
    println!("  Activation: {}", plan.activation);
    if !plan.missing_inputs.is_empty() && plan.deployment != "reuse" {
        println!(
            "  {} {}",
            "Missing for a new deployment:".yellow(),
            plan.missing_inputs.join(", ")
        );
    }
    println!();
    println!("{}", "Run without --dry-run to deploy.".dimmed());
}

fn print_deployment(reporter: &Reporter, deployment: &ProtocolDeployment, status: &str) {
    if reporter.json {
        return;
    }
    println!();
    println!("  Deployment: {}", deployment.id);
    println!("  Short id:   {}", deployment.short_id.cyan());
    println!("  Status:     {}", status.bold());
    println!(
        "  Trust tier: {}",
        deployment.trust_tier.as_deref().unwrap_or("unknown")
    );
    println!("  Release:    {}", deployment.release_hash);
}

#[cfg(test)]
mod tests;
