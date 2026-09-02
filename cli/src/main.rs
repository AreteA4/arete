//! # a4-cli
//!
//! Command-line tool for building, deploying, and managing Arete
//! stream stacks.
//!
//! ## Installation
//!
//! ```bash
//! cargo install a4-cli
//! ```
//!
//! ## Commands
//!
//! - `a4 init` - Initialize configuration
//! - `a4 up [stack]` - Deploy a stack (push + build + deploy)
//! - `a4 stack list` - List all stacks
//! - `a4 stack show` - Show stack details
//! - `a4 sdk create` - Generate TypeScript/Rust/Python SDK
//! - `a4 install` - Generate TypeScript/Rust/Python SDK from a hosted stack
//!
//! See `a4 --help` for the full command reference.

use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use colored::Colorize;
use std::io;
use std::process;

mod api_client;
mod commands;
mod config;
mod project;
mod telemetry;
mod templates;
mod ui;

#[derive(Parser)]
#[command(name = "a4")]
#[command(about = "Arete CLI - Build, deploy, and manage stream stacks", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Path to arete.toml configuration file
    #[arg(short, long, global = true, default_value = "arete.toml")]
    config: String,

    /// Output as JSON (machine-readable format)
    #[arg(long, global = true)]
    json: bool,

    /// Enable verbose output
    #[arg(long, global = true)]
    verbose: bool,

    /// API URL to use (overrides ARETE_API_URL env var)
    #[arg(long, global = true, env = "ARETE_API_URL")]
    api_url: Option<String>,

    /// Generate shell completions
    #[arg(long, value_name = "SHELL")]
    completions: Option<Shell>,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new Arete project from a template
    Create {
        /// Project name (creates directory)
        name: Option<String>,

        /// Template: react-ore, rust-ore, typescript-ore, python-ore
        #[arg(short, long)]
        template: Option<String>,

        /// Use cached templates only (no network)
        #[arg(long)]
        offline: bool,

        /// Force re-download templates even if cached
        #[arg(long)]
        force_refresh: bool,

        /// Skip installing dependencies
        #[arg(long)]
        skip_install: bool,
    },

    /// Initialize a new Arete project (auto-detects stack files)
    Init,

    /// Deploy a stack: push, build, and watch until completion
    Up {
        /// Name of specific stack to deploy (deploys all if not specified)
        stack_name: Option<String>,

        /// Deploy to a specific branch (creates {stack-name}-{branch}.stack.arete.run)
        #[arg(short, long)]
        branch: Option<String>,

        /// Create a preview deployment with auto-generated branch name
        #[arg(long, conflicts_with = "branch")]
        preview: bool,

        /// Show what would be deployed without actually deploying
        #[arg(long)]
        dry_run: bool,

        /// Plan only from local artifacts; requires --dry-run and skips server checks
        #[arg(long, requires = "dry_run")]
        local_only: bool,

        /// Permit persisting a plan that contains observed private programs
        #[arg(long)]
        allow_unverified_programs: bool,
    },

    /// Show overview of stacks, builds, and deployments
    Status,

    /// Discover installable stacks and programs through pinned descriptors
    Explore {
        /// `stack`, `program`, `programs`, or a legacy stack reference
        target: Option<String>,

        /// Resource reference, or an entity for legacy `explore <stack> <entity>`
        reference: Option<String>,

        /// Entity for explicit `explore stack <ref> <entity>` drill-down
        entity: Option<String>,
    },

    /// Query the curated knowledge layer: protocols, programs, recipes, concepts
    #[command(subcommand)]
    Know(KnowCommands),

    /// Resolve and install dependencies from arete.toml, or add one package
    Install {
        /// Package kind (`stack` or `program`), or legacy stack shorthand
        target: Option<String>,

        /// Program install identifier when using `a4 install program <program>`
        install_name: Option<String>,

        /// Generate a TypeScript SDK
        #[arg(long, conflicts_with_all = ["rust", "python"])]
        ts: bool,

        /// Generate a Rust SDK
        #[arg(long, conflicts_with_all = ["ts", "python"])]
        rust: bool,

        /// Generate a Python SDK
        #[arg(long, conflicts_with_all = ["ts", "rust"])]
        python: bool,

        /// Output path (file for TypeScript, directory for Rust or Python)
        #[arg(short, long)]
        output: Option<String>,

        /// Package name for TypeScript imports, or the generated Python distribution
        #[arg(short, long)]
        package_name: Option<String>,

        /// Crate name for generated Rust crate
        #[arg(long)]
        crate_name: Option<String>,

        /// Generate Rust (mod.rs) or Python as a module instead of a standalone crate/package
        #[arg(long)]
        module: bool,

        /// WebSocket URL for the stack
        #[arg(long)]
        url: Option<String>,

        /// Local extensions artifact source (manifest file, entry file, or directory)
        #[arg(long)]
        extensions: Option<String>,

        /// Require an existing fresh arete.lock and never change resolution
        #[arg(long, conflicts_with = "no_save")]
        locked: bool,

        /// Validate and print the complete install graph without writing
        #[arg(long)]
        dry_run: bool,

        /// Consent to outputs outside the project when the manifest also opts in
        #[arg(long)]
        allow_outside_project: bool,

        /// Generate one package without changing arete.toml or arete.lock
        #[arg(long)]
        no_save: bool,

        /// Local dependency alias to write into arete.toml
        #[arg(long)]
        alias: Option<String>,

        /// Save a bare package add as an exact requirement
        #[arg(long)]
        exact: bool,
    },

    /// Advance registry dependencies within their manifest requirements
    Update {
        /// Optional dependency kind (stack or program)
        kind: Option<String>,

        /// Optional dependency alias (requires a kind)
        alias: Option<String>,

        /// Consent to outputs outside the project when the manifest also opts in
        #[arg(long)]
        allow_outside_project: bool,
    },

    /// Remove one project dependency and its provenance-owned SDK outputs
    Remove {
        /// Dependency kind (`stack` or `program`)
        kind: String,

        /// Local dependency alias from arete.toml
        alias: String,

        /// Keep generated outputs while removing manifest and lock entries
        #[arg(long)]
        keep_output: bool,

        /// Consent to outputs outside the project when the manifest also opts in
        #[arg(long)]
        allow_outside_project: bool,
    },

    /// SDK generation commands
    #[command(subcommand)]
    Sdk(SdkCommands),

    /// Configuration management commands
    #[command(subcommand)]
    Config(ConfigCommands),

    /// Authentication commands
    #[command(subcommand)]
    Auth(AuthCommands),

    /// Stack management commands - manage your deployed stacks
    #[command(subcommand)]
    Stack(StackCommands),

    /// Build and validate portable program artifacts
    #[command(subcommand)]
    Program(ProgramCommands),

    /// Build commands (advanced) - low-level build management
    #[command(subcommand, hide = true)]
    Build(BuildCommands),

    /// Manage anonymous usage telemetry
    #[command(subcommand)]
    Telemetry(TelemetryCommands),

    /// Inspect and analyze Anchor/Shank IDL files
    Idl(commands::idl::IdlArgs),

    /// Stream live entity data from a deployed stack via WebSocket
    Stream(commands::stream::StreamArgs),
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // Clap owns this short-lived command value.
enum SdkCommands {
    /// Create SDK from a stack
    Create(SdkCreateArgs),

    /// Regenerate SDKs for every configured stack
    Sync(SdkSyncArgs),

    /// List all available stacks from arete.toml
    List,
}

#[derive(Args)]
struct SdkCreateArgs {
    /// Name of the stack to generate SDK for
    #[arg(
        required_unless_present_any = ["idl", "program_spec", "manifest"],
        conflicts_with_all = ["idl", "program_spec", "manifest"]
    )]
    stack_name: Option<String>,

    /// Generate a TypeScript SDK
    #[arg(long, conflicts_with_all = ["rust", "python"])]
    ts: bool,

    /// Generate a Rust SDK
    #[arg(long, conflicts_with_all = ["ts", "python"])]
    rust: bool,

    /// Generate a Python SDK
    #[arg(long, conflicts_with_all = ["ts", "rust"])]
    python: bool,

    /// Output path (file for TypeScript, directory for Rust or Python)
    #[arg(short, long)]
    output: Option<String>,

    /// Package name for TypeScript imports, or the generated Python distribution
    #[arg(short, long)]
    package_name: Option<String>,

    /// Crate name for generated Rust crate
    #[arg(long)]
    crate_name: Option<String>,

    /// Generate Rust (mod.rs) or Python as a module instead of a standalone crate/package
    #[arg(long)]
    module: bool,

    /// WebSocket URL for the stack (overrides config)
    #[arg(long)]
    url: Option<String>,

    /// Local extensions artifact source (manifest file, entry file, or directory)
    #[arg(long)]
    extensions: Option<String>,

    /// Raw IDL file to generate a standalone program SDK from (TypeScript + --program-only only)
    #[arg(
        long,
        requires = "program_only",
        conflicts_with_all = ["stack_name", "program_spec", "manifest"]
    )]
    idl: Option<String>,

    /// Local ProgramSpec artifact to generate a standalone program SDK from
    #[arg(
        long,
        requires = "program_only",
        conflicts_with_all = ["stack_name", "idl", "manifest"]
    )]
    program_spec: Option<String>,

    /// Local StackManifest artifact; dependencies default to its directory
    #[arg(
        long,
        conflicts_with_all = ["stack_name", "idl", "program_spec", "program_only"]
    )]
    manifest: Option<String>,

    /// Approved recursive artifact search root; repeat for dependencies outside the manifest directory
    #[arg(long, requires = "manifest")]
    artifact_dir: Vec<String>,

    /// Existing aliased live SDK import (`alias=./path.js`); repeat for composed manifests
    #[arg(long, requires = "manifest")]
    live_module: Vec<String>,

    /// Existing independent program SDK import (`alias=./path.js`); repeat for composed manifests
    #[arg(long, requires = "manifest")]
    program_module: Vec<String>,

    /// Emit a standalone program-SDK module (pdas/accounts/instructions, no
    /// views or stack const). TypeScript only.
    #[arg(long, conflicts_with_all = ["rust", "python"])]
    program_only: bool,
}

#[derive(Args)]
struct SdkSyncArgs {
    /// Sync TypeScript SDKs only
    #[arg(long, conflicts_with_all = ["rust", "python"])]
    ts: bool,

    /// Sync Rust SDKs only
    #[arg(long, conflicts_with_all = ["ts", "python"])]
    rust: bool,

    /// Sync Python SDKs only
    #[arg(long, conflicts_with_all = ["ts", "rust"])]
    python: bool,

    /// Limit sync to one or more configured stack names
    #[arg(long = "stack", short = 's')]
    stacks: Vec<String>,
}

#[derive(Subcommand)]
enum KnowCommands {
    /// Search protocols, programs, stacks, and recipes by intent
    Search {
        /// Free-text intent (e.g. "monitor swaps"); matches concept synonyms first
        #[arg(short, long)]
        query: Option<String>,

        /// Filter by concept slug (see `a4 know concepts`)
        #[arg(long)]
        concept: Option<String>,

        /// Filter by category slug (see `a4 know concepts`)
        #[arg(long)]
        category: Option<String>,

        /// Maximum number of results
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Show curated knowledge for one protocol
    Protocol {
        /// Protocol slug (e.g. meteora-damm)
        slug: String,
    },

    /// Show curated annotations for one program
    Program {
        /// Program slug (e.g. meteora-cp-amm)
        slug: String,

        /// Section to fetch: summary (default), instructions, accounts, or surface
        #[arg(long)]
        section: Option<String>,
    },

    /// Show one cross-protocol recipe
    Recipe {
        /// Recipe slug (e.g. execute-presale-purchase-via-squads)
        slug: String,
    },

    /// List the concept and category vocabularies
    Concepts,
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Validate the configuration file
    Validate,
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Login with your API key
    Login {
        /// API key (prompts if not provided)
        #[arg(short, long)]
        key: Option<String>,
    },

    /// Logout (remove stored credentials for current environment)
    Logout,

    /// Logout from all environments (remove all stored credentials)
    LogoutAll,

    /// Check authentication status (shows current environment and all stored credentials)
    Status,

    /// Verify authentication and show user info
    Whoami,

    /// Manage API keys for browser/client use
    #[command(subcommand)]
    Keys(KeysCommands),
}

#[derive(Subcommand)]
enum KeysCommands {
    /// List all your API keys
    List,

    /// Create a new publishable API key for browser/client use
    CreatePublishable {
        /// Name for the key (optional)
        #[arg(short, long)]
        name: Option<String>,

        /// Allowed origins (e.g., https://example.com or http://localhost:5173)
        /// Can specify multiple: --origin https://app.com --origin https://www.app.com
        #[arg(short, long, required = true, num_args = 1..)]
        origin: Vec<String>,

        /// Number of days until the key expires (default: 365)
        #[arg(short, long)]
        expiry_days: Option<i64>,
    },
}

#[derive(Subcommand)]
enum StackCommands {
    /// Compose ProgramSpecs and LiveSpecs into a portable StackManifest
    Compose {
        /// Client-facing stack name
        #[arg(long)]
        name: String,

        /// ProgramSpec artifact path; repeat for each program
        #[arg(long = "program")]
        programs: Vec<String>,

        /// Aliased LiveSpec artifact (`alias=path`); repeat to compose live packages
        #[arg(long = "live")]
        live_specs: Vec<String>,

        /// Approved recursive artifact search root; repeat for multiple roots
        #[arg(long = "artifact-dir")]
        artifact_dirs: Vec<String>,

        /// Selected client view (`alias=view_id`); repeat for an exact ordered allowlist
        #[arg(long = "selected-view")]
        selected_views: Vec<String>,

        /// StackManifest output path
        #[arg(short, long)]
        output: String,
    },

    /// List all stacks with their deployment status
    List,

    /// Show detailed stack information including deployment status and versions
    Show {
        /// Name of the stack
        stack_name: String,

        /// Show specific version details
        #[arg(short, long)]
        version: Option<i32>,
    },

    /// Show version history for a stack
    Versions {
        /// Name of the stack
        stack_name: String,

        /// Maximum number of versions to show
        #[arg(short, long, default_value = "20")]
        limit: i64,
    },

    /// Delete a stack from remote
    Delete {
        /// Name of the stack to delete
        stack_name: String,

        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },

    /// Stop a deployment
    Stop {
        /// Name of the stack to stop
        stack_name: String,

        /// Branch deployment to stop (default: production)
        #[arg(long)]
        branch: Option<String>,

        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum ProgramCommands {
    /// Normalize an IDL into a portable ProgramSpec artifact
    Build {
        /// IDL JSON input
        input: String,

        /// ProgramSpec output path
        #[arg(short, long)]
        output: String,

        /// Program ID when the IDL does not declare one
        #[arg(long)]
        program_id: Option<String>,
    },

    /// Upload an IDL or ProgramSpec as an owner-private hosted program
    Push {
        /// IDL JSON or ProgramSpec artifact input
        input: String,
        /// Program ID when an IDL does not declare one
        #[arg(long)]
        program_id: Option<String>,
        /// Optional owner-private label
        #[arg(long)]
        alias: Option<String>,
        /// Canonical UUID used to replay this upload safely
        #[arg(long)]
        idempotency_key: Option<String>,
        /// Wait until admission is ready or failed
        #[arg(long)]
        wait: bool,
    },

    /// List owner-visible uploaded programs
    List {
        /// Resume at an opaque cursor returned by the previous page
        #[arg(long)]
        cursor: Option<String>,
    },

    /// Show admission, visibility, release, and health state
    Status {
        user_program_id: String,
        /// Watch until admission is ready or failed
        #[arg(long)]
        watch: bool,
    },

    /// List append-only admission, health, archive, and promotion events
    Events {
        user_program_id: String,
        /// Resume after an opaque event cursor
        #[arg(long)]
        after: Option<String>,
    },

    /// Archive a registration without deleting immutable content
    Archive {
        user_program_id: String,
        /// Confirm archival without an interactive prompt
        #[arg(long)]
        yes: bool,
    },

    /// Request reviewed promotion of the baseline IDL
    Promote {
        user_program_id: String,
        /// Consent to public OSS distribution of the baseline IDL
        #[arg(long)]
        make_idl_public: bool,
    },
}

#[derive(Subcommand)]
enum TelemetryCommands {
    /// Show current telemetry status
    Status,

    /// Enable telemetry collection
    Enable,

    /// Disable telemetry collection
    Disable,
}

/// Build commands - advanced low-level build management
/// These are power-user commands; most users should use `a4 up` instead.
#[derive(Subcommand)]
enum BuildCommands {
    /// List builds
    List {
        /// Maximum number of builds to show
        #[arg(short, long, default_value = "20")]
        limit: i64,

        /// Filter by status (pending, building, completed, failed, etc.)
        #[arg(short, long)]
        status: Option<String>,
    },

    /// Get detailed build status
    Status {
        /// Build ID
        build_id: i32,

        /// Watch build progress until completion
        #[arg(short, long)]
        watch: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    // Set ARETE_API_URL env var if --api-url flag is provided
    // This ensures all ApiClient instances use the correct URL
    if let Some(ref api_url) = cli.api_url {
        std::env::set_var("ARETE_API_URL", api_url);
    }

    if let Some(shell) = cli.completions {
        let mut cmd = Cli::command();
        generate(shell, &mut cmd, "a4", &mut io::stdout());
        return;
    }

    telemetry::show_consent_banner_if_needed();

    let cmd_name = cli.command.as_ref().map(command_name).unwrap_or("help");
    let start = std::time::Instant::now();
    let result = run(cli);

    telemetry::record_command(
        cmd_name,
        result.is_ok(),
        result
            .as_ref()
            .err()
            .and_then(telemetry::extract_error_code)
            .as_deref(),
        start.elapsed(),
        None,
    );

    telemetry::flush();

    if let Err(e) = result {
        eprintln!("{} {}", "Error:".red().bold(), e);
        process::exit(1);
    }
}

fn command_name(cmd: &Commands) -> &'static str {
    match cmd {
        Commands::Create { .. } => "create",
        Commands::Init => "init",
        Commands::Up { .. } => "up",
        Commands::Status => "status",
        Commands::Explore { .. } => "explore",
        Commands::Know(_) => "know",
        Commands::Install { .. } => "install",
        Commands::Update { .. } => "update",
        Commands::Remove { .. } => "remove",
        Commands::Sdk(_) => "sdk",
        Commands::Config(_) => "config",
        Commands::Auth(_) => "auth",
        Commands::Stack(_) => "stack",
        Commands::Program(_) => "program",
        Commands::Build(_) => "build",
        Commands::Telemetry(_) => "telemetry",
        Commands::Idl(_) => "idl",
        Commands::Stream(_) => "stream",
    }
}

fn parse_dependency_kind(value: &str) -> anyhow::Result<project::manifest::DependencyKind> {
    match value {
        "stack" => Ok(project::manifest::DependencyKind::Stack),
        "program" => Ok(project::manifest::DependencyKind::Program),
        other => Err(anyhow::anyhow!(
            "Unknown dependency kind '{other}'; expected 'stack' or 'program'"
        )),
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        return Ok(());
    };

    match command {
        Commands::Create {
            name,
            template,
            offline,
            force_refresh,
            skip_install,
        } => commands::create::create(name, template, offline, force_refresh, skip_install),
        Commands::Init => commands::config::init(&cli.config),
        Commands::Up {
            stack_name,
            branch,
            preview,
            dry_run,
            local_only,
            allow_unverified_programs,
        } => commands::up::up(
            &cli.config,
            stack_name.as_deref(),
            branch,
            preview,
            dry_run,
            local_only,
            allow_unverified_programs,
            cli.json,
        ),
        Commands::Status => commands::status::status(cli.json),
        Commands::Explore {
            target,
            reference,
            entity,
        } => match (target.as_deref(), reference.as_deref(), entity.as_deref()) {
            (None, None, None) => commands::explore::list(cli.json),
            (Some("programs"), None, None) => commands::explore::list_programs(cli.json),
            (Some("program"), Some(reference), None) => {
                commands::explore::show_program(reference, cli.json)
            }
            (Some("stack"), Some(reference), entity) => {
                commands::explore::show_stack(reference, entity, cli.json)
            }
            (Some("program"), None, None) => Err(anyhow::anyhow!(
                "Program reference required. Usage: a4 explore program <ref>"
            )),
            (Some("stack"), None, None) => Err(anyhow::anyhow!(
                "Stack reference required. Usage: a4 explore stack <ref>"
            )),
            (Some(stack), entity, None) => {
                commands::explore::show_stack(stack, entity, cli.json)
            }
            _ => Err(anyhow::anyhow!(
                "Invalid explore arguments. Use `a4 explore`, `a4 explore programs`, `a4 explore stack <ref>`, or `a4 explore program <ref>`."
            )),
        },
        Commands::Know(know_cmd) => match know_cmd {
            KnowCommands::Search {
                query,
                concept,
                category,
                limit,
            } => commands::know::search(
                query.as_deref(),
                concept.as_deref(),
                category.as_deref(),
                limit,
                cli.json,
            ),
            KnowCommands::Protocol { slug } => commands::know::protocol(&slug, cli.json),
            KnowCommands::Program { slug, section } => {
                commands::know::program(&slug, section.as_deref(), cli.json)
            }
            KnowCommands::Recipe { slug } => commands::know::recipe(&slug, cli.json),
            KnowCommands::Concepts => commands::know::concepts(cli.json),
        },
        Commands::Install {
            target,
            install_name,
            ts,
            rust,
            python,
            output,
            package_name,
            crate_name,
            module,
            url,
            extensions,
            locked,
            dry_run,
            allow_outside_project,
            no_save,
            alias,
            exact,
        } => match target.as_deref() {
            None
                if install_name.is_some()
                    || ts
                    || rust
                    || python
                    || output.is_some()
                    || package_name.is_some()
                    || crate_name.is_some()
                    || module
                    || url.is_some()
                    || extensions.is_some()
                    || no_save
                    || alias.is_some()
                    || exact =>
            {
                Err(anyhow::anyhow!(
                    "Package and generation flags require a package add; plain a4 install reads arete.toml"
                ))
            }
            None => project::installer::install_project(
                &cli.config,
                project::installer::InstallOptions {
                    locked,
                    allow_outside_project,
                    dry_run,
                    update: None,
                },
            ),
            Some(target) => {
                if locked || dry_run {
                    Err(anyhow::anyhow!(
                        "--locked and --dry-run apply to project-level a4 install"
                    ))
                } else {
                    let (kind, package) = match (target, install_name) {
                        ("stack", Some(package)) => {
                            (project::manifest::DependencyKind::Stack, package)
                        }
                        ("program", Some(package)) => {
                            (project::manifest::DependencyKind::Program, package)
                        }
                        ("program", None) | ("stack", None) => {
                            return Err(anyhow::anyhow!(
                                "Package required. Usage: a4 install <stack|program> <package>[@<requirement>]"
                            ));
                        }
                        (stack, None) => (
                            project::manifest::DependencyKind::Stack,
                            stack.to_string(),
                        ),
                        (_, Some(_)) => {
                            return Err(anyhow::anyhow!(
                                "Package kind must be 'stack' or 'program'"
                            ));
                        }
                    };
                    let selected_target = match (ts, rust, python) {
                        (true, false, false) => {
                            Some(project::manifest::InstallTarget::TypeScript)
                        }
                        (false, true, false) => Some(project::manifest::InstallTarget::Rust),
                        (false, false, true) => Some(project::manifest::InstallTarget::Python),
                        (false, false, false) => None,
                        _ => unreachable!("clap rejects conflicting target flags"),
                    };
                    if no_save {
                        if exact || allow_outside_project {
                            return Err(anyhow::anyhow!(
                                "--exact and --allow-outside-project require a saved project dependency"
                            ));
                        }
                        if url.is_some() || extensions.is_some() {
                            return Err(anyhow::anyhow!(
                                "--no-save uses the exact registry descriptor; --url and --extensions are only supported by low-level a4 sdk create"
                            ));
                        }
                        project::installer::install_without_saving(
                            kind,
                            &package,
                            project::installer::NoSaveDependencyOptions {
                                alias,
                                target: selected_target,
                                output,
                                typescript_package: package_name,
                                rust_crate_prefix: crate_name,
                                module,
                            },
                        )
                    } else {
                        if url.is_some() || extensions.is_some() || crate_name.is_some() {
                            return Err(anyhow::anyhow!(
                                "--url, --extensions, and --crate-name require --no-save"
                            ));
                        }
                        project::installer::add_and_install(
                            &cli.config,
                            kind,
                            &package,
                            project::installer::AddDependencyOptions {
                                alias,
                                exact,
                                target: selected_target,
                                output,
                                typescript_package: package_name,
                                module,
                                allow_outside_project,
                            },
                        )
                    }
                }
            }
        },
        Commands::Update {
            kind,
            alias,
            allow_outside_project,
        } => {
            if alias.is_some() && kind.is_none() {
                return Err(anyhow::anyhow!(
                    "An update alias requires its dependency kind"
                ));
            }
            let kind = kind
                .as_deref()
                .map(parse_dependency_kind)
                .transpose()?;
            project::installer::install_project(
                &cli.config,
                project::installer::InstallOptions {
                    allow_outside_project,
                    update: Some(project::installer::UpdateSelection {
                        kind,
                        alias: alias.as_deref(),
                    }),
                    ..project::installer::InstallOptions::default()
                },
            )
        },
        Commands::Remove {
            kind,
            alias,
            keep_output,
            allow_outside_project,
        } => project::installer::remove_and_install(
            &cli.config,
            parse_dependency_kind(&kind)?,
            &alias,
            project::installer::RemoveDependencyOptions {
                keep_output,
                allow_outside_project,
            },
        ),
        Commands::Sdk(sdk_cmd) => match sdk_cmd {
            SdkCommands::Create(create_args) => commands::sdk::create(
                &cli.config,
                create_args.stack_name.as_deref(),
                create_args.ts,
                create_args.rust,
                create_args.python,
                create_args.output,
                create_args.package_name,
                create_args.crate_name,
                create_args.module,
                create_args.url,
                create_args.extensions,
                create_args.idl,
                create_args.program_spec,
                create_args.manifest,
                create_args.artifact_dir,
                create_args.live_module,
                create_args.program_module,
                create_args.program_only,
            ),
            SdkCommands::Sync(sync_args) => commands::sdk::sync(
                &cli.config,
                sync_args.ts,
                sync_args.rust,
                sync_args.python,
                sync_args.stacks,
            ),
            SdkCommands::List => commands::sdk::list(&cli.config),
        },
        Commands::Config(config_cmd) => match config_cmd {
            ConfigCommands::Validate => commands::config::validate(&cli.config),
        },
        Commands::Auth(auth_cmd) => match auth_cmd {
            AuthCommands::Login { key } => commands::auth::login(key),
            AuthCommands::Logout => commands::auth::logout(),
            AuthCommands::LogoutAll => commands::auth::logout_all(),
            AuthCommands::Status => commands::auth::status(),
            AuthCommands::Whoami => commands::auth::whoami(),
            AuthCommands::Keys(keys_cmd) => match keys_cmd {
                KeysCommands::List => commands::auth::list_keys(),
                KeysCommands::CreatePublishable {
                    name,
                    origin,
                    expiry_days,
                } => commands::auth::create_publishable_key(name, origin, expiry_days),
            },
        },
        Commands::Stack(stack_cmd) => match stack_cmd {
            StackCommands::Compose {
                name,
                programs,
                live_specs,
                artifact_dirs,
                selected_views,
                output,
            } => commands::public_artifacts::compose_stack(
                &name,
                &programs,
                &live_specs,
                &artifact_dirs,
                &selected_views,
                &output,
            ),
            StackCommands::List => commands::stack::list(cli.json),
            StackCommands::Show {
                stack_name,
                version,
            } => commands::stack::show(&stack_name, version, cli.json),
            StackCommands::Versions { stack_name, limit } => {
                commands::stack::versions(&stack_name, limit, cli.json)
            }
            StackCommands::Delete { stack_name, force } => {
                commands::stack::delete(&stack_name, force)
            }
            StackCommands::Stop {
                stack_name,
                branch,
                force,
            } => commands::stack::stop(&stack_name, branch.as_deref(), force),
        },
        Commands::Program(program_cmd) => match program_cmd {
            ProgramCommands::Build {
                input,
                output,
                program_id,
            } => commands::public_artifacts::build_program(&input, &output, program_id.as_deref()),
            ProgramCommands::Push {
                input,
                program_id,
                alias,
                idempotency_key,
                wait,
            } => commands::programs::push(
                &input,
                program_id.as_deref(),
                alias,
                idempotency_key,
                wait,
                cli.json,
            ),
            ProgramCommands::List { cursor } => {
                commands::programs::list(cursor.as_deref(), cli.json)
            }
            ProgramCommands::Status {
                user_program_id,
                watch,
            } => commands::programs::status(&user_program_id, watch, cli.json),
            ProgramCommands::Events {
                user_program_id,
                after,
            } => commands::programs::events(&user_program_id, after.as_deref(), cli.json),
            ProgramCommands::Archive {
                user_program_id,
                yes,
            } => commands::programs::archive(&user_program_id, yes, cli.json),
            ProgramCommands::Promote {
                user_program_id,
                make_idl_public,
            } => commands::programs::promote(&user_program_id, make_idl_public, cli.json),
        },
        Commands::Build(build_cmd) => match build_cmd {
            BuildCommands::List { limit, status } => {
                commands::build::list(limit, status.as_deref(), cli.json)
            }
            BuildCommands::Status {
                build_id,
                watch,
                json,
            } => commands::build::status(build_id, watch, json || cli.json),
        },
        Commands::Idl(args) => commands::idl::run(args),
        Commands::Stream(args) => commands::stream::run(args, &cli.config),
        Commands::Telemetry(telemetry_cmd) => match telemetry_cmd {
            TelemetryCommands::Status => commands::telemetry::status(),
            TelemetryCommands::Enable => commands::telemetry::enable(),
            TelemetryCommands::Disable => commands::telemetry::disable(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_only_is_restricted_to_dry_run() {
        assert!(
            Cli::try_parse_from(["a4", "up", "stack.stack-manifest.json", "--local-only"]).is_err()
        );

        let cli = Cli::try_parse_from([
            "a4",
            "up",
            "stack.stack-manifest.json",
            "--dry-run",
            "--local-only",
        ])
        .expect("local-only dry run should parse");
        match cli.command {
            Some(Commands::Up {
                dry_run,
                local_only,
                ..
            }) => {
                assert!(dry_run);
                assert!(local_only);
            }
            _ => panic!("expected up command"),
        }
    }

    #[test]
    fn global_json_is_available_to_manifest_native_up() {
        let cli = Cli::try_parse_from([
            "a4",
            "--json",
            "up",
            "stack.stack-manifest.json",
            "--dry-run",
        ])
        .expect("manifest-native JSON dry run should parse");
        assert!(cli.json);
        assert!(matches!(
            cli.command,
            Some(Commands::Up {
                dry_run: true,
                local_only: false,
                ..
            })
        ));
    }

    #[test]
    fn parse_install_stack_shorthand() {
        let cli = Cli::try_parse_from(["a4", "install", "ore"]).expect("cli should parse");

        match cli.command {
            Some(Commands::Install {
                target,
                install_name,
                ..
            }) => {
                assert_eq!(target.as_deref(), Some("ore"));
                assert_eq!(install_name, None);
            }
            _ => panic!("expected install command"),
        }
    }

    #[test]
    fn parse_install_program_target() {
        let cli = Cli::try_parse_from(["a4", "install", "program", "spl-token", "--ts"])
            .expect("cli should parse");

        match cli.command {
            Some(Commands::Install {
                target,
                install_name,
                ts,
                ..
            }) => {
                assert_eq!(target.as_deref(), Some("program"));
                assert_eq!(install_name.as_deref(), Some("spl-token"));
                assert!(ts);
            }
            _ => panic!("expected install command"),
        }
    }

    #[test]
    fn parse_explicit_stack_explore() {
        let cli = Cli::try_parse_from(["a4", "explore", "stack", "ore", "Position", "--json"])
            .expect("cli should parse");
        match cli.command {
            Some(Commands::Explore {
                target,
                reference,
                entity,
            }) => {
                assert_eq!(target.as_deref(), Some("stack"));
                assert_eq!(reference.as_deref(), Some("ore"));
                assert_eq!(entity.as_deref(), Some("Position"));
                assert!(cli.json);
            }
            _ => panic!("expected explore command"),
        }
    }

    #[test]
    fn parse_program_list_and_program_explore() {
        for (args, expected_reference) in [
            (vec!["a4", "explore", "programs"], None),
            (
                vec!["a4", "explore", "program", "spl-token"],
                Some("spl-token"),
            ),
        ] {
            let cli = Cli::try_parse_from(args).expect("cli should parse");
            match cli.command {
                Some(Commands::Explore {
                    target,
                    reference,
                    entity,
                }) => {
                    assert_eq!(
                        target.as_deref(),
                        Some(if expected_reference.is_some() {
                            "program"
                        } else {
                            "programs"
                        })
                    );
                    assert_eq!(reference.as_deref(), expected_reference);
                    assert!(entity.is_none());
                }
                _ => panic!("expected explore command"),
            }
        }
    }

    #[test]
    fn parse_know_search_and_program_section() {
        let cli = Cli::try_parse_from([
            "a4",
            "know",
            "search",
            "--query",
            "monitor swaps",
            "--limit",
            "5",
            "--json",
        ])
        .expect("cli should parse");
        match cli.command {
            Some(Commands::Know(KnowCommands::Search {
                query,
                concept,
                category,
                limit,
            })) => {
                assert_eq!(query.as_deref(), Some("monitor swaps"));
                assert!(concept.is_none());
                assert!(category.is_none());
                assert_eq!(limit, Some(5));
                assert!(cli.json);
            }
            _ => panic!("expected know search command"),
        }

        let cli = Cli::try_parse_from([
            "a4",
            "know",
            "program",
            "meteora-cp-amm",
            "--section",
            "instructions",
        ])
        .expect("cli should parse");
        match cli.command {
            Some(Commands::Know(KnowCommands::Program { slug, section })) => {
                assert_eq!(slug, "meteora-cp-amm");
                assert_eq!(section.as_deref(), Some("instructions"));
            }
            _ => panic!("expected know program command"),
        }
    }

    #[test]
    fn parse_program_pagination_cursors() {
        let cli = Cli::try_parse_from(["a4", "program", "list", "--cursor", "upc_next-page_123"])
            .expect("program list cursor should parse");
        match cli.command {
            Some(Commands::Program(ProgramCommands::List { cursor })) => {
                assert_eq!(cursor.as_deref(), Some("upc_next-page_123"));
            }
            _ => panic!("expected program list command"),
        }

        let cli = Cli::try_parse_from([
            "a4",
            "program",
            "events",
            "upr_abcdefghijklmnopqrstuvwxyzABCDEF",
            "--after",
            "uev_00000000001",
        ])
        .expect("program events cursor should parse");
        match cli.command {
            Some(Commands::Program(ProgramCommands::Events { after, .. })) => {
                assert_eq!(after.as_deref(), Some("uev_00000000001"));
            }
            _ => panic!("expected program events command"),
        }
    }

    #[test]
    fn parse_legacy_stack_entity_explore() {
        let cli = Cli::try_parse_from(["a4", "explore", "ore", "Position"])
            .expect("legacy CLI should parse");
        match cli.command {
            Some(Commands::Explore {
                target,
                reference,
                entity,
            }) => {
                assert_eq!(target.as_deref(), Some("ore"));
                assert_eq!(reference.as_deref(), Some("Position"));
                assert!(entity.is_none());
            }
            _ => panic!("expected explore command"),
        }
    }
}
