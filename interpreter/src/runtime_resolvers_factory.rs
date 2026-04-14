use std::sync::{Arc, OnceLock};

use crate::runtime_resolvers::{InProcessResolver, SharedRuntimeResolver};

pub type ResolverBuildError = Box<dyn std::error::Error + Send + Sync>;
pub type ResolverFactory =
    Box<dyn Fn() -> Result<SharedRuntimeResolver, ResolverBuildError> + Send + Sync>;

static FACTORY: OnceLock<ResolverFactory> = OnceLock::new();

/// Register a custom resolver factory. Intended for closed-source backends
/// (e.g. a remote gRPC resolver) to inject themselves before server startup.
///
/// Only the first call takes effect; subsequent calls are ignored.
pub fn set_resolver_factory(factory: ResolverFactory) {
    let _ = FACTORY.set(factory);
}

/// Build the runtime resolver. Uses the registered factory if set, otherwise
/// falls back to `InProcessResolver::from_env()`.
pub fn build_resolver() -> Result<SharedRuntimeResolver, ResolverBuildError> {
    if let Some(factory) = FACTORY.get() {
        return factory();
    }
    Ok(Arc::new(InProcessResolver::from_env()?))
}
