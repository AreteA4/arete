//! Diagnostics out of a failed `arete-protocol build`'s output, for the dev
//! UI's build-failure overlay and `build.failed`.
//!
//! The build prints findings as `<file>:<line>:<column>: <message> [<code>]`
//! (source positions), `<path> @<offset>+<length>: <message> [<code>]`
//! (spec positions) or plain `error: <message>` lines; esbuild prints
//! `<file>:<line>:<column>: ERROR: <message>`.

use serde_json::{json, Value};

fn code_suffix(message: &str) -> (String, Option<String>) {
    let trimmed = message.trim();
    if let Some(start) = trimmed.rfind(" [") {
        if trimmed.ends_with(']') {
            let code = &trimmed[start + 2..trimmed.len() - 1];
            if !code.is_empty() && !code.contains(' ') {
                return (trimmed[..start].trim().to_string(), Some(code.to_string()));
            }
        }
    }
    (trimmed.to_string(), None)
}

fn source_position(line: &str) -> Option<Value> {
    // `<file>:<line>:<column>: <message>`, the file without spaces. A
    // Windows file starts with a drive (`C:\` or `C:/`), whose colon is
    // part of the file.
    let bytes = line.as_bytes();
    let drive = bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let (drive, line) = if drive { line.split_at(2) } else { ("", line) };
    let mut parts = line.splitn(4, ':');
    let file = format!("{drive}{}", parts.next()?.trim());
    let file = file.as_str();
    let line_number: u64 = parts.next()?.trim().parse().ok()?;
    let column: u64 = parts.next()?.trim().parse().ok()?;
    let rest = parts.next()?.trim();
    if file.is_empty() || file.contains(' ') {
        return None;
    }
    let rest = rest
        .strip_prefix("ERROR:")
        .or_else(|| rest.strip_prefix("error:"))
        .unwrap_or(rest);
    let (message, code) = code_suffix(rest);
    Some(json!({
        "severity": "error",
        "file": file,
        "line": line_number,
        "column": column,
        "message": message,
        "code": code,
    }))
}

fn coded_path(line: &str) -> Option<Value> {
    // `<code> at <spec pointer>: <message>`
    let (code, rest) = line.split_once(" at /")?;
    if code.is_empty()
        || !code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return None;
    }
    let (path, message) = rest.split_once(": ")?;
    Some(json!({
        "severity": "error",
        "path": format!("/{path}"),
        "message": message.trim(),
        "code": code,
    }))
}

fn pointer_message(line: &str) -> Option<Value> {
    // `/<spec pointer>: <message>`
    let rest = line.strip_prefix('/')?;
    let (path, message) = rest.split_once(": ")?;
    if path.contains(' ') {
        return None;
    }
    let (message, code) = code_suffix(message);
    Some(json!({
        "severity": "error",
        "path": format!("/{path}"),
        "message": message,
        "code": code,
    }))
}

fn spec_position(line: &str) -> Option<Value> {
    // `<path> @<offset>+<length>: <message>`
    let (path, rest) = line.split_once(" @")?;
    let (span, message) = rest.split_once(": ")?;
    let (offset, length) = span.split_once('+')?;
    let (message, code) = code_suffix(message);
    Some(json!({
        "severity": "error",
        "path": path.trim(),
        "offset": offset.parse::<u64>().ok()?,
        "length": length.parse::<u64>().ok()?,
        "message": message,
        "code": code,
    }))
}

/// The diagnostics in a build failure's text. Never empty for a non-empty
/// text: unrecognised output becomes one `error` with the first line.
pub fn parse(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("at ") {
            continue;
        }
        if let Some(diagnostic) = source_position(line).or_else(|| spec_position(line)) {
            out.push(diagnostic);
            continue;
        }
        for (prefix, severity) in [("error: ", "error"), ("warning: ", "warning")] {
            if let Some(message) = line.strip_prefix(prefix) {
                let mut diagnostic = coded_path(message)
                    .or_else(|| pointer_message(message))
                    .unwrap_or_else(|| {
                        let (message, code) = code_suffix(message);
                        json!({ "severity": severity, "message": message, "code": code })
                    });
                diagnostic["severity"] = json!(severity);
                out.push(diagnostic);
            }
        }
    }
    let errors = out.iter().filter(|d| d["severity"] == "error").count();
    if errors == 0 {
        let first = text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("`arete-protocol build` failed"))
            .or_else(|| text.lines().map(str::trim).find(|line| !line.is_empty()));
        if let Some(first) = first {
            out.push(json!({ "severity": "error", "message": first, "code": null }));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_findings_and_plain_errors() {
        let text = "`arete-protocol build` failed:\nerror: the protocol has 2 problem(s)\n  protocol.ts:41:7: unknown state 'clsed' [unknown-state]\n  steps.close.guards[0].expr @12+4: unknown identifier 'walet' [expr-unknown-identifier]\nwarning: handlers/tipJar.ts is large";
        let diagnostics = parse(text);
        assert_eq!(diagnostics.len(), 4, "{diagnostics:#?}");
        assert_eq!(diagnostics[0]["message"], "the protocol has 2 problem(s)");
        assert_eq!(diagnostics[1]["file"], "protocol.ts");
        assert_eq!(diagnostics[1]["line"], 41);
        assert_eq!(diagnostics[1]["code"], "unknown-state");
        assert_eq!(diagnostics[2]["path"], "steps.close.guards[0].expr");
        assert_eq!(diagnostics[2]["offset"], 12);
        assert_eq!(diagnostics[3]["severity"], "warning");
    }

    #[test]
    fn parses_spec_pointer_errors() {
        let coded = parse("error: dead-end-state at /machines/instance/states/paused: non-terminal state 'paused' has no exit");
        assert_eq!(coded[0]["code"], "dead-end-state");
        assert_eq!(coded[0]["path"], "/machines/instance/states/paused");
        assert_eq!(
            coded[0]["message"],
            "non-terminal state 'paused' has no exit"
        );
        let bare = parse("error: /steps/pause: handler export 'closeJar' is used by two steps");
        assert_eq!(bare[0]["path"], "/steps/pause");
        assert_eq!(
            bare[0]["message"],
            "handler export 'closeJar' is used by two steps"
        );
    }

    #[test]
    fn parses_windows_paths() {
        let windows =
            parse("C:\\work\\jar\\protocol.ts:41:7: unknown state 'clsed' [unknown-state]");
        assert_eq!(windows[0]["file"], "C:\\work\\jar\\protocol.ts");
        assert_eq!(windows[0]["line"], 41);
        assert_eq!(windows[0]["column"], 7);
        assert_eq!(windows[0]["code"], "unknown-state");
        let forward = parse("d:/jar/protocol.ts:10:2: ERROR: Expected \"}\"");
        assert_eq!(forward[0]["file"], "d:/jar/protocol.ts");
        assert_eq!(forward[0]["line"], 10);
    }

    #[test]
    fn parses_esbuild_errors_and_falls_back_to_the_first_line() {
        let esbuild = parse("protocol.ts:10:2: ERROR: Expected \"}\" but found \"steps\"");
        assert_eq!(esbuild[0]["line"], 10);
        assert_eq!(esbuild[0]["message"], "Expected \"}\" but found \"steps\"");
        let odd = parse("`arete-protocol build` failed:\nTypeError: x is not a function\n    at foo (bar.js:1:2)");
        assert_eq!(odd.len(), 1);
        assert_eq!(odd[0]["message"], "TypeError: x is not a function");
    }
}
