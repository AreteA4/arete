//! The installed client of this project's protocol, from the local build.
//!
//! `a4 dev up` (after every deploy that changes the release) and `a4 dev sdk`
//! (on demand) generate the protocol client where `a4 install` puts it,
//! `generated/<lang>/protocols/<alias>/`, from the local build's
//! `protocol-spec.json`, so the app's types follow the local spec. The lock
//! entry records `local_build = "<release hash>"`; `a4 install --locked`
//! restores the published client and `a4 install --check` fails while a
//! local build is installed.
//!
//! The descriptor names the deployment `a4 dev up` keeps in the project's
//! cloud dev environment, so the app's calls (with the account's key) go to
//! the environment. `a4 install --locked` puts the published client, and
//! with it the production deployment, back.
//!
//! Before the first publish arete.toml does not install the protocol, so
//! `[dependencies.protocols.<slug>]` (`source = { registry = "<slug>" }`) is
//! added, exactly as `a4 install protocol <slug>` would write it, with the
//! `[dev] sdk` targets (TypeScript by default). A local build hoists no
//! registry stacks or programs: the client carries the protocol module
//! only until the published release is installed.

use std::path::Path;

use anyhow::{bail, Context, Result};
use arete_mcp::dev::DevProject;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::deploy::{read_build, DeployState, BUILD_DIR};
use crate::project::installer::{
    ensure_own_protocol_dependency, install_local_protocol_build, LocalBuildDeployment,
    LocalProtocolBuild, LocalProtocolInstall,
};
use crate::project::manifest::InstallTarget;

/// What the client is generated from.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalProtocol {
    /// The build's `protocol-spec.json`.
    pub spec_json: Value,
    /// The protocol slug: the registry package the client installs as.
    pub slug: String,
    pub release_hash: String,
    /// The deployment in the dev environment.
    pub deployment_id: String,
}

impl LocalProtocol {
    /// From the build in `.arete/protocol` and the deployment `a4 dev up`
    /// recorded in `.arete/dev/deploy.json`.
    pub fn from_project(root: &Path) -> Result<Self> {
        let project = DevProject::new(root);
        let state = DeployState::load(&project.deploy_state_path());
        let (Some(slug), Some(release_hash), Some(deployment_id)) =
            (state.slug, state.release_hash, state.deployment_id)
        else {
            bail!("No deployment in the dev environment yet; `a4 dev up` deploys the protocol");
        };
        let build = read_build(&root.join(BUILD_DIR))?;
        Ok(Self {
            spec_json: build.spec_json,
            slug,
            release_hash,
            deployment_id,
        })
    }

    fn version(&self) -> Result<String> {
        self.spec_json["version"]
            .as_str()
            .map(str::to_string)
            .context("protocol-spec.json has no version")
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DevConfig {
    #[serde(default)]
    sdk: Vec<InstallTarget>,
    /// Stack Source files to run besides `stacks/` (read by
    /// [`super::custom_stacks::detect`]).
    #[serde(default)]
    #[serde(rename = "stacks")]
    _stacks: Vec<String>,
    /// The automations `a4 dev up` runs (read by [`super::automations`]).
    #[serde(default)]
    #[serde(rename = "automations")]
    _automations: Option<toml::Value>,
}

/// `[dev] sdk` from `<root>/arete.toml`: the targets of the client `a4 dev`
/// adds when the project does not install its protocol yet. Absent file,
/// table or key means TypeScript.
pub fn configured_targets(root: &Path) -> Result<Vec<InstallTarget>> {
    let path = root.join("arete.toml");
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read {}", path.display()))
        }
    };
    parse_targets(&source).with_context(|| format!("Invalid {}", path.display()))
}

fn parse_targets(source: &str) -> Result<Vec<InstallTarget>> {
    let value: toml::Value = toml::from_str(source).context("Failed to parse TOML")?;
    let Some(table) = value.get("dev") else {
        return Ok(Vec::new());
    };
    let config: DevConfig = table
        .clone()
        .try_into()
        .context("Invalid [dev] section (keys: sdk = [\"typescript\", \"rust\", \"python\"], stacks = [\"<path>.stack.yaml\"], [dev.automations])")?;
    let mut targets = config.sdk;
    targets.sort();
    targets.dedup();
    Ok(targets)
}

/// What [`sync_installed_client`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    #[serde(flatten)]
    pub install: LocalProtocolInstall,
    /// Whether arete.toml gained the protocol dependency.
    pub added_dependency: bool,
}

impl SyncReport {
    /// One line for the console.
    pub fn summary(&self) -> String {
        let outputs: Vec<String> = self
            .install
            .outputs
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        format!(
            "{} the {} client from the local build in {}; it calls deployment {} in the dev environment",
            if self.install.changed {
                "Generated"
            } else {
                "Already current:"
            },
            self.install.alias,
            outputs.join(", "),
            self.install.deployment_id
        )
    }
}

/// Generate this protocol's installed client from the local build (adding
/// the dependency to arete.toml when the project does not install it yet),
/// calling the deployment in the dev environment.
pub fn sync_installed_client(
    root: &Path,
    protocol: &LocalProtocol,
    dev_targets: &[InstallTarget],
    api_base: &str,
) -> Result<SyncReport> {
    let ensured =
        ensure_own_protocol_dependency(root, &protocol.slug, &protocol.version()?, dev_targets)?;
    let build = LocalProtocolBuild {
        spec_json: protocol.spec_json.clone(),
        release_hash: protocol.release_hash.clone(),
        deployment: LocalBuildDeployment::Deployed {
            id: protocol.deployment_id.clone(),
            access: "private".into(),
        },
        api_base: api_base.to_string(),
    };
    let install = install_local_protocol_build(
        &root.join("arete.toml"),
        &ensured.alias,
        &build,
        ensured.lock_was_fresh,
    )?;
    Ok(SyncReport {
        install,
        added_dependency: ensured.added,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectLock;
    use std::collections::BTreeMap;

    use crate::commands::sdk::stack_name_golden::{
        assert_syntax, compare_with_golden, without_release_version,
    };
    use crate::project::installer::check_project;
    use crate::project::manifest::{DependencyKind, ManifestV1};
    use crate::project::{InstallPlan, ProjectManifest};

    const LOCAL_RELEASE: &str =
        "arete:h1:protocol-release:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn spec() -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/dev/local-tip-jar/protocol-spec.json");
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn build(release_hash: &str, local_id: &str) -> LocalProtocolBuild {
        LocalProtocolBuild {
            spec_json: spec(),
            release_hash: release_hash.into(),
            deployment: LocalBuildDeployment::Deployed {
                id: local_id.into(),
                access: "private".into(),
            },
            api_base: "https://api.example.test".into(),
        }
    }

    fn project(targets: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut manifest = ManifestV1::new("app".into()).to_toml_pretty().unwrap();
        if !targets.is_empty() {
            manifest.push_str(&format!(
                "\n[dev]\nsdk = [{}]\n",
                targets
                    .iter()
                    .map(|target| format!("\"{target}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        std::fs::write(dir.path().join("arete.toml"), manifest).unwrap();
        dir
    }

    fn lock(root: &Path) -> ProjectLock {
        ProjectLock::load(root.join("arete.lock")).unwrap()
    }

    #[test]
    fn reads_dev_sdk_targets() {
        assert!(parse_targets("[project]\nname = \"x\"\n")
            .unwrap()
            .is_empty());
        assert_eq!(
            parse_targets("[dev]\nsdk = [\"rust\", \"typescript\", \"rust\"]\n").unwrap(),
            vec![InstallTarget::TypeScript, InstallTarget::Rust]
        );
        assert!(parse_targets("[dev]\nsdk = [\"go\"]\n").is_err());
        assert_eq!(
            parse_targets("[dev]\nstacks = [\"extra/a.stack.yaml\"]\n").unwrap(),
            Vec::<InstallTarget>::new()
        );
        assert!(parse_targets("[dev]\nsdks = []\n").is_err());
    }

    /// Before the first deploy: the dependency is added as `a4 install
    /// protocol` would write it, the client lands in the installed location
    /// with the local deployment, the lock records the local build, and the
    /// check refuses it.
    #[test]
    fn a_never_published_protocol_gets_a_dependency_and_a_local_client() {
        let dir = project(&[]);
        let root = dir.path();
        let ensured = ensure_own_protocol_dependency(root, "local-tip-jar", "0.1.0", &[]).unwrap();
        assert!(ensured.added && ensured.lock_was_fresh);
        let toml = std::fs::read_to_string(root.join("arete.toml")).unwrap();
        assert!(
            toml.contains("[dependencies.protocols.local-tip-jar]")
                && toml.contains("source = { registry = \"local-tip-jar\" }")
                && toml.contains("version = \"^0.1.0\""),
            "{toml}"
        );
        // Asking again finds it.
        let again = ensure_own_protocol_dependency(root, "local-tip-jar", "0.1.0", &[]).unwrap();
        assert!(!again.added && again.alias == "local-tip-jar");

        let install = install_local_protocol_build(
            &root.join("arete.toml"),
            "local-tip-jar",
            &build(LOCAL_RELEASE, "dep-local-1"),
            ensured.lock_was_fresh,
        )
        .unwrap();
        assert!(install.changed && !install.published);
        assert_eq!(install.deployment_id, "dep-local-1");
        let manifest = ProjectManifest::load(root.join("arete.toml")).unwrap();
        let plan = InstallPlan::build(&manifest, false).unwrap();
        let output = plan
            .for_dependency(DependencyKind::Protocol, "local-tip-jar")
            .next()
            .unwrap();
        assert!(output
            .path
            .ends_with("generated/typescript/protocols/local-tip-jar"));
        assert_eq!(install.outputs, vec![output.path.clone()]);

        let lock = lock(root);
        assert!(lock.is_fresh(&manifest.manifest_hash));
        let entry = &lock.dependencies[0];
        assert_eq!(entry.local_build.as_deref(), Some(LOCAL_RELEASE));
        assert!(!entry.is_published());
        assert_eq!(
            entry.protocol.as_ref().unwrap().deployment_id,
            "dep-local-1"
        );
        let provenance: Value = serde_json::from_slice(
            &std::fs::read(output.path.join("sdk-provenance.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(provenance["project"]["localBuild"], LOCAL_RELEASE);
        assert_eq!(provenance["project"]["packageReleaseHash"], Value::Null);
        assert_eq!(
            provenance["project"]["localBuildDeploymentId"],
            "dep-local-1"
        );
        // The descriptor marks the dev environment's deployment, which
        // automations use as their target.
        let module = std::fs::read_to_string(output.path.join("local-tip-jar.ts")).unwrap();
        assert!(
            module.contains("deploymentId: 'dep-local-1', access: 'private', apiUrl:")
                && module.contains(", environment: 'dev' } as ProtocolDefinition['deployment'],"),
            "{module}"
        );

        // The same build again changes nothing; a redeploy (a new local
        // deployment of a never-published protocol) regenerates.
        let same = install_local_protocol_build(
            &root.join("arete.toml"),
            "local-tip-jar",
            &build(LOCAL_RELEASE, "dep-local-1"),
            true,
        )
        .unwrap();
        assert!(!same.changed);
        let redeployed = install_local_protocol_build(
            &root.join("arete.toml"),
            "local-tip-jar",
            &build(LOCAL_RELEASE, "dep-local-2"),
            true,
        )
        .unwrap();
        assert!(redeployed.changed);
        assert_eq!(redeployed.deployment_id, "dep-local-2");

        // arete.lock names another deployment than the generated files (the
        // lock was written elsewhere): the same build is generated again.
        let mut edited = ProjectLock::load(root.join("arete.lock")).unwrap();
        edited.dependencies[0]
            .protocol
            .as_mut()
            .unwrap()
            .deployment_id = "33333333-3333-4333-8333-333333333333".into();
        edited.write_atomic(root.join("arete.lock")).unwrap();
        let regenerated = install_local_protocol_build(
            &root.join("arete.toml"),
            "local-tip-jar",
            &build(LOCAL_RELEASE, "dep-local-3"),
            true,
        )
        .unwrap();
        assert!(regenerated.changed);
        let module =
            std::fs::read_to_string(regenerated.outputs[0].join("local-tip-jar.ts")).unwrap();
        assert!(module.contains("dep-local-3"));

        let problems = check_project(root.join("arete.toml")).unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].problem.contains("never published"));
    }

    /// After a publish the lock names a production deployment: a local build
    /// keeps the published pins for `a4 install --locked`, and its client
    /// calls the deployment in the dev environment.
    #[test]
    fn a_published_protocol_keeps_its_pins_and_calls_the_dev_deployment() {
        let dir = project(&[]);
        let root = dir.path();
        let ensured = ensure_own_protocol_dependency(root, "local-tip-jar", "0.1.0", &[]).unwrap();
        install_local_protocol_build(
            &root.join("arete.toml"),
            "local-tip-jar",
            &build(LOCAL_RELEASE, "dep-local-1"),
            ensured.lock_was_fresh,
        )
        .unwrap();
        // What a registry install would have locked.
        let mut published = lock(root);
        let entry = &mut published.dependencies[0];
        entry.local_build = None;
        entry.version = Some("0.1.0".into());
        entry.package_release_hash = Some(format!(
            "arete:registry-package-release:v2:sha256:{}",
            "b".repeat(64)
        ));
        let protocol = entry.protocol.as_mut().unwrap();
        protocol.deployment_id = "11111111-1111-4111-8111-111111111111".into();
        published.write_atomic(root.join("arete.lock")).unwrap();

        let next = format!("arete:h1:protocol-release:sha256:{}", "c".repeat(64));
        let install = install_local_protocol_build(
            &root.join("arete.toml"),
            "local-tip-jar",
            &build(&next, "dep-local-3"),
            true,
        )
        .unwrap();
        assert!(install.changed && install.published);
        assert_eq!(install.deployment_id, "dep-local-3");
        let entry = lock(root).dependencies[0].clone();
        assert_eq!(entry.local_build.as_deref(), Some(next.as_str()));
        assert_eq!(entry.version.as_deref(), Some("0.1.0"));
        assert_eq!(
            entry.protocol.as_ref().unwrap().deployment_id,
            "11111111-1111-4111-8111-111111111111"
        );
        let module = std::fs::read_dir(&install.outputs[0])
            .unwrap()
            .filter_map(|entry| std::fs::read_to_string(entry.unwrap().path()).ok())
            .collect::<String>();
        assert!(
            module.contains("dep-local-3") && module.contains("https://api.example.test/api"),
            "{module}"
        );
        let problems = check_project(root.join("arete.toml")).unwrap();
        assert!(
            problems[0].problem.contains("a4 install --locked"),
            "{problems:?}"
        );
    }

    /// Each new deployment in the environment regenerates the client for it.
    #[test]
    fn a_redeploy_in_the_environment_regenerates_the_client_for_it() {
        let dir = project(&[]);
        let root = dir.path();
        let protocol = |deployment: &str| LocalProtocol {
            spec_json: spec(),
            slug: "local-tip-jar".into(),
            release_hash: LOCAL_RELEASE.into(),
            deployment_id: deployment.into(),
        };
        let first = sync_installed_client(
            root,
            &protocol("dep-env-1"),
            &[],
            "https://api.example.test",
        )
        .unwrap();
        assert!(first.added_dependency && first.install.changed);
        assert!(first
            .summary()
            .contains("deployment dep-env-1 in the dev environment"));
        let second = sync_installed_client(
            root,
            &protocol("dep-env-2"),
            &[],
            "https://api.example.test",
        )
        .unwrap();
        assert!(second.install.changed);
        assert_eq!(
            lock(root).dependencies[0]
                .protocol
                .as_ref()
                .unwrap()
                .deployment_id,
            "dep-env-2"
        );
    }

    #[test]
    fn a_deploy_before_publishing_pins_the_production_deployment() {
        let dir = project(&[]);
        let root = dir.path();
        let ensured = ensure_own_protocol_dependency(root, "local-tip-jar", "0.1.0", &[]).unwrap();
        install_local_protocol_build(
            &root.join("arete.toml"),
            "local-tip-jar",
            &build(LOCAL_RELEASE, "dep-local-1"),
            ensured.lock_was_fresh,
        )
        .unwrap();
        let production = "22222222-2222-4222-8222-222222222222";
        let mut deployed = build(LOCAL_RELEASE, "unused");
        deployed.deployment = LocalBuildDeployment::Deployed {
            id: production.into(),
            access: "private".into(),
        };
        let install = install_local_protocol_build(
            &root.join("arete.toml"),
            "local-tip-jar",
            &deployed,
            true,
        )
        .unwrap();
        assert!(install.changed && !install.published);
        let entry = lock(root).dependencies[0].clone();
        assert_eq!(entry.protocol.unwrap().deployment_id, production);
        assert_eq!(entry.local_build.as_deref(), Some(LOCAL_RELEASE));

        // The next `a4 dev` build calls its own deployment again.
        let next = build(
            &format!("arete:h1:protocol-release:sha256:{}", "e".repeat(64)),
            "dep-local-2",
        );
        let dev =
            install_local_protocol_build(&root.join("arete.toml"), "local-tip-jar", &next, true)
                .unwrap();
        assert!(dev.changed && dev.deployment_id == "dep-local-2");
    }

    /// `a4 protocol deploy` after a version bump: the own dependency's
    /// requirement moves to the deployed version, so the reinstall resolves
    /// the new package; a matching requirement is left as written.
    #[test]
    fn a_version_bump_moves_the_own_requirement() {
        use crate::project::installer::require_own_protocol_version;

        let dir = project(&[]);
        let root = dir.path();
        let manifest = root.join("arete.toml");
        ensure_own_protocol_dependency(root, "local-tip-jar", "0.1.0", &[]).unwrap();
        assert_eq!(
            require_own_protocol_version(&manifest, "local-tip-jar", "0.1.4").unwrap(),
            None
        );
        assert_eq!(
            require_own_protocol_version(&manifest, "local-tip-jar", "0.2.0").unwrap(),
            Some("^0.2.0".to_string())
        );
        let toml = std::fs::read_to_string(&manifest).unwrap();
        assert!(
            toml.contains("version = \"^0.2.0\"")
                && toml.contains("source = { registry = \"local-tip-jar\" }")
                && !toml.contains("^0.1.0"),
            "{toml}"
        );
        assert_eq!(
            require_own_protocol_version(&manifest, "local-tip-jar", "0.2.0").unwrap(),
            None
        );
        assert_eq!(
            require_own_protocol_version(&manifest, "not-installed", "9.0.0").unwrap(),
            None
        );
    }

    #[test]
    fn dev_sdk_targets_must_be_project_targets() {
        let dir = project(&[]);
        let error = ensure_own_protocol_dependency(
            dir.path(),
            "local-tip-jar",
            "0.1.0",
            &[InstallTarget::Python],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("[sdk].targets"), "{error}");
    }

    /// The local tip jar's client in all three languages, generated where
    /// `a4 install` puts it. The goldens are compiled elsewhere: the
    /// TypeScript under `cli/tests/golden/tsconfig.strict.json` (CI), the
    /// Rust by `rust/arete-a4-sdk/tests/generated_protocol.rs`. Python is
    /// imported here when the in-repo SDK imports. Regenerate with
    /// `A4_UPDATE_GOLDEN=1 cargo test -p a4-cli dev::sdk_gen`.
    #[test]
    fn generates_the_local_client_in_every_language() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut manifest = ManifestV1::new("app".into());
        manifest.sdk.targets = vec![
            InstallTarget::TypeScript,
            InstallTarget::Rust,
            InstallTarget::Python,
        ];
        std::fs::write(root.join("arete.toml"), manifest.to_toml_pretty().unwrap()).unwrap();
        let ensured = ensure_own_protocol_dependency(
            root,
            "local-tip-jar",
            "0.1.0",
            &[
                InstallTarget::TypeScript,
                InstallTarget::Rust,
                InstallTarget::Python,
            ],
        )
        .unwrap();
        let install = install_local_protocol_build(
            &root.join("arete.toml"),
            "local-tip-jar",
            &build(LOCAL_RELEASE, "dep-local-1"),
            ensured.lock_was_fresh,
        )
        .unwrap();
        assert_eq!(install.outputs.len(), 3);
        let find = |target: &str, name: &str| -> String {
            let output = install
                .outputs
                .iter()
                .find(|path| {
                    path.to_string_lossy()
                        .contains(&format!("generated/{target}/"))
                })
                .unwrap_or_else(|| panic!("no {target} output in {:?}", install.outputs));
            let mut stack = vec![output.clone()];
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(&dir).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.file_name().unwrap() == name {
                        return std::fs::read_to_string(path).unwrap();
                    }
                }
            }
            panic!("no {name} under {}", output.display());
        };
        let typescript = find("typescript", "local-tip-jar.ts");
        let rust = find("rust", "lib.rs");
        let python = find("python", "__init__.py");
        for text in [&typescript, &rust, &python] {
            assert!(text.contains("dep-local-1"), "{text}");
            assert!(text.contains("https://api.example.test/api"));
            assert!(text.contains("https://api.example.test/ws/sessions"));
        }
        assert!(typescript.contains("access: 'private'"), "{typescript}");

        let python_output = install
            .outputs
            .iter()
            .find(|path| path.to_string_lossy().contains("generated/python/"))
            .unwrap();
        python_imports(python_output, "local_tip_jar_protocol");

        let generated: BTreeMap<String, String> = without_release_version(BTreeMap::from([
            ("typescript/local-tip-jar.ts".to_string(), typescript),
            ("rust/lib.rs".to_string(), rust),
            ("python/__init__.py".to_string(), python),
        ]));
        let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/dev-sdk");
        // Python syntax is checked on the golden files, so compare first.
        compare_with_golden("local-tip-jar", &golden, generated.clone());
        assert_syntax("local-tip-jar", &golden, &generated);
    }

    /// `import <package>` with the in-repo Python SDK on the path. Skipped
    /// (with a note) when Python or the SDK's own dependencies are missing.
    fn python_imports(output: &Path, package: &str) {
        let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
        let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../python/arete-sdk");
        let path = std::env::join_paths([output, sdk.as_path()]).unwrap();
        let run = |code: &str| {
            std::process::Command::new(&python)
                .args(["-c", code])
                .env("PYTHONPATH", &path)
                .output()
        };
        match run("import arete.protocol") {
            Ok(output) if output.status.success() => {}
            _ => {
                eprintln!("skipping the Python import check: the in-repo SDK does not import here");
                return;
            }
        }
        let imported = run(&format!(
            "import {package} as m\nassert m.LOCAL_TIP_JAR_PROTOCOL is not None"
        ))
        .unwrap();
        assert!(
            imported.status.success(),
            "the generated Python client does not import:\n{}",
            String::from_utf8_lossy(&imported.stderr)
        );
    }
}
