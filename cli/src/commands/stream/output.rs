use anyhow::Result;
use arete_sdk::Frame;
use std::io::{self, BufWriter, Write};

pub enum OutputMode {
    Raw,
    Merged,
    NoDna,
}

/// Buffered stdout writer. Holds a single lock for the lifetime of the stream.
/// Flushes on drop.
pub struct StdoutWriter {
    inner: BufWriter<io::Stdout>,
}

impl StdoutWriter {
    pub fn new() -> Self {
        Self {
            inner: BufWriter::new(io::stdout()),
        }
    }

    pub fn writeln(&mut self, line: &str) -> Result<()> {
        writeln!(self.inner, "{}", line)?;
        self.inner.flush()?;
        Ok(())
    }
}

impl Drop for StdoutWriter {
    fn drop(&mut self) {
        let _ = self.inner.flush();
    }
}

/// Print a raw WebSocket frame as a single JSON line to stdout.
pub fn print_raw_frame(out: &mut StdoutWriter, frame: &Frame) -> Result<()> {
    let line = serde_json::to_string(frame)?;
    out.writeln(&line)
}

/// Print a merged entity update as a single JSON line to stdout.
pub fn print_entity_update(
    out: &mut StdoutWriter,
    view: &str,
    key: &str,
    op: &str,
    data: &serde_json::Value,
) -> Result<()> {
    let output = serde_json::json!({
        "view": view,
        "key": key,
        "op": op,
        "data": data,
    });
    let line = serde_json::to_string(&output)?;
    out.writeln(&line)
}

/// Print an entity deletion as a single JSON line to stdout.
pub fn print_delete(out: &mut StdoutWriter, view: &str, key: &str) -> Result<()> {
    let output = serde_json::json!({
        "view": view,
        "key": key,
        "op": "delete",
        "data": null,
    });
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
        "data": {
            "view": view,
            "payload": data,
        },
        "meta": {
            "update_count": update_count,
            "entities_tracked": entity_count,
        },
    });
    let line = serde_json::to_string(&output)?;
    out.writeln(&line)
}
