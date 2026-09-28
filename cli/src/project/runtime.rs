//! The SDK runtime that generated code needs, and what a project has
//! installed.
//!
//! `a4 install` prints the TypeScript runtime set (it never edits
//! `package.json` or runs a package manager), `a4 doctor` checks the installed
//! set, and SDK generation compares an extension's `extensionApi` with the
//! installed runtime's.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Serialize;

pub const TYPESCRIPT_SDK: &str = "@usearete/sdk";
pub const TYPESCRIPT_REACT: &str = "@usearete/react";
pub const TYPESCRIPT_ADAPTER_WEB3JS: &str = "@usearete/adapter-web3js";
pub const TYPESCRIPT_ADAPTER_KIT: &str = "@usearete/adapter-kit";
/// The TypeScript runtime packages released in lockstep with the CLI.
pub const TYPESCRIPT_LOCKSTEP: [&str; 4] = [
    TYPESCRIPT_SDK,
    TYPESCRIPT_REACT,
    TYPESCRIPT_ADAPTER_WEB3JS,
    TYPESCRIPT_ADAPTER_KIT,
];
/// Generated TypeScript imports `zod` directly; this is the range
/// `@usearete/sdk` itself declares (`typescript/core/package.json`).
pub const ZOD: &str = "zod";
pub const ZOD_RANGE: &str = "^3.24.1";

pub const RUST_SDK_CRATE: &str = "arete-a4-sdk";
pub const PYTHON_SDK_DISTRIBUTION: &str = "arete-sdk";

/// One package of a runtime set: an exact lockstep version, or the range the
/// SDK declares for a third-party dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimePackage {
    pub package: String,
    pub version: String,
}

/// The runtime release the CLI generates code for: its own lockstep version.
pub fn lockstep_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The extension API of the lockstep runtime (the same contract version in
/// every SDK language).
pub fn lockstep_extension_api() -> u32 {
    arete_sdk::EXTENSION_API_VERSION
}

/// The nearest `package.json` at or above `start`.
pub fn nearest_package_json(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|ancestor| ancestor.join("package.json"))
        .find(|candidate| candidate.is_file())
}

/// Every package a `package.json` depends on, in any dependency section.
pub fn declared_dependencies(package_json: &Path) -> BTreeSet<String> {
    let Some(value) = fs::read(package_json)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    else {
        return BTreeSet::new();
    };
    [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ]
    .iter()
    .filter_map(|section| value.get(section).and_then(serde_json::Value::as_object))
    .flat_map(|section| section.keys().cloned())
    .collect()
}

/// The TypeScript runtime set for an app that declares `declared`:
/// `@usearete/sdk`, `@usearete/react` for React apps, the wallet adapter for
/// the detected Solana client, all at the CLI's lockstep version, and `zod`.
pub fn typescript_runtime_set(declared: &BTreeSet<String>) -> Vec<RuntimePackage> {
    typescript_runtime_set_at(declared, lockstep_version())
}

/// [`typescript_runtime_set`] at another lockstep `version`.
pub fn typescript_runtime_set_at(
    declared: &BTreeSet<String>,
    version: &str,
) -> Vec<RuntimePackage> {
    let lockstep = |package: &str| RuntimePackage {
        package: package.to_string(),
        version: version.to_string(),
    };
    let mut set = vec![lockstep(TYPESCRIPT_SDK)];
    if declared.contains("react") {
        set.push(lockstep(TYPESCRIPT_REACT));
    }
    if declared.iter().any(|package| {
        package == "@solana/web3.js" || package.starts_with("@solana/wallet-adapter")
    }) {
        set.push(lockstep(TYPESCRIPT_ADAPTER_WEB3JS));
    }
    if declared.contains("@solana/kit") {
        set.push(lockstep(TYPESCRIPT_ADAPTER_KIT));
    }
    set.push(RuntimePackage {
        package: ZOD.to_string(),
        version: ZOD_RANGE.to_string(),
    });
    set
}

/// The runtime set for TypeScript outputs written to `outputs`, detected from
/// each output's nearest `package.json`. Empty when there are no outputs.
pub fn typescript_runtime_for_outputs<'a>(
    outputs: impl IntoIterator<Item = &'a Path>,
) -> Vec<RuntimePackage> {
    let mut any = false;
    let mut declared = BTreeSet::new();
    for output in outputs {
        any = true;
        declared.extend(app_dependencies(output));
    }
    if any {
        typescript_runtime_set(&declared)
    } else {
        Vec::new()
    }
}

/// The dependencies of the app that contains `output`: its nearest
/// `package.json`.
pub fn app_dependencies(output: &Path) -> BTreeSet<String> {
    nearest_package_json(output)
        .map(|package_json| declared_dependencies(&package_json))
        .unwrap_or_default()
}

/// `tsconfig.json` `module` settings under which `package.json` `"type"`
/// decides whether a `.ts` file is an ES module or CommonJS. Under
/// `commonjs` every file is CommonJS whatever `"type"` says.
const PACKAGE_TYPE_MODULE_SETTINGS: [&str; 4] = ["node16", "node18", "node20", "nodenext"];

/// The longest `extends` chain followed; a longer or cyclic one stops there.
const MAX_TSCONFIG_EXTENDS: usize = 8;

/// The CommonJS `package.json` of a TypeScript output whose `tsc` rejects ES
/// module syntax, and that `"type": "module"` would fix. Generated TypeScript
/// is ES modules (`import`/`export`), and with the `npm init` and `tsc --init`
/// defaults (no `"type": "module"`; `verbatimModuleSyntax` with a Node
/// `module` setting) `tsc` reports every `import` in the project, the
/// generated SDK's and the app's own alike. `None` when the nearest
/// `package.json` declares `"type": "module"`, or the nearest `tsconfig.json`,
/// with what it inherits through `extends`, does not compile that way.
pub fn commonjs_package_json_rejecting_imports(output: &Path) -> Option<PathBuf> {
    let package_json = nearest_package_json(output)?;
    let package: serde_json::Value = serde_json::from_slice(&fs::read(&package_json).ok()?).ok()?;
    if package.get("type").and_then(serde_json::Value::as_str) == Some("module") {
        return None;
    }
    let tsconfig = output
        .ancestors()
        .map(|ancestor| ancestor.join("tsconfig.json"))
        .find(|candidate| candidate.is_file())?;
    let verbatim = compiler_option(&tsconfig, "verbatimModuleSyntax", 0)
        == Some(serde_json::Value::Bool(true));
    let module = compiler_option(&tsconfig, "module", 0)
        .and_then(|module| module.as_str().map(str::to_ascii_lowercase));
    let by_package_type = module
        .as_deref()
        .is_some_and(|module| PACKAGE_TYPE_MODULE_SETTINGS.contains(&module));
    (verbatim && by_package_type).then_some(package_json)
}

/// A `compilerOptions` value as `tsc` resolves it: the config's own, or else
/// the one it inherits through `extends` (one config or an array, where a
/// later entry wins).
fn compiler_option(tsconfig: &Path, option: &str, depth: usize) -> Option<serde_json::Value> {
    if depth > MAX_TSCONFIG_EXTENDS {
        return None;
    }
    let config = crate::agents::jsonc::JsonDoc::parse(&fs::read_to_string(tsconfig).ok()?).ok()?;
    if let Some(value) = config.get(&["compilerOptions", option]) {
        return Some(value);
    }
    let extends = match config.get(&["extends"])? {
        serde_json::Value::String(one) => vec![one],
        serde_json::Value::Array(many) => many
            .into_iter()
            .filter_map(|entry| entry.as_str().map(str::to_string))
            .collect(),
        _ => return None,
    };
    let directory = tsconfig.parent()?;
    extends.iter().rev().find_map(|entry| {
        resolve_extends(directory, entry).and_then(|base| compiler_option(&base, option, depth + 1))
    })
}

/// The config an `extends` entry names: a path relative to the extending
/// config, or a package config in `node_modules`
/// (`@tsconfig/node20/tsconfig.json`, or a package's own `tsconfig.json`).
fn resolve_extends(directory: &Path, entry: &str) -> Option<PathBuf> {
    // As `tsc` does: the path as written, with `.json` added, or a
    // directory's `tsconfig.json`.
    let config_at = |path: PathBuf| {
        [
            path.clone(),
            PathBuf::from(format!("{}.json", path.display())),
            path.join("tsconfig.json"),
        ]
        .into_iter()
        .find(|candidate| candidate.is_file())
    };
    if entry.starts_with("./") || entry.starts_with("../") || Path::new(entry).is_absolute() {
        return config_at(directory.join(entry));
    }
    directory
        .ancestors()
        .find_map(|ancestor| config_at(ancestor.join("node_modules").join(entry)))
}

/// One copy-pasteable `npm install` line. Ranges are quoted so no shell
/// expands them.
pub fn npm_install_command(packages: &[RuntimePackage]) -> String {
    let mut command = "npm install".to_string();
    for package in packages {
        let spec = format!("{}@{}", package.package, package.version);
        let plain = spec
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '/' | '.' | '-' | '_'));
        command.push(' ');
        if plain {
            command.push_str(&spec);
        } else {
            command.push('"');
            command.push_str(&spec);
            command.push('"');
        }
    }
    command
}

/// An installed SDK runtime package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledRuntime {
    pub version: String,
    /// The extension API the installed package declares; `None` for releases
    /// that predate the contract.
    pub extension_api: Option<u32>,
    /// Where it was read, for messages.
    pub location: PathBuf,
}

/// Read an installed npm package directory (`node_modules/<name>`).
pub fn read_installed_npm_package(package_dir: &Path) -> Option<InstalledRuntime> {
    let manifest = package_dir.join("package.json");
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&manifest).ok()?).ok()?;
    Some(InstalledRuntime {
        version: value.get("version")?.as_str()?.to_string(),
        extension_api: value
            .pointer("/arete/extensionApi")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
        location: manifest,
    })
}

/// The `node_modules` directory Node resolves `package` from for code in
/// `start`: the nearest one at or above `start` that contains it.
pub fn resolve_npm_package(start: &Path, package: &str) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|ancestor| ancestor.join("node_modules").join(package))
        .find(|candidate| candidate.join("package.json").is_file())
}

/// The `@usearete/sdk` that TypeScript code in `start` resolves.
pub fn installed_typescript_sdk(start: &Path) -> Option<InstalledRuntime> {
    read_installed_npm_package(&resolve_npm_package(start, TYPESCRIPT_SDK)?)
}

/// The `arete-a4-sdk` crate a Rust package at or above `start` builds
/// against: a path dependency or `[patch]` entry, otherwise the registry
/// release its `Cargo.lock` pins, read from Cargo's source cache. `None` when
/// it cannot be located without running Cargo.
pub fn installed_rust_sdk(start: &Path) -> Option<InstalledRuntime> {
    for ancestor in start.ancestors() {
        let Some(manifest) = read_toml(&ancestor.join("Cargo.toml")) else {
            continue;
        };
        if let Some(path) = rust_sdk_path(&manifest) {
            return read_rust_sdk_manifest(&ancestor.join(path).join("Cargo.toml"));
        }
    }
    let lock = start
        .ancestors()
        .map(|ancestor| ancestor.join("Cargo.lock"))
        .find(|candidate| candidate.is_file())?;
    let version = locked_registry_version(&read_toml(&lock)?)?;
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".cargo")))?;
    let sources = fs::read_dir(cargo_home.join("registry").join("src")).ok()?;
    sources
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            entry
                .path()
                .join(format!("{RUST_SDK_CRATE}-{version}"))
                .join("Cargo.toml")
        })
        .find(|candidate| candidate.is_file())
        .and_then(|manifest| read_rust_sdk_manifest(&manifest))
}

fn read_toml(path: &Path) -> Option<toml::Table> {
    fs::read_to_string(path).ok()?.parse().ok()
}

/// The path of a local `arete-a4-sdk` in any dependency table or `[patch]`
/// section of a Cargo manifest, relative to that manifest.
fn rust_sdk_path(manifest: &toml::Table) -> Option<String> {
    let mut tables: Vec<&toml::Table> = Vec::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        tables.extend(manifest.get(section).and_then(toml::Value::as_table));
    }
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for target in targets.values().filter_map(toml::Value::as_table) {
            for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                tables.extend(target.get(section).and_then(toml::Value::as_table));
            }
        }
    }
    if let Some(workspace) = manifest.get("workspace").and_then(toml::Value::as_table) {
        tables.extend(
            workspace
                .get("dependencies")
                .and_then(toml::Value::as_table),
        );
    }
    if let Some(patches) = manifest.get("patch").and_then(toml::Value::as_table) {
        tables.extend(patches.values().filter_map(toml::Value::as_table));
    }
    tables
        .iter()
        .flat_map(|table| table.iter())
        .find_map(|(key, value)| {
            let dependency = value.as_table()?;
            let package = dependency
                .get("package")
                .and_then(toml::Value::as_str)
                .unwrap_or(key);
            (package == RUST_SDK_CRATE)
                .then(|| dependency.get("path").and_then(toml::Value::as_str))
                .flatten()
                .map(str::to_string)
        })
}

/// The single registry release of `arete-a4-sdk` a `Cargo.lock` pins.
fn locked_registry_version(lock: &toml::Table) -> Option<String> {
    let mut versions = lock
        .get("package")?
        .as_array()?
        .iter()
        .filter_map(toml::Value::as_table)
        .filter(|package| {
            package.get("name").and_then(toml::Value::as_str) == Some(RUST_SDK_CRATE)
                && package
                    .get("source")
                    .and_then(toml::Value::as_str)
                    .is_some_and(|source| source.starts_with("registry+"))
        })
        .filter_map(|package| package.get("version").and_then(toml::Value::as_str));
    let version = versions.next()?.to_string();
    versions.next().is_none().then_some(version)
}

fn read_rust_sdk_manifest(path: &Path) -> Option<InstalledRuntime> {
    let manifest = read_toml(path)?;
    let package = manifest.get("package")?.as_table()?;
    if package.get("name").and_then(toml::Value::as_str) != Some(RUST_SDK_CRATE) {
        return None;
    }
    Some(InstalledRuntime {
        version: package.get("version")?.as_str()?.to_string(),
        extension_api: package
            .get("metadata")
            .and_then(|metadata| metadata.get("arete"))
            .and_then(|arete| arete.get("extension-api"))
            .and_then(toml::Value::as_integer)
            .and_then(|value| u32::try_from(value).ok()),
        location: path.to_path_buf(),
    })
}

/// The `arete-sdk` Python distribution installed in the active virtual
/// environment (`VIRTUAL_ENV`) or a `.venv` / `venv` at or above `start`.
/// `None` when neither holds it (for example a system or editable install).
pub fn installed_python_sdk(start: &Path) -> Option<InstalledRuntime> {
    let mut environments = Vec::new();
    environments.extend(std::env::var_os("VIRTUAL_ENV").map(PathBuf::from));
    environments.extend(project_virtualenvs(start));
    environments
        .iter()
        .find_map(|environment| python_sdk_in_environment(environment))
}

fn project_virtualenvs(start: &Path) -> Vec<PathBuf> {
    start
        .ancestors()
        .flat_map(|ancestor| [ancestor.join(".venv"), ancestor.join("venv")])
        .filter(|candidate| candidate.join("pyvenv.cfg").is_file())
        .collect()
}

fn python_sdk_in_environment(environment: &Path) -> Option<InstalledRuntime> {
    let mut site_packages = vec![environment.join("Lib").join("site-packages")];
    if let Ok(entries) = fs::read_dir(environment.join("lib")) {
        let mut versions = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path().join("site-packages"))
            .collect::<Vec<_>>();
        versions.sort();
        site_packages.extend(versions);
    }
    site_packages
        .iter()
        .find_map(|site| python_sdk_in_site_packages(site))
}

fn python_sdk_in_site_packages(site: &Path) -> Option<InstalledRuntime> {
    let prefix = format!("{}-", PYTHON_SDK_DISTRIBUTION.replace('-', "_"));
    let version = fs::read_dir(site)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .find_map(|name| {
            name.strip_suffix(".dist-info")?
                .strip_prefix(&prefix)
                .map(str::to_string)
        })?;
    let module = site.join("arete").join("extensions.py");
    let source = fs::read_to_string(&module).ok()?;
    let declared = Regex::new(r"(?m)^EXTENSION_API_VERSION\s*(?::[^=]+)?=\s*(\d+)")
        .expect("extension API regex should compile");
    Some(InstalledRuntime {
        version,
        extension_api: declared
            .captures(&source)
            .and_then(|captures| captures[1].parse().ok()),
        location: module,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn zod_range_matches_the_typescript_sdk() {
        let manifest =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../typescript/core/package.json");
        let Ok(bytes) = fs::read(&manifest) else {
            return; // Packaged crate: the TypeScript SDK is not alongside.
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["dependencies"]["zod"], ZOD_RANGE);
        assert_eq!(value["version"], lockstep_version());
        assert_eq!(
            value["arete"]["extensionApi"].as_u64(),
            Some(u64::from(lockstep_extension_api()))
        );
    }

    #[test]
    fn runtime_set_follows_the_app_dependencies() {
        let version = lockstep_version();
        let names = |declared: &[&str]| {
            typescript_runtime_set(&declared.iter().map(|name| name.to_string()).collect())
                .into_iter()
                .map(|package| format!("{}@{}", package.package, package.version))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(&[]),
            vec![
                format!("@usearete/sdk@{version}"),
                "zod@^3.24.1".to_string()
            ]
        );
        assert_eq!(
            names(&["react", "@solana/wallet-adapter-react"]),
            vec![
                format!("@usearete/sdk@{version}"),
                format!("@usearete/react@{version}"),
                format!("@usearete/adapter-web3js@{version}"),
                "zod@^3.24.1".to_string(),
            ]
        );
        assert_eq!(
            names(&["@solana/kit"]),
            vec![
                format!("@usearete/sdk@{version}"),
                format!("@usearete/adapter-kit@{version}"),
                "zod@^3.24.1".to_string(),
            ]
        );
    }

    #[test]
    fn runtime_for_outputs_reads_the_nearest_package_json() {
        let temp = tempfile::tempdir().unwrap();
        write(
            &temp.path().join("app/package.json"),
            r#"{"dependencies":{"react":"^19"},"devDependencies":{"@solana/web3.js":"^1.95"}}"#,
        );
        let output = temp.path().join("app/src/arete/ore");
        let set = typescript_runtime_for_outputs([output.as_path()]);
        let packages = set.iter().map(|p| p.package.as_str()).collect::<Vec<_>>();
        assert_eq!(
            packages,
            vec![
                "@usearete/sdk",
                "@usearete/react",
                "@usearete/adapter-web3js",
                "zod"
            ]
        );
        assert!(typescript_runtime_for_outputs(std::iter::empty()).is_empty());
    }

    /// `tsc --init`'s output: JSONC with comments and a trailing comma.
    const TSC_INIT_TSCONFIG: &str = r#"{
  // Visit https://aka.ms/tsconfig to read more about this file
  "compilerOptions": {
    "module": "nodenext",
    "target": "esnext",
    "strict": true,
    "verbatimModuleSyntax": true,
    "isolatedModules": true,
    "skipLibCheck": true,
  }
}
"#;

    #[test]
    fn a_commonjs_package_under_verbatim_module_syntax_rejects_imports() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let output = root.join("generated/typescript/stacks/ore");
        write(&root.join("tsconfig.json"), TSC_INIT_TSCONFIG);

        // `npm init -y` writes `"type": "commonjs"`; no `type` is CommonJS too.
        for package in [r#"{"name":"app","type":"commonjs"}"#, r#"{"name":"app"}"#] {
            write(&root.join("package.json"), package);
            assert_eq!(
                commonjs_package_json_rejecting_imports(&output),
                Some(root.join("package.json")),
                "{package}"
            );
        }

        write(
            &root.join("package.json"),
            r#"{"name":"app","type":"module"}"#,
        );
        assert_eq!(commonjs_package_json_rejecting_imports(&output), None);
    }

    #[test]
    fn a_commonjs_package_compiling_imports_needs_no_module_type() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let output = root.join("src/generated/ore");
        write(&root.join("package.json"), r#"{"name":"app"}"#);
        // No tsconfig.json: nothing type-checks the output.
        assert_eq!(commonjs_package_json_rejecting_imports(&output), None);
        for tsconfig in [
            // Bundler projects: every file is an ES module.
            r#"{"compilerOptions":{"module":"esnext","moduleResolution":"bundler","verbatimModuleSyntax":true}}"#,
            // Without verbatimModuleSyntax, tsc compiles imports to require().
            r#"{"compilerOptions":{"module":"nodenext"}}"#,
        ] {
            write(&root.join("tsconfig.json"), tsconfig);
            assert_eq!(
                commonjs_package_json_rejecting_imports(&output),
                None,
                "{tsconfig}"
            );
        }
    }

    #[test]
    fn a_commonjs_module_setting_is_not_fixed_by_the_package_type() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let output = root.join("generated/typescript/stacks/ore");
        write(&root.join("package.json"), r#"{"name":"app"}"#);
        write(
            &root.join("tsconfig.json"),
            r#"{"compilerOptions":{"module":"commonjs","verbatimModuleSyntax":true}}"#,
        );
        assert_eq!(commonjs_package_json_rejecting_imports(&output), None);
    }

    #[test]
    fn settings_inherited_through_extends_count() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let output = root.join("generated/typescript/stacks/ore");
        write(&root.join("package.json"), r#"{"name":"app"}"#);
        write(
            &root.join("node_modules/@tsconfig/node20/tsconfig.json"),
            r#"{"compilerOptions":{"module":"node16"}}"#,
        );
        write(
            &root.join("tsconfig.base.json"),
            r#"{
  // A shared base, extending a package config.
  "extends": "@tsconfig/node20/tsconfig.json",
  "compilerOptions": { "verbatimModuleSyntax": true },
}"#,
        );

        // A relative path, with or without `.json`, and an array.
        for extends in [
            r#""./tsconfig.base.json""#,
            r#""./tsconfig.base""#,
            r#"["./missing.json", "./tsconfig.base.json"]"#,
        ] {
            write(
                &root.join("tsconfig.json"),
                &format!(r#"{{"extends":{extends},"compilerOptions":{{"strict":true}}}}"#),
            );
            assert_eq!(
                commonjs_package_json_rejecting_imports(&output),
                Some(root.join("package.json")),
                "{extends}"
            );
        }

        // The extending config's own setting wins over the inherited one.
        write(
            &root.join("tsconfig.json"),
            r#"{"extends":"./tsconfig.base.json","compilerOptions":{"verbatimModuleSyntax":false}}"#,
        );
        assert_eq!(commonjs_package_json_rejecting_imports(&output), None);
        // So does a later entry of an array over an earlier one.
        write(
            &root.join("tsconfig.esm.json"),
            r#"{"compilerOptions":{"module":"esnext"}}"#,
        );
        write(
            &root.join("tsconfig.json"),
            r#"{"extends":["./tsconfig.base.json","./tsconfig.esm.json"]}"#,
        );
        assert_eq!(commonjs_package_json_rejecting_imports(&output), None);
        // A cycle ends.
        write(
            &root.join("tsconfig.json"),
            r#"{"extends":"./tsconfig.json"}"#,
        );
        assert_eq!(commonjs_package_json_rejecting_imports(&output), None);
    }

    #[test]
    fn npm_install_command_quotes_ranges_only() {
        let command = npm_install_command(&[
            RuntimePackage {
                package: "@usearete/sdk".into(),
                version: "0.23.0".into(),
            },
            RuntimePackage {
                package: "zod".into(),
                version: "^3.24.1".into(),
            },
        ]);
        assert_eq!(command, r#"npm install @usearete/sdk@0.23.0 "zod@^3.24.1""#);
    }

    #[test]
    fn installed_typescript_sdk_reads_the_resolved_copy() {
        let temp = tempfile::tempdir().unwrap();
        write(
            &temp.path().join("node_modules/@usearete/sdk/package.json"),
            r#"{"name":"@usearete/sdk","version":"0.23.0","arete":{"extensionApi":1}}"#,
        );
        let installed = installed_typescript_sdk(&temp.path().join("src/generated")).unwrap();
        assert_eq!(installed.version, "0.23.0");
        assert_eq!(installed.extension_api, Some(1));

        write(
            &temp
                .path()
                .join("old/node_modules/@usearete/sdk/package.json"),
            r#"{"name":"@usearete/sdk","version":"0.20.3"}"#,
        );
        let older = installed_typescript_sdk(&temp.path().join("old/src")).unwrap();
        assert_eq!(older.version, "0.20.3");
        assert_eq!(older.extension_api, None);
    }

    #[test]
    fn installed_rust_sdk_follows_path_dependencies_and_patches() {
        let temp = tempfile::tempdir().unwrap();
        write(
            &temp.path().join("sdk/Cargo.toml"),
            "[package]\nname = \"arete-a4-sdk\"\nversion = \"0.23.0\"\n\n[package.metadata.arete]\nextension-api = 1\n",
        );
        write(
            &temp.path().join("app/Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\narete-sdk = { package = \"arete-a4-sdk\", version = \"0\" }\n\n[patch.crates-io]\narete-a4-sdk = { path = \"../sdk\" }\n",
        );
        let installed = installed_rust_sdk(&temp.path().join("app/src/generated")).unwrap();
        assert_eq!(installed.version, "0.23.0");
        assert_eq!(installed.extension_api, Some(1));
    }

    #[test]
    fn locked_registry_version_requires_one_registry_release() {
        let lock = |body: &str| body.parse::<toml::Table>().unwrap();
        assert_eq!(
            locked_registry_version(&lock(
                "[[package]]\nname = \"arete-a4-sdk\"\nversion = \"0.22.1\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n"
            )),
            Some("0.22.1".to_string())
        );
        assert_eq!(
            locked_registry_version(&lock(
                "[[package]]\nname = \"arete-a4-sdk\"\nversion = \"0.22.1\"\n"
            )),
            None
        );
    }

    #[test]
    fn installed_python_sdk_reads_a_project_virtualenv() {
        let temp = tempfile::tempdir().unwrap();
        let site = temp.path().join(".venv/lib/python3.12/site-packages");
        write(&temp.path().join(".venv/pyvenv.cfg"), "home = /usr/bin\n");
        write(&site.join("arete_sdk-0.23.0.dist-info/METADATA"), "");
        write(
            &site.join("arete/extensions.py"),
            "\"\"\"Extensions.\"\"\"\n\nEXTENSION_API_VERSION = 1\n",
        );
        let environments = project_virtualenvs(&temp.path().join("app"));
        let installed = python_sdk_in_environment(&environments[0]).unwrap();
        assert_eq!(installed.version, "0.23.0");
        assert_eq!(installed.extension_api, Some(1));
    }
}
