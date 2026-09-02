use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use semver::VersionReq;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct ProjectManifest {
    pub path: PathBuf,
    pub root: PathBuf,
    pub document: ManifestV1,
    pub manifest_hash: String,
}

impl ProjectManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let supplied = path.as_ref();
        let bytes = fs::read(supplied)
            .with_context(|| format!("Failed to read project manifest {}", supplied.display()))?;
        let source = std::str::from_utf8(&bytes)
            .with_context(|| format!("Project manifest {} is not UTF-8", supplied.display()))?;
        let value: toml::Value = toml::from_str(source)
            .with_context(|| format!("Failed to parse project manifest {}", supplied.display()))?;

        let version = value
            .get("manifest_version")
            .and_then(toml::Value::as_integer);
        if version.is_none() && value.get("stacks").is_some() {
            bail!(
                "{} uses the removed [[stacks]] configuration. Declare authored artifacts under [authoring] and installable SDKs under [dependencies]; .stack.json inputs are not inspected or converted",
                supplied.display()
            );
        }
        match version {
            Some(version) if version == i64::from(MANIFEST_VERSION) => {}
            Some(version) => bail!(
                "Unsupported manifest_version {version} in {}; this CLI supports manifest_version = {MANIFEST_VERSION}",
                supplied.display()
            ),
            None => bail!(
                "{} is missing required manifest_version = {MANIFEST_VERSION}",
                supplied.display()
            ),
        }

        let document: ManifestV1 = toml::from_str(source).with_context(|| {
            format!(
                "Failed to decode strict manifest_version = {MANIFEST_VERSION} document {}",
                supplied.display()
            )
        })?;
        document.validate()?;

        let path = fs::canonicalize(supplied)
            .with_context(|| format!("Failed to resolve manifest path {}", supplied.display()))?;
        let root = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Project manifest has no parent directory"))?
            .to_path_buf();
        let manifest_hash = document.resolution_hash()?;
        Ok(Self {
            path,
            root,
            document,
            manifest_hash,
        })
    }

    pub fn dependency(&self, kind: DependencyKind, alias: &str) -> Option<&DependencyV1> {
        match kind {
            DependencyKind::Stack => self.document.dependencies.stacks.get(alias),
            DependencyKind::Program => self.document.dependencies.programs.get(alias),
        }
    }

    pub fn dependencies(&self) -> impl Iterator<Item = (DependencyKind, &String, &DependencyV1)> {
        self.document
            .dependencies
            .stacks
            .iter()
            .map(|(alias, dependency)| (DependencyKind::Stack, alias, dependency))
            .chain(
                self.document
                    .dependencies
                    .programs
                    .iter()
                    .map(|(alias, dependency)| (DependencyKind::Program, alias, dependency)),
            )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestV1 {
    pub manifest_version: u32,
    pub project: ProjectV1,
    #[serde(default)]
    pub install: InstallV1,
    #[serde(default)]
    pub sdk: SdkV1,
    #[serde(default)]
    pub dependencies: DependenciesV1,
    #[serde(default)]
    pub authoring: AuthoringV1,
}

impl ManifestV1 {
    pub fn new(project_name: String) -> Self {
        Self {
            manifest_version: MANIFEST_VERSION,
            project: ProjectV1 {
                name: project_name,
                private: false,
            },
            install: InstallV1::default(),
            sdk: SdkV1::default(),
            dependencies: DependenciesV1::default(),
            authoring: AuthoringV1::default(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.manifest_version != MANIFEST_VERSION {
            bail!("manifest_version must be {MANIFEST_VERSION}");
        }
        if self.project.name.trim().is_empty() {
            bail!("project.name cannot be empty");
        }
        validate_targets(&self.sdk.targets, "sdk.targets", true)?;

        for (kind, entries) in [
            (DependencyKind::Stack, &self.dependencies.stacks),
            (DependencyKind::Program, &self.dependencies.programs),
        ] {
            for (alias, dependency) in entries {
                validate_alias(alias, "dependency alias")?;
                dependency.validate(kind, alias, &self.sdk.targets, &self.authoring)?;
            }
        }

        for (name, stack) in &self.authoring.stacks {
            validate_alias(name, "authoring stack name")?;
            validate_relative_artifact_path(&stack.manifest, ArtifactPathKind::StackManifest)?;
            validate_artifact_roots(&stack.artifact_roots, name)?;
            if stack
                .deployment_name
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            {
                bail!("authoring stack '{name}' has an empty deployment_name");
            }
        }
        for (name, program) in &self.authoring.programs {
            validate_alias(name, "authoring program name")?;
            validate_relative_artifact_path(&program.program_spec, ArtifactPathKind::ProgramSpec)?;
        }
        Ok(())
    }

    pub fn resolution_hash(&self) -> Result<String> {
        let bytes = serde_json::to_vec(self)?;
        let digest = Sha256::digest(bytes);
        Ok(format!("arete-manifest-v1:{digest:x}"))
    }

    pub fn to_toml_pretty(&self) -> Result<String> {
        let mut value = toml::Value::try_from(self).context("Failed to encode project manifest")?;
        compact_manifest_value(&mut value)?;
        toml::to_string_pretty(&value).context("Failed to serialize project manifest")
    }
}

fn compact_manifest_value(value: &mut toml::Value) -> Result<()> {
    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("Project manifest did not encode as a TOML table"))?;

    if let Some(project) = root.get_mut("project").and_then(toml::Value::as_table_mut) {
        if project.get("private").and_then(toml::Value::as_bool) == Some(false) {
            project.remove("private");
        }
    }

    let default_install = toml::Value::try_from(InstallV1::default())?;
    if root.get("install") == Some(&default_install) {
        root.remove("install");
    }

    let default_sdk = toml::Value::try_from(SdkV1::default())?;
    let default_sdk = default_sdk
        .as_table()
        .ok_or_else(|| anyhow::anyhow!("Default SDK configuration did not encode as a table"))?;
    let remove_sdk = if let Some(sdk) = root.get_mut("sdk").and_then(toml::Value::as_table_mut) {
        if sdk.get("targets") == default_sdk.get("targets") {
            sdk.remove("targets");
        }
        for language in ["typescript", "rust", "python"] {
            let remove_language = if let (Some(configuration), Some(default_configuration)) = (
                sdk.get_mut(language).and_then(toml::Value::as_table_mut),
                default_sdk.get(language).and_then(toml::Value::as_table),
            ) {
                for (key, default_value) in default_configuration {
                    if configuration.get(key) == Some(default_value) {
                        configuration.remove(key);
                    }
                }
                configuration.is_empty()
            } else {
                false
            };
            if remove_language {
                sdk.remove(language);
            }
        }
        sdk.is_empty()
    } else {
        false
    };
    if remove_sdk {
        root.remove("sdk");
    }

    let remove_dependencies = if let Some(dependencies) = root
        .get_mut("dependencies")
        .and_then(toml::Value::as_table_mut)
    {
        for kind in ["stacks", "programs"] {
            let remove_kind = if let Some(entries) = dependencies
                .get_mut(kind)
                .and_then(toml::Value::as_table_mut)
            {
                for dependency in entries
                    .iter_mut()
                    .filter_map(|(_, value)| value.as_table_mut())
                {
                    if dependency
                        .get("outputs")
                        .and_then(toml::Value::as_table)
                        .is_some_and(toml::map::Map::is_empty)
                    {
                        dependency.remove("outputs");
                    }
                }
                entries.is_empty()
            } else {
                false
            };
            if remove_kind {
                dependencies.remove(kind);
            }
        }
        dependencies.is_empty()
    } else {
        false
    };
    if remove_dependencies {
        root.remove("dependencies");
    }

    let remove_authoring = if let Some(authoring) = root
        .get_mut("authoring")
        .and_then(toml::Value::as_table_mut)
    {
        for kind in ["stacks", "programs"] {
            if authoring
                .get(kind)
                .and_then(toml::Value::as_table)
                .is_some_and(toml::map::Map::is_empty)
            {
                authoring.remove(kind);
            }
        }
        authoring.is_empty()
    } else {
        false
    };
    if remove_authoring {
        root.remove("authoring");
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectV1 {
    pub name: String,
    #[serde(default)]
    pub private: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallV1 {
    #[serde(default)]
    pub allow_outside_project: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SdkV1 {
    #[serde(default = "default_targets")]
    pub targets: Vec<InstallTarget>,
    #[serde(default)]
    pub typescript: TypeScriptSdkV1,
    #[serde(default)]
    pub rust: RustSdkV1,
    #[serde(default)]
    pub python: PythonSdkV1,
}

impl Default for SdkV1 {
    fn default() -> Self {
        Self {
            targets: default_targets(),
            typescript: TypeScriptSdkV1::default(),
            rust: RustSdkV1::default(),
            python: PythonSdkV1::default(),
        }
    }
}

fn default_targets() -> Vec<InstallTarget> {
    vec![InstallTarget::TypeScript, InstallTarget::Rust]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeScriptSdkV1 {
    #[serde(default = "default_typescript_output")]
    pub output_dir: String,
    #[serde(default = "default_typescript_package")]
    pub package: String,
}

impl Default for TypeScriptSdkV1 {
    fn default() -> Self {
        Self {
            output_dir: default_typescript_output(),
            package: default_typescript_package(),
        }
    }
}

fn default_typescript_output() -> String {
    "./generated/typescript".into()
}

fn default_typescript_package() -> String {
    "@usearete/react".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustSdkV1 {
    #[serde(default = "default_rust_output")]
    pub output_dir: String,
    #[serde(default)]
    pub module_mode: bool,
    #[serde(default)]
    pub crate_prefix: String,
}

impl Default for RustSdkV1 {
    fn default() -> Self {
        Self {
            output_dir: default_rust_output(),
            module_mode: false,
            crate_prefix: String::new(),
        }
    }
}

fn default_rust_output() -> String {
    "./generated/rust".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PythonSdkV1 {
    #[serde(default = "default_python_output")]
    pub output_dir: String,
    #[serde(default)]
    pub module_mode: bool,
    #[serde(default)]
    pub package_prefix: String,
}

impl Default for PythonSdkV1 {
    fn default() -> Self {
        Self {
            output_dir: default_python_output(),
            module_mode: false,
            package_prefix: String::new(),
        }
    }
}

fn default_python_output() -> String {
    "./generated/python".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependenciesV1 {
    #[serde(default)]
    pub stacks: BTreeMap<String, DependencyV1>,
    #[serde(default)]
    pub programs: BTreeMap<String, DependencyV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyV1 {
    pub source: DependencySourceV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<InstallTarget>>,
    #[serde(default)]
    pub outputs: DependencyOutputsV1,
}

impl DependencyV1 {
    fn validate(
        &self,
        kind: DependencyKind,
        alias: &str,
        supported_targets: &[InstallTarget],
        authoring: &AuthoringV1,
    ) -> Result<()> {
        match &self.source {
            DependencySourceV1::Registry(RegistrySourceV1 { registry }) => {
                validate_registry_package(registry)?;
                let requirement = self.version.as_deref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "registry dependency '{alias}' requires a semantic version requirement"
                    )
                })?;
                if matches!(requirement, "latest" | "stable" | "next") {
                    bail!("registry dependency '{alias}' cannot use mutable tag '{requirement}'");
                }
                VersionReq::parse(requirement).with_context(|| {
                    format!("dependency '{alias}' has invalid version requirement '{requirement}'")
                })?;
            }
            DependencySourceV1::Path(PathSourceV1 { path }) => {
                if self.version.is_some() {
                    bail!("path dependency '{alias}' cannot declare version");
                }
                validate_relative_artifact_path(
                    path,
                    match kind {
                        DependencyKind::Stack => ArtifactPathKind::StackManifest,
                        DependencyKind::Program => ArtifactPathKind::ProgramSpec,
                    },
                )?;
            }
            DependencySourceV1::Workspace(WorkspaceSourceV1 { workspace }) => {
                if self.version.is_some() {
                    bail!("workspace dependency '{alias}' cannot declare version");
                }
                validate_alias(workspace, "workspace source")?;
                let exists = match kind {
                    DependencyKind::Stack => authoring.stacks.contains_key(workspace),
                    DependencyKind::Program => authoring.programs.contains_key(workspace),
                };
                if !exists {
                    bail!(
                        "workspace dependency '{alias}' refers to missing same-kind authoring entry '{workspace}'"
                    );
                }
            }
        }

        let targets = self.targets.as_deref().unwrap_or(supported_targets);
        validate_targets(targets, &format!("dependency '{alias}' targets"), true)?;
        for target in targets {
            if !supported_targets.contains(target) {
                bail!(
                    "dependency '{alias}' requests target '{}' absent from sdk.targets",
                    target.as_str()
                );
            }
        }
        Ok(())
    }

    pub fn selected_targets<'a>(&'a self, sdk: &'a SdkV1) -> &'a [InstallTarget] {
        self.targets.as_deref().unwrap_or(&sdk.targets)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DependencySourceV1 {
    Registry(RegistrySourceV1),
    Path(PathSourceV1),
    Workspace(WorkspaceSourceV1),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrySourceV1 {
    pub registry: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathSourceV1 {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSourceV1 {
    pub workspace: String,
}

impl DependencySourceV1 {
    pub fn stable_description(&self) -> String {
        match self {
            Self::Registry(RegistrySourceV1 { registry }) => format!("registry:{registry}"),
            Self::Path(PathSourceV1 { path }) => format!("path:{}", normalize_relative(path)),
            Self::Workspace(WorkspaceSourceV1 { workspace }) => {
                format!("workspace:{workspace}")
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyOutputsV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typescript: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rust: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python: Option<String>,
}

impl DependencyOutputsV1 {
    pub fn get(&self, target: InstallTarget) -> Option<&str> {
        match target {
            InstallTarget::TypeScript => self.typescript.as_deref(),
            InstallTarget::Rust => self.rust.as_deref(),
            InstallTarget::Python => self.python.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringV1 {
    #[serde(default)]
    pub stacks: BTreeMap<String, AuthoringStackV1>,
    #[serde(default)]
    pub programs: BTreeMap<String, AuthoringProgramV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringStackV1 {
    pub manifest: String,
    #[serde(default)]
    pub artifact_roots: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringProgramV1 {
    pub program_spec: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallTarget {
    TypeScript,
    Rust,
    Python,
}

impl InstallTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Rust => "rust",
            Self::Python => "python",
        }
    }
}

impl std::fmt::Display for InstallTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyKind {
    Stack,
    Program,
}

impl DependencyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stack => "stack",
            Self::Program => "program",
        }
    }

    pub fn namespace(self) -> &'static str {
        match self {
            Self::Stack => "stacks",
            Self::Program => "programs",
        }
    }
}

impl std::fmt::Display for DependencyKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy)]
enum ArtifactPathKind {
    StackManifest,
    ProgramSpec,
}

fn validate_relative_artifact_path(path: &str, kind: ArtifactPathKind) -> Result<()> {
    let parsed = Path::new(path);
    if path.trim().is_empty() || parsed.is_absolute() {
        bail!("artifact path '{path}' must be a non-empty manifest-relative path");
    }
    if parsed
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("artifact path '{path}' cannot contain parent traversal");
    }
    if path.ends_with(".stack.json") {
        bail!(
            "'{path}' uses the removed .stack.json format; use an exact StackManifest or ProgramSpec artifact"
        );
    }
    let expected = match kind {
        ArtifactPathKind::StackManifest => ".stack-manifest.json",
        ArtifactPathKind::ProgramSpec => ".program-spec.json",
    };
    if !path.ends_with(expected) {
        bail!("artifact path '{path}' must end with '{expected}'");
    }
    Ok(())
}

fn validate_artifact_roots(roots: &[String], authoring_name: &str) -> Result<()> {
    let mut unique = BTreeSet::new();
    for root in roots {
        let parsed = Path::new(root);
        if root.trim().is_empty()
            || parsed.is_absolute()
            || parsed
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            bail!(
                "authoring stack '{authoring_name}' artifact root '{root}' must be manifest-relative without parent traversal"
            );
        }
        if !unique.insert(normalize_relative(root)) {
            bail!("authoring stack '{authoring_name}' repeats artifact root '{root}'");
        }
    }
    Ok(())
}

fn validate_targets(targets: &[InstallTarget], field: &str, require_non_empty: bool) -> Result<()> {
    if require_non_empty && targets.is_empty() {
        bail!("{field} cannot be empty");
    }
    let mut unique = BTreeSet::new();
    for target in targets {
        if !unique.insert(*target) {
            bail!("{field} repeats target '{target}'");
        }
    }
    Ok(())
}

fn validate_alias(alias: &str, kind: &str) -> Result<()> {
    let valid = !alias.is_empty()
        && alias.len() <= 64
        && alias
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && alias.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        });
    if !valid {
        bail!(
            "{kind} '{alias}' must be 1-64 lowercase ASCII letters, digits, '-' or '_', starting with a letter or digit"
        );
    }
    Ok(())
}

fn validate_registry_package(package: &str) -> Result<()> {
    if package.is_empty()
        || package.len() > 128
        || !package
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        bail!("registry package '{package}' is not a portable package name");
    }
    Ok(())
}

fn normalize_relative(path: &str) -> String {
    Path::new(path)
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Result<ManifestV1> {
        let manifest: ManifestV1 = toml::from_str(source)?;
        manifest.validate()?;
        Ok(manifest)
    }

    #[test]
    fn strict_manifest_accepts_registry_path_and_workspace_sources() {
        let manifest = parse(
            r#"
manifest_version = 1
[project]
name = "example"
[sdk]
targets = ["typescript", "rust"]
[dependencies.stacks.ore]
source = { registry = "ore" }
version = "^1.4"
[dependencies.stacks.local]
source = { workspace = "local-stack" }
[dependencies.programs.token]
source = { path = "./artifacts/token.program-spec.json" }
targets = ["typescript"]
[authoring.stacks.local-stack]
manifest = "./.arete/Local.stack-manifest.json"
artifact_roots = ["./.arete"]
"#,
        )
        .expect("manifest should validate");
        assert_eq!(manifest.dependencies.stacks.len(), 2);
    }

    #[test]
    fn strict_manifest_rejects_unknown_fields_and_accepts_cross_kind_aliases() {
        let unknown = toml::from_str::<ManifestV1>(
            r#"
manifest_version = 1
[project]
name = "example"
typo = true
"#,
        );
        assert!(unknown.is_err());

        let same_alias = parse(
            r#"
manifest_version = 1
[project]
name = "example"
[dependencies.stacks.same]
source = { registry = "one" }
version = "1"
[dependencies.programs.same]
source = { registry = "two" }
version = "1"
"#,
        )
        .expect("stack and program aliases occupy separate namespaces");
        assert!(same_alias.dependencies.stacks.contains_key("same"));
        assert!(same_alias.dependencies.programs.contains_key("same"));
    }

    #[test]
    fn source_union_and_legacy_suffix_fail_closed() {
        let ambiguous = parse(
            r#"
manifest_version = 1
[project]
name = "example"
[dependencies.stacks.bad]
source = { registry = "ore", path = "./ore.stack-manifest.json" }
version = "1"
"#,
        );
        assert!(ambiguous.is_err());

        let legacy = parse(
            r#"
manifest_version = 1
[project]
name = "example"
[dependencies.stacks.bad]
source = { path = "./ore.stack.json" }
"#,
        );
        assert!(legacy
            .unwrap_err()
            .to_string()
            .contains("removed .stack.json"));
    }

    #[test]
    fn sdk_targets_must_not_be_empty() {
        let empty = parse(
            r#"
manifest_version = 1
[project]
name = "example"
[sdk]
targets = []
"#,
        );

        assert!(empty
            .unwrap_err()
            .to_string()
            .contains("sdk.targets cannot be empty"));
    }

    #[test]
    fn resolution_hash_ignores_toml_formatting() {
        let compact = parse("manifest_version=1\n[project]\nname='same'\n").unwrap();
        let formatted = parse(
            r#"
                manifest_version = 1

                # Comments are not resolution input.
                [project]
                name = "same"
            "#,
        )
        .unwrap();
        assert_eq!(
            compact.resolution_hash().unwrap(),
            formatted.resolution_hash().unwrap()
        );
    }

    #[test]
    fn generated_toml_omits_semantically_empty_defaults() {
        let manifest = ManifestV1::new("clean-project".into());
        let rendered = manifest.to_toml_pretty().unwrap();

        assert!(rendered.contains("manifest_version = 1"));
        assert!(rendered.contains("name = \"clean-project\""));
        assert!(!rendered.contains("private = false"));
        assert!(!rendered.contains("[install]"));
        assert!(!rendered.contains("[sdk"));
        assert!(!rendered.contains("[dependencies"));
        assert!(!rendered.contains("[authoring"));

        let reparsed = parse(&rendered).unwrap();
        assert_eq!(
            manifest.resolution_hash().unwrap(),
            reparsed.resolution_hash().unwrap()
        );
    }
}
