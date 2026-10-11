//! `a4 sdk describe` and the `describe_sdk` MCP tool: the reference of an
//! installed SDK, read from the `sdk-reference.json` `a4 install` wrote into
//! its folder. Never the registry: the reference describes exactly what is
//! installed.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use arete_mcp::sdk_reference::{
    render_selection, DescribeRequest, SdkKind, SdkReference, SdkReferenceSource, DESCRIBE_COMMAND,
    README_FILE, REFERENCE_FILE,
};
use serde::Serialize;
use serde_json::json;

use super::graph::{InstallPlan, PlannedOutput};
use super::manifest::{DependencyKind, InstallTarget, ProjectManifest};

/// One SDK output of the project, and its reference files.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstalledSdk {
    alias: String,
    kind: DependencyKind,
    target: InstallTarget,
    directory: String,
    /// The README, when the install wrote a reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    readme: Option<String>,
}

impl InstalledSdk {
    fn new(output: &PlannedOutput, root: &Path) -> Self {
        let directory = output_directory(&output.path);
        let readme = directory
            .join(REFERENCE_FILE)
            .is_file()
            .then(|| display(&directory.join(README_FILE), root));
        Self {
            alias: output.alias.clone(),
            kind: output.kind,
            target: output.target,
            directory: display(&directory, root),
            readme,
        }
    }
}

/// The SDK folder of an output: TypeScript outputs may name the entry file.
fn output_directory(path: &Path) -> PathBuf {
    if path.extension().and_then(|extension| extension.to_str()) == Some("ts") {
        path.parent().unwrap_or(path).to_path_buf()
    } else {
        path.to_path_buf()
    }
}

/// `path` relative to the project `root` when it is inside it.
fn display(path: &Path, root: &Path) -> String {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    path.strip_prefix(&root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

/// Answer `request` for the project whose manifest is `manifest_path`.
pub fn describe(manifest_path: &Path, request: &DescribeRequest) -> Result<String> {
    let manifest = ProjectManifest::load(manifest_path).with_context(|| {
        format!(
            "No Arete project at {}: SDK references describe the SDKs `a4 install` generated there",
            manifest_path.display()
        )
    })?;
    let plan = InstallPlan::build(&manifest, true)?;
    let Some(alias) = request.alias.as_deref() else {
        return list(&manifest, &plan, request.json);
    };
    let kind = request.kind.map(|kind| match kind {
        SdkKind::Stack => DependencyKind::Stack,
        SdkKind::Program => DependencyKind::Program,
    });
    let outputs = plan
        .outputs
        .iter()
        .filter(|output| output.alias == alias && kind.is_none_or(|kind| kind == output.kind))
        .collect::<Vec<_>>();
    let output = match outputs.as_slice() {
        [] => bail!(
            "No dependency '{alias}' in {}; installed: {}",
            manifest.path.display(),
            names(&plan)
        ),
        [output] => *output,
        _ => outputs
            .iter()
            .find(|output| output.target == InstallTarget::TypeScript)
            .copied()
            .filter(|_| {
                outputs
                    .iter()
                    .map(|output| output.kind)
                    .all(|kind| kind == outputs[0].kind)
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "'{alias}' is both a stack and a program dependency; pass --kind stack or --kind program"
                )
            })?,
    };
    let installed = InstalledSdk::new(output, &manifest.root);
    let directory = output_directory(&output.path);
    let path = directory.join(REFERENCE_FILE);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if output.target != InstallTarget::TypeScript {
                bail!(
                    "{} {alias} has no SDK reference: references are generated for TypeScript SDKs only; see {} for the generated {} SDK",
                    output.kind,
                    installed.directory,
                    output.target
                );
            }
            bail!(
                "{} {alias} has no SDK reference in {}: run `a4 install` to regenerate it with this a4",
                output.kind,
                installed.directory
            );
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read {}", path.display()))
        }
    };
    let reference = SdkReference::from_json(&contents)
        .with_context(|| format!("Invalid SDK reference {}", path.display()))?;
    if request.json {
        let selected = reference.select(&request.selection)?;
        return Ok(serde_json::to_string_pretty(&json!({
            "alias": installed.alias,
            "kind": installed.kind,
            "target": installed.target,
            "directory": installed.directory,
            "readme": installed.readme,
            "reference": selected,
        }))?);
    }
    let mut text = render_selection(&reference, &request.selection)?;
    if !request.selection.is_empty() {
        if let Some(readme) = &installed.readme {
            text.push_str(&format!("Everything else: {readme}\n"));
        }
    }
    Ok(text)
}

fn list(manifest: &ProjectManifest, plan: &InstallPlan, json: bool) -> Result<String> {
    let installed = plan
        .outputs
        .iter()
        .map(|output| InstalledSdk::new(output, &manifest.root))
        .collect::<Vec<_>>();
    if json {
        return Ok(serde_json::to_string_pretty(&json!({ "sdks": installed }))?);
    }
    if installed.is_empty() {
        return Ok(format!(
            "No dependencies in {}. Add one with `a4 install stack <name> --ts`.\n",
            manifest.path.display()
        ));
    }
    let mut text = String::from("Installed SDKs:\n");
    for sdk in &installed {
        let reference = match &sdk.readme {
            Some(readme) => format!("reference: {readme}"),
            None => format!("no reference ({})", sdk.directory),
        };
        text.push_str(&format!(
            "  {} {} ({}): {reference}\n",
            sdk.kind, sdk.alias, sdk.target
        ));
    }
    text.push_str(&format!(
        "Describe one with `{DESCRIBE_COMMAND} <alias> [--view <Entity/view>] [--read <name>] [--program <key>]`.\n"
    ));
    Ok(text)
}

fn names(plan: &InstallPlan) -> String {
    let names = plan
        .outputs
        .iter()
        .map(|output| format!("{} {}", output.kind, output.alias))
        .collect::<std::collections::BTreeSet<_>>();
    if names.is_empty() {
        "(none)".to_string()
    } else {
        names.into_iter().collect::<Vec<_>>().join(", ")
    }
}

/// The SDK references of the project the MCP server runs in: the nearest
/// `arete.toml` at or above its working directory.
pub struct ProjectSdkReferences;

impl SdkReferenceSource for ProjectSdkReferences {
    fn describe(&self, request: &DescribeRequest) -> Result<String> {
        let current = std::env::current_dir()?;
        let manifest = current
            .ancestors()
            .map(|directory| directory.join("arete.toml"))
            .find(|path| path.is_file())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No arete.toml at or above {}: start the MCP server in an Arete project",
                    current.display()
                )
            })?;
        describe(&manifest, request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arete_mcp::sdk_reference::Selection;

    /// A project with an installed `ore` stack whose folder holds the
    /// reference, and a `spl` program installed before references existed.
    fn project() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("arete.toml"),
            r#"manifest_version = 1

[project]
name = "app"

[dependencies.stacks.ore]
source = { registry = "ore" }
version = "^1.1.9"
targets = ["typescript"]

[dependencies.programs.spl]
source = { registry = "spl-token" }
version = "^4.0.0"
targets = ["typescript"]
"#,
        )
        .unwrap();
        let ore = root.path().join("generated/typescript/stacks/ore");
        std::fs::create_dir_all(&ore).unwrap();
        std::fs::create_dir_all(root.path().join("generated/typescript/programs/spl")).unwrap();
        let reference = serde_json::json!({
            "schemaVersion": 1,
            "kind": "stack",
            "alias": "ore",
            "language": "typescript",
            "package": "ore",
            "version": "1.1.9",
            "import": {
                "module": "ore.ts",
                "specifier": "./generated/typescript/stacks/ore/ore.js",
                "export": "ORE_STREAM_STACK"
            },
            "entities": [{
                "name": "OreRound",
                "typeName": "OreRound",
                "views": [{"id": "OreRound/latest", "kind": "list", "access": "views.OreRound.latest"}],
                "fields": [{"path": "id.roundId", "wire": "id.round_id", "type": "bigint", "nullable": true}]
            }],
            "reads": [{"path": "read.currentRound", "params": [], "title": "Current round"}]
        });
        std::fs::write(ore.join(REFERENCE_FILE), reference.to_string()).unwrap();
        std::fs::write(ore.join(README_FILE), "# ore\n").unwrap();
        root
    }

    fn request(alias: Option<&str>) -> DescribeRequest {
        DescribeRequest {
            alias: alias.map(str::to_string),
            ..DescribeRequest::default()
        }
    }

    #[test]
    fn describes_an_installed_stack_from_its_folder() {
        let root = project();
        let manifest = root.path().join("arete.toml");

        let text = describe(&manifest, &request(Some("ore"))).unwrap();
        assert!(
            text.starts_with("# `ore` TypeScript SDK reference"),
            "{text}"
        );
        assert!(
            text.contains("id.roundId  bigint | null  ← round_id"),
            "{text}"
        );

        let text = describe(
            &manifest,
            &DescribeRequest {
                selection: Selection {
                    read: Some("currentRound".into()),
                    ..Selection::default()
                },
                ..request(Some("ore"))
            },
        )
        .unwrap();
        assert!(
            text.contains("- `read.currentRound()` — Current round."),
            "{text}"
        );
        assert!(!text.contains("OreRound/latest"), "{text}");
        assert!(
            text.ends_with("Everything else: generated/typescript/stacks/ore/README.md\n"),
            "{text}"
        );

        let json: serde_json::Value = serde_json::from_str(
            &describe(
                &manifest,
                &DescribeRequest {
                    json: true,
                    ..request(Some("ore"))
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(json["directory"], "generated/typescript/stacks/ore");
        assert_eq!(json["readme"], "generated/typescript/stacks/ore/README.md");
        assert_eq!(
            json["reference"]["entities"][0]["fields"][0]["wire"],
            "id.round_id"
        );
    }

    #[test]
    fn lists_installed_sdks_and_explains_a_missing_reference() {
        let root = project();
        let manifest = root.path().join("arete.toml");

        let text = describe(&manifest, &request(None)).unwrap();
        assert!(
            text.contains(
                "stack ore (typescript): reference: generated/typescript/stacks/ore/README.md"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "program spl (typescript): no reference (generated/typescript/programs/spl)"
            ),
            "{text}"
        );

        let error = describe(&manifest, &request(Some("spl")))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("run `a4 install` to regenerate it"),
            "{error}"
        );

        let error = describe(&manifest, &request(Some("raydium")))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("installed: program spl, stack ore"),
            "{error}"
        );
    }
}
