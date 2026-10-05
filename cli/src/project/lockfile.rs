use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::manifest::{DependencyKind, InstallTarget};
use super::{GENERATOR_CONTRACT, RESOLVER_CONTRACT};

pub const LOCK_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectLock {
    pub lock_version: u32,
    pub manifest_hash: String,
    pub resolver_contract: String,
    #[serde(default, rename = "dependency")]
    pub dependencies: Vec<LockedDependency>,
}

impl ProjectLock {
    pub fn empty(manifest_hash: String) -> Self {
        Self {
            lock_version: LOCK_VERSION,
            manifest_hash,
            resolver_contract: RESOLVER_CONTRACT.into(),
            dependencies: Vec::new(),
        }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let source = fs::read_to_string(path)
            .with_context(|| format!("Failed to read lockfile {}", path.display()))?;
        // The version is read before the strict decode, so a lock a newer a4
        // wrote in a newer format says so instead of naming a field.
        if let Some(version) = toml::from_str::<toml::Value>(&source)
            .ok()
            .and_then(|value| value.get("lock_version")?.as_integer())
            .filter(|version| *version > i64::from(LOCK_VERSION))
        {
            bail!(
                "{} has lock_version {version}, written by a newer a4; this a4 reads lock_version {LOCK_VERSION}. Upgrade a4 (`a4 self update`)",
                path.display()
            );
        }
        let mut lock: Self = toml::from_str(&source).with_context(|| {
            format!(
                "Failed to decode strict lockfile {} (if a newer a4 wrote it, upgrade a4 with `a4 self update`)",
                path.display()
            )
        })?;
        lock.normalize_and_validate()?;
        Ok(lock)
    }

    pub fn load_optional(path: impl AsRef<Path>) -> Result<Option<Self>> {
        let path = path.as_ref();
        path.exists().then(|| Self::load(path)).transpose()
    }

    pub fn normalize_and_validate(&mut self) -> Result<()> {
        if self.lock_version != LOCK_VERSION {
            bail!(
                "Unsupported lock_version {}; expected {LOCK_VERSION}",
                self.lock_version
            );
        }
        if !self.manifest_hash.starts_with("arete-manifest-v1:") {
            bail!("lockfile manifest_hash is not an Arete manifest v1 hash");
        }
        if self.resolver_contract != RESOLVER_CONTRACT {
            bail!(
                "Unsupported resolver contract '{}'; expected '{}'",
                self.resolver_contract,
                RESOLVER_CONTRACT
            );
        }
        for dependency in &mut self.dependencies {
            dependency.normalize_and_validate()?;
        }
        self.dependencies.sort_by(|left, right| {
            (left.kind, left.alias.as_str()).cmp(&(right.kind, right.alias.as_str()))
        });
        for pair in self.dependencies.windows(2) {
            if pair[0].kind == pair[1].kind && pair[0].alias == pair[1].alias {
                bail!(
                    "lockfile {} dependency alias '{}' is duplicated",
                    pair[0].kind,
                    pair[0].alias
                );
            }
        }
        Ok(())
    }

    pub fn is_fresh(&self, manifest_hash: &str) -> bool {
        self.manifest_hash == manifest_hash
    }

    pub fn canonical_toml(&self) -> Result<String> {
        let mut normalized = self.clone();
        normalized.normalize_and_validate()?;
        toml::to_string_pretty(&normalized).context("Failed to serialize lockfile")
    }

    pub fn write_atomic(&self, path: impl AsRef<Path>) -> Result<bool> {
        let path = path.as_ref();
        let contents = self.canonical_toml()?;
        if fs::read_to_string(path).ok().as_deref() == Some(contents.as_str()) {
            return Ok(false);
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create lockfile directory {}", parent.display()))?;
        let temporary = temporary_path(path);
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .with_context(|| {
                    format!(
                        "Failed to create temporary lockfile {}",
                        temporary.display()
                    )
                })?;
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temporary, path).with_context(|| {
                format!(
                    "Failed to atomically replace lockfile {} with {}",
                    path.display(),
                    temporary.display()
                )
            })?;
            if let Ok(directory) = OpenOptions::new().read(true).open(parent) {
                let _ = directory.sync_all();
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        Ok(true)
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("arete.lock");
    path.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedDependency {
    pub kind: DependencyKind,
    pub alias: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_release_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_manifest_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_spec_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_release_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub live_specs: Vec<LockedLiveSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub programs: Vec<LockedProgram>,
    /// The parts a composed stack was composed from: each published stack
    /// and program package at its exact release, and each local file by its
    /// artifact hash. Empty for every other dependency, so their entries are
    /// unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<LockedPart>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sdk_extension_hashes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<InstallTarget>,
    pub generator_contract: String,
}

impl LockedDependency {
    fn normalize_and_validate(&mut self) -> Result<()> {
        if self.alias.is_empty() || self.source.is_empty() {
            bail!("lockfile dependency alias and source cannot be empty");
        }
        if self.generator_contract != GENERATOR_CONTRACT {
            bail!(
                "lockfile dependency '{}' uses unsupported generator contract '{}'",
                self.alias,
                self.generator_contract
            );
        }
        let registry = self.source.starts_with("registry:");
        if registry
            && (self.requirement.is_none()
                || self.version.is_none()
                || self.package_release_hash.is_none())
        {
            bail!(
                "registry dependency '{}' lacks a requirement, version, or package release hash",
                self.alias
            );
        }
        if !registry
            && (self.version.is_some()
                || self.package_release_hash.is_some()
                || self.requirement.is_some())
        {
            bail!(
                "local dependency '{}' contains registry-only fields",
                self.alias
            );
        }
        match self.kind {
            DependencyKind::Stack if self.stack_manifest_hash.is_none() => {
                bail!(
                    "stack dependency '{}' lacks StackManifest identity",
                    self.alias
                )
            }
            DependencyKind::Program
                if self.program_id.is_none() || self.program_spec_hash.is_none() =>
            {
                bail!(
                    "program dependency '{}' lacks ProgramSpec identity",
                    self.alias
                )
            }
            _ => {}
        }
        self.live_specs
            .sort_by(|left, right| left.alias.cmp(&right.alias));
        self.programs.sort_by(|left, right| {
            (left.program_id.as_str(), left.program_spec_hash.as_str())
                .cmp(&(right.program_id.as_str(), right.program_spec_hash.as_str()))
        });
        // A composed stack is local, but the program SDKs it composes are
        // published program package releases.
        let composed = !self.parts.is_empty();
        if composed && (registry || self.kind != DependencyKind::Stack) {
            bail!(
                "dependency '{}' locks composed parts, which only a composed stack has",
                self.alias
            );
        }
        for program in &mut self.programs {
            if !registry && !composed && program.package_release_hash.is_some() {
                bail!(
                    "local dependency '{}' locks a program package release",
                    self.alias
                );
            }
            program.sdk_extension_hashes.sort();
            program.sdk_extension_hashes.dedup();
        }
        for part in &mut self.parts {
            part.normalize_and_validate(&self.alias)?;
        }
        self.parts
            .sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
        self.sdk_extension_hashes.sort();
        self.sdk_extension_hashes.dedup();
        self.targets.sort();
        self.targets.dedup();
        Ok(())
    }
}

/// One part of a composed stack, as resolved.
///
/// A `stack` part provides the composed live alias `live`: the LiveSpec
/// `artifact_hash`, which is `live_alias` of the published stack release
/// `package_release_hash` (StackManifest `stack_manifest_hash`), or a local
/// LiveSpec file. A `program` part is a program package release or a local
/// ProgramSpec file (`artifact_hash`). Delivery is transport state and is
/// never locked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedPart {
    pub kind: DependencyKind,
    /// `registry:<package>` or `path:<manifest-relative file>`.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_release_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_manifest_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    pub artifact_hash: String,
}

impl LockedPart {
    fn sort_key(&self) -> (DependencyKind, &str, &str, &str) {
        (
            self.kind,
            self.live.as_deref().unwrap_or_default(),
            self.source.as_str(),
            self.artifact_hash.as_str(),
        )
    }

    fn normalize_and_validate(&self, alias: &str) -> Result<()> {
        let registry = self.source.starts_with("registry:");
        if !registry && !self.source.starts_with("path:") {
            bail!(
                "composed stack '{alias}' locks a part with unsupported source '{}'",
                self.source
            );
        }
        if self.artifact_hash.is_empty() {
            bail!("composed stack '{alias}' locks a part without an artifact hash");
        }
        if registry
            && (self.requirement.is_none()
                || self.version.is_none()
                || self.package_release_hash.is_none())
        {
            bail!(
                "composed stack '{alias}' part '{}' lacks a requirement, version, or package release hash",
                self.source
            );
        }
        if !registry
            && (self.requirement.is_some()
                || self.version.is_some()
                || self.package_release_hash.is_some()
                || self.stack_manifest_hash.is_some()
                || self.live_alias.is_some())
        {
            bail!(
                "composed stack '{alias}' part '{}' is a local file but locks registry fields",
                self.source
            );
        }
        match self.kind {
            DependencyKind::Stack if self.live.is_none() => bail!(
                "composed stack '{alias}' part '{}' does not name the live alias it provides",
                self.source
            ),
            DependencyKind::Stack
                if registry && (self.stack_manifest_hash.is_none() || self.live_alias.is_none()) =>
            {
                bail!(
                    "composed stack '{alias}' part '{}' lacks its source StackManifest or live alias",
                    self.source
                )
            }
            DependencyKind::Program if self.live.is_some() || self.program_id.is_none() => bail!(
                "composed stack '{alias}' program part '{}' must name its program ID and no live alias",
                self.source
            ),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedLiveSpec {
    pub alias: String,
    pub artifact_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedProgram {
    pub program_id: String,
    pub program_spec_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_release_hash: Option<String>,
    /// The program package release (program SDK identity) the stack
    /// references for this program. Absent when the stack embeds the core
    /// program only, so locks written before it existed are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_release_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sdk_extension_hashes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(alias: &str) -> LockedDependency {
        LockedDependency {
            kind: DependencyKind::Program,
            alias: alias.into(),
            source: format!("path:artifacts/{alias}.program-spec.json"),
            requirement: None,
            version: None,
            package_release_hash: None,
            stack_manifest_hash: None,
            program_id: Some(format!("{alias}111")),
            program_spec_hash: Some(format!("hash-{alias}")),
            program_release_hash: None,
            live_specs: Vec::new(),
            programs: Vec::new(),
            parts: Vec::new(),
            sdk_extension_hashes: Vec::new(),
            targets: vec![InstallTarget::TypeScript],
            generator_contract: GENERATOR_CONTRACT.into(),
        }
    }

    #[test]
    fn canonical_lock_order_is_deterministic_and_write_is_idempotent() {
        let mut first = ProjectLock::empty(format!("arete-manifest-v1:{:064x}", 1));
        first.dependencies = vec![local("zeta"), local("alpha")];
        let mut second = first.clone();
        second.dependencies.reverse();
        assert_eq!(
            first.canonical_toml().unwrap(),
            second.canonical_toml().unwrap()
        );

        let root = std::env::temp_dir().join(format!("arete-lock-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("arete.lock");
        assert!(first.write_atomic(&path).unwrap());
        assert!(!second.write_atomic(&path).unwrap());
        assert_eq!(
            ProjectLock::load(path).unwrap().dependencies[0].alias,
            "alpha"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn registry_stack(programs: Vec<LockedProgram>) -> LockedDependency {
        LockedDependency {
            kind: DependencyKind::Stack,
            alias: "ore".into(),
            source: "registry:ore".into(),
            requirement: Some("^1.0.0".into()),
            version: Some("1.0.1".into()),
            package_release_hash: Some(format!(
                "arete:registry-package-release:v2:sha256:{}",
                "5".repeat(64)
            )),
            stack_manifest_hash: Some("stack-manifest-hash".into()),
            program_id: None,
            program_spec_hash: None,
            program_release_hash: None,
            live_specs: Vec::new(),
            programs,
            parts: Vec::new(),
            sdk_extension_hashes: Vec::new(),
            targets: vec![InstallTarget::TypeScript],
            generator_contract: GENERATOR_CONTRACT.into(),
        }
    }

    #[test]
    fn program_sdk_releases_round_trip_and_older_locks_still_load() {
        let release = format!(
            "arete:registry-package-release:v2:sha256:{}",
            "7".repeat(64)
        );
        let mut lock = ProjectLock::empty(format!("arete-manifest-v1:{:064x}", 4));
        lock.dependencies = vec![registry_stack(vec![
            LockedProgram {
                program_id: "ore111".into(),
                program_spec_hash: "spec-ore".into(),
                program_release_hash: Some("release-ore".into()),
                package_release_hash: Some(release.clone()),
                sdk_extension_hashes: vec!["b".repeat(64), "a".repeat(64)],
            },
            LockedProgram {
                program_id: "entropy111".into(),
                program_spec_hash: "spec-entropy".into(),
                program_release_hash: Some("release-entropy".into()),
                package_release_hash: None,
                sdk_extension_hashes: Vec::new(),
            },
        ])];
        let text = lock.canonical_toml().unwrap();
        assert!(text.contains(&format!("package_release_hash = \"{release}\"")));
        let mut reloaded: ProjectLock = toml::from_str(&text).unwrap();
        reloaded.normalize_and_validate().unwrap();
        lock.normalize_and_validate().unwrap();
        assert_eq!(reloaded, lock);
        let ore = &reloaded.dependencies[0].programs[1];
        assert_eq!(ore.program_id, "ore111");
        assert_eq!(ore.package_release_hash.as_deref(), Some(release.as_str()));
        assert_eq!(
            ore.sdk_extension_hashes,
            vec!["a".repeat(64), "b".repeat(64)]
        );

        // A lock written before program SDK releases were recorded loads
        // unchanged and serializes byte-for-byte as before.
        let older = text
            .lines()
            .filter(|line| {
                !line.starts_with(
                    "package_release_hash = \"arete:registry-package-release:v2:sha256:7",
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let mut older_lock: ProjectLock = toml::from_str(&older).unwrap();
        older_lock.normalize_and_validate().unwrap();
        assert!(older_lock.dependencies[0]
            .programs
            .iter()
            .all(|program| program.package_release_hash.is_none()));
        assert_eq!(older_lock.canonical_toml().unwrap(), older);
    }

    fn composed_stack() -> LockedDependency {
        let mut stack = registry_stack(vec![LockedProgram {
            program_id: "token111".into(),
            program_spec_hash: "spec-token".into(),
            program_release_hash: Some("release-token".into()),
            package_release_hash: Some(format!(
                "arete:registry-package-release:v2:sha256:{}",
                "8".repeat(64)
            )),
            sdk_extension_hashes: Vec::new(),
        }]);
        stack.alias = "ore-plus-token".into();
        stack.source = "workspace:ore-plus-token".into();
        stack.requirement = None;
        stack.version = None;
        stack.package_release_hash = None;
        stack.parts = vec![
            LockedPart {
                kind: DependencyKind::Program,
                source: "registry:spl-token".into(),
                live: None,
                requirement: Some("^4".into()),
                version: Some("4.0.1".into()),
                package_release_hash: Some(format!(
                    "arete:registry-package-release:v2:sha256:{}",
                    "8".repeat(64)
                )),
                stack_manifest_hash: None,
                live_alias: None,
                program_id: Some("token111".into()),
                artifact_hash: "spec-token".into(),
            },
            LockedPart {
                kind: DependencyKind::Stack,
                source: "registry:ore".into(),
                live: Some("ore".into()),
                requirement: Some("^1".into()),
                version: Some("1.0.2".into()),
                package_release_hash: Some(format!(
                    "arete:registry-package-release:v2:sha256:{}",
                    "5".repeat(64)
                )),
                stack_manifest_hash: Some("source-stack-manifest".into()),
                live_alias: Some("live".into()),
                program_id: None,
                artifact_hash: "live-spec-ore".into(),
            },
            LockedPart {
                kind: DependencyKind::Stack,
                source: "path:artifacts/local.live-spec.json".into(),
                live: Some("local".into()),
                requirement: None,
                version: None,
                package_release_hash: None,
                stack_manifest_hash: None,
                live_alias: None,
                program_id: None,
                artifact_hash: "live-spec-local".into(),
            },
        ];
        stack
    }

    #[test]
    fn a_composed_stack_locks_its_parts_and_program_sdk_releases() {
        let mut lock = ProjectLock::empty(format!("arete-manifest-v1:{:064x}", 6));
        lock.dependencies = vec![composed_stack()];
        let text = lock.canonical_toml().unwrap();
        assert!(text.contains("[[dependency.parts]]"), "{text}");
        assert!(text.contains("source = \"registry:spl-token\""), "{text}");
        assert!(text.contains("live_alias = \"live\""), "{text}");
        let mut reloaded: ProjectLock = toml::from_str(&text).unwrap();
        reloaded.normalize_and_validate().unwrap();
        lock.normalize_and_validate().unwrap();
        assert_eq!(reloaded, lock);
        // Parts are ordered stacks (by live alias) before programs.
        let parts = &reloaded.dependencies[0].parts;
        assert_eq!(parts[0].live.as_deref(), Some("local"));
        assert_eq!(parts[1].live.as_deref(), Some("ore"));
        assert_eq!(parts[2].kind, DependencyKind::Program);
    }

    #[test]
    fn composed_parts_must_carry_their_identities() {
        let invalid = |edit: fn(&mut LockedDependency), expected: &str| {
            let mut stack = composed_stack();
            edit(&mut stack);
            let mut lock = ProjectLock::empty(format!("arete-manifest-v1:{:064x}", 7));
            lock.dependencies = vec![stack];
            let error = lock.normalize_and_validate().unwrap_err().to_string();
            assert!(error.contains(expected), "{expected}: {error}");
        };
        invalid(
            |stack| stack.parts[0].package_release_hash = None,
            "lacks a requirement, version, or package release hash",
        );
        invalid(
            |stack| stack.parts[1].live_alias = None,
            "lacks its source StackManifest or live alias",
        );
        invalid(
            |stack| stack.parts[2].version = Some("1.0.0".into()),
            "is a local file but locks registry fields",
        );
        invalid(
            |stack| stack.parts[1].live = None,
            "does not name the live alias",
        );
        invalid(
            |stack| stack.parts[0].source = "workspace:other".into(),
            "unsupported source",
        );
        invalid(
            |stack| {
                stack.source = "registry:ore-plus-token".into();
                stack.requirement = Some("^1".into());
                stack.version = Some("1.0.0".into());
                stack.package_release_hash = Some("release".into());
            },
            "only a composed stack has",
        );
    }

    #[test]
    fn a_lock_from_a_newer_format_says_to_upgrade() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("arete.lock");
        std::fs::write(
            &path,
            format!(
                "lock_version = 2\nmanifest_hash = \"arete-manifest-v1:{:064x}\"\nresolver_contract = \"{RESOLVER_CONTRACT}\"\nfuture_field = true\n",
                8
            ),
        )
        .unwrap();
        let error = format!("{:#}", ProjectLock::load(&path).unwrap_err());
        assert!(
            error.contains("lock_version 2, written by a newer a4"),
            "{error}"
        );
        assert!(error.contains("a4 self update"), "{error}");

        // A current-version lock with a field this a4 does not know points at
        // the same fix.
        std::fs::write(
            &path,
            format!(
                "lock_version = 1\nmanifest_hash = \"arete-manifest-v1:{:064x}\"\nresolver_contract = \"{RESOLVER_CONTRACT}\"\nfuture_field = true\n",
                8
            ),
        )
        .unwrap();
        let error = format!("{:#}", ProjectLock::load(&path).unwrap_err());
        assert!(error.contains("unknown field `future_field`"), "{error}");
        assert!(error.contains("a4 self update"), "{error}");
    }

    #[test]
    fn a_local_stack_cannot_lock_a_program_sdk_release() {
        let mut stack = registry_stack(vec![LockedProgram {
            program_id: "ore111".into(),
            program_spec_hash: "spec-ore".into(),
            program_release_hash: None,
            package_release_hash: Some("release".into()),
            sdk_extension_hashes: Vec::new(),
        }]);
        stack.source = "path:ore.stack-manifest.json".into();
        stack.requirement = None;
        stack.version = None;
        stack.package_release_hash = None;
        let mut lock = ProjectLock::empty(format!("arete-manifest-v1:{:064x}", 5));
        lock.dependencies = vec![stack];
        assert!(lock
            .normalize_and_validate()
            .unwrap_err()
            .to_string()
            .contains("locks a program package release"));
    }

    #[test]
    fn lock_identity_is_scoped_by_dependency_kind() {
        let mut stack = local("shared");
        stack.kind = DependencyKind::Stack;
        stack.stack_manifest_hash = Some("stack-manifest-hash".into());
        let program = local("shared");
        let mut lock = ProjectLock::empty(format!("arete-manifest-v1:{:064x}", 2));
        lock.dependencies = vec![program, stack];

        lock.normalize_and_validate().unwrap();
        assert_eq!(lock.dependencies.len(), 2);
        assert_eq!(lock.dependencies[0].kind, DependencyKind::Stack);
        assert_eq!(lock.dependencies[1].kind, DependencyKind::Program);

        lock.dependencies.push(lock.dependencies[1].clone());
        assert!(lock
            .normalize_and_validate()
            .unwrap_err()
            .to_string()
            .contains("program dependency alias 'shared' is duplicated"));
    }
}
