//! `a4 dev up` and `a4 dev down`.
//!
//! `a4 dev up` creates or wakes the project's Arete cloud dev environment,
//! serves it on loopback (the local RPC endpoint, [`super::proxy`]), runs
//! the dev UI against it, and, in a protocol project, builds the protocol
//! and deploys it into the environment, redeploying when its sources
//! change. Ctrl-C (or `a4 dev down`) stops what runs locally; the
//! environment keeps its state and sleeps when idle.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use arete_mcp::dev::{
    mint_stack_session, unix_now, write_private, DevProject, DevSession, EnvRef, StackSession,
    SESSION_SCHEMA,
};
use arete_mcp::dev_env::{
    self, api_code, describe_env, env_name_from, is_env_id, is_not_found, valid_env_name,
    ws_url_of, EnvApi, EnvRpc, EnvSelection,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::api::DevApi;
use super::automations::{self, AutomationsConfig};
use super::deploy::{run_build, DeployOutcome, DeploySettings, DeployState, Deployer, BUILD_DIR};
use super::proxy::{self, RpcProxy};
use super::runtime::{pid_alive, LogSink};
use super::sdk_gen::{self, LocalProtocol};
use super::ui::{self, DevUi};
use super::watch;
use crate::project::manifest::InstallTarget;

pub const STATE_SCHEMA: &str = "arete.dev-state/v2";
const POLL: Duration = Duration::from_millis(400);
const DEBOUNCE: Duration = Duration::from_millis(350);
/// How long a new environment may take to start.
const START_TIMEOUT: Duration = Duration::from_secs(180);
/// Renew the stack session when it has less than this left (seconds).
/// The API issues five-minute sessions, so the threshold must be shorter.
const STACK_SESSION_RENEW_BEFORE: u64 = 60;
/// How often a deployment without a stack looks for it again.
const STACK_LOOKUP_EVERY: Duration = Duration::from_secs(10);

/// The ports `a4 dev up` binds, all on 127.0.0.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ports {
    /// The local RPC endpoint (JSON-RPC).
    pub rpc: u16,
    /// Its WebSocket (subscriptions): `rpc + 1` in a fixed block.
    pub ws: u16,
    /// The dev UI and its event feed (0 with `--no-ui`).
    pub ui: u16,
}

impl Ports {
    /// Fixed offsets from `base`: RPC +0, WebSocket +1, dev UI +2.
    pub fn from_base(base: u16) -> Result<Self> {
        if base == 0 || base > u16::MAX - 2 {
            bail!("--port-base must be between 1 and {}", u16::MAX - 2);
        }
        Ok(Self {
            rpc: base,
            ws: base + 1,
            ui: base + 2,
        })
    }
}

/// The listeners [`allocate_ports`] keeps bound for `a4 dev up`.
#[derive(Debug)]
pub struct Bound {
    pub rpc: TcpListener,
    pub ws: TcpListener,
    /// The dev UI's listener, unless `--no-ui`.
    pub ui: Option<TcpListener>,
    /// The fixed block's UI port, when it was busy and the UI moved to a
    /// free port instead.
    pub ui_busy: Option<u16>,
}

fn loopback(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

/// Pick the ports: fixed offsets from `base`, or free ports with `base ==
/// 0` (the WebSocket then takes the RPC port + 1 when it is free). A busy UI
/// port in a fixed block is not an error: the UI takes a free port instead.
pub fn allocate_ports(base: u16, with_ui: bool) -> Result<(Ports, Bound)> {
    let (rpc, ws) = if base == 0 {
        let mut pair = None;
        for _ in 0..20 {
            let rpc = TcpListener::bind(loopback(0))?;
            let port = rpc.local_addr()?.port();
            if let Some(ws) = port
                .checked_add(1)
                .and_then(|next| TcpListener::bind(loopback(next)).ok())
            {
                pair = Some((rpc, ws));
                break;
            }
        }
        match pair {
            Some(pair) => pair,
            None => (
                TcpListener::bind(loopback(0))?,
                TcpListener::bind(loopback(0))?,
            ),
        }
    } else {
        let ports = Ports::from_base(base)?;
        let bind = |port: u16, name: &str| {
            TcpListener::bind(loopback(port)).with_context(|| {
                format!("Port {port} ({name}) is in use; pass --port-base <N> to use another block")
            })
        };
        (
            bind(ports.rpc, "local RPC")?,
            bind(ports.ws, "local RPC WebSocket")?,
        )
    };
    let mut ports = Ports {
        rpc: rpc.local_addr()?.port(),
        ws: ws.local_addr()?.port(),
        ui: 0,
    };
    let (ui, ui_busy) = if !with_ui {
        (None, None)
    } else if base == 0 {
        let listener = TcpListener::bind(loopback(0))?;
        ports.ui = listener.local_addr()?.port();
        (Some(listener), None)
    } else {
        let wanted = base + 2;
        match TcpListener::bind(loopback(wanted)) {
            Ok(listener) => {
                ports.ui = wanted;
                (Some(listener), None)
            }
            Err(_) => {
                let listener = TcpListener::bind(loopback(0)).with_context(|| {
                    format!("Port {wanted} (dev ui) is in use and no other port is free; pass --no-ui or --port-base <N>")
                })?;
                ports.ui = listener.local_addr()?.port();
                (Some(listener), Some(wanted))
            }
        }
    };
    Ok((
        ports,
        Bound {
            rpc,
            ws,
            ui,
            ui_busy,
        },
    ))
}

/// `.arete/dev/state.json`: the running `a4 dev up`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevState {
    pub schema: String,
    pub pid: u32,
    pub started_at: u64,
    pub environment: String,
    pub ports: Ports,
}

pub fn state_path(project: &DevProject) -> PathBuf {
    project.dev_dir().join("state.json")
}

pub fn load_state(project: &DevProject) -> Option<DevState> {
    std::fs::read(state_path(project))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

pub struct UpConfig {
    pub project_root: PathBuf,
    /// Whether the project has a protocol (`protocol.ts`) to build, deploy
    /// and watch.
    pub protocol: bool,
    /// The account's API with its key.
    pub api: EnvApi,
    /// `--env`: the environment to use (a name or an id).
    pub environment: Option<String>,
    /// `--template`: what a new environment starts from.
    pub template: Option<String>,
    pub settings: DeploySettings,
    pub port_base: u16,
    pub watch: bool,
    /// Deploy the protocol into the environment (not with `--no-deploy`).
    pub deploy: bool,
    /// Generate this protocol's installed client from each new build.
    pub sync_sdk: bool,
    /// `[dev] sdk` in arete.toml: the targets of the client added when the
    /// project does not install its protocol yet.
    pub sdk_targets: Vec<InstallTarget>,
    pub json: bool,
    /// Serve the dev UI and its event feed (not with `--no-ui`).
    pub ui: bool,
    /// Open the dev UI in the browser once the session is ready.
    pub open_ui: bool,
    /// A Vite dev server to proxy the UI's files to (UI development).
    pub ui_dev_url: Option<String>,
}

/// Prints `a4 dev up` progress: JSON event lines on stdout with `--json`,
/// readable lines otherwise. Everything also goes to `a4.log`.
#[derive(Clone)]
struct Events {
    json: bool,
    log: LogSink,
}

impl Events {
    fn emit(&self, event: &str, human: &str, mut fields: Value) {
        self.log
            .line(&redact_hs_tokens(&format!("{event}: {human}")));
        if self.json {
            if let Some(object) = fields.as_object_mut() {
                object.insert("event".into(), json!(event));
            }
            println!("{fields}");
        } else {
            println!("{human}");
        }
    }
}

/// `text` with every `hs_token=<token>` value replaced, for the log file.
pub fn redact_hs_tokens(text: &str) -> String {
    const MARKER: &str = "hs_token=";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find(MARKER) {
        let (head, tail) = rest.split_at(index + MARKER.len());
        out.push_str(head);
        out.push_str("<redacted>");
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '%')))
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// How the environment came to be running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvOutcome {
    Created,
    Woken,
    Running,
}

impl EnvOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Woken => "woken",
            Self::Running => "running",
        }
    }
}

/// The name a project's environment gets: `[protocol].slug`, else the
/// project directory's name.
pub fn default_env_name(project_root: &Path) -> String {
    let slug = crate::commands::protocol::load_protocol_config(project_root)
        .ok()
        .and_then(|config| config.slug);
    match slug {
        Some(slug) => env_name_from(&slug),
        None => env_name_from(
            project_root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("dev"),
        ),
    }
}

/// Find (or create) the environment and make sure it is running.
pub async fn ensure_environment(
    api: &EnvApi,
    project: &DevProject,
    requested: Option<&str>,
    template: Option<&str>,
) -> Result<(Value, EnvOutcome)> {
    let remembered = project
        .load_selection()?
        .filter(|selection| dev_env::api_origin(&selection.api_url) == api.origin());
    let (existing, name) = match requested {
        Some(reference) => (api.find(reference).await?, reference.to_string()),
        None => match &remembered {
            Some(selection) => match api.get_env(&selection.id).await {
                Ok(env) if env["state"] != "deleted" => (Some(env), selection.name.clone()),
                Ok(_) => (None, selection.name.clone()),
                Err(error) if is_not_found(&error) => (None, selection.name.clone()),
                Err(error) => return Err(error),
            },
            None => {
                let name = default_env_name(&project.root);
                (api.find(&name).await?, name)
            }
        },
    };
    let (env, outcome) = match existing {
        Some(env) => {
            let outcome = if env["state"] == "running" {
                EnvOutcome::Running
            } else {
                EnvOutcome::Woken
            };
            (env, outcome)
        }
        None => {
            if is_env_id(&name) {
                bail!("You have no dev environment {name}; `a4 dev list` shows yours");
            }
            if !valid_env_name(&name) {
                bail!("'{name}' is not an environment name (1-63 lowercase letters, digits or '-'); pass --env <name>");
            }
            let env = api
                .create(&name, template)
                .await
                .map_err(super::env::explain)?;
            (env, EnvOutcome::Created)
        }
    };
    let id = env["id"]
        .as_str()
        .context("The Arete API returned an environment without an id")?
        .to_string();
    let deadline = std::time::Instant::now() + START_TIMEOUT;
    let mut env = env;
    loop {
        match env["state"].as_str() {
            Some("running") if env["busy"].is_null() => break,
            Some("sleeping") => {
                env = api.wake(&id).await.map_err(super::env::explain)?;
                continue;
            }
            Some("deleted") => bail!("The dev environment {id} was deleted"),
            _ => {}
        }
        if std::time::Instant::now() >= deadline {
            bail!(
                "The dev environment {id} is still {} after {}s; try `a4 dev up` again",
                env["state"].as_str().unwrap_or("starting"),
                START_TIMEOUT.as_secs()
            );
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        env = match api.get_env(&id).await {
            Ok(env) => env,
            Err(error) if api_code(&error) == Some("devenv-busy") => continue,
            Err(error) => return Err(error),
        };
    }
    Ok((env, outcome))
}

struct Session<'a> {
    config: &'a UpConfig,
    project: DevProject,
    events: Events,
    env: EnvRef,
    environment_rpc_url: String,
    proxy: Option<RpcProxy>,
    ports: Ports,
    deployer: Option<Deployer>,
    deploy_state: DeployState,
    stack: Option<(String, String)>,
    stack_session: Option<StackSession>,
    /// No stack session mint before this (Unix seconds) after a failure.
    stack_session_retry_at: u64,
    stack_looked_up: Option<std::time::Instant>,
    /// The deployment and release the installed client was last generated
    /// for.
    sdk_generated_for: Option<(String, String)>,
    ui: DevUi,
    /// `[dev.automations]`: the project's automations, restarted on changes.
    automations: Option<automations::Runner>,
}

/// Run `a4 dev up` until Ctrl-C or SIGTERM (`a4 dev down`).
pub async fn run(config: UpConfig) -> Result<()> {
    let project = DevProject::new(&config.project_root);
    std::fs::create_dir_all(project.dev_dir())?;
    if let Some(state) = load_state(&project) {
        if state.pid != std::process::id() && pid_alive(state.pid) {
            bail!(
                "a4 dev is already running for this project (pid {}). Stop it with `a4 dev down`.",
                state.pid
            );
        }
    }
    let events = Events {
        json: config.json,
        log: LogSink::open(&project.dev_dir().join("a4.log"), "a4", false)?,
    };
    let automations_config = AutomationsConfig::load(&config.project_root)?;

    // 1. The environment.
    let (view, outcome) = ensure_environment(
        &config.api,
        &project,
        config.environment.as_deref(),
        config.template.as_deref(),
    )
    .await?;
    let env = EnvRef {
        id: view["id"].as_str().unwrap_or_default().to_string(),
        name: view["name"].as_str().unwrap_or_default().to_string(),
    };
    EnvSelection::new(&env.id, &env.name, config.api.origin()).save(&project.environment_path())?;
    events.emit(
        "environment",
        &format!(
            "Dev environment {} ({})",
            describe_env(&view),
            outcome.as_str()
        ),
        json!({ "environment": dev_env::public_view(&view), "outcome": outcome.as_str() }),
    );
    let rpc = Arc::new(EnvRpc::new(config.api.clone(), &env.id)?);
    let token = rpc.token(false).await?;

    // 2. The local RPC endpoint and the dev UI.
    let (ports, bound) = allocate_ports(config.port_base, config.ui)?;
    let proxy = proxy::start(bound.rpc, bound.ws, rpc.clone(), Some(events.log.clone())).await?;
    let ui_options = ui::UiOptions {
        project: project.clone(),
        ui_dev_url: config.ui_dev_url.clone(),
        api_key: Some(config.api.key().to_string()),
    };
    let ui = match bound.ui {
        Some(listener) => ui::start(listener, ui_options)?,
        None => ui::disabled(ui_options),
    };
    if let (Some(busy), Some(url)) = (bound.ui_busy, ui.url.as_deref()) {
        events.emit(
            "ui-warning",
            &format!("Port {busy} (dev UI) is in use; serving the dev UI at {url} instead (--no-ui skips it)"),
            json!({ "busyPort": busy, "uiUrl": url }),
        );
    }
    ui.set_urls(json!({
        "api": format!("{}/api", config.api.origin()),
        "rpc": proxy.rpc_url(),
        "rpcWs": proxy.ws_url(),
        "environmentRpc": token.rpc_url,
    }));
    ui.set_environment(json!({ "id": env.id, "name": env.name }));
    ui.start_feeds(rpc.clone());

    let deploy_state = DeployState::load_for(&project.deploy_state_path(), &env.id);
    let automation_events = events.clone();
    let mut session = Session {
        config: &config,
        project: project.clone(),
        events,
        environment_rpc_url: token.rpc_url.clone(),
        env,
        ports: Ports {
            rpc: proxy.http_addr.port(),
            ws: proxy.ws_addr.port(),
            ui: ports.ui,
        },
        proxy: Some(proxy),
        deployer: None,
        deploy_state,
        stack: None,
        stack_session: None,
        stack_session_retry_at: 0,
        stack_looked_up: None,
        sdk_generated_for: None,
        automations: automations_config.map(|automations_config| {
            let events = automation_events;
            automations::Runner::new(
                automations_config,
                &config.project_root,
                config.api.key(),
                Arc::new(move |event: &str, human: &str, fields: Value| {
                    events.emit(event, human, fields)
                }),
            )
        }),
        ui,
    };
    let result = match session.start(&token).await {
        Ok(()) => session.supervise().await,
        Err(error) => Err(error),
    };
    if let Some(runner) = &mut session.automations {
        runner.stop().await;
    }
    let Session {
        events, ui, proxy, ..
    } = session;
    ui.shutdown();
    if let Some(proxy) = proxy {
        proxy.stop().await;
    }
    let _ = std::fs::remove_file(state_path(&project));
    let _ = std::fs::remove_file(project.session_path());
    match &result {
        Ok(()) => events.emit(
            "stopped",
            "a4 dev stopped. The environment keeps its state and sleeps when idle (`a4 dev sleep` now, `a4 dev status` to look).",
            json!({}),
        ),
        Err(error) => events.emit(
            "failed",
            &format!("a4 dev stopped: {error:#}"),
            json!({ "error": format!("{error:#}") }),
        ),
    }
    result
}

impl Session<'_> {
    fn write_state(&self) -> Result<()> {
        let state = DevState {
            schema: STATE_SCHEMA.into(),
            pid: std::process::id(),
            started_at: unix_now(),
            environment: self.env.id.clone(),
            ports: self.ports,
        };
        write_private(
            &state_path(&self.project),
            &serde_json::to_vec_pretty(&state)?,
        )
    }

    fn rpc_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.ports.rpc)
    }

    fn rpc_ws_url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.ports.ws)
    }

    fn session(&self) -> DevSession {
        let api_url = self.config.api.origin().to_string();
        let state = &self.deploy_state;
        DevSession {
            schema: SESSION_SCHEMA.into(),
            environment: self.env.clone(),
            session_endpoint: format!("{api_url}/ws/sessions"),
            api_url,
            rpc_url: self.rpc_url(),
            rpc_ws_url: self.rpc_ws_url(),
            environment_rpc_url: self.environment_rpc_url.clone(),
            protocol_id: state.protocol_id.clone(),
            protocol_slug: state.slug.clone(),
            release_hash: state.release_hash.clone(),
            deployment_id: state.deployment_id.clone(),
            short_id: state.short_id.clone(),
            stack: self.stack.as_ref().map(|(name, _)| name.clone()),
            stack_ws_url: self.stack.as_ref().map(|(_, url)| url.clone()),
            stack_session: self.stack_session.clone(),
            ui_url: self.ui.url.clone(),
            updated_at: unix_now(),
        }
    }

    fn write_session(&self) -> Result<DevSession> {
        let session = self.session();
        write_private(
            &self.project.session_path(),
            &serde_json::to_vec_pretty(&session)?,
        )?;
        Ok(session)
    }

    async fn start(&mut self, token: &dev_env::EnvToken) -> Result<()> {
        let config = self.config;
        self.write_state()?;
        self.write_session()?;

        if config.protocol && config.deploy {
            self.build(&[]).await?;
            self.deployer = Some(Deployer {
                project_root: config.project_root.clone(),
                api: DevApi::new(config.api.origin(), config.api.key()),
                environment: self.env.id.clone(),
                settings: config.settings.clone(),
                state_path: self.project.deploy_state_path(),
                fee_dir: self.project.dev_dir().join("fee"),
                log: self.events.log.clone(),
            });
            let outcome = self.deploy().await;
            let outcome = match outcome {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.ui.deploy_failed(&error);
                    return Err(error);
                }
            };
            self.after_deploy(&outcome).await?;
        } else if config.protocol {
            self.build(&[]).await?;
            // --no-deploy: show the deployment an earlier `a4 dev up` left in
            // the environment.
            if let Some(deployment) = self.deploy_state.deployment_id.clone() {
                self.ui.deployed(
                    json!({
                        "deploymentId": deployment,
                        "releaseHash": self.deploy_state.release_hash,
                        "protocolId": self.deploy_state.protocol_id,
                        "slug": self.deploy_state.slug,
                        "shortId": self.deploy_state.short_id,
                        "environment": self.env.id,
                    }),
                    false,
                );
                self.find_stack().await;
                self.refresh_stack_session(true).await;
                self.write_session()?;
            }
        }

        if config.deploy {
            self.deploy_project_stacks().await?;
        } else {
            self.find_stack().await;
            self.refresh_stack_session(true).await;
            self.write_session()?;
        }
        self.ui.start_stack_rows();

        self.events.emit(
            "ready",
            &self.ready_summary(token),
            json!({
                "session": self.session().redacted(),
                "sessionFile": self.project.session_path().display().to_string(),
                "uiUrl": self.ui.url,
                "environmentRpcUrl": token.rpc_url,
                "environmentWsUrl": ws_url_of(&token.rpc_url),
            }),
        );
        if !config.json {
            // The token goes to the terminal only, never to a4.log.
            println!(
                "  Token       {}\n              (for the environment RPC above as `Authorization: Bearer`; expires at {}; `a4 dev token` mints another)",
                token.token, token.expires_at
            );
        }
        if let (true, Some(url)) = (config.open_ui, self.ui.url.as_deref()) {
            if let Err(error) = ui::open_browser(url) {
                self.events.emit(
                    "ui-warning",
                    &format!("Could not open the browser: {error:#}; open {url} yourself"),
                    json!({ "error": format!("{error:#}") }),
                );
            }
        }
        Ok(())
    }

    /// `arete-protocol build`, reported to the dev UI.
    async fn build(&mut self, changed: &[String]) -> Result<()> {
        self.ui.build_started(changed);
        match run_build(&self.config.project_root, &self.events.log).await {
            Ok(()) => {
                self.ui.build_succeeded(&self.project.root.join(BUILD_DIR));
                Ok(())
            }
            Err(error) => {
                self.ui.build_failed(&error);
                Err(error)
            }
        }
    }

    async fn deploy(&mut self) -> Result<DeployOutcome> {
        let mut state = std::mem::take(&mut self.deploy_state);
        let outcome = self
            .deployer
            .as_ref()
            .context("nothing deploys with --no-deploy")?
            .deploy(&mut state)
            .await;
        self.deploy_state = state;
        outcome
    }

    fn ready_summary(&self, token: &dev_env::EnvToken) -> String {
        let state = &self.deploy_state;
        let mut lines = vec![format!(
            "a4 dev is ready: dev environment {} ({})",
            self.env.name, self.env.id
        )];
        lines.push(format!(
            "  RPC         {}   (wallets, apps, the Solana CLI: no token needed)",
            self.rpc_url()
        ));
        lines.push(format!("  WebSocket   {}", self.rpc_ws_url()));
        lines.push(format!("  Env RPC     {}", token.rpc_url));
        lines.push(format!("  Env WS      {}", ws_url_of(&token.rpc_url)));
        if let Some(url) = &self.ui.url {
            lines.push(format!("  UI          {url}"));
        }
        if let Some(deployment) = &state.deployment_id {
            lines.push(format!("  Deployment  {deployment}"));
        }
        if let Some((name, url)) = &self.stack {
            lines.push(format!("  Stack       {name} {url}"));
            lines.push(
                "  Stream      a4 stream --dev            (lists the stack's views; --tui to browse them)"
                    .into(),
            );
        }
        lines.push(format!(
            "  Session     {}",
            self.project.session_path().display()
        ));
        let mut edits = Vec::new();
        if self.config.protocol && self.config.deploy && self.config.watch {
            edits.push("Edit protocol.ts, handlers/ or idl/ to redeploy.");
        }
        edits.push("Ctrl-C (or `a4 dev down`) stops the local endpoints; the environment keeps its state and sleeps when idle.");
        lines.push(edits.join(" "));
        lines.join("\n")
    }

    /// Look up the stack Arete runs for the deployment in the environment,
    /// when there is none yet. Returns whether it changed.
    async fn find_stack(&mut self) -> bool {
        if self.stack.is_some() {
            return false;
        }
        if self
            .stack_looked_up
            .is_some_and(|at| at.elapsed() < STACK_LOOKUP_EVERY)
        {
            return false;
        }
        self.stack_looked_up = Some(std::time::Instant::now());
        let api = DevApi::new(self.config.api.origin(), self.config.api.key());
        let found = if let Some(deployment) = self.deploy_state.deployment_id.as_deref() {
            api.protocol_env_stack(&self.env.id, deployment).await
        } else {
            let metadata: Value = std::fs::read(self.project.dev_dir().join("stacks.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or(json!([]));
            api.env_stacks(&self.env.id).await.map(|entries| {
                entries
                    .into_iter()
                    .filter(|entry| {
                        metadata
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|stack| stack["name"] == entry["specName"])
                    })
                    .find_map(|entry| super::api::env_stack(&entry))
            })
        };
        match found {
            Ok(Some(stack)) => {
                self.events.emit(
                    "stack",
                    &format!("Stack {} is serving at {}", stack.name, stack.ws_url),
                    json!({ "stack": stack.name, "wsUrl": stack.ws_url }),
                );
                self.stack = Some((stack.name, stack.ws_url));
                true
            }
            Ok(None) => false,
            Err(error) => {
                self.events.log.line(&format!("stack lookup: {error:#}"));
                false
            }
        }
    }

    /// Mint (or renew) the stack session when there is a stack and the
    /// current one is missing or about to expire. Returns whether it
    /// changed.
    async fn refresh_stack_session(&mut self, force: bool) -> bool {
        let Some((_, ws_url)) = self.stack.clone() else {
            let changed = self.stack_session.is_some();
            self.stack_session = None;
            return changed;
        };
        if !force {
            if unix_now() < self.stack_session_retry_at {
                return false;
            }
            if let Some(current) = &self.stack_session {
                if current.expires_at > unix_now() + STACK_SESSION_RENEW_BEFORE {
                    return false;
                }
            }
        }
        let endpoint = format!("{}/ws/sessions", self.config.api.origin());
        match mint_stack_session(
            self.config.api.http(),
            &endpoint,
            self.config.api.key(),
            &ws_url,
            self.deploy_state.deployment_id.as_deref(),
        )
        .await
        {
            Ok(session) => {
                self.events.log.line(&format!(
                    "stack session minted, expires at {}",
                    session.expires_at
                ));
                self.stack_session = Some(session);
                true
            }
            Err(error) => {
                self.stack_session_retry_at = unix_now() + 30;
                self.events.emit(
                    "stack-warning",
                    &format!("Could not mint a stack session: {error:#}"),
                    json!({ "error": format!("{error:#}") }),
                );
                false
            }
        }
    }

    async fn after_deploy(&mut self, outcome: &DeployOutcome) -> Result<()> {
        if outcome.changed {
            // A new deployment gets a new stack.
            self.stack = None;
            self.stack_session = None;
            self.stack_looked_up = None;
        }
        self.find_stack().await;
        self.refresh_stack_session(outcome.changed || self.stack_session.is_none())
            .await;
        self.write_session()?;
        let generated_for = (outcome.deployment_id.clone(), outcome.release_hash.clone());
        if self.config.sync_sdk && self.sdk_generated_for.as_ref() != Some(&generated_for) {
            self.sync_sdk(outcome);
        }
        if !outcome.disabled.is_empty() {
            self.events.emit(
                "disabled",
                &format!(
                    "Disabled the earlier deployment(s) {}",
                    outcome.disabled.join(", ")
                ),
                json!({ "deploymentIds": outcome.disabled }),
            );
        }
        for (deployment, error) in &outcome.disable_errors {
            self.events.emit(
                "disable-warning",
                &format!("Could not disable the earlier deployment {deployment}: {error}"),
                json!({ "deploymentId": deployment, "error": error }),
            );
        }
        self.ui.deployed(
            json!({
                "deploymentId": outcome.deployment_id,
                "releaseHash": outcome.release_hash,
                "protocolId": self.deploy_state.protocol_id,
                "slug": self.deploy_state.slug,
                "shortId": self.deploy_state.short_id,
                "environment": self.env.id,
                "stack": self.stack.as_ref().map(|(name, _)| name),
                "disabled": outcome.disabled,
            }),
            outcome.changed,
        );
        if outcome.changed {
            self.events.emit(
                "deployed",
                &format!(
                    "Deployed release {} as {} in {}",
                    outcome.release_hash, outcome.deployment_id, self.env.name
                ),
                json!({
                    "releaseHash": outcome.release_hash,
                    "deploymentId": outcome.deployment_id,
                    "environment": self.env.id,
                }),
            );
        }
        Ok(())
    }

    /// Generate the installed client from the build just deployed.
    fn sync_sdk(&mut self, outcome: &DeployOutcome) {
        let synced = (|| {
            let build = super::deploy::read_build(&self.project.root.join(BUILD_DIR))?;
            let protocol = LocalProtocol {
                spec_json: build.spec_json,
                slug: self
                    .deploy_state
                    .slug
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("the protocol has no slug yet"))?,
                release_hash: outcome.release_hash.clone(),
                deployment_id: outcome.deployment_id.clone(),
            };
            sdk_gen::sync_installed_client(
                &self.project.root,
                &protocol,
                &self.config.sdk_targets,
                self.config.api.origin(),
            )
        })();
        match synced {
            Ok(report) => {
                self.sdk_generated_for =
                    Some((outcome.deployment_id.clone(), outcome.release_hash.clone()));
                if report.added_dependency {
                    self.events.emit(
                        "sdk-dependency",
                        &format!(
                            "Added [dependencies.protocols.{}] to arete.toml (this protocol's own client)",
                            report.install.alias
                        ),
                        json!({ "alias": report.install.alias }),
                    );
                }
                if report.install.changed {
                    self.events.emit(
                        "sdk-generated",
                        &report.summary(),
                        serde_json::to_value(&report).unwrap_or_default(),
                    );
                }
            }
            Err(error) => self.events.emit(
                "sdk-warning",
                &format!("Could not generate the protocol client from the local build: {error:#}"),
                json!({ "error": format!("{error:#}") }),
            ),
        }
    }

    async fn redeploy(&mut self, changed: &[String]) -> Result<()> {
        self.build(changed).await?;
        if self.deployer.is_none() {
            return Ok(());
        }
        let outcome = match self.deploy().await {
            Ok(outcome) => outcome,
            Err(error) => {
                self.ui.deploy_failed(&error);
                return Err(error);
            }
        };
        if !outcome.changed {
            self.events.emit(
                "unchanged",
                "Rebuilt; the release is unchanged",
                json!({ "releaseHash": outcome.release_hash }),
            );
        }
        self.after_deploy(&outcome).await
    }

    async fn deploy_project_stacks(&mut self) -> Result<()> {
        let stacks = super::custom_stacks::deploy(
            &self.project.root,
            &self.config.api,
            &self.env.id,
            &self.events.log,
        )
        .await?;
        for stack in &stacks {
            self.events.emit("stack", &format!("Stack {} is serving at {}", stack.name, stack.ws_url),
                json!({ "stack": stack.name, "wsUrl": stack.ws_url, "deploymentId": stack.deployment_id }));
        }
        if !self.config.protocol {
            self.stack = stacks
                .first()
                .map(|stack| (stack.name.clone(), stack.ws_url.clone()));
            self.refresh_stack_session(true).await;
        }
        self.write_session()?;
        Ok(())
    }

    async fn supervise(&mut self) -> Result<()> {
        // One Ctrl-C listener and one SIGTERM stream for the whole session:
        // a signal that arrives while a pass is debouncing or building is
        // kept, and interrupts that pass, instead of being lost between two
        // listeners.
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        #[cfg(unix)]
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        #[cfg(not(unix))]
        let mut terminate = ();
        let watching = self.config.watch;
        let mut snapshot = if watching {
            watch::snapshot(&self.project.root)
        } else {
            watch::Snapshot::new()
        };
        let mut tick = tokio::time::interval(POLL);
        if let Some(runner) = &mut self.automations {
            runner.start_or_report();
        }
        loop {
            tokio::select! {
                _ = &mut ctrl_c => return Ok(()),
                _ = terminated(&mut terminate) => return Ok(()),
                _ = tick.tick() => {}
            }
            if let Some(runner) = &mut self.automations {
                runner.poll(watching).await;
            }
            let found = self.find_stack().await;
            if self.refresh_stack_session(found).await || found {
                self.write_session()?;
            }
            if !watching {
                continue;
            }
            tokio::select! {
                _ = &mut ctrl_c => return Ok(()),
                _ = terminated(&mut terminate) => return Ok(()),
                result = self.watch_pass(&mut snapshot) => result?,
            }
        }
    }

    /// Rebuild when the protocol's sources changed since `snapshot`, then
    /// move `snapshot` on.
    async fn watch_pass(&mut self, snapshot: &mut watch::Snapshot) -> Result<()> {
        let now = watch::snapshot(&self.project.root);
        if watch::changed_paths(snapshot, &now).is_empty() {
            return Ok(());
        }
        // Debounce: wait until the files stop changing.
        let mut settled = now;
        loop {
            tokio::time::sleep(DEBOUNCE).await;
            let again = watch::snapshot(&self.project.root);
            if again == settled {
                break;
            }
            settled = again;
        }
        let changed = watch::changed_paths(snapshot, &settled);
        *snapshot = settled;
        let names: Vec<String> = changed
            .iter()
            .map(|path| {
                path.strip_prefix(&self.project.root)
                    .unwrap_or(path)
                    .display()
                    .to_string()
            })
            .collect();
        self.events.emit(
            "rebuilding",
            &format!("Changed: {}; rebuilding", names.join(", ")),
            json!({ "changed": names }),
        );
        let result = async {
            if self.config.protocol {
                self.redeploy(&names).await?;
            }
            if self.config.deploy {
                self.deploy_project_stacks().await?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            self.events.emit(
                "deploy-failed",
                &format!("Redeploy failed: {error:#}"),
                json!({ "error": format!("{error:#}") }),
            );
        }
        // The client sync writes arete.lock (`local_build`); that is not a
        // change to redeploy for.
        watch::refresh(snapshot, &self.project.root, "arete.lock");
        Ok(())
    }
}

/// `a4 dev down`: stop a running `a4 dev up` (it cleans up after itself).
/// `purge` deletes `.arete/dev` (the project forgets its environment, which
/// keeps running in the cloud; `a4 dev delete` deletes it).
pub fn down(project_root: &Path, purge: bool) -> Result<Value> {
    down_with_stop(project_root, purge, |pid| {
        super::runtime::stop_process(pid, Duration::from_secs(30))
    })
}

fn down_with_stop(
    project_root: &Path,
    purge: bool,
    mut stop: impl FnMut(u32) -> Result<()>,
) -> Result<Value> {
    let project = DevProject::new(project_root);
    let mut stopped = Vec::new();
    if let Some(state) = load_state(&project) {
        stop(state.pid)?;
        stopped.push(json!({ "name": "a4 dev up", "pid": state.pid }));
        let _ = std::fs::remove_file(state_path(&project));
    }
    let _ = std::fs::remove_file(project.session_path());
    if purge && project.dev_dir().is_dir() {
        std::fs::remove_dir_all(project.dev_dir())
            .with_context(|| format!("Failed to delete {}", project.dev_dir().display()))?;
    }
    Ok(json!({ "stopped": stopped, "purged": purge }))
}

/// Resolves when SIGTERM arrives (never off Unix).
#[cfg(unix)]
async fn terminated(terminate: &mut tokio::signal::unix::Signal) {
    terminate.recv().await;
}

/// Resolves when SIGTERM arrives (never off Unix).
#[cfg(not(unix))]
async fn terminated(_: &mut ()) {
    std::future::pending::<()>().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_are_fixed_offsets_from_the_base() {
        let ports = Ports::from_base(48_700).unwrap();
        assert_eq!((ports.rpc, ports.ws, ports.ui), (48_700, 48_701, 48_702));
        assert!(Ports::from_base(0).is_err());
        assert!(Ports::from_base(u16::MAX).is_err());
    }

    #[test]
    fn allocates_ephemeral_ports_and_refuses_busy_blocks() {
        let (ports, bound) = allocate_ports(0, true).unwrap();
        assert_eq!(bound.rpc.local_addr().unwrap().port(), ports.rpc);
        assert_eq!(bound.ws.local_addr().unwrap().port(), ports.ws);
        assert_eq!(bound.ui.unwrap().local_addr().unwrap().port(), ports.ui);
        let distinct: std::collections::BTreeSet<u16> =
            [ports.rpc, ports.ws, ports.ui].into_iter().collect();
        assert_eq!(distinct.len(), 3);

        // A busy port in a fixed block is an error naming --port-base.
        let busy = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = busy.local_addr().unwrap().port();
        let error = allocate_ports(base, true).unwrap_err().to_string();
        assert!(error.contains("--port-base"), "{error}");

        let (ports, bound) = allocate_ports(0, false).unwrap();
        assert!(bound.ui.is_none());
        assert_eq!(ports.ui, 0);
    }

    #[test]
    fn a_busy_ui_port_moves_the_ui() {
        for _ in 0..20 {
            let busy = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = busy.local_addr().unwrap().port();
            let Some(base) = port.checked_sub(2).filter(|base| *base > 0) else {
                continue;
            };
            let Ok((ports, bound)) = allocate_ports(base, true) else {
                continue;
            };
            assert_eq!(bound.ui_busy, Some(port));
            assert_ne!(ports.ui, port);
            assert_eq!(ports.rpc, base);
            return;
        }
        panic!("no free port block next to a busy port");
    }

    #[test]
    fn redacts_stack_tokens_for_the_log() {
        assert_eq!(
            redact_hs_tokens("ready: --url 'ws://127.0.0.1:9/?hs_token=eyJ.a-b_c.d' done"),
            "ready: --url 'ws://127.0.0.1:9/?hs_token=<redacted>' done"
        );
        assert_eq!(redact_hs_tokens("no token"), "no token");
    }

    #[test]
    fn environment_names_default_to_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("My Tip_Jar");
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(default_env_name(&root), "my-tip-jar");
        std::fs::write(
            root.join("arete.toml"),
            "[project]\nname = \"x\"\n\n[protocol]\nslug = \"tip-jar\"\n",
        )
        .unwrap();
        assert_eq!(default_env_name(&root), "tip-jar");
    }

    #[test]
    fn shutdown_failure_preserves_session_and_state_even_with_purge() {
        let dir = tempfile::tempdir().unwrap();
        let project = DevProject::new(dir.path());
        let state = DevState {
            schema: STATE_SCHEMA.into(),
            pid: 12345,
            started_at: 0,
            environment: "denv_test".into(),
            ports: Ports {
                rpc: 1,
                ws: 2,
                ui: 3,
            },
        };
        write_private(&state_path(&project), &serde_json::to_vec(&state).unwrap()).unwrap();
        write_private(&project.session_path(), b"saved session").unwrap();
        let result = down_with_stop(dir.path(), true, |_| bail!("shutdown denied"));
        assert!(result.is_err());
        assert_eq!(load_state(&project), Some(state));
        assert_eq!(
            std::fs::read(project.session_path()).unwrap(),
            b"saved session"
        );
    }

    #[test]
    fn down_without_a_session_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let report = down(dir.path(), true).unwrap();
        assert_eq!(report["stopped"], json!([]));
    }
}
