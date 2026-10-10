pub mod alias;
pub mod composition;
pub mod graph;
pub mod installer;
pub mod lockfile;
pub mod manifest;
pub mod paths;
pub mod registry_cache;
pub mod resolver;
pub mod runtime;
pub mod typescript_setup;
pub mod typescript_usage;

pub use graph::InstallPlan;
pub use lockfile::ProjectLock;
pub use manifest::ProjectManifest;

pub const GENERATOR_CONTRACT: &str = "sdk-generator-v1";
pub const RESOLVER_CONTRACT: &str = "registry-resolver-v1";
