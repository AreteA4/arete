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
            stack.validate(name)?;
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
    /// Where the generated SDK reads a registry stack, per LiveSpec alias,
    /// instead of the endpoints the stack's delivery provides: the user's own
    /// deployment, which `a4 up <alias>` records.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub endpoints: BTreeMap<String, StackEndpointsV1>,
}

/// One LiveSpec's stream endpoints in a user's own deployment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StackEndpointsV1 {
    pub websocket: String,
    pub query: String,
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

        if !self.endpoints.is_empty() {
            // A registry stack, or a composed stack of this project: both
            // generate from exact LiveSpecs a deployment of them serves.
            let composed_workspace = matches!(
                &self.source,
                DependencySourceV1::Workspace(WorkspaceSourceV1 { workspace })
                    if authoring.stacks.get(workspace).is_some_and(AuthoringStackV1::is_composed)
            );
            if kind != DependencyKind::Stack
                || !(matches!(self.source, DependencySourceV1::Registry(_)) || composed_workspace)
            {
                bail!(
                    "dependency '{alias}' declares endpoints; only a registry stack or a composed [authoring.stacks] entry can"
                );
            }
            for (live, endpoints) in &self.endpoints {
                validate_live_alias(live, alias)?;
                validate_endpoint(
                    &endpoints.websocket,
                    &["ws", "wss"],
                    &format!("dependency '{alias}' endpoints.{live}.websocket"),
                )?;
                validate_endpoint(
                    &endpoints.query,
                    &["http", "https"],
                    &format!("dependency '{alias}' endpoints.{live}.query"),
                )?;
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

/// One `[authoring.stacks.<name>]` entry: either a prebuilt StackManifest
/// (`manifest`, with `artifact_roots`) or a composition of parts (`live` and
/// `programs`), never both.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringStackV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<String>,
    #[serde(default)]
    pub artifact_roots: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_name: Option<String>,
    /// Composed live views, by the live alias they take in the composed
    /// stack. Empty (and not serialized) for a prebuilt StackManifest, so its
    /// resolution hash is unchanged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub live: BTreeMap<String, ComposedLiveV1>,
    /// Composed program SDKs, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub programs: Vec<ComposedProgramV1>,
}

impl AuthoringStackV1 {
    /// A prebuilt StackManifest file.
    pub fn prebuilt(manifest: String, artifact_roots: Vec<String>) -> Self {
        Self {
            manifest: Some(manifest),
            artifact_roots,
            deployment_name: None,
            live: BTreeMap::new(),
            programs: Vec::new(),
        }
    }

    /// Whether this entry composes parts instead of naming a StackManifest.
    pub fn is_composed(&self) -> bool {
        self.manifest.is_none()
    }

    /// The StackManifest path of a prebuilt entry.
    pub fn manifest_path(&self, name: &str) -> Result<&str> {
        self.manifest.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "authoring stack '{name}' composes parts and has no StackManifest file; `a4 install` composes it"
            )
        })
    }

    fn validate(&self, name: &str) -> Result<()> {
        if let Some(manifest) = &self.manifest {
            if !self.live.is_empty() || !self.programs.is_empty() {
                bail!(
                    "authoring stack '{name}' sets both `manifest` and composed parts (`live`, `programs`); use one or the other"
                );
            }
            validate_relative_artifact_path(manifest, ArtifactPathKind::StackManifest)?;
            validate_artifact_roots(&self.artifact_roots, name)?;
            return Ok(());
        }
        if !self.artifact_roots.is_empty() {
            bail!(
                "authoring stack '{name}' sets artifact_roots without a `manifest`; a composed stack names its parts' files directly"
            );
        }
        if self.live.is_empty() {
            bail!(
                "authoring stack '{name}' needs a `manifest`, or composed `live` views (with optional `programs`)"
            );
        }
        let mut sources = BTreeSet::new();
        for (alias, live) in &self.live {
            validate_composed_live_alias(alias, name)?;
            live.validate(name, alias)?;
            if let Some(path) = &live.path {
                if !sources.insert(format!("path:{}", normalize_relative(path))) {
                    bail!(
                        "authoring stack '{name}' composes LiveSpec file '{path}' more than once"
                    );
                }
            }
        }
        let mut packages = BTreeSet::new();
        let mut program_paths = BTreeSet::new();
        for program in &self.programs {
            program.validate(name)?;
            if let Some(package) = &program.package {
                if !packages.insert(package.to_ascii_lowercase()) {
                    bail!(
                        "authoring stack '{name}' lists program package '{package}' more than once"
                    );
                }
            }
            if let Some(path) = &program.path {
                if !program_paths.insert(normalize_relative(path)) {
                    bail!(
                        "authoring stack '{name}' lists ProgramSpec file '{path}' more than once"
                    );
                }
            }
        }
        Ok(())
    }
}

/// One composed live alias: views from a published stack (`stack`, with an
/// optional `version`, `live_alias` and `views`), or from a local LiveSpec
/// file (`path`, with optional `views`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComposedLiveV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The source stack's live alias, required when it has more than one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The selected views, in order. Absent selects every view the source
    /// stack selects (or, for a LiveSpec file, every view it defines).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub views: Option<Vec<String>>,
}

/// Where a composed live alias comes from.
pub enum ComposedLiveSource<'a> {
    Registry {
        stack: &'a str,
        requirement: &'a str,
        live_alias: Option<&'a str>,
    },
    Path(&'a str),
}

impl ComposedLiveV1 {
    pub fn source(&self) -> ComposedLiveSource<'_> {
        match (&self.stack, &self.path) {
            (Some(stack), _) => ComposedLiveSource::Registry {
                stack,
                requirement: self.version.as_deref().unwrap_or("*"),
                live_alias: self.live_alias.as_deref(),
            },
            (None, Some(path)) => ComposedLiveSource::Path(path),
            (None, None) => unreachable!("validated composed live source"),
        }
    }

    fn validate(&self, name: &str, alias: &str) -> Result<()> {
        let field = format!("authoring stack '{name}' live.{alias}");
        match (&self.stack, &self.path) {
            (Some(stack), None) => {
                validate_registry_package(stack)?;
                if let Some(version) = &self.version {
                    validate_part_requirement(version, &field)?;
                }
                if let Some(live_alias) = &self.live_alias {
                    if !is_live_alias(live_alias) {
                        bail!("{field} live_alias '{live_alias}' is not a LiveSpec alias (1-64 ASCII letters, digits, '-' or '_')");
                    }
                }
            }
            (None, Some(path)) => {
                if self.version.is_some() || self.live_alias.is_some() {
                    bail!("{field} reads a LiveSpec file, so it cannot set `version` or `live_alias`");
                }
                validate_relative_artifact_path(path, ArtifactPathKind::LiveSpec)?;
            }
            (Some(_), Some(_)) => bail!("{field} sets both `stack` and `path`; use one"),
            (None, None) => bail!(
                "{field} needs a source: `stack = \"<package>\"` or `path = \"<file>.live-spec.json\"`"
            ),
        }
        if let Some(views) = &self.views {
            if views.is_empty() {
                bail!("{field} views cannot be empty; omit `views` to select every view");
            }
            let mut unique = BTreeSet::new();
            for view in views {
                if !is_view_id(view) {
                    bail!("{field} view '{view}' is not a view ID ('<Entity>/<view>')");
                }
                if !unique.insert(view.as_str()) {
                    bail!("{field} selects view '{view}' more than once");
                }
            }
        }
        Ok(())
    }
}

/// One composed program SDK: a program package (`package`, with an optional
/// `version`) or a local ProgramSpec file (`path`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComposedProgramV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Where a composed program comes from.
pub enum ComposedProgramSource<'a> {
    Registry {
        package: &'a str,
        requirement: &'a str,
    },
    Path(&'a str),
}

impl ComposedProgramV1 {
    pub fn source(&self) -> ComposedProgramSource<'_> {
        match (&self.package, &self.path) {
            (Some(package), _) => ComposedProgramSource::Registry {
                package,
                requirement: self.version.as_deref().unwrap_or("*"),
            },
            (None, Some(path)) => ComposedProgramSource::Path(path),
            (None, None) => unreachable!("validated composed program source"),
        }
    }

    fn validate(&self, name: &str) -> Result<()> {
        let field = format!("authoring stack '{name}' programs");
        match (&self.package, &self.path) {
            (Some(package), None) => {
                validate_registry_package(package)?;
                if let Some(version) = &self.version {
                    validate_part_requirement(version, &format!("{field} entry '{package}'"))?;
                }
            }
            (None, Some(path)) => {
                if self.version.is_some() {
                    bail!("{field} entry '{path}' reads a ProgramSpec file, so it cannot set `version`");
                }
                validate_relative_artifact_path(path, ArtifactPathKind::ProgramSpec)?;
            }
            (Some(_), Some(_)) => bail!("{field} entry sets both `package` and `path`; use one"),
            (None, None) => bail!(
                "{field} entry needs a source: `package = \"<package>\"` or `path = \"<file>.program-spec.json\"`"
            ),
        }
        Ok(())
    }
}

/// A composed part's version requirement: a semantic version range, never a
/// mutable tag.
fn validate_part_requirement(requirement: &str, field: &str) -> Result<()> {
    if matches!(requirement, "latest" | "stable" | "next") {
        bail!("{field} cannot use mutable tag '{requirement}'");
    }
    VersionReq::parse(requirement)
        .with_context(|| format!("{field} has invalid version requirement '{requirement}'"))?;
    Ok(())
}

/// A view ID's shape: `<Entity>/<view>`, both parts non-empty, no whitespace.
/// Whether the view exists is checked against its LiveSpec on install.
fn is_view_id(view: &str) -> bool {
    view.split_once('/')
        .is_some_and(|(entity, name)| !entity.is_empty() && !name.is_empty())
        && !view
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

/// A LiveSpec alias as a StackManifest spells it.
fn is_live_alias(live: &str) -> bool {
    !live.is_empty()
        && live.len() <= 64
        && live.bytes().any(|byte| byte.is_ascii_alphanumeric())
        && live
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// A composed stack's live alias: a StackManifest LiveSpec alias.
fn validate_composed_live_alias(alias: &str, name: &str) -> Result<()> {
    if !is_live_alias(alias) {
        bail!(
            "authoring stack '{name}' live alias '{alias}' is not a LiveSpec alias (1-64 ASCII letters, digits, '-' or '_')"
        );
    }
    Ok(())
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
    LiveSpec,
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
        ArtifactPathKind::LiveSpec => ".live-spec.json",
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

/// A LiveSpec alias as a StackManifest spells it (artifact rules, which allow
/// upper case), used as an `endpoints` key.
fn validate_live_alias(live: &str, dependency: &str) -> Result<()> {
    if !is_live_alias(live) {
        bail!(
            "dependency '{dependency}' endpoints key '{live}' is not a LiveSpec alias (1-64 ASCII letters, digits, '-' or '_')"
        );
    }
    Ok(())
}

/// An absolute endpoint URL with one of the allowed schemes and a host.
fn validate_endpoint(endpoint: &str, schemes: &[&str], field: &str) -> Result<()> {
    let url =
        url::Url::parse(endpoint).with_context(|| format!("{field} is not a URL: '{endpoint}'"))?;
    if !schemes.contains(&url.scheme()) || url.host_str().is_none_or(str::is_empty) {
        bail!(
            "{field} must be a {} URL with a host, not '{endpoint}'",
            schemes.join(" or ")
        );
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

    fn with_endpoints(dependency: &str, endpoints: &str) -> Result<ManifestV1> {
        parse(&format!(
            r#"
manifest_version = 1
[project]
name = "example"
[authoring.stacks.mine]
manifest = "./.arete/Mine.stack-manifest.json"
[dependencies.stacks.dep]
{dependency}
endpoints = {endpoints}
"#
        ))
    }

    #[test]
    fn a_registry_stack_records_its_deployment_endpoints_per_live_spec() {
        let manifest = with_endpoints(
            "source = { registry = \"ore\" }\nversion = \"^1.0.0\"",
            r#"{ live = { websocket = "wss://ore.stack.example", query = "https://ore.stack.example" }, Other_2 = { websocket = "ws://127.0.0.1:8878", query = "http://127.0.0.1:8878" } }"#,
        )
        .expect("endpoints validate");
        let endpoints = &manifest.dependencies.stacks["dep"].endpoints;
        assert_eq!(endpoints["live"].websocket, "wss://ore.stack.example");
        assert_eq!(endpoints["Other_2"].query, "http://127.0.0.1:8878");

        let written = manifest.to_toml_pretty().unwrap();
        let reparsed = parse(&written).unwrap();
        assert_eq!(&reparsed.dependencies.stacks["dep"].endpoints, endpoints);
    }

    #[test]
    fn a_manifest_without_endpoints_keeps_its_resolution_hash_shape() {
        let manifest = parse(
            "manifest_version = 1\n[project]\nname = \"example\"\n\
             [dependencies.stacks.ore]\nsource = { registry = \"ore\" }\nversion = \"^1.0.0\"\n",
        )
        .unwrap();
        let encoded = serde_json::to_string(&manifest).unwrap();
        assert!(!encoded.contains("endpoints"), "{encoded}");
    }

    #[test]
    fn endpoints_belong_to_registry_stacks_with_stream_urls() {
        let registry = "source = { registry = \"ore\" }\nversion = \"^1.0.0\"";
        let good = r#"{ live = { websocket = "wss://ore.stack.example", query = "https://ore.stack.example" } }"#;
        let cases = [
            (
                "source = { workspace = \"mine\" }",
                good,
                "only a registry stack",
            ),
            (
                registry,
                r#"{ live = { websocket = "https://ore.stack.example", query = "https://ore.stack.example" } }"#,
                "must be a ws or wss URL",
            ),
            (
                registry,
                r#"{ live = { websocket = "wss://ore.stack.example", query = "wss://ore.stack.example" } }"#,
                "must be a http or https URL",
            ),
            (
                registry,
                r#"{ live = { websocket = "not a url", query = "https://ore.stack.example" } }"#,
                "is not a URL",
            ),
            (
                registry,
                r#"{ "bad alias" = { websocket = "wss://ore.stack.example", query = "https://ore.stack.example" } }"#,
                "is not a LiveSpec alias",
            ),
        ];
        for (dependency, endpoints, expected) in cases {
            let error = format!("{:#}", with_endpoints(dependency, endpoints).unwrap_err());
            assert!(error.contains(expected), "{expected}: {error}");
        }
        let program = parse(&format!(
            "manifest_version = 1\n[project]\nname = \"example\"\n\
             [dependencies.programs.token]\nsource = {{ registry = \"spl-token\" }}\nversion = \"^1.0.0\"\n\
             endpoints = {good}\n"
        ))
        .unwrap_err();
        assert!(
            format!("{program:#}").contains("only a registry stack"),
            "{program:#}"
        );
        let unknown = with_endpoints(
            registry,
            r#"{ live = { websocket = "wss://ore.stack.example", query = "https://ore.stack.example", auth = "x" } }"#,
        )
        .unwrap_err();
        assert!(
            format!("{unknown:#}").contains("unknown field"),
            "{unknown:#}"
        );
    }

    fn composed(entry: &str) -> Result<ManifestV1> {
        parse(&format!(
            "manifest_version = 1\n[project]\nname = \"example\"\n\n[authoring.stacks.ore-plus-token]\n{entry}\n"
        ))
    }

    #[test]
    fn a_composed_stack_names_registry_parts_and_local_files() {
        let manifest = composed(
            r#"live.ore = { stack = "ore", version = "^1", views = ["OreRound/latest", "OreMiner/state"] }
live.multi = { stack = "multi", live_alias = "beta" }
live.local = { path = "./artifacts/local.live-spec.json" }
programs = [{ package = "spl-token", version = "^4" }, { package = "memo" }, { path = "./artifacts/vote.program-spec.json" }]
deployment_name = "ore-plus-token-prod"

[dependencies.stacks.ore-plus-token]
source = { workspace = "ore-plus-token" }
targets = ["typescript"]
endpoints = { ore = { websocket = "wss://mine.example", query = "https://mine.example" }, multi = { websocket = "wss://mine.example", query = "https://mine.example" }, local = { websocket = "wss://mine.example", query = "https://mine.example" } }"#,
        )
        .expect("a composed stack validates");
        let entry = &manifest.authoring.stacks["ore-plus-token"];
        assert!(entry.is_composed());
        assert_eq!(entry.live.len(), 3);
        assert!(matches!(
            entry.live["ore"].source(),
            ComposedLiveSource::Registry {
                stack: "ore",
                requirement: "^1",
                live_alias: None
            }
        ));
        assert!(matches!(
            entry.live["multi"].source(),
            ComposedLiveSource::Registry {
                requirement: "*",
                live_alias: Some("beta"),
                ..
            }
        ));
        assert!(matches!(
            entry.programs[1].source(),
            ComposedProgramSource::Registry {
                package: "memo",
                requirement: "*"
            }
        ));
        assert!(matches!(
            entry.programs[2].source(),
            ComposedProgramSource::Path("./artifacts/vote.program-spec.json")
        ));
        assert_eq!(
            entry.manifest_path("ore-plus-token").unwrap_err().to_string(),
            "authoring stack 'ore-plus-token' composes parts and has no StackManifest file; `a4 install` composes it"
        );
        // The composed parts round-trip through arete.toml.
        let reparsed = parse(&manifest.to_toml_pretty().unwrap()).unwrap();
        assert_eq!(
            reparsed.resolution_hash().unwrap(),
            manifest.resolution_hash().unwrap()
        );
    }

    #[test]
    fn a_prebuilt_stack_manifest_entry_keeps_its_resolution_hash_shape() {
        let manifest = parse(
            "manifest_version = 1\n[project]\nname = \"example\"\n\
             [authoring.stacks.local]\nmanifest = \"./.arete/Local.stack-manifest.json\"\nartifact_roots = [\"./.arete\"]\n",
        )
        .unwrap();
        let entry = &manifest.authoring.stacks["local"];
        assert!(!entry.is_composed());
        assert_eq!(
            serde_json::to_value(entry).unwrap(),
            serde_json::json!({
                "manifest": "./.arete/Local.stack-manifest.json",
                "artifact_roots": ["./.arete"],
            })
        );
    }

    #[test]
    fn composed_parts_are_validated() {
        let cases = [
            (
                "manifest = \"./Mine.stack-manifest.json\"\nlive.ore = { stack = \"ore\" }",
                "sets both `manifest` and composed parts",
            ),
            (
                "programs = [{ package = \"spl-token\" }]",
                "composed `live` views",
            ),
            (
                "artifact_roots = [\"./.arete\"]\nlive.ore = { stack = \"ore\" }",
                "artifact_roots without a `manifest`",
            ),
            (
                "live.ore = { stack = \"ore\", path = \"./ore.live-spec.json\" }",
                "sets both `stack` and `path`",
            ),
            ("live.ore = { views = [\"OreRound/latest\"] }", "needs a source"),
            (
                "live.ore = { path = \"./ore.live-spec.json\", version = \"^1\" }",
                "cannot set `version` or `live_alias`",
            ),
            (
                "live.ore = { path = \"./ore.program-spec.json\" }",
                "must end with '.live-spec.json'",
            ),
            (
                "live.ore = { path = \"../ore.live-spec.json\" }",
                "parent traversal",
            ),
            (
                "live.\"bad alias\" = { stack = \"ore\" }",
                "is not a LiveSpec alias",
            ),
            (
                "live.ore = { stack = \"ore\", live_alias = \"not an alias\" }",
                "live_alias 'not an alias' is not a LiveSpec alias",
            ),
            ("live.ore = { stack = \"ore\", views = [] }", "views cannot be empty"),
            (
                "live.ore = { stack = \"ore\", views = [\"OreRound/latest\", \"OreRound/latest\"] }",
                "selects view 'OreRound/latest' more than once",
            ),
            (
                "live.ore = { stack = \"ore\", views = [\"OreRound\"] }",
                "is not a view ID",
            ),
            (
                "live.ore = { stack = \"ore\", version = \"latest\" }",
                "mutable tag 'latest'",
            ),
            (
                "live.ore = { stack = \"ore\", version = \"not-a-range\" }",
                "invalid version requirement",
            ),
            (
                "live.ore = { stack = \"ore\" }\nprograms = [{ package = \"spl-token\" }, { package = \"SPL-token\" }]",
                "lists program package 'SPL-token' more than once",
            ),
            (
                "live.ore = { stack = \"ore\" }\nprograms = [{ package = \"spl-token\", path = \"./t.program-spec.json\" }]",
                "sets both `package` and `path`",
            ),
            (
                "live.ore = { stack = \"ore\" }\nprograms = [{ path = \"./t.program-spec.json\", version = \"^1\" }]",
                "cannot set `version`",
            ),
            (
                "live.a = { path = \"./x.live-spec.json\" }\nlive.b = { path = \"x.live-spec.json\" }",
                "composes LiveSpec file 'x.live-spec.json' more than once",
            ),
            (
                "live.ore = { stack = \"ore\", unknown = true }",
                "unknown field",
            ),
        ];
        for (entry, expected) in cases {
            let error = format!("{:#}", composed(entry).unwrap_err());
            assert!(error.contains(expected), "{entry}\n{expected}\n{error}");
        }
    }

    #[test]
    fn endpoints_belong_to_composed_but_not_prebuilt_workspace_stacks() {
        let endpoints = r#"endpoints = { ore = { websocket = "wss://mine.example", query = "https://mine.example" } }"#;
        let composed_endpoints = parse(&format!(
            "manifest_version = 1\n[project]\nname = \"example\"\n\
             [authoring.stacks.mix]\nlive.ore = {{ stack = \"ore\" }}\n\
             [dependencies.stacks.mix]\nsource = {{ workspace = \"mix\" }}\n{endpoints}\n"
        ));
        assert!(composed_endpoints.is_ok(), "{composed_endpoints:?}");
        let prebuilt = parse(&format!(
            "manifest_version = 1\n[project]\nname = \"example\"\n\
             [authoring.stacks.mine]\nmanifest = \"./Mine.stack-manifest.json\"\n\
             [dependencies.stacks.mine]\nsource = {{ workspace = \"mine\" }}\n{endpoints}\n"
        ))
        .unwrap_err();
        assert!(
            format!("{prebuilt:#}").contains("only a registry stack or a composed"),
            "{prebuilt:#}"
        );
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
