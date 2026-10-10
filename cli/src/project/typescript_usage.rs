//! The first lines of code after `a4 install stack <package> --ts`: importing
//! the generated stack and subscribing to one of its views, spelled from the
//! files that were generated.

use std::fs;
use std::path::{Component, Path, PathBuf};

use regex::Regex;
use serde::Serialize;

use super::runtime::{TYPESCRIPT_REACT, TYPESCRIPT_SDK};
use crate::commands::auth::Framework;

/// How to use one generated TypeScript stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackUsage {
    /// The dependency alias in `arete.toml`.
    pub stack: String,
    /// The generated stack's exported definition, e.g. `ORE_STREAM_STACK`.
    pub export_name: String,
    /// The module specifier that imports it from a file in `from_dir`.
    pub import_path: String,
    /// The directory the snippet's file lives in: the app's `package.json`
    /// directory.
    pub from_dir: String,
    /// The list view the snippet subscribes to, e.g. `Round/list`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// How the snippet authenticates.
    pub auth: UsageAuth,
    /// The snippet itself, one line per entry.
    pub snippet: Vec<String>,
    /// How to run the snippet, when Node runs it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
}

/// The key a snippet authenticates with: a server-side key read from the
/// environment by the SDK itself, or an origin-bound publishable key that the
/// app's bundler exposes to browser code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageAuth {
    /// The SDK auth option: `secretKey` or `publishableKey`.
    pub option: &'static str,
    /// The environment variable that holds the key.
    pub env_var: &'static str,
    /// The command that creates the key, for a publishable key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// The kind of app a stack is used from, which decides its first lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKind {
    /// A Node service or script: `@usearete/sdk`, authenticated with a
    /// server-side key from `ARETE_API_KEY`.
    Node,
    /// A React app bundled by `Framework`: `@usearete/react`, authenticated
    /// with a publishable key bound to the app's origin.
    Browser(Framework),
}

/// The file the Node snippet is saved as.
const NODE_ENTRY: &str = "index.ts";

/// The environment variable the SDK reads a server-side key from.
const SERVER_KEY_ENV: &str = "ARETE_API_KEY";

impl AppKind {
    /// `app_dir` is where the key command must run: the app's directory as
    /// [`app_dir_from_current`] spells it, `None` for the current directory.
    fn auth(self, app_dir: Option<&str>) -> UsageAuth {
        match self {
            AppKind::Node => UsageAuth {
                option: "secretKey",
                env_var: SERVER_KEY_ENV,
                command: None,
            },
            AppKind::Browser(framework) => UsageAuth {
                option: "publishableKey",
                env_var: framework.env_var(),
                // `create-publishable` detects the framework, and so the
                // variable name, from the directory it runs in, and writes
                // the env file there: the app's own directory.
                command: Some(format!(
                    "{}a4 auth keys create-publishable --origin {} --env-file .env.local",
                    app_dir
                        .map(|dir| format!("cd {dir} && "))
                        .unwrap_or_default(),
                    dev_origin(framework)
                )),
            },
        }
    }
}

/// `app_dir` as a shell word for `cd` from `current`: `None` when they are
/// the same directory, relative when it is inside `current`, else absolute.
fn app_dir_from(app_dir: &Path, current: Option<&Path>) -> Option<String> {
    let app_dir = fs::canonicalize(app_dir).unwrap_or_else(|_| app_dir.to_path_buf());
    let current = current.and_then(|current| fs::canonicalize(current).ok());
    let path = match current
        .as_deref()
        .map(|current| app_dir.strip_prefix(current))
    {
        Some(Ok(relative)) if relative.as_os_str().is_empty() => return None,
        Some(Ok(relative)) => relative.to_path_buf(),
        _ => app_dir,
    };
    let path = path.display().to_string();
    if path
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "/._-+@:".contains(character))
    {
        Some(path)
    } else {
        Some(format!("'{}'", path.replace('\'', r"'\''")))
    }
}

/// [`app_dir_from`] the process's current directory.
fn app_dir_from_current(app_dir: &Path) -> Option<String> {
    app_dir_from(app_dir, std::env::current_dir().ok().as_deref())
}

/// The origin a framework's development server serves the app from.
fn dev_origin(framework: Framework) -> &'static str {
    match framework {
        Framework::NextJs => "http://localhost:3000",
        Framework::Vite => "http://localhost:5173",
        Framework::Generic => "<origin>",
    }
}

/// How browser code reads the publishable key the framework exposes.
fn browser_env_access(framework: Framework) -> String {
    match framework {
        Framework::Vite => format!("import.meta.env.{}", framework.env_var()),
        Framework::NextJs | Framework::Generic => format!("process.env.{}", framework.env_var()),
    }
}

/// How to use the stack generated at `output` from a file in `from_dir`, in an
/// app of kind `app`. `None` when `output` holds no stack definition (a
/// composed stack's session definition, or a program).
pub fn stack_usage(
    alias: &str,
    output: &Path,
    from_dir: &Path,
    app: AppKind,
) -> Option<StackUsage> {
    let generated = GeneratedStack::read(output)?;
    let import_path = import_specifier(from_dir, &generated.entry);
    let export = &generated.export_name;
    let view = generated.list_view.as_ref();
    let auth = app.auth(app_dir_from_current(from_dir).as_deref());
    let mut snippet = Vec::new();
    let run = match app {
        AppKind::Browser(framework) => {
            if framework == Framework::NextJs {
                snippet.push(r#""use client";"#.to_string());
                snippet.push(String::new());
            }
            snippet.push(format!(
                r#"import {{ AreteProvider, useArete }} from "{TYPESCRIPT_REACT}";"#
            ));
            snippet.push(format!(r#"import {{ {export} }} from "{import_path}";"#));
            snippet.push(String::new());
            snippet
                .push("// A publishable key bound to this app's origin, created with:".to_string());
            if let Some(command) = &auth.command {
                snippet.push(format!("// {command}"));
            }
            snippet.push(format!(
                "const publishableKey = {};",
                browser_env_access(framework)
            ));
            snippet.push(String::new());
            snippet.push("export function App() {".to_string());
            snippet.push("  return (".to_string());
            snippet.push(format!(
                "    <AreteProvider stack={{{export}}} auth={{{{ publishableKey }}}}>"
            ));
            snippet.push("      <Rows />".to_string());
            snippet.push("    </AreteProvider>".to_string());
            snippet.push("  );".to_string());
            snippet.push("}".to_string());
            snippet.push(String::new());
            snippet.push("function Rows() {".to_string());
            snippet.push(format!("  const arete = useArete({export});"));
            if let Some(view) = view {
                snippet.push(format!(
                    "  const rows = arete.views{}.use({{ take: 20 }});",
                    view.access
                ));
                snippet
                    .push("  return <pre>{JSON.stringify(rows.data, null, 2)}</pre>;".to_string());
            } else {
                snippet.push("  return <pre>{arete.status}</pre>;".to_string());
            }
            snippet.push("}".to_string());
            None
        }
        AppKind::Node => {
            snippet.push(format!(
                r#"import {{ createSession }} from "{TYPESCRIPT_SDK}";"#
            ));
            snippet.push(format!(r#"import {{ {export} }} from "{import_path}";"#));
            snippet.push(String::new());
            snippet.push(format!(
                "// No auth option: server-side, the SDK reads an agent or secret key from {SERVER_KEY_ENV}."
            ));
            snippet.push(format!(
                "const session = await createSession({{ stacks: {{ app: {export} }} }});"
            ));
            if let Some(view) = view {
                snippet.push(format!(
                    "for await (const update of session.stacks.app.views{}.watch({{ take: 20 }})) {{",
                    view.access
                ));
                snippet.push("  console.log(update);".to_string());
                snippet.push("}".to_string());
            } else {
                snippet.push("console.log(Object.keys(session.stacks.app.views));".to_string());
            }
            Some(format!("npx tsx {NODE_ENTRY}"))
        }
    };
    Some(StackUsage {
        stack: alias.to_string(),
        export_name: generated.export_name.clone(),
        import_path,
        from_dir: from_dir.display().to_string(),
        view: view.map(|view| view.path.clone()),
        auth,
        snippet,
        run,
    })
}

/// What a generated stack exports, read from its files.
#[derive(Debug, PartialEq, Eq)]
struct GeneratedStack {
    entry: PathBuf,
    export_name: String,
    list_view: Option<ListView>,
}

#[derive(Debug, PartialEq, Eq)]
struct ListView {
    /// `.Round.list`, or `["round-x"].list` for a key that is not an
    /// identifier.
    access: String,
    /// The view path, e.g. `Round/list`.
    path: String,
}

impl GeneratedStack {
    fn read(output: &Path) -> Option<Self> {
        let directory = if output.extension().and_then(|ext| ext.to_str()) == Some("ts") {
            output.parent()?.to_path_buf()
        } else {
            output.to_path_buf()
        };
        let mut files = fs::read_dir(&directory)
            .ok()?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path.extension().and_then(|ext| ext.to_str()) == Some("ts")
                    && !path.to_string_lossy().ends_with(".d.ts")
            })
            .filter_map(|path| fs::read_to_string(&path).ok().map(|text| (path, text)))
            .collect::<Vec<_>>();
        files.sort();
        let export = Regex::new(r"(?m)^export const ([A-Z_][A-Z0-9_]*_STACK)\b\s*[:=]")
            .expect("stack export regex should compile");
        let (entry, export_name) = files.iter().find_map(|(path, text)| {
            export
                .captures(text)
                .map(|captures| (path.clone(), captures[1].to_string()))
        })?;
        let core_marker = format!("export const {export_name}_CORE = {{");
        let list_view = files
            .iter()
            .find_map(|(_, text)| text.find(&core_marker).map(|at| &text[at..]))
            .and_then(first_list_view);
        Some(Self {
            entry,
            export_name,
            list_view,
        })
    }
}

/// The first list view in a generated stack definition's `views` block.
fn first_list_view(definition: &str) -> Option<ListView> {
    let entity_line = Regex::new(r"^ {4}(.+): \{$").expect("entity regex should compile");
    let list_line = Regex::new(r"^ {6}(\w+): listView<[^>]*>\('([^']+)'\)")
        .expect("list view regex should compile");
    let mut lines = definition.lines().skip_while(|line| *line != "  views: {");
    lines.next()?;
    let mut entity: Option<String> = None;
    for line in lines {
        if line.starts_with("  }") {
            break;
        }
        if let Some(captures) = entity_line.captures(line) {
            entity = Some(captures[1].to_string());
            continue;
        }
        if let (Some(entity), Some(captures)) = (&entity, list_line.captures(line)) {
            return Some(ListView {
                access: format!("{}.{}", member(entity), &captures[1]),
                path: captures[2].to_string(),
            });
        }
    }
    None
}

/// A property access for an object key as the generator writes it.
fn member(key: &str) -> String {
    let unquoted = key
        .strip_prefix('\'')
        .and_then(|key| key.strip_suffix('\''))
        .or_else(|| key.strip_prefix('"').and_then(|key| key.strip_suffix('"')));
    match unquoted {
        Some(key) => format!("[{}]", serde_json::Value::String(key.to_string())),
        None => format!(".{key}"),
    }
}

/// The ES module specifier for `entry` from a file in `from_dir`: relative,
/// with the `.js` extension Node's ES module resolution needs.
fn import_specifier(from_dir: &Path, entry: &Path) -> String {
    let relative = relative_path(from_dir, &entry.with_extension("js"));
    let relative = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    if relative.starts_with("../") {
        relative
    } else {
        format!("./{relative}")
    }
}

/// `path` relative to `base`, both absolute or both relative to the same
/// directory.
fn relative_path(base: &Path, path: &Path) -> PathBuf {
    let base = base.components().collect::<Vec<_>>();
    let path_components = path.components().collect::<Vec<_>>();
    let shared = base
        .iter()
        .zip(&path_components)
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = PathBuf::new();
    for component in &base[shared..] {
        if !matches!(component, Component::CurDir) {
            relative.push("..");
        }
    }
    for component in &path_components[shared..] {
        relative.push(component.as_os_str());
    }
    relative
}

#[cfg(test)]
mod tests {
    use super::*;

    fn golden(path: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(path)
    }

    #[test]
    fn reads_the_export_and_first_list_view_of_an_installed_stack() {
        let output = golden("installed-typescript/stacks/vault");
        let generated = GeneratedStack::read(&output).unwrap();
        assert_eq!(generated.export_name, "VAULT_STREAM_STACK");
        assert_eq!(generated.entry, output.join("vault.ts"));
        assert_eq!(
            generated.list_view,
            Some(ListView {
                access: ".Vault.list".into(),
                path: "Vault/list".into(),
            })
        );
    }

    #[test]
    fn a_composed_stack_or_program_has_no_stack_usage() {
        for output in [
            "stack-names/VaultStream/typescript-composition",
            "installed-typescript/programs/vault",
        ] {
            assert_eq!(GeneratedStack::read(&golden(output)), None, "{output}");
        }
    }

    #[test]
    fn node_usage_imports_the_sdk_and_reads_the_server_key_from_the_environment() {
        let output = golden("installed-typescript/stacks/vault");
        let app = golden("installed-typescript");
        let usage = stack_usage("vault", &output, &app, AppKind::Node).unwrap();
        assert_eq!(usage.import_path, "./stacks/vault/vault.js");
        assert_eq!(usage.view.as_deref(), Some("Vault/list"));
        assert_eq!(usage.run.as_deref(), Some("npx tsx index.ts"));
        assert_eq!(
            usage.auth,
            UsageAuth {
                option: "secretKey",
                env_var: "ARETE_API_KEY",
                command: None,
            }
        );
        assert_eq!(
            usage.snippet.join("\n"),
            r#"import { createSession } from "@usearete/sdk";
import { VAULT_STREAM_STACK } from "./stacks/vault/vault.js";

// No auth option: server-side, the SDK reads an agent or secret key from ARETE_API_KEY.
const session = await createSession({ stacks: { app: VAULT_STREAM_STACK } });
for await (const update of session.stacks.app.views.Vault.list.watch({ take: 20 })) {
  console.log(update);
}"#
        );
        assert!(!usage.snippet.join("\n").contains("publishable"));
    }

    #[test]
    fn react_usage_wraps_the_hook_in_a_provider_with_a_publishable_key() {
        let output = golden("installed-typescript/stacks/vault");
        let app = golden("installed-typescript/programs");
        let usage = stack_usage("vault", &output, &app, AppKind::Browser(Framework::Vite)).unwrap();
        assert_eq!(usage.import_path, "../stacks/vault/vault.js");
        assert_eq!(usage.run, None);
        assert_eq!(usage.auth.option, "publishableKey");
        assert_eq!(usage.auth.env_var, "VITE_ARETE_PUBLISHABLE_KEY");
        assert_eq!(
            usage.auth.command.as_deref(),
            // Tests run in the crate directory, so the command enters the
            // app's directory first.
            Some("cd tests/golden/installed-typescript/programs && a4 auth keys create-publishable --origin http://localhost:5173 --env-file .env.local")
        );
        assert_eq!(
            usage.snippet.join("\n"),
            r#"import { AreteProvider, useArete } from "@usearete/react";
import { VAULT_STREAM_STACK } from "../stacks/vault/vault.js";

// A publishable key bound to this app's origin, created with:
// cd tests/golden/installed-typescript/programs && a4 auth keys create-publishable --origin http://localhost:5173 --env-file .env.local
const publishableKey = import.meta.env.VITE_ARETE_PUBLISHABLE_KEY;

export function App() {
  return (
    <AreteProvider stack={VAULT_STREAM_STACK} auth={{ publishableKey }}>
      <Rows />
    </AreteProvider>
  );
}

function Rows() {
  const arete = useArete(VAULT_STREAM_STACK);
  const rows = arete.views.Vault.list.use({ take: 20 });
  return <pre>{JSON.stringify(rows.data, null, 2)}</pre>;
}"#
        );
    }

    #[test]
    fn next_usage_is_a_client_component_reading_the_public_env_var() {
        let output = golden("installed-typescript/stacks/vault");
        let app = golden("installed-typescript");
        let usage =
            stack_usage("vault", &output, &app, AppKind::Browser(Framework::NextJs)).unwrap();
        assert_eq!(usage.snippet[0], r#""use client";"#);
        assert!(usage.snippet.contains(
            &"const publishableKey = process.env.NEXT_PUBLIC_ARETE_PUBLISHABLE_KEY;".to_string()
        ));
        assert_eq!(
            usage.auth.command.as_deref(),
            Some("cd tests/golden/installed-typescript && a4 auth keys create-publishable --origin http://localhost:3000 --env-file .env.local")
        );
    }

    #[test]
    fn the_key_command_runs_in_the_app_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let web = root.join("apps/my web");
        fs::create_dir_all(&web).unwrap();
        assert_eq!(app_dir_from(root, Some(root)), None);
        assert_eq!(
            app_dir_from(&web, Some(root)).as_deref(),
            Some("'apps/my web'")
        );
        let elsewhere = tempfile::tempdir().unwrap();
        let absolute = fs::canonicalize(elsewhere.path()).unwrap();
        assert_eq!(
            app_dir_from(elsewhere.path(), Some(root)),
            Some(absolute.display().to_string())
        );
        let command = AppKind::Browser(Framework::Vite)
            .auth(Some("apps/web"))
            .command
            .unwrap();
        assert_eq!(
            command,
            "cd apps/web && a4 auth keys create-publishable --origin http://localhost:5173 --env-file .env.local"
        );
        assert_eq!(AppKind::Node.auth(Some("apps/web")).command, None);
    }

    #[test]
    fn view_keys_that_are_not_identifiers_use_brackets() {
        let definition = "export const X_STACK_CORE = {\n  views: {\n    'round-x': {\n      state: stateView<A, { a: string }>('round-x/state', ['a']),\n      list: listView<A>('round-x/list'),\n    },\n  },\n};\n";
        assert_eq!(
            first_list_view(definition),
            Some(ListView {
                access: r#"["round-x"].list"#.into(),
                path: "round-x/list".into(),
            })
        );
        assert_eq!(
            first_list_view("export const X_STACK_CORE = {\n  views: {\n  },\n};"),
            None
        );
    }
}
