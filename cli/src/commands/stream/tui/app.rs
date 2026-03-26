use hyperstack_sdk::{parse_snapshot_entities, Frame, Operation};
use ratatui::widgets::ListState;
use serde_json::Value;
use std::collections::{HashSet, VecDeque};
use std::fmt::Write as FmtWrite;

use crate::commands::stream::snapshot::SnapshotRecorder;
use crate::commands::stream::store::EntityStore;

const MAX_STATUS_AGE_MS: u128 = 3000;

/// Pretty-print JSON with compact inline arrays when they fit within max_width.
pub fn compact_pretty(value: &Value, max_width: usize) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0, max_width);
    out
}

fn write_value(out: &mut String, value: &Value, indent: usize, max_width: usize) {
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            let inner = indent + 2;
            for (i, (k, v)) in map.iter().enumerate() {
                write_indent(out, inner);
                let _ = write!(out, "\"{}\": ", k);
                write_value(out, v, inner, max_width);
                if i + 1 < map.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            write_indent(out, indent);
            out.push('}');
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                out.push_str("[]");
                return;
            }
            // Try compact form: [elem1, elem2, ...]
            let compact = serde_json::to_string(value).unwrap_or_default();
            if indent + compact.len() <= max_width {
                out.push_str(&compact);
                return;
            }
            // Fall back to expanded form
            out.push_str("[\n");
            let inner = indent + 2;
            for (i, v) in arr.iter().enumerate() {
                write_indent(out, inner);
                write_value(out, v, inner, max_width);
                if i + 1 < arr.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            write_indent(out, indent);
            out.push(']');
        }
        _ => {
            let s = serde_json::to_string(value).unwrap_or_default();
            out.push_str(&s);
        }
    }
}

fn write_indent(out: &mut String, n: usize) {
    for _ in 0..n {
        out.push(' ');
    }
}

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
    // Sorting
    CycleSortMode,
    ToggleSortDirection,
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

#[derive(Clone, PartialEq)]
pub enum SortMode {
    Insertion,
    Field(String),
}

#[derive(Clone, Copy, PartialEq)]
pub enum SortDirection {
    Ascending,
    Descending,
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
    /// Absolute VecDeque index when browsing history (position > 0).
    /// Stays stable as new frames arrive. None when viewing latest.
    history_anchor: Option<usize>,
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
    pub terminal_width: u16,
    pub sort_mode: SortMode,
    pub sort_direction: SortDirection,
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
            history_anchor: None,
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
            terminal_width: 120,
            sort_mode: SortMode::Insertion,
            sort_direction: SortDirection::Descending,
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

    /// Compensate history_anchor when the selected entity's history grows.
    /// If a pop_front happened (len didn't grow despite a push), decrement anchor.
    fn compensate_history_anchor(&mut self, updated_key: &str, len_before: usize) {
        if let Some(anchor) = self.history_anchor {
            if let Some(selected) = self.selected_key() {
                if selected == updated_key {
                    let len_after = self.store.history_len(updated_key);
                    // pop_front happened if length didn't increase
                    if len_after == len_before {
                        if anchor == 0 {
                            // The entry we were viewing was evicted
                            self.set_status("History entry evicted");
                            // Stay at oldest available
                        } else {
                            self.history_anchor = Some(anchor - 1);
                        }
                    }
                    // No pop: anchor stays valid (new entry appended to back)
                }
            }
        }
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
                let len_before = self.store.history_len(&key);
                self.store
                    .upsert(&key, frame.data, &frame.op, seq);
                self.compensate_history_anchor(&key, len_before);
                if self.entity_key_set.insert(key.clone()) {
                    self.entity_keys.push(key);
                }
                self.update_count += 1;
            }
            Operation::Patch => {
                let key = frame.key.clone();
                let seq = frame.seq.clone();
                let len_before = self.store.history_len(&key);
                self.store
                    .patch(&key, &frame.data, &frame.append, seq);
                self.compensate_history_anchor(&key, len_before);
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
                self.history_anchor = None;
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
                self.scroll_offset = self.scroll_offset.saturating_add(n as u16)
                    .min(self.max_scroll_offset());
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
                self.scroll_offset = self.max_scroll_offset();
            }
            TuiAction::ScrollDetailHalfDown => {
                let half = (self.visible_rows / 2).max(1);
                self.scroll_offset = self.scroll_offset.saturating_add(half as u16)
                    .min(self.max_scroll_offset());
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
                self.history_anchor = None;
                    self.scroll_offset = 0;
                }
            }
            TuiAction::PrevEntity => {
                let n = self.take_count();
                self.selected_index = self.selected_index.saturating_sub(n);
                self.history_position = 0;
                self.history_anchor = None;
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
                if let Some(key) = self.selected_key() {
                    let hist_len = self.store.history_len(&key);
                    if hist_len == 0 { /* no-op */ }
                    else if let Some(anchor) = self.history_anchor {
                        // Already browsing — move anchor backward (toward older)
                        if anchor > 0 {
                            self.history_anchor = Some(anchor - 1);
                            self.history_position += 1;
                        }
                    } else if hist_len >= 2 {
                        // Start browsing — anchor to second-to-last entry
                        self.history_anchor = Some(hist_len - 2);
                        self.history_position = 1;
                    }
                }
                self.scroll_offset = 0;
            }
            TuiAction::HistoryForward => {
                if let Some(key) = self.selected_key() {
                    let hist_len = self.store.history_len(&key);
                    if let Some(anchor) = self.history_anchor {
                        if anchor + 1 >= hist_len {
                            // Reached latest — clear anchor
                            self.history_anchor = None;
                            self.history_position = 0;
                self.history_anchor = None;
                        } else {
                            self.history_anchor = Some(anchor + 1);
                            self.history_position = self.history_position.saturating_sub(1);
                        }
                    }
                }
                self.scroll_offset = 0;
            }
            TuiAction::HistoryOldest => {
                if let Some(key) = self.selected_key() {
                    let hist_len = self.store.history_len(&key);
                    if hist_len > 0 {
                        self.history_anchor = Some(0);
                        self.history_position = hist_len.saturating_sub(1);
                    }
                }
                self.scroll_offset = 0;
            }
            TuiAction::HistoryNewest => {
                self.history_position = 0;
                self.history_anchor = None;
                self.history_anchor = None;
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
            TuiAction::CycleSortMode => {
                self.sort_mode = match &self.sort_mode {
                    SortMode::Insertion => SortMode::Field("_seq".to_string()),
                    SortMode::Field(_) => SortMode::Insertion,
                };
                self.invalidate_filter_cache();
                let label = match &self.sort_mode {
                    SortMode::Insertion => "Sort: insertion order".to_string(),
                    SortMode::Field(f) => format!("Sort: {} {}", f, match self.sort_direction {
                        SortDirection::Ascending => "asc",
                        SortDirection::Descending => "desc",
                    }),
                };
                self.set_status(&label);
            }
            TuiAction::ToggleSortDirection => {
                self.sort_direction = match self.sort_direction {
                    SortDirection::Ascending => SortDirection::Descending,
                    SortDirection::Descending => SortDirection::Ascending,
                };
                self.invalidate_filter_cache();
                let label = match &self.sort_mode {
                    SortMode::Insertion => "Sort direction toggled (no effect in insertion order)".to_string(),
                    SortMode::Field(f) => format!("Sort: {} {}", f, match self.sort_direction {
                        SortDirection::Ascending => "asc",
                        SortDirection::Descending => "desc",
                    }),
                };
                self.set_status(&label);
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
                self.history_anchor = None;
                self.scroll_offset = 0;
            }
            TuiAction::GotoBottom => {
                self.pending_count = None;
                let count = self.filtered_keys().len();
                if count > 0 {
                    self.selected_index = count - 1;
                }
                self.history_position = 0;
                self.history_anchor = None;
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
                self.history_anchor = None;
                self.scroll_offset = 0;
            }
            TuiAction::HalfPageUp => {
                let n = self.take_count();
                let half = self.visible_rows / 2;
                self.selected_index = self.selected_index.saturating_sub(half * n);
                self.history_position = 0;
                self.history_anchor = None;
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
                self.history_anchor = None;
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
                self.history_anchor = None;
        self.scroll_offset = 0;
        self.list_state.select(Some(self.selected_index));
    }

    /// Maximum scroll offset for the detail pane (total lines - visible height).
    fn max_scroll_offset(&self) -> u16 {
        let total_lines = self.selected_entity_data()
            .map(|s| s.lines().count())
            .unwrap_or(0);
        // visible_rows approximates the detail pane height (minus borders)
        let visible = self.visible_rows.saturating_sub(2);
        if total_lines > visible {
            (total_lines - visible) as u16
        } else {
            0
        }
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
            // Use anchor-based index if available for stable diff view
            if let Some(anchor) = self.history_anchor {
                let entry = self.store.at_absolute(&key, anchor)?;
                // Compute diff manually against previous entry
                if anchor > 0 {
                    if let Some(prev) = self.store.at_absolute(&key, anchor - 1) {
                        let diff = serde_json::json!({
                            "op": entry.op,
                            "state": entry.state,
                            "patch": entry.patch,
                            "previous_state": prev.state,
                        });
                        return Some(serde_json::to_string_pretty(&diff).unwrap_or_default());
                    }
                }
                return Some(serde_json::to_string_pretty(&serde_json::json!({
                    "op": entry.op,
                    "state": entry.state,
                    "patch": entry.patch,
                })).unwrap_or_default());
            }
            let diff = self.store.diff_at(&key, self.history_position)?;
            return Some(serde_json::to_string_pretty(&diff).unwrap_or_default());
        }

        let w = self.terminal_width as usize;

        // Use anchor for stable history browsing during streaming
        if let Some(anchor) = self.history_anchor {
            let entry = self.store.at_absolute(&key, anchor)?;
            return Some(compact_pretty(&entry.state, w));
        }

        if self.history_position > 0 {
            let entry = self.store.at(&key, self.history_position)?;
            return Some(compact_pretty(&entry.state, w));
        }

        let record = self.store.get(&key)?;
        Some(compact_pretty(&record.current, w))
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
        let mut result = if self.filter_text.is_empty() {
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
        // Apply sort if not insertion order
        if let SortMode::Field(ref path) = self.sort_mode {
            let path = path.clone();
            let dir = self.sort_direction;
            let store = &self.store;
            result.sort_by(|a, b| {
                let va = store.get(a).and_then(|r| resolve_dot_path(&r.current, &path));
                let vb = store.get(b).and_then(|r| resolve_dot_path(&r.current, &path));
                let cmp = compare_json_values(va, vb);
                match dir {
                    SortDirection::Ascending => cmp,
                    SortDirection::Descending => cmp.reverse(),
                }
            });
        }

        self.filtered_cache = Some(result);
    }
}

/// Resolve a dot-path like "_seq" or "info.name" into a JSON value.
fn resolve_dot_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    if current.is_null() { None } else { Some(current) }
}

/// Compare two optional JSON values. Numbers compare numerically, strings
/// lexicographically, null/missing sorts last.
fn compare_json_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater, // missing sorts last
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(va), Some(vb)) => {
            // Try numeric comparison first
            if let (Some(na), Some(nb)) = (as_f64(va), as_f64(vb)) {
                return na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal);
            }
            // Fall back to string comparison
            let sa = value_to_sort_string(va);
            let sb = value_to_sort_string(vb);
            sa.cmp(&sb)
        }
    }
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

fn value_to_sort_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        _ => serde_json::to_string(v).unwrap_or_default(),
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
