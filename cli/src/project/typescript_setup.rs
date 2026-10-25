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

/// The import alias installed programs are imported by: `@programs/<name>`.
pub const PROGRAM_IMPORT_ALIAS: &str = "@programs/*";

/// Make installed programs importable as `@programs/<name>`: ensure the
/// project's `tsconfig.json` maps `compilerOptions.paths["@programs/*"]` to
/// `<programs_dir>/*/index.js` (each installed program directory has an
/// `index.ts`), or to the extensionless `<programs_dir>/*/index` in a project
/// that resolves like a bundler, whose bundler may not map `.js` to `.ts`
/// (see `runtime::tsconfig_resolves_like_a_bundler`). The file is edited in place, inserting only the missing key
/// and leaving everything else as written. Returns a note when it changed
/// the file, or when it could not and the user should add the line.
pub fn ensure_program_import_alias(root: &Path, programs_dir: &Path) -> Option<String> {
    let relative = programs_dir.strip_prefix(root).ok()?;
    let relative = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let extension = if super::runtime::tsconfig_resolves_like_a_bundler(root) {
        ""
    } else {
        ".js"
    };
    let target = format!("./{relative}/*/index{extension}");
    let entry = format!(
        "\"{PROGRAM_IMPORT_ALIAS}\": [{}]",
        serde_json::Value::String(target.clone())
    );
    let manual = |reason: &str| {
        Some(format!(
            "{reason}; to import installed programs as `@programs/<name>`, add to its compilerOptions: \"paths\": {{ {entry} }}"
        ))
    };
    let path = root.join("tsconfig.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return manual("No tsconfig.json at the project root");
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) else {
        return manual("tsconfig.json is not plain JSON, so a4 left it alone");
    };
    let options = &parsed["compilerOptions"];
    match options["paths"].get(PROGRAM_IMPORT_ALIAS) {
        Some(existing) if *existing == serde_json::json!([target]) => return None,
        Some(_) => {
            return Some(format!(
                "tsconfig.json already maps \"{PROGRAM_IMPORT_ALIAS}\"; a4 left it alone (installed programs are under ./{relative}/<name>/index.ts)"
            ))
        }
        None => {}
    }
    let root_object = text.find('{')?;
    let edited = if let Some(options_body) = object_member(&text, root_object, "compilerOptions") {
        match object_member(&text, options_body - 1, "paths") {
            Some(paths_body) => insert_member(&text, paths_body, &entry),
            None if options.is_object() => {
                insert_member(&text, options_body, &format!("\"paths\": {{ {entry} }}"))
            }
            None => return manual("tsconfig.json has an unexpected compilerOptions"),
        }
    } else if parsed.get("compilerOptions").is_some() {
        return manual("tsconfig.json has an unexpected compilerOptions");
    } else {
        insert_member(
            &text,
            root_object + 1,
            &format!("\"compilerOptions\": {{ \"paths\": {{ {entry} }} }}"),
        )
    };
    // Write only an edit that parses to the same document plus the alias.
    let check = serde_json::from_str::<serde_json::Value>(&edited).ok()?;
    if check["compilerOptions"]["paths"][PROGRAM_IMPORT_ALIAS] != serde_json::json!([target]) {
        return manual("a4 could not edit tsconfig.json");
    }
    if fs::write(&path, edited).is_err() {
        return manual("a4 could not write tsconfig.json");
    }
    Some(format!(
        "tsconfig.json: added compilerOptions.paths[\"{PROGRAM_IMPORT_ALIAS}\"] = [\"{target}\"], so installed programs import as `@programs/<name>`"
    ))
}

/// The byte offset just past the `{` that opens the object value of `key`,
/// a direct member of the object whose `{` is at `object`.
fn object_member(text: &str, object: usize, key: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut index = object + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let start = index + 1;
                index += 1;
                while index < bytes.len() && bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                let name = text.get(start..index)?;
                index += 1;
                if depth == 0 {
                    let mut next = index;
                    while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                        next += 1;
                    }
                    if bytes.get(next) == Some(&b':') && name == key {
                        next += 1;
                        while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                            next += 1;
                        }
                        return (bytes.get(next) == Some(&b'{')).then_some(next + 1);
                    }
                }
                continue;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Insert `member` as the first member of the object whose body starts at
/// `body`, in the layout of the members already there.
fn insert_member(text: &str, body: usize, member: &str) -> String {
    let rest = &text[body..];
    let whitespace = &rest[..rest.len() - rest.trim_start().len()];
    if rest.trim_start().starts_with('}') {
        format!("{} {member} {}", &text[..body], rest.trim_start())
    } else {
        format!("{}{whitespace}{member},{rest}", &text[..body])
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

    #[test]
    fn the_program_import_alias_is_added_in_place() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let programs = root.join("generated/typescript/programs");
        let original = "{\n  \"compilerOptions\": {\n    \"strict\": true\n  },\n  \"include\": [\"*.ts\"]\n}\n";
        fs::write(root.join("tsconfig.json"), original).unwrap();
        let note = ensure_program_import_alias(root, &programs).expect("a note");
        assert!(note.contains("added"), "{note}");
        let edited = fs::read_to_string(root.join("tsconfig.json")).unwrap();
        assert_eq!(
            edited,
            "{\n  \"compilerOptions\": {\n    \"paths\": { \"@programs/*\": [\"./generated/typescript/programs/*/index.js\"] },\n    \"strict\": true\n  },\n  \"include\": [\"*.ts\"]\n}\n"
        );
        // Idempotent.
        assert_eq!(ensure_program_import_alias(root, &programs), None);

        // An existing `paths` gains the key; other mappings stay.
        fs::write(
            root.join("tsconfig.json"),
            "{\"compilerOptions\":{\"paths\":{\"@/*\":[\"./src/*\"]}}}",
        )
        .unwrap();
        ensure_program_import_alias(root, &programs).unwrap();
        let edited: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("tsconfig.json")).unwrap()).unwrap();
        assert_eq!(
            edited["compilerOptions"]["paths"]["@/*"],
            serde_json::json!(["./src/*"])
        );
        assert_eq!(
            edited["compilerOptions"]["paths"]["@programs/*"],
            serde_json::json!(["./generated/typescript/programs/*/index.js"])
        );

        // No compilerOptions at all.
        fs::write(root.join("tsconfig.json"), "{}").unwrap();
        ensure_program_import_alias(root, &programs).unwrap();
        let edited: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("tsconfig.json")).unwrap()).unwrap();
        assert_eq!(
            edited["compilerOptions"]["paths"]["@programs/*"],
            serde_json::json!(["./generated/typescript/programs/*/index.js"])
        );
    }

    #[test]
    fn a_bundler_project_maps_installed_programs_without_an_extension() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let programs = root.join("generated/typescript/programs");
        // What `create-next-app` writes, trimmed.
        fs::write(
            root.join("tsconfig.json"),
            "{\"compilerOptions\":{\"module\":\"esnext\",\"moduleResolution\":\"bundler\",\"paths\":{\"@/*\":[\"./*\"]}}}",
        )
        .unwrap();
        ensure_program_import_alias(root, &programs).unwrap();
        let edited: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("tsconfig.json")).unwrap()).unwrap();
        assert_eq!(
            edited["compilerOptions"]["paths"]["@programs/*"],
            serde_json::json!(["./generated/typescript/programs/*/index"])
        );
        assert_eq!(ensure_program_import_alias(root, &programs), None);
    }

    #[test]
    fn a_tsconfig_a4_cannot_edit_is_left_alone_with_the_line_to_add() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let programs = root.join("generated/typescript/programs");
        let note = ensure_program_import_alias(root, &programs).unwrap();
        assert!(note.contains("No tsconfig.json"), "{note}");
        let commented = "{\n  // comment\n  \"compilerOptions\": {}\n}\n";
        fs::write(root.join("tsconfig.json"), commented).unwrap();
        let note = ensure_program_import_alias(root, &programs).unwrap();
        assert!(
            note.contains("\"@programs/*\": [\"./generated/typescript/programs/*/index.js\"]"),
            "{note}"
        );
        assert_eq!(
            fs::read_to_string(root.join("tsconfig.json")).unwrap(),
            commented
        );
        fs::write(
            root.join("tsconfig.json"),
            "{\"compilerOptions\":{\"paths\":{\"@programs/*\":[\"./elsewhere/*\"]}}}",
        )
        .unwrap();
        let note = ensure_program_import_alias(root, &programs).unwrap();
        assert!(note.contains("left it alone"), "{note}");
    }
}
