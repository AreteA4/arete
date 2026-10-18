//! `a4 stream --dev`: streams from the stacks serving in the project's
//! Arete cloud dev environment.
//!
//! The project is found from `.arete/dev/` (at or above the working
//! directory). The environment's stacks come from
//! `GET <api>/api/dev/environments/:env/stacks`, and
//! stack sessions are minted at `POST <api>/ws/sessions` with the account's
//! key, so no token is copied around: each connection gets a fresh one, and
//! an open connection is refreshed before its token expires.
//!
//! A watcher polls the listing while the stream runs. When `a4 dev`
//! redeploys (a new protocol deployment with a new stack, or a new
//! WebSocket address), the stream moves to it and says so.

use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use arete_mcp::dev::{DevClient, DevProject, DevSession};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::session::Retarget;
use super::token::SessionRefresh;

/// How often the watcher looks for a redeploy.
const POLL_PERIOD: Duration = Duration::from_secs(2);
/// How long a redeploy may sit half done (the stack reloaded for a new
/// deployment that is not active yet) before the stream moves anyway.
const SETTLE_LIMIT: Duration = Duration::from_secs(30);

/// One stack listed in an Arete cloud dev environment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevStack {
    pub name: String,
    pub ws_url: String,
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub generation: Option<i64>,
    #[serde(default)]
    pub views: Vec<String>,
    /// The protocol deployment whose current revision owns this stack.
    #[serde(default)]
    pub deployment_id: Option<String>,
    #[serde(default)]
    pub short_id: Option<String>,
    /// The protocol's slug.
    #[serde(default)]
    pub protocol: Option<String>,
    /// Runtime detail, when a stack reports its current protocol deployment.
    #[serde(default)]
    pub current: Option<DeploymentRef>,
    /// Every protocol deployment recorded on the stack, oldest first.
    #[serde(default)]
    pub deployments: Vec<DeploymentRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRef {
    pub deployment_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub short_id: Option<String>,
    /// The protocol's slug.
    #[serde(default)]
    pub protocol: Option<String>,
    /// The spec's `name` and `version`.
    #[serde(default)]
    pub protocol_name: Option<String>,
    #[serde(default)]
    pub protocol_version: Option<String>,
    /// Whether the spec declares a participant machine.
    #[serde(default)]
    pub has_participants: Option<bool>,
    #[serde(default)]
    pub instances: Option<InstanceCounts>,
    #[serde(default)]
    pub participants: Option<i64>,
}

/// A deployment's instances, as the local database counts them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceCounts {
    #[serde(default)]
    pub total: i64,
    /// In a terminal state.
    #[serde(default)]
    pub closed: i64,
    /// Created before the current Surfpool fork began.
    #[serde(default)]
    pub before_fork: i64,
}

/// When the current Surfpool fork began: `a4 dev up` starts it fresh with
/// the runtime, while the database keeps instances from earlier runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkMarker {
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub start_slot: Option<u64>,
    /// Instances, of any deployment, created before it.
    #[serde(default)]
    pub instances_before: i64,
}

impl ForkMarker {
    /// `restarted 14:02` (local time; with the date when not today).
    pub fn restarted(&self) -> Option<String> {
        let started = chrono::DateTime::parse_from_rfc3339(self.started_at.as_deref()?).ok()?;
        let local = started.with_timezone(&chrono::Local);
        let format = if local.date_naive() == chrono::Local::now().date_naive() {
            "%H:%M"
        } else {
            "%b %-d %H:%M"
        };
        Some(format!("restarted {}", local.format(format)))
    }

    /// `2 instances predate this fork (restarted 14:02); their chain history
    /// isn't on this fork`, or nothing when `count` is 0.
    pub fn notice(&self, count: i64) -> Option<String> {
        if count <= 0 {
            return None;
        }
        let (noun, verb) = if count == 1 {
            ("instance", "predates")
        } else {
            ("instances", "predate")
        };
        let when = self
            .restarted()
            .map(|restarted| format!(" ({restarted})"))
            .unwrap_or_default();
        let their = if count == 1 { "its" } else { "their" };
        Some(format!(
            "{count} {noun} {verb} this fork{when}; {their} chain history isn't on this fork"
        ))
    }
}

impl DevStack {
    /// Whether a redeploy is half done: `a4 dev` reloads the stack for a
    /// new deployment and records it before activating it, so for a moment
    /// the newest deployment on the stack is not the active one.
    pub fn settling(&self) -> bool {
        self.deployments
            .last()
            .is_some_and(|newest| Some(&newest.deployment_id) != self.deployment_id.as_ref())
    }

    /// What identifies one deploy of the stack: a change in any of it is a
    /// redeploy the stream follows.
    pub fn identity(&self) -> StackIdentity {
        StackIdentity {
            name: self.name.clone(),
            ws_url: self.ws_url.clone(),
            generation: self.generation,
            deployment_id: self.deployment_id.clone(),
            short_id: self.short_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackIdentity {
    pub name: String,
    pub ws_url: String,
    pub generation: Option<i64>,
    pub deployment_id: Option<String>,
    pub short_id: Option<String>,
}

impl StackIdentity {
    /// How a redeploy marker names it: the stack, its protocol deployment
    /// and the stack's generation.
    pub fn describe(&self) -> String {
        let deployment = self
            .short_id
            .clone()
            .or_else(|| self.deployment_id.clone())
            .unwrap_or_else(|| "no deployment".to_string());
        match self.generation {
            Some(generation) => format!("{} {deployment} (gen {generation})", self.name),
            None => format!("{} {deployment}", self.name),
        }
    }
}

/// `GET /dev/v1/stacks`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DevListing {
    pub stacks: Vec<DevStack>,
    /// Older runtimes leave it out.
    #[serde(default)]
    pub fork: Option<ForkMarker>,
}

/// The project's cloud dev environment, as `a4 stream --dev` reads it.
#[derive(Debug, Clone)]
pub struct DevRuntime {
    pub project: DevProject,
    pub session: DevSession,
    client: DevClient,
}

/// The views of a stack built from the protocol's local stack source:
/// every entity's `list` and `state` views and the views it declares.
pub fn views_of_stack_source(source: &serde_json::Value) -> Vec<String> {
    let mut views = Vec::new();
    for (entity, definition) in source["stackSource"]["entities"]
        .as_object()
        .into_iter()
        .flatten()
    {
        views.push(format!("{entity}/list"));
        views.push(format!("{entity}/state"));
        for name in definition["views"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, _)| name)
        {
            let view = format!("{entity}/{name}");
            if !views.contains(&view) {
                views.push(view);
            }
        }
    }
    views
}

impl DevRuntime {
    /// The project at or above `project` (default: the current directory).
    pub fn open(project: Option<&Path>) -> Result<Self> {
        let project = DevProject::discover(project)?;
        let client = DevClient::open(project.clone())?;
        Ok(Self {
            project,
            session: client.session.clone(),
            client,
        })
    }

    /// Read the session again: `a4 dev up` may have redeployed.
    pub fn reload(&mut self) {
        if let Ok(Some(selection)) = self.project.load_selection() {
            self.session = self.project.session_for(&selection);
        }
    }

    /// The stacks serving in the environment.
    pub async fn stacks(&self) -> Result<Vec<DevStack>> {
        Ok(self.listing().await?.stacks)
    }

    /// The stacks serving in the environment: each stack deployment that is
    /// assigned. Protocol identity and WebSocket URLs come from the listing;
    /// local views are attached only to the matching deployment or project stack.
    pub async fn listing(&self) -> Result<DevListing> {
        let api = self.client.env.api();
        let listing = api.stacks(self.client.env.env()).await?;
        let views = std::fs::read(
            self.project
                .root
                .join(".arete/protocol/protocol-stack-source.json"),
        )
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .map(|source| views_of_stack_source(&source))
        .unwrap_or_default();
        let project_stacks: serde_json::Value =
            std::fs::read(self.project.dev_dir().join("stacks.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
        let mut stacks = Vec::new();
        for entry in listing["stacks"].as_array().into_iter().flatten() {
            if entry["assigned"] != true {
                continue;
            }
            let Some(ws_url) = entry["websocketUrl"].as_str().filter(|url| !url.is_empty()) else {
                continue;
            };
            let name = entry["specName"].as_str().unwrap_or("stack");
            let deployment_id = entry["protocolDeploymentId"].as_str().map(str::to_string);
            let own_protocol =
                deployment_id.is_some() && deployment_id == self.session.deployment_id;
            let custom_views = project_stacks
                .as_array()
                .into_iter()
                .flatten()
                .find(|stack| stack["name"].as_str() == Some(name))
                .and_then(|stack| {
                    serde_json::from_value::<Vec<String>>(stack["views"].clone()).ok()
                })
                .unwrap_or_default();
            stacks.push(DevStack {
                name: name.to_string(),
                ws_url: ws_url.to_string(),
                ready: entry["status"]
                    .as_str()
                    .is_none_or(|status| !matches!(status, "failed" | "stopped" | "pending")),
                generation: entry["deploymentId"].as_i64(),
                views: if own_protocol {
                    views.clone()
                } else {
                    custom_views
                },
                deployment_id,
                short_id: own_protocol
                    .then(|| self.session.short_id.clone())
                    .flatten(),
                protocol: entry["protocolSlug"].as_str().map(str::to_string),
                current: None,
                deployments: Vec::new(),
            });
        }
        Ok(DevListing { stacks, fork: None })
    }

    /// Mints sessions for `stack`, carrying the row scopes of the protocol
    /// deployment recorded on it.
    pub fn refresh_for(&self, stack: &DevStack) -> SessionRefresh {
        SessionRefresh::local(
            self.session.session_endpoint.clone(),
            self.client.env.api().key().to_string(),
            stack.ws_url.clone(),
            stack.deployment_id.clone(),
        )
    }
}

/// The stack to stream: the one `requested` names; else the only one;
/// else the active protocol deployment's.
pub fn select_stack<'a>(
    stacks: &'a [DevStack],
    requested: Option<&str>,
    session: &DevSession,
) -> Result<&'a DevStack> {
    if let Some(name) = requested {
        return stacks
            .iter()
            .find(|stack| stack.name == name)
            .with_context(|| {
                format!(
                    "No stack named '{name}' in the a4 dev session.\n{}",
                    available(stacks)
                )
            });
    }
    match stacks {
        [] => bail!("{}", no_stacks(session)),
        [only] => Ok(only),
        _ => {
            let by_name = session
                .stack
                .as_deref()
                .and_then(|name| stacks.iter().find(|stack| stack.name == name));
            let by_deployment = || {
                session.deployment_id.as_deref().and_then(|deployment| {
                    stacks
                        .iter()
                        .find(|stack| stack.deployment_id.as_deref() == Some(deployment))
                })
            };
            by_name.or_else(by_deployment).with_context(|| {
                format!(
                    "The a4 dev session serves several stacks; choose one with --stack <name>.\n{}",
                    available(stacks)
                )
            })
        }
    }
}

fn available(stacks: &[DevStack]) -> String {
    if stacks.is_empty() {
        return "No stacks are loaded.".to_string();
    }
    let names: Vec<&str> = stacks.iter().map(|stack| stack.name.as_str()).collect();
    format!("Stacks: {}", names.join(", "))
}

pub(crate) fn no_stacks(session: &DevSession) -> String {
    format!(
        "No stack is serving in the dev environment {} yet. `a4 dev up` deploys the protocol, and Arete loads its stack into the environment; `a4 dev status` shows where that is.",
        session.environment.name
    )
}

/// Watch the listing for redeploys of the stack being streamed. `requested`
/// pins the stack by name; without it the stream follows whichever stack
/// [`select_stack`] picks, as the active protocol deployment moves.
pub fn watch(
    mut runtime: DevRuntime,
    requested: Option<String>,
    mut current: StackIdentity,
) -> mpsc::Receiver<Retarget> {
    let (retargets, receiver) = mpsc::channel(4);
    tokio::spawn(async move {
        // When a half-done redeploy was first seen.
        let mut settling_since: Option<std::time::Instant> = None;
        loop {
            tokio::select! {
                () = tokio::time::sleep(POLL_PERIOD) => {}
                () = retargets.closed() => return,
            }
            runtime.reload();
            let Ok(stacks) = runtime.stacks().await else {
                // The runtime is restarting; the stream's own reconnects
                // cover the gap.
                continue;
            };
            let settled = settling_since.is_some_and(|since| since.elapsed() >= SETTLE_LIMIT);
            let next = match next_target(
                &stacks,
                requested.as_deref(),
                &runtime.session,
                &current,
                settled,
            ) {
                Target::Move(next) => next,
                Target::Settling => {
                    settling_since.get_or_insert_with(std::time::Instant::now);
                    continue;
                }
                Target::Stay => {
                    settling_since = None;
                    continue;
                }
            };
            settling_since = None;
            let retarget = Retarget {
                refresh: runtime.refresh_for(next),
                from: current.describe(),
                to: next.identity().describe(),
            };
            current = next.identity();
            if retargets.send(retarget).await.is_err() {
                return;
            }
        }
    });
    receiver
}

#[derive(Debug, PartialEq)]
enum Target<'a> {
    /// Nothing to follow.
    Stay,
    /// A redeploy is under way; wait for it to finish.
    Settling,
    /// Move to this stack.
    Move(&'a DevStack),
}

/// What to do about the stack being streamed: move when it was redeployed
/// and its replacement is ready, once the redeploy has finished (its new
/// deployment active) or `settled` says waiting has gone on long enough.
fn next_target<'a>(
    stacks: &'a [DevStack],
    requested: Option<&str>,
    session: &DevSession,
    current: &StackIdentity,
    settled: bool,
) -> Target<'a> {
    let Ok(stack) = select_stack(stacks, requested, session) else {
        return Target::Stay;
    };
    if !stack.ready || stack.identity() == *current {
        return Target::Stay;
    }
    if stack.settling() && !settled {
        return Target::Settling;
    }
    Target::Move(stack)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(name: &str, deployment: Option<&str>, generation: i64) -> DevStack {
        DevStack {
            name: name.to_string(),
            ws_url: format!("ws://127.0.0.1:4930{generation}"),
            ready: true,
            generation: Some(generation),
            views: vec![
                "ProtocolActivity/byInstance".to_string(),
                "ProtocolActivity/list".to_string(),
                "ProtocolActivity/state".to_string(),
                "TipJar/latest".to_string(),
            ],
            deployment_id: deployment.map(str::to_string),
            short_id: deployment.map(|id| format!("{id}short")),
            protocol: deployment.map(|_| "tip-jar".to_string()),
            current: None,
            deployments: deployment
                .map(|id| DeploymentRef {
                    deployment_id: id.to_string(),
                    status: "active".to_string(),
                    ..DeploymentRef::default()
                })
                .into_iter()
                .collect(),
        }
    }

    fn session(stack: Option<&str>, deployment: Option<&str>) -> DevSession {
        let mut session = DevSession::offline(&arete_mcp::dev_env::EnvSelection::new(
            "denv_01hzx3k8m9q2r4s5t6v7w8x9yz",
            "app",
            "http://127.0.0.1:9",
        ));
        session.stack = stack.map(str::to_string);
        session.deployment_id = deployment.map(str::to_string);
        session
    }

    #[test]
    fn a_named_stack_is_chosen_or_the_names_are_listed() {
        let stacks = [stack("tip-jar", Some("d1"), 1), stack("other", None, 2)];
        let chosen = select_stack(&stacks, Some("other"), &session(None, None)).unwrap();
        assert_eq!(chosen.name, "other");

        let error = select_stack(&stacks, Some("nope"), &session(None, None))
            .unwrap_err()
            .to_string();
        assert!(error.contains("No stack named 'nope'"), "{error}");
        assert!(error.contains("tip-jar, other"), "{error}");
    }

    #[test]
    fn the_only_stack_needs_no_name() {
        let stacks = [stack("tip-jar", None, 1)];
        assert_eq!(
            select_stack(&stacks, None, &session(None, None))
                .unwrap()
                .name,
            "tip-jar"
        );
        let error = select_stack(&[], None, &session(None, None)).unwrap_err();
        assert!(error.to_string().contains("No stack is serving"), "{error}");
    }

    #[test]
    fn several_stacks_default_to_the_active_protocol_deployments() {
        let stacks = [stack("other", None, 2), stack("tip-jar", Some("d1"), 1)];
        // The session names its stack...
        let by_name = select_stack(&stacks, None, &session(Some("tip-jar"), None)).unwrap();
        assert_eq!(by_name.name, "tip-jar");
        // ...or its deployment, which the listing records on the stack.
        let by_deployment = select_stack(&stacks, None, &session(None, Some("d1"))).unwrap();
        assert_eq!(by_deployment.name, "tip-jar");
        // Otherwise it is ambiguous, with a hint.
        let error = select_stack(&stacks, None, &session(None, Some("d9")))
            .unwrap_err()
            .to_string();
        assert!(error.contains("--stack <name>"), "{error}");
        assert!(error.contains("other, tip-jar"), "{error}");
    }

    #[test]
    fn a_redeploy_is_a_new_deployment_generation_or_address() {
        let session = session(Some("tip-jar"), None);
        let before = stack("tip-jar", Some("d1"), 1);
        let current = before.identity();
        let target = |stack: &DevStack, requested: Option<&str>, settled: bool| match next_target(
            std::slice::from_ref(stack),
            requested,
            &session,
            &current,
            settled,
        ) {
            Target::Move(next) => Target::Move(next).describe_for_test(),
            other => format!("{other:?}"),
        };
        assert_eq!(target(&before, None, false), "Stay");

        let redeployed = stack("tip-jar", Some("d2"), 2);
        assert_eq!(
            target(&redeployed, None, false),
            "move to tip-jar d2short (gen 2)"
        );
        assert_eq!(current.describe(), "tip-jar d1short (gen 1)");

        // Not before the new stack is ready.
        let loading = DevStack {
            ready: false,
            ..redeployed.clone()
        };
        assert_eq!(target(&loading, None, false), "Stay");

        // The stack reloaded for d2, which is recorded but not active yet:
        // wait for the activation, unless that takes too long.
        let half_done = DevStack {
            deployment_id: Some("d1".to_string()),
            short_id: Some("d1short".to_string()),
            deployments: vec![
                DeploymentRef {
                    deployment_id: "d1".to_string(),
                    status: "active".to_string(),
                    ..DeploymentRef::default()
                },
                DeploymentRef {
                    deployment_id: "d2".to_string(),
                    status: "disabled".to_string(),
                    ..DeploymentRef::default()
                },
            ],
            ..redeployed.clone()
        };
        assert_eq!(target(&half_done, None, false), "Settling");
        assert_eq!(
            target(&half_done, None, true),
            "move to tip-jar d1short (gen 2)"
        );

        // A pinned stack is only followed under its own name.
        let renamed = stack("tip-jar-2", Some("d3"), 3);
        assert_eq!(target(&renamed, Some("tip-jar"), false), "Stay");
    }

    impl Target<'_> {
        fn describe_for_test(&self) -> String {
            match self {
                Target::Move(stack) => format!("move to {}", stack.identity().describe()),
                other => format!("{other:?}"),
            }
        }
    }

    #[test]
    fn sessions_are_minted_for_the_stack_with_the_account_key() {
        let session = session(None, Some("d1"));
        let api = arete_mcp::dev_env::EnvApi::new(
            reqwest::Client::new(),
            "http://127.0.0.1:9",
            "a4_sk_dev",
        )
        .unwrap();
        let env = arete_mcp::dev_env::EnvRpc::new(api, "denv_01hzx3k8m9q2r4s5t6v7w8x9yz").unwrap();
        let project = DevProject::new("/nonexistent");
        let runtime = DevRuntime {
            project: project.clone(),
            session: session.clone(),
            client: DevClient::new(project, session, env),
        };
        let stack = stack("tip-jar", Some("d1"), 1);
        assert_eq!(runtime.refresh_for(&stack).websocket_url(), stack.ws_url);
    }

    #[test]
    fn views_come_from_the_stack_source() {
        let source = serde_json::json!({ "stackSource": { "entities": {
            "ProtocolActivity": { "views": { "byInstance": {} } },
            "ProtocolInstance": { "views": { "byId": {}, "list": {} } },
        }}});
        assert_eq!(
            views_of_stack_source(&source),
            [
                "ProtocolActivity/list",
                "ProtocolActivity/state",
                "ProtocolActivity/byInstance",
                "ProtocolInstance/list",
                "ProtocolInstance/state",
                "ProtocolInstance/byId",
            ]
        );
        assert!(views_of_stack_source(&serde_json::Value::Null).is_empty());
    }
}
