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

/// Most arguments a read may declare. A declaration above it is not a
/// plausible read signature and is skipped, so a hostile or broken source
/// cannot make the summary allocate one placeholder per declared argument.
const MAX_READ_ARGS: usize = 16;

/// Most reads reported from one declaration.
const MAX_READS: usize = 256;

/// Deepest namespace nesting followed inside `readArgCounts`.
const MAX_NESTING: usize = 8;

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
        .filter(|arity| arity.max <= MAX_READ_ARGS && arity.required <= arity.max)
        .take(MAX_READS)
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

/// The entries of the first `readArgCounts: { … }` object literal in
/// `source` that parses strictly (see [`parse_object`]), in source order.
///
/// There is no JavaScript lexer here: telling a regex literal from a division
/// needs a real parser, and a lexer that guesses wrong (a `/'/g` literal
/// opening a "string") hides every read. Instead every whole-identifier
/// occurrence followed by `:` and `{` is a candidate, and the first whose
/// object parses strictly wins. A prose mention (`// readArgCounts describes
/// optional arguments`) is not followed by an object and is skipped. The
/// tradeoff: a comment or string holding a well-formed `readArgCounts: {
/// x: 1 }` before the real declaration would be picked. Hosted extension
/// files come from the SDK codegen, which emits no such text.
fn read_arg_counts(source: &str) -> Option<Vec<ReadArity>> {
    identifier_occurrences(source, "readArgCounts")
        .into_iter()
        .find_map(|start| {
            let rest = source[start + "readArgCounts".len()..].trim_start();
            let body = rest.strip_prefix(':')?.trim_start().strip_prefix('{')?;
            let mut arities = Vec::new();
            parse_object(body, "", 0, &mut arities)?;
            (!arities.is_empty() && arities.len() <= MAX_READS).then_some(arities)
        })
}

/// Byte offsets of `word` as a whole identifier in `source`.
fn identifier_occurrences(source: &str, word: &str) -> Vec<usize> {
    let bytes = source.as_bytes();
    let is_ident = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$';
    source
        .match_indices(word)
        .map(|(index, _)| index)
        .filter(|&index| {
            (index == 0 || !is_ident(bytes[index - 1]))
                && bytes
                    .get(index + word.len())
                    .is_none_or(|&next| !is_ident(next))
        })
        .collect()
}

/// Strictly parse `key: 0, key: [1, 2], ns: { … }, }` up to the object's
/// closing brace, appending leaves. Keys are identifiers or quoted strings
/// without escapes; values are an argument count of at most
/// [`MAX_READ_ARGS`], a `[min, max]` pair of them, or a nested object (at
/// most [`MAX_NESTING`] deep). Entries are separated by commas, with an
/// optional trailing comma; only whitespace may appear between tokens.
/// Anything else fails the whole candidate. Returns the text after the brace.
fn parse_object<'a>(
    mut text: &'a str,
    prefix: &str,
    depth: usize,
    out: &mut Vec<ReadArity>,
) -> Option<&'a str> {
    if depth > MAX_NESTING {
        return None;
    }
    loop {
        text = text.trim_start();
        if let Some(rest) = text.strip_prefix('}') {
            return Some(rest);
        }
        if out.len() >= MAX_READS {
            return None;
        }
        let (key, rest) = parse_key(text)?;
        let rest = rest.trim_start().strip_prefix(':')?.trim_start();
        let path = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        text = if let Some(nested) = rest.strip_prefix('{') {
            parse_object(nested, &path, depth + 1, out)?
        } else if let Some(list) = rest.strip_prefix('[') {
            let (first, list) = parse_count(list.trim_start())?;
            let list = list.trim_start().strip_prefix(',')?.trim_start();
            let (second, list) = parse_count(list)?;
            let list = list.trim_start().strip_prefix(']')?;
            out.push(ReadArity {
                path,
                required: first.min(second),
                max: first.max(second),
            });
            list
        } else {
            let (count, rest) = parse_count(rest)?;
            out.push(ReadArity {
                path,
                required: count,
                max: count,
            });
            rest
        };
        text = text.trim_start();
        match text.strip_prefix(',') {
            Some(rest) => text = rest,
            None if text.starts_with('}') => {}
            None => return None,
        }
    }
}

/// A small non-negative argument count (at most [`MAX_READ_ARGS`]) and the
/// text after it.
fn parse_count(text: &str) -> Option<(usize, &str)> {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    if end == 0 || end > 2 {
        return None;
    }
    let count = text[..end].parse::<usize>().ok()?;
    (count <= MAX_READ_ARGS).then_some((count, &text[end..]))
}

/// An identifier or a quoted key without escapes, and the text after it.
fn parse_key(text: &str) -> Option<(&str, &str)> {
    if let Some(quote) = text.chars().next().filter(|c| *c == '\'' || *c == '"') {
        let quoted = &text[1..];
        let end = quoted.find([quote, '\\', '\n'])?;
        if !quoted[end..].starts_with(quote) || end == 0 {
            return None;
        }
        return Some((&quoted[..end], &quoted[end + 1..]));
    }
    if !text
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
    {
        return None;
    }
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .unwrap_or(text.len());
    Some((&text[..end], &text[end..]))
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
    roundState: 1,
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

    #[test]
    fn unreasonable_declarations_are_refused_without_allocating() {
        for source in [
            "x({ readArgCounts: { huge: 1000000000, ok: 1 } })",
            "x({ readArgCounts: { range: [0, 4000000000] } })",
            "x({ readArgCounts: { tooMany: 17 } })",
        ] {
            assert!(
                stack_extension_reads(&extension(source)).is_empty(),
                "{source}"
            );
        }
        let deep = format!(
            "readArgCounts: {}a: 0{}",
            "{ n: ".repeat(64),
            " }".repeat(65)
        );
        assert!(stack_extension_reads(&extension(&deep)).is_empty());
        let wide = format!(
            "readArgCounts: {{ {} }}",
            (0..300)
                .map(|n| format!("r{n}: 0"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert!(stack_extension_reads(&extension(&wide)).is_empty());
    }

    #[test]
    fn regex_literals_and_comment_mentions_do_not_hide_the_declaration() {
        let source = r#"
// readArgCounts describes optional arguments — même ici: «readArgCounts»
const apostrophe = /'/g;
const opener = /\/*/;
const quote = /"/;
const myreadArgCounts = 1;
export default defineStackExtensions()({
  readArgCounts: { currentRound: 0, 'quoted': [2, 1], },
});
"#;
        assert_eq!(
            stack_extension_reads(&extension(source)),
            vec![
                json!({"call": "read.currentRound()"}),
                json!({"call": "read.quoted(arg1, arg2?)"}),
            ]
        );
    }

    #[test]
    fn the_first_candidate_that_parses_strictly_wins() {
        // Not an object, or not a strict one: skipped.
        let source = "/* readArgCounts: { see the docs } */ readArgCounts: { a: 0 b: 1 } \
                      readArgCounts: { real: 1 }";
        assert_eq!(
            stack_extension_reads(&extension(source)),
            vec![json!({"call": "read.real(arg1)"})]
        );
        // The accepted tradeoff: a well-formed declaration in a comment that
        // comes first is picked.
        let source = "// readArgCounts: { x: 1 }\nreadArgCounts: { real: 0 }";
        assert_eq!(
            stack_extension_reads(&extension(source)),
            vec![json!({"call": "read.x(arg1)"})]
        );
    }
}
