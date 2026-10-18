//! `a4 protocol deploy`, before the build: push the programs the project
//! installed from a local IDL, so the release's templates, its stack and its
//! registry package resolve them on Arete.
//!
//! A release uploads only its spec. Arete checks templates against, compiles
//! the stack from, and pins in the protocol's package only programs it has.
//! A program installed with `a4 install program ./idl/<name>.json` exists
//! only in the project, so deploy registers it through the program flow
//! (`a4 program push`): the account's private program, which the platform
//! admits and publishes as an owner-private program package. Then:
//!
//! - **Detect:** every `[dependencies.programs]` entry with an `idl` source.
//!   Its ProgramSpec is built from the IDL exactly as `a4 install` builds it
//!   and must equal the one arete.lock pins.
//! - **Already there:** the account's own program with that ProgramSpec
//!   (`GET /api/programs`; ready, or still being admitted, which is waited
//!   for), else a registry program package of that exact ProgramSpec
//!   (`GET /api/registry/programs`). Nothing is uploaded for those.
//! - **Push:** `POST /api/programs` with alias `<protocol slug>-<alias>`
//!   (with the ProgramSpec digest's first 8 characters added when that alias
//!   already names another program of the account), then poll until the
//!   admission is `ready` or `failed`, at most `--wait-secs`.
//! - **Record:** arete.lock's entry keeps `source = { idl = ... }` and gains
//!   the admitted `program_release_hash`, and the generated program is
//!   regenerated with it, so the build pins that release and the protocol's
//!   package pins the program package instead of leaving it unpinned. With
//!   `--no-build` nothing is recorded: the build being uploaded was made
//!   without it, and the platform still finds the program by its ProgramSpec.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::api_client::{
    api_error_details, ApiClient, CreateUserProgramRequest, UserProgramResponse,
    USER_PROGRAM_UPLOAD_SCHEMA,
};
use crate::project::manifest::{DependencyKind, DependencySourceV1, IdlSourceV1};
use crate::project::{ProjectLock, ProjectManifest};

const PAGE_LIMIT: usize = 100;
/// The longest user-program alias the platform accepts.
const MAX_ALIAS_LEN: usize = 64;

/// What deploy did about one program installed from a local IDL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalProgramOutcome {
    /// The dependency's alias in arete.toml.
    pub alias: String,
    pub idl: String,
    pub program_id: String,
    pub program_spec_hash: String,
    /// `pushed` (uploaded now), `registered` (the account already had it),
    /// `registry` (a registry program package has this exact ProgramSpec)
    /// or `planned` (a dry run, which uploads nothing).
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_program_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_release_hash: Option<String>,
    /// Whether arete.lock now records `program_release_hash`.
    pub locked: bool,
}

/// One `idl` program dependency and the ProgramSpec its IDL builds.
#[derive(Debug, Clone)]
struct LocalProgram {
    alias: String,
    idl: String,
    program_spec: arete_artifacts::ProgramSpecArtifact,
    /// What arete.lock records for it now.
    locked_release: Option<String>,
}

impl LocalProgram {
    fn program_id(&self) -> &str {
        &self.program_spec.payload.program_id
    }
    fn program_spec_hash(&self) -> String {
        self.program_spec.artifact_hash.to_string()
    }
}

/// The progress lines deploy prints.
pub trait Progress {
    fn step(&self, message: &str);
    fn success(&self, message: &str);
    fn info(&self, message: &str);
}

/// How [`publish`] runs.
pub struct PublishOptions<'a> {
    pub slug: &'a str,
    pub dry_run: bool,
    /// Record the admitted releases in arete.lock and regenerate.
    pub record: bool,
    pub wait: Duration,
}

/// The project's programs installed from a local IDL, each with the
/// ProgramSpec its IDL builds. An IDL whose ProgramSpec differs from the
/// lock's is refused: the project must be reinstalled first.
fn detect(dir: &Path) -> Result<Vec<LocalProgram>> {
    let manifest_path = dir.join("arete.toml");
    if !manifest_path.is_file() {
        return Ok(Vec::new());
    }
    // An arete.toml with only `[protocol]` declares no dependencies.
    let declares_dependencies = std::fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|source| toml::from_str::<toml::Value>(&source).ok())
        .is_some_and(|value| value.get("manifest_version").is_some());
    if !declares_dependencies {
        return Ok(Vec::new());
    }
    let manifest = ProjectManifest::load(&manifest_path)?;
    let lock = ProjectLock::load_optional(manifest.root.join("arete.lock"))?;
    let mut programs = Vec::new();
    for (kind, alias, dependency) in manifest.dependencies() {
        let DependencySourceV1::Idl(IdlSourceV1 { idl, program_id }) = &dependency.source else {
            continue;
        };
        if kind != DependencyKind::Program {
            continue;
        }
        let path = idl_path(&manifest.root, idl)?;
        let bytes = std::fs::read(&path)
            .with_context(|| format!("Failed to read IDL {}", path.display()))?;
        let payload = arete_hash::build_program_spec_v1_from_bytes(&bytes, program_id.as_deref())
            .map_err(anyhow::Error::new)
            .with_context(|| format!("Failed to build a ProgramSpec from IDL {idl}"))?;
        let program_spec = arete_artifacts::ProgramSpecArtifact::new(payload)?;
        let entry = lock.as_ref().and_then(|lock| {
            lock.dependencies
                .iter()
                .find(|entry| entry.kind == DependencyKind::Program && entry.alias == *alias)
        });
        let hash = program_spec.artifact_hash.to_string();
        if let Some(locked) = entry.and_then(|entry| entry.program_spec_hash.as_deref()) {
            if locked != hash {
                bail!(
                    "program '{alias}': {idl} builds ProgramSpec {hash}, but arete.lock pins {locked}. Run `a4 install`, then deploy again."
                );
            }
        }
        programs.push(LocalProgram {
            alias: alias.clone(),
            idl: idl.clone(),
            program_spec,
            locked_release: entry.and_then(|entry| entry.program_release_hash.clone()),
        });
    }
    Ok(programs)
}

/// A project-relative IDL path, never outside the project.
fn idl_path(root: &Path, idl: &str) -> Result<PathBuf> {
    let relative = Path::new(idl);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        bail!("IDL {idl} is outside the project");
    }
    Ok(root.join(relative))
}

/// What [`publish`] did.
#[derive(Debug, Default)]
pub struct Published {
    pub programs: Vec<LocalProgramOutcome>,
    /// arete.lock and the generated programs changed: the build is stale.
    pub lock_changed: bool,
}

/// Push and record the project's local-IDL programs; see the module docs.
pub fn publish(
    client: &ApiClient,
    dir: &Path,
    options: &PublishOptions<'_>,
    progress: &dyn Progress,
) -> Result<Published> {
    let programs = detect(dir)?;
    if programs.is_empty() {
        return Ok(Published::default());
    }
    if options.dry_run {
        let programs = programs
            .iter()
            .map(|program| {
                progress.info(&format!(
                    "would register program {} ({}) from {} unless the account has it",
                    program.alias,
                    program.program_id(),
                    program.idl
                ));
                LocalProgramOutcome {
                    alias: program.alias.clone(),
                    idl: program.idl.clone(),
                    program_id: program.program_id().to_string(),
                    program_spec_hash: program.program_spec_hash(),
                    action: "planned".into(),
                    user_program_id: None,
                    program_release_hash: program.locked_release.clone(),
                    locked: program.locked_release.is_some(),
                }
            })
            .collect();
        return Ok(Published {
            programs,
            lock_changed: false,
        });
    }

    let owned = list_owned_programs(client)?;
    // A registry package of the exact ProgramSpec needs no private copy. The
    // list is a convenience: without it the push links to it server-side.
    let registry = client.list_registry_programs().unwrap_or_default();
    let mut outcomes = Vec::new();
    for program in &programs {
        let hash = program.program_spec_hash();
        let existing = owned.iter().find(|candidate| {
            candidate.program_spec_hash == hash
                && candidate.program_id == program.program_id()
                && candidate.lifecycle_state == "active"
        });
        let (action, resource) = match existing {
            Some(resource) => ("registered", resource.clone()),
            None => {
                if let Some(package) = registry.iter().find(|candidate| {
                    candidate.program_spec_hash == hash
                        && candidate.program_id == program.program_id()
                }) {
                    progress.success(&format!(
                        "Program {} is the registry package {} (same ProgramSpec); nothing to push",
                        program.alias, package.install_name
                    ));
                    outcomes.push(LocalProgramOutcome {
                        alias: program.alias.clone(),
                        idl: program.idl.clone(),
                        program_id: program.program_id().to_string(),
                        program_spec_hash: hash,
                        action: "registry".into(),
                        user_program_id: None,
                        program_release_hash: Some(package.program_release_hash.clone()),
                        locked: false,
                    });
                    continue;
                }
                progress.step(&format!(
                    "Pushing program {} ({}) from {} as a private program",
                    program.alias,
                    program.program_id(),
                    program.idl
                ));
                ("pushed", push(client, options.slug, program)?)
            }
        };
        let ready = wait_for_admission(client, program, resource, options.wait, progress)?;
        let release = ready
            .program_release_hash
            .clone()
            .context("A ready program carries its Program Release")?;
        progress.success(&format!(
            "Program {} is ready ({}, release {release})",
            program.alias, ready.user_program_id
        ));
        outcomes.push(LocalProgramOutcome {
            alias: program.alias.clone(),
            idl: program.idl.clone(),
            program_id: program.program_id().to_string(),
            program_spec_hash: hash,
            action: action.into(),
            user_program_id: Some(ready.user_program_id.clone()),
            program_release_hash: Some(release),
            locked: false,
        });
    }

    let mut lock_changed = false;
    if options.record {
        lock_changed = record(dir, &programs, &mut outcomes, progress)?;
    } else {
        for outcome in &mut outcomes {
            let program = programs
                .iter()
                .find(|program| program.alias == outcome.alias);
            outcome.locked = program.and_then(|program| program.locked_release.as_ref())
                == outcome.program_release_hash.as_ref();
        }
        if outcomes.iter().any(|outcome| !outcome.locked) {
            progress.info(
                "arete.lock is unchanged (--no-build uploads the existing build); the next deploy without --no-build records the program releases",
            );
        }
    }
    Ok(Published {
        programs: outcomes,
        lock_changed,
    })
}

fn list_owned_programs(client: &ApiClient) -> Result<Vec<UserProgramResponse>> {
    let mut programs = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = client
            .list_user_programs(PAGE_LIMIT, cursor.as_deref())
            .context("Failed to list the account's programs")?;
        for program in &page.items {
            crate::commands::programs::validate_program_response(program)?;
        }
        programs.extend(page.items);
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return Ok(programs),
        }
    }
}

/// The user-program alias for `program` in protocol `slug`, which is also its
/// owner-private package slug: `<slug>-<alias>`, lowercase, within 64
/// characters; `suffix` adds the ProgramSpec digest's first 8 characters.
fn push_alias(slug: &str, program: &LocalProgram, suffix: bool) -> String {
    let mut alias: String = format!("{slug}-{}", program.alias)
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let digest = program
        .program_spec_hash()
        .rsplit(':')
        .next()
        .unwrap_or_default()
        .chars()
        .take(8)
        .collect::<String>();
    let reserved = if suffix { digest.len() + 1 } else { 0 };
    alias.truncate(MAX_ALIAS_LEN - reserved);
    let mut alias = alias.trim_end_matches('-').to_string();
    if suffix {
        alias.push('-');
        alias.push_str(&digest);
    }
    alias
}

fn push(client: &ApiClient, slug: &str, program: &LocalProgram) -> Result<UserProgramResponse> {
    let upload = |alias: String| {
        client.create_user_program(&CreateUserProgramRequest {
            schema: USER_PROGRAM_UPLOAD_SCHEMA.to_string(),
            idempotency_key: uuid::Uuid::new_v4().to_string(),
            alias: Some(alias),
            program_spec: program.program_spec.clone(),
        })
    };
    let response = match upload(push_alias(slug, program, false)) {
        // The alias names another of the account's programs (an earlier
        // version of this IDL): register this one beside it.
        Err(error)
            if api_error_details(&error).is_some_and(|details| {
                details.status == 409 && details.message.contains("alias")
            }) =>
        {
            upload(push_alias(slug, program, true))
        }
        other => other,
    }
    .with_context(|| {
        format!(
            "Failed to push program '{}' from {} (as `a4 program push {}` would)",
            program.alias, program.idl, program.idl
        )
    })?;
    crate::commands::programs::validate_program_response(&response)?;
    if response.program_spec_hash != program.program_spec_hash()
        || response.program_id != program.program_id()
    {
        bail!(
            "The server did not echo the pushed ProgramSpec of program '{}'",
            program.alias
        );
    }
    Ok(response)
}

/// Poll until the admission is `ready`; a `failed` one, or the deadline,
/// fails the deploy.
fn wait_for_admission(
    client: &ApiClient,
    program: &LocalProgram,
    mut resource: UserProgramResponse,
    wait: Duration,
    progress: &dyn Progress,
) -> Result<UserProgramResponse> {
    let started = Instant::now();
    let mut reported = String::new();
    let mut backoff = Duration::from_millis(500);
    loop {
        match resource.admission_state.as_str() {
            "ready" => return Ok(resource),
            "failed" => bail!(
                "Arete could not admit program '{}' ({}): {}. Fix {} and deploy again; the failed registration stays until `a4 program archive {} --yes`.",
                program.alias,
                resource.user_program_id,
                if resource.diagnostic_codes.is_empty() {
                    "no diagnostic".to_string()
                } else {
                    resource.diagnostic_codes.join(", ")
                },
                program.idl,
                resource.user_program_id
            ),
            state => {
                if state != reported {
                    progress.info(&format!(
                        "waiting: program {} admission {state} ({})",
                        program.alias, resource.user_program_id
                    ));
                    reported = state.to_string();
                }
            }
        }
        if started.elapsed() >= wait {
            bail!(
                "Program '{}' ({}) is still being admitted after {}s; the release would leave it unpinned. Check `a4 program status {} --watch`, then deploy again (or pass a longer --wait-secs).",
                program.alias,
                resource.user_program_id,
                wait.as_secs(),
                resource.user_program_id
            );
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(5));
        resource = client.get_user_program(&resource.user_program_id)?;
        crate::commands::programs::validate_program_response(&resource)?;
    }
}

/// Write each admitted release into its arete.lock entry and regenerate the
/// project, so the generated programs and the next build carry them.
/// Returns whether anything changed.
fn record(
    dir: &Path,
    programs: &[LocalProgram],
    outcomes: &mut [LocalProgramOutcome],
    progress: &dyn Progress,
) -> Result<bool> {
    use crate::project::installer::install_project;

    let changed: Vec<&LocalProgramOutcome> = outcomes
        .iter()
        .filter(|outcome| {
            let locked = programs
                .iter()
                .find(|program| program.alias == outcome.alias)
                .and_then(|program| program.locked_release.as_ref());
            outcome.program_release_hash.is_some()
                && locked != outcome.program_release_hash.as_ref()
        })
        .collect();
    if !changed.is_empty() {
        let manifest_path = dir.join("arete.toml");
        let lock_path = dir.join("arete.lock");
        let manifest = ProjectManifest::load(&manifest_path)?;
        let fresh = ProjectLock::load_optional(&lock_path)?
            .is_some_and(|lock| lock.is_fresh(&manifest.manifest_hash));
        if !fresh {
            install_project(&manifest_path, quiet())?;
        }
        let mut lock = ProjectLock::load(&lock_path)?;
        for outcome in &changed {
            let entry = lock
                .dependencies
                .iter_mut()
                .find(|entry| entry.kind == DependencyKind::Program && entry.alias == outcome.alias)
                .with_context(|| format!("arete.lock has no program '{}'", outcome.alias))?;
            if entry.program_spec_hash.as_deref() != Some(outcome.program_spec_hash.as_str()) {
                bail!(
                    "arete.lock's program '{}' changed during the deploy; run `a4 install` and deploy again",
                    outcome.alias
                );
            }
            entry.program_release_hash = outcome.program_release_hash.clone();
        }
        lock.write_atomic(&lock_path)?;
        install_project(&manifest_path, quiet())
            .context("Failed to regenerate the programs with their releases")?;
        for outcome in &changed {
            progress.success(&format!(
                "arete.lock: program {} pins release {}",
                outcome.alias,
                outcome.program_release_hash.as_deref().unwrap_or_default()
            ));
        }
    }
    let changed = !changed.is_empty();
    for outcome in outcomes.iter_mut() {
        outcome.locked = outcome.program_release_hash.is_some();
    }
    Ok(changed)
}

fn quiet() -> crate::project::installer::InstallOptions<'static> {
    crate::project::installer::InstallOptions {
        quiet: true,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(alias: &str) -> LocalProgram {
        let idl = br#"{"address":"11111111111111111111111111111111","metadata":{"name":"system_program","version":"1.0.0","spec":"0.1.0"},"instructions":[{"name":"transfer","discriminator":[2,0,0,0],"accounts":[{"name":"fundingAccount","writable":true,"signer":true},{"name":"recipientAccount","writable":true}],"args":[{"name":"lamports","type":"u64"}]}],"accounts":[],"types":[],"events":[],"errors":[]}"#;
        let payload = arete_hash::build_program_spec_v1_from_bytes(idl, None).unwrap();
        LocalProgram {
            alias: alias.into(),
            idl: "idl/system.idl.json".into(),
            program_spec: arete_artifacts::ProgramSpecArtifact::new(payload).unwrap(),
            locked_release: None,
        }
    }

    #[test]
    fn push_aliases_are_scoped_to_the_protocol_and_fit_the_platform() {
        let system = program("system");
        assert_eq!(push_alias("tip-jar", &system, false), "tip-jar-system");
        let suffixed = push_alias("tip-jar", &system, true);
        let hash = system.program_spec_hash();
        let digest = &hash.rsplit(':').next().unwrap()[..8];
        assert_eq!(suffixed, format!("tip-jar-system-{digest}"));
        let long = push_alias(&"a".repeat(63), &program("my_program"), true);
        assert!(long.len() <= MAX_ALIAS_LEN, "{long}");
        assert!(long.ends_with(digest));
        assert!(long
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'));
    }

    #[test]
    fn detects_idl_programs_and_refuses_a_stale_lock() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("p");
        crate::commands::protocol::init::scaffold(&crate::commands::protocol::init::InitArgs {
            dir: root.clone(),
            local_idl: true,
            ..Default::default()
        })
        .unwrap();
        let found = detect(&root).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].alias, "system");
        assert_eq!(found[0].idl, "idl/system.idl.json");
        assert_eq!(found[0].locked_release, None);
        let lock = std::fs::read_to_string(root.join("arete.lock")).unwrap();
        assert!(lock.contains(&found[0].program_spec_hash()));

        // An edited IDL no longer matches the lock.
        let path = root.join("idl/system.idl.json");
        let edited = std::fs::read_to_string(&path)
            .unwrap()
            .replace("\"lamports\"", "\"amount\"");
        std::fs::write(&path, edited).unwrap();
        let error = detect(&root).unwrap_err().to_string();
        assert!(error.contains("Run `a4 install`"), "{error}");
        assert!(idl_path(&root, "../x.json").is_err());
    }

    /// The recorded release survives `a4 install`, reaches the generated
    /// program, and is dropped when the IDL changes.
    #[test]
    fn a_recorded_release_is_kept_by_install_and_generated() {
        use crate::project::installer::{install_project, InstallOptions};

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("p");
        crate::commands::protocol::init::scaffold(&crate::commands::protocol::init::InitArgs {
            dir: root.clone(),
            local_idl: true,
            ..Default::default()
        })
        .unwrap();
        let programs = detect(&root).unwrap();
        let release = format!("arete:h1:program-release:sha256:{}", "7".repeat(64));
        let mut outcomes = vec![LocalProgramOutcome {
            alias: "system".into(),
            idl: "idl/system.idl.json".into(),
            program_id: programs[0].program_id().into(),
            program_spec_hash: programs[0].program_spec_hash(),
            action: "pushed".into(),
            user_program_id: None,
            program_release_hash: Some(release.clone()),
            locked: false,
        }];
        struct Silent;
        impl Progress for Silent {
            fn step(&self, _: &str) {}
            fn success(&self, _: &str) {}
            fn info(&self, _: &str) {}
        }
        assert!(record(&root, &programs, &mut outcomes, &Silent).unwrap());
        assert!(outcomes[0].locked);
        let generated = root.join("generated/typescript/programs/system/index.ts");
        let read_generated = || {
            std::fs::read_dir(generated.parent().unwrap())
                .unwrap()
                .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap_or_default())
                .collect::<String>()
        };
        assert!(read_generated().contains(&release));
        assert_eq!(
            detect(&root).unwrap()[0].locked_release,
            Some(release.clone())
        );

        // A plain reinstall keeps it; `--check`-style regeneration agrees.
        install_project(root.join("arete.toml"), quiet()).unwrap();
        assert_eq!(
            detect(&root).unwrap()[0].locked_release,
            Some(release.clone())
        );
        install_project(
            root.join("arete.toml"),
            InstallOptions {
                locked: true,
                quiet: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(read_generated().contains(&release));

        // A changed IDL is a new ProgramSpec with no release yet.
        let path = root.join("idl/system.idl.json");
        let edited = std::fs::read_to_string(&path)
            .unwrap()
            .replace("\"lamports\"", "\"amount\"");
        std::fs::write(&path, edited).unwrap();
        install_project(root.join("arete.toml"), quiet()).unwrap();
        assert_eq!(detect(&root).unwrap()[0].locked_release, None);
        assert!(!read_generated().contains(&release));
    }
}
