//! The `read.*` helpers a stack's SDK extension adds, read from its source.
//!
//! A TypeScript stack extension that defines reads (`createRead`) must also
//! declare `readArgCounts`: one entry per read with the number of arguments
//! it takes (`currentRound: 0`, `round: 1`, or `[min, max]`), nested for
//! namespaced reads. That declaration is the list of `read.*` calls the
//! installed SDK exposes, so it is what this module reports, with each
//! function's parameter names and JSDoc `@title` when the source has them.
//!
//! Nothing is executed or type-checked: a source without a recognisable
//! `readArgCounts` object yields no reads, and a read whose function cannot
//! be found is reported with placeholder argument names.

use regex::Regex;
use serde_json::{json, Map, Value};

/// One read and how many arguments it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadArity {
    /// Dotted path under `read` (`currentRound`, `mining.context`).
    path: String,
    required: usize,
    max: usize,
}

/// The `read.*` calls a stack extension artifact (`{manifest: {entry},
/// files: {name: source}}`) declares, as `{call, title?}` objects:
/// `{"call": "read.round(roundId)", "title": "Round state"}`. Arguments past
/// the required count are marked optional (`arg?`). Empty when the artifact
/// carries no sources or declares no reads.
pub fn stack_extension_reads(extension: &Value) -> Vec<Value> {
    let Some(files) = extension.get("files").and_then(Value::as_object) else {
        return Vec::new();
    };
    let entry = extension.pointer("/manifest/entry").and_then(Value::as_str);
    // The entry file first, then the rest in name order.
    let mut sources: Vec<&str> = Vec::new();
    if let Some(source) = entry
        .and_then(|entry| files.get(entry))
        .and_then(Value::as_str)
    {
        sources.push(source);
    }
    for (name, source) in files {
        if Some(name.as_str()) != entry {
            if let Some(source) = source.as_str() {
                sources.push(source);
            }
        }
    }
    let Some((source, arities)) = sources
        .iter()
        .find_map(|source| read_arg_counts(source).map(|arities| (*source, arities)))
    else {
        return Vec::new();
    };
    arities
        .into_iter()
        .map(|arity| {
            let leaf = arity.path.rsplit('.').next().unwrap_or(&arity.path);
            let declared = function_signature(source, leaf);
            let names = declared
                .as_ref()
                .map(|(params, _)| params.clone())
                .filter(|params| params.len() == arity.max)
                .unwrap_or_else(|| (1..=arity.max).map(|n| format!("arg{n}")).collect());
            let params = names
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    if index >= arity.required {
                        format!("{name}?")
                    } else {
                        name.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let mut out = Map::new();
            out.insert(
                "call".into(),
                json!(format!("read.{}({params})", arity.path)),
            );
            if let Some(title) = declared.and_then(|(_, title)| title) {
                out.insert("title".into(), json!(title));
            }
            Value::Object(out)
        })
        .collect()
}

/// The entries of the `readArgCounts` object literal in `source`, in source
/// order.
fn read_arg_counts(source: &str) -> Option<Vec<ReadArity>> {
    let start = source.find("readArgCounts")?;
    let rest = &source[start + "readArgCounts".len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let body = rest.strip_prefix('{')?;
    let mut arities = Vec::new();
    parse_object(body, "", &mut arities)?;
    (!arities.is_empty()).then_some(arities)
}

/// Parse `key: 0, key: [1, 2], ns: { … } }` up to the object's closing
/// brace, appending leaves. Returns the text after the brace.
fn parse_object<'a>(mut text: &'a str, prefix: &str, out: &mut Vec<ReadArity>) -> Option<&'a str> {
    loop {
        text = skip_trivia(text);
        if let Some(rest) = text.strip_prefix('}') {
            return Some(rest);
        }
        let (key, rest) = parse_key(text)?;
        let rest = skip_trivia(rest).strip_prefix(':')?;
        let rest = skip_trivia(rest);
        let path = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        text = if let Some(nested) = rest.strip_prefix('{') {
            parse_object(nested, &path, out)?
        } else if let Some(list) = rest.strip_prefix('[') {
            let end = list.find(']')?;
            let counts = list[..end]
                .split(',')
                .map(|count| count.trim().parse::<usize>().ok())
                .collect::<Option<Vec<_>>>()?;
            let required = *counts.iter().min()?;
            let max = *counts.iter().max()?;
            out.push(ReadArity {
                path,
                required,
                max,
            });
            &list[end + 1..]
        } else {
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            let count = rest[..end].parse::<usize>().ok()?;
            out.push(ReadArity {
                path,
                required: count,
                max: count,
            });
            &rest[end..]
        };
        text = skip_trivia(text);
        text = text.strip_prefix(',').unwrap_or(text);
    }
}

fn skip_trivia(mut text: &str) -> &str {
    loop {
        text = text.trim_start();
        if let Some(rest) = text.strip_prefix("//") {
            text = rest.find('\n').map_or("", |end| &rest[end..]);
        } else if let Some(rest) = text.strip_prefix("/*") {
            text = rest.find("*/").map_or("", |end| &rest[end + 2..]);
        } else {
            return text;
        }
    }
}

/// An identifier or quoted key, and the text after it.
fn parse_key(text: &str) -> Option<(&str, &str)> {
    if let Some(quoted) = text.strip_prefix(['\'', '"']) {
        let quote = text.chars().next()?;
        let end = quoted.find(quote)?;
        return Some((&quoted[..end], &quoted[end + 1..]));
    }
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .unwrap_or(text.len());
    (end > 0).then(|| (&text[..end], &text[end..]))
}

/// The parameter names of `function name(…)` (or the method `name(…)`) in
/// `source`, and the `@title` (or first line) of the JSDoc right above it.
fn function_signature(source: &str, name: &str) -> Option<(Vec<String>, Option<String>)> {
    let name = regex::escape(name);
    // A function declaration first, then a method at the start of a line;
    // calls (`await name(…)`) match neither.
    let found = [
        format!(r"\b(?:async\s+)?function\s+{name}\s*\("),
        format!(r"(?m)^[ \t]*(?:async\s+)?{name}\s*\("),
    ]
    .iter()
    .find_map(|pattern| Regex::new(pattern).ok()?.find(source))?;
    let params_start = found.end();
    let params = balanced_params(&source[params_start..])?;
    let names = split_top_level(params)
        .into_iter()
        .filter_map(|param| {
            let param = param.trim().trim_start_matches("...");
            let end = param
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
                .unwrap_or(param.len());
            (end > 0).then(|| param[..end].to_string())
        })
        .collect();
    Some((names, jsdoc_title(&source[..found.start()])))
}

/// The text inside the parentheses that start `text` (just after `(`).
fn balanced_params(text: &str) -> Option<&str> {
    let mut depth = 0usize;
    for (index, c) in text.char_indices() {
        match c {
            '(' | '{' | '[' | '<' => depth += 1,
            ')' if depth == 0 => return Some(&text[..index]),
            ')' | '}' | ']' | '>' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

fn split_top_level(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, c) in text.char_indices() {
        match c {
            '(' | '{' | '[' | '<' => depth += 1,
            ')' | '}' | ']' | '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&text[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if !text[start..].trim().is_empty() {
        parts.push(&text[start..]);
    }
    parts
}

/// The `@title` of the JSDoc block that ends `before` (only whitespace and
/// `export` may follow it), or its first prose line.
fn jsdoc_title(before: &str) -> Option<String> {
    let trimmed = before.trim_end();
    let trimmed = trimmed.strip_suffix("export").unwrap_or(trimmed).trim_end();
    let body_end = trimmed.strip_suffix("*/")?;
    let body = &body_end[body_end.rfind("/**")? + 3..];
    let lines = body
        .lines()
        .map(|line| line.trim().trim_start_matches('*').trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    lines
        .iter()
        .find_map(|line| line.strip_prefix("@title").map(str::trim))
        .filter(|title| !title.is_empty())
        .or_else(|| lines.iter().copied().find(|line| !line.starts_with('@')))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"
export default defineStackExtensions<typeof CORE>()({
  readArgCounts: {
    boardState: 0,
    roundState: 1, // one round
    claimPreview: [1, 2],
    mining: { context: 1 },
    undeclared: 0,
  },
  createRead(client) {
    /**
     * Reads the board.
     *
     * @title Board state
     * @concept mining
     */
    async function boardState() {
      return client.views.OreBoard.state.get();
    }

    /** Reads one round. */
    async function roundState(roundId: bigint) {
      return client.views.OreRound.state.get(roundId.toString());
    }

    async function claimPreview(authority: Address, options?: { bps: number }) {
      return null;
    }

    async function context(input: { authority: Address; round: Map<string, number> }) {
      return null;
    }
    return { boardState, roundState, claimPreview, mining: { context } };
  },
});
"#;

    fn extension(source: &str) -> Value {
        json!({
            "manifest": {"entry": "demo-stack-extensions.ts"},
            "files": {"demo-stack-extensions.ts": source, "other.ts": "export {}"}
        })
    }

    #[test]
    fn reads_come_from_read_arg_counts_with_parameter_names_and_titles() {
        assert_eq!(
            stack_extension_reads(&extension(SOURCE)),
            vec![
                json!({"call": "read.boardState()", "title": "Board state"}),
                json!({"call": "read.roundState(roundId)", "title": "Reads one round."}),
                json!({"call": "read.claimPreview(authority, options?)"}),
                json!({"call": "read.mining.context(input)"}),
                json!({"call": "read.undeclared()"}),
            ]
        );
    }

    #[test]
    fn sources_without_reads_yield_nothing() {
        assert!(stack_extension_reads(&extension("export default {}")).is_empty());
        assert!(stack_extension_reads(&json!({"manifest": {"entry": "x.ts"}})).is_empty());
        assert!(stack_extension_reads(&extension("readArgCounts: { broken")).is_empty());
    }

    #[test]
    fn a_read_without_a_matching_function_gets_placeholder_arguments() {
        let source = "x({ readArgCounts: { 'quoted': [0, 2] } })";
        assert_eq!(
            stack_extension_reads(&extension(source)),
            vec![json!({"call": "read.quoted(arg1?, arg2?)"})]
        );
    }
}
