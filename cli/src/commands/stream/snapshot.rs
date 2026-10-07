use anyhow::{Context, Result};
use arete_sdk::Frame;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};

use super::token;

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

/// Where the recorded stream reconnected.
///
/// A reconnect drops the stream's entity state, so replay has to drop it at
/// the same point to end up where the live stream did. Recordings list
/// these in an optional top-level `reconnects` array, separate from
/// `frames`: recordings without one replay as before, and a CLI that does
/// not know the field still reads the frames.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotReconnect {
    /// How many frames were recorded before the new connection; replay
    /// starts over before the frame at this index.
    pub at_frame: u64,
    pub ts: u64,
}

pub struct SnapshotRecorder {
    frames: Vec<SnapshotFrame>,
    reconnects: Vec<SnapshotReconnect>,
    view: String,
    url: String,
    start_time: std::time::Instant,
    start_timestamp: chrono::DateTime<chrono::Utc>,
    limit_warned: bool,
}

impl SnapshotRecorder {
    pub fn new(view: &str, url: &str) -> Self {
        Self {
            frames: Vec::new(),
            reconnects: Vec::new(),
            view: view.to_string(),
            url: token::redact_hs_token_for_display(url),
            start_time: std::time::Instant::now(),
            start_timestamp: chrono::Utc::now(),
            limit_warned: false,
        }
    }

    const MAX_FRAMES: usize = 100_000;

    pub fn record(&mut self, frame: &Frame) {
        if self.frames.len() >= Self::MAX_FRAMES {
            if !self.limit_warned {
                eprintln!(
                    "Warning: snapshot recorder reached {} frames limit. Further frames will be dropped. \
                     Use --duration to limit recording time.",
                    Self::MAX_FRAMES
                );
                self.limit_warned = true;
            }
            return;
        }
        let ts = self.start_time.elapsed().as_millis() as u64;
        self.frames.push(SnapshotFrame {
            ts,
            frame: frame.clone(),
        });
    }

    /// Note that the stream reconnected after the frames recorded so far.
    pub fn record_reconnect(&mut self) {
        let ts = self.start_time.elapsed().as_millis() as u64;
        self.record_reconnect_with_ts(ts);
    }

    pub fn record_reconnect_with_ts(&mut self, ts_ms: u64) {
        self.reconnects.push(SnapshotReconnect {
            at_frame: self.frames.len() as u64,
            ts: ts_ms,
        });
    }

    #[cfg(feature = "tui")]
    pub fn record_with_ts(&mut self, frame: &Frame, ts_ms: u64) {
        if self.frames.len() >= Self::MAX_FRAMES {
            return;
        }
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
            version: 2,
            view: self.view.clone(),
            url: self.url.clone(),
            captured_at: self.start_timestamp.to_rfc3339(),
            duration_ms,
            frame_count: self.frames.len() as u64,
        };

        // Stream-serialize to tmp file to avoid holding the entire JSON in memory.
        let dest = std::path::Path::new(path);
        let parent = dest.parent().unwrap_or_else(|| std::path::Path::new("."));
        let file_name = dest.file_name().unwrap_or_default();
        let tmp_path = parent
            .join(format!("{}.tmp", file_name.to_string_lossy()))
            .to_string_lossy()
            .into_owned();
        {
            let file = fs::File::create(&tmp_path)
                .with_context(|| format!("Failed to create snapshot file: {}", tmp_path))?;
            let mut writer = io::BufWriter::new(file);

            // Write header fields
            writeln!(writer, "{{")?;
            writeln!(writer, "  \"version\": {},", header.version)?;
            writeln!(
                writer,
                "  \"view\": {},",
                serde_json::to_string(&header.view)?
            )?;
            writeln!(
                writer,
                "  \"url\": {},",
                serde_json::to_string(&header.url)?
            )?;
            writeln!(
                writer,
                "  \"captured_at\": {},",
                serde_json::to_string(&header.captured_at)?
            )?;
            writeln!(writer, "  \"duration_ms\": {},", header.duration_ms)?;
            writeln!(writer, "  \"frame_count\": {},", header.frame_count)?;
            // Only recordings that span a reconnect carry the field.
            if !self.reconnects.is_empty() {
                writeln!(
                    writer,
                    "  \"reconnects\": {},",
                    serde_json::to_string(&self.reconnects)?
                )?;
            }

            // Stream frames array one entry at a time
            writeln!(writer, "  \"frames\": [")?;
            for (i, frame) in self.frames.iter().enumerate() {
                let frame_json = serde_json::to_string(frame)?;
                if i > 0 {
                    writeln!(writer, ",")?;
                }
                write!(writer, "    {}", frame_json)?;
            }
            writeln!(writer, "\n  ]")?;
            writeln!(writer, "}}")?;
            writer.flush()?;
        }
        // Attempt remove; if it fails, let rename itself fail with a clear error
        // (don't silently swallow remove errors that may mask the true state).
        #[cfg(windows)]
        if dest.exists() {
            fs::remove_file(path)
                .with_context(|| format!("Failed to remove existing snapshot at {}", path))?;
        }
        fs::rename(&tmp_path, path).map_err(|e| {
            // Best-effort cleanup of the tmp file before propagating
            let _ = fs::remove_file(&tmp_path);
            anyhow::anyhow!("Failed to rename snapshot to {}: {}", path, e)
        })?;

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
    /// In frame order. Empty for a recording that never reconnected.
    pub reconnects: Vec<SnapshotReconnect>,
}

/// Combined struct for single-pass deserialization (avoids cloning the entire JSON)
#[derive(Deserialize)]
struct SnapshotFile {
    #[serde(flatten)]
    header: SnapshotHeader,
    #[serde(default)]
    frames: Vec<SnapshotFrame>,
    #[serde(default)]
    reconnects: Vec<SnapshotReconnect>,
}

impl SnapshotPlayer {
    pub fn load(path: &str) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read snapshot file: {}", path))?;

        let file: SnapshotFile = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse snapshot file: {}", path))?;

        if file.header.version != 2 {
            anyhow::bail!(
                "Unsupported snapshot version {} in {}. This CLI supports protocol v2 snapshots (version 2).",
                file.header.version,
                path
            );
        }

        if file.frames.is_empty() {
            eprintln!(
                "Warning: snapshot file {} has no 'frames' key — replaying 0 frames.",
                path
            );
        }
        let frames = file.frames;
        let mut reconnects = file.reconnects;
        reconnects.sort_by_key(|reconnect| reconnect.at_frame);

        eprintln!(
            "Loaded snapshot: {} frames{}, {:.1}s, view={}, captured={}",
            frames.len(),
            match reconnects.len() {
                0 => String::new(),
                1 => ", 1 reconnect".to_string(),
                n => format!(", {n} reconnects"),
            },
            file.header.duration_ms as f64 / 1000.0,
            file.header.view,
            file.header.captured_at,
        );

        Ok(Self {
            header: file.header,
            frames,
            reconnects,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arete_sdk::Mode;

    fn upsert(key: &str) -> Frame {
        Frame::Upsert {
            protocol_version: 2,
            subscription_id: "cli:test".to_string(),
            mode: Mode::List,
            entity: "Ore/list".to_string(),
            key: key.to_string(),
            data: serde_json::json!({ "id": key }),
            append: Vec::new(),
            seq: None,
            offset: None,
        }
    }

    #[test]
    fn reconnects_are_saved_beside_the_frames_and_loaded_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recording.json");
        let path = path.to_str().unwrap();
        let mut recorder = SnapshotRecorder::new("Ore/list", "ws://localhost/");
        recorder.record(&upsert("1"));
        recorder.record_reconnect();
        recorder.record(&upsert("2"));
        recorder.record_reconnect();
        recorder.save(path).unwrap();

        let player = SnapshotPlayer::load(path).unwrap();
        assert_eq!(player.frames.len(), 2);
        assert_eq!(
            player
                .reconnects
                .iter()
                .map(|reconnect| reconnect.at_frame)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );

        // The frames array keeps its shape, so a reader that predates the
        // field still loads them.
        let file: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(file["version"], 2);
        for frame in file["frames"].as_array().unwrap() {
            let mut fields: Vec<_> = frame.as_object().unwrap().keys().cloned().collect();
            fields.sort();
            assert_eq!(fields, vec!["frame", "ts"]);
        }
    }

    #[test]
    fn a_recording_without_reconnects_is_written_and_read_as_before() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recording.json");
        let path = path.to_str().unwrap();
        let mut recorder = SnapshotRecorder::new("Ore/list", "ws://localhost/");
        recorder.record(&upsert("1"));
        recorder.save(path).unwrap();

        let file: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert!(file.get("reconnects").is_none(), "{file}");
        assert!(SnapshotPlayer::load(path).unwrap().reconnects.is_empty());
    }
}
