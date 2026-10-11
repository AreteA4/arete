//! `a4 install --setup`: make a directory with no `package.json` a Node
//! TypeScript project that type-checks and runs the generated SDK. It writes
//! the missing `package.json` (ES modules) and `tsconfig.json`, never
//! replacing a file that exists, and installs the packages with npm.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde::Serialize;

use super::runtime::{self, RuntimePackage};

/// The command that performs the setup from the project root.
pub const SETUP_COMMAND: &str = "a4 install --setup";

/// The `tsc --init` that creates, by hand, a `tsconfig.json` with the
/// options of [`NODE_COMPILER_OPTIONS`] the generated SDK depends on: Node ES
/// module resolution (which follows its `.js` specifiers and allows
/// top-level `await`) and Node's types.
pub const TSC_INIT_COMMAND: &str = "npx tsc --init --module nodenext --target es2022 --types node";

/// The `compilerOptions` of the `tsconfig.json` setup writes: Node ES
/// modules with Node's types. The generated SDK is ES module source with
/// `.js` import specifiers, which `NodeNext` resolution follows; `tsx` runs it
/// as is.
const NODE_COMPILER_OPTIONS: &str = r#"  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022"],
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "types": ["node"],
    "strict": true,
    "isolatedModules": true,
    "skipLibCheck": true,
    "noEmit": true
  }"#;

/// A `tsconfig.json` for the Node app in `directory` that covers only its
/// top-level `.ts` files (the `index.ts` entry), `src/`, and the generated
/// TypeScript `outputs`, so other apps beneath it, such as a React app with
/// its own config, are not type-checked as Node code. List the project's
/// TypeScript output directory first: stacks installed later land there too.
fn node_tsconfig(directory: &Path, outputs: &[PathBuf]) -> String {
    let mut include = vec!["*.ts".to_string(), "src/**/*.ts".to_string()];
    let mut covered: Vec<&Path> = Vec::new();
    for output in outputs {
        let dir = if output.extension().and_then(|ext| ext.to_str()) == Some("ts") {
            output.parent().unwrap_or(output)
        } else {
            output.as_path()
        };
        if covered.iter().any(|covered| dir.starts_with(covered)) {
            continue;
        }
        let Ok(relative) = dir.strip_prefix(directory) else {
            continue;
        };
        covered.push(dir);
        let relative = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let pattern = if relative.is_empty() {
            "**/*.ts".to_string()
        } else {
            format!("{relative}/**/*.ts")
        };
        if !include.contains(&pattern) {
            include.push(pattern);
        }
    }
    let include = include
        .iter()
        .map(|pattern| serde_json::Value::String(pattern.clone()).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{\n{NODE_COMPILER_OPTIONS},\n  \"include\": [{include}]\n}}\n")
}

/// What `--setup` did in one directory.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupReport {
    pub directory: String,
    /// Files written: `package.json`, and `tsconfig.json` when no
    /// `tsconfig.json` was at or above the directory.
    pub created: Vec<String>,
    /// Package manager commands that succeeded, in order.
    pub ran: Vec<String>,
    /// The command that failed, with why. Later commands did not run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
}

impl SetupReport {
    pub fn emit(&self, display_directory: &str) {
        if !self.created.is_empty() {
            let names = self
                .created
                .iter()
                .map(|path| {
                    Path::new(path)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.clone())
                })
                .collect::<Vec<_>>();
            println!(
                "Setup:       created {} in {display_directory}",
                names.join(", ")
            );
        }
        for command in &self.ran {
            println!("Setup:       ran {command}");
        }
        if let Some(failed) = &self.failed {
            crate::ui::print_warning(&format!(
                "Setup did not finish: {failed}. Run the remaining steps below by hand."
            ));
        }
    }
}

/// Set up `directory`, which has no `package.json`, for the generated
/// TypeScript `outputs` beneath it: write the project files, then install
/// `runtime` and the Node development tools.
pub fn set_up(
    directory: &Path,
    outputs: &[PathBuf],
    runtime: &[RuntimePackage],
    json: bool,
) -> SetupReport {
    let mut report = SetupReport {
        directory: directory.display().to_string(),
        created: Vec::new(),
        ran: Vec::new(),
        failed: None,
    };
    let mut created = Vec::new();
    let written = write_project_files(directory, outputs, &mut created);
    // Files written before a failure are still reported.
    report.created = created
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    if let Err(error) = written {
        report.failed = Some(format!("{error:#}"));
        return report;
    }
    let dev_tools = runtime::typescript_dev_tools(&Default::default());
    for args in npm_install_args(runtime, &dev_tools) {
        let command = format!("npm {}", args.join(" "));
        match run_npm(directory, &args, json) {
            Ok(()) => report.ran.push(command),
            Err(error) => {
                report.failed = Some(format!("{command}: {error:#}"));
                break;
            }
        }
    }
    report
}

/// Write `package.json` and, when no `tsconfig.json` is at or above
/// `directory`, a `tsconfig.json` covering `outputs`. Existing files are left
/// alone. Each file written is pushed to `created`, also when a later write
/// fails.
pub fn write_project_files(
    directory: &Path,
    outputs: &[PathBuf],
    created: &mut Vec<PathBuf>,
) -> Result<()> {
    let package_json = directory.join("package.json");
    if write_new(&package_json, &package_json_contents(directory))? {
        created.push(package_json);
    }
    if runtime::nearest_tsconfig(directory).is_none() {
        let tsconfig = directory.join("tsconfig.json");
        if write_new(&tsconfig, &node_tsconfig(directory, outputs))? {
            created.push(tsconfig);
        }
    }
    Ok(())
}

/// Create `path` with `contents` unless it exists. `true` when written.
fn write_new(path: &Path, contents: &str) -> Result<bool> {
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        // An existing file is kept; anything else in its place is an error.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && path.is_file() => {
            return Ok(false)
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to create {}", path.display()))
        }
    };
    file.write_all(contents.as_bytes())
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(true)
}

/// The file a `start` script runs, when the directory has one.
const NODE_ENTRY: &str = "index.ts";

/// A private ES module package named after `directory`, with a script that
/// type-checks the project, and one that runs `index.ts` when that file
/// exists: a script for a file that is not there fails when it is run.
/// npm adds the dependencies.
fn package_json_contents(directory: &Path) -> String {
    let name = serde_json::Value::String(package_name(directory));
    let start = if directory.join(NODE_ENTRY).is_file() {
        format!("\n    \"start\": \"tsx {NODE_ENTRY}\",")
    } else {
        String::new()
    };
    format!(
        r#"{{
  "name": {name},
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {{{start}
    "typecheck": "tsc --noEmit"
  }}
}}
"#
    )
}

/// An npm package name from a directory name: lower case, with runs of other
/// characters replaced by `-`.
fn package_name(directory: &Path) -> String {
    let base = directory
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut name = String::new();
    for character in base.chars() {
        if character.is_ascii_alphanumeric() {
            name.push(character);
        } else if !name.is_empty() && !name.ends_with('-') {
            name.push('-');
        }
    }
    let name = name.trim_end_matches('-');
    if name.is_empty() {
        "arete-app".to_string()
    } else {
        name.to_string()
    }
}

/// What `--setup` does, as commands to run by hand in a directory with no
/// `package.json`, in the order that works there: an ES module package, the
/// runtime and dev tools it installs, then, when `tsconfig` is set, a
/// `tsconfig.json`.
pub fn manual_commands(
    runtime: &[RuntimePackage],
    dev_tools: &[String],
    tsconfig: bool,
) -> Vec<String> {
    let mut commands = vec![
        "npm init -y".to_string(),
        "npm pkg set type=module".to_string(),
    ];
    commands.extend(npm_install_args(runtime, dev_tools).iter().map(|args| {
        let args = args
            .iter()
            .map(|arg| shell_word(arg))
            .collect::<Vec<_>>()
            .join(" ");
        format!("npm {args}")
    }));
    if tsconfig {
        commands.push(TSC_INIT_COMMAND.to_string());
    }
    commands
}

/// `arg` as one word in any common shell: quoted when it holds characters
/// such as `^`.
fn shell_word(arg: &str) -> String {
    if arg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '/' | '.' | '-' | '_'))
    {
        arg.to_string()
    } else {
        format!("\"{arg}\"")
    }
}

/// The npm argument lists that install `runtime` and then `dev_tools`.
fn npm_install_args(runtime: &[RuntimePackage], dev_tools: &[String]) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    if !runtime.is_empty() {
        let mut args = vec!["install".to_string()];
        args.extend(
            runtime
                .iter()
                .map(|package| format!("{}@{}", package.package, package.version)),
        );
        commands.push(args);
    }
    if !dev_tools.is_empty() {
        let mut args = vec!["install".to_string(), "-D".to_string()];
        args.extend(dev_tools.iter().cloned());
        commands.push(args);
    }
    commands
}

/// Run npm in `directory` without a terminal to prompt on. Under `--json`
/// its output goes to stderr so stdout carries only the report.
fn run_npm(directory: &Path, args: &[String], json: bool) -> Result<()> {
    let stdout = if json {
        Stdio::from(std::io::stderr())
    } else {
        Stdio::inherit()
    };
    let status = Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" })
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::inherit())
        .status()
        .context("could not start npm")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("exit code {}", status.code().unwrap_or(-1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(directory: &Path, outputs: &[PathBuf]) -> Vec<PathBuf> {
        let mut created = Vec::new();
        write_project_files(directory, outputs, &mut created).unwrap();
        created
    }

    #[test]
    fn writes_an_es_module_package_and_a_node_tsconfig() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("My Ore_App!");
        fs::create_dir(&root).unwrap();
        // The project's output directory, then outputs: one inside it,
        // one configured elsewhere.
        let outputs = [
            root.join("./generated/typescript"),
            root.join("generated/typescript/stacks/ore"),
            root.join("src/ore-sdk.ts"),
        ];
        let created = write(&root, &outputs);
        assert_eq!(
            created,
            vec![root.join("package.json"), root.join("tsconfig.json")]
        );
        let package: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("package.json")).unwrap()).unwrap();
        assert_eq!(package["name"], "my-ore-app");
        assert_eq!(package["type"], "module");
        assert_eq!(package["private"], true);
        // Type-checking always works; there is no `index.ts` to start.
        assert_eq!(package["scripts"]["typecheck"], "tsc --noEmit");
        assert!(package["scripts"].get("start").is_none(), "{package}");
        let tsconfig: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("tsconfig.json")).unwrap()).unwrap();
        assert_eq!(tsconfig["compilerOptions"]["module"], "NodeNext");
        assert_eq!(tsconfig["compilerOptions"]["moduleResolution"], "NodeNext");
        assert_eq!(
            tsconfig["compilerOptions"]["types"],
            serde_json::json!(["node"])
        );
        // Only the entry and the generated outputs, including stacks installed
        // later into the output directory: not other apps beneath.
        assert_eq!(
            tsconfig["include"],
            serde_json::json!(["*.ts", "src/**/*.ts", "generated/typescript/**/*.ts"])
        );
        // The written tsconfig is one the install guidance accepts.
        assert_eq!(runtime::tsconfig_hiding_node_types(&root), None);
        assert_eq!(
            runtime::commonjs_package_json_rejecting_imports(&root),
            None
        );
    }

    #[test]
    fn starts_an_existing_entry() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("index.ts"), "console.log(1);\n").unwrap();
        write(root, &[]);
        let package: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("package.json")).unwrap()).unwrap();
        assert_eq!(
            package["scripts"],
            serde_json::json!({"start": "tsx index.ts", "typecheck": "tsc --noEmit"})
        );
    }

    #[test]
    fn never_replaces_existing_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("package.json"), "{}").unwrap();
        fs::write(root.join("tsconfig.json"), "{}").unwrap();
        assert!(write(root, &[]).is_empty());
        assert_eq!(fs::read_to_string(root.join("package.json")).unwrap(), "{}");
        assert_eq!(
            fs::read_to_string(root.join("tsconfig.json")).unwrap(),
            "{}"
        );
    }

    #[test]
    fn a_tsconfig_above_the_directory_is_kept() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("tsconfig.json"), "{}").unwrap();
        let app = temp.path().join("app");
        fs::create_dir(&app).unwrap();
        assert_eq!(write(&app, &[]), vec![app.join("package.json")]);
    }

    #[test]
    fn installs_the_runtime_then_the_dev_tools() {
        let runtime = runtime::typescript_runtime_set_at(&Default::default(), "0.33.0");
        let dev_tools = runtime::typescript_dev_tools(&Default::default());
        assert_eq!(
            npm_install_args(&runtime, &dev_tools),
            vec![
                vec!["install", "@usearete/sdk@0.33.0", "zod@^3.24.1"],
                vec!["install", "-D", "typescript", "tsx", "@types/node"],
            ]
        );
    }

    #[test]
    fn manual_commands_are_the_setup_steps_in_order() {
        let runtime = runtime::typescript_runtime_set_at(&Default::default(), "0.33.0");
        let dev_tools = runtime::typescript_dev_tools(&Default::default());
        assert_eq!(
            manual_commands(&runtime, &dev_tools, true),
            vec![
                "npm init -y",
                "npm pkg set type=module",
                r#"npm install @usearete/sdk@0.33.0 "zod@^3.24.1""#,
                "npm install -D typescript tsx @types/node",
                TSC_INIT_COMMAND,
            ]
        );
        // The runtime line is the one the rest of the install prints.
        assert_eq!(
            manual_commands(&runtime, &[], false)[2],
            runtime::npm_install_command(&runtime)
        );
    }

    #[test]
    fn the_tsc_init_command_sets_the_options_setup_writes() {
        let options: serde_json::Value =
            serde_json::from_str(&format!("{{\n{NODE_COMPILER_OPTIONS}\n}}")).unwrap();
        let options = &options["compilerOptions"];
        let (_, flags) = TSC_INIT_COMMAND.split_once("--init ").unwrap();
        let flags = flags.split(" --").collect::<Vec<_>>();
        assert_eq!(flags.len(), 3);
        for flag in flags {
            let (name, value) = flag.trim_start_matches("--").split_once(' ').unwrap();
            let expected = match &options[name] {
                serde_json::Value::Array(values) => values[0].as_str().unwrap(),
                value => value.as_str().unwrap(),
            };
            assert!(expected.eq_ignore_ascii_case(value), "--{name} {value}");
        }
    }

    #[test]
    fn package_names_fall_back_for_unusable_directory_names() {
        assert_eq!(package_name(Path::new("/x/__")), "arete-app");
        assert_eq!(package_name(Path::new("/x/ore-bot")), "ore-bot");
    }

    #[test]
    fn files_written_before_a_failure_are_reported() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        // A directory where tsconfig.json would go makes that write fail
        // after package.json is written.
        fs::create_dir(root.join("tsconfig.json")).unwrap();
        let report = set_up(root, &[], &[], true);
        assert_eq!(
            report.created,
            vec![root.join("package.json").display().to_string()]
        );
        assert!(report.ran.is_empty());
        assert!(report.failed.is_some());
    }
}
