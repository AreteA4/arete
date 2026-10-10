//! The first lines of code after `a4 install stack <package> --ts`: importing
//! the generated stack and reading one of its views once, spelled from the
//! files that were generated, and the reads its stack extension adds.

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
    /// The list view the snippet reads, e.g. `Round/latest`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// How the snippet authenticates.
    pub auth: UsageAuth,
    /// The snippet itself, one line per entry.
    pub snippet: Vec<String>,
    /// How to run the snippet, when Node runs it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// How to stream the view's merged rows instead of reading it once
    /// (`.watch()` streams the raw updates).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
    /// The command that lists the view's fields with their units, for a
    /// registry stack: as of the registry's current release, which can be
    /// newer than the one installed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<String>,
    /// How the stack extension's reads are called, e.g.
    /// `await session.stacks.ore.read.<name>(...)`, when it has any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reads_call: Option<String>,
    /// The stack extension's reads: derived values that combine views,
    /// program accounts and chain state in one call.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<StackRead>,
}

/// One read a stack extension adds, e.g. ORE's `currentRound()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackRead {
    pub name: String,
    /// Parameter names, an optional one ending in `?`.
    pub params: Vec<String>,
    /// The read's `@title`, else the first sentence of its doc comment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl StackRead {
    /// `currentRound()`, `roundState(roundId)`.
    pub fn signature(&self) -> String {
        format!("{}({})", self.name, self.params.join(", "))
    }
}

/// The key a snippet authenticates with: a server-side key the SDK finds by
/// itself (`ARETE_API_KEY`, else the `a4` login), or an origin-bound
/// publishable key that the app's bundler exposes to browser code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageAuth {
    /// The SDK auth option: `secretKey` or `publishableKey`.
    pub option: &'static str,
    /// The environment variable that holds the key. For a server-side key it
    /// is optional: it overrides the `a4` login.
    pub env_var: &'static str,
    /// Where the SDK finds a server-side key when `env_var` is unset:
    /// `a4-login`, the key of the `a4` CLI login on this machine. So after
    /// `a4 init` or `a4 auth signup` a script needs no key setup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<&'static str>,
    /// The command that creates the key, for a publishable key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The directory to run `command` in, when it is not the current one:
    /// `a4 auth keys create-publishable` detects the framework (and so the
    /// variable name) from, and writes the env file in, the directory it runs
    /// in. Kept out of `command` so it works in any shell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
}

/// The kind of app a stack is used from, which decides its first lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKind {
    /// A Node service or script: `@usearete/sdk`, authenticated with a
    /// server-side key from `ARETE_API_KEY` or the `a4` login.
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
    fn auth(self, app_dir: Option<String>) -> UsageAuth {
        match self {
            AppKind::Node => UsageAuth {
                option: "secretKey",
                env_var: SERVER_KEY_ENV,
                fallback: Some("a4-login"),
                command: None,
                directory: None,
            },
            AppKind::Browser(framework) => UsageAuth {
                option: "publishableKey",
                env_var: framework.env_var(),
                fallback: None,
                command: Some(format!(
                    "a4 auth keys create-publishable --origin {} --env-file .env.local",
                    dev_origin(framework)
                )),
                directory: app_dir,
            },
        }
    }
}

/// `app_dir` for people, from `current`: `None` when they are the same
/// directory, relative when it is inside `current`, else absolute.
fn app_dir_from(app_dir: &Path, current: Option<&Path>) -> Option<String> {
    let app_dir = fs::canonicalize(app_dir).unwrap_or_else(|_| app_dir.to_path_buf());
    let current = current.and_then(|current| fs::canonicalize(current).ok());
    match current
        .as_deref()
        .map(|current| app_dir.strip_prefix(current))
    {
        Some(Ok(relative)) if relative.as_os_str().is_empty() => None,
        Some(Ok(relative)) => Some(relative.display().to_string()),
        _ => Some(app_dir.display().to_string()),
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
    let auth = app.auth(app_dir_from_current(from_dir));
    // The session key: the alias, when it is an identifier.
    let key = if is_identifier(alias) { alias } else { "app" };
    let mut snippet = Vec::new();
    let mut stream = None;
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
            snippet.push(match &auth.directory {
                Some(directory) => format!(
                    "// A publishable key bound to this app's origin, created by running in {directory}:"
                ),
                None => "// A publishable key bound to this app's origin, created with:".to_string(),
            });
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
                "// No auth option: server-side, the SDK uses {SERVER_KEY_ENV} if set, else your a4 login."
            ));
            snippet.push(format!(
                "const session = await createSession({{ stacks: {{ {key}: {export} }} }});"
            ));
            // One read that ends: the script exits once the session closes,
            // also when the read fails.
            snippet.push("try {".to_string());
            if let Some(view) = view {
                snippet.push(format!(
                    "  const row = await session.stacks.{key}.views{}.getOne({{ timeoutMs: 10_000 }});",
                    view.access
                ));
                snippet.push("  console.log(row);".to_string());
                stream = Some(format!(
                    "for await (const row of session.stacks.{key}.views{}.use()) {{ ... }}",
                    view.access
                ));
            } else {
                snippet.push(format!(
                    "  console.log(Object.keys(session.stacks.{key}.views));"
                ));
            }
            snippet.push("} finally {".to_string());
            snippet.push("  session.close();".to_string());
            snippet.push("}".to_string());
            Some(format!("npx tsx {NODE_ENTRY}"))
        }
    };
    let reads_call = (!generated.reads.is_empty()).then(|| match app {
        AppKind::Node => format!("await session.stacks.{key}.read.<name>(...)"),
        AppKind::Browser(_) => "arete.read.<name>.use(...)".to_string(),
    });
    Some(StackUsage {
        stack: alias.to_string(),
        export_name: generated.export_name.clone(),
        import_path,
        from_dir: from_dir.display().to_string(),
        view: view.map(|view| view.path.clone()),
        auth,
        snippet,
        run,
        stream,
        fields: None,
        reads_call,
        reads: generated.reads,
    })
}

/// Whether `name` can be written as `a.name` and `{ name: ... }`.
fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && characters
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '$'))
}

/// What a generated stack exports, read from its files.
#[derive(Debug, PartialEq, Eq)]
struct GeneratedStack {
    entry: PathBuf,
    export_name: String,
    list_view: Option<ListView>,
    reads: Vec<StackRead>,
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
        let reads = extension_entry(&directory)
            .and_then(|entry| fs::read_to_string(entry).ok())
            .map(|text| stack_reads(&text))
            .unwrap_or_default();
        Some(Self {
            entry,
            export_name,
            list_view,
            reads,
        })
    }
}

/// The list view a one-shot read starts from in a generated stack
/// definition's `views` block: the first `latest` view, else the first list
/// view.
fn first_list_view(definition: &str) -> Option<ListView> {
    let entity_line = Regex::new(r"^ {4}(.+): \{$").expect("entity regex should compile");
    let list_line = Regex::new(r"^ {6}(\w+): listView<[^>]*>\('([^']+)'\)")
        .expect("list view regex should compile");
    let mut lines = definition.lines().skip_while(|line| *line != "  views: {");
    lines.next()?;
    let mut entity: Option<String> = None;
    let mut first = None;
    for line in lines {
        if line.starts_with("  }") {
            break;
        }
        if let Some(captures) = entity_line.captures(line) {
            entity = Some(captures[1].to_string());
            continue;
        }
        if let (Some(entity), Some(captures)) = (&entity, list_line.captures(line)) {
            let view = ListView {
                access: format!("{}.{}", member(entity), &captures[1]),
                path: captures[2].to_string(),
            };
            if &captures[1] == "latest" {
                return Some(view);
            }
            first.get_or_insert(view);
        }
    }
    first
}

/// The stack extension entry beside a generated stack, named by its
/// `extensions.json`.
fn extension_entry(directory: &Path) -> Option<PathBuf> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("extensions.json")).ok()?).ok()?;
    let entry = manifest.get("entry")?.as_str()?;
    let path = directory.join(entry);
    path.is_file().then_some(path)
}

/// The most arguments a read is listed with: a larger count names no
/// placeholder parameters.
const MAX_READ_ARGS: usize = 8;

/// The reads a stack extension declares in its `readArgCounts`, with the
/// parameters and title of the function that implements each.
///
/// The extension is not lexed: every `readArgCounts: {` is a candidate, and
/// the first whose object literal parses strictly wins. A mention in a
/// comment or string does not parse as a whole object and is skipped, and
/// regex literals cannot confuse it. A comment holding a well-formed fake
/// declaration before the real one would be taken instead; these files come
/// from the SDK generator, which writes none.
fn stack_reads(extension: &str) -> Vec<StackRead> {
    let declaration =
        Regex::new(r"\breadArgCounts\s*:\s*\{").expect("read counts regex should compile");
    let Some(counts) = declaration
        .find_iter(extension)
        .find_map(|found| read_counts(&extension[found.end()..]))
    else {
        return Vec::new();
    };
    counts
        .into_iter()
        .map(|(name, count)| {
            let (params, title) = match read_function(extension, &name) {
                Some((params, title)) => (params, title),
                None => {
                    let params = match count {
                        Some(count) => (1..=count).map(|n| format!("arg{n}")).collect(),
                        None => vec!["...".to_string()],
                    };
                    (params, None)
                }
            };
            StackRead {
                name,
                params,
                title,
            }
        })
        .collect()
}

/// A strict parse of the `readArgCounts` object literal whose body starts
/// at `body` (after its `{`): identifier or quoted keys, each mapped to an
/// argument count or a `[min, max]` pair, with commas and an optional
/// trailing comma. `None` when it is anything else. A count above
/// [`MAX_READ_ARGS`] is kept as `None`: not trusted to name parameters.
fn read_counts(body: &str) -> Option<Vec<(String, Option<usize>)>> {
    let mut rest = body;
    let mut counts = Vec::new();
    loop {
        rest = rest.trim_start();
        if rest.starts_with('}') {
            return Some(counts);
        }
        let (key, after) = object_key(rest)?;
        rest = after.trim_start().strip_prefix(':')?.trim_start();
        let count = if let Some(after) = rest.strip_prefix('[') {
            let (min, after) = count_literal(after.trim_start())?;
            let after = after.trim_start().strip_prefix(',')?;
            let (max, after) = count_literal(after.trim_start())?;
            rest = after.trim_start().strip_prefix(']')?;
            min.zip(max).map(|(min, max)| min.max(max))
        } else {
            let (count, after) = count_literal(rest)?;
            rest = after;
            count
        };
        counts.push((key, count.filter(|count| *count <= MAX_READ_ARGS)));
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix(',') {
            rest = after;
        } else if !rest.starts_with('}') {
            return None;
        }
    }
}

/// An object key at the start of `text`: an identifier or a quoted string
/// without escapes.
fn object_key(text: &str) -> Option<(String, &str)> {
    if let Some(quote) = text.chars().next().filter(|c| *c == '\'' || *c == '"') {
        let inner = &text[1..];
        let end = inner.find(quote)?;
        let key = &inner[..end];
        if key.contains(['\\', '\n']) {
            return None;
        }
        return Some((key.to_string(), &inner[end + 1..]));
    }
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .unwrap_or(text.len());
    let key = &text[..end];
    is_identifier(key).then(|| (key.to_string(), &text[end..]))
}

/// A non-negative integer at the start of `text`: `Some` when it fits a
/// `usize`, `None` inside when it is too large.
fn count_literal(text: &str) -> Option<(Option<usize>, &str)> {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    (end > 0).then(|| (text[..end].parse().ok(), &text[end..]))
}

/// The parameter names and doc title of the function that implements read
/// `name`: the first `function name(` or method `name(...) {` whose
/// parameter list parses strictly. A call such as `name(x);` is not one.
fn read_function(extension: &str, name: &str) -> Option<(Vec<String>, Option<String>)> {
    let candidate = Regex::new(&format!(r"\b(function\s+)?{}\s*\(", regex::escape(name)))
        .expect("read function regex should compile");
    let found = candidate.captures_iter(extension).find_map(|captures| {
        let found = captures.get(0)?;
        let (params, after) = parameter_list(&extension[found.end()..])?;
        if captures.get(1).is_none() && !starts_body(after) {
            return None;
        }
        let params = params
            .into_iter()
            .enumerate()
            .map(|(index, param)| param_name(&param, index))
            .collect();
        let before = extension[..found.start()].trim_end();
        let before = before.strip_suffix("async").unwrap_or(before).trim_end();
        let title = before
            .strip_suffix("*/")
            .and_then(|before| before.rfind("/**").map(|at| &before[at + 3..]))
            .and_then(doc_title);
        Some((params, title))
    });
    found
}

/// The parameters of a list whose body starts at `text` (after its `(`),
/// and the text after its `)`. `None` when the brackets do not balance
/// before the end of `text`, or a parameter has no usable name.
fn parameter_list(text: &str) -> Option<(Vec<String>, &str)> {
    let mut depth = 0usize;
    let mut end = None;
    for (at, character) in text.char_indices() {
        match character {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' if depth == 0 => {
                end = Some(at);
                break;
            }
            ')' | ']' | '}' => depth = depth.checked_sub(1)?,
            // `=>` closes nothing.
            '>' if !text[..at].ends_with('=') => depth = depth.checked_sub(1)?,
            ';' if depth == 0 => return None,
            _ => {}
        }
    }
    let end = end?;
    let params = split_top_level(&text[..end]);
    let named = params.iter().all(|param| {
        let name = param.trim_start_matches("...");
        let name = name.split([':', '=']).next().unwrap_or_default().trim();
        let name = name.trim_end_matches('?');
        is_identifier(name) || name.starts_with('{') || name.starts_with('[')
    });
    named.then(|| (params, &text[end + 1..]))
}

/// Whether `text`, after a parameter list, starts a function body: an
/// optional return type, then `{`.
fn starts_body(text: &str) -> bool {
    let text = text.trim_start();
    let Some(annotation) = text.strip_prefix(':') else {
        return text.starts_with('{');
    };
    let mut depth = 0usize;
    for (at, character) in annotation.char_indices() {
        match character {
            '{' if depth == 0 => return !annotation[..at].trim().is_empty(),
            '(' | '[' | '<' | '{' => depth += 1,
            ')' | ']' | '}' => match depth.checked_sub(1) {
                Some(next) => depth = next,
                None => return false,
            },
            '>' if !annotation[..at].ends_with('=') => match depth.checked_sub(1) {
                Some(next) => depth = next,
                None => return false,
            },
            ';' | '\n' if depth == 0 => return false,
            _ => {}
        }
    }
    false
}

/// A doc comment's `@title`, else its first sentence when that is short.
fn doc_title(comment: &str) -> Option<String> {
    let lines = comment
        .lines()
        .map(|line| line.trim().trim_start_matches('*').trim())
        .collect::<Vec<_>>();
    if let Some(title) = lines.iter().find_map(|line| line.strip_prefix("@title ")) {
        return Some(title.trim().to_string());
    }
    let description = lines
        .iter()
        .take_while(|line| !line.starts_with('@'))
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    let sentence = description.split(". ").next()?.trim().trim_end_matches('.');
    (!sentence.is_empty() && sentence.len() <= 80).then(|| sentence.to_string())
}

/// `params` split at commas outside brackets.
fn split_top_level(params: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for character in params.chars() {
        match character {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(character);
    }
    parts.push(current);
    parts
        .into_iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

/// A parameter's name, `?` marking an optional one; `input` for a
/// destructured object, `args` for a destructured array.
fn param_name(param: &str, index: usize) -> String {
    let param = param.trim_start_matches("...");
    let name = param.split([':', '=']).next().unwrap_or_default().trim();
    let optional = name.ends_with('?') || param.contains(" = ");
    let name = name.trim_end_matches('?');
    let name = if is_identifier(name) {
        name.to_string()
    } else if name.starts_with('{') {
        "input".to_string()
    } else if name.starts_with('[') {
        "args".to_string()
    } else {
        format!("arg{}", index + 1)
    };
    if optional {
        format!("{name}?")
    } else {
        name
    }
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
        // Its stack extension adds defaults, no reads.
        assert_eq!(generated.reads, Vec::new());
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
                fallback: Some("a4-login"),
                command: None,
                directory: None,
            }
        );
        // One read that ends, then the stream as an alternative.
        assert_eq!(
            usage.snippet.join("\n"),
            r#"import { createSession } from "@usearete/sdk";
import { VAULT_STREAM_STACK } from "./stacks/vault/vault.js";

// No auth option: server-side, the SDK uses ARETE_API_KEY if set, else your a4 login.
const session = await createSession({ stacks: { vault: VAULT_STREAM_STACK } });
try {
  const row = await session.stacks.vault.views.Vault.list.getOne({ timeoutMs: 10_000 });
  console.log(row);
} finally {
  session.close();
}"#
        );
        assert_eq!(
            usage.stream.as_deref(),
            Some("for await (const row of session.stacks.vault.views.Vault.list.use()) { ... }")
        );
        assert!(!usage.snippet.join("\n").contains("publishable"));
        assert_eq!((usage.reads_call, usage.reads), (None, Vec::new()));
        // An alias that is not an identifier keys the session as `app`.
        let usage = stack_usage("vault-v2", &output, &app, AppKind::Node).unwrap();
        assert!(usage.snippet[4].contains("{ app: VAULT_STREAM_STACK }"));
        assert!(usage.snippet[6].starts_with("  const row = await session.stacks.app."));
    }

    #[test]
    fn node_usage_lists_the_stack_extension_reads() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("stacks/ore");
        let golden = golden("installed-typescript/stacks/vault");
        fs::create_dir_all(&output).unwrap();
        for file in ["vault.ts", "vault-core.ts"] {
            fs::copy(golden.join(file), output.join(file)).unwrap();
        }
        fs::write(
            output.join("extensions.json"),
            r#"{"entry":"ore-stack-extensions.ts","files":["ore-stack-extensions.ts"]}"#,
        )
        .unwrap();
        fs::write(
            output.join("ore-stack-extensions.ts"),
            r#"export default defineStackExtensions<typeof CORE>()({
  readArgCounts: {
    roundState: 1,
    currentRound: 0,
    claimPreview: [1, 2],
    quote: 1,
  },
  createRead(client) {
    /**
     * Reads the round entity for a round id.
     *
     * @title Round state
     */
    async function roundState(roundId: bigint) {
      return client.views.OreRound.state.get({ roundId });
    }

    /** The current round, with its phase. */
    async function currentRound() {
      return null;
    }

    async function claimPreview(
      authority: Address,
      bps: bigint | number = BPS_DENOMINATOR,
    ) {
      return null;
    }

    return { roundState, currentRound, claimPreview, quote: (input) => input };
  },
});
"#,
        )
        .unwrap();
        let usage = stack_usage("ore", &output, temp.path(), AppKind::Node).unwrap();
        assert_eq!(
            usage.reads_call.as_deref(),
            Some("await session.stacks.ore.read.<name>(...)")
        );
        let reads = usage
            .reads
            .iter()
            .map(|read| (read.signature(), read.title.as_deref()))
            .collect::<Vec<_>>();
        assert_eq!(
            reads,
            vec![
                ("roundState(roundId)".to_string(), Some("Round state")),
                (
                    "currentRound()".to_string(),
                    Some("The current round, with its phase")
                ),
                ("claimPreview(authority, bps?)".to_string(), None),
                // No function to read parameters from: the count names them.
                ("quote(arg1)".to_string(), None),
            ]
        );
        let browser = stack_usage(
            "ore",
            &output,
            temp.path(),
            AppKind::Browser(Framework::Vite),
        )
        .unwrap();
        assert_eq!(
            browser.reads_call.as_deref(),
            Some("arete.read.<name>.use(...)")
        );
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
            Some("a4 auth keys create-publishable --origin http://localhost:5173 --env-file .env.local")
        );
        // Tests run in the crate directory, so the command runs in the app's.
        assert_eq!(
            usage.auth.directory.as_deref(),
            Some("tests/golden/installed-typescript/programs")
        );
        assert_eq!(
            usage.snippet.join("\n"),
            r#"import { AreteProvider, useArete } from "@usearete/react";
import { VAULT_STREAM_STACK } from "../stacks/vault/vault.js";

// A publishable key bound to this app's origin, created by running in tests/golden/installed-typescript/programs:
// a4 auth keys create-publishable --origin http://localhost:5173 --env-file .env.local
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
            Some("a4 auth keys create-publishable --origin http://localhost:3000 --env-file .env.local")
        );
    }

    #[test]
    fn the_key_command_runs_in_the_app_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let web = root.join("apps/-web");
        fs::create_dir_all(&web).unwrap();
        assert_eq!(app_dir_from(root, Some(root)), None);
        assert_eq!(
            app_dir_from(&web, Some(root)),
            Some(Path::new("apps/-web").display().to_string())
        );
        let elsewhere = tempfile::tempdir().unwrap();
        let absolute = fs::canonicalize(elsewhere.path()).unwrap();
        assert_eq!(
            app_dir_from(elsewhere.path(), Some(root)),
            Some(absolute.display().to_string())
        );
        // The command itself stays shell-neutral: no `cd`, no `&&`.
        let auth = AppKind::Browser(Framework::Vite).auth(Some("apps/-web".into()));
        assert_eq!(
            auth.command.as_deref(),
            Some("a4 auth keys create-publishable --origin http://localhost:5173 --env-file .env.local")
        );
        assert_eq!(auth.directory.as_deref(), Some("apps/-web"));
        let node = AppKind::Node.auth(Some("apps/-web".into()));
        assert_eq!((node.command, node.directory), (None, None));
    }

    #[test]
    fn read_counts_in_comments_strings_or_too_large_are_not_trusted() {
        let extension = r#"// readArgCounts: { see below
const slashes = /[/*]/;
const quote = /'/;
const note = "readArgCounts: {";
/* currentRound(fake: string) is documented elsewhere */
export default defineStackExtensions<typeof CORE>()({
  readArgCounts: {
    currentRound: 0,
    'round-state': 1,
    example: 1000000000,
    huge: 99999999999999999999999,
    many: [1, 900],
  },
  createRead(client) {
    const id = roundState(7);
    async function currentRound(): Promise<{ id: bigint } | null> {
      return null;
    }
    return { currentRound };
  },
});
"#;
        let reads = stack_reads(extension)
            .iter()
            .map(StackRead::signature)
            .collect::<Vec<_>>();
        assert_eq!(
            reads,
            vec![
                "currentRound()",
                "round-state(arg1)",
                "example(...)",
                "huge(...)",
                "many(...)"
            ]
        );
        // A method body counts as the implementation; a call does not.
        let method =
            "readArgCounts: { quote: 1 },\nconst x = quote(input);\nasync quote(amount: bigint) {}";
        assert_eq!(stack_reads(method)[0].signature(), "quote(amount)");
        // Anything but a strict object literal is not a declaration.
        assert_eq!(read_counts("a: 1, b: x }"), None);
        assert_eq!(read_counts("a: [1, 2, 3] }"), None);
        assert_eq!(
            read_counts(" a: 0, \"b\": [1, 2], }"),
            Some(vec![("a".into(), Some(0)), ("b".into(), Some(2))])
        );
    }

    #[test]
    fn a_latest_view_is_read_before_the_first_list_view() {
        let definition = "export const X_STACK_CORE = {\n  views: {\n    Round: {\n      list: listView<Round>('Round/list'),\n      latest: listView<Round>('Round/latest'),\n    },\n  },\n};\n";
        assert_eq!(
            first_list_view(definition),
            Some(ListView {
                access: ".Round.latest".into(),
                path: "Round/latest".into(),
            })
        );
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
