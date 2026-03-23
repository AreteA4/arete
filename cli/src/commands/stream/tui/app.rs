use hyperstack_sdk::{parse_snapshot_entities, Frame, Operation};
use serde_json::Value;

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
    pub selected_index: usize,
    pub history_position: usize,
    pub show_diff: bool,
    pub show_raw: bool,
    pub paused: bool,
    pub filter_input_active: bool,
    pub filter_text: String,
    pub status_message: String,
    pub status_time: std::time::Instant,
    pub update_count: u64,
    pub scroll_offset: u16,
    store: EntityStore,
    raw_frames: Vec<Frame>,
    recorder: Option<SnapshotRecorder>,
}

impl App {
    pub fn new(view: String, url: String) -> Self {
        Self {
            view: view.clone(),
            url: url.clone(),
            view_mode: ViewMode::List,
            entity_keys: Vec::new(),
            selected_index: 0,
            history_position: 0,
            show_diff: false,
            show_raw: false,
            paused: false,
            filter_input_active: false,
            filter_text: String::new(),
            status_message: "Connected".to_string(),
            status_time: std::time::Instant::now(),
            update_count: 0,
            scroll_offset: 0,
            store: EntityStore::new(),
            raw_frames: Vec::new(),
            recorder: Some(SnapshotRecorder::new(&view, &url)),
        }
    }

    pub fn apply_frame(&mut self, frame: Frame) {
        // Record for save
        if let Some(recorder) = &mut self.recorder {
            recorder.record(&frame);
        }

        let raw_frame = if self.show_raw { Some(frame.clone()) } else { None };
        let op = frame.operation();

        match op {
            Operation::Snapshot => {
                let entities = parse_snapshot_entities(&frame.data);
                for entity in entities {
                    self.store.upsert(&entity.key, entity.data, "snapshot", None);
                    if !self.entity_keys.contains(&entity.key) {
                        self.entity_keys.push(entity.key);
                    }
                }
                self.update_count += 1;
            }
            Operation::Upsert | Operation::Create => {
                let key = frame.key.clone();
                let seq = frame.seq.clone();
                self.store
                    .upsert(&key, frame.data, &frame.op, seq);
                if !self.entity_keys.contains(&key) {
                    self.entity_keys.push(key);
                }
                self.update_count += 1;
            }
            Operation::Patch => {
                let key = frame.key.clone();
                let seq = frame.seq.clone();
                self.store
                    .patch(&key, &frame.data, &frame.append, seq);
                if !self.entity_keys.contains(&key) {
                    self.entity_keys.push(key);
                }
                self.update_count += 1;
            }
            Operation::Delete => {
                self.store.delete(&frame.key);
                self.entity_keys.retain(|k| k != &frame.key);
                self.update_count += 1;
                if self.selected_index >= self.entity_keys.len() && self.selected_index > 0 {
                    self.selected_index -= 1;
                }
            }
            Operation::Subscribed => {
                self.set_status("Subscribed");
            }
        }

        if let Some(raw) = raw_frame {
            self.raw_frames.push(raw);
            if self.raw_frames.len() > 1000 {
                self.raw_frames.drain(0..500);
            }
        }
    }

    pub fn handle_action(&mut self, action: TuiAction) {
        match action {
            TuiAction::Quit => {}
            TuiAction::NextEntity => {
                let count = self.filtered_keys().len();
                if count > 0 {
                    self.selected_index = (self.selected_index + 1).min(count - 1);
                    self.history_position = 0;
                    self.scroll_offset = 0;
                }
            }
            TuiAction::PrevEntity => {
                self.selected_index = self.selected_index.saturating_sub(1);
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
                if let Some(recorder) = &self.recorder {
                    let filename = format!("hs-stream-{}.json", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
                    match recorder.save(&filename) {
                        Ok(_) => self.set_status(&format!("Saved to {}", filename)),
                        Err(e) => self.set_status(&format!("Save failed: {}", e)),
                    }
                }
            }
            TuiAction::FilterChar(c) => {
                self.filter_text.push(c);
                self.clamp_selection();
            }
            TuiAction::FilterBackspace => {
                self.filter_text.pop();
                self.clamp_selection();
            }
        }
    }

    fn clamp_selection(&mut self) {
        let count = self.filtered_keys().len();
        if count == 0 {
            self.selected_index = 0;
        } else if self.selected_index >= count {
            self.selected_index = count - 1;
        }
        self.history_position = 0;
        self.scroll_offset = 0;
    }

    pub fn selected_key(&self) -> Option<String> {
        let keys = self.filtered_keys();
        keys.get(self.selected_index).map(|s| s.to_string())
    }

    pub fn selected_entity_data(&self) -> Option<String> {
        let key = self.selected_key()?;

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

    pub fn filtered_keys(&self) -> Vec<&str> {
        if self.filter_text.is_empty() {
            self.entity_keys.iter().map(|s| s.as_str()).collect()
        } else {
            let lower = self.filter_text.to_lowercase();
            self.entity_keys
                .iter()
                .filter(|k| {
                    // Match on key itself
                    if k.to_lowercase().contains(&lower) {
                        return true;
                    }
                    // Match on any value inside the entity data
                    if let Some(record) = self.store.get(k) {
                        return value_contains_str(&record.current, &lower);
                    }
                    false
                })
                .map(|s| s.as_str())
                .collect()
        }
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
            map.values().any(|v| value_contains_str(v, needle))
        }
        Value::Array(arr) => {
            arr.iter().any(|v| value_contains_str(v, needle))
        }
        Value::Null => false,
    }
}
