//! Which sandbox a sandbox call acts on.
//!
//! An agent account may be granted several sandboxes. The Arete API lists
//! the ones its key can mint a sandbox token for right now
//! (`GET /api/agent/sandboxes`), and a call resolves its sandbox in this
//! order:
//!
//! 1. the call's own `sandbox` (an id, or a granted sandbox's name);
//! 2. the sandbox chosen with `sandbox_select` / `a4 sandbox use`, which is
//!    saved per project (the working directory) and API;
//! 3. `ARETE_SANDBOX_ID`;
//! 4. the only active sandbox the account has been granted.
//!
//! Anything else (no grant, or several and none chosen) is refused with a
//! code and a hint naming the tool that fixes it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The file, in the Arete home directory, that keeps each project's choice.
const SELECTIONS_FILE: &str = "sandbox-selections.json";
/// The most choices the file keeps; the oldest go first.
const MAX_SAVED: usize = 256;

/// Where a call's sandbox came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The call named it.
    Argument,
    /// Chosen in this session (`sandbox_select`).
    Selected,
    /// Chosen earlier for this project and saved.
    Saved,
    /// `ARETE_SANDBOX_ID`.
    Env,
    /// The only active sandbox the account has been granted.
    OnlyGrant,
    /// The only sandbox the API named when it refused a direct read by a
    /// key a sandbox grant binds (`sandbox-read-required`).
    BoundKey,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Argument => "argument",
            Self::Selected => "selected",
            Self::Saved => "saved",
            Self::Env => "env:ARETE_SANDBOX_ID",
            Self::OnlyGrant => "only-grant",
            Self::BoundKey => "bound-key",
        }
    }
}

/// The sandbox a call acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub sandbox_id: String,
    pub source: Source,
}

impl Resolved {
    pub fn to_json(&self) -> Value {
        json!({ "sandboxId": self.sandbox_id, "source": self.source.as_str() })
    }
}

/// One sandbox the account's key can mint a sandbox token for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantedSandbox {
    pub sandbox_id: String,
    pub name: Option<String>,
    pub wallet_address: Option<String>,
    pub cluster: Option<String>,
    /// `active` or `paused`; a paused sandbox refuses every action until its
    /// owner resumes it.
    pub status: String,
    /// The granted actions, as the token's `sandbox:<action>` scopes.
    pub actions: Vec<String>,
    pub grant_expires_at: Option<String>,
}

impl GrantedSandbox {
    pub fn active(&self) -> bool {
        self.status == "active"
    }

    pub fn to_json(&self) -> Value {
        json!({
            "sandboxId": self.sandbox_id,
            "name": self.name,
            "walletAddress": self.wallet_address,
            "cluster": self.cluster,
            "status": self.status,
            "actions": self.actions,
            "grantExpiresAt": self.grant_expires_at,
        })
    }

    /// The entries of a listing. An entry without a sandbox id is skipped:
    /// nothing could be done with it.
    pub fn from_listing(body: &Value) -> Option<Vec<Self>> {
        let entries = body.get("sandboxes")?.as_array()?;
        Some(entries.iter().filter_map(Self::from_entry).collect())
    }

    fn from_entry(entry: &Value) -> Option<Self> {
        let text = |key: &str| {
            entry
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let sandbox_id = text("sandboxId").filter(|id| is_sandbox_id(id))?;
        let mut actions: Vec<String> = entry
            .get("actions")
            .and_then(Value::as_array)
            .map(|actions| {
                actions
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        actions.sort();
        actions.dedup();
        Some(Self {
            sandbox_id,
            name: text("name"),
            wallet_address: text("walletAddress"),
            cluster: text("cluster"),
            // Fail closed: a sandbox of unknown status is not picked.
            status: text("status").unwrap_or_else(|| "unknown".into()),
            actions,
            grant_expires_at: text("grantExpiresAt"),
        })
    }
}

/// Whether `value` has the shape of a sandbox id (a UUID).
pub fn is_sandbox_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value.trim()).is_ok()
}

/// Why no sandbox could be chosen from the grants alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unresolved {
    /// The account has no granted sandbox.
    NoneGranted,
    /// Every granted sandbox is paused.
    AllPaused(Vec<GrantedSandbox>),
    /// Several active sandboxes and none chosen.
    SeveralGranted(Vec<GrantedSandbox>),
    /// The call named a sandbox by a name several granted sandboxes share.
    AmbiguousName(String, Vec<GrantedSandbox>),
    /// The call named a sandbox the account has not been granted.
    Unknown(String),
}

/// The only active grant, else why there is none to pick.
pub fn only_active(grants: &[GrantedSandbox]) -> Result<&GrantedSandbox, Unresolved> {
    let active: Vec<&GrantedSandbox> = grants.iter().filter(|g| g.active()).collect();
    match active.as_slice() {
        [only] => Ok(only),
        [] if grants.is_empty() => Err(Unresolved::NoneGranted),
        [] => Err(Unresolved::AllPaused(grants.to_vec())),
        _ => Err(Unresolved::SeveralGranted(
            active.into_iter().cloned().collect(),
        )),
    }
}

/// The granted sandbox `wanted` names: its id, or its name (any case) when
/// exactly one has it.
pub fn find<'a>(
    grants: &'a [GrantedSandbox],
    wanted: &str,
) -> Result<&'a GrantedSandbox, Unresolved> {
    let wanted = wanted.trim();
    if let Some(grant) = grants
        .iter()
        .find(|grant| grant.sandbox_id.eq_ignore_ascii_case(wanted))
    {
        return Ok(grant);
    }
    let named: Vec<&GrantedSandbox> = grants
        .iter()
        .filter(|grant| {
            grant
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
        })
        .collect();
    match named.as_slice() {
        [only] => Ok(only),
        [] => Err(Unresolved::Unknown(wanted.to_string())),
        _ => Err(Unresolved::AmbiguousName(
            wanted.to_string(),
            named.into_iter().cloned().collect(),
        )),
    }
}

// ---- The saved choice --------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
struct SavedFile {
    #[serde(default)]
    selections: Vec<SavedChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedChoice {
    /// The API origin the choice was made against.
    api: String,
    /// The project: the working directory it was made in.
    project: String,
    sandbox_id: String,
}

/// Each project's chosen sandbox, kept in the Arete home directory (never
/// in the project, so nothing lands in a repository).
#[derive(Debug, Clone)]
pub struct SelectionStore {
    path: PathBuf,
    project: PathBuf,
}

impl SelectionStore {
    /// The choices file for `project`.
    pub fn at(path: PathBuf, project: PathBuf) -> Self {
        Self { path, project }
    }

    /// The Arete home directory's file, for the working directory. `None`
    /// when either is unknown: the choice then lasts only the session.
    pub fn for_working_dir() -> Option<Self> {
        let home = match std::env::var_os("ARETE_HOME").filter(|path| !path.is_empty()) {
            Some(path) => PathBuf::from(path),
            None => dirs::home_dir()?.join(".arete"),
        };
        let cwd = std::env::current_dir().ok()?;
        let project = std::fs::canonicalize(&cwd).unwrap_or(cwd);
        Some(Self::at(home.join(SELECTIONS_FILE), project))
    }

    fn project_key(&self) -> String {
        self.project.to_string_lossy().into_owned()
    }

    fn read(&self) -> SavedFile {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// The sandbox chosen for this project and `api`.
    pub fn load(&self, api: &str) -> Option<String> {
        let project = self.project_key();
        self.read()
            .selections
            .into_iter()
            .rev()
            .find(|choice| choice.api == api && choice.project == project)
            .map(|choice| choice.sandbox_id)
            .filter(|id| is_sandbox_id(id))
    }

    /// Save `sandbox_id` as this project's choice for `api`, or forget the
    /// choice with `None`. The file is shared by every project and process
    /// (`a4 sandbox use` beside a running `a4 mcp`, say), so the whole
    /// read, change and write holds an exclusive lock: a concurrent save
    /// waits instead of overwriting this one's change, or this one's.
    pub fn save(&self, api: &str, sandbox_id: Option<&str>) -> Result<(), String> {
        let _lock = self.lock()?;
        let project = self.project_key();
        let mut file = self.read();
        file.selections
            .retain(|choice| !(choice.api == api && choice.project == project));
        if let Some(sandbox_id) = sandbox_id {
            file.selections.push(SavedChoice {
                api: api.to_string(),
                project,
                sandbox_id: sandbox_id.to_string(),
            });
        }
        let excess = file.selections.len().saturating_sub(MAX_SAVED);
        file.selections.drain(..excess);
        let text = serde_json::to_string_pretty(&file).map_err(|error| error.to_string())?;
        write_private(&self.path, text.as_bytes())
    }

    /// An exclusive lock on the file's sibling `<file>.lock`, held until the
    /// returned handle is dropped. The lock file is separate because the
    /// choices file itself is replaced by a rename on every save.
    fn lock(&self) -> Result<std::fs::File, String> {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| format!("{} has no parent directory", self.path.display()))?;
        create_private_dir(dir)?;
        let mut lock_path = self.path.clone().into_os_string();
        lock_path.push(".lock");
        let mut options = std::fs::OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(&lock_path).map_err(|error| {
            format!(
                "could not open {}: {error}",
                Path::new(&lock_path).display()
            )
        })?;
        lock.lock()
            .map_err(|error| format!("could not lock the sandbox choices: {error}"))?;
        Ok(lock)
    }
}

/// Create `dir` (and its parents), entered only by its owner, unless it
/// exists.
fn create_private_dir(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        return Ok(());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))
}

/// Write `bytes` to `path` whole (a temporary file renamed over it),
/// readable only by its owner, in a directory only its owner can enter
/// when this creates it.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let dir = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    create_private_dir(dir)?;
    // A named temporary file is created owner-only.
    let mut file = tempfile::NamedTempFile::new_in(dir)
        .map_err(|error| format!("could not write in {}: {error}", dir.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("could not write the sandbox choice: {error}"))?;
    file.persist(path)
        .map_err(|error| format!("could not save {}: {}", path.display(), error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10";
    const B: &str = "0b5a3e8e-2f7d-4c1a-8e2b-7f3c9d1e4a20";

    fn grant(id: &str, name: Option<&str>, status: &str) -> GrantedSandbox {
        GrantedSandbox {
            sandbox_id: id.into(),
            name: name.map(str::to_string),
            wallet_address: None,
            cluster: Some("mainnet-beta".into()),
            status: status.into(),
            actions: vec!["invoke".into()],
            grant_expires_at: None,
        }
    }

    #[test]
    fn a_listing_is_read_leniently_and_fails_closed_on_status() {
        let body = json!({ "sandboxes": [
            { "sandboxId": A, "name": "Bot", "walletAddress": "W", "cluster": "devnet",
              "status": "active", "actions": ["withdraw", "invoke", "invoke"],
              "grantExpiresAt": "2026-12-01T00:00:00Z" },
            { "sandboxId": "not-a-uuid", "status": "active" },
            { "sandboxId": B, "name": null },
        ], "requestId": "r" });
        let grants = GrantedSandbox::from_listing(&body).unwrap();
        assert_eq!(grants.len(), 2);
        assert_eq!(grants[0].actions, ["invoke", "withdraw"]);
        assert_eq!(grants[0].name.as_deref(), Some("Bot"));
        assert_eq!(grants[1].status, "unknown");
        assert!(!grants[1].active());
        assert!(GrantedSandbox::from_listing(&json!({ "items": [] })).is_none());
    }

    #[test]
    fn only_one_active_grant_is_picked() {
        assert_eq!(only_active(&[]), Err(Unresolved::NoneGranted));
        let one = [grant(A, None, "active"), grant(B, None, "paused")];
        assert_eq!(only_active(&one).unwrap().sandbox_id, A);
        let paused = [grant(A, None, "paused")];
        assert!(matches!(
            only_active(&paused),
            Err(Unresolved::AllPaused(list)) if list.len() == 1
        ));
        let two = [grant(A, None, "active"), grant(B, None, "active")];
        assert!(matches!(
            only_active(&two),
            Err(Unresolved::SeveralGranted(list)) if list.len() == 2
        ));
    }

    #[test]
    fn a_sandbox_is_found_by_id_or_unique_name() {
        let grants = [
            grant(A, Some("Bot"), "active"),
            grant(B, Some("bot"), "active"),
        ];
        assert_eq!(find(&grants, &A.to_uppercase()).unwrap().sandbox_id, A);
        assert!(matches!(
            find(&grants, "BOT"),
            Err(Unresolved::AmbiguousName(name, list)) if name == "BOT" && list.len() == 2
        ));
        assert_eq!(
            find(&grants, "other"),
            Err(Unresolved::Unknown("other".into()))
        );
        let grants = [grant(A, Some("Bot"), "active")];
        assert_eq!(find(&grants, " bot ").unwrap().sandbox_id, A);
    }

    #[test]
    fn choices_are_saved_per_project_and_api() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("home").join(SELECTIONS_FILE);
        let here = SelectionStore::at(path.clone(), "/work/one".into());
        let there = SelectionStore::at(path.clone(), "/work/two".into());
        assert_eq!(here.load("https://api"), None);

        here.save("https://api", Some(A)).unwrap();
        there.save("https://api", Some(B)).unwrap();
        assert_eq!(here.load("https://api").as_deref(), Some(A));
        assert_eq!(there.load("https://api").as_deref(), Some(B));
        assert_eq!(here.load("http://127.0.0.1:1"), None);

        here.save("https://api", Some(B)).unwrap();
        assert_eq!(here.load("https://api").as_deref(), Some(B));
        here.save("https://api", None).unwrap();
        assert_eq!(here.load("https://api"), None);
        assert_eq!(there.load("https://api").as_deref(), Some(B));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(path.parent().unwrap()), 0o700);
        }
    }

    #[test]
    fn concurrent_saves_keep_every_choice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("home").join(SELECTIONS_FILE);
        // Many projects save at once, each from its own handle (as separate
        // processes would): no save may overwrite another's choice.
        let writers: Vec<_> = (0..16)
            .map(|n| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let store = SelectionStore::at(path, format!("/p/{n}").into());
                    for round in 0..8 {
                        let id = if (n + round) % 2 == 0 { A } else { B };
                        store.save("https://api", Some(id)).unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        for n in 0..16 {
            let store = SelectionStore::at(path.clone(), format!("/p/{n}").into());
            let last = if (n + 7) % 2 == 0 { A } else { B };
            assert_eq!(store.load("https://api").as_deref(), Some(last), "/p/{n}");
        }
        let file: SavedFile =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(file.selections.len(), 16);
        // Nothing but the file and its lock is left behind.
        let mut names: Vec<String> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                SELECTIONS_FILE.to_string(),
                format!("{SELECTIONS_FILE}.lock")
            ]
        );
    }

    #[test]
    fn an_unreadable_file_is_no_choice_and_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SELECTIONS_FILE);
        std::fs::write(&path, "not json").unwrap();
        let store = SelectionStore::at(path, "/work".into());
        assert_eq!(store.load("https://api"), None);
        store.save("https://api", Some(A)).unwrap();
        assert_eq!(store.load("https://api").as_deref(), Some(A));
    }

    #[test]
    fn the_file_keeps_the_newest_choices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SELECTIONS_FILE);
        for n in 0..(MAX_SAVED + 3) {
            SelectionStore::at(path.clone(), format!("/p/{n}").into())
                .save("https://api", Some(A))
                .unwrap();
        }
        let file: SavedFile =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(file.selections.len(), MAX_SAVED);
        assert_eq!(file.selections[0].project, "/p/3");
    }
}
