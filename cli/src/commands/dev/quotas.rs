//! The stack slot preflight `a4 dev up` runs before it loads anything: an
//! environment holds at most its dev tier's stacks, so a project that needs
//! more is refused up front, with every stack named, instead of failing
//! halfway with some slots spent.

use std::path::Path;

use anyhow::{bail, Result};
use arete_mcp::dev_env::EnvApi;
use serde_json::Value;

/// One stack `a4 dev up` would load into the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedStack {
    /// How the refusal names it: `stack pools`, `protocol tipjar's stack`.
    pub label: String,
    /// The environment already holds it, so loading it again reuses its slot.
    pub loaded: bool,
}

/// The stacks this project loads: the registry stacks `arete.lock` pins
/// (`(package, StackManifest hash, live alias)`), its protocol's generated
/// stack and its own stacks. `slug` is the protocol's slug once a deploy has
/// named it.
pub fn planned_stacks(
    root: &Path,
    lock_stacks: &[(String, String, Option<String>)],
    protocol: bool,
    slug: Option<&str>,
    listing: &[Value],
) -> Result<Vec<PlannedStack>> {
    let mut planned = Vec::new();
    // The environment's registry stack deployments are named
    // `registry-<package>-<digest>`, one per loaded alias. A package's
    // aliases count as loaded up to the deployments it already has; the rest
    // each need a slot.
    let mut held_per_package = std::collections::BTreeMap::<&str, usize>::new();
    for (package, _, alias) in lock_stacks {
        let held = held_per_package.entry(package.as_str()).or_insert_with(|| {
            let prefix = format!("registry-{package}-");
            listing
                .iter()
                .filter(|entry| {
                    entry["specName"]
                        .as_str()
                        .is_some_and(|name| name.starts_with(&prefix))
                })
                .count()
        });
        let loaded = *held > 0;
        *held = held.saturating_sub(1);
        planned.push(PlannedStack {
            label: match alias {
                Some(alias) => format!("registry stack {package} ({alias})"),
                None => format!("registry stack {package}"),
            },
            loaded,
        });
    }
    if protocol {
        let loaded = slug.is_some_and(|slug| {
            listing
                .iter()
                .any(|stack| stack["protocolSlug"].as_str() == Some(slug))
        });
        planned.push(PlannedStack {
            label: match slug {
                Some(slug) => format!("protocol {slug}'s stack"),
                None => "the protocol's stack".to_string(),
            },
            loaded,
        });
    }
    for stack in super::custom_stacks::detect(root)?.stacks {
        // `a4 dev up` deploys each live alias of a stack separately.
        for name in project_stack_names(&stack) {
            let loaded = listing
                .iter()
                .any(|entry| entry["specName"].as_str() == Some(name.as_str()));
            planned.push(PlannedStack {
                label: format!("stack {name}"),
                loaded,
            });
        }
    }
    Ok(planned)
}

/// The deployment names one project stack loads under: one per live alias of
/// an authored StackManifest (read as it is on disk), else the stack's own
/// name (a Stack Source compiles to one live spec).
fn project_stack_names(stack: &super::custom_stacks::CustomStack) -> Vec<String> {
    use super::custom_stacks::StackOrigin;
    let StackOrigin::Manifest {
        manifest,
        artifact_roots,
        ..
    } = &stack.origin
    else {
        return vec![stack.name.clone()];
    };
    match crate::commands::public_artifacts::load_local_artifact_stack_with_roots(
        manifest,
        artifact_roots,
    ) {
        Ok(artifacts) if !artifacts.live_specs.is_empty() => artifacts
            .live_specs
            .iter()
            .enumerate()
            .map(|(index, (alias, _))| {
                crate::commands::up::child_spec_name(&stack.name, alias, index)
            })
            .collect(),
        _ => vec![stack.name.clone()],
    }
}

/// Refuse `planned` when the environment, which holds `held` stacks, has
/// fewer free slots than the new ones need.
pub fn check(planned: &[PlannedStack], held: usize, tier: &str, limit: u64) -> Result<()> {
    let needed = planned.iter().filter(|stack| !stack.loaded).count();
    let free = limit.saturating_sub(held as u64);
    if needed as u64 <= free {
        return Ok(());
    }
    let lines: Vec<String> = planned
        .iter()
        .map(|stack| {
            if stack.loaded {
                format!("  - {}: 0 slots (already loaded)", stack.label)
            } else {
                format!("  - {}: 1 slot", stack.label)
            }
        })
        .collect();
    bail!(
        "This dev environment can't hold the stacks `a4 dev up` loads: it holds {held} of the {limit} stacks the {tier} dev tier allows, and these need {needed} more:\n{}\nNothing was loaded. The pro plan's dev tier holds more stacks per environment (`a4 auth whoami` shows your plan; `a4 doctor` your dev tier), or use another environment with `--env <name>`.",
        lines.join("\n")
    )
}

/// Read the tier's slots and the environment's stacks, then [`check`] the
/// stacks this project loads. An API that reports no stack limit has none to
/// check.
pub async fn preflight(
    api: &EnvApi,
    env: &str,
    root: &Path,
    lock_stacks: &[(String, String, Option<String>)],
    protocol: bool,
    slug: Option<&str>,
) -> Result<()> {
    let quotas = api.quotas().await?;
    let Some(limit) = quotas["limits"]["maxStacksPerEnvironment"].as_u64() else {
        return Ok(());
    };
    let tier = quotas["tier"].as_str().unwrap_or("current");
    let listing = api.stacks(env).await?;
    let listing = listing["stacks"].as_array().cloned().unwrap_or_default();
    let planned = planned_stacks(root, lock_stacks, protocol, slug, &listing)?;
    check(&planned, listing.len(), tier, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stack(label: &str, loaded: bool) -> PlannedStack {
        PlannedStack {
            label: label.to_string(),
            loaded,
        }
    }

    #[test]
    fn a_project_over_the_free_tier_is_refused_with_every_stack_named() {
        let planned = [
            stack("protocol tipjar's stack", true),
            stack("stack pools", false),
            stack("stack rates", false),
        ];
        let error = check(&planned, 4, "free", 5).unwrap_err().to_string();
        assert!(
            error.contains("holds 4 of the 5 stacks the free dev tier allows"),
            "{error}"
        );
        assert!(error.contains("these need 2 more"), "{error}");
        assert!(
            error.contains("protocol tipjar's stack: 0 slots (already loaded)"),
            "{error}"
        );
        assert!(error.contains("stack pools: 1 slot"), "{error}");
        assert!(error.contains("stack rates: 1 slot"), "{error}");
        assert!(error.contains("Nothing was loaded"), "{error}");
        assert!(error.contains("pro plan"), "{error}");
    }

    #[test]
    fn lock_stacks_count_before_any_loads() {
        let root = tempfile::tempdir().unwrap();
        let lock = vec![
            ("klend".to_string(), "hash-k".to_string(), None),
            (
                "meteora".to_string(),
                "hash-m".to_string(),
                Some("pools".to_string()),
            ),
            (
                "meteora".to_string(),
                "hash-m".to_string(),
                Some("swaps".to_string()),
            ),
        ];
        let listing = vec![
            json!({"specName": "registry-klend-0123456789abcdef"}),
            json!({"specName": "registry-meteora-fedcba9876543210"}),
        ];
        let planned = planned_stacks(root.path(), &lock, false, None, &listing).unwrap();
        assert_eq!(
            planned,
            vec![
                stack("registry stack klend", true),
                // One meteora alias is loaded; the other still needs a slot.
                stack("registry stack meteora (pools)", true),
                stack("registry stack meteora (swaps)", false),
            ]
        );
        // A full free tier refuses, naming every stack.
        let error = check(&planned, 5, "free", 5).unwrap_err().to_string();
        assert!(
            error.contains("registry stack meteora (pools): 0 slots (already loaded)"),
            "{error}"
        );
        assert!(
            error.contains("registry stack meteora (swaps): 1 slot"),
            "{error}"
        );
        assert!(
            error.contains("registry stack klend: 0 slots (already loaded)"),
            "{error}"
        );
    }

    #[test]
    fn stacks_that_fit_or_reload_pass() {
        let planned = [stack("stack pools", false), stack("stack rates", true)];
        check(&planned, 4, "free", 5).unwrap();
        check(&[stack("stack rates", true)], 5, "free", 5).unwrap();
        check(&[], 9, "free", 5).unwrap();
    }

    #[test]
    fn the_plan_matches_loaded_stacks_by_name_and_protocol() {
        let root = tempfile::tempdir().unwrap();
        let listing = vec![json!({"specName": "other", "protocolSlug": "tipjar"})];
        let planned = planned_stacks(root.path(), &[], true, Some("tipjar"), &listing).unwrap();
        assert_eq!(planned, vec![stack("protocol tipjar's stack", true)]);
        let first_deploy = planned_stacks(root.path(), &[], true, None, &listing).unwrap();
        assert_eq!(first_deploy, vec![stack("the protocol's stack", false)]);
        assert!(planned_stacks(root.path(), &[], false, None, &listing)
            .unwrap()
            .is_empty());
    }
}
