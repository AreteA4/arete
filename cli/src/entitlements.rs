//! Hide the command groups the account is not entitled to from help,
//! suggestions and completions.
//!
//! The groups come from the Arete API (`GET /api/auth/me`, see
//! [`arete_mcp::entitlements`]). Without an account key, when the API can't
//! be reached, or when it reports no groups, every command is listed as
//! before. Hiding is UX only: a command typed anyway still runs, and the API
//! refuses what the account may not use.

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
        "stream" | "get" => &[Group::Streams],
        "up" | "status" | "stack" | "build" | "program" | "idl" => &[Group::Stacks],
        "explore" | "know" | "install" | "update" | "remove" | "sdk" => &[Group::Registry],
        "protocol" | "squads" => &[Group::Protocols],
        "dev" => &[Group::Dev],
        // Templates for stack apps and for protocols.
        "create" => &[Group::Stacks, Group::Protocols],
        _ => return None,
    })
}

/// `cmd` without the top-level commands `surface` does not grant.
pub fn visible(cmd: Command, surface: &Surface) -> Command {
    if *surface == Surface::Everything {
        return cmd;
    }
    let kept: Vec<Command> = cmd
        .get_subcommands()
        .filter(|sub| {
            command_groups(sub.get_name()).is_none_or(|groups| surface.allows_any(groups))
        })
        .cloned()
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
    root.args(cmd.get_arguments().cloned()).subcommands(kept)
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
}
