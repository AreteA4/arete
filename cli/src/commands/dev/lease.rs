//! One project per cloud dev environment.
//!
//! `a4 dev up` takes the environment's lease before it deploys anything,
//! renews it while it runs and releases it on exit. Another project's
//! `a4 dev up` is refused with the holder's name (`--take-over` moves the
//! lease; `a4 dev branch` gives a project its own copy). Every write that
//! changes what the environment serves carries the lease generation; once
//! another project takes the lease, those writes are refused and this
//! process stops, without retrying under a new lease.
//!
//! An Arete API without environment leases answers the lease route with a
//! plain 404: `a4 dev up` then runs as before, without a lease.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use arete_mcp::dev_env::{api_code, EnvApi, LEASED, LEASE_LOST};
use sha2::Digest as _;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// The environment was taken over: the process must stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseLost {
    pub holder: String,
}

impl std::fmt::Display for LeaseLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "this environment was taken over by {}; nothing more was written",
            self.holder
        )
    }
}

impl std::error::Error for LeaseLost {}

/// Whether `error` says the environment's lease moved to another project: a
/// [`LeaseLost`], or an API answer with `devenv-lease-lost` anywhere in it.
pub fn lease_lost(error: &anyhow::Error) -> Option<LeaseLost> {
    if let Some(lost) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<LeaseLost>())
    {
        return Some(lost.clone());
    }
    let text = format!("{error:#}");
    text.contains(LEASE_LOST).then(|| LeaseLost {
        holder: holder_in(&text).unwrap_or_else(|| "another project".to_string()),
    })
}

/// The holder an API refusal names ("… taken over by <holder>; …").
fn holder_in(text: &str) -> Option<String> {
    let (_, rest) = text.split_once("taken over by ")?;
    let holder = rest.split(';').next()?.trim();
    (!holder.is_empty()).then(|| holder.to_string())
}

/// A stable id for the project at `root`: a digest of its canonical path,
/// so it names the checkout without revealing it.
pub fn project_id(root: &Path) -> String {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let digest = sha2::Sha256::digest(root.to_string_lossy().as_bytes());
    let hex: String = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("proj_{hex}")
}

/// What other projects see as the holder: the project folder's name.
pub fn holder_label(root: &Path) -> String {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "a project".to_string());
    let name: String = name.chars().filter(|c| !c.is_control()).take(120).collect();
    format!("{name} (a4 dev up)")
}

/// `devenv-environment-leased`, explained.
fn explain_refusal(error: anyhow::Error) -> anyhow::Error {
    if api_code(&error) == Some(LEASED) {
        return error.context(
            "Another project is using this dev environment. Run `a4 dev up --take-over` to move it to this project, or give this project its own copy with `a4 dev branch` and `a4 dev up --env <name>`",
        );
    }
    error
}

/// The lease this process holds, renewed in the background.
pub struct Held {
    api: EnvApi,
    env: String,
    generation: i64,
    lost: watch::Receiver<Option<LeaseLost>>,
    renewal: JoinHandle<()>,
    /// The other project's holder this take replaced (`--take-over`, or an
    /// expired lease).
    pub replaced: Option<String>,
}

impl Held {
    pub fn generation(&self) -> i64 {
        self.generation
    }

    /// A handle that resolves, through [`wait_lost`], once the lease is
    /// lost (a renewal was refused).
    pub fn watch(&self) -> LostWatch {
        LostWatch(self.lost.clone())
    }

    /// Stop renewing and release the lease (unless it was lost).
    pub async fn release(self) {
        self.renewal.abort();
        if self.lost.borrow().is_none() {
            let _ = self.api.release_lease(&self.env, self.generation).await;
        }
    }
}

/// See [`Held::watch`].
#[derive(Clone)]
pub struct LostWatch(watch::Receiver<Option<LeaseLost>>);

/// Resolves when the lease behind `watch` is lost; never without a lease.
pub async fn wait_lost(watch: Option<LostWatch>) -> LeaseLost {
    if let Some(LostWatch(mut lost)) = watch {
        loop {
            if let Some(lost) = lost.borrow_and_update().clone() {
                return lost;
            }
            if lost.changed().await.is_err() {
                break;
            }
        }
    }
    std::future::pending().await
}

/// Take `env`'s lease for the project at `root`. `Ok(None)` when the API has
/// no environment leases.
pub async fn take(api: &EnvApi, env: &str, root: &Path, take_over: bool) -> Result<Option<Held>> {
    let project = project_id(root);
    let holder = holder_label(root);
    let Some(lease) = api
        .take_lease(env, &project, &holder, take_over)
        .await
        .map_err(explain_refusal)?
    else {
        return Ok(None);
    };
    let (sender, receiver) = watch::channel(None);
    let every = Duration::from_secs((lease.ttl_seconds / 3).max(5));
    let renewal = tokio::spawn(renew(
        api.clone(),
        env.to_string(),
        project,
        holder,
        lease.generation,
        every,
        Arc::new(sender),
    ));
    Ok(Some(Held {
        api: api.clone(),
        env: env.to_string(),
        generation: lease.generation,
        lost: receiver,
        renewal,
        replaced: lease.replaced,
    }))
}

/// Renew every `every` until a renewal is refused with `devenv-lease-lost`.
/// Other failures (the network, the API restarting) are retried: the lease
/// keeps its generation until another project takes it.
async fn renew(
    api: EnvApi,
    env: String,
    project: String,
    holder: String,
    generation: i64,
    every: Duration,
    lost: Arc<watch::Sender<Option<LeaseLost>>>,
) {
    let mut tick = tokio::time::interval(every);
    tick.tick().await;
    loop {
        tick.tick().await;
        match api.renew_lease(&env, &project, &holder, generation).await {
            Ok(_) => {}
            Err(error) if api_code(&error) == Some(LEASE_LOST) => {
                let holder = current_holder(&api, &env)
                    .await
                    .or_else(|| holder_in(&format!("{error:#}")))
                    .unwrap_or_else(|| "another project".to_string());
                let _ = lost.send(Some(LeaseLost { holder }));
                return;
            }
            Err(_) => {}
        }
    }
}

async fn current_holder(api: &EnvApi, env: &str) -> Option<String> {
    let lease = api.lease(env).await.ok()??;
    lease["holder"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lost_leases_are_recognised_in_api_errors() {
        let error = anyhow::anyhow!(
            "Creating the deployment failed (409 Conflict): Dev environment denv_x was taken over by savings-app (a4 dev up); nothing was written (devenv-lease-lost)"
        );
        assert_eq!(
            lease_lost(&error),
            Some(LeaseLost {
                holder: "savings-app (a4 dev up)".into()
            })
        );
        let typed = anyhow::Error::new(LeaseLost { holder: "x".into() }).context("Redeploy failed");
        assert_eq!(lease_lost(&typed).unwrap().holder, "x");
        assert!(lease_lost(&anyhow::anyhow!("devenv-busy")).is_none());
        assert_eq!(
            LeaseLost {
                holder: "savings-app".into()
            }
            .to_string(),
            "this environment was taken over by savings-app; nothing more was written"
        );
    }

    #[test]
    fn project_ids_are_stable_and_reveal_no_path() {
        let dir = tempfile::tempdir().unwrap();
        let id = project_id(dir.path());
        assert_eq!(id, project_id(dir.path()));
        assert!(id.starts_with("proj_") && id.len() == 29, "{id}");
        assert!(!id.contains(&*dir.path().to_string_lossy()));
        let other = tempfile::tempdir().unwrap();
        assert_ne!(id, project_id(other.path()));
        assert!(holder_label(dir.path()).ends_with(" (a4 dev up)"));
    }
}
