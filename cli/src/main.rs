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
//! - `a4 sdk create` - Generate TypeScript/Rust SDK
//! - `a4 install` - Generate TypeScript/Rust SDK from a hosted stack
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

        /// Template: react-ore, rust-ore
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
    },

    /// Show overview of stacks, builds, and deployments
    Status,

    /// Discover stacks and explore their schemas
    Explore {
        /// Stack name to explore
        name: Option<String>,

        /// Entity name to show field details
        entity: Option<String>,
    },

    /// Push local stacks to remote (alias for 'stack push')
    Push {
        /// Name of specific stack to push (pushes all if not specified)
        stack_name: Option<String>,
    },

    /// Generate a TypeScript or Rust SDK from a hosted stack
    Install {
        /// Hosted stack identifier, or the reserved install target `program`
        target: String,

        /// Program install identifier when using `a4 install program <program>`
        install_name: Option<String>,

        /// Generate a TypeScript SDK
        #[arg(long, conflicts_with = "rust")]
        ts: bool,

        /// Generate a Rust SDK
        #[arg(long, conflicts_with = "ts")]
        rust: bool,

        /// Output path (file for TypeScript, directory for Rust)
        #[arg(short, long)]
        output: Option<String>,

        /// Package name for TypeScript
        #[arg(short, long)]
        package_name: Option<String>,

        /// Crate name for generated Rust crate
        #[arg(long)]
        crate_name: Option<String>,

        /// Generate Rust as a module (mod.rs) instead of a standalone crate
        #[arg(long)]
        module: bool,

        /// WebSocket URL for the stack
        #[arg(long)]
        url: Option<String>,

        /// Local extensions artifact source (manifest file, entry file, or directory)
        #[arg(long)]
        extensions: Option<String>,
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

    /// Build and validate portable live artifacts
    #[command(subcommand)]
    Live(LiveCommands),

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
    #[arg(long, conflicts_with = "rust")]
    ts: bool,

    /// Generate a Rust SDK
    #[arg(long, conflicts_with = "ts")]
    rust: bool,

    /// Output path (file for TypeScript, directory for Rust)
    #[arg(short, long)]
    output: Option<String>,

    /// Package name for TypeScript
    #[arg(short, long)]
    package_name: Option<String>,

    /// Crate name for generated Rust crate
    #[arg(long)]
    crate_name: Option<String>,

    /// Generate Rust as a module (mod.rs) instead of a standalone crate
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
    #[arg(long, conflicts_with = "rust")]
    program_only: bool,
}

#[derive(Args)]
struct SdkSyncArgs {
    /// Sync TypeScript SDKs only
    #[arg(long, conflicts_with = "rust")]
    ts: bool,

    /// Sync Rust SDKs only
    #[arg(long, conflicts_with = "ts")]
    rust: bool,

    /// Limit sync to one or more configured stack names
    #[arg(long = "stack", short = 's')]
    stacks: Vec<String>,
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

    /// Push local stacks with their stack file to remote
    Push {
        /// Name of specific stack to push (pushes all if not specified)
        stack_name: Option<String>,
    },

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

    /// Rollback to a previous deployment
    Rollback {
        /// Name of the stack to rollback
        stack_name: String,

        /// Rollback to specific version number (uses previous successful if not specified)
        #[arg(long)]
        to: Option<i32>,

        /// Rollback to specific build ID
        #[arg(long)]
        build: Option<i32>,

        /// Branch deployment to rollback (default: production)
        #[arg(long, default_value = "production")]
        branch: String,

        /// Force full rebuild instead of using existing image
        #[arg(long)]
        rebuild: bool,

        /// Don't watch the rollback progress
        #[arg(long)]
        no_wait: bool,
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
enum LiveCommands {
    /// Normalize a supported legacy stack into LiveSpec and ProgramSpec artifacts
    Build {
        /// Legacy .stack.json input
        input: String,

        /// LiveSpec output path
        #[arg(short, long)]
        output: Option<String>,

        /// Directory for ProgramSpec outputs
        #[arg(long)]
        program_dir: Option<String>,
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
    /// Create a new build from a stack (watches progress by default)
    Create {
        /// Name of the stack to build
        stack_name: String,

        /// Use specific version (default: latest)
        #[arg(short, long)]
        version: Option<i32>,

        /// Use local stack file directly instead of stack version
        #[arg(long)]
        ast_file: Option<String>,

        /// Don't wait for build to complete (return immediately)
        #[arg(long)]
        no_wait: bool,
    },

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
        Commands::Push { .. } => "push",
        Commands::Install { .. } => "install",
        Commands::Sdk(_) => "sdk",
        Commands::Config(_) => "config",
        Commands::Auth(_) => "auth",
        Commands::Stack(_) => "stack",
        Commands::Live(_) => "live",
        Commands::Program(_) => "program",
        Commands::Build(_) => "build",
        Commands::Telemetry(_) => "telemetry",
        Commands::Idl(_) => "idl",
        Commands::Stream(_) => "stream",
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
        } => commands::up::up(&cli.config, stack_name.as_deref(), branch, preview, dry_run),
        Commands::Status => commands::status::status(cli.json),
        Commands::Explore { name, entity } => match name {
            Some(name) => commands::explore::show(&name, entity.as_deref(), cli.json),
            None => commands::explore::list(cli.json),
        },
        Commands::Push { stack_name } => commands::stack::push(&cli.config, stack_name.as_deref()),
        Commands::Install {
            target,
            install_name,
            ts,
            rust,
            output,
            package_name,
            crate_name,
            module,
            url,
            extensions,
        } => commands::sdk::install_command(
            &target,
            install_name.as_deref(),
            ts,
            rust,
            output,
            package_name,
            crate_name,
            module,
            url,
            extensions,
        ),
        Commands::Sdk(sdk_cmd) => match sdk_cmd {
            SdkCommands::Create(create_args) => commands::sdk::create(
                &cli.config,
                create_args.stack_name.as_deref(),
                create_args.ts,
                create_args.rust,
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
            SdkCommands::Sync(sync_args) => {
                commands::sdk::sync(&cli.config, sync_args.ts, sync_args.rust, sync_args.stacks)
            }
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
            StackCommands::Push { stack_name } => {
                commands::stack::push(&cli.config, stack_name.as_deref())
            }
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
            StackCommands::Rollback {
                stack_name,
                to,
                build,
                branch,
                rebuild,
                no_wait,
            } => commands::stack::rollback(&stack_name, to, build, &branch, rebuild, !no_wait),
            StackCommands::Stop {
                stack_name,
                branch,
                force,
            } => commands::stack::stop(&stack_name, branch.as_deref(), force),
        },
        Commands::Live(live_cmd) => match live_cmd {
            LiveCommands::Build {
                input,
                output,
                program_dir,
            } => commands::public_artifacts::build_live(&input, output, program_dir),
        },
        Commands::Program(program_cmd) => match program_cmd {
            ProgramCommands::Build {
                input,
                output,
                program_id,
            } => commands::public_artifacts::build_program(&input, &output, program_id.as_deref()),
        },
        Commands::Build(build_cmd) => match build_cmd {
            BuildCommands::Create {
                stack_name,
                version,
                ast_file,
                no_wait,
            } => commands::build::create(
                &cli.config,
                &stack_name,
                version,
                ast_file.as_deref(),
                !no_wait,
            ),
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
    fn parse_install_stack_shorthand() {
        let cli = Cli::try_parse_from(["a4", "install", "ore"]).expect("cli should parse");

        match cli.command {
            Some(Commands::Install {
                target,
                install_name,
                ..
            }) => {
                assert_eq!(target, "ore");
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
                assert_eq!(target, "program");
                assert_eq!(install_name.as_deref(), Some("spl-token"));
                assert!(ts);
            }
            _ => panic!("expected install command"),
        }
    }
}
