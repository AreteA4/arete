//! The project's automations under `a4 dev up`.
//!
//! `[dev.automations]` in arete.toml names the module that exports the
//! project's `defineAutomations(...)` (`entry`), or the app's own webhook
//! route to forward events to (`forward`):
//!
//! ```toml
//! [dev.automations]
//! entry = "automations/index.ts"     # or: forward = "http://localhost:3000/api/arete"
//! watch = ["lib"]                    # optional: more paths that restart it
//! ```
//!
//! `a4 dev up` runs `arete-automations dev` (from `@usearete/automations`,
//! installed in the project) with `node` as a child process, with the
//! session's settings in its environment, prints its output, and restarts it
//! when the entry's directory, a `watch` path or the generated protocol
//! client changes. The runner keeps its state under `.arete/dev/automations/`,
//! so jobs, consent and spending survive restarts.
//!
//! Dev environments deliver no webhooks, so the runner follows the
//! environment's instances and turns each emission into a delivery signed
//! with this project's dev webhook secret (`.arete/dev/automations/
//! webhook-secret`), exactly as a production webhook is signed. `a4 dev
//! consent` signs a consent grant with a dev wallet through the runner's
//! loopback control port.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use super::watch::{self, Snapshot};

/// The runner inside `@usearete/automations`, run with `node` directly so
/// SIGTERM reaches it (and it saves its state) rather than a wrapper.
const RUNNER_SCRIPT: [&str; 4] = ["node_modules", "@usearete", "automations", "dist/cli.js"];
/// How long a stopping runner gets to save its state before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// `[dev.automations]` in arete.toml.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationsConfig {
    /// The module exporting the definition, relative to the project root.
    pub entry: Option<String>,
    /// The app's webhook route, to forward signed deliveries to instead.
    pub forward: Option<String>,
    /// More files or directories whose changes restart the runner.
    #[serde(default)]
    pub watch: Vec<String>,
    /// The runner's command (default `node <@usearete/automations>/dist/cli.js dev`,
    /// found in the nearest `node_modules`).
    pub command: Option<Vec<String>>,
}

impl AutomationsConfig {
    /// `[dev.automations]` from `<root>/arete.toml`, when it is set.
    pub fn load(root: &Path) -> Result<Option<Self>> {
        let path = root.join("arete.toml");
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to read {}", path.display()))
            }
        };
        Self::parse(&source, root).map_err(|error| anyhow!("Invalid {}: {error:#}", path.display()))
    }

    fn parse(source: &str, root: &Path) -> Result<Option<Self>> {
        let value: toml::Value = toml::from_str(source).context("Failed to parse TOML")?;
        let Some(table) = value.get("dev").and_then(|dev| dev.get("automations")) else {
            return Ok(None);
        };
        let config: Self = table.clone().try_into().context(
            "Invalid [dev.automations] (keys: entry = \"<module>\" or forward = \"<url>\", watch = [\"<path>\"], command = [\"<program>\", ...])",
        )?;
        match (&config.entry, &config.forward) {
            (Some(_), Some(_)) | (None, None) => {
                bail!("[dev.automations] needs exactly one of `entry` (a module exporting defineAutomations) and `forward` (your app's webhook URL)")
            }
            (Some(entry), None) => {
                if !root.join(entry).is_file() {
                    bail!("[dev.automations] entry `{entry}` is not a file in the project");
                }
            }
            (None, Some(url)) => {
                let parsed = reqwest::Url::parse(url)
                    .with_context(|| format!("[dev.automations] forward `{url}` is not a URL"))?;
                if !matches!(parsed.scheme(), "http" | "https") {
                    bail!("[dev.automations] forward `{url}` must be an http(s) URL");
                }
            }
        }
        if matches!(&config.command, Some(command) if command.is_empty()) {
            bail!("[dev.automations] command must name a program");
        }
        Ok(Some(config))
    }

    /// The paths whose changes restart the runner: the entry's directory (or
    /// the entry itself at the project root), the `watch` paths, and the
    /// generated TypeScript protocol clients.
    fn watched(&self, root: &Path) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(entry) = &self.entry {
            let entry = root.join(entry);
            match entry.parent() {
                Some(parent) if parent != root => paths.push(parent.to_path_buf()),
                _ => paths.push(entry),
            }
        }
        paths.extend(self.watch.iter().map(|path| root.join(path)));
        paths.push(root.join("generated").join("typescript").join("protocols"));
        paths
    }
}

/// The files of `paths` now.
fn snapshot(paths: &[PathBuf]) -> Snapshot {
    let mut snapshot = Snapshot::new();
    for path in paths {
        if path.is_dir() {
            watch::walk(path, &mut snapshot, 0);
        } else if let Ok(metadata) = std::fs::metadata(path) {
            snapshot.insert(path.clone(), (metadata.modified().ok(), metadata.len()));
        }
    }
    snapshot
}

/// `node <runner> dev`, with the runner from the nearest `node_modules` at
/// or above `root` (workspaces hoist packages to the repository root).
fn default_command(root: &Path) -> Result<Vec<String>> {
    let script = root
        .ancestors()
        .map(|dir| RUNNER_SCRIPT.iter().fold(dir.to_path_buf(), |path, part| path.join(part)))
        .find(|path| path.is_file())
        .context("@usearete/automations is not installed in the project: run `npm install @usearete/automations` (and `tsx` for TypeScript)")?;
    Ok(vec![
        "node".into(),
        script.display().to_string(),
        "dev".into(),
    ])
}

/// Where the runner keeps its state.
pub fn state_dir(root: &Path) -> PathBuf {
    root.join(".arete").join("dev").join("automations")
}

/// Only the owner may read `path`: a mode set when a file is created does
/// not change one that already exists.
fn owner_only(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
}

/// The project's dev webhook secret, created on first use (mode 0600).
pub fn webhook_secret(root: &Path) -> Result<String> {
    let path = state_dir(root).join("webhook-secret");
    if let Ok(secret) = std::fs::read_to_string(&path) {
        let secret = secret.trim();
        if !secret.is_empty() {
            owner_only(&state_dir(root), 0o700);
            owner_only(&path, 0o600);
            return Ok(secret.to_string());
        }
    }
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| anyhow::anyhow!("no randomness: {error}"))?;
    let secret = format!(
        "whsec_dev_{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    std::fs::create_dir_all(state_dir(root))?;
    owner_only(&state_dir(root), 0o700);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    writeln!(file, "{secret}")?;
    owner_only(&path, 0o600);
    Ok(secret)
}

/// Reports a line: (event, human text, JSON fields).
pub type Emit = std::sync::Arc<dyn Fn(&str, &str, Value) + Send + Sync>;

/// The running automations and what restarts them.
pub struct Runner {
    config: AutomationsConfig,
    root: PathBuf,
    api_key: String,
    /// The edge the runner's SDKs call in dev mode.
    edge_url: String,
    emit: Emit,
    watched: Vec<PathBuf>,
    snapshot: Snapshot,
    child: Option<Child>,
    /// The exit was reported; wait for a change before starting again.
    exited: bool,
    /// The deployment and stack the session named when last seen.
    target: Option<(Option<String>, Option<String>)>,
}

impl Runner {
    pub fn new(
        config: AutomationsConfig,
        root: &Path,
        api_key: &str,
        edge_url: &str,
        emit: Emit,
    ) -> Self {
        let watched = config.watched(root);
        Self {
            snapshot: snapshot(&watched),
            config,
            root: root.to_path_buf(),
            api_key: api_key.to_string(),
            edge_url: edge_url.to_string(),
            emit,
            watched,
            child: None,
            exited: false,
            target: None,
        }
    }

    /// Restart when the session's deployment or stack changes: the runner
    /// reads them once, so a redeploy to a new deployment, or a stack found
    /// after it started, needs a fresh runner (in forward mode too, where no
    /// generated client changes).
    pub async fn follow_target(&mut self, deployment: Option<&str>, stack: Option<&str>) {
        let next = (deployment.map(str::to_string), stack.map(str::to_string));
        let previous = self.target.replace(next.clone());
        let Some(previous) = previous else {
            return;
        };
        // The runner waits for the first deployment itself.
        if previous == next || previous.0.is_none() && next.1.is_none() {
            return;
        }
        (self.emit)(
            "automations-reload",
            "The deployment or its stack changed; restarting the automations",
            json!({ "deploymentId": next.0, "stack": next.1 }),
        );
        self.stop().await;
        if let Err(error) = self.start() {
            self.report_failure(&error);
        }
    }

    fn describe(&self) -> String {
        match (&self.config.entry, &self.config.forward) {
            (Some(entry), _) => format!("automations from {entry}"),
            (_, Some(url)) => format!("event forwarding to {url}"),
            _ => "automations".into(),
        }
    }

    pub fn start(&mut self) -> Result<()> {
        let command = match &self.config.command {
            Some(command) => command.clone(),
            None => default_command(&self.root)?,
        };
        let mut process = Command::new(&command[0]);
        process
            .args(&command[1..])
            .current_dir(&self.root)
            .env("ARETE_PROJECT_ROOT", &self.root)
            .env("ARETE_API_KEY", &self.api_key)
            .env("ARETE_WEBHOOK_SECRET", webhook_secret(&self.root)?)
            .env("ARETE_AUTOMATIONS_PORT", "0")
            // The installed SDKs in the runner call the environment through
            // the edge, as `a4 dev run` does.
            .envs(super::run::variables(&self.edge_url, &[]))
            .env_remove("ARETE_DEV_ENV")
            .env_remove("ARETE_AUTOMATIONS_ENTRY")
            .env_remove("ARETE_AUTOMATIONS_FORWARD")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(entry) = &self.config.entry {
            process.env("ARETE_AUTOMATIONS_ENTRY", entry);
        }
        if let Some(url) = &self.config.forward {
            process.env("ARETE_AUTOMATIONS_FORWARD", url);
        }
        let mut child = process.spawn().with_context(|| {
            format!(
                "Failed to run `{}` for [dev.automations]; install @usearete/automations in the project",
                command.join(" ")
            )
        })?;
        for stream in [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        ]
        .into_iter()
        .flatten()
        {
            let emit = self.emit.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    emit(
                        "automations",
                        &format!("automations │ {line}"),
                        json!({ "line": line }),
                    );
                }
            });
        }
        (self.emit)(
            "automations-started",
            &format!("Running {}", self.describe()),
            json!({ "entry": self.config.entry, "forward": self.config.forward }),
        );
        self.child = Some(child);
        self.exited = false;
        Ok(())
    }

    /// Restart on a change to the watched files (with `reload`); report an
    /// exit once.
    pub async fn poll(&mut self, reload: bool) {
        let now = if reload {
            snapshot(&self.watched)
        } else {
            self.snapshot.clone()
        };
        let changed = watch::changed_paths(&self.snapshot, &now);
        if !changed.is_empty() {
            self.snapshot = now;
            let names: Vec<String> = changed
                .iter()
                .map(|path| {
                    path.strip_prefix(&self.root)
                        .unwrap_or(path)
                        .display()
                        .to_string()
                })
                .collect();
            (self.emit)(
                "automations-reload",
                &format!("Changed: {}; restarting the automations", names.join(", ")),
                json!({ "changed": names }),
            );
            self.stop().await;
            if let Err(error) = self.start() {
                self.report_failure(&error);
            }
            return;
        }
        if let Some(child) = &mut self.child {
            if let Ok(Some(status)) = child.try_wait() {
                self.child = None;
                if !self.exited {
                    self.exited = true;
                    (self.emit)(
                        "automations-exited",
                        &format!("The automations stopped ({status}); they restart when you save a change"),
                        json!({ "code": status.code() }),
                    );
                }
            }
        }
    }

    fn report_failure(&mut self, error: &anyhow::Error) {
        self.exited = true;
        (self.emit)(
            "automations-failed",
            &format!("{error:#}"),
            json!({ "error": format!("{error:#}") }),
        );
    }

    /// Stop the runner, giving it [`STOP_GRACE`] to save its state.
    pub async fn stop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        #[cfg(unix)]
        if let Some(pid) = child.id().and_then(|pid| libc::pid_t::try_from(pid).ok()) {
            // SAFETY: signalling our own child by its pid.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
            if tokio::time::timeout(STOP_GRACE, child.wait()).await.is_ok() {
                return;
            }
        }
        let _ = child.kill().await;
    }

    /// Start, reporting a failure instead of ending the session.
    pub fn start_or_report(&mut self) {
        if let Err(error) = self.start() {
            self.report_failure(&error);
        }
    }
}

/// The running runner's control port and token, from
/// `.arete/dev/automations/runner.json` (mode 0600).
pub fn control(root: &Path) -> Result<(u16, String)> {
    let path = state_dir(root).join("runner.json");
    let runner: Value = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .context("The automations are not running: set [dev.automations] in arete.toml and run `a4 dev up`")?;
    let pid = runner["pid"].as_u64().unwrap_or(0) as u32;
    if !super::runtime::pid_alive(pid) {
        bail!("The automations are not running: set [dev.automations] in arete.toml and run `a4 dev up`");
    }
    let port = runner["port"]
        .as_u64()
        .and_then(|port| u16::try_from(port).ok())
        .context("runner.json has no port")?;
    let token = runner["token"]
        .as_str()
        .context("runner.json has no token")?
        .to_string();
    Ok((port, token))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str, root: &Path) -> Result<Option<AutomationsConfig>> {
        AutomationsConfig::parse(source, root)
    }

    #[test]
    fn reads_an_entry_or_a_forward_url() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("automations")).unwrap();
        std::fs::write(dir.path().join("automations/index.ts"), "").unwrap();
        assert_eq!(parse("[dev]\nsdk = []\n", dir.path()).unwrap(), None);
        let config = parse(
            "[dev.automations]\nentry = \"automations/index.ts\"\nwatch = [\"lib\"]\n",
            dir.path(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            config.watched(dir.path()),
            vec![
                dir.path().join("automations"),
                dir.path().join("lib"),
                dir.path().join("generated/typescript/protocols"),
            ]
        );
        let forward = parse(
            "[dev.automations]\nforward = \"http://localhost:3000/api/arete\"\n",
            dir.path(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            forward.forward.as_deref(),
            Some("http://localhost:3000/api/arete")
        );
        for bad in [
            "[dev.automations]\n",
            "[dev.automations]\nentry = \"automations/index.ts\"\nforward = \"http://x\"\n",
            "[dev.automations]\nentry = \"missing.ts\"\n",
            "[dev.automations]\nforward = \"ftp://x\"\n",
            "[dev.automations]\nentry = \"automations/index.ts\"\ncommand = []\n",
            "[dev.automations]\nentry = \"automations/index.ts\"\nsurprise = 1\n",
        ] {
            assert!(parse(bad, dir.path()).is_err(), "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_webhook_secret_is_made_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(state_dir(dir.path())).unwrap();
        let path = state_dir(dir.path()).join("webhook-secret");
        std::fs::write(&path, "whsec_dev_existing\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(webhook_secret(dir.path()).unwrap(), "whsec_dev_existing");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn a_load_error_says_what_is_wrong() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("arete.toml"),
            "[dev.automations]\nentry = \"automations.ts\"\n",
        )
        .unwrap();
        // The top-level message is all `a4` prints, so it carries the cause.
        let error = AutomationsConfig::load(dir.path()).unwrap_err().to_string();
        assert!(error.starts_with("Invalid "), "{error}");
        assert!(
            error.contains("entry `automations.ts` is not a file"),
            "{error}"
        );
    }

    #[test]
    fn an_entry_at_the_root_watches_itself() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("automations.ts"), "").unwrap();
        let config = parse(
            "[dev.automations]\nentry = \"automations.ts\"\n",
            dir.path(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            config.watched(dir.path())[0],
            dir.path().join("automations.ts")
        );
    }

    #[test]
    fn finds_the_runner_in_the_nearest_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("packages/app");
        std::fs::create_dir_all(&project).unwrap();
        assert!(default_command(&project).is_err());
        let script = dir
            .path()
            .join("node_modules/@usearete/automations/dist/cli.js");
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        std::fs::write(&script, "").unwrap();
        assert_eq!(
            default_command(&project).unwrap(),
            vec![
                "node".to_string(),
                script.display().to_string(),
                "dev".into()
            ]
        );
    }

    #[test]
    fn the_webhook_secret_is_created_once() {
        let dir = tempfile::tempdir().unwrap();
        let first = webhook_secret(dir.path()).unwrap();
        assert!(first.starts_with("whsec_dev_") && first.len() == "whsec_dev_".len() + 64);
        assert_eq!(webhook_secret(dir.path()).unwrap(), first);
    }

    #[tokio::test]
    async fn restarts_when_the_deployment_or_stack_changes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("automations.ts"), "1").unwrap();
        let config = AutomationsConfig {
            entry: Some("automations.ts".into()),
            command: Some(vec!["sh".into(), "-c".into(), "sleep 30".into()]),
            ..Default::default()
        };
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink = events.clone();
        let emit: Emit = std::sync::Arc::new(move |event: &str, _: &str, _: Value| {
            sink.lock().unwrap().push(event.to_string());
        });
        let mut runner = Runner::new(
            config,
            dir.path(),
            "a4_sk_test",
            "http://127.0.0.1:8899",
            emit,
        );
        runner.start().unwrap();
        let reloads = || {
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| *e == "automations-reload")
                .count()
        };
        // Waiting for the first deployment, then the deployment appearing: no restart.
        runner.follow_target(None, None).await;
        runner.follow_target(Some("d-1"), None).await;
        runner.follow_target(Some("d-1"), None).await;
        assert_eq!(reloads(), 0);
        // The stack found, then a redeploy: one restart each.
        runner.follow_target(Some("d-1"), Some("wss://s1")).await;
        runner.follow_target(Some("d-2"), Some("wss://s1")).await;
        assert_eq!(reloads(), 2);
        runner.stop().await;
    }

    /// The runner's SDKs are in dev mode, through the edge; a direct-mode
    /// environment from the user's shell does not leak in.
    #[tokio::test]
    async fn runs_the_runner_in_dev_mode_through_the_edge() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("automations.ts"), "1").unwrap();
        let config = AutomationsConfig {
            entry: Some("automations.ts".into()),
            command: Some(vec![
                "sh".into(),
                "-c".into(),
                "echo \"env=$ARETE_ENVIRONMENT url=$ARETE_DEV_URL direct=${ARETE_DEV_ENV:-none}\""
                    .into(),
            ]),
            ..Default::default()
        };
        let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink = lines.clone();
        let emit: Emit = std::sync::Arc::new(move |event: &str, _: &str, fields: Value| {
            if event == "automations" {
                sink.lock()
                    .unwrap()
                    .push(fields["line"].as_str().unwrap_or_default().to_string());
            }
        });
        let mut runner = Runner::new(
            config,
            dir.path(),
            "a4_sk_test",
            "http://127.0.0.1:8899",
            emit,
        );
        runner.start().unwrap();
        for _ in 0..100 {
            if !lines.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            lines.lock().unwrap().clone(),
            vec!["env=dev url=http://127.0.0.1:8899 direct=none".to_string()]
        );
        runner.stop().await;
    }

    #[tokio::test]
    async fn restarts_on_a_change_and_reports_an_exit_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("automations.ts"), "1").unwrap();
        let config = AutomationsConfig {
            entry: Some("automations.ts".into()),
            command: Some(vec![
                "sh".into(),
                "-c".into(),
                "echo started; exit 3".into(),
            ]),
            ..Default::default()
        };
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink = events.clone();
        let emit: Emit = std::sync::Arc::new(move |event: &str, _: &str, _: Value| {
            sink.lock().unwrap().push(event.to_string());
        });
        let mut runner = Runner::new(
            config,
            dir.path(),
            "a4_sk_test",
            "http://127.0.0.1:8899",
            emit,
        );
        runner.start().unwrap();
        for _ in 0..50 {
            runner.poll(true).await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let seen = events.lock().unwrap().clone();
        assert_eq!(
            seen.iter().filter(|e| *e == "automations-exited").count(),
            1,
            "{seen:?}"
        );
        assert!(seen.contains(&"automations".to_string()), "{seen:?}");

        // A change to the entry starts it again.
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(dir.path().join("automations.ts"), "22").unwrap();
        runner.poll(true).await;
        assert_eq!(
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| *e == "automations-started")
                .count(),
            2
        );
        runner.stop().await;
    }
}
