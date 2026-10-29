//! `a4 dev promote --check`: what the app uses that exists only in the dev
//! environment, each with the command that publishes it.
//!
//! An app developed with `ARETE_ENVIRONMENT=dev` may call things production
//! does not have yet: a protocol built locally, a program the environment
//! changed, a project stack loaded under a dependency's alias. The check
//! lists each and exits non-zero while any remain, so CI can gate a
//! production deploy on it. The deploy story is: run the check, publish what
//! it lists, `a4 install --locked`, deploy the app with `ARETE_ENVIRONMENT`
//! unset.

use std::path::Path;

use anyhow::{bail, Result};
use arete_mcp::dev::DevProject;
use arete_mcp::dev_env::EnvApi;
use clap::Args;
use serde::Serialize;
use serde_json::{json, Value};

use crate::project::lockfile::{LockedDependency, ProjectLock};

#[derive(Args, Debug, Clone)]
pub struct PromoteArgs {
    /// List what exists only in the dev environment and exit non-zero while
    /// anything does (the only mode)
    #[arg(long)]
    pub check: bool,
}

/// One thing the app uses that production does not have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    pub kind: &'static str,
    /// The dependency alias in arete.toml.
    pub name: String,
    pub detail: String,
    pub fix: String,
}

/// Every program a dependency pins, with its release: a program dependency
/// itself, and the programs of a stack or protocol.
fn pinned_programs(dependency: &LockedDependency) -> Vec<(&str, Option<&str>)> {
    let mut programs: Vec<(&str, Option<&str>)> = Vec::new();
    if let Some(program_id) = &dependency.program_id {
        programs.push((program_id, dependency.program_release_hash.as_deref()));
    }
    for program in dependency.programs.iter().chain(
        dependency
            .stacks
            .iter()
            .flat_map(|stack| stack.programs.iter()),
    ) {
        programs.push((&program.program_id, program.program_release_hash.as_deref()));
    }
    programs
}

/// What `lock` alone says exists only in development.
pub fn lock_pending(lock: &ProjectLock) -> Vec<Pending> {
    lock.dependencies
        .iter()
        .filter_map(|dependency| {
            let build = dependency.local_build.as_ref()?;
            Some(Pending {
                kind: "local-protocol-build",
                name: dependency.alias.clone(),
                detail: format!("the protocol client is generated from a local build ({build})"),
                fix: "Publish it with `a4 protocol deploy` in the protocol's project, then run `a4 install --locked` here".into(),
            })
        })
        .collect()
}

/// What the environment's `overrides` serve in place of what `lock` pins.
pub fn override_pending(lock: &ProjectLock, overrides: &[Value]) -> Vec<Pending> {
    let mut pending = Vec::new();
    for item in overrides {
        let (Some(kind), Some(key), Some(target)) = (
            item["kind"].as_str(),
            item["key"].as_str(),
            item["target"].as_str(),
        ) else {
            continue;
        };
        match kind {
            "program_release" => {
                for dependency in &lock.dependencies {
                    let Some((_, pinned)) = pinned_programs(dependency)
                        .into_iter()
                        .find(|(program_id, _)| *program_id == key)
                    else {
                        continue;
                    };
                    if pinned == Some(target) {
                        continue;
                    }
                    pending.push(Pending {
                        kind: "program-release-override",
                        name: dependency.alias.clone(),
                        detail: format!(
                            "program {key} runs a changed release in the environment ({target}); arete.lock pins {}",
                            pinned.unwrap_or("no release")
                        ),
                        fix: "Deploy the program to mainnet and push its IDL (`a4 program push <idl>`), then `a4 install` the new release".into(),
                    });
                }
            }
            "stack_alias" => {
                let Some((manifest, alias)) = key.split_once('#') else {
                    continue;
                };
                for dependency in lock.dependencies.iter().filter(|dependency| {
                    dependency.stack_manifest_hash.as_deref() == Some(manifest)
                }) {
                    pending.push(Pending {
                        kind: "stack-alias-override",
                        name: dependency.alias.clone(),
                        detail: format!(
                            "the environment serves a project stack under the dependency's live alias '{alias}'"
                        ),
                        fix: "Publish the project stack with `a4 up`, point the dependency at it in arete.toml, then `a4 install`".into(),
                    });
                }
            }
            _ => {}
        }
    }
    pending
}

pub fn promote(args: PromoteArgs, json_output: bool) -> Result<()> {
    if !args.check {
        bail!("`a4 dev promote` only checks for now: run `a4 dev promote --check`");
    }
    let project = DevProject::discover(None).ok();
    let root = match &project {
        Some(project) => project.root.clone(),
        None => std::env::current_dir()?,
    };
    let lock = ProjectLock::load_optional(root.join("arete.lock"))?;
    let Some(lock) = lock else {
        bail!(
            "No arete.lock in {}: run the check in the app's project",
            root.display()
        );
    };
    let mut pending = lock_pending(&lock);
    let selection = project
        .as_ref()
        .and_then(|project| project.load_selection().ok().flatten());
    let environment = match &selection {
        Some(selection) => {
            let overrides = super::block_on(async {
                let api = EnvApi::for_origin(reqwest::Client::new(), &selection.api_url)?;
                api.overrides(&selection.id).await
            })?;
            pending.extend(override_pending(&lock, &overrides));
            Some(json!({ "id": selection.id, "name": selection.name }))
        }
        None => None,
    };
    report(&root, &pending, environment.as_ref(), json_output);
    if !pending.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn report(root: &Path, pending: &[Pending], environment: Option<&Value>, json_output: bool) {
    if json_output {
        println!(
            "{}",
            json!({ "clean": pending.is_empty(), "environment": environment, "pending": pending })
        );
        return;
    }
    if environment.is_none() {
        println!(
            "No dev environment for {}: checked arete.lock only.",
            root.display()
        );
    }
    if pending.is_empty() {
        println!("Nothing exists only in the dev environment: the app can run against production.");
        return;
    }
    println!(
        "{} thing(s) exist only in the dev environment:",
        pending.len()
    );
    for item in pending {
        println!("  {} ({}): {}", item.name, item.kind, item.detail);
        println!("    fix: {}", item.fix);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::lockfile::{LockedDependency, LockedProgram};
    use crate::project::manifest::DependencyKind;

    fn dependency(alias: &str, kind: DependencyKind) -> LockedDependency {
        LockedDependency {
            kind,
            alias: alias.into(),
            source: format!("registry:{alias}"),
            requirement: None,
            version: None,
            package_release_hash: None,
            stack_manifest_hash: None,
            program_id: None,
            program_spec_hash: None,
            program_release_hash: None,
            live_specs: Vec::new(),
            programs: Vec::new(),
            parts: Vec::new(),
            sdk_extension_hashes: Vec::new(),
            protocol: None,
            stacks: Vec::new(),
            targets: Vec::new(),
            generator_contract: String::new(),
            local_build: None,
        }
    }

    fn lock() -> ProjectLock {
        let mut raise = dependency("token-raise", DependencyKind::Protocol);
        raise.local_build = Some("arete:h1:protocol-release:sha256:aa".into());
        let mut klend = dependency("klend", DependencyKind::Stack);
        klend.stack_manifest_hash = Some("manifest-klend".into());
        klend.programs = vec![LockedProgram {
            program_id: "KLend".into(),
            program_spec_hash: "spec".into(),
            program_release_hash: Some("release-installed".into()),
            package_release_hash: None,
            sdk_extension_hashes: Vec::new(),
        }];
        let mut lock = ProjectLock::empty("manifest".into());
        lock.dependencies = vec![raise, klend, dependency("clean", DependencyKind::Program)];
        lock
    }

    #[test]
    fn a_local_protocol_build_is_pending() {
        let pending = lock_pending(&lock());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].kind, "local-protocol-build");
        assert_eq!(pending[0].name, "token-raise");
        assert!(pending[0].fix.contains("a4 protocol deploy"));
    }

    #[test]
    fn overrides_of_what_the_lock_pins_are_pending() {
        let overrides = [
            json!({ "kind": "program_release", "key": "KLend", "target": "release-changed" }),
            // The same release the lock pins changes nothing.
            json!({ "kind": "program_release", "key": "KLend", "target": "release-installed" }),
            // A program the app doesn't use.
            json!({ "kind": "program_release", "key": "Other", "target": "x" }),
            json!({ "kind": "stack_alias", "key": "manifest-klend#default", "target": "17" }),
            json!({ "kind": "stack_alias", "key": "manifest-other#default", "target": "18" }),
        ];
        let pending = override_pending(&lock(), &overrides);
        let kinds: Vec<_> = pending
            .iter()
            .map(|item| (item.kind, item.name.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("program-release-override", "klend"),
                ("stack-alias-override", "klend"),
            ]
        );
        assert!(pending[0].detail.contains("release-changed"));
    }
}
