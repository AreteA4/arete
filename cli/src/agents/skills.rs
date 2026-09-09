//! Skills installation via `npx skills` (spec WP7 §5) and the skill-dir
//! checks used by `a4 doctor`.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{display_path, find_on_path, read_optional, Env, ItemResult, Outcome};

pub const SKILLS_SOURCE: &str = "AreteA4/skills";
pub const SKILL_NAMES: [&str; 5] = [
    "arete",
    "arete-streams",
    "arete-programs",
    "arete-stack-authoring",
    "arete-deploy",
];
const LEGACY_SKILL_NAMES: [&str; 2] = ["arete-consume", "arete-build"];
pub const SKILLS_TIMEOUT: Duration = Duration::from_secs(120);
const LOCK_FILE: &str = "skills-lock.json";

/// `skills` CLI agent name for an a4 agent id (`None` = not supported).
pub fn skills_agent_name(id: &str) -> Option<&'static str> {
    Some(match id {
        "claude-code" => "claude-code",
        "cursor" => "cursor",
        "codex" => "codex",
        "opencode" => "opencode",
        "gemini-cli" => "gemini-cli",
        "copilot-cli" => "github-copilot",
        "windsurf" => "windsurf",
        "cline" => "cline",
        "zed" => "zed",
        "amp" => "amp",
        "kiro" => "kiro-cli",
        "roo" => "roo",
        "goose" => "goose",
        _ => return None,
    })
}

/// Map selected ids to distinct `skills` agent names, in order.
pub fn skills_agent_names(ids: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    ids.iter()
        .filter_map(|id| skills_agent_name(id))
        .filter(|name| seen.insert(*name))
        .map(str::to_string)
        .collect()
}

/// Skills source argument for `--skills-ref`.
///
/// Verified 2026-09-03 against `skills` 1.5.23: the `tree/<ref>` URL is the
/// pinning syntax (there is no `--ref` flag; `owner/repo#<ref>` also works
/// but `owner/repo@<ref>` is silently ignored). The CLI runs
/// `git clone --branch <ref>`, so branches and tags pin and are recorded
/// as `"ref"` in `skills-lock.json`, while a commit SHA fails with
/// `fatal: Remote branch <sha> not found`. Hence `is_commit_sha` below.
pub fn source_for_ref(skills_ref: Option<&str>) -> String {
    match skills_ref {
        Some(reference) if !reference.trim().is_empty() && reference != "main" => {
            format!(
                "https://github.com/AreteA4/skills/tree/{}",
                reference.trim()
            )
        }
        _ => SKILLS_SOURCE.to_string(),
    }
}

/// Whether `reference` looks like a git commit SHA (7 to 40 hex digits),
/// which `npx skills add` cannot pin (see `source_for_ref`).
pub fn is_commit_sha(reference: &str) -> bool {
    let reference = reference.trim();
    (7..=40).contains(&reference.len()) && reference.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The manual fix command.
pub fn fix_command(agent_names: &[String]) -> String {
    if agent_names.is_empty() {
        format!("npx skills add {SKILLS_SOURCE}")
    } else {
        format!(
            "npx skills add {SKILLS_SOURCE} --agent {}",
            agent_names.join(" ")
        )
    }
}

/// Where `npx skills` writes its lock file for this scope.
fn lock_paths(env: &Env, global: bool) -> Vec<PathBuf> {
    let mut paths = vec![env.root.join(LOCK_FILE)];
    if global {
        if let Some(home) = &env.home {
            paths.push(home.join(".agents").join(LOCK_FILE));
        }
    }
    paths
}

fn lock_snapshot(env: &Env, global: bool) -> Vec<Option<String>> {
    lock_paths(env, global)
        .iter()
        .map(|path| read_optional(path).ok().flatten())
        .collect()
}

/// Legacy skill names that the lock records as owned by AreteA4/skills.
///
/// The skills CLI does not remove skills that disappear from a source, so a
/// normal add would otherwise leave both the old and replacement activation
/// units installed. Never remove an unrecorded or differently sourced skill.
fn owned_legacy_skills(snapshot: &[Option<String>]) -> Vec<&'static str> {
    let mut found = BTreeSet::new();
    for content in snapshot.iter().flatten() {
        let Ok(lock) = serde_json::from_str::<serde_json::Value>(content) else {
            continue;
        };
        let Some(skills) = lock.get("skills").and_then(serde_json::Value::as_object) else {
            continue;
        };
        for name in LEGACY_SKILL_NAMES {
            let source = skills
                .get(name)
                .and_then(|entry| entry.get("source"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if is_official_skills_source(source) {
                found.insert(name);
            }
        }
    }
    LEGACY_SKILL_NAMES
        .into_iter()
        .filter(|name| found.contains(name))
        .collect()
}

/// Whether a lockfile `source` is exactly the official AreteA4/skills
/// repository, in any form `source_for_ref` can produce. A substring test
/// would misclassify lookalikes such as `notaretea4/skills` or
/// `AreteA4/skills-fork` as officially owned and delete a user-managed skill.
fn is_official_skills_source(source: &str) -> bool {
    let source = source.trim().to_ascii_lowercase();
    source == "aretea4/skills"
        || source.starts_with("aretea4/skills#")
        || source.starts_with("https://github.com/aretea4/skills/tree/")
}

fn legacy_remove_command(names: &[&str], global: bool) -> String {
    format!(
        "npx skills remove {} -y{}",
        names.join(" "),
        if global { " -g" } else { "" }
    )
}

fn remove_legacy_skills(
    npx: &std::path::Path,
    env: &Env,
    names: &[&str],
    global: bool,
    timeout: Duration,
) -> Result<(), String> {
    if names.is_empty() {
        return Ok(());
    }
    let mut command = Command::new(npx);
    command
        .arg("-y")
        .arg("skills")
        .arg("remove")
        .args(names)
        .arg("-y")
        .current_dir(&env.root)
        .env("DO_NOT_TRACK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if global {
        command.arg("-g");
    }
    if let Some(home) = &env.home {
        command.env("HOME", home);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to run legacy skill cleanup: {error}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!("legacy skill cleanup exited with {status}"));
            }
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "legacy skill cleanup timed out after {} s",
                    timeout.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => return Err(format!("failed waiting for legacy skill cleanup: {error}")),
        }
    }
}

/// Candidate skill directories for one agent (agent-specific first, then
/// the universal `.agents/skills`).
pub fn skill_dirs(env: &Env, id: &str, global: bool) -> Vec<PathBuf> {
    let base: Option<PathBuf> = if global {
        env.home.clone()
    } else {
        Some(env.root.clone())
    };
    let Some(base) = base else {
        return Vec::new();
    };
    let config = if global {
        env.xdg_config_home()
            .unwrap_or_else(|| base.join(".config"))
    } else {
        base.clone()
    };
    let mut dirs: Vec<PathBuf> = match id {
        "claude-code" => vec![base.join(".claude/skills")],
        "oh-my-pi" => vec![base.join(".omp/skills")],
        "cursor" => vec![base.join(".cursor/skills")],
        "codex" => vec![if global {
            env.codex_home()
                .unwrap_or_else(|| base.join(".codex"))
                .join("skills")
        } else {
            base.join(".codex/skills")
        }],
        "opencode" => vec![
            if global {
                config.join("opencode/skills")
            } else {
                base.join(".opencode/skills")
            },
            if global {
                config.join("opencode/skill")
            } else {
                base.join(".opencode/skill")
            },
        ],
        "gemini-cli" => vec![base.join(".gemini/skills")],
        "copilot-cli" => vec![if global {
            base.join(".copilot/skills")
        } else {
            base.join(".github/skills")
        }],
        "windsurf" => vec![if global {
            base.join(".codeium/windsurf/skills")
        } else {
            base.join(".windsurf/skills")
        }],
        "cline" => vec![base.join(".cline/skills"), base.join(".clinerules/skills")],
        "zed" => vec![if global {
            config.join("zed/skills")
        } else {
            base.join(".zed/skills")
        }],
        "amp" => vec![if global {
            config.join("amp/skills")
        } else {
            base.join(".amp/skills")
        }],
        "kiro" => vec![base.join(".kiro/skills")],
        "roo" => vec![base.join(".roo/skills")],
        "goose" => vec![if global {
            config.join("goose/skills")
        } else {
            base.join(".goose/skills")
        }],
        _ => Vec::new(),
    };
    dirs.push(base.join(".agents/skills"));
    dirs
}

/// Skill names missing for `id` (empty = all present in at least one dir).
pub fn missing_skills(env: &Env, id: &str, global: bool) -> Vec<&'static str> {
    let dirs = skill_dirs(env, id, global);
    let mut best: Option<Vec<&'static str>> = None;
    for dir in &dirs {
        let missing: Vec<&'static str> = SKILL_NAMES
            .iter()
            .copied()
            .filter(|name| !dir.join(name).join("SKILL.md").is_file())
            .collect();
        if missing.is_empty() {
            return Vec::new();
        }
        if best.as_ref().is_none_or(|b| missing.len() < b.len()) {
            best = Some(missing);
        }
    }
    best.unwrap_or_else(|| SKILL_NAMES.to_vec())
}

/// Options for one skills run.
#[derive(Debug, Clone)]
pub struct SkillsOptions {
    /// Selected a4 agent ids (mapped to `skills` names here).
    pub agent_ids: Vec<String>,
    pub skills_ref: Option<String>,
    pub global: bool,
    pub timeout: Duration,
}

/// Run `npx skills add …` and report `created|updated|unchanged`.
pub fn install(env: &Env, options: &SkillsOptions, dry_run: bool) -> ItemResult {
    let item = "skills";
    let names = skills_agent_names(&options.agent_ids);
    let fix = fix_command(&names);
    if names.is_empty() {
        return ItemResult::new(
            item,
            Outcome::skipped("no skills-capable agent selected", None),
            None,
        );
    }
    if let Some(reference) = options.skills_ref.as_deref().filter(|r| is_commit_sha(r)) {
        return ItemResult::new(
            item,
            Outcome::error(
                format!(
                    "--skills-ref {reference}: npx skills cannot pin a commit SHA (it runs `git clone --branch <ref>`); use a tag or branch"
                ),
                Some(fix),
            ),
            None,
        );
    }
    let Some(npx) = find_on_path(&env.path_env(), "npx") else {
        return ItemResult::new(item, Outcome::skipped("npx not found", Some(fix)), None);
    };
    let before = lock_snapshot(env, options.global);
    // A global install must never infer ownership from this project's lock:
    // cleanup runs with `-g`, so only the global lock can authorize it.
    let legacy = if options.global {
        owned_legacy_skills(&before[1..])
    } else {
        owned_legacy_skills(&before[..1])
    };
    let lock_display = display_path(env, &lock_paths(env, options.global)[0]);
    if dry_run {
        let complete = options
            .agent_ids
            .iter()
            .filter(|id| skills_agent_name(id).is_some())
            .all(|id| missing_skills(env, id, options.global).is_empty());
        let outcome = if complete {
            Outcome::Unchanged
        } else if before[0].is_some() {
            Outcome::Updated
        } else {
            Outcome::Created
        };
        return ItemResult::new(item, outcome, Some(lock_display));
    }

    let mut command = Command::new(&npx);
    command
        .arg("-y")
        .arg("skills")
        .arg("add")
        .arg(source_for_ref(options.skills_ref.as_deref()))
        .arg("--skill")
        .arg("*")
        .arg("--agent")
        .args(&names)
        .arg("-y");
    if cfg!(windows) {
        command.arg("--copy");
    }
    if options.global {
        command.arg("-g");
    }
    command
        .current_dir(&env.root)
        .env("DO_NOT_TRACK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(home) = &env.home {
        command.env("HOME", home);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ItemResult::new(
                item,
                Outcome::error(
                    format!("failed to run {}: {error}", npx.display()),
                    Some(fix),
                ),
                None,
            )
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let drain = |pipe: Option<std::process::ChildStdout>,
                 err: Option<std::process::ChildStderr>| {
        std::thread::spawn(move || {
            let mut out = String::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_string(&mut out);
            }
            let mut err_text = String::new();
            if let Some(mut err) = err {
                let _ = err.read_to_string(&mut err_text);
            }
            (out, err_text)
        })
    };
    let reader = drain(stdout, stderr);
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() >= options.timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!(
                    "npx skills timed out after {} s",
                    options.timeout.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => break Err(format!("failed waiting for npx skills: {error}")),
        }
    };
    // Only join the reader after a normal exit: after a kill, grandchildren
    // (node processes spawned by npx) may keep the pipes open for a while and
    // the detached thread simply finishes when they close.
    let stderr_text = match &status {
        Ok(_) => reader.join().map(|(_, err)| err).unwrap_or_default(),
        Err(_) => String::new(),
    };
    match status {
        Err(reason) => ItemResult::new(item, Outcome::error(reason, Some(fix)), None),
        Ok(status) if !status.success() => {
            let tail: Vec<&str> = stderr_text
                .lines()
                .rev()
                .take(5)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let detail = if tail.is_empty() {
                String::new()
            } else {
                format!(": {}", tail.join(" | "))
            };
            ItemResult::new(
                item,
                Outcome::error(
                    format!("npx skills exited with {status}{detail}"),
                    Some(fix),
                ),
                None,
            )
        }
        Ok(_) => {
            if let Err(reason) =
                remove_legacy_skills(&npx, env, &legacy, options.global, options.timeout)
            {
                return ItemResult::new(
                    item,
                    Outcome::error(reason, Some(legacy_remove_command(&legacy, options.global))),
                    Some(lock_display),
                );
            }
            let after = lock_snapshot(env, options.global);
            let outcome = if before == after {
                Outcome::Unchanged
            } else if before[0].is_none() && after[0].is_some() {
                Outcome::Created
            } else {
                Outcome::Updated
            };
            ItemResult::new(item, outcome, Some(lock_display))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn agent_names_map_per_spec() {
        let ids: Vec<String> = [
            "claude-code",
            "vscode",
            "copilot-cli",
            "kiro",
            "copilot-cli",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            skills_agent_names(&ids),
            vec!["claude-code", "github-copilot", "kiro-cli"]
        );
        assert_eq!(skills_agent_name("vscode"), None);
    }

    #[test]
    fn source_and_fix_strings() {
        assert_eq!(source_for_ref(None), "AreteA4/skills");
        assert_eq!(source_for_ref(Some("main")), "AreteA4/skills");
        assert_eq!(
            source_for_ref(Some("v1.2.0")),
            "https://github.com/AreteA4/skills/tree/v1.2.0"
        );
        assert!(is_commit_sha("c247da25a2ffa77e016a2b750e42420c4d84a52c"));
        assert!(is_commit_sha("c247da2"));
        assert!(!is_commit_sha("v1.2.0"));
        assert!(!is_commit_sha("main"));
        assert!(!is_commit_sha("abc"));
        assert_eq!(fix_command(&[]), "npx skills add AreteA4/skills");
        assert_eq!(
            fix_command(&["cursor".into()]),
            "npx skills add AreteA4/skills --agent cursor"
        );
    }

    #[test]
    fn official_skills_source_matching_is_exact() {
        assert!(is_official_skills_source("AreteA4/skills"));
        assert!(is_official_skills_source("aretea4/skills"));
        assert!(is_official_skills_source("AreteA4/skills#v0.6.0"));
        assert!(is_official_skills_source(
            "https://github.com/AreteA4/skills/tree/v0.6.0"
        ));
        assert!(!is_official_skills_source("notaretea4/skills"));
        assert!(!is_official_skills_source("AreteA4/skills-fork"));
        assert!(!is_official_skills_source("github.com/AreteA4/skills"));
        assert!(!is_official_skills_source("usehyperstack/skills"));
        assert!(!is_official_skills_source(""));
    }

    #[test]
    fn legacy_cleanup_only_takes_lockfile_owned_skills() {
        let lock = serde_json::json!({
            "version": 1,
            "skills": {
                "arete-consume": { "source": "AreteA4/skills" },
                "arete-build": { "source": "notaretea4/skills" }
            }
        })
        .to_string();
        assert_eq!(
            owned_legacy_skills(&[Some(lock)]),
            vec!["arete-consume"],
            "only the officially sourced legacy skill is eligible for removal"
        );
    }

    #[test]
    fn legacy_cleanup_requires_arete_owned_lock_entries() {
        let official = Some(
            r#"{"skills":{"arete-consume":{"source":"AreteA4/skills"},"arete-build":{"source":"https://github.com/AreteA4/skills/tree/v0.6.0"}}}"#.to_string(),
        );
        assert_eq!(
            owned_legacy_skills(&[official]),
            vec!["arete-consume", "arete-build"]
        );

        let other = Some(
            r#"{"skills":{"arete-consume":{"source":"someone/custom-skills"},"arete-build":{}}}"#
                .to_string(),
        );
        assert!(owned_legacy_skills(&[other]).is_empty());
        assert!(owned_legacy_skills(&[Some("not json".into())]).is_empty());
    }

    #[test]
    fn global_legacy_cleanup_ignores_project_ownership() {
        let project =
            Some(r#"{"skills":{"arete-consume":{"source":"AreteA4/skills"}}}"#.to_string());
        let global =
            Some(r#"{"skills":{"arete-consume":{"source":"someone/custom-skills"}}}"#.to_string());
        let snapshot = [project, global];
        assert!(owned_legacy_skills(&snapshot[1..]).is_empty());
    }

    #[test]
    fn legacy_cleanup_command_preserves_scope() {
        let names = ["arete-consume", "arete-build"];
        assert_eq!(
            legacy_remove_command(&names, false),
            "npx skills remove arete-consume arete-build -y"
        );
        assert_eq!(
            legacy_remove_command(&names, true),
            "npx skills remove arete-consume arete-build -y -g"
        );
    }

    #[cfg(unix)]
    #[test]
    fn install_runs_cleanup_for_lock_owned_legacy_skills() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let root = dir.path().join("proj");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join(LOCK_FILE),
            r#"{"skills":{"arete-consume":{"source":"AreteA4/skills"},"arete-build":{"source":"AreteA4/skills"}}}"#,
        )
        .unwrap();
        let npx = bin.join("npx");
        fs::write(
            &npx,
            r#"#!/bin/sh
if [ "$3" = "remove" ]; then
  printf '%s\n' "$@" > remove-args.txt
  printf '{"skills":{"arete":{"source":"AreteA4/skills"}}}\n' > skills-lock.json
else
  printf '%s\n' "$@" > add-args.txt
fi
"#,
        )
        .unwrap();
        fs::set_permissions(&npx, fs::Permissions::from_mode(0o755)).unwrap();

        let env = Env::new(&root, None, &[("PATH", bin.to_str().unwrap())]);
        let options = SkillsOptions {
            agent_ids: vec!["codex".into()],
            skills_ref: None,
            global: false,
            timeout: SKILLS_TIMEOUT,
        };
        assert_eq!(install(&env, &options, false).outcome, Outcome::Updated);
        let args = fs::read_to_string(root.join("remove-args.txt")).unwrap();
        assert_eq!(
            args.lines().collect::<Vec<_>>(),
            vec![
                "-y",
                "skills",
                "remove",
                "arete-consume",
                "arete-build",
                "-y"
            ]
        );
    }

    #[test]
    fn missing_skills_accepts_universal_dir() {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::new(dir.path(), None, &[]);
        assert_eq!(missing_skills(&env, "cursor", false), SKILL_NAMES.to_vec());
        for name in [
            "arete",
            "arete-streams",
            "arete-programs",
            "arete-stack-authoring",
        ] {
            fs::create_dir_all(dir.path().join(".agents/skills").join(name)).unwrap();
            fs::write(
                dir.path()
                    .join(".agents/skills")
                    .join(name)
                    .join("SKILL.md"),
                "x",
            )
            .unwrap();
        }
        assert_eq!(missing_skills(&env, "cursor", false), vec!["arete-deploy"]);
        fs::create_dir_all(dir.path().join(".agents/skills/arete-deploy")).unwrap();
        fs::write(dir.path().join(".agents/skills/arete-deploy/SKILL.md"), "x").unwrap();
        assert!(missing_skills(&env, "cursor", false).is_empty());
    }

    #[test]
    fn skipped_without_npx_and_without_capable_agents() {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::new(dir.path(), None, &[("PATH", dir.path().to_str().unwrap())]);
        let options = SkillsOptions {
            agent_ids: vec!["claude-code".into()],
            skills_ref: None,
            global: false,
            timeout: SKILLS_TIMEOUT,
        };
        let result = install(&env, &options, false);
        assert_eq!(
            result.outcome,
            Outcome::skipped(
                "npx not found",
                Some("npx skills add AreteA4/skills --agent claude-code".into())
            )
        );
        let options = SkillsOptions {
            agent_ids: vec!["vscode".into()],
            ..options
        };
        assert!(matches!(
            install(&env, &options, false).outcome,
            Outcome::Skipped { .. }
        ));
    }

    #[test]
    fn commit_sha_ref_is_rejected_before_running_npx() {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::new(dir.path(), None, &[("PATH", dir.path().to_str().unwrap())]);
        let options = SkillsOptions {
            agent_ids: vec!["claude-code".into()],
            skills_ref: Some("c247da25a2ffa77e016a2b750e42420c4d84a52c".into()),
            global: false,
            timeout: SKILLS_TIMEOUT,
        };
        for dry_run in [true, false] {
            let result = install(&env, &options, dry_run);
            assert!(
                matches!(&result.outcome, Outcome::Error { reason, .. } if reason.contains("cannot pin a commit SHA")),
                "{:?}",
                result.outcome
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn fake_npx_reports_created_then_unchanged_and_timeouts_kill() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let root = dir.path().join("proj");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&root).unwrap();
        let npx = bin.join("npx");
        fs::write(
            &npx,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > args.txt\necho '{\"skills\":{}}' > skills-lock.json\n",
        )
        .unwrap();
        fs::set_permissions(&npx, fs::Permissions::from_mode(0o755)).unwrap();
        let env = Env::new(&root, None, &[("PATH", bin.to_str().unwrap())]);
        let options = SkillsOptions {
            agent_ids: vec!["claude-code".into(), "vscode".into(), "copilot-cli".into()],
            skills_ref: Some("v2".into()),
            global: false,
            timeout: SKILLS_TIMEOUT,
        };
        assert_eq!(install(&env, &options, false).outcome, Outcome::Created);
        let args = fs::read_to_string(root.join("args.txt")).unwrap();
        let args: Vec<&str> = args.lines().collect();
        assert_eq!(
            args,
            vec![
                "-y",
                "skills",
                "add",
                "https://github.com/AreteA4/skills/tree/v2",
                "--skill",
                "*",
                "--agent",
                "claude-code",
                "github-copilot",
                "-y"
            ]
        );
        assert_eq!(install(&env, &options, false).outcome, Outcome::Unchanged);
        assert_eq!(
            install(&env, &options, true).outcome,
            Outcome::Updated,
            "dry run: skills dirs missing"
        );

        fs::write(&npx, "#!/bin/sh\nsleep 30\n").unwrap();
        let options = SkillsOptions {
            timeout: Duration::from_millis(300),
            ..options
        };
        let result = install(&env, &options, false);
        assert!(
            matches!(&result.outcome, Outcome::Error { reason, .. } if reason.contains("timed out")),
            "{:?}",
            result.outcome
        );
    }
}
