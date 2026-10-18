use anyhow::Result;
use arete_sdk::Frame;
use std::io::{self, BufWriter, Write};

pub enum OutputMode {
    Raw,
    Merged,
    NoDna,
}

/// Buffered stdout writer. Flushes after each line and on drop.
pub struct StdoutWriter {
    inner: BufWriter<io::Stdout>,
    /// The stack the stream reads (`a4 stream --dev`), added to each JSON
    /// line as `"stack"`.
    stack: Option<String>,
    /// The protocol that stack serves, added as `"protocol"`.
    protocol: Option<String>,
    /// Written before each line: `stack · View` when several views share
    /// the terminal.
    prefix: Option<String>,
}

impl StdoutWriter {
    pub fn new() -> Self {
        Self {
            inner: BufWriter::new(io::stdout()),
            stack: None,
            protocol: None,
            prefix: None,
        }
    }

    /// Label every line with `stack`.
    pub fn with_stack(mut self, stack: Option<String>) -> Self {
        self.stack = stack;
        self
    }

    /// Label every line with `protocol`.
    pub fn with_protocol(mut self, protocol: Option<String>) -> Self {
        self.protocol = protocol;
        self
    }

    /// Start every line with `prefix`.
    pub fn with_prefix(mut self, prefix: Option<String>) -> Self {
        self.prefix = prefix;
        self
    }

    pub fn writeln(&mut self, line: &str) -> Result<()> {
        match &self.prefix {
            Some(prefix) => writeln!(self.inner, "{prefix} {line}")?,
            None => writeln!(self.inner, "{line}")?,
        }
        self.inner.flush()?;
        Ok(())
    }
}

/// `stack · View`, as merged feeds label their lines.
pub fn feed_label(stack: Option<&str>, view: &str) -> String {
    match stack {
        Some(stack) => format!("{stack} \u{b7} {view}"),
        None => view.to_string(),
    }
}

impl Drop for StdoutWriter {
    fn drop(&mut self) {
        let _ = self.inner.flush();
    }
}

/// Print a raw WebSocket frame as a single JSON line to stdout.
///
/// A labelled stream (`a4 stream --dev`) adds `"stack"` and `"view"` to it.
pub fn print_raw_frame(out: &mut StdoutWriter, frame: &Frame, view: &str) -> Result<()> {
    let line = match &out.stack {
        Some(stack) => {
            let mut value = serde_json::to_value(frame)?;
            if let Some(object) = value.as_object_mut() {
                if let Some(protocol) = &out.protocol {
                    object.insert("protocol".into(), serde_json::json!(protocol));
                }
                object.insert("stack".into(), serde_json::json!(stack));
                object.insert("view".into(), serde_json::json!(view));
            }
            serde_json::to_string(&value)?
        }
        None => serde_json::to_string(frame)?,
    };
    out.writeln(&line)
}

/// `value` with the writer's `"protocol"` and `"stack"` labels, if it has
/// them.
fn labelled(out: &StdoutWriter, mut value: serde_json::Value) -> serde_json::Value {
    if let Some(object) = value.as_object_mut() {
        if let Some(protocol) = &out.protocol {
            object.insert("protocol".into(), serde_json::json!(protocol));
        }
        if let Some(stack) = &out.stack {
            object.insert("stack".into(), serde_json::json!(stack));
        }
    }
    value
}

/// Print a merged entity update as a single JSON line to stdout.
pub fn print_entity_update(
    out: &mut StdoutWriter,
    view: &str,
    key: &str,
    op: &str,
    data: &serde_json::Value,
) -> Result<()> {
    let output = labelled(
        out,
        serde_json::json!({
            "view": view,
            "key": key,
            "op": op,
            "data": data,
        }),
    );
    let line = serde_json::to_string(&output)?;
    out.writeln(&line)
}

/// Print an entity removal or deletion as a single JSON line to stdout.
pub fn print_removal(out: &mut StdoutWriter, view: &str, key: &str, op: &str) -> Result<()> {
    let output = labelled(
        out,
        serde_json::json!({
            "view": view,
            "key": key,
            "op": op,
            "data": null,
        }),
    );
    let line = serde_json::to_string(&output)?;
    out.writeln(&line)
}

/// Print a running update count to stderr (overwrites line).
pub fn print_count(count: u64) -> Result<()> {
    eprint!("\rUpdates: {}    ", count); // trailing spaces clear leftover chars
    std::io::stderr().flush()?;
    Ok(())
}

/// Move to a new line after overwriting count display.
pub fn finalize_count() {
    eprintln!();
}

/// Emit a NO_DNA envelope event as a single JSON line to stdout.
pub fn emit_no_dna_event(
    out: &mut StdoutWriter,
    action: &str,
    view: &str,
    data: &serde_json::Value,
    update_count: u64,
    entity_count: u64,
) -> Result<()> {
    let output = serde_json::json!({
        "schema": "no-dna/v1",
        "tool": "a4-stream",
        "action": action,
        "status": if action == "disconnected" || action == "error" { "done" } else { "streaming" },
        "data": labelled(out, serde_json::json!({
            "view": view,
            "payload": data,
        })),
        "meta": {
            "update_count": update_count,
            "entities_tracked": entity_count,
        },
    });
    let line = serde_json::to_string(&output)?;
    out.writeln(&line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feeds_are_labelled_stack_dot_view() {
        assert_eq!(
            feed_label(Some("tip-jar"), "ProtocolActivity/list"),
            "tip-jar \u{b7} ProtocolActivity/list"
        );
        assert_eq!(feed_label(None, "Ore/list"), "Ore/list");
    }

    #[test]
    fn a_labelled_writer_adds_the_stack() {
        let out = StdoutWriter::new().with_stack(Some("tip-jar".into()));
        assert_eq!(
            labelled(&out, serde_json::json!({"view": "A/list"})),
            serde_json::json!({"view": "A/list", "stack": "tip-jar"})
        );
        let protocol = StdoutWriter::new()
            .with_stack(Some("tip-jar".into()))
            .with_protocol(Some("localTipJar".into()));
        assert_eq!(
            labelled(&protocol, serde_json::json!({"view": "A/list"})),
            serde_json::json!({"view": "A/list", "stack": "tip-jar", "protocol": "localTipJar"})
        );
        let plain = StdoutWriter::new();
        assert_eq!(
            labelled(&plain, serde_json::json!({"view": "A/list"})),
            serde_json::json!({"view": "A/list"})
        );
    }
}
