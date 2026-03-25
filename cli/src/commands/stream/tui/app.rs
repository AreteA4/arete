use hyperstack_sdk::{parse_snapshot_entities, Frame, Operation};
use ratatui::widgets::ListState;
use serde_json::Value;
use std::collections::{HashSet, VecDeque};

use crate::commands::stream::snapshot::SnapshotRecorder;
use crate::commands::stream::store::EntityStore;

const MAX_STATUS_AGE_MS: u128 = 3000;

pub enum TuiAction {
    Quit,
    NextEntity,
    PrevEntity,
    FocusDetail,
    BackToList,
    HistoryForward,
    HistoryBack,
    HistoryOldest,
    HistoryNewest,
    ToggleDiff,
    ToggleRaw,
    TogglePause,
    StartFilter,
    SaveSnapshot,
    FilterChar(char),
    FilterBackspace,
    FilterClear,
    FilterDeleteWord,
    // Detail pane scroll
    ScrollDetailDown,
    ScrollDetailUp,
    ScrollDetailTop,
    ScrollDetailBottom,
    ScrollDetailHalfDown,
    ScrollDetailHalfUp,
    // Vim motions
    GotoTop,
    GotoBottom,
    HalfPageDown,
    HalfPageUp,
    NextMatch,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ViewMode {
    List,
    Detail,
}

#[allow(dead_code)]
pub struct App {
    pub view: String,
    pub url: String,
    pub view_mode: ViewMode,
    pub entity_keys: Vec<String>,
    entity_key_set: HashSet<String>,
    pub selected_index: usize,
    pub history_position: usize,
    pub show_diff: bool,
    pub show_raw: bool,
    pub paused: bool,
    pub disconnected: bool,
    pub filter_input_active: bool,
    pub filter_text: String,
    pub status_message: String,
    pub status_time: std::time::Instant,
    pub update_count: u64,
    pub scroll_offset: u16,
    pub visible_rows: usize,
    pub pending_count: Option<usize>,
    pub pending_g: bool,
    pub list_state: ListState,
    store: EntityStore,
    raw_frames: VecDeque<(std::time::Instant, Frame)>,
    stream_start: std::time::Instant,
    pub dropped_frames: std::sync::Arc<std::sync::atomic::AtomicU64>,
    filtered_cache: Option<Vec<String>>,
}

impl App {
    pub fn new(view: String, url: String, dropped_frames: std::sync::Arc<std::sync::atomic::AtomicU64>) -> Self {
        Self {
            view: view.clone(),
            url: url.clone(),
            view_mode: ViewMode::List,
            entity_keys: Vec::new(),
            entity_key_set: HashSet::new(),
            selected_index: 0,
            history_position: 0,
            show_diff: false,
            show_raw: false,
            paused: false,
            disconnected: false,
            filter_input_active: false,
            filter_text: String::new(),
            status_message: "Connected".to_string(),
            status_time: std::time::Instant::now(),
            update_count: 0,
            scroll_offset: 0,
            visible_rows: 30,
            pending_count: None,
            pending_g: false,
            list_state: ListState::default().with_selected(Some(0)),
            store: EntityStore::new(),
            raw_frames: VecDeque::new(),
            stream_start: std::time::Instant::now(),
            dropped_frames,
            filtered_cache: None,
        }
    }

    fn invalidate_filter_cache(&mut self) {
        self.filtered_cache = None;
    }

    pub fn apply_frame(&mut self, frame: Frame) {
        // Invalidation is cheap (sets to None). The cache is only rebuilt once per
        // render tick in ensure_filtered_cache(), not per-frame, since we drain all
        // frames before drawing.
        self.invalidate_filter_cache();

        // Always collect raw frames so toggling on shows recent data
        let raw_frame = frame.clone();
        let op = frame.operation();

        match op {
            Operation::Snapshot => {
                let entities = parse_snapshot_entities(&frame.data);
                let count = entities.len() as u64;
                for entity in entities {
                    self.store.upsert(&entity.key, entity.data, "snapshot", None);
                    if self.entity_key_set.insert(entity.key.clone()) {
                        self.entity_keys.push(entity.key);
                    }
                }
                self.update_count += count;
            }
            Operation::Upsert | Operation::Create => {
                let key = frame.key.clone();
                let seq = frame.seq.clone();
                self.store
                    .upsert(&key, frame.data, &frame.op, seq);
                if self.entity_key_set.insert(key.clone()) {
                    self.entity_keys.push(key);
                }
                self.update_count += 1;
            }
            Operation::Patch => {
                let key = frame.key.clone();
                let seq = frame.seq.clone();
                self.store
                    .patch(&key, &frame.data, &frame.append, seq);
                if self.entity_key_set.insert(key.clone()) {
                    self.entity_keys.push(key);
                }
                self.update_count += 1;
            }
            Operation::Delete => {
                let deleted_pos = self.entity_keys.iter().position(|k| k == &frame.key);
                self.store.delete(&frame.key);
                self.entity_key_set.remove(&frame.key);
                self.entity_keys.retain(|k| k != &frame.key);
                self.update_count += 1;
                // If deleted entity was before cursor, shift cursor back to preserve selection
                if let Some(pos) = deleted_pos {
                    if pos < self.selected_index && self.selected_index > 0 {
                        self.selected_index -= 1;
                    }
                }
                if self.selected_index >= self.entity_keys.len() && !self.entity_keys.is_empty() {
                    self.selected_index = self.entity_keys.len() - 1;
                }
                self.history_position = 0;
                self.scroll_offset = 0;
            }
            Operation::Subscribed => {
                self.set_status("Subscribed");
            }
        }

        self.raw_frames.push_back((std::time::Instant::now(), raw_frame));
        while self.raw_frames.len() > 1000 {
            self.raw_frames.pop_front();
        }
    }

    /// Take and reset the pending count prefix (e.g. "10j" → 10). Returns 1 if no count.
    fn take_count(&mut self) -> usize {
        let n = self.pending_count.unwrap_or(1);
        self.pending_count = None;
        self.pending_g = false;
        n
    }

    pub fn handle_action(&mut self, action: TuiAction) {
        self.ensure_filtered_cache();
        // Reset pending_g after every action (including GotoTop)
        self.pending_g = false;

        match action {
            TuiAction::Quit => {}
            TuiAction::ScrollDetailDown => {
                let n = self.take_count();
                self.scroll_offset = self.scroll_offset.saturating_add(n as u16);
            }
            TuiAction::ScrollDetailUp => {
                let n = self.take_count();
                self.scroll_offset = self.scroll_offset.saturating_sub(n as u16);
            }
            TuiAction::ScrollDetailTop => {
                self.pending_count = None;
                self.scroll_offset = 0;
            }
            TuiAction::ScrollDetailBottom => {
                self.pending_count = None;
                // Set to a large value; rendering will clamp naturally
                self.scroll_offset = u16::MAX;
            }
            TuiAction::ScrollDetailHalfDown => {
                let half = (self.visible_rows / 2).max(1);
                self.scroll_offset = self.scroll_offset.saturating_add(half as u16);
            }
            TuiAction::ScrollDetailHalfUp => {
                let half = (self.visible_rows / 2).max(1);
                self.scroll_offset = self.scroll_offset.saturating_sub(half as u16);
            }
            TuiAction::NextEntity => {
                let n = self.take_count();
                let count = self.filtered_keys().len();
                if count > 0 {
                    self.selected_index = (self.selected_index + n).min(count - 1);
                    self.history_position = 0;
                    self.scroll_offset = 0;
                }
            }
            TuiAction::PrevEntity => {
                let n = self.take_count();
                self.selected_index = self.selected_index.saturating_sub(n);
                self.history_position = 0;
                self.scroll_offset = 0;
            }
            TuiAction::FocusDetail => {
                self.view_mode = ViewMode::Detail;
                self.scroll_offset = 0;
            }
            TuiAction::BackToList => {
                if self.filter_input_active {
                    self.filter_input_active = false;
                } else {
                    self.view_mode = ViewMode::List;
                    self.scroll_offset = 0;
                }
            }
            TuiAction::HistoryBack => {
                self.history_position += 1;
                self.scroll_offset = 0;
                // Clamp to max history for selected entity
                if let Some(key) = self.selected_key() {
                    if let Some(record) = self.store.get(&key) {
                        if self.history_position >= record.history.len() {
                            self.history_position = record.history.len().saturating_sub(1);
                        }
                    }
                }
            }
            TuiAction::HistoryForward => {
                self.history_position = self.history_position.saturating_sub(1);
                self.scroll_offset = 0;
            }
            TuiAction::HistoryOldest => {
                if let Some(key) = self.selected_key() {
                    if let Some(record) = self.store.get(&key) {
                        self.history_position = record.history.len().saturating_sub(1);
                    }
                }
                self.scroll_offset = 0;
            }
            TuiAction::HistoryNewest => {
                self.history_position = 0;
                self.scroll_offset = 0;
            }
            TuiAction::ToggleDiff => {
                self.show_diff = !self.show_diff;
                self.set_status(if self.show_diff { "Diff view ON" } else { "Diff view OFF" });
            }
            TuiAction::ToggleRaw => {
                self.show_raw = !self.show_raw;
                self.set_status(if self.show_raw { "Raw frames ON" } else { "Raw frames OFF" });
            }
            TuiAction::TogglePause => {
                self.paused = !self.paused;
                self.set_status(if self.paused { "PAUSED" } else { "Resumed" });
            }
            TuiAction::StartFilter => {
                self.filter_input_active = true;
                self.filter_text.clear();
            }
            TuiAction::SaveSnapshot => {
                // Note: this does synchronous file I/O on the runtime thread. Acceptable
                // because raw_frames is capped at 1000 entries. For larger caps, consider
                // spawning onto a blocking thread.
                let mut recorder = SnapshotRecorder::new(&self.view, &self.url);
                for (arrival_time, frame) in &self.raw_frames {
                    let ts_ms = arrival_time.duration_since(self.stream_start).as_millis() as u64;
                    recorder.record_with_ts(frame, ts_ms);
                }
                let filename = format!("hs-stream-{}.json", chrono::Utc::now().format("%Y%m%d-%H%M%S%.3f"));
                match recorder.save(&filename) {
                    Ok(_) => self.set_status(&format!("Saved to {}", filename)),
                    Err(e) => self.set_status(&format!("Save failed: {}", e)),
                }
            }
            TuiAction::FilterChar(c) => {
                self.filter_text.push(c);
                self.invalidate_filter_cache();
                self.clamp_selection();
            }
            TuiAction::FilterBackspace => {
                self.filter_text.pop();
                self.invalidate_filter_cache();
                self.clamp_selection();
            }
            TuiAction::FilterClear => {
                self.filter_text.clear();
                self.invalidate_filter_cache();
                self.clamp_selection();
            }
            TuiAction::FilterDeleteWord => {
                // Delete back to previous word boundary (or start)
                let trimmed = self.filter_text.trim_end();
                if let Some(pos) = trimmed.rfind(|c: char| c.is_whitespace()) {
                    self.filter_text.truncate(pos + 1);
                } else {
                    self.filter_text.clear();
                }
                self.invalidate_filter_cache();
                self.clamp_selection();
            }
            TuiAction::GotoTop => {
                self.pending_count = None;
                self.selected_index = 0;
                self.history_position = 0;
                self.scroll_offset = 0;
            }
            TuiAction::GotoBottom => {
                self.pending_count = None;
                let count = self.filtered_keys().len();
                if count > 0 {
                    self.selected_index = count - 1;
                }
                self.history_position = 0;
                self.scroll_offset = 0;
            }
            TuiAction::HalfPageDown => {
                let n = self.take_count();
                let half = self.visible_rows / 2;
                let count = self.filtered_keys().len();
                if count > 0 {
                    self.selected_index = (self.selected_index + half * n).min(count - 1);
                }
                self.history_position = 0;
                self.scroll_offset = 0;
            }
            TuiAction::HalfPageUp => {
                let n = self.take_count();
                let half = self.visible_rows / 2;
                self.selected_index = self.selected_index.saturating_sub(half * n);
                self.history_position = 0;
                self.scroll_offset = 0;
            }
            TuiAction::NextMatch => {
                if self.filter_text.is_empty() {
                    return;
                }
                let n = self.take_count();
                let keys = self.filtered_keys();
                let count = keys.len();
                if count > 0 {
                    self.selected_index = (self.selected_index + n) % count;
                }
                self.history_position = 0;
                self.scroll_offset = 0;
            }
        }
        self.list_state.select(Some(self.selected_index));
    }

    fn clamp_selection(&mut self) {
        self.ensure_filtered_cache();
        let count = self.filtered_keys().len();
        if count == 0 {
            self.selected_index = 0;
        } else if self.selected_index >= count {
            self.selected_index = count - 1;
        }
        self.history_position = 0;
        self.scroll_offset = 0;
        self.list_state.select(Some(self.selected_index));
    }

    pub fn selected_key(&self) -> Option<String> {
        let keys = self.filtered_keys();
        keys.get(self.selected_index).map(|s| s.to_string())
    }

    pub fn selected_entity_data(&self) -> Option<String> {
        let key = self.selected_key()?;

        // Raw mode: show the most recent raw frame for this entity key.
        // Snapshot frames have key="" (entities are in data array), so fall back
        // to showing the merged state with a note for snapshot-only entities.
        if self.show_raw {
            if let Some((_, raw)) = self.raw_frames.iter().rev().find(|(_, f)| f.key == key) {
                return Some(serde_json::to_string_pretty(raw).unwrap_or_default());
            }
            // Entity was ingested via snapshot batch — no individual raw frame exists
            let record = self.store.get(&key)?;
            let fallback = serde_json::json!({
                "_note": "Received via snapshot batch (no individual raw frame)",
                "key": key,
                "data": record.current,
            });
            return Some(serde_json::to_string_pretty(&fallback).unwrap_or_default());
        }

        if self.show_diff {
            let diff = self.store.diff_at(&key, self.history_position)?;
            return Some(serde_json::to_string_pretty(&diff).unwrap_or_default());
        }

        if self.history_position > 0 {
            let entry = self.store.at(&key, self.history_position)?;
            return Some(serde_json::to_string_pretty(&entry.state).unwrap_or_default());
        }

        let record = self.store.get(&key)?;
        Some(serde_json::to_string_pretty(&record.current).unwrap_or_default())
    }

    pub fn selected_history_len(&self) -> usize {
        self.selected_key()
            .and_then(|k| self.store.get(&k))
            .map(|r| r.history.len())
            .unwrap_or(0)
    }

    pub fn status(&self) -> &str {
        if self.status_time.elapsed().as_millis() < MAX_STATUS_AGE_MS {
            &self.status_message
        } else if self.paused {
            "PAUSED"
        } else {
            "Streaming"
        }
    }

    fn set_status(&mut self, msg: &str) {
        self.status_message = msg.to_string();
        self.status_time = std::time::Instant::now();
    }

    pub fn set_disconnected(&mut self) {
        self.disconnected = true;
        self.set_status("Disconnected");
    }

    /// Returns cached filtered keys.
    /// Panics in debug builds if `ensure_filtered_cache()` was not called first.
    pub fn filtered_keys(&self) -> &[String] {
        debug_assert!(self.filtered_cache.is_some(), "filtered_keys() called without ensure_filtered_cache()");
        self.filtered_cache.as_deref().unwrap_or(&[])
    }

    /// Rebuild the filter cache if invalidated.
    pub fn ensure_filtered_cache(&mut self) {
        if self.filtered_cache.is_some() {
            return;
        }
        let result = if self.filter_text.is_empty() {
            self.entity_keys.clone()
        } else {
            let lower = self.filter_text.to_lowercase();
            self.entity_keys
                .iter()
                .filter(|k| {
                    if k.to_lowercase().contains(&lower) {
                        return true;
                    }
                    if let Some(record) = self.store.get(k) {
                        return value_contains_str(&record.current, &lower);
                    }
                    false
                })
                .cloned()
                .collect()
        };
        self.filtered_cache = Some(result);
    }
}

/// Recursively search all values in a JSON tree for a substring match.
fn value_contains_str(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(s) => s.to_lowercase().contains(needle),
        Value::Number(n) => n.to_string().contains(needle),
        Value::Bool(b) => {
            let s = if *b { "true" } else { "false" };
            s.contains(needle)
        }
        Value::Object(map) => {
            map.iter().any(|(k, v)| {
                k.to_lowercase().contains(needle) || value_contains_str(v, needle)
            })
        }
        Value::Array(arr) => {
            arr.iter().any(|v| value_contains_str(v, needle))
        }
        Value::Null => false,
    }
}
