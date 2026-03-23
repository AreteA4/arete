use anyhow::Result;
use hyperstack_sdk::Frame;
use std::io::{self, Write};

/// Print a raw WebSocket frame as a single JSON line to stdout.
pub fn print_raw_frame(frame: &Frame) -> Result<()> {
    let line = serde_json::to_string(frame)?;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "{}", line)?;
    Ok(())
}

/// Print a merged entity update as a single JSON line to stdout.
/// Output format: {"view": "...", "key": "...", "op": "...", "data": {...}}
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
