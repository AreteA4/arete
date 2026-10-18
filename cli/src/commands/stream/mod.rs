pub(crate) mod client;
pub(crate) mod dev;
mod filter;
mod once;
mod output;
pub(crate) mod protocols;
pub(crate) mod session;
mod snapshot;
mod store;
pub(crate) mod token;
#[cfg(feature = "tui")]
mod tui;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use arete_sdk::{Subscription, SubscriptionQuery};
use clap::Args;

use crate::api_client::{ApiClient, DeploymentPhase, DeploymentResponse, DeploymentStatus};
use crate::commands::stack::deployment_selection_key;

pub use once::{run as run_get, GetArgs, GET_HELP};

#[derive(Args)]
pub struct StreamArgs {
    /// View to subscribe to: EntityName/mode (e.g. OreRound/latest).
    /// With --dev: the protocol to stream (or a view of the default one);
    /// without it, lists the local protocols and their views
    pub view: Option<String>,

    /// With --dev: the protocol's view, by name (Activity, Instances,
    /// Participants) or id (ProtocolActivity/list)
    #[arg(value_name = "DEV_VIEW", requires = "dev")]
    pub dev_view: Option<String>,

    /// Entity key to watch (for state-mode subscriptions)
    #[arg(short, long)]
    pub key: Option<String>,

    /// WebSocket URL override
    #[arg(long)]
    pub url: Option<String>,

    /// Owned deployment or registry stack name; with --dev, the local stack
    #[arg(short, long)]
    pub stack: Option<String>,

    /// Stream from the running `a4 dev` session: finds it, lists its stacks,
    /// mints session tokens and follows redeploys
    #[arg(long, conflicts_with_all = ["url", "load"])]
    pub dev: bool,

    /// The a4 dev project [default: the nearest directory at or above the
    /// current one with .arete/dev/session.json]
    #[arg(long, value_name = "DIR", requires = "dev")]
    pub project: Option<PathBuf>,

    /// Subscribe to every view of the stack (except <Entity>/state views,
    /// which need --key), merged into one feed labelled by protocol, stack
    /// and view
    #[arg(long, requires = "dev")]
    pub all: bool,

    /// With --dev: one protocol instance, its state and its activity. The
    /// instance id `a4 dev step` prints, or the short id the stream keys
    /// its rows by
    #[arg(long, value_name = "ID", requires = "dev", conflicts_with_all = ["all", "key"])]
    pub instance: Option<String>,

    /// Output raw WebSocket frames instead of merged entities
    #[arg(long)]
    pub raw: bool,

    /// NO_DNA agent-friendly envelope format
    #[arg(long)]
    pub no_dna: bool,

    /// Filter expression: field=value, field>N, field~regex (repeatable, ANDed).
    /// Note: field? treats null as absent (returns false for null values)
    #[arg(long = "where", value_name = "EXPR")]
    pub filters: Vec<String>,

    /// Select specific fields to output (comma-separated dot paths). Nested paths are
    /// flattened to literal keys, e.g. --select "info.name" outputs {"info.name": "..."}
    #[arg(long)]
    pub select: Option<String>,

    /// Exit after first entity matches filter criteria
    #[arg(long)]
    pub first: bool,

    /// Filter by operation type (comma-separated: snapshot,upsert,patch,remove,delete).
    /// Snapshot entities are always tracked for state merging but only emitted
    /// when "snapshot" is in the allowed set.
    #[arg(long)]
    pub ops: Option<String>,

    /// Show running count of entities/updates only
    #[arg(long)]
    pub count: bool,

    /// Maximum size of the live query window
    #[arg(long)]
    pub take: Option<u32>,

    /// Skip N matching entities in the live query window
    #[arg(long)]
    pub skip: Option<u32>,

    /// Disable initial snapshot
    #[arg(long)]
    pub no_snapshot: bool,

    /// Resume from cursor (seq value)
    #[arg(long)]
    pub after: Option<String>,

    /// Record frames to a JSON snapshot file
    #[arg(long)]
    pub save: Option<String>,

    /// Auto-stop the stream after N seconds
    #[arg(long)]
    pub duration: Option<u64>,

    /// Replay a previously saved snapshot file instead of connecting live
    #[arg(
        long,
        conflicts_with = "url",
        conflicts_with = "tui",
        conflicts_with = "duration"
    )]
    pub load: Option<String>,

    /// Show update history for the specified --key entity
    #[arg(long)]
    pub history: bool,

    /// Show entity at a specific history index (0 = latest)
    #[arg(long)]
    pub at: Option<usize>,

    /// Show diff between consecutive updates
    #[arg(long)]
    pub diff: bool,

    /// Interactive TUI mode
    #[arg(long, short = 'i')]
    pub tui: bool,

    /// End the stream when the connection drops instead of reconnecting
    #[arg(long, conflicts_with = "max_reconnects")]
    pub no_reconnect: bool,

    /// Give up after N reconnect attempts in a row [default: no limit].
    /// A connection that stays up for 30 seconds starts the count again
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    pub max_reconnects: Option<u32>,
}

/// How `a4 stream` handles a dropped connection, for `a4 stream --help`.
pub const RECONNECT_HELP: &str = "\
Reconnecting:
  When the connection drops (the server restarts, or the connection is reset
  without a close frame), the stream reconnects. It waits up to 0.5s before
  the first attempt and twice as long before each further one, up to 30s.
  Each attempt mints a fresh session token for hosted stacks and resubscribes
  to the same view with the same options. The new subscription takes a fresh
  snapshot and the stream's entity state is rebuilt from it; updates sent
  while disconnected are not replayed. With --no-snapshot no snapshot is
  taken on reconnect either, and the stream continues with live updates only.

  --duration counts wall time across reconnects. A policy close from the
  server, such as an expired or refused session, is not retried and exits
  with an error, as does giving up after --max-reconnects attempts. Use
  --no-reconnect to end the stream on the first drop instead.

Local development (--dev):
  a4 stream --dev                         list the protocols, their views and counts
  a4 stream --dev --tui                   browse protocols, views, rows and instances
  a4 stream --dev <protocol>              the protocol's views, one labelled feed
  a4 stream --dev <protocol> Activity     one view: Instances, Activity, Participants,
                                          or an id such as ProtocolActivity/list
  a4 stream --dev <protocol> --instance <id>
                                          one instance's state and activity
  a4 stream --dev --all                   every view of the stack, one feed
  a4 stream --dev --stack <name> <VIEW>   a stack by name, one view by id

  A protocol is named by its spec name (localTipJar), its slug, its stack or
  its deployment's short id; with one protocol it can be left out. An
  instance is the id `a4 dev step` prints, or the short id the stream keys
  its rows by. The session is found from .arete/dev/session.json at or above
  the current directory (or --project). Tokens are minted from the local
  runtime for every connection and refreshed before they expire. When
  `a4 dev` redeploys, the stream moves to the new deployment and prints
  `— redeployed: <old> → <new>` on stderr. JSON lines carry \"stack\" and,
  for a protocol, \"protocol\" (and, with --raw, \"view\"); with several
  views on a terminal each line starts with `protocol · View`.

  In the TUI: Enter opens, Esc goes back, Tab switches between the
  protocols and their views, / filters. Enter on an instance opens its
  state and activity.";

/// The reconnect behaviour `args` ask for.
pub fn reconnect_policy(args: &StreamArgs) -> session::ReconnectPolicy {
    session::ReconnectPolicy::new(!args.no_reconnect, args.max_reconnects)
}

/// Flags that need a single view.
fn check_multi_view_args(args: &StreamArgs) -> Result<()> {
    let refused = [
        (args.key.is_some(), "--key"),
        (args.save.is_some(), "--save"),
        (args.count, "--count"),
        (
            args.history || args.at.is_some() || args.diff,
            "--history/--at/--diff",
        ),
        (args.after.is_some(), "--after"),
    ];
    if let Some((_, flag)) = refused.iter().find(|(set, _)| *set) {
        bail!("{flag} works on one view; drop --all or name a single view.");
    }
    Ok(())
}

/// The flags TUI mode does not take.
fn check_tui_args(args: &StreamArgs) -> Result<()> {
    if args.duration.is_some() {
        bail!("--duration has no effect in TUI mode; stop with 'q' or Ctrl+C.");
    }
    if args.count {
        bail!("--count is incompatible with TUI mode.");
    }
    if args.save.is_some() {
        bail!("--save is not yet supported in TUI mode; use 's' inside the TUI to save.");
    }
    if args.history || args.at.is_some() || args.diff {
        bail!(
            "--history/--at/--diff are not supported in TUI mode; use h/l keys to browse history."
        );
    }
    if args.raw {
        bail!("--raw is incompatible with TUI mode; omit --tui to use raw output.");
    }
    if args.no_dna {
        bail!("--no-dna is incompatible with TUI mode; omit --tui to use NO_DNA output.");
    }
    if !args.filters.is_empty() {
        bail!("--where is not supported in TUI mode; use '/' inside the TUI to filter.");
    }
    if args.select.is_some() {
        bail!("--select is not supported in TUI mode.");
    }
    if args.ops.is_some() {
        bail!("--ops is not supported in TUI mode.");
    }
    if args.first {
        bail!("--first is not supported in TUI mode.");
    }
    if cfg!(not(feature = "tui")) {
        bail!(
            "TUI mode requires the 'tui' feature.\n\
             Install with: cargo install a4-cli --features tui"
        );
    }
    Ok(())
}

/// `a4 stream --dev`.
fn run_dev(args: StreamArgs, json: bool) -> Result<()> {
    if args.all {
        check_multi_view_args(&args)?;
    }
    if args.tui {
        check_tui_args(&args)?;
    }
    let rt = tokio::runtime::Runtime::new().context("Failed to create async runtime")?;
    rt.block_on(async move {
        let runtime = dev::DevRuntime::open(args.project.as_deref())?;
        let listing = runtime.listing().await?;
        let request = protocols::DevRequest {
            first: args.view.clone(),
            second: args.dev_view.clone(),
            stack: args.stack.clone(),
            all: args.all,
            instance: args.instance.clone(),
        };
        let target = protocols::resolve_target(&request, &listing.stacks, &runtime.session)?;

        // The protocol browser, for all but --all and --key (the tabbed and
        // single-view TUIs).
        #[cfg(feature = "tui")]
        if args.tui && !args.all && args.key.is_none() {
            if listing.stacks.is_empty() {
                bail!("{}", dev::no_stacks(&runtime.session));
            }
            return tui::browse(runtime, listing, target, &args).await;
        }

        if target == protocols::DevTarget::Overview {
            if args.tui {
                bail!("--key needs a view: a4 stream --dev <protocol> <VIEW> --key <key> --tui");
            }
            if json {
                println!("{}", serde_json::to_string(&listing)?);
            } else {
                println!("{}", protocols::render_listing(&listing, &runtime.session));
            }
            return Ok(());
        }

        let name = target.stack().unwrap_or_default();
        let stack = dev::select_stack(&listing.stacks, Some(name), &runtime.session)?;
        let feeds = dev_feeds(stack, &target)?;
        if feeds.is_empty() {
            bail!(
                "Stack '{}' lists no views to stream without --key.",
                stack.name
            );
        }
        if feeds.len() > 1 {
            check_multi_view_args(&args)?;
        }
        if let protocols::DevTarget::Instance { key, .. } = &target {
            eprintln!("Instance {}", key.describe());
        }

        let mut refresh = runtime.refresh_for(stack);
        let url = refresh
            .renew()
            .await
            .context("Could not mint a session for the local stack")?;
        // Follow the stack the user named, or whichever stack the active
        // protocol deployment moves to.
        let requested = target.pinned().then(|| stack.name.clone());
        let follow = dev::watch(runtime.clone(), requested, stack.identity());

        if args.tui {
            #[cfg(feature = "tui")]
            return tui::run_tui_feeds(url, Some(refresh), Some(follow), feeds, &args).await;
        }
        if let Some(notice) = protocols::fork_notice(&listing, stack) {
            eprintln!("Note: {notice}.");
        }
        let labels: Vec<String> = feeds.iter().map(client::Feed::label).collect();
        eprintln!(
            "Streaming {} from {} ...",
            labels.join(", "),
            token::redact_hs_token_for_display(&url)
        );
        client::stream_feeds(
            url,
            Some(refresh),
            Some(follow),
            &feeds,
            &args,
            reconnect_policy(&args),
        )
        .await
    })
}

/// The feeds `target` streams from `stack`, labelled with its protocol.
fn dev_feeds(stack: &dev::DevStack, target: &protocols::DevTarget) -> Result<Vec<client::Feed>> {
    use protocols::{DevTarget, ViewEntry};

    let protocol = stack
        .is_protocol()
        .then(|| stack.protocol_name().to_string());
    let catalog = stack.catalog();
    let feed = |view: &str, title: &str| client::Feed {
        protocol: protocol.clone(),
        title: protocol.as_ref().map(|_| title.to_string()),
        ..client::Feed::new(view, Some(stack.name.clone()))
    };
    let of = |entry: &ViewEntry| feed(&entry.raw, &entry.friendly);
    Ok(match target {
        DevTarget::Overview => Vec::new(),
        DevTarget::View { view, .. } => vec![of(protocols::resolve_view(&catalog, view)?)],
        DevTarget::All { .. } => catalog
            .iter()
            .filter(|entry| entry.streamable())
            .map(of)
            .collect(),
        DevTarget::Protocol { .. } => protocols::primary_views(stack).iter().map(of).collect(),
        DevTarget::Instance { key, .. } => {
            let (state, activity) = protocols::instance_views(stack);
            let mut feeds = vec![client::Feed {
                key: Some(key.key.clone()),
                ..feed(&state, "Instance")
            }];
            if let Some(activity) = activity {
                feeds.push(client::Feed {
                    filters: vec![format!("instance={}", key.key)],
                    ..feed(&activity, "Activity")
                });
            }
            feeds
        }
    })
}

pub fn run(args: StreamArgs, config_path: &str, json: bool) -> Result<()> {
    if args.dev {
        return run_dev(args, json);
    }
    if args.all || args.project.is_some() || args.instance.is_some() {
        bail!("--all, --instance and --project need --dev.");
    }

    // --load mode: replay from file, no WebSocket needed
    // (--load + --tui conflict is enforced by clap at the arg level)
    if let Some(load_path) = &args.load {
        let player = snapshot::SnapshotPlayer::load(load_path)?;
        let default_view = player.header.view.clone();
        let view = args.view.as_deref().unwrap_or(&default_view);
        let rt = tokio::runtime::Runtime::new().context("Failed to create async runtime")?;
        return rt.block_on(client::replay(player, view, &args));
    }

    let view = match args.view.as_deref() {
        Some(v) => v,
        None => bail!("<VIEW> argument is required (e.g. OreRound/latest)"),
    };

    let url = resolve_url(&args, config_path, view)?;
    let (url, refresh) = token::ensure_hosted_ws_token(url)?;

    let rt = tokio::runtime::Runtime::new().context("Failed to create async runtime")?;

    if args.tui {
        check_tui_args(&args)?;
        #[cfg(feature = "tui")]
        {
            return rt.block_on(tui::run_tui(url, refresh, view, &args));
        }
    }

    eprintln!(
        "Connecting to {} ...",
        token::redact_hs_token_for_display(&url)
    );

    rt.block_on(client::stream(url, refresh, view, &args))
}

/// The subscription for one feed: its own key, else `--key`.
pub fn build_feed_subscription(feed: &client::Feed, args: &StreamArgs) -> Subscription {
    let key = feed.key.as_deref().or(args.key.as_deref());
    let mut query = SubscriptionQuery::new(&feed.view);
    if let Some(key) = key {
        query = query.with_key(key);
    }
    if let Some(take) = args.take {
        query = query.with_take(take as usize);
    }
    if let Some(skip) = args.skip {
        query = query.with_skip(skip as usize);
    }
    if let Some(after) = &args.after {
        query = query.after(after);
    }
    Subscription::new(format!("a4-cli:{}", uuid::Uuid::new_v4().simple()), query)
        .with_snapshot(!args.no_snapshot)
}

fn validate_ws_url(url: &str) -> Result<()> {
    if !url.starts_with("ws://") && !url.starts_with("wss://") {
        bail!("Invalid URL scheme. Expected ws:// or wss://, got: {}", url);
    }
    Ok(())
}

fn resolve_url(args: &StreamArgs, _config_path: &str, _view: &str) -> Result<String> {
    resolve_ws_url(args.url.as_deref(), args.stack.as_deref())
}

/// The WebSocket URL for `--url`, or for the owned deployment or hosted
/// registry stack `--stack` names.
fn resolve_ws_url(url: Option<&str>, stack: Option<&str>) -> Result<String> {
    // 1. Explicit --url
    if let Some(url) = url {
        validate_ws_url(url)?;
        return Ok(url.to_string());
    }

    // 2. Explicit owned deployment or hosted registry stack name
    if let Some(stack_name) = stack {
        return resolve_stack_url(&ApiClient::new()?, stack_name);
    }

    bail!(
        "Could not determine WebSocket URL.\n\n\
         Specify one of:\n  \
         --url wss://your-stack.stack.arete.run\n  \
         --stack <name>  (resolves an owned deployment or hosted registry endpoint)",
    )
}

fn resolve_stack_url(client: &ApiClient, stack_name: &str) -> Result<String> {
    if client.has_api_key() {
        if let Some(deployment) = find_serving_owned_deployment(client, stack_name)? {
            let url = deployment.websocket_url;
            validate_ws_url(&url)?;
            return Ok(url);
        }
    }

    let install = client.get_registry_stack_install(stack_name, None)?;
    let url = install.websocket_url.ok_or_else(|| {
        anyhow::anyhow!(
            "Hosted stack '{stack_name}' has no single WebSocket endpoint; pass --url for the desired live binding"
        )
    })?;
    validate_ws_url(&url)?;
    Ok(url)
}

fn find_serving_owned_deployment(
    client: &ApiClient,
    stack_name: &str,
) -> Result<Option<DeploymentResponse>> {
    const PAGE_SIZE: i64 = 100;

    let mut offset = 0;
    let mut selected: Option<DeploymentResponse> = None;
    loop {
        let page = client.list_deployments_page(PAGE_SIZE, offset)?;
        let page_len = page.len() as i64;
        for deployment in page {
            let serving = matches!(
                (deployment.status, deployment.live_status.phase),
                (
                    DeploymentStatus::Active | DeploymentStatus::Updating,
                    DeploymentPhase::Running | DeploymentPhase::Updating
                )
            );
            if deployment.spec_name.eq_ignore_ascii_case(stack_name)
                && deployment.branch.is_none()
                && serving
                && selected.as_ref().is_none_or(|current| {
                    deployment_selection_key(&deployment) > deployment_selection_key(current)
                })
            {
                selected = Some(deployment);
            }
        }
        if page_len < PAGE_SIZE {
            break;
        }
        offset += page_len;
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::test_support::MockServer;

    fn deployment_response(
        id: i32,
        spec_name: &str,
        status: &str,
        phase: &str,
        websocket_url: &str,
        last_deployed_at: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "spec_id": 38,
            "spec_name": spec_name,
            "atom_name": "ore-m4jgyh",
            "branch": null,
            "current_build_id": 191,
            "current_spec_version_id": 38,
            "current_version": 1,
            "portable_ast_hash": null,
            "deployment_release_hash": null,
            "current_idl_program_ids": [],
            "current_image_tag": null,
            "websocket_url": websocket_url,
            "http_url": "https://ore-m4jgyh.custom.example",
            "websocket_auth": {},
            "http_auth": {},
            "transaction_relay_enabled": false,
            "status": status,
            "status_message": null,
            "first_deployed_at": "2026-09-05T00:00:00Z",
            "last_deployed_at": last_deployed_at,
            "live_status": {
                "phase": phase,
                "desired_replicas": 1,
                "ready_replicas": 1,
                "available_replicas": 1,
                "updated_replicas": 1,
                "last_transition_time": null,
                "source": "kubernetes",
                "error_category": null
            },
            "latest_operation": null
        })
    }

    fn deployments_response(deployments: Vec<serde_json::Value>) -> String {
        serde_json::Value::Array(deployments).to_string()
    }

    fn registry_response() -> String {
        r#"{"name":"ore","stack":"arete:h1:stack-manifest:sha256:test","websocketUrl":"wss://managed-ore.stack.arete.run","httpUrl":null,"websocketAuth":null,"httpAuth":null,"description":null,"visibility":"public","specVersionId":null,"liveSpecHash":null,"liveSpec":null,"liveSpecs":[],"stackManifestHash":"arete:h1:stack-manifest:sha256:test","stackManifest":{},"chainBinding":null,"transactionBinding":null,"extensions":null,"programs":[]}"#.to_string()
    }

    fn parse(args: &[&str]) -> Result<StreamArgs, clap::Error> {
        #[derive(clap::Parser)]
        struct Cli {
            #[command(flatten)]
            args: StreamArgs,
        }
        <Cli as clap::Parser>::try_parse_from(std::iter::once("a4").chain(args.iter().copied()))
            .map(|cli| cli.args)
    }

    #[test]
    fn hosted_streams_still_take_one_view() {
        let args = parse(&["OreRound/latest", "--key", "k", "--stack", "ore"]).unwrap();
        assert_eq!(args.view.as_deref(), Some("OreRound/latest"));
        assert_eq!(args.dev_view, None);
        assert!(!args.dev);
        // A second positional, --instance and --all are --dev only.
        assert!(parse(&["OreRound/latest", "Activity"]).is_err());
        assert!(parse(&["OreRound/latest", "--instance", "x"]).is_err());
        assert!(parse(&["--all"]).is_err());
    }

    #[test]
    fn dev_streams_take_a_protocol_a_view_or_an_instance() {
        let args = parse(&["--dev", "localTipJar", "Activity", "--tui"]).unwrap();
        assert_eq!(args.view.as_deref(), Some("localTipJar"));
        assert_eq!(args.dev_view.as_deref(), Some("Activity"));
        assert!(args.tui);

        let args = parse(&["--dev", "localTipJar", "--instance", "8c2b1c56"]).unwrap();
        assert_eq!(args.instance.as_deref(), Some("8c2b1c56"));
        assert_eq!(args.dev_view, None);

        let args = parse(&["--dev", "ProtocolActivity/list"]).unwrap();
        assert_eq!(args.view.as_deref(), Some("ProtocolActivity/list"));

        // --all may name the protocol; --instance takes no --all or --key.
        assert!(parse(&["--dev", "localTipJar", "--all"]).is_ok());
        assert!(parse(&["--dev", "--all", "--instance", "x"]).is_err());
        assert!(parse(&["--dev", "--key", "k", "--instance", "x"]).is_err());
    }

    #[test]
    fn a_feed_takes_its_own_key() {
        let args = parse(&["--dev", "--key", "fallback"]).unwrap();
        let keyed = client::Feed {
            key: Some("short".to_string()),
            ..client::Feed::new("ProtocolInstance/state", None)
        };
        assert_eq!(
            build_feed_subscription(&keyed, &args).query.key.as_deref(),
            Some("short")
        );
        let plain = client::Feed::new("ProtocolInstance/state", None);
        assert_eq!(
            build_feed_subscription(&plain, &args).query.key.as_deref(),
            Some("fallback")
        );
    }

    #[test]
    fn stack_name_prefers_the_authenticated_owners_serving_deployment() {
        let server = MockServer::json(
            200,
            &deployments_response(vec![deployment_response(
                29,
                "Ore",
                "active",
                "running",
                "wss://ore-m4jgyh.custom.example",
                "2026-09-05T00:01:00Z",
            )]),
        );
        let client =
            ApiClient::with_base_url(server.base_url()).with_api_key("a4_ak_test".to_string());

        let url = resolve_stack_url(&client, "ore").expect("owned deployment resolves");

        assert_eq!(url, "wss://ore-m4jgyh.custom.example");
        let request = server.request();
        assert_eq!(
            request.request_line,
            "GET /api/deployments?limit=100&offset=0 HTTP/1.1"
        );
        assert_eq!(request.header("authorization"), Some("Bearer a4_ak_test"));
    }

    #[test]
    fn inactive_owned_stack_does_not_shadow_the_registry() {
        let server = MockServer::json_sequence(vec![
            (
                200,
                deployments_response(vec![deployment_response(
                    29,
                    "ore",
                    "stopped",
                    "scaled_down",
                    "wss://stopped.example.test",
                    "2026-09-05T00:01:00Z",
                )]),
            ),
            (200, registry_response()),
        ]);
        let client =
            ApiClient::with_base_url(server.base_url()).with_api_key("a4_ak_test".to_string());

        let url = resolve_stack_url(&client, "ore").expect("registry fallback resolves");

        assert_eq!(url, "wss://managed-ore.stack.arete.run");
        assert_eq!(
            server.request().request_line,
            "GET /api/deployments?limit=100&offset=0 HTTP/1.1"
        );
        assert_eq!(
            server.request().request_line,
            "GET /api/registry/stacks/ore/install?capabilities=managed-solana-gateway-v1 HTTP/1.1"
        );
    }

    #[test]
    fn owned_stack_uses_established_deployment_precedence() {
        let server = MockServer::json(
            200,
            &deployments_response(vec![
                deployment_response(
                    40,
                    "ore",
                    "updating",
                    "updating",
                    "wss://newer-updating.example.test",
                    "2026-09-05T00:02:00Z",
                ),
                deployment_response(
                    29,
                    "ORE",
                    "active",
                    "running",
                    "wss://active.example.test",
                    "2026-09-05T00:01:00Z",
                ),
            ]),
        );
        let client =
            ApiClient::with_base_url(server.base_url()).with_api_key("a4_ak_test".to_string());

        let url = resolve_stack_url(&client, "Ore").expect("serving deployment resolves");

        assert_eq!(url, "wss://active.example.test");
    }

    #[test]
    fn unauthenticated_stack_name_resolves_the_registry_endpoint() {
        let server = MockServer::json(200, &registry_response());
        let client = ApiClient::with_base_url(server.base_url());

        let url = resolve_stack_url(&client, "ore").expect("registry stack resolves");

        assert_eq!(url, "wss://managed-ore.stack.arete.run");
        assert_eq!(
            server.request().request_line,
            "GET /api/registry/stacks/ore/install?capabilities=managed-solana-gateway-v1 HTTP/1.1"
        );
    }
}
