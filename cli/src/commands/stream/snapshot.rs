use anyhow::{Context, Result};
use hyperstack_sdk::Frame;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Serialize, Deserialize)]
pub struct SnapshotHeader {
    pub version: u32,
    pub view: String,
    pub url: String,
    pub captured_at: String,
    pub duration_ms: u64,
    pub frame_count: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SnapshotFrame {
    pub ts: u64,
    pub frame: Frame,
}

pub struct SnapshotRecorder {
    frames: Vec<SnapshotFrame>,
    view: String,
    url: String,
    start_time: std::time::Instant,
    start_timestamp: chrono::DateTime<chrono::Utc>,
}

impl SnapshotRecorder {
    pub fn new(view: &str, url: &str) -> Self {
        Self {
            frames: Vec::new(),
            view: view.to_string(),
            url: url.to_string(),
            start_time: std::time::Instant::now(),
            start_timestamp: chrono::Utc::now(),
        }
    }

    pub fn record(&mut self, frame: &Frame) {
        let ts = self.start_time.elapsed().as_millis() as u64;
        self.frames.push(SnapshotFrame {
            ts,
            frame: frame.clone(),
        });
    }

    pub fn save(&self, path: &str) -> Result<()> {
        let duration_ms = self.start_time.elapsed().as_millis() as u64;
        let header = SnapshotHeader {
            version: 1,
            view: self.view.clone(),
            url: self.url.clone(),
            captured_at: self.start_timestamp.to_rfc3339(),
            duration_ms,
            frame_count: self.frames.len() as u64,
        };

        let output = serde_json::json!({
            "version": header.version,
            "view": header.view,
            "url": header.url,
            "captured_at": header.captured_at,
            "duration_ms": header.duration_ms,
            "frame_count": header.frame_count,
            "frames": self.frames,
        });

        let json = serde_json::to_string_pretty(&output)?;
        fs::write(path, json)
            .with_context(|| format!("Failed to write snapshot to {}", path))?;

        eprintln!(
            "Saved {} frames ({:.1}s) to {}",
            self.frames.len(),
            duration_ms as f64 / 1000.0,
            path
        );
        Ok(())
    }
}

pub struct SnapshotPlayer {
    pub header: SnapshotHeader,
    pub frames: Vec<SnapshotFrame>,
}

impl SnapshotPlayer {
    pub fn load(path: &str) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read snapshot file: {}", path))?;

        let value: serde_json::Value = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse snapshot JSON: {}", path))?;

        let header = SnapshotHeader {
            version: value.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
            view: value.get("view").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            url: value.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            captured_at: value.get("captured_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            duration_ms: value.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0),
            frame_count: value.get("frame_count").and_then(|v| v.as_u64()).unwrap_or(0),
        };

        let frames: Vec<SnapshotFrame> = value
            .get("frames")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        eprintln!(
            "Loaded snapshot: {} frames, {:.1}s, view={}, captured={}",
            frames.len(),
            header.duration_ms as f64 / 1000.0,
            header.view,
            header.captured_at,
        );

        Ok(Self { header, frames })
    }
}
