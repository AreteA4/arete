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

    const MAX_FRAMES: usize = 100_000;

    pub fn record(&mut self, frame: &Frame) {
        if self.frames.len() == Self::MAX_FRAMES {
            eprintln!(
                "Warning: snapshot recorder reached {} frames limit. Further frames will be dropped. \
                 Use --duration to limit recording time.",
                Self::MAX_FRAMES
            );
        }
        if self.frames.len() >= Self::MAX_FRAMES {
            return;
        }
        let ts = self.start_time.elapsed().as_millis() as u64;
        self.frames.push(SnapshotFrame {
            ts,
            frame: frame.clone(),
        });
    }

    #[cfg(feature = "tui")]
    pub fn record_with_ts(&mut self, frame: &Frame, ts_ms: u64) {
        self.frames.push(SnapshotFrame {
            ts: ts_ms,
            frame: frame.clone(),
        });
    }

    pub fn save(&self, path: &str) -> Result<()> {
        // Compute duration from frame timestamps (first to last), falling back to elapsed
        let duration_ms = if self.frames.len() >= 2 {
            self.frames.last().unwrap().ts - self.frames.first().unwrap().ts
        } else {
            self.start_time.elapsed().as_millis() as u64
        };
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

/// Combined struct for single-pass deserialization (avoids cloning the entire JSON)
#[derive(Deserialize)]
struct SnapshotFile {
    #[serde(flatten)]
    header: SnapshotHeader,
    #[serde(default)]
    frames: Vec<SnapshotFrame>,
}

impl SnapshotPlayer {
    pub fn load(path: &str) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read snapshot file: {}", path))?;

        let file: SnapshotFile = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse snapshot file: {}", path))?;

        if file.header.version != 1 {
            anyhow::bail!(
                "Unsupported snapshot version {} in {}. This CLI supports version 1.",
                file.header.version,
                path
            );
        }

        if file.frames.is_empty() {
            eprintln!("Warning: snapshot file {} has no 'frames' key — replaying 0 frames.", path);
        }
        let frames = file.frames;

        eprintln!(
            "Loaded snapshot: {} frames, {:.1}s, view={}, captured={}",
            frames.len(),
            file.header.duration_ms as f64 / 1000.0,
            file.header.view,
            file.header.captured_at,
        );

        Ok(Self { header: file.header, frames })
    }
}
