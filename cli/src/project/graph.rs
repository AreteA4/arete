use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use super::manifest::{DependencyKind, InstallTarget, ProjectManifest};
use super::paths::ProjectPaths;

#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub outputs: Vec<PlannedOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedOutput {
    pub kind: DependencyKind,
    pub alias: String,
    pub target: InstallTarget,
    pub path: PathBuf,
}

impl InstallPlan {
    pub fn build(manifest: &ProjectManifest, allow_outside_project: bool) -> Result<Self> {
        let paths = ProjectPaths::new(
            &manifest.root,
            manifest.document.install.allow_outside_project,
            allow_outside_project,
        )?;
        let mut outputs = Vec::new();
        for (kind, alias, dependency) in manifest.dependencies() {
            for target in dependency.selected_targets(&manifest.document.sdk) {
                let configured = dependency
                    .outputs
                    .get(*target)
                    .map(str::to_owned)
                    .unwrap_or_else(|| default_output(manifest, kind, alias, *target));
                outputs.push(PlannedOutput {
                    kind,
                    alias: alias.clone(),
                    target: *target,
                    path: paths.output(
                        &configured,
                        &format!("{kind} dependency '{alias}' {target} output"),
                    )?,
                });
            }
        }
        outputs.sort_by(|left, right| {
            (left.kind, left.alias.as_str(), left.target).cmp(&(
                right.kind,
                right.alias.as_str(),
                right.target,
            ))
        });
        validate_collisions(&outputs)?;
        Ok(Self { outputs })
    }

    pub fn for_dependency<'a>(
        &'a self,
        kind: DependencyKind,
        alias: &'a str,
    ) -> impl Iterator<Item = &'a PlannedOutput> + 'a {
        self.outputs
            .iter()
            .filter(move |output| output.kind == kind && output.alias == alias)
    }
}

fn default_output(
    manifest: &ProjectManifest,
    kind: DependencyKind,
    alias: &str,
    target: InstallTarget,
) -> String {
    let (base, name) = match target {
        InstallTarget::TypeScript => (
            manifest.document.sdk.typescript.output_dir.as_str(),
            alias.to_string(),
        ),
        InstallTarget::Rust => {
            let prefix = &manifest.document.sdk.rust.crate_prefix;
            let stem = if prefix.is_empty() {
                alias.to_string()
            } else {
                format!("{prefix}-{alias}")
            };
            (
                manifest.document.sdk.rust.output_dir.as_str(),
                format!("{stem}-{}", kind.as_str()),
            )
        }
        InstallTarget::Python => {
            let prefix = &manifest.document.sdk.python.package_prefix;
            let stem = if prefix.is_empty() {
                alias.to_string()
            } else {
                format!("{prefix}-{alias}")
            };
            (
                manifest.document.sdk.python.output_dir.as_str(),
                format!("{stem}-{}", kind.as_str()),
            )
        }
    };
    Path::new(base)
        .join(kind.namespace())
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn validate_collisions(outputs: &[PlannedOutput]) -> Result<()> {
    for (index, left) in outputs.iter().enumerate() {
        for right in &outputs[index + 1..] {
            if left.path == right.path
                || left.path.starts_with(&right.path)
                || right.path.starts_with(&left.path)
            {
                bail!(
                    "SDK output collision: {} dependency '{}' {} output {} overlaps {} dependency '{}' {} output {}",
                    left.kind,
                    left.alias,
                    left.target,
                    left.path.display(),
                    right.kind,
                    right.alias,
                    right.target,
                    right.path.display()
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::manifest::ProjectManifest;

    #[test]
    fn cross_kind_aliases_have_distinct_default_outputs_for_every_target() {
        let root = std::env::temp_dir().join(format!("arete-kind-paths-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let manifest_path = root.join("arete.toml");
        std::fs::write(
            &manifest_path,
            r#"
manifest_version = 1
[project]
name = "kind-paths"
[sdk]
targets = ["typescript", "rust", "python"]
[dependencies.stacks.shared]
source = { registry = "shared-stack" }
version = "1"
[dependencies.programs.shared]
source = { registry = "shared-program" }
version = "1"
"#,
        )
        .unwrap();
        let manifest = ProjectManifest::load(&manifest_path).unwrap();
        let project_root = manifest.root.clone();
        let plan = InstallPlan::build(&manifest, false).unwrap();
        let relative = plan
            .outputs
            .iter()
            .map(|output| {
                output
                    .path
                    .strip_prefix(&project_root)
                    .unwrap()
                    .to_path_buf()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            relative,
            vec![
                PathBuf::from("generated/typescript/stacks/shared"),
                PathBuf::from("generated/rust/stacks/shared-stack"),
                PathBuf::from("generated/python/stacks/shared-stack"),
                PathBuf::from("generated/typescript/programs/shared"),
                PathBuf::from("generated/rust/programs/shared-program"),
                PathBuf::from("generated/python/programs/shared-program"),
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_and_nested_outputs_are_rejected_before_generation() {
        let root = std::env::temp_dir().join(format!("arete-graph-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let manifest_path = root.join("arete.toml");
        std::fs::write(
            &manifest_path,
            r#"
manifest_version = 1
[project]
name = "collision"
[sdk]
targets = ["typescript"]
[dependencies.stacks.one]
source = { registry = "one" }
version = "1"
[dependencies.programs.two]
source = { registry = "two" }
version = "1"
[dependencies.programs.two.outputs]
typescript = "./generated/typescript/stacks/one/nested"
"#,
        )
        .unwrap();
        let manifest = ProjectManifest::load(&manifest_path).unwrap();
        let error = InstallPlan::build(&manifest, false).unwrap_err();
        assert!(error.to_string().contains("SDK output collision"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
