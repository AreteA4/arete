//! Which surfaces (CLI command groups and MCP tools) an account sees.
//!
//! `GET /api/auth/me` reports the command groups the account may use
//! (`commandGroups`; an admin gets every group). `a4` and `a4 mcp` hide what
//! the account is not entitled to. Hiding is UX only: the API still refuses
//! calls the account may not make.
//!
//! The groups always come from the API. When there is no account key, the
//! API can't be reached, or it does not report groups, the groups are
//! unknown ([`Surface::Everything`]): everything is listed and the API
//! decides, except the `install` group. Project installs generate SDKs on
//! this machine, so they are listed and run only when the API positively
//! reports `install` (admins have every group).

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::Value;

/// How long startup waits for the account's groups before listing
/// everything.
pub const LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);

/// One command group: a set of CLI commands and MCP tools granted together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// Sign-in, keys, `doctor`, config, self-update. Always listed here,
    /// whatever the API reports, so an account can always log in.
    Account,
    /// Sandbox wallets (`sandbox_*` tools).
    Sandboxes,
    /// Live stack streams, views and snapshots (`a4 stream`, `a4 get`), and
    /// program reads (`a4 program account`).
    Streams,
    /// The account's own stacks and programs (`a4 create`, `a4 up`,
    /// `a4 stack`, `a4 program`).
    Stacks,
    /// Protocols and their Squads multisigs (`a4 protocol`, `a4 squads`).
    Protocols,
    /// Dev environments (`a4 dev`, `dev_*` tools).
    Dev,
    /// Discovery (`a4 explore`, `a4 know`, the catalog and knowledge tools).
    Registry,
    /// Project installs: `a4 install`, `a4 update`, `a4 remove`, `a4 sdk`,
    /// and the `arete.toml` / `arete.lock` flow that generates SDKs from the
    /// registry. The registry's resolver refuses accounts without it.
    Install,
}

impl Group {
    pub const ALL: [Group; 8] = [
        Group::Account,
        Group::Sandboxes,
        Group::Streams,
        Group::Stacks,
        Group::Protocols,
        Group::Dev,
        Group::Registry,
        Group::Install,
    ];

    /// The group's name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Group::Account => "account",
            Group::Sandboxes => "sandboxes",
            Group::Streams => "streams",
            Group::Stacks => "stacks",
            Group::Protocols => "protocols",
            Group::Dev => "dev",
            Group::Registry => "registry",
            Group::Install => "install",
        }
    }

    /// A wire name, case-insensitively. Unknown names are `None` (a newer
    /// API's groups).
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        Group::ALL.into_iter().find(|group| group.as_str() == name)
    }
}

/// What the account sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Surface {
    /// No groups to go by (no key, API unreachable, or an API that does not
    /// report groups): every command and tool except the `install` group's.
    Everything,
    /// Only the always-listed account commands and these groups.
    Groups(BTreeSet<Group>),
}

impl Surface {
    /// Decode a `GET /api/auth/me` body. Only an explicit `commandGroups`
    /// list narrows the surface; without one it is [`Surface::Everything`].
    pub fn from_account(value: &Value) -> Self {
        let Some(groups) = ["commandGroups", "command_groups"]
            .iter()
            .find_map(|key| value.get(*key).and_then(Value::as_array))
        else {
            return Surface::Everything;
        };
        Surface::Groups(
            groups
                .iter()
                .filter_map(Value::as_str)
                .filter_map(Group::parse)
                .collect(),
        )
    }

    /// Whether `group` is granted. With no groups to go by, every group but
    /// `install` is: project installs need the API to report it.
    pub fn allows(&self, group: Group) -> bool {
        match self {
            Surface::Everything => group != Group::Install,
            Surface::Groups(groups) => groups.contains(&group),
        }
    }

    /// Whether something in `groups` is visible: account commands always
    /// are, anything else when one of its groups is granted.
    pub fn allows_any(&self, groups: &[Group]) -> bool {
        groups.contains(&Group::Account) || groups.iter().any(|group| self.allows(*group))
    }
}

/// Internal hook for this repository's CI, which builds example SDKs and
/// runs integration tests from local files with no account. Not a product
/// setting. See [`ci_install_hook`].
const CI_INSTALL_HOOK_ENV: &str = "ARETE_INSTALL_WITHOUT_ACCOUNT";

/// Whether the CI hook is on, reading variables through `var`: the hook
/// variable is `1` and the process runs under CI (`CI` is `true` or `1`, or
/// `GITHUB_ACTIONS` is `true`). The hook variable alone does nothing.
#[doc(hidden)]
pub fn ci_install_hook(var: impl Fn(&str) -> Option<String>) -> bool {
    let is = |name: &str, accepted: &[&str]| {
        var(name)
            .is_some_and(|value| accepted.contains(&value.trim().to_ascii_lowercase().as_str()))
    };
    is(CI_INSTALL_HOOK_ENV, &["1"]) && (is("CI", &["true", "1"]) || is("GITHUB_ACTIONS", &["true"]))
}

/// `surface`, widened to every group when it is unknown and the CI hook
/// ([`ci_install_hook`]) is on. Groups the API reports are never widened.
#[doc(hidden)]
pub fn with_install_opt_in(surface: Surface) -> Surface {
    widen_unknown(surface, ci_install_hook(|name| std::env::var(name).ok()))
}

/// `surface`, widened to every group when it is unknown and `opted_in`.
#[doc(hidden)]
pub fn widen_unknown(surface: Surface, opted_in: bool) -> Surface {
    if opted_in && surface == Surface::Everything {
        Surface::Groups(Group::ALL.into_iter().collect())
    } else {
        surface
    }
}

/// What discovery output tells an account without the `install` group in
/// place of an SDK install command: the descriptor it is reading, and the
/// sandbox path every account has.
pub const WITHOUT_INSTALL_HINT: &str = "Generating SDKs (`a4 install`) needs the `install` command group, which this account does not have. Read schemas, instructions and accounts from this descriptor, and act through a sandbox: `sandbox_preview` checks an action without signing, `sandbox_invoke` runs it (CLI: `a4 sandbox`).";

impl Surface {
    /// Whether output may suggest a project install (`a4 install ...`).
    pub fn can_install(&self) -> bool {
        self.allows(Group::Install)
    }
}

/// The groups an MCP tool belongs to; `None` is a tool this table does not
/// know (listed, and caught by the test that classifies every tool).
pub fn mcp_tool_groups(tool: &str) -> Option<&'static [Group]> {
    const ACCOUNT: &[Group] = &[Group::Account];
    const SANDBOXES: &[Group] = &[Group::Sandboxes];
    const STREAM_READS: &[Group] = &[Group::Streams, Group::Sandboxes];
    const PROGRAM_READS: &[Group] = &[Group::Stacks, Group::Streams, Group::Sandboxes];
    // Discovery is how an agent works out what to do, so every account with
    // sandboxes (the default) has it too.
    const DISCOVERY: &[Group] = &[Group::Registry, Group::Sandboxes];
    const INSTALL: &[Group] = &[Group::Install];
    // Raw artifacts are SDK inputs, and discovery reads them too.
    const ARTIFACTS: &[Group] = &[Group::Install, Group::Registry];
    const PROTOCOLS: &[Group] = &[Group::Protocols];
    const DEV: &[Group] = &[Group::Dev];
    Some(match tool {
        "ping" | "account_status" | "rename_agent" | "create_claim_link" => ACCOUNT,
        // The installed SDKs of this project: only an install writes them.
        "describe_sdk" => INSTALL,
        "resolve_artifact" => ARTIFACTS,
        "search_catalog"
        | "get_catalog_entry"
        | "list_catalog_vocabulary"
        | "search_knowledge"
        | "get_protocol"
        | "get_program_knowledge"
        | "get_recipe"
        | "list_concepts" => DISCOVERY,
        // Live views and program reads: a sandbox reads its own programs
        // through them when it is the only one of their groups granted.
        "connect" | "disconnect" | "read_view" | "subscribe" | "unsubscribe" | "query_entities"
        | "get_entity" | "list_entities" | "get_recent" | "list_subscriptions"
        | "list_connections" => STREAM_READS,
        "program_read_account" => PROGRAM_READS,
        _ if tool.starts_with("sandbox_") => SANDBOXES,
        _ if tool.starts_with("explore_") => DISCOVERY,
        _ if tool.starts_with("dev_") => DEV,
        _ if tool.starts_with("protocol_") || tool.starts_with("squads_") => PROTOCOLS,
        _ => return None,
    })
}

/// Whether `a4 mcp` lists `tool` for this surface.
pub fn mcp_tool_visible(surface: &Surface, tool: &str) -> bool {
    mcp_tool_groups(tool).is_none_or(|groups| surface.allows_any(groups))
}

/// The account's surface from `GET {api_url}/api/auth/me`, with the stored
/// or `ARETE_API_KEY` key for that API. Everything when there is no key, the
/// key may not go to `api_url`, or the call fails in any way.
pub async fn fetch_surface(api_url: &str) -> Surface {
    with_install_opt_in(fetch_reported_surface(api_url).await)
}

/// The surface `GET /api/auth/me` reports, before any opt-in.
async fn fetch_reported_surface(api_url: &str) -> Surface {
    let api_url = api_url.trim().trim_end_matches('/');
    let key = match crate::credentials::resolve_key_for_api(api_url) {
        Ok(Some(key)) => key,
        _ => return Surface::Everything,
    };
    let url = format!("{api_url}/api/auth/me");
    if crate::credentials::check_key_destination(&key, &url).is_err() {
        return Surface::Everything;
    }
    let response = reqwest::Client::new()
        .get(&url)
        .timeout(LOOKUP_TIMEOUT)
        .bearer_auth(&key.key)
        .send()
        .await;
    match response {
        Ok(response) if response.status().is_success() => response
            .json::<Value>()
            .await
            .map(|body| Surface::from_account(&body))
            .unwrap_or(Surface::Everything),
        _ => Surface::Everything,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_an_explicit_group_list_narrows_the_surface() {
        // An API that does not report groups: nothing is hidden.
        let older = json!({"accountKind": "agent", "plan": "free", "capabilities": []});
        assert_eq!(Surface::from_account(&older), Surface::Everything);

        let granted = json!({"commandGroups": ["account", "sandboxes", "Registry", "teleport"]});
        assert_eq!(
            Surface::from_account(&granted),
            Surface::Groups(
                [Group::Account, Group::Sandboxes, Group::Registry]
                    .into_iter()
                    .collect()
            )
        );

        let snake = json!({"command_groups": []});
        assert_eq!(
            Surface::from_account(&snake),
            Surface::Groups(BTreeSet::new())
        );
    }

    #[test]
    fn account_tools_are_always_listed_and_grouped_tools_need_their_group() {
        // Even a list without `account` keeps sign-in and health.
        let sandboxes = Surface::Groups([Group::Sandboxes].into_iter().collect());
        assert!(mcp_tool_visible(&sandboxes, "ping"));
        assert!(mcp_tool_visible(&sandboxes, "account_status"));
        assert!(mcp_tool_visible(&sandboxes, "sandbox_invoke"));
        assert!(!mcp_tool_visible(&sandboxes, "dev_status"));
        assert!(!mcp_tool_visible(&sandboxes, "protocol_submit"));
        // A sandbox reads its own programs and their stacks.
        for read in ["connect", "read_view", "subscribe", "program_read_account"] {
            assert!(mcp_tool_visible(&sandboxes, read), "{read}");
        }
        let registry = Surface::Groups([Group::Registry].into_iter().collect());
        assert!(!mcp_tool_visible(&registry, "read_view"));
        assert!(!mcp_tool_visible(&registry, "program_read_account"));
        let stacks = Surface::Groups([Group::Stacks].into_iter().collect());
        assert!(mcp_tool_visible(&stacks, "program_read_account"));
        assert!(!mcp_tool_visible(&stacks, "read_view"));
        // Discovery is open to sandboxes (the default); SDK generation isn't.
        assert!(mcp_tool_visible(&sandboxes, "explore_stacks"));
        assert!(!mcp_tool_visible(&sandboxes, "describe_sdk"));
        assert!(mcp_tool_visible(&Surface::Everything, "dev_status"));
    }

    /// Discovery and installs are separate grants: `registry` lists the
    /// explore and catalog tools but not the installed-SDK reference, which
    /// needs `install`; raw artifacts are listed with either.
    #[test]
    fn install_is_its_own_group() {
        let registry = Surface::Groups([Group::Registry].into_iter().collect());
        assert!(mcp_tool_visible(&registry, "explore_stack"));
        assert!(mcp_tool_visible(&registry, "resolve_artifact"));
        assert!(!mcp_tool_visible(&registry, "describe_sdk"));
        let install = Surface::Groups([Group::Install].into_iter().collect());
        assert!(mcp_tool_visible(&install, "describe_sdk"));
        assert!(mcp_tool_visible(&install, "resolve_artifact"));
        assert!(!mcp_tool_visible(&install, "explore_stack"));
        assert_eq!(
            Surface::from_account(&json!({"commandGroups": ["install"]})),
            Surface::Groups([Group::Install].into_iter().collect())
        );
    }

    /// An account whose `GET /api/auth/me` reports only `account` and
    /// `sandboxes` sees every live-view and program-read tool, which then
    /// read through its sandbox, and none of the other groups' tools.
    #[test]
    fn a_sandbox_only_account_sees_the_read_tools() {
        let surface = Surface::from_account(&json!({"commandGroups": ["account", "sandboxes"]}));
        for read in [
            "connect",
            "disconnect",
            "read_view",
            "subscribe",
            "unsubscribe",
            "query_entities",
            "get_entity",
            "list_entities",
            "get_recent",
            "list_subscriptions",
            "list_connections",
            "program_read_account",
        ] {
            assert_eq!(
                mcp_tool_groups(read).map(|groups| groups.contains(&Group::Sandboxes)),
                Some(true),
                "{read}"
            );
            assert!(mcp_tool_visible(&surface, read), "{read}");
        }
        for tool in crate::sandbox::TOOL_NAMES {
            assert!(mcp_tool_visible(&surface, tool), "{tool}");
        }
        // Discovery: explore, the catalog and knowledge.
        for discovery in [
            "explore_stacks",
            "explore_stack",
            "explore_stack_views",
            "explore_stack_schema",
            "explore_programs",
            "explore_program",
            "explore_protocols",
            "explore_protocol",
            "search_catalog",
            "get_catalog_entry",
            "list_catalog_vocabulary",
            "search_knowledge",
            "get_protocol",
            "get_program_knowledge",
            "get_recipe",
            "list_concepts",
        ] {
            assert!(mcp_tool_visible(&surface, discovery), "{discovery}");
        }
        // Not SDK generation or raw SDK inputs, nor other groups' tools.
        for hidden in [
            "describe_sdk",
            "resolve_artifact",
            "protocol_submit",
            "squads_status",
            "dev_status",
        ] {
            assert!(!mcp_tool_visible(&surface, hidden), "{hidden}");
        }
    }

    /// With no groups to go by, everything is listed except the `install`
    /// group's tools: generating SDKs needs the API to report `install`.
    #[test]
    fn unknown_groups_hide_the_install_tools() {
        assert!(!Surface::Everything.allows(Group::Install));
        assert!(!Surface::Everything.can_install());
        assert!(!mcp_tool_visible(&Surface::Everything, "describe_sdk"));
        for listed in [
            "explore_stack",
            "resolve_artifact",
            "dev_status",
            "protocol_submit",
            "connect",
        ] {
            assert!(mcp_tool_visible(&Surface::Everything, listed), "{listed}");
        }
        let admin = Surface::Groups(Group::ALL.into_iter().collect());
        assert!(admin.can_install());
        assert!(mcp_tool_visible(&admin, "describe_sdk"));
    }

    /// The CI hook needs both its variable and a CI environment; the
    /// variable alone does nothing, and it never widens reported groups.
    #[test]
    fn the_ci_hook_needs_a_ci_environment() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                vars.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        const HOOK: &str = "ARETE_INSTALL_WITHOUT_ACCOUNT";
        assert!(!ci_install_hook(env(&[])));
        assert!(!ci_install_hook(env(&[(HOOK, "1")])));
        assert!(!ci_install_hook(env(&[(HOOK, "1"), ("CI", "false")])));
        assert!(!ci_install_hook(env(&[(HOOK, "1"), ("CI", "")])));
        assert!(!ci_install_hook(env(&[(HOOK, "true"), ("CI", "true")])));
        assert!(!ci_install_hook(env(&[("CI", "true")])));
        assert!(ci_install_hook(env(&[(HOOK, "1"), ("CI", "true")])));
        assert!(ci_install_hook(env(&[(HOOK, "1"), ("CI", "1")])));
        assert!(ci_install_hook(env(&[
            (HOOK, "1"),
            ("GITHUB_ACTIONS", "true")
        ])));

        let admin = Surface::Groups(Group::ALL.into_iter().collect());
        assert_eq!(widen_unknown(Surface::Everything, true), admin);
        assert_eq!(
            widen_unknown(Surface::Everything, false),
            Surface::Everything
        );
        let default = Surface::Groups([Group::Account, Group::Sandboxes].into_iter().collect());
        assert_eq!(widen_unknown(default.clone(), true), default);
    }

    #[test]
    fn group_names_round_trip() {
        for group in Group::ALL {
            assert_eq!(Group::parse(group.as_str()), Some(group));
        }
    }
}
