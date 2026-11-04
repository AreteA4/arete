//! Which surfaces (CLI command groups and MCP tools) an account sees.
//!
//! `GET /api/auth/me` may report the entitlement groups the account may use
//! (`entitlementGroups`) and whether it is an admin (`isAdmin`). `a4` and
//! `a4 mcp` hide what the account is not entitled to. Hiding is UX only: the
//! API still refuses calls the account may not make.
//!
//! The groups always come from the API. When there is no account key, the
//! API can't be reached, or it does not report groups, everything is listed
//! as before and the API decides.

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::Value;

/// How long startup waits for the account's groups before listing
/// everything.
pub const LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);

/// One entitlement group: a set of CLI commands and MCP tools granted
/// together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// Sandbox wallets (`sandbox_*` tools).
    Sandboxes,
    /// Catalog and knowledge discovery (`a4 explore`, `a4 know`).
    Catalog,
    /// Stack streams and deployments (`a4 stream`, `a4 get`, `a4 stack`, `a4 up`).
    Streams,
    /// SDK installs from the registry (`a4 install`, `a4 sdk`).
    Registry,
    /// Program artifacts (`a4 program`, `a4 idl`).
    Programs,
    /// Protocols and their Squads multisigs (`a4 protocol`, `a4 squads`).
    Protocols,
    /// Dev environments (`a4 dev`, `dev_*` tools).
    Dev,
}

impl Group {
    pub const ALL: [Group; 7] = [
        Group::Sandboxes,
        Group::Catalog,
        Group::Streams,
        Group::Registry,
        Group::Programs,
        Group::Protocols,
        Group::Dev,
    ];

    /// The group's name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Group::Sandboxes => "sandboxes",
            Group::Catalog => "catalog",
            Group::Streams => "streams",
            Group::Registry => "registry",
            Group::Programs => "programs",
            Group::Protocols => "protocols",
            Group::Dev => "dev",
        }
    }

    /// A wire name, case-insensitively. `install` is an alias of
    /// `registry`. Unknown names are `None` (a newer API's groups).
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        if name == "install" {
            return Some(Group::Registry);
        }
        Group::ALL.into_iter().find(|group| group.as_str() == name)
    }
}

/// What the account sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Surface {
    /// Every command and tool: an admin, or no groups to go by (no key, API
    /// unreachable, or an API that does not report groups).
    Everything,
    /// Only the always-available core and these groups.
    Groups(BTreeSet<Group>),
}

impl Surface {
    /// Decode a `GET /api/auth/me` body. Only an explicit group list
    /// narrows the surface; without one (or for an admin) it is
    /// [`Surface::Everything`].
    pub fn from_account(value: &Value) -> Self {
        let admin = ["isAdmin", "is_admin", "admin"]
            .iter()
            .any(|key| value.get(*key).and_then(Value::as_bool) == Some(true));
        if admin {
            return Surface::Everything;
        }
        let Some(groups) = ["entitlementGroups", "entitlement_groups"]
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

    pub fn allows(&self, group: Group) -> bool {
        match self {
            Surface::Everything => true,
            Surface::Groups(groups) => groups.contains(&group),
        }
    }

    /// Whether something in `groups` is visible: core (no groups) always
    /// is, otherwise when any of its groups is granted.
    pub fn allows_any(&self, groups: &[Group]) -> bool {
        groups.is_empty() || groups.iter().any(|group| self.allows(*group))
    }
}

/// The groups an MCP tool belongs to. `Some(&[])` is core, listed for every
/// account; `None` is a tool this table does not know (listed, and caught
/// by the test that classifies every tool).
pub fn mcp_tool_groups(tool: &str) -> Option<&'static [Group]> {
    const CORE: &[Group] = &[];
    const SANDBOXES: &[Group] = &[Group::Sandboxes];
    const CATALOG: &[Group] = &[Group::Catalog];
    const STREAMS: &[Group] = &[Group::Streams];
    const REGISTRY: &[Group] = &[Group::Registry];
    const PROTOCOLS: &[Group] = &[Group::Protocols];
    const DEV: &[Group] = &[Group::Dev];
    Some(match tool {
        "ping" | "account_status" | "create_claim_link" => CORE,
        "describe_sdk" => REGISTRY,
        "resolve_artifact"
        | "search_catalog"
        | "get_catalog_entry"
        | "list_catalog_vocabulary"
        | "search_knowledge"
        | "get_protocol"
        | "get_program_knowledge"
        | "get_recipe"
        | "list_concepts" => CATALOG,
        "connect" | "disconnect" | "read_view" | "subscribe" | "unsubscribe" | "query_entities"
        | "get_entity" | "list_entities" | "get_recent" | "list_subscriptions"
        | "list_connections" => STREAMS,
        _ if tool.starts_with("sandbox_") => SANDBOXES,
        _ if tool.starts_with("explore_") => CATALOG,
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
        // Today's API: no groups reported, so nothing is hidden.
        let today = json!({"accountKind": "agent", "plan": "free", "capabilities": []});
        assert_eq!(Surface::from_account(&today), Surface::Everything);

        let granted = json!({"entitlementGroups": ["sandboxes", "Install", "teleport"]});
        assert_eq!(
            Surface::from_account(&granted),
            Surface::Groups([Group::Sandboxes, Group::Registry].into_iter().collect())
        );

        let snake = json!({"entitlement_groups": []});
        assert_eq!(
            Surface::from_account(&snake),
            Surface::Groups(BTreeSet::new())
        );
    }

    #[test]
    fn admins_see_everything_whatever_groups_they_report() {
        for admin in [
            json!({"isAdmin": true, "entitlementGroups": ["sandboxes"]}),
            json!({"is_admin": true, "entitlementGroups": []}),
        ] {
            assert_eq!(Surface::from_account(&admin), Surface::Everything);
        }
        let not_admin = json!({"isAdmin": false, "entitlementGroups": ["dev"]});
        assert!(!Surface::from_account(&not_admin).allows(Group::Sandboxes));
    }

    #[test]
    fn core_tools_are_always_listed_and_grouped_tools_need_their_group() {
        let sandboxes = Surface::Groups([Group::Sandboxes].into_iter().collect());
        assert!(mcp_tool_visible(&sandboxes, "ping"));
        assert!(mcp_tool_visible(&sandboxes, "account_status"));
        assert!(mcp_tool_visible(&sandboxes, "sandbox_invoke"));
        assert!(!mcp_tool_visible(&sandboxes, "dev_status"));
        assert!(!mcp_tool_visible(&sandboxes, "protocol_submit"));
        assert!(!mcp_tool_visible(&sandboxes, "connect"));
        assert!(!mcp_tool_visible(&sandboxes, "explore_stacks"));
        assert!(!mcp_tool_visible(&sandboxes, "describe_sdk"));
        assert!(mcp_tool_visible(&Surface::Everything, "dev_status"));
    }

    #[test]
    fn group_names_round_trip() {
        for group in Group::ALL {
            assert_eq!(Group::parse(group.as_str()), Some(group));
        }
    }
}
