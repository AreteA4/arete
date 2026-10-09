//! The first lines of code after `a4 install stack <package> --ts`: importing
//! the generated stack and subscribing to one of its views, spelled from the
//! files that were generated.

use std::fs;
use std::path::{Component, Path, PathBuf};

use regex::Regex;
use serde::Serialize;

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
    /// The snippet itself, one line per entry.
    pub snippet: Vec<String>,
    /// How to run the snippet, when Node runs it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
}

/// The file the Node snippet is saved as.
const NODE_ENTRY: &str = "index.ts";

/// How to use the stack generated at `output` from a file in `from_dir`, in a
/// Node app or, when `browser`, a React app. `None` when `output` holds no
/// stack definition (a composed stack's session definition, or a program).
pub fn stack_usage(
    alias: &str,
    output: &Path,
    from_dir: &Path,
    browser: bool,
    sdk_package: &str,
) -> Option<StackUsage> {
    let generated = GeneratedStack::read(output)?;
    let import_path = import_specifier(from_dir, &generated.entry);
    let export = &generated.export_name;
    let view = generated.list_view.as_ref();
    let mut snippet = Vec::new();
    let run = if browser {
        snippet.push(r#"import { useArete } from "@usearete/react";"#.to_string());
        snippet.push(format!(r#"import {{ {export} }} from "{import_path}";"#));
        snippet.push(String::new());
        snippet.push(format!(
            "// Render inside <AreteProvider stack={{{export}}}>."
        ));
        snippet.push("export function Rows() {".to_string());
        snippet.push(format!("  const arete = useArete({export});"));
        if let Some(view) = view {
            snippet.push(format!(
                "  const rows = arete.views{}.use({{ take: 20 }});",
                view.access
            ));
            snippet.push("  return <pre>{JSON.stringify(rows.data, null, 2)}</pre>;".to_string());
        } else {
            snippet.push("  return <pre>{arete.status}</pre>;".to_string());
        }
        snippet.push("}".to_string());
        None
    } else {
        snippet.push(format!(
            r#"import {{ createSession }} from "{sdk_package}";"#
        ));
        snippet.push(format!(r#"import {{ {export} }} from "{import_path}";"#));
        snippet.push(String::new());
        snippet.push("const publishableKey = process.env.ARETE_PUBLISHABLE_KEY;".to_string());
        snippet.push("const session = await createSession(".to_string());
        snippet.push(format!("  {{ stacks: {{ app: {export} }} }},"));
        snippet.push("  publishableKey ? { auth: { publishableKey } } : {},".to_string());
        snippet.push(");".to_string());
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
    };
    Some(StackUsage {
        stack: alias.to_string(),
        export_name: generated.export_name.clone(),
        import_path,
        from_dir: from_dir.display().to_string(),
        view: view.map(|view| view.path.clone()),
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
    fn node_usage_imports_the_stack_and_watches_a_list_view() {
        let output = golden("installed-typescript/stacks/vault");
        let app = golden("installed-typescript");
        let usage = stack_usage("vault", &output, &app, false, "@usearete/sdk").unwrap();
        assert_eq!(usage.import_path, "./stacks/vault/vault.js");
        assert_eq!(usage.view.as_deref(), Some("Vault/list"));
        assert_eq!(usage.run.as_deref(), Some("npx tsx index.ts"));
        assert_eq!(
            usage.snippet.join("\n"),
            r#"import { createSession } from "@usearete/sdk";
import { VAULT_STREAM_STACK } from "./stacks/vault/vault.js";

const publishableKey = process.env.ARETE_PUBLISHABLE_KEY;
const session = await createSession(
  { stacks: { app: VAULT_STREAM_STACK } },
  publishableKey ? { auth: { publishableKey } } : {},
);
for await (const update of session.stacks.app.views.Vault.list.watch({ take: 20 })) {
  console.log(update);
}"#
        );
    }

    #[test]
    fn react_usage_reads_the_view_with_the_hook() {
        let output = golden("installed-typescript/stacks/vault");
        let app = golden("installed-typescript/programs");
        let usage = stack_usage("vault", &output, &app, true, "@usearete/sdk").unwrap();
        assert_eq!(usage.import_path, "../stacks/vault/vault.js");
        assert_eq!(usage.run, None);
        assert!(usage
            .snippet
            .contains(&"  const rows = arete.views.Vault.list.use({ take: 20 });".to_string()));
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
