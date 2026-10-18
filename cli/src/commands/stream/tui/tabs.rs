//! Tabs for a TUI streaming several views: one per view, plus an "All" tab
//! that merges them, its rows keyed `View · key`.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arete_sdk::Frame;

use super::app::App;

/// How recently a tab must have had a frame to show as live.
const LIVE_WINDOW: Duration = Duration::from_secs(2);

pub struct Tab {
    pub title: String,
    /// The subscription whose frames this tab shows; `None` for "All".
    subscription_id: Option<String>,
    /// What the All tab prefixes this view's keys with.
    view: String,
    pub app: App,
    last_frame: Option<Instant>,
}

pub struct Tabs {
    pub tabs: Vec<Tab>,
    pub active: usize,
}

/// One view the TUI shows.
pub struct TabFeed {
    pub subscription_id: String,
    pub view: String,
    /// `stack · View`.
    pub label: String,
}

impl Tabs {
    /// Tabs for `feeds`: a single view gets no tab bar; several get an
    /// "All" tab first.
    pub fn new(
        feeds: Vec<TabFeed>,
        url: &str,
        dropped_frames: Arc<AtomicU64>,
        snapshots: bool,
    ) -> Self {
        let mut tabs = Vec::new();
        if feeds.len() > 1 {
            let stack = feeds[0]
                .label
                .split_once(" \u{b7} ")
                .map(|(stack, _)| format!("{stack} \u{b7} "))
                .unwrap_or_default();
            tabs.push(Tab {
                title: "All".to_string(),
                subscription_id: None,
                view: String::new(),
                // Its rows come from every view's snapshots as upserts, so
                // after a reconnect it starts over and they fill it again.
                app: App::new(
                    format!("{stack}all views"),
                    url.to_string(),
                    Arc::clone(&dropped_frames),
                )
                .with_snapshots(false),
                last_frame: None,
            });
        }
        for feed in feeds {
            let view_id = feed.view.clone();
            tabs.push(Tab {
                title: feed.view.clone(),
                subscription_id: Some(feed.subscription_id),
                view: feed.view,
                app: App::new(feed.label, url.to_string(), Arc::clone(&dropped_frames))
                    .with_snapshots(snapshots)
                    .with_source_view(view_id),
                last_frame: None,
            });
        }
        Self { tabs, active: 0 }
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn current(&mut self) -> &mut App {
        &mut self.tabs[self.active].app
    }

    pub fn apps(&mut self) -> impl Iterator<Item = &mut App> {
        self.tabs.iter_mut().map(|tab| &mut tab.app)
    }

    pub fn next(&mut self) {
        self.active = (self.active + 1) % self.tabs.len();
    }

    pub fn prev(&mut self) {
        self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
    }

    /// Select tab `index` (from 0), if there is one.
    pub fn select(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    /// Whether tab `index` had a frame in the last couple of seconds.
    pub fn is_live(&self, index: usize, now: Instant) -> bool {
        self.tabs[index]
            .last_frame
            .is_some_and(|at| now.duration_since(at) < LIVE_WINDOW)
    }

    /// Hand a frame to its view's tab, and to the All tab.
    pub fn apply(&mut self, connection: u64, frame: Frame) {
        let now = Instant::now();
        if self.tabs.len() == 1 {
            self.tabs[0].last_frame = Some(now);
            self.tabs[0].app.apply_connection_frame(connection, frame);
            return;
        }
        let id = frame.subscription_id().to_string();
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.subscription_id.as_deref() == Some(id.as_str()))
        else {
            return;
        };
        let merged = for_all_tab(&self.tabs[index].view, &frame);
        let tab = &mut self.tabs[index];
        tab.last_frame = Some(now);
        tab.app.apply_connection_frame(connection, frame);
        if !merged.is_empty() {
            let all = &mut self.tabs[0];
            all.last_frame = Some(now);
            for frame in merged {
                all.app.apply_connection_frame(connection, frame);
            }
        }
    }
}

/// `frame` as the All tab takes it: keys prefixed with the view, and a
/// snapshot's rows as upserts, so one view's snapshot never replaces
/// another's rows.
pub fn for_all_tab(view: &str, frame: &Frame) -> Vec<Frame> {
    let relabel = |key: &str| format!("{view} \u{b7} {key}");
    match frame.clone() {
        Frame::Snapshot {
            protocol_version,
            subscription_id,
            mode,
            entity,
            data,
            ..
        } => data
            .into_iter()
            .map(|row| Frame::Upsert {
                protocol_version,
                subscription_id: subscription_id.clone(),
                mode,
                entity: entity.clone(),
                key: relabel(&row.key),
                data: row.data,
                append: Vec::new(),
                seq: None,
                offset: None,
            })
            .collect(),
        Frame::Subscribed { .. } | Frame::Unsubscribed { .. } => Vec::new(),
        mut frame => {
            if let Frame::Upsert { key, .. }
            | Frame::Patch { key, .. }
            | Frame::Remove { key, .. }
            | Frame::Delete { key, .. } = &mut frame
            {
                *key = relabel(key);
            }
            vec![frame]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arete_sdk::{Mode, SnapshotEntity};

    fn feeds() -> Vec<TabFeed> {
        ["A/list", "B/list"]
            .into_iter()
            .map(|view| TabFeed {
                subscription_id: format!("sub:{view}"),
                view: view.to_string(),
                label: format!("tip-jar \u{b7} {view}"),
            })
            .collect()
    }

    fn tabs() -> Tabs {
        Tabs::new(
            feeds(),
            "ws://localhost/",
            Arc::new(AtomicU64::new(0)),
            true,
        )
    }

    fn upsert(view: &str, key: &str) -> Frame {
        Frame::Upsert {
            protocol_version: 2,
            subscription_id: format!("sub:{view}"),
            mode: Mode::List,
            entity: view.to_string(),
            key: key.to_string(),
            data: serde_json::json!({ "id": key }),
            append: Vec::new(),
            seq: None,
            offset: None,
        }
    }

    fn snapshot(view: &str, keys: &[&str]) -> Frame {
        Frame::Snapshot {
            protocol_version: 2,
            subscription_id: format!("sub:{view}"),
            snapshot_id: "s".to_string(),
            authoritative: true,
            mode: Mode::List,
            entity: view.to_string(),
            key: None,
            data: keys
                .iter()
                .map(|key| SnapshotEntity {
                    key: key.to_string(),
                    data: serde_json::json!({ "id": key }),
                })
                .collect(),
            complete: true,
        }
    }

    #[test]
    fn several_views_get_an_all_tab_first() {
        let tabs = tabs();
        let titles: Vec<&str> = tabs.tabs.iter().map(|tab| tab.title.as_str()).collect();
        assert_eq!(titles, ["All", "A/list", "B/list"]);
        assert_eq!(tabs.tabs[0].app.view, "tip-jar \u{b7} all views");
        assert_eq!(tabs.tabs[1].app.view, "tip-jar \u{b7} A/list");

        let single = Tabs::new(
            feeds().into_iter().take(1).collect(),
            "ws://localhost/",
            Arc::new(AtomicU64::new(0)),
            true,
        );
        assert_eq!(single.len(), 1);
    }

    #[test]
    fn frames_reach_their_tab_and_the_all_tab() {
        let mut tabs = tabs();
        tabs.apply(0, snapshot("A/list", &["1"]));
        tabs.apply(0, upsert("B/list", "1"));
        // A frame for no subscription of ours is dropped.
        tabs.apply(0, upsert("C/list", "9"));

        assert_eq!(tabs.tabs[1].app.entity_keys, ["1"]);
        assert_eq!(tabs.tabs[2].app.entity_keys, ["1"]);
        assert_eq!(
            tabs.tabs[0].app.entity_keys,
            ["A/list \u{b7} 1", "B/list \u{b7} 1"]
        );
        // One view's authoritative snapshot leaves the other's rows alone.
        tabs.apply(0, snapshot("B/list", &["2"]));
        assert_eq!(tabs.tabs[2].app.entity_keys, ["2"]);
        assert!(tabs.tabs[0]
            .app
            .entity_keys
            .contains(&"A/list \u{b7} 1".to_string()));
    }

    #[test]
    fn tabs_cycle_and_select_by_number() {
        let mut tabs = tabs();
        tabs.next();
        assert_eq!(tabs.active, 1);
        tabs.prev();
        tabs.prev();
        assert_eq!(tabs.active, 2);
        tabs.select(0);
        assert_eq!(tabs.active, 0);
        tabs.select(7);
        assert_eq!(tabs.active, 0);
    }

    #[test]
    fn a_tab_is_live_while_frames_arrive() {
        let mut tabs = tabs();
        let now = Instant::now();
        assert!(!tabs.is_live(1, now));
        tabs.apply(0, upsert("A/list", "1"));
        let now = Instant::now();
        assert!(tabs.is_live(0, now) && tabs.is_live(1, now));
        assert!(!tabs.is_live(2, now));
        assert!(!tabs.is_live(1, now + LIVE_WINDOW));
    }
}
