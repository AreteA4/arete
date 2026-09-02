pub mod graph;
pub mod installer;
pub mod lockfile;
pub mod manifest;
pub mod paths;
pub mod resolver;

pub use graph::InstallPlan;
pub use lockfile::ProjectLock;
pub use manifest::ProjectManifest;

pub const GENERATOR_CONTRACT: &str = "sdk-generator-v1";
pub const RESOLVER_CONTRACT: &str = "registry-resolver-v1";
