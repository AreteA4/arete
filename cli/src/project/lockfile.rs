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
        let mut lock: Self = toml::from_str(&source)
            .with_context(|| format!("Failed to decode strict lockfile {}", path.display()))?;
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
        for program in &mut self.programs {
            if !registry && program.package_release_hash.is_some() {
                bail!(
                    "local dependency '{}' locks a program package release",
                    self.alias
                );
            }
            program.sdk_extension_hashes.sort();
            program.sdk_extension_hashes.dedup();
        }
        self.sdk_extension_hashes.sort();
        self.sdk_extension_hashes.dedup();
        self.targets.sort();
        self.targets.dedup();
        Ok(())
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
