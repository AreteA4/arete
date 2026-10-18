//! Listed protocol packages: callable protocols hosted on Arete that their
//! publishers listed in the public registry (`GET /api/registry/protocols`).
//!
//! These are runnable packages (`a4 install protocol`), not the curated
//! knowledge protocols of `search_knowledge` and `get_protocol`. A listing
//! carries its trust tier: `reviewed` when Arete reviewed the release (with
//! the review's hash), `unreviewed` otherwise.
//!
//! The helpers here are shared by the `explore_protocols` MCP tool and
//! `a4 protocol search`/`show`, so both describe a listing the same way.

use serde_json::{json, Map, Value};

/// What a reviewed listing is called.
pub const REVIEWED_LABEL: &str = "reviewed by Arete";
/// What an unreviewed listing is called.
pub const UNREVIEWED_LABEL: &str = "unreviewed";

/// Whether Arete reviewed the listed release.
pub fn is_reviewed(listing: &Value) -> bool {
    listing["reviewedByArete"] == true || listing["trustTier"] == "reviewed"
}

/// `reviewed by Arete` or `unreviewed`.
pub fn trust_label(listing: &Value) -> &'static str {
    if is_reviewed(listing) {
        REVIEWED_LABEL
    } else {
        UNREVIEWED_LABEL
    }
}

/// The command that installs `package_ref` (`<package>@<version>`) at its
/// exact version.
pub fn install_command(package_ref: &str) -> String {
    match package_ref.rsplit_once('@') {
        Some((package, version)) if !package.is_empty() && !version.is_empty() => {
            let version = version.trim_start_matches('=');
            format!("a4 install protocol {package}@={version}")
        }
        _ => format!("a4 install protocol {package_ref}"),
    }
}

/// One listing as the tool returns it: the registry's fields plus `trust`
/// (a readable trust tier) and `install` (the exact install command).
pub fn listing_view(listing: &Value) -> Value {
    let mut view = match listing {
        Value::Object(object) => object.clone(),
        _ => Map::new(),
    };
    view.insert("trust".into(), json!(trust_label(listing)));
    if let Some(package_ref) = listing["packageRef"].as_str() {
        view.insert("install".into(), json!(install_command(package_ref)));
    }
    Value::Object(view)
}

/// A page of listings (`{ items, nextCursor? }`) with each item through
/// [`listing_view`] and a `hint`.
pub fn listings_view(page: &Value) -> Value {
    let items: Vec<Value> = page["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(listing_view)
        .collect();
    let mut view = json!({ "items": items });
    let mut hint = String::from(
        "Callable protocol packages hosted on Arete, reviewed ones first. `trust` is \
         \"reviewed by Arete\" (with `reviewHash`) or \"unreviewed\"; `attestedExecutions30d` \
         counts attested executions in the last 30 days. Install one with its `install` \
         command; `protocol_actions` lists what a wallet can do on its deployment.",
    );
    if let Some(cursor) = page["nextCursor"]
        .as_str()
        .filter(|cursor| !cursor.is_empty())
    {
        view["nextCursor"] = json!(cursor);
        hint.push_str(" Pass `nextCursor` back as `cursor` with the same `query` for more.");
    }
    view["hint"] = json!(hint);
    view
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(tier: &str) -> Value {
        json!({
            "slug": "tip-jar",
            "name": "Tip Jar",
            "version": "1.2.0",
            "packageRef": "acme/tip-jar@1.2.0",
            "deploymentId": "6f1c0e2a-0000-4000-8000-000000000000",
            "trustTier": tier,
            "reviewedByArete": tier == "reviewed",
            "steps": [{ "name": "tip", "scope": "create", "access": "public" }],
            "attestedExecutions30d": 12
        })
    }

    #[test]
    fn trust_tiers_read_as_reviewed_by_arete_or_unreviewed() {
        assert_eq!(trust_label(&listing("reviewed")), "reviewed by Arete");
        assert_eq!(trust_label(&listing("unreviewed")), "unreviewed");
        assert_eq!(trust_label(&json!({})), "unreviewed");
    }

    #[test]
    fn install_commands_pin_the_exact_version() {
        assert_eq!(
            install_command("acme/tip-jar@1.2.0"),
            "a4 install protocol acme/tip-jar@=1.2.0"
        );
        assert_eq!(install_command("tip-jar"), "a4 install protocol tip-jar");
    }

    #[test]
    fn a_page_keeps_the_registry_fields_and_adds_trust_install_and_cursor() {
        let page = json!({ "items": [listing("reviewed")], "nextCursor": "c2" });
        let view = listings_view(&page);
        let item = &view["items"][0];
        assert_eq!(item["slug"], "tip-jar");
        assert_eq!(item["attestedExecutions30d"], 12);
        assert_eq!(item["steps"][0]["scope"], "create");
        assert_eq!(item["trust"], "reviewed by Arete");
        assert_eq!(item["install"], "a4 install protocol acme/tip-jar@=1.2.0");
        assert_eq!(view["nextCursor"], "c2");
        assert!(view["hint"].as_str().unwrap().contains("cursor"));

        let last = listings_view(&json!({ "items": [] }));
        assert!(last.get("nextCursor").is_none());
        assert_eq!(last["items"], json!([]));
    }
}
