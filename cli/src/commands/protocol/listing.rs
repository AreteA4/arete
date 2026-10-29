//! `a4 protocol search`, `show`, `request-listing` and `listing-status`:
//! the public registry of callable protocol packages hosted on Arete.
//!
//! Search and show are anonymous. A publisher asks for one of its
//! deployments to be listed with `request-listing` and follows it with
//! `listing-status`; both need the owner's secret key. Without
//! `--deployment` they find the deployment the way `a4 protocol deploy`
//! does: the project's protocol slug (`[protocol].slug`, else the built
//! spec's name), the account's deployments of it, and the one arete.lock
//! records for the project's own client or the active one of the built
//! release.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use arete_mcp::protocol_listing;
use clap::Args;
use colored::Colorize;
use serde_json::Value;

use crate::api_client::{ApiClient, ProtocolDeployment, ProtocolListingStatus};

/// Options for `a4 protocol search`.
#[derive(Args, Debug, Clone, Default)]
pub struct SearchArgs {
    /// Free text matched against protocol names and descriptions (omit to
    /// list every listed protocol)
    pub query: Option<String>,

    /// `nextCursor` of the previous page
    #[arg(long)]
    pub cursor: Option<String>,

    /// Results per page (1-50, default 20)
    #[arg(long)]
    pub limit: Option<usize>,
}

/// Options for `a4 protocol show`.
#[derive(Args, Debug, Clone, Default)]
pub struct ShowArgs {
    /// The listed protocol's slug (from `a4 protocol search`)
    pub slug: String,
}

/// Options for `a4 protocol request-listing` and `listing-status`.
#[derive(Args, Debug, Clone, Default)]
pub struct ListingArgs {
    /// The deployment id (default: the project's deployment, found the way
    /// `a4 protocol deploy` finds it)
    #[arg(long)]
    pub deployment: Option<String>,

    /// Protocol project directory, read when --deployment is not given
    #[arg(long, default_value = ".")]
    pub dir: PathBuf,
}

/// Options for `a4 protocol request-listing`.
#[derive(Args, Debug, Clone, Default)]
pub struct RequestListingArgs {
    #[command(flatten)]
    pub listing: ListingArgs,

    /// A note for the reviewer
    #[arg(long)]
    pub note: Option<String>,
}

/// `a4 protocol search`.
pub fn search(args: &SearchArgs, json: bool) -> Result<()> {
    let page = ApiClient::new()?.search_protocol_listings(
        args.query.as_deref(),
        args.cursor.as_deref(),
        args.limit,
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&page)?);
    } else {
        print!("{}", render_search(&page, args.query.as_deref()));
    }
    Ok(())
}

/// `a4 protocol show`.
pub fn show(args: &ShowArgs, json: bool) -> Result<()> {
    let listing = ApiClient::new()?.get_protocol_listing(&args.slug)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&listing)?);
    } else {
        print!("{}", render_listing(&listing));
    }
    Ok(())
}

/// `a4 protocol request-listing`.
pub fn request_listing(args: &RequestListingArgs, json: bool) -> Result<()> {
    let client = ApiClient::new()?;
    require_secret_key(&client, "request-listing")?;
    let deployment = resolve_deployment(&client, &args.listing)?;
    let status = client.request_protocol_listing(&deployment, args.note.as_deref())?;
    print_status(&status, json)
}

/// `a4 protocol listing-status`.
pub fn listing_status(args: &ListingArgs, json: bool) -> Result<()> {
    let client = ApiClient::new()?;
    require_secret_key(&client, "listing-status")?;
    let deployment = resolve_deployment(&client, args)?;
    let status = client.get_protocol_listing_status(&deployment)?;
    print_status(&status, json)
}

pub(super) fn require_secret_key(client: &ApiClient, command: &str) -> Result<()> {
    if !client.has_api_key() {
        bail!(
            "`a4 protocol {command}` needs the deployment owner's secret API key. Run `a4 login`, or set ARETE_API_KEY."
        );
    }
    if client.api_key_is_publishable() {
        bail!(
            "`a4 protocol {command}` needs a secret API key; the configured key is a publishable (browser) key."
        );
    }
    Ok(())
}

fn print_status(status: &ProtocolListingStatus, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(status)?);
    } else {
        print!("{}", render_status(status));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Finding the deployment
// ---------------------------------------------------------------------------

/// The deployment `args` names, else the project's.
pub fn resolve_deployment(client: &ApiClient, args: &ListingArgs) -> Result<String> {
    if let Some(deployment) = args.deployment.as_deref().map(str::trim) {
        uuid::Uuid::parse_str(deployment)
            .with_context(|| format!("--deployment '{deployment}' is not a deployment id"))?;
        return Ok(deployment.to_string());
    }
    let project = ProjectDeployment::read(&args.dir)?;
    let protocol = client
        .list_protocols()?
        .into_iter()
        .find(|protocol| protocol.slug == project.slug)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "This account has no protocol '{}'. Deploy it with `a4 protocol deploy`, or pass --deployment <id>.",
                project.slug
            )
        })?;
    let deployments = client.list_protocol_deployments(&protocol.id)?;
    select_deployment(&deployments, &project)
}

/// What the project says about its deployment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectDeployment {
    pub slug: String,
    /// The release the project's build derives.
    pub release_hash: Option<String>,
    /// The deployment arete.lock records for the project's own client.
    pub locked_deployment: Option<String>,
}

impl ProjectDeployment {
    pub fn read(dir: &Path) -> Result<Self> {
        let config = super::load_protocol_config(dir)?;
        let local = super::load_local_release(dir).ok();
        let slug = match (config.slug, &local) {
            (Some(slug), _) => slug,
            (None, Some(local)) => super::derive_slug(&local.spec.name)?,
            (None, None) => bail!(
                "{} is not a protocol project (no [protocol].slug in arete.toml and no build in {}). Pass --deployment <id>, or run this in the project.",
                dir.display(),
                super::PROTOCOL_BUILD_DIR
            ),
        };
        let locked_deployment = locked_deployment(dir, &slug);
        Ok(Self {
            slug,
            release_hash: local.map(|local| local.expected_release_hash),
            locked_deployment,
        })
    }
}

/// The deployment id arete.lock records for the project's client of its own
/// protocol, when it installs one.
fn locked_deployment(dir: &Path, slug: &str) -> Option<String> {
    use crate::project::installer::own_protocol_dependency;
    use crate::project::manifest::DependencyKind;
    use crate::project::{ProjectLock, ProjectManifest};

    let manifest = ProjectManifest::load(dir.join("arete.toml")).ok()?;
    let alias = own_protocol_dependency(&manifest, slug)?;
    let lock = ProjectLock::load_optional(dir.join("arete.lock")).ok()??;
    lock.dependencies
        .into_iter()
        .find(|entry| entry.kind == DependencyKind::Protocol && entry.alias == alias)?
        .protocol
        .map(|protocol| protocol.deployment_id)
        // A protocol `a4 dev` generated before its first deploy names none.
        .filter(|deployment| !deployment.is_empty())
}

/// The active deployment the lock records, else the active one of the
/// built release, else the only active one.
pub fn select_deployment(
    deployments: &[ProtocolDeployment],
    project: &ProjectDeployment,
) -> Result<String> {
    let active: Vec<&ProtocolDeployment> = deployments
        .iter()
        .filter(|deployment| deployment.status == "active")
        .collect();
    if let Some(locked) = project.locked_deployment.as_deref() {
        if let Some(deployment) = active.iter().find(|deployment| deployment.id == locked) {
            return Ok(deployment.id.clone());
        }
    }
    if let Some(release) = project.release_hash.as_deref() {
        let built: Vec<&&ProtocolDeployment> = active
            .iter()
            .filter(|deployment| deployment.release_hash == release)
            .collect();
        if let [only] = built.as_slice() {
            return Ok(only.id.clone());
        }
    }
    match active.as_slice() {
        [only] => Ok(only.id.clone()),
        [] => bail!(
            "Protocol '{}' has no active deployment. Deploy it with `a4 protocol deploy`, or pass --deployment <id>.",
            project.slug
        ),
        several => bail!(
            "Protocol '{}' has {} active deployments; pass --deployment with one of: {}",
            project.slug,
            several.len(),
            several
                .iter()
                .map(|deployment| format!("{} ({})", deployment.id, deployment.short_id))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    let mut out: String = value.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The `a4 protocol search` table.
pub fn render_search(page: &Value, query: Option<&str>) -> String {
    let items: Vec<&Value> = page["items"].as_array().into_iter().flatten().collect();
    if items.is_empty() {
        return match query.filter(|query| !query.trim().is_empty()) {
            Some(query) => format!("No listed protocols match '{query}'.\n"),
            None => "No protocols are listed yet.\n".to_string(),
        };
    }
    let mut out = format!(
        "{:<24} {:<10} {:<18} {:>10} {:>5}  {}\n",
        "SLUG", "VERSION", "TRUST", "EXEC (30D)", "STEPS", "PACKAGE"
    );
    for item in &items {
        let trust = protocol_listing::trust_label(item);
        out.push_str(&format!(
            "{:<24} {:<10} {:<18} {:>10} {:>5}  {}\n",
            truncate(text(item, "slug"), 24),
            truncate(text(item, "version"), 10),
            trust,
            item["attestedExecutions30d"].as_u64().unwrap_or(0),
            item["steps"].as_array().map_or(0, Vec::len),
            text(item, "packageRef"),
        ));
    }
    out.push_str("\nDetails and install command: a4 protocol show <slug>\n");
    if let Some(cursor) = page["nextCursor"]
        .as_str()
        .filter(|cursor| !cursor.is_empty())
    {
        out.push_str(&format!("More results: --cursor {cursor}\n"));
    }
    out
}

/// The `a4 protocol show` view.
pub fn render_listing(listing: &Value) -> String {
    let mut out = format!(
        "{} {}\n",
        text(listing, "name").bold(),
        text(listing, "version")
    );
    if let Some(description) = listing["description"].as_str().filter(|d| !d.is_empty()) {
        out.push_str(&format!("{description}\n"));
    }
    out.push('\n');
    let mut field = |label: &str, value: &str| {
        if !value.is_empty() {
            out.push_str(&format!("  {label:<22}{value}\n"));
        }
    };
    field("Slug", text(listing, "slug"));
    field("Package", text(listing, "packageRef"));
    if let Some(handle) = listing["publisher"]["handle"].as_str() {
        field("Publisher", handle);
    }
    let trust = if protocol_listing::is_reviewed(listing) {
        protocol_listing::REVIEWED_LABEL.green().to_string()
    } else {
        format!(
            "{} (Arete has not reviewed this protocol's code)",
            protocol_listing::UNREVIEWED_LABEL.yellow()
        )
    };
    field("Trust", &trust);
    field("Review", text(listing, "reviewHash"));
    let executions = listing["attestedExecutions30d"]
        .as_u64()
        .unwrap_or(0)
        .to_string();
    field("Attested runs (30d)", &executions);
    field("Deployment", text(listing, "deploymentId"));
    field("Attestation key", text(listing, "attestationPubkey"));
    field("Listed", text(listing, "listedAt"));
    let steps: Vec<&Value> = listing["steps"].as_array().into_iter().flatten().collect();
    out.push_str("\n  Steps\n");
    if steps.is_empty() {
        out.push_str("    (none)\n");
    }
    for step in steps {
        out.push_str(&format!(
            "    {:<24} scope {:<12} access {}\n",
            text(step, "name"),
            text(step, "scope"),
            text(step, "access")
        ));
    }
    if let Some(package_ref) = listing["packageRef"].as_str() {
        out.push_str(&format!(
            "\n  Install: {}\n",
            protocol_listing::install_command(package_ref).cyan()
        ));
    }
    out
}

/// The `request-listing`/`listing-status` view.
pub fn render_status(status: &ProtocolListingStatus) -> String {
    let mut out = format!(
        "Listing of deployment {}: {}\n",
        status.deployment_id,
        status.status.bold()
    );
    if let Some(tier) = &status.trust_tier {
        out.push_str(&format!(
            "  Trust tier: {}\n",
            if tier == "reviewed" {
                protocol_listing::REVIEWED_LABEL
            } else {
                tier.as_str()
            }
        ));
    }
    if let Some(reason) = &status.reason {
        out.push_str(&format!("  Reason:     {reason}\n"));
    }
    if let Some(updated_at) = &status.updated_at {
        out.push_str(&format!("  Updated:    {updated_at}\n"));
    }
    let next = match status.status.as_str() {
        "requested" => Some("Arete reviews the request; follow it with `a4 protocol listing-status`."),
        "listed" => Some("Listed: `a4 protocol search` finds it."),
        "rejected" => Some("Fix what the reason names, deploy again if needed, then `a4 protocol request-listing`."),
        "delisted" => Some("No longer listed; `a4 protocol request-listing` asks again."),
        _ => None,
    };
    if let Some(next) = next {
        out.push_str(&format!("\n{next}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::test_support::MockServer;
    use serde_json::json;

    fn deployment(id: &str, status: &str, release: &str) -> ProtocolDeployment {
        ProtocolDeployment {
            id: id.into(),
            protocol_id: None,
            environment_id: None,
            short_id: format!("s-{id}"),
            status: status.into(),
            revision: None,
            release_hash: release.into(),
            access_mode: "public".into(),
            attestation_key_id: None,
            trust_tier: None,
            runtime_kind: None,
        }
    }

    fn project(locked: Option<&str>, release: Option<&str>) -> ProjectDeployment {
        ProjectDeployment {
            slug: "tip-jar".into(),
            release_hash: release.map(str::to_owned),
            locked_deployment: locked.map(str::to_owned),
        }
    }

    #[test]
    fn the_locked_deployment_wins_then_the_built_release_then_the_only_active_one() {
        let deployments = [
            deployment("a", "active", "r1"),
            deployment("b", "active", "r2"),
            deployment("c", "disabled", "r3"),
        ];
        assert_eq!(
            select_deployment(&deployments, &project(Some("b"), Some("r1"))).unwrap(),
            "b"
        );
        assert_eq!(
            select_deployment(&deployments, &project(Some("c"), Some("r1"))).unwrap(),
            "a"
        );
        let error = select_deployment(&deployments, &project(None, Some("r3")))
            .unwrap_err()
            .to_string();
        assert!(error.contains("2 active deployments"), "{error}");
        assert!(error.contains("--deployment"), "{error}");
        assert_eq!(
            select_deployment(&deployments[..1], &project(None, None)).unwrap(),
            "a"
        );
        let error = select_deployment(&deployments[2..], &project(None, None))
            .unwrap_err()
            .to_string();
        assert!(error.contains("no active deployment"), "{error}");
    }

    #[test]
    fn an_explicit_deployment_must_be_an_id() {
        let client = ApiClient::with_base_url("http://127.0.0.1:9");
        let args = ListingArgs {
            deployment: Some("not-an-id".into()),
            dir: PathBuf::from("."),
        };
        assert!(resolve_deployment(&client, &args).is_err());
        let id = "6f1c0e2a-0000-4000-8000-000000000000";
        let args = ListingArgs {
            deployment: Some(id.into()),
            dir: PathBuf::from("."),
        };
        assert_eq!(resolve_deployment(&client, &args).unwrap(), id);
    }

    #[test]
    fn a_project_without_slug_or_build_asks_for_the_deployment() {
        let dir = tempfile::tempdir().unwrap();
        let error = ProjectDeployment::read(dir.path()).unwrap_err().to_string();
        assert!(error.contains("--deployment"), "{error}");
        std::fs::write(
            dir.path().join("arete.toml"),
            "[protocol]\nslug = \"tip-jar\"\n",
        )
        .unwrap();
        let project = ProjectDeployment::read(dir.path()).unwrap();
        assert_eq!(project.slug, "tip-jar");
        assert_eq!(project.release_hash, None);
        assert_eq!(project.locked_deployment, None);
    }

    fn page() -> Value {
        json!({
            "items": [{
                "slug": "tip-jar", "name": "Tip Jar", "version": "1.2.0",
                "description": "Tips for creators.",
                "packageRef": "acme/tip-jar@1.2.0",
                "deploymentId": "6f1c0e2a-0000-4000-8000-000000000000",
                "trustTier": "reviewed", "reviewedByArete": true,
                "reviewHash": "arete:h1:protocol-release-review:sha256:aa",
                "attestationPubkey": "11111111111111111111111111111111",
                "publisher": {"handle": "acme"},
                "steps": [{"name": "tip", "scope": "create", "access": "public"}],
                "attestedExecutions30d": 123,
                "listedAt": "2026-10-01T00:00:00Z"
            }, {
                "slug": "raffle", "name": "Raffle", "version": "0.1.0",
                "packageRef": "bob/raffle@0.1.0",
                "trustTier": "unreviewed", "reviewedByArete": false,
                "steps": [], "attestedExecutions30d": 0
            }],
            "nextCursor": "c2"
        })
    }

    #[test]
    fn the_search_table_shows_trust_executions_steps_and_package() {
        colored::control::set_override(false);
        let out = render_search(&page(), Some("tip"));
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("SLUG"), "{out}");
        assert!(lines[1].contains("tip-jar") && lines[1].contains("reviewed by Arete"));
        assert!(lines[1].contains("123") && lines[1].contains("acme/tip-jar@1.2.0"));
        assert!(lines[2].contains("raffle") && lines[2].contains("unreviewed"));
        assert!(out.contains("--cursor c2"), "{out}");
        assert_eq!(
            render_search(&json!({ "items": [] }), Some("zzz")),
            "No listed protocols match 'zzz'.\n"
        );
    }

    #[test]
    fn show_renders_the_review_steps_and_install_command() {
        colored::control::set_override(false);
        let out = render_listing(&page()["items"][0]);
        assert!(out.contains("reviewed by Arete"), "{out}");
        assert!(
            out.contains("arete:h1:protocol-release-review:sha256:aa"),
            "{out}"
        );
        assert!(out.contains("scope create"), "{out}");
        assert!(
            out.contains("a4 install protocol acme/tip-jar@=1.2.0"),
            "{out}"
        );
        let out = render_listing(&page()["items"][1]);
        assert!(out.contains("has not reviewed"), "{out}");
    }

    #[test]
    fn search_sends_the_filters_and_clamps_the_limit() {
        let server = MockServer::json(200, &page().to_string());
        let client = ApiClient::with_base_url(server.base_url());
        let result = client
            .search_protocol_listings(Some("tip jar"), Some("c1"), Some(500))
            .unwrap();
        assert_eq!(result["items"][0]["slug"], "tip-jar");
        let request = server.request();
        assert_eq!(
            request.request_line,
            "GET /api/registry/protocols?q=tip+jar&cursor=c1&limit=50 HTTP/1.1"
        );
    }

    #[test]
    fn request_listing_posts_the_note_with_the_secret_key() {
        let server = MockServer::json(
            200,
            r#"{"deploymentId":"6f1c0e2a-0000-4000-8000-000000000000","status":"requested","trustTier":"unreviewed","updatedAt":"2026-10-16T00:00:00Z"}"#,
        );
        let client = ApiClient::with_base_url(server.base_url()).with_api_key("a4_sk_test".into());
        let status = client
            .request_protocol_listing("6f1c0e2a-0000-4000-8000-000000000000", Some("hi"))
            .unwrap();
        assert_eq!(status.status, "requested");
        let request = server.request();
        assert_eq!(
            request.request_line,
            "POST /api/protocol-deployments/6f1c0e2a-0000-4000-8000-000000000000/listing HTTP/1.1"
        );
        assert_eq!(request.header("authorization"), Some("Bearer a4_sk_test"));
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).unwrap(),
            json!({ "note": "hi" })
        );
        colored::control::set_override(false);
        assert!(render_status(&status).contains("listing-status"));
    }

    #[test]
    fn listing_refusals_explain_what_to_do() {
        let server = MockServer::json(
            422,
            r#"{"error":"edges","code":"protocol-listing-edges-private","details":{"edges":["stack:private-feed"]}}"#,
        );
        let client = ApiClient::with_base_url(server.base_url()).with_api_key("a4_sk_test".into());
        let error = client
            .request_protocol_listing("6f1c0e2a-0000-4000-8000-000000000000", None)
            .unwrap_err();
        let api_error = error
            .downcast_ref::<crate::api_client::ApiClientError>()
            .unwrap();
        assert_eq!(
            api_error.problem.code.as_deref(),
            Some("protocol-listing-edges-private")
        );
        assert!(
            api_error.problem.error.contains("stack:private-feed"),
            "{error}"
        );
        assert!(api_error.problem.error.contains("public stacks"), "{error}");
        assert_eq!(
            serde_json::from_str::<Value>(&server.request().body).unwrap(),
            json!({})
        );

        for code in [
            "protocol-listing-not-public",
            "protocol-listing-fee-policy",
            "protocol-listing-not-owner",
            "protocol-listing-account-standing",
            "protocol-listing-not-found",
        ] {
            assert!(
                crate::api_client::protocol_listing_message(code, None).is_some(),
                "{code}"
            );
        }
        assert!(crate::api_client::protocol_listing_message("other", None).is_none());
    }
}
