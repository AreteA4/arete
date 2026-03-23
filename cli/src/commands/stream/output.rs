use anyhow::Result;
use hyperstack_sdk::Frame;
use std::io::{self, Write};

pub enum OutputMode {
    Raw,
    Merged,
    NoDna,
}

/// Print a raw WebSocket frame as a single JSON line to stdout.
pub fn print_raw_frame(frame: &Frame) -> Result<()> {
    let line = serde_json::to_string(frame)?;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "{}", line)?;
    Ok(())
}

/// Print a merged entity update as a single JSON line to stdout.
pub fn print_entity_update(
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
    let stdout = io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "{}", line)?;
    Ok(())
}

/// Print an entity deletion as a single JSON line to stdout.
pub fn print_delete(view: &str, key: &str) -> Result<()> {
    let output = serde_json::json!({
        "view": view,
        "key": key,
        "op": "delete",
        "data": null,
    });
    let line = serde_json::to_string(&output)?;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "{}", line)?;
    Ok(())
}

/// Print a running update count to stderr (overwrites line).
pub fn print_count(count: u64) -> Result<()> {
    eprint!("\rUpdates: {}", count);
    use std::io::Write;
    std::io::stderr().flush()?;
    Ok(())
}

/// Emit a NO_DNA envelope event as a single JSON line to stdout.
pub fn emit_no_dna_event(
    action: &str,
    view: &str,
    data: &serde_json::Value,
    update_count: u64,
    entity_count: u64,
) -> Result<()> {
    let output = serde_json::json!({
        "schema": "no-dna/v1",
        "tool": "hs-stream",
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
    let stdout = io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "{}", line)?;
    Ok(())
}
