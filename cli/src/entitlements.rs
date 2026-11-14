//! Hide the command groups the account is not entitled to from help,
//! suggestions and completions.
//!
//! The groups come from the Arete API (`GET /api/auth/me`, see
//! [`arete_mcp::entitlements`]). Without an account key, when the API can't
//! be reached, or when it reports no groups, every command is listed as
//! before. Hiding is UX only: a command typed anyway still runs, and the API
//! refuses what the account may not use. The exception is the project
//! install commands ([`INSTALL_COMMANDS`]): typed by an account the API
//! reports without `install`, they stop with [`require_install`]'s error
//! before doing anything, local or remote. A command named on the command line
//! (`a4 explore --help`, `a4 help explore`, `a4 get` with a missing
//! argument) is never hidden: its own help and errors are shown as for any
//! account.
//!
//! Read commands (`get`, `stream`, `program account`) are also listed for an
//! account granted `sandboxes`: a sandbox reads the programs in its rules and
//! their stacks through them (`--sandbox`).

use arete_mcp::entitlements::{Group, Surface};
use clap::Command;
use std::ffi::OsString;

use crate::api_client::ApiClient;

/// The command groups of a top-level command; `None` is a command this
/// table does not know (listed, and caught by the test that classifies every
/// command). `account` commands are listed for every account.
pub fn command_groups(name: &str) -> Option<&'static [Group]> {
    Some(match name {
        "init" | "doctor" | "self" | "upgrade" | "mcp" | "auth" | "login" | "config"
        | "telemetry" | "help" => &[Group::Account],
        "stream" | "get" => &[Group::Streams, Group::Sandboxes],
        "up" | "status" | "stack" | "build" | "idl" => &[Group::Stacks],
        // `program account` reads; the rest manage the account's programs
        // (see `subcommand_groups`).
        "program" => &[Group::Stacks, Group::Streams, Group::Sandboxes],
        "explore" | "know" => &[Group::Registry],
        "install" | "update" | "remove" | "sdk" => &[Group::Install],
        "protocol" | "squads" => &[Group::Protocols],
        "dev" => &[Group::Dev],
        "sandbox" => &[Group::Sandboxes],
        // Templates for stack apps and for protocols.
        "create" => &[Group::Stacks, Group::Protocols],
        _ => return None,
    })
}

/// The groups of a subcommand that differ from its parent's: `None` when the
/// subcommand has its parent's groups.
pub fn subcommand_groups(parent: &str, name: &str) -> Option<&'static [Group]> {
    match (parent, name) {
        ("program", "account" | "help") => None,
        ("program", _) => Some(&[Group::Stacks]),
        _ => None,
    }
}

/// The help footer when `visible` leaves commands out: they still run, and
/// naming one shows its help.
pub const HIDDEN_COMMANDS_NOTE: &str = "\
Commands outside this account's command groups are not listed. Most still run
when the API allows them; project installs (install, update, remove, sdk) need
the `install` command group. `a4 help <command>` (or `a4 <command> --help`)
shows a command's help.";

/// The project install commands: they write `arete.toml` / `arete.lock` and
/// generate SDKs from the registry, and need the `install` group.
pub const INSTALL_COMMANDS: [&str; 4] = ["install", "update", "remove", "sdk"];

/// What an account without the `install` group is told when it runs one of
/// [`INSTALL_COMMANDS`].
pub const INSTALL_GROUP_REQUIRED: &str = "\
this account does not have the `install` command group, which `a4 install`, \
`a4 update`, `a4 remove` and `a4 sdk` need (they generate SDKs from the \
registry and write arete.toml / arete.lock). Ask the Arete team for access. \
Without it, act through a sandbox: `a4 sandbox list`, or the sandbox MCP tools \
(`sandbox_preview`, then `sandbox_invoke`).";

/// The `--config` path `main` parsed, for lookups made deep inside a command.
static CONFIG_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static CAN_INSTALL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Remember the `--config` path for [`can_install`]. `main` calls it once.
pub fn remember_config_path(config: &str) {
    let _ = CONFIG_PATH.set(config.to_string());
}

/// Whether output may suggest a project install (`a4 install ...`) to this
/// account: `GET /api/auth/me` reports `install`, or reports no groups at
/// all. Looked up at most once per process.
pub fn can_install() -> bool {
    if cfg!(test) {
        return true;
    }
    *CAN_INSTALL.get_or_init(|| {
        let config = CONFIG_PATH.get().map_or("arete.toml", String::as_str);
        selected_account_surface(config).can_install()
    })
}

/// Refuse `command` when it is one of [`INSTALL_COMMANDS`] and `surface`
/// lacks `install`. [`Surface::Everything`] (no key, API unreachable, or an
/// API that reports no groups) passes: the API decides.
pub fn require_install(surface: &Surface, command: &str) -> anyhow::Result<()> {
    if INSTALL_COMMANDS.contains(&command) && !surface.allows(Group::Install) {
        anyhow::bail!("a4 {command}: {INSTALL_GROUP_REQUIRED}");
    }
    Ok(())
}

/// `cmd` without the top-level commands `surface` does not grant.
pub fn visible(cmd: Command, surface: &Surface) -> Command {
    if *surface == Surface::Everything {
        return cmd;
    }
    let all = cmd.get_subcommands().count();
    let kept: Vec<Command> = cmd
        .get_subcommands()
        .filter(|sub| {
            command_groups(sub.get_name()).is_none_or(|groups| surface.allows_any(groups))
        })
        .cloned()
        .map(|sub| hide_subcommands(sub, surface))
        .collect();
    // clap can't remove a subcommand, so rebuild the root with the same
    // settings (pinned by the test that compares the rendered help).
    let mut root = Command::new("a4").version(env!("CARGO_PKG_VERSION"));
    if let Some(about) = cmd.get_about() {
        root = root.about(about.clone());
    }
    if let Some(about) = cmd.get_long_about() {
        root = root.long_about(about.clone());
    }
    if kept.len() < all {
        root = root.after_help(HIDDEN_COMMANDS_NOTE);
    }
    root.args(cmd.get_arguments().cloned()).subcommands(kept)
}

/// `cmd` with the subcommands `surface` does not grant hidden from its help
/// and completions; they still run when named.
fn hide_subcommands(cmd: Command, surface: &Surface) -> Command {
    let parent = cmd.get_name().to_string();
    let hidden: Vec<String> = cmd
        .get_subcommands()
        .map(|sub| sub.get_name().to_string())
        .filter(|name| {
            subcommand_groups(&parent, name).is_some_and(|groups| !surface.allows_any(groups))
        })
        .collect();
    hidden.iter().fold(cmd, |cmd, name| {
        cmd.mut_subcommand(name, |sub| sub.hide(true))
    })
}

/// The top-level command the raw arguments name, as typed: the first
/// argument that is not an option (or an option's value), or the target of
/// `a4 help <command>`. `None` for `a4`, `a4 --help` and `a4 help`.
pub fn named_command(cmd: &Command, args: &[OsString]) -> Option<String> {
    let takes_value =
        |found: Option<&clap::Arg>| found.is_some_and(|arg| arg.get_action().takes_values());
    let mut positionals = Vec::new();
    let mut args = args.iter().skip(1).map(|arg| arg.to_string_lossy());
    while let Some(arg) = args.next() {
        if arg == "--" {
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            if !long.contains('=')
                && takes_value(cmd.get_arguments().find(|a| a.get_long() == Some(long)))
            {
                args.next();
            }
            continue;
        }
        if let Some(shorts) = arg.strip_prefix('-').filter(|rest| !rest.is_empty()) {
            // Short flags combine (`-yc path`, `-ycpath`): the first one that
            // takes a value ends the group, with the rest of the group as its
            // value, or the next argument when nothing is left.
            for (index, short) in shorts.char_indices() {
                if takes_value(cmd.get_arguments().find(|a| a.get_short() == Some(short))) {
                    if index + short.len_utf8() == shorts.len() {
                        args.next();
                    }
                    break;
                }
            }
            continue;
        }
        positionals.push(arg.into_owned());
        if positionals.len() == 2 || positionals[0] != "help" {
            break;
        }
    }
    let mut positionals = positionals.into_iter();
    match positionals.next()? {
        help if help == "help" => positionals.next(),
        name => Some(name),
    }
}

/// The account's surface for the API and profile these global options (or
/// the environment and the project's `arete.toml`) select.
pub fn account_surface(api_url: Option<&str>, profile: Option<&str>, config: &str) -> Surface {
    let api_url = crate::config::get_api_url(api_url);
    let inherited = std::env::var(arete_mcp::credentials::ENV_VAR_PROFILE).ok();
    let Ok(profile) = crate::config::resolve_auth_profile(profile, config, inherited.as_deref())
    else {
        return Surface::Everything;
    };
    let env_key = profile
        .is_none()
        .then(|| std::env::var(arete_mcp::credentials::ENV_VAR_API_KEY).ok())
        .flatten()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty());
    let key = match env_key {
        Some(key) => Some(key),
        None => ApiClient::load_optional_api_key_for_profile(&api_url, profile.as_deref())
            .ok()
            .flatten(),
    };
    match key {
        Some(key) => ApiClient::account_surface_at(&api_url, &key),
        None => Surface::Everything,
    }
}

/// The account's surface for the profile `main` already selected, which it
/// passes on in `ARETE_PROFILE`. Resolving the profile again would let the
/// project's default (`.arete/auth.toml`) override an explicit `--profile`.
pub fn selected_account_surface(config: &str) -> Surface {
    let profile = std::env::var(arete_mcp::credentials::ENV_VAR_PROFILE)
        .ok()
        .filter(|profile| !profile.trim().is_empty());
    account_surface(None, profile.as_deref(), config)
}

/// The value of a global option in raw arguments (`--name value`,
/// `--name=value`, `-s value`, `-svalue` or `-s=value`), before any `--`.
pub fn global_option(args: &[OsString], long: &str, short: Option<&str>) -> Option<String> {
    let mut args = args.iter().skip(1).map(|arg| arg.to_string_lossy());
    while let Some(arg) = args.next() {
        if arg == "--" {
            return None;
        }
        if arg == long || short.is_some_and(|short| arg == short) {
            return args.next().map(|value| value.into_owned());
        }
        if let Some(value) = arg
            .strip_prefix(long)
            .and_then(|rest| rest.strip_prefix('='))
        {
            return Some(value.to_string());
        }
        // `-cvalue` or `-c=value`, as clap accepts them.
        if let Some(value) = short
            .and_then(|short| arg.strip_prefix(short))
            .filter(|value| !value.is_empty())
        {
            return Some(value.strip_prefix('=').unwrap_or(value).to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn global_options_are_read_from_raw_arguments() {
        let raw = args(&[
            "a4",
            "--api-url=http://x",
            "help",
            "--profile",
            "agent",
            "-c",
            "a.toml",
        ]);
        assert_eq!(
            global_option(&raw, "--api-url", None).as_deref(),
            Some("http://x")
        );
        assert_eq!(
            global_option(&raw, "--profile", None).as_deref(),
            Some("agent")
        );
        assert_eq!(
            global_option(&raw, "--config", Some("-c")).as_deref(),
            Some("a.toml")
        );
        for attached in ["-cproject/arete.toml", "-c=project/arete.toml"] {
            let raw = args(&["a4", attached, "--help"]);
            assert_eq!(
                global_option(&raw, "--config", Some("-c")).as_deref(),
                Some("project/arete.toml")
            );
        }
        let after = args(&["a4", "get", "--", "--profile", "x"]);
        assert_eq!(global_option(&after, "--profile", None), None);
    }

    #[test]
    fn the_account_endpoint_decides_the_surface() {
        use crate::api_client::test_support::MockServer;
        // Offline: today's listing.
        assert_eq!(
            ApiClient::account_surface_at("http://127.0.0.1:1", "a4_sk_test"),
            Surface::Everything
        );
        // An API that does not report groups yet, or refuses the key.
        for (status, body) in [
            (
                200,
                r#"{"accountKind":"agent","plan":"free","capabilities":[]}"#,
            ),
            (401, r#"{"error":"unauthorized"}"#),
        ] {
            let server = MockServer::json(status, body);
            assert_eq!(
                ApiClient::account_surface_at(server.base_url(), "a4_sk_test"),
                Surface::Everything
            );
        }
        let server = MockServer::json(
            200,
            r#"{"accountKind":"agent","commandGroups":["account","sandboxes","registry"]}"#,
        );
        assert_eq!(
            ApiClient::account_surface_at(server.base_url(), "a4_sk_test"),
            Surface::Groups(
                [Group::Account, Group::Sandboxes, Group::Registry]
                    .into_iter()
                    .collect()
            )
        );
        let request = server.request();
        assert!(request.request_line.starts_with("GET /api/auth/me "));
        assert_eq!(request.header("authorization"), Some("Bearer a4_sk_test"));
    }

    #[test]
    fn project_installs_need_the_install_group() {
        let default = Surface::Groups([Group::Account, Group::Sandboxes].into_iter().collect());
        let registry = Surface::Groups([Group::Sandboxes, Group::Registry].into_iter().collect());
        let install = Surface::Groups([Group::Install].into_iter().collect());
        for command in INSTALL_COMMANDS {
            assert_eq!(
                command_groups(command),
                Some(&[Group::Install][..]),
                "{command}"
            );
            for surface in [&default, &registry] {
                let error = require_install(surface, command).unwrap_err().to_string();
                assert!(error.starts_with(&format!("a4 {command}: ")), "{error}");
                assert!(error.contains("`install` command group"), "{error}");
                assert!(error.contains("sandbox_preview"), "{error}");
            }
            require_install(&install, command).unwrap();
            // No groups to go by: the API decides.
            require_install(&Surface::Everything, command).unwrap();
        }
        // Discovery and everything else are not install commands.
        for command in ["explore", "know", "init", "doctor", "sandbox", "up"] {
            require_install(&default, command).unwrap();
        }
        assert_eq!(command_groups("explore"), Some(&[Group::Registry][..]));
        // Hidden from a default account's help, listed with the grant.
        let help = visible(<crate::Cli as clap::CommandFactory>::command(), &default)
            .render_help()
            .to_string();
        assert!(!help.contains("\n  install"), "{help}");
        let help = visible(<crate::Cli as clap::CommandFactory>::command(), &install)
            .render_help()
            .to_string();
        assert!(help.contains("\n  install"), "{help}");
    }
}
