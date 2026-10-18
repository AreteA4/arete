//! `a4 stream --dev --tui`: browse the `a4 dev` session protocol first.
//!
//! 1. **Protocols** (left pane): each protocol stack with its spec's name
//!    and version, whether it is live, and its instance count. With one
//!    protocol the focus starts on its views.
//! 2. **One protocol** (right pane): its deployment, counts, a restart
//!    notice when instances predate the fork, and its views (Instances,
//!    Activity, Participants, ...) with row counts and last-update ages, or
//!    why they are empty.
//! 3. **One view**: its rows and the selected row (the stream TUI).
//! 4. **One instance**: Enter on an instance opens its `ProtocolInstance/state`
//!    row by key, with the instance's machine state (`open`, `closed`, ...)
//!    from the local API in the header, and its rows in each per-instance
//!    view (`ProtocolActivity/byInstance`, `ProtocolParticipant/byInstance`
//!    when the spec has participants, `ProtocolInstance/byId`); Tab switches
//!    between them.
//!
//! Enter opens, Esc goes back, Tab switches pane, / filters the pane.
//! Each protocol's stack streams over its own connection (every view but
//! the keyed ones), which follows `a4 dev` redeploys; an open instance gets
//! one more, closed when it is left.

use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use arete_sdk::{Frame, ServerMessage, Subscription};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use tokio::sync::{mpsc, oneshot};

use super::app::{App, TuiAction, ViewMode};
use super::app_key_action;
use crate::commands::stream::client::Feed;
use crate::commands::stream::dev::{self, DevListing, DevRuntime, DevStack, ForkMarker};
use crate::commands::stream::protocols::{
    self, DevTarget, InstanceKey, ViewEntry, ViewRole, ACTIVITY_ENTITY, INSTANCE_ENTITY,
    PARTICIPANT_ENTITY,
};
use crate::commands::stream::session::{
    redeploy_marker, Notice, Retarget, SessionHandler, StreamEnd, StreamSession,
};
use crate::commands::stream::{build_feed_subscription, reconnect_policy, StreamArgs};

/// How often the listing (counts, readiness, the fork) is read again.
const LISTING_PERIOD: Duration = Duration::from_secs(2);
/// How recently a stack must have sent a frame to show as busy.
const BUSY_WINDOW: Duration = Duration::from_secs(2);

/// Which connection a frame or notice came from.
type Tag = u64;

/// What the connections tell the browser.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)] // Frames are the common case; boxing each costs more.
enum NavEvent {
    Frame(Tag, u64, Frame),
    Connected(Tag),
    Reconnecting(Tag, String),
    Reconnected(Tag),
    Status(Tag, String),
    Ended(Tag, Option<String>),
    /// The open instance's machine state, read from the local API.
    Machine(Tag, MachineState),
}

/// An instance's machine state as the local API reports it
/// (`GET /api/protocol-instances/:id`); the stack rows don't carry it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MachineState {
    Known { state: String, terminal: bool },
    Unavailable(String),
}

impl MachineState {
    /// From the instance detail the local API returns.
    pub(super) fn from_detail(detail: &serde_json::Value) -> Self {
        match detail.get("state").and_then(serde_json::Value::as_str) {
            Some(state) => Self::Known {
                state: state.to_string(),
                terminal: detail
                    .get("terminal")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            },
            None => Self::Unavailable("the API returned no state".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Conn {
    Connecting,
    Live,
    Reconnecting,
    Down,
}

/// One view a protocol's connection streams.
pub(super) struct ViewFeed {
    entry: ViewEntry,
    subscription_id: String,
    pub(super) app: App,
    /// Its subscription answered.
    loaded: bool,
    /// When rows last changed.
    last_update: Option<Instant>,
}

/// One protocol stack and its connection.
pub(super) struct ProtocolPane {
    pub(super) stack: DevStack,
    tag: Tag,
    pub(super) views: Vec<ViewFeed>,
    conn: Conn,
    last_frame: Option<Instant>,
}

impl ProtocolPane {
    fn new(stack: DevStack, tag: Tag, dropped: &Arc<AtomicU64>, snapshots: bool) -> Self {
        let name = stack.protocol_name().to_string();
        let views = stack
            .catalog()
            .into_iter()
            .filter(ViewEntry::streamable)
            .map(|entry| ViewFeed {
                subscription_id: format!("a4-cli:nav:{tag}:{}", entry.raw),
                app: App::new(
                    format!("{name} \u{b7} {}", entry.friendly),
                    stack.ws_url.clone(),
                    Arc::clone(dropped),
                )
                .with_snapshots(snapshots)
                .with_source_view(entry.raw.clone()),
                entry,
                loaded: false,
                last_update: None,
            })
            .collect();
        Self {
            stack,
            tag,
            views,
            conn: Conn::Connecting,
            last_frame: None,
        }
    }

    /// The index of the primary view of `entity`.
    fn primary(&self, entity: &str) -> Option<usize> {
        self.views
            .iter()
            .position(|view| view.entry.entity == entity && view.entry.role == ViewRole::Primary)
    }

    fn rows(&self, entity: &str) -> Option<usize> {
        let view = &self.views[self.primary(entity)?];
        view.loaded.then_some(view.app.entity_keys.len())
    }

    /// The level-2 rows: each primary view, then the instance drill-in.
    fn home_rows(&self, filter: &str) -> Vec<HomeRow> {
        let filter = filter.to_lowercase();
        let mut rows: Vec<HomeRow> = self
            .views
            .iter()
            .enumerate()
            .filter(|(_, view)| view.entry.role == ViewRole::Primary)
            .filter(|(_, view)| view.entry.friendly.to_lowercase().contains(&filter))
            .map(|(index, _)| HomeRow::View(index))
            .collect();
        if self.primary(INSTANCE_ENTITY).is_some() && "instance".contains(&filter) {
            rows.push(HomeRow::Instance);
        }
        rows
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HomeRow {
    View(usize),
    /// `Instance …`: pick one in Instances.
    Instance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pane {
    Protocols,
    Views,
}

/// One per-instance view of an open instance: the view's rows of that
/// instance only.
pub(super) struct InstanceFeed {
    pub(super) entry: ViewEntry,
    pub(super) subscription_id: String,
    pub(super) app: App,
    admitted: HashSet<String>,
}

/// An open instance: its state row, its machine state and its rows in each
/// per-instance view.
pub(super) struct InstanceScreen {
    protocol: usize,
    pub(super) key: InstanceKey,
    tag: Tag,
    state_sub: String,
    pub(super) state: App,
    state_loaded: bool,
    pub(super) feeds: Vec<InstanceFeed>,
    /// The feed shown.
    pub(super) active: usize,
    /// From the local API; `None` until the first read.
    pub(super) machine: Option<MachineState>,
    /// The view it was opened from, for Esc.
    from: Option<usize>,
    /// Some instances were created before this fork: this may be one.
    fork_restarted: bool,
}

impl InstanceScreen {
    pub(super) fn active_app(&mut self) -> Option<&mut App> {
        self.feeds.get_mut(self.active).map(|feed| &mut feed.app)
    }
}

pub(super) enum Screen {
    Home,
    View { protocol: usize, view: usize },
    Instance(Box<InstanceScreen>),
}

/// What the browser asks the loop to do.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Command {
    Quit,
    /// Open a connection for the instance screen just shown.
    OpenInstance,
    /// Close the instance connection `tag`.
    CloseInstance(Tag),
}

/// The browser's state: everything but the connections.
pub(super) struct Browser {
    pub(super) protocols: Vec<ProtocolPane>,
    fork: Option<ForkMarker>,
    pub(super) selected: usize,
    pub(super) view_row: usize,
    pub(super) focus: Pane,
    pub(super) screen: Screen,
    protocol_filter: String,
    view_filter: String,
    /// The pane `/` is typing into.
    filtering: Option<Pane>,
    status: Option<(String, Instant)>,
    next_tag: Tag,
    dropped: Arc<AtomicU64>,
    snapshots: bool,
}

impl Browser {
    pub(super) fn new(
        listing: DevListing,
        default: Option<&str>,
        dropped: Arc<AtomicU64>,
        snapshots: bool,
    ) -> Self {
        let mut browser = Self {
            protocols: Vec::new(),
            fork: listing.fork,
            selected: 0,
            view_row: 0,
            focus: Pane::Protocols,
            screen: Screen::Home,
            protocol_filter: String::new(),
            view_filter: String::new(),
            filtering: None,
            status: None,
            next_tag: 0,
            dropped,
            snapshots,
        };
        for stack in listing.stacks {
            browser.add(stack);
        }
        if let Some(index) = default.and_then(|name| browser.index_of(name)) {
            browser.selected = index;
        }
        if browser.protocols.len() == 1 {
            browser.focus = Pane::Views;
        }
        browser
    }

    fn add(&mut self, stack: DevStack) -> usize {
        let tag = self.tag();
        self.protocols
            .push(ProtocolPane::new(stack, tag, &self.dropped, self.snapshots));
        self.protocols.len() - 1
    }

    fn tag(&mut self) -> Tag {
        self.next_tag += 1;
        self.next_tag
    }

    fn index_of(&self, stack: &str) -> Option<usize> {
        self.protocols
            .iter()
            .position(|pane| pane.stack.name == stack)
    }

    /// Start where `target` points.
    pub(super) fn start_at(&mut self, target: &DevTarget) -> Vec<Command> {
        let Some(index) = target.stack().and_then(|name| self.index_of(name)) else {
            return Vec::new();
        };
        self.selected = index;
        self.focus = Pane::Views;
        match target {
            DevTarget::View { view, .. } => {
                let pane = &self.protocols[index];
                if let Some(position) = pane.views.iter().position(|feed| &feed.entry.raw == view) {
                    self.screen = Screen::View {
                        protocol: index,
                        view: position,
                    };
                } else if let Some(instances) = pane.primary(INSTANCE_ENTITY) {
                    // A keyed view: pick the row in Instances.
                    self.screen = Screen::View {
                        protocol: index,
                        view: instances,
                    };
                    self.say("Pick an instance and press Enter to open its state");
                } else {
                    self.say(&format!("{view} needs a key; use --key without --tui"));
                }
                Vec::new()
            }
            DevTarget::Instance { key, .. } => self.open_instance(index, key.clone(), None),
            _ => Vec::new(),
        }
    }

    fn say(&mut self, message: &str) {
        self.status = Some((message.to_string(), Instant::now()));
    }

    /// The protocols the left pane shows (filtered).
    pub(super) fn visible_protocols(&self) -> Vec<usize> {
        let filter = self.protocol_filter.to_lowercase();
        (0..self.protocols.len())
            .filter(|&index| {
                filter.is_empty()
                    || self.protocols[index]
                        .stack
                        .protocol_name()
                        .to_lowercase()
                        .contains(&filter)
            })
            .collect()
    }

    pub(super) fn home_rows(&self) -> Vec<HomeRow> {
        self.protocols
            .get(self.selected)
            .map(|pane| pane.home_rows(&self.view_filter))
            .unwrap_or_default()
    }

    fn open_instance(
        &mut self,
        protocol: usize,
        key: InstanceKey,
        from: Option<usize>,
    ) -> Vec<Command> {
        let tag = self.tag();
        let pane = &self.protocols[protocol];
        let name = pane.stack.protocol_name().to_string();
        let (state_view, _) = protocols::instance_views(&pane.stack);
        let url = pane.stack.ws_url.clone();
        let short = key.key.chars().take(12).collect::<String>();
        let feeds = protocols::instance_collections(&pane.stack)
            .into_iter()
            .map(|entry| InstanceFeed {
                subscription_id: format!("a4-cli:nav:{tag}:{}", entry.raw),
                app: App::new(
                    format!("{name} \u{b7} Instance {short} \u{b7} {}", entry.friendly),
                    url.clone(),
                    Arc::clone(&self.dropped),
                )
                .with_snapshots(self.snapshots)
                .with_source_view(entry.raw.clone()),
                entry,
                admitted: HashSet::new(),
            })
            .collect();
        self.screen = Screen::Instance(Box::new(InstanceScreen {
            protocol,
            tag,
            state_sub: format!("a4-cli:nav:{tag}:{state_view}"),
            state: App::new(
                format!("{name} \u{b7} Instance {short}"),
                url,
                Arc::clone(&self.dropped),
            )
            .with_snapshots(self.snapshots)
            .with_source_view(state_view.clone()),
            state_loaded: false,
            feeds,
            active: 0,
            machine: None,
            fork_restarted: pane
                .stack
                .instance_counts()
                .is_some_and(|counts| counts.before_fork > 0),
            key,
            from,
        }));
        vec![Command::OpenInstance]
    }

    /// Handle a key; the loop carries out what it returns.
    pub(super) fn on_key(&mut self, key: KeyEvent) -> Vec<Command> {
        let ctrl_c =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl_c {
            return vec![Command::Quit];
        }
        match &mut self.screen {
            Screen::Home => self.home_key(key),
            Screen::View { protocol, view } => {
                let (protocol, view) = (*protocol, *view);
                let feed = &mut self.protocols[protocol].views[view];
                let app = &mut feed.app;
                let at_list = !app.filter_input_active && app.view_mode == ViewMode::List;
                if at_list
                    && key.code == KeyCode::Esc
                    && app.pending_count.is_none()
                    && !app.pending_g
                {
                    self.screen = Screen::Home;
                    self.focus = Pane::Views;
                    return Vec::new();
                }
                if at_list && key.code == KeyCode::Enter && feed.entry.entity == INSTANCE_ENTITY {
                    app.ensure_filtered_cache();
                    let Some(selected) = app.selected_key() else {
                        self.say("No instance yet: run a create step");
                        return Vec::new();
                    };
                    let key = match InstanceKey::parse(&selected) {
                        Ok(key) => key,
                        Err(error) => {
                            self.say(&error.to_string());
                            return Vec::new();
                        }
                    };
                    return self.open_instance(protocol, key, Some(view));
                }
                app_command(app, key)
            }
            Screen::Instance(instance) => {
                let feeds = instance.feeds.len();
                let (at_list, idle) = match instance.active_app() {
                    Some(app) => (
                        !app.filter_input_active && app.view_mode == ViewMode::List,
                        app.pending_count.is_none() && !app.pending_g,
                    ),
                    None => (true, true),
                };
                if at_list && feeds > 1 && matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                    instance.active = if key.code == KeyCode::Tab {
                        (instance.active + 1) % feeds
                    } else {
                        (instance.active + feeds - 1) % feeds
                    };
                    return Vec::new();
                }
                if at_list && key.code == KeyCode::Esc && idle {
                    let (tag, protocol, from) = (instance.tag, instance.protocol, instance.from);
                    self.screen = match from {
                        Some(view) => Screen::View { protocol, view },
                        None => Screen::Home,
                    };
                    self.focus = Pane::Views;
                    return vec![Command::CloseInstance(tag)];
                }
                match instance.active_app() {
                    Some(app) => app_command(app, key),
                    None if key.code == KeyCode::Char('q') => vec![Command::Quit],
                    None => Vec::new(),
                }
            }
        }
    }

    fn home_key(&mut self, key: KeyEvent) -> Vec<Command> {
        if let Some(pane) = self.filtering {
            let filter = match pane {
                Pane::Protocols => &mut self.protocol_filter,
                Pane::Views => &mut self.view_filter,
            };
            match key.code {
                KeyCode::Esc => {
                    filter.clear();
                    self.filtering = None;
                }
                KeyCode::Enter => self.filtering = None,
                KeyCode::Backspace => {
                    filter.pop();
                }
                KeyCode::Char(c) => filter.push(c),
                _ => {}
            }
            self.clamp();
            return Vec::new();
        }
        let several = self.protocols.len() > 1;
        match key.code {
            KeyCode::Char('q') => return vec![Command::Quit],
            KeyCode::Tab | KeyCode::BackTab if several => {
                self.focus = match self.focus {
                    Pane::Protocols => Pane::Views,
                    Pane::Views => Pane::Protocols,
                };
            }
            KeyCode::Char('/') => self.filtering = Some(self.focus),
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::Esc => match self.focus {
                Pane::Views if !self.view_filter.is_empty() => self.view_filter.clear(),
                Pane::Views if several => self.focus = Pane::Protocols,
                Pane::Protocols => self.protocol_filter.clear(),
                Pane::Views => {}
            },
            KeyCode::Enter => match self.focus {
                Pane::Protocols => {
                    if !self.protocols.is_empty() {
                        self.focus = Pane::Views;
                        self.view_row = 0;
                    }
                }
                Pane::Views => {
                    let rows = self.home_rows();
                    match rows.get(self.view_row) {
                        Some(HomeRow::View(view)) => {
                            self.screen = Screen::View {
                                protocol: self.selected,
                                view: *view,
                            };
                        }
                        Some(HomeRow::Instance) => {
                            if let Some(view) =
                                self.protocols[self.selected].primary(INSTANCE_ENTITY)
                            {
                                self.screen = Screen::View {
                                    protocol: self.selected,
                                    view,
                                };
                                self.say("Pick an instance and press Enter to open it");
                            }
                        }
                        None => {}
                    }
                }
            },
            _ => {}
        }
        self.clamp();
        Vec::new()
    }

    fn step(&mut self, delta: isize) {
        match self.focus {
            Pane::Protocols => {
                let visible = self.visible_protocols();
                if visible.is_empty() {
                    return;
                }
                let at = visible
                    .iter()
                    .position(|&index| index == self.selected)
                    .unwrap_or(0);
                let next = (at as isize + delta).clamp(0, visible.len() as isize - 1) as usize;
                if visible[next] != self.selected {
                    self.selected = visible[next];
                    self.view_row = 0;
                }
            }
            Pane::Views => {
                let rows = self.home_rows().len();
                if rows > 0 {
                    self.view_row =
                        (self.view_row as isize + delta).clamp(0, rows as isize - 1) as usize;
                }
            }
        }
    }

    /// Keep the selections on rows that are shown.
    fn clamp(&mut self) {
        let visible = self.visible_protocols();
        if !visible.is_empty() && !visible.contains(&self.selected) {
            self.selected = visible[0];
            self.view_row = 0;
        }
        let rows = self.home_rows().len();
        if self.view_row >= rows {
            self.view_row = rows.saturating_sub(1);
        }
    }

    /// Take a newer listing: counts, readiness, the fork. Returns the panes
    /// of stacks it lists for the first time.
    fn refresh(&mut self, listing: DevListing) -> Vec<usize> {
        self.fork = listing.fork;
        let mut added = Vec::new();
        for stack in listing.stacks {
            match self.index_of(&stack.name) {
                // The pane keeps the views it subscribed to; its connection
                // follows reloads.
                Some(index) => self.protocols[index].stack = stack,
                None => added.push(self.add(stack)),
            }
        }
        added
    }

    fn apply(&mut self, event: NavEvent) {
        let tag = match &event {
            NavEvent::Frame(tag, ..)
            | NavEvent::Connected(tag)
            | NavEvent::Reconnecting(tag, _)
            | NavEvent::Reconnected(tag)
            | NavEvent::Status(tag, _)
            | NavEvent::Ended(tag, _)
            | NavEvent::Machine(tag, _) => *tag,
        };
        if let Screen::Instance(instance) = &mut self.screen {
            if instance.tag == tag {
                apply_instance(instance, event);
                return;
            }
        }
        let Some(pane) = self.protocols.iter_mut().find(|pane| pane.tag == tag) else {
            return;
        };
        let name = pane.stack.protocol_name().to_string();
        let mut say = None;
        match event {
            NavEvent::Frame(_, connection, frame) => {
                let now = Instant::now();
                pane.last_frame = Some(now);
                pane.conn = Conn::Live;
                let id = frame.subscription_id().to_string();
                if let Some(feed) = pane
                    .views
                    .iter_mut()
                    .find(|feed| feed.subscription_id == id)
                {
                    feed.loaded = true;
                    if carries_rows(&frame) {
                        feed.last_update = Some(now);
                    }
                    feed.app.apply_connection_frame(connection, frame);
                }
            }
            NavEvent::Connected(_) | NavEvent::Reconnected(_) => {
                pane.conn = Conn::Live;
                for feed in &mut pane.views {
                    feed.app.set_reconnected();
                }
            }
            NavEvent::Reconnecting(_, message) => {
                pane.conn = Conn::Reconnecting;
                for feed in &mut pane.views {
                    feed.app.set_reconnecting(&message);
                }
            }
            NavEvent::Status(_, message) => {
                for feed in &mut pane.views {
                    feed.app.set_status(&message);
                }
                say = Some(format!("{name}: {message}"));
            }
            NavEvent::Ended(_, failure) => {
                pane.conn = Conn::Down;
                for feed in &mut pane.views {
                    feed.app.set_disconnected(failure.as_deref());
                }
                say = failure.map(|failure| format!("{name}: {failure}"));
            }
            // A machine state for an instance screen that has closed.
            NavEvent::Machine(..) => {}
        }
        if let Some(message) = say {
            self.say(&message);
        }
    }

    /// The app the screen shows, if it is a stream view.
    fn shown_apps(&mut self) -> Vec<&mut App> {
        match &mut self.screen {
            Screen::Home => Vec::new(),
            Screen::View { protocol, view } => {
                vec![&mut self.protocols[*protocol].views[*view].app]
            }
            Screen::Instance(instance) => instance.active_app().into_iter().collect(),
        }
    }
}

/// A key for a stream view, as the stream TUI handles it.
fn app_command(app: &mut App, key: KeyEvent) -> Vec<Command> {
    match app_key_action(app, key) {
        Some(TuiAction::Quit) => vec![Command::Quit],
        Some(action) => {
            app.handle_action(action);
            Vec::new()
        }
        None => Vec::new(),
    }
}

fn carries_rows(frame: &Frame) -> bool {
    match frame {
        Frame::Snapshot { data, .. } => !data.is_empty(),
        Frame::Subscribed { .. } | Frame::Unsubscribed { .. } => false,
        _ => true,
    }
}

fn instance_apps(instance: &mut InstanceScreen) -> impl Iterator<Item = &mut App> {
    instance.feeds.iter_mut().map(|feed| &mut feed.app)
}

fn apply_instance(instance: &mut InstanceScreen, event: NavEvent) {
    match event {
        NavEvent::Frame(_, connection, frame) => {
            let id = frame.subscription_id().to_string();
            if id == instance.state_sub {
                instance.state_loaded |=
                    matches!(frame, Frame::Snapshot { complete: true, .. }) || carries_rows(&frame);
                instance.state.apply_connection_frame(connection, frame);
            } else if let Some(feed) = instance
                .feeds
                .iter_mut()
                .find(|feed| feed.subscription_id == id)
            {
                let entity = feed.entry.entity.clone();
                if let Some(frame) = instance_frame(
                    frame,
                    &instance.key.key,
                    &mut feed.admitted,
                    |row, data, key| protocols::row_of_instance(&entity, row, data, key),
                ) {
                    feed.app.apply_connection_frame(connection, frame);
                }
            }
        }
        NavEvent::Reconnecting(_, message) => {
            for app in instance_apps(instance) {
                app.set_reconnecting(&message);
            }
        }
        NavEvent::Connected(_) | NavEvent::Reconnected(_) => {
            for app in instance_apps(instance) {
                app.set_reconnected();
            }
        }
        NavEvent::Status(_, message) => {
            for app in instance_apps(instance) {
                app.set_status(&message);
            }
        }
        NavEvent::Ended(_, failure) => {
            for app in instance_apps(instance) {
                app.set_disconnected(failure.as_deref());
            }
        }
        NavEvent::Machine(_, machine) => instance.machine = Some(machine),
    }
}

/// `frame` with only the rows of the instance keyed `key`, as `belongs`
/// (row key, row data, instance key) decides. A row is admitted by fields
/// written once (its `instance`, its key); later patches and removals
/// follow the admitted keys.
pub(super) fn instance_frame(
    frame: Frame,
    key: &str,
    admitted: &mut HashSet<String>,
    belongs: impl Fn(&str, &serde_json::Value, &str) -> bool,
) -> Option<Frame> {
    match frame {
        Frame::Snapshot {
            protocol_version,
            subscription_id,
            snapshot_id,
            authoritative,
            mode,
            entity,
            key: snapshot_key,
            data,
            complete,
        } => {
            let data: Vec<_> = data
                .into_iter()
                .filter(|row| belongs(&row.key, &row.data, key) || admitted.contains(&row.key))
                .collect();
            admitted.extend(data.iter().map(|row| row.key.clone()));
            Some(Frame::Snapshot {
                protocol_version,
                subscription_id,
                snapshot_id,
                authoritative,
                mode,
                entity,
                key: snapshot_key,
                data,
                complete,
            })
        }
        Frame::Upsert {
            key: ref row,
            ref data,
            ..
        }
        | Frame::Patch {
            key: ref row,
            ref data,
            ..
        } => {
            if belongs(row, data, key) {
                admitted.insert(row.clone());
            }
            admitted.contains(row).then_some(frame)
        }
        Frame::Remove { key: ref row, .. } | Frame::Delete { key: ref row, .. } => {
            admitted.remove(row).then_some(frame)
        }
        frame => Some(frame),
    }
}

/// The socket side of one connection.
struct TaggedOutputs {
    tag: Tag,
    events: mpsc::Sender<NavEvent>,
    dropped: Arc<AtomicU64>,
    connection: u64,
}

impl TaggedOutputs {
    fn send(&self, event: NavEvent) {
        if self.events.try_send(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl SessionHandler for TaggedOutputs {
    fn on_message(&mut self, message: ServerMessage) -> Result<bool> {
        match message {
            ServerMessage::Frame(frame) => {
                self.send(NavEvent::Frame(self.tag, self.connection, frame))
            }
            ServerMessage::Error(error) => self.send(NavEvent::Status(
                self.tag,
                format!("Server error [{}]: {}", error.code, error.message),
            )),
        }
        Ok(false)
    }

    fn on_notice(&mut self, notice: Notice<'_>) {
        let event = match notice {
            Notice::Unparsed { .. } => return,
            Notice::RefreshRefused(reason) => NavEvent::Status(
                self.tag,
                format!(
                    "Session refresh refused ({}); the stream ends when the current token expires",
                    reason.unwrap_or("no reason given")
                ),
            ),
            Notice::RefreshFailed(error) => NavEvent::Status(
                self.tag,
                format!("Could not refresh the session token, retrying: {error:#}"),
            ),
            Notice::Reconnecting(reconnecting) => {
                NavEvent::Reconnecting(self.tag, reconnecting.to_string())
            }
            Notice::Redeployed { from, to } => {
                NavEvent::Status(self.tag, redeploy_marker(from, to))
            }
        };
        self.send(event);
    }

    fn on_reconnected(&mut self, _url: &str) -> Result<()> {
        self.connection += 1;
        self.send(NavEvent::Reconnected(self.tag));
        Ok(())
    }
}

/// A running connection; dropping it stops it.
struct Connection {
    stop: Option<oneshot::Sender<()>>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

struct Spawner {
    runtime: DevRuntime,
    events: mpsc::Sender<NavEvent>,
    dropped: Arc<AtomicU64>,
    policy: crate::commands::stream::session::ReconnectPolicy,
}

impl Spawner {
    /// Connect to `stack` with `subscriptions`, following redeploys of it
    /// when `follow`.
    fn spawn(
        &self,
        tag: Tag,
        stack: &DevStack,
        subscriptions: Vec<Subscription>,
        follow: bool,
    ) -> Connection {
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        let mut refresh = self.runtime.refresh_for(stack);
        let follow: Option<mpsc::Receiver<Retarget>> = follow.then(|| {
            dev::watch(
                self.runtime.clone(),
                Some(stack.name.clone()),
                stack.identity(),
            )
        });
        let mut outputs = TaggedOutputs {
            tag,
            events: self.events.clone(),
            dropped: Arc::clone(&self.dropped),
            connection: 0,
        };
        let policy = self.policy.clone();
        tokio::spawn(async move {
            let url = match refresh.renew().await {
                Ok(url) => url,
                Err(error) => {
                    outputs.send(NavEvent::Ended(
                        tag,
                        Some(format!("could not mint a session: {error:#}")),
                    ));
                    return;
                }
            };
            let session =
                StreamSession::with_subscriptions(url, Some(refresh), subscriptions, policy);
            let mut session = match session {
                Ok(session) => session.follow(follow),
                Err(error) => {
                    outputs.send(NavEvent::Ended(tag, Some(format!("{error:#}"))));
                    return;
                }
            };
            let socket = session.connect().await;
            // Left before it connected.
            if !matches!(stop_rx.try_recv(), Err(oneshot::error::TryRecvError::Empty)) {
                return;
            }
            let socket = match socket {
                Ok(socket) => socket,
                Err(error) => {
                    outputs.send(NavEvent::Ended(tag, Some(format!("{error:#}"))));
                    return;
                }
            };
            outputs.send(NavEvent::Connected(tag));
            let stop = async move {
                let _ = stop_rx.await;
            };
            tokio::pin!(stop);
            let end = session.run(socket, &mut outputs, stop).await;
            let failure = match end {
                Ok(StreamEnd::Failed(failure)) => Some(failure.to_string()),
                Ok(StreamEnd::Lost(loss)) => Some(loss.to_string()),
                Ok(_) => None,
                Err(error) => Some(format!("{error:#}")),
            };
            outputs.send(NavEvent::Ended(tag, failure));
        });
        Connection {
            stop: Some(stop_tx),
        }
    }
}

fn protocol_subscriptions(pane: &ProtocolPane, args: &StreamArgs) -> Vec<Subscription> {
    pane.views
        .iter()
        .map(|feed| {
            let mut subscription =
                build_feed_subscription(&Feed::new(feed.entry.raw.clone(), None), args);
            subscription.subscription_id = feed.subscription_id.clone();
            subscription
        })
        .collect()
}

fn instance_subscriptions(
    instance: &InstanceScreen,
    stack: &DevStack,
    args: &StreamArgs,
) -> Vec<Subscription> {
    let (state, _) = protocols::instance_views(stack);
    let mut state = build_feed_subscription(
        &Feed {
            key: Some(instance.key.key.clone()),
            ..Feed::new(state, None)
        },
        args,
    );
    state.subscription_id = instance.state_sub.clone();
    let mut subscriptions = vec![state];
    for feed in &instance.feeds {
        let mut subscription =
            build_feed_subscription(&Feed::new(feed.entry.raw.clone(), None), args);
        subscription.subscription_id = feed.subscription_id.clone();
        subscriptions.push(subscription);
    }
    subscriptions
}

/// How often an open instance's machine state is read again.
const MACHINE_PERIOD: Duration = Duration::from_secs(2);

/// Read the open instance's machine state from the local API until the
/// task is aborted (the instance screen closed) or the browser is gone.
fn poll_machine(spawner: &Spawner, tag: Tag, key: &InstanceKey) -> tokio::task::JoinHandle<()> {
    let events = spawner.events.clone();
    let uuid = key.uuid.clone();
    let runtime = spawner.runtime.clone();
    tokio::spawn(async move {
        let Some(uuid) = uuid else {
            let _ = events
                .send(NavEvent::Machine(
                    tag,
                    MachineState::Unavailable("not an instance id".to_string()),
                ))
                .await;
            return;
        };
        let client = match arete_mcp::dev::DevClient::open(runtime.project.clone()) {
            Ok(client) => client,
            Err(error) => {
                let _ = events
                    .send(NavEvent::Machine(
                        tag,
                        MachineState::Unavailable(format!("{error:#}")),
                    ))
                    .await;
                return;
            }
        };
        loop {
            let machine = match client.instance(&uuid, None).await {
                Ok(detail) => MachineState::from_detail(&detail),
                Err(error) => MachineState::Unavailable(format!("{error:#}")),
            };
            if events.send(NavEvent::Machine(tag, machine)).await.is_err() {
                return;
            }
            tokio::time::sleep(MACHINE_PERIOD).await;
        }
    })
}

/// The open instance's connection and machine-state reader; dropping it
/// closes both.
struct OpenInstance {
    tag: Tag,
    _connection: Connection,
    machine: tokio::task::JoinHandle<()>,
}

impl Drop for OpenInstance {
    fn drop(&mut self) {
        self.machine.abort();
    }
}

/// `a4 stream --dev --tui`.
pub async fn browse(
    runtime: DevRuntime,
    listing: DevListing,
    target: DevTarget,
    args: &StreamArgs,
) -> Result<()> {
    let dropped = Arc::new(AtomicU64::new(0));
    let default = dev::select_stack(&listing.stacks, None, &runtime.session)
        .ok()
        .map(|stack| stack.name.clone());
    let mut browser = Browser::new(
        listing,
        default.as_deref(),
        Arc::clone(&dropped),
        !args.no_snapshot,
    );
    let (events_tx, mut events) = mpsc::channel::<NavEvent>(10_000);
    let spawner = Spawner {
        runtime: runtime.clone(),
        events: events_tx,
        dropped: Arc::clone(&dropped),
        policy: reconnect_policy(args),
    };
    let mut connections: Vec<(Tag, Connection)> = browser
        .protocols
        .iter()
        .map(|pane| {
            (
                pane.tag,
                spawner.spawn(
                    pane.tag,
                    &pane.stack,
                    protocol_subscriptions(pane, args),
                    true,
                ),
            )
        })
        .collect();

    // The listing, read again for counts and new stacks.
    let (listings_tx, mut listings) = mpsc::channel::<DevListing>(2);
    let poller = runtime.clone();
    let poll = tokio::spawn(async move {
        let mut runtime = poller;
        loop {
            tokio::time::sleep(LISTING_PERIOD).await;
            runtime.reload();
            if let Ok(listing) = runtime.listing().await {
                if listings_tx.send(listing).await.is_err() {
                    return;
                }
            }
        }
    });

    let commands = browser.start_at(&target);
    let mut instance: Option<OpenInstance> = None;
    run_commands(&mut browser, &spawner, &mut instance, commands, args);

    let original_hook = Arc::new(std::sync::Mutex::new(Some(std::panic::take_hook())));
    let hook = Arc::clone(&original_hook);
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        if let Ok(guard) = hook.lock() {
            if let Some(original) = &*guard {
                original(info);
            }
        }
    }));
    enable_raw_mode()?;
    let setup = || -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        Ok(Terminal::new(CrosstermBackend::new(stdout))?)
    };
    let mut terminal = match setup() {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            return Err(error);
        }
    };

    let result = async {
        loop {
            let size = terminal.size()?;
            for app in browser.shown_apps() {
                app.terminal_width = size.width;
            }
            terminal.draw(|f| draw(f, &mut browser))?;
            while let Ok(event) = events.try_recv() {
                browser.apply(event);
            }
            while let Ok(listing) = listings.try_recv() {
                for index in browser.refresh(listing) {
                    let pane = &browser.protocols[index];
                    connections.push((
                        pane.tag,
                        spawner.spawn(
                            pane.tag,
                            &pane.stack,
                            protocol_subscriptions(pane, args),
                            true,
                        ),
                    ));
                }
            }
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    let commands = browser.on_key(key);
                    if commands.contains(&Command::Quit) {
                        return Ok::<(), anyhow::Error>(());
                    }
                    run_commands(&mut browser, &spawner, &mut instance, commands, args);
                }
            }
        }
    }
    .await;

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    let _ = std::panic::take_hook();
    if let Ok(mut guard) = original_hook.lock() {
        if let Some(original) = guard.take() {
            std::panic::set_hook(original);
        }
    }
    poll.abort();
    drop(instance);
    drop(connections);
    // Give the connections a moment to close their sockets.
    tokio::time::sleep(Duration::from_millis(100)).await;
    result
}

fn run_commands(
    browser: &mut Browser,
    spawner: &Spawner,
    instance: &mut Option<OpenInstance>,
    commands: Vec<Command>,
    args: &StreamArgs,
) {
    for command in commands {
        match command {
            Command::Quit => {}
            Command::CloseInstance(tag) => {
                if instance.as_ref().is_some_and(|open| open.tag == tag) {
                    *instance = None;
                }
            }
            Command::OpenInstance => {
                if let Screen::Instance(screen) = &browser.screen {
                    let stack = &browser.protocols[screen.protocol].stack;
                    let subscriptions = instance_subscriptions(screen, stack, args);
                    *instance = Some(OpenInstance {
                        tag: screen.tag,
                        _connection: spawner.spawn(screen.tag, stack, subscriptions, false),
                        machine: poll_machine(spawner, screen.tag, &screen.key),
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn draw(f: &mut ratatui::Frame, browser: &mut Browser) {
    let area = f.area();
    if matches!(browser.screen, Screen::Home) {
        return draw_home(f, browser, area);
    }
    match &mut browser.screen {
        Screen::Home => {}
        Screen::View { protocol, view } => {
            let app = &mut browser.protocols[*protocol].views[*view].app;
            // The stream view's own rows, less the help line.
            app.visible_rows = area.height.saturating_sub(6) as usize;
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(area);
            super::ui::draw(f, app, chunks[0]);
            let hint = if browser.protocols[*protocol].views[*view].entry.entity == INSTANCE_ENTITY
            {
                " Enter open instance \u{b7} Esc back \u{b7} / filter \u{b7} q quit"
            } else {
                " Esc back \u{b7} Enter detail \u{b7} / filter \u{b7} q quit"
            };
            let status = browser.status.as_ref().map(|(text, _)| text.as_str());
            draw_help(f, chunks[1], hint, status);
        }
        Screen::Instance(instance) => {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(40),
                    Constraint::Length(1),
                    Constraint::Min(0),
                    Constraint::Length(1),
                ])
                .split(area);
            draw_instance_state(f, instance, chunks[0]);
            f.render_widget(Paragraph::new(instance_tabs(instance)), chunks[1]);
            let rows = chunks[2].height.saturating_sub(5) as usize;
            match instance.active_app() {
                Some(app) => {
                    app.visible_rows = rows;
                    super::ui::draw(f, app, chunks[2]);
                }
                None => f.render_widget(
                    Paragraph::new("This stack has no per-instance views.")
                        .block(Block::default().borders(Borders::ALL)),
                    chunks[2],
                ),
            }
            let keys = if instance.feeds.len() > 1 {
                " Tab next view \u{b7} Esc back \u{b7} Enter detail \u{b7} / filter \u{b7} q quit"
            } else {
                " Esc back \u{b7} Enter detail \u{b7} / filter \u{b7} q quit"
            };
            draw_help(f, chunks[3], keys, None);
        }
    }
}

fn draw_help(f: &mut ratatui::Frame, area: Rect, keys: &str, status: Option<&str>) {
    let mut spans = vec![Span::styled(
        keys.to_string(),
        Style::default().fg(Color::DarkGray),
    )];
    if let Some(status) = status {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            status.to_string(),
            Style::default().fg(Color::Yellow),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The per-instance views, the shown one highlighted, with row counts.
fn instance_tabs(instance: &mut InstanceScreen) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    let active = instance.active;
    for (index, feed) in instance.feeds.iter_mut().enumerate() {
        feed.app.ensure_filtered_cache();
        let label = format!(" {} ({}) ", feed.entry.friendly, feed.app.entity_keys.len());
        let style = if index == active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

/// The header's machine state: `open`, `closed (terminal)`, or why it is
/// unknown.
pub(super) fn machine_label(machine: Option<&MachineState>) -> (String, Color) {
    match machine {
        None => ("state ...".to_string(), Color::DarkGray),
        Some(MachineState::Known {
            state,
            terminal: true,
        }) => (format!("{state} (terminal)"), Color::Yellow),
        Some(MachineState::Known { state, .. }) => (state.clone(), Color::Green),
        Some(MachineState::Unavailable(_)) => ("state unknown".to_string(), Color::DarkGray),
    }
}

fn draw_instance_state(f: &mut ratatui::Frame, instance: &mut InstanceScreen, area: Rect) {
    instance.state.ensure_filtered_cache();
    let (machine, color) = machine_label(instance.machine.as_ref());
    let mut title = vec![Span::raw(format!(" Instance {} ", instance.key.key))];
    if let Some(uuid) = &instance.key.uuid {
        title.push(Span::raw(format!("\u{b7} {uuid} ")));
    }
    title.push(Span::raw("\u{b7} "));
    title.push(Span::styled(
        machine,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ));
    title.push(Span::raw(" "));
    let title = Line::from(title);
    let body = match instance.state.selected_entity_data() {
        Some(data) => data,
        None => {
            let mut body = if instance.state_loaded {
                "No ProtocolInstance row for this key yet. Its row appears once the stack \
                 sees the instance's attested activity on this fork."
                    .to_string()
            } else {
                "Waiting for the instance's row...".to_string()
            };
            if instance.fork_restarted {
                body.push_str(
                    "\n\nSome instances were created before this fork began (a4 dev was \
                     restarted); their chain history, and so their rows, are not on it.",
                );
            }
            body
        }
    };
    f.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(Color::DarkGray)),
        ),
        area,
    );
}

fn focused(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

/// `12s ago`.
pub(super) fn age(since: Instant, now: Instant) -> String {
    let seconds = now.saturating_duration_since(since).as_secs();
    match seconds {
        0 => "just now".to_string(),
        1..=59 => format!("{seconds}s ago"),
        60..=3599 => format!("{}m ago", seconds / 60),
        _ => format!("{}h ago", seconds / 3600),
    }
}

fn liveness(pane: &ProtocolPane, now: Instant) -> Span<'static> {
    match pane.conn {
        Conn::Live if pane.stack.ready => {
            let busy = pane
                .last_frame
                .is_some_and(|at| now.saturating_duration_since(at) < BUSY_WINDOW);
            let style = if busy {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Green)
            };
            Span::styled("\u{25cf} live", style)
        }
        Conn::Live => Span::styled("\u{25cb} loading", Style::default().fg(Color::Yellow)),
        Conn::Connecting => Span::styled("\u{25cb} starting", Style::default().fg(Color::Yellow)),
        Conn::Reconnecting => Span::styled("\u{25cc} retrying", Style::default().fg(Color::Yellow)),
        Conn::Down => Span::styled("\u{25cb} down", Style::default().fg(Color::Red)),
    }
}

fn instance_count(pane: &ProtocolPane) -> Option<i64> {
    pane.stack
        .instance_counts()
        .map(|counts| counts.total)
        .or_else(|| pane.rows(INSTANCE_ENTITY).map(|rows| rows as i64))
}

pub(super) fn draw_home(f: &mut ratatui::Frame, browser: &Browser, area: Rect) {
    let now = Instant::now();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(42), Constraint::Min(0)])
        .split(rows[0]);

    // Protocols.
    let visible = browser.visible_protocols();
    let width = visible
        .iter()
        .map(|&index| browser.protocols[index].stack.protocol_name().len())
        .max()
        .unwrap_or(0)
        .min(16);
    let items: Vec<ListItem> = visible
        .iter()
        .map(|&index| {
            let pane = &browser.protocols[index];
            let name: String = pane.stack.protocol_name().chars().take(16).collect();
            let version = pane.stack.protocol_version().unwrap_or("");
            let count = instance_count(pane)
                .map(|count| format!("{count}i"))
                .unwrap_or_default();
            ListItem::new(Line::from(vec![
                Span::raw(format!("{name:width$} {version:6} ")),
                liveness(pane, now),
                Span::raw(format!(" {count:>4}")),
            ]))
        })
        .collect();
    let mut state = ListState::default()
        .with_selected(visible.iter().position(|&index| index == browser.selected));
    let heading = pane_heading(
        browser
            .protocols
            .iter()
            .map(|pane| pane.stack.is_protocol()),
    );
    let title = match (&browser.filtering, browser.protocol_filter.as_str()) {
        (Some(Pane::Protocols), filter) => format!(" {heading} [/{filter}] "),
        (_, "") => format!(" {heading} "),
        (_, filter) => format!(" {heading} [{filter}] "),
    };
    f.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(focused(browser.focus == Pane::Protocols)),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> "),
        panes[0],
        &mut state,
    );

    // The selected protocol.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(focused(browser.focus == Pane::Views));
    match browser.protocols.get(browser.selected) {
        Some(pane) => draw_protocol(f, browser, pane, block, panes[1], now),
        None => f.render_widget(
            Paragraph::new("No protocol stacks are loaded.").block(block),
            panes[1],
        ),
    }

    let keys = if browser.protocols.len() > 1 {
        " Enter open \u{b7} Esc back \u{b7} / filter \u{b7} Tab protocols \u{2194} views \u{b7} q quit"
    } else {
        " Enter open \u{b7} Esc back \u{b7} / filter \u{b7} q quit"
    };
    let status = browser
        .status
        .as_ref()
        .filter(|(_, at)| now.saturating_duration_since(*at) < Duration::from_secs(8))
        .map(|(text, _)| text.as_str());
    draw_help(f, rows[1], keys, status);
}

/// The left pane's title: `Protocols`, `Stacks` (the project's own, no
/// protocol), or both.
fn pane_heading(kinds: impl Iterator<Item = bool>) -> &'static str {
    let (mut protocols, mut stacks) = (false, false);
    for is_protocol in kinds {
        if is_protocol {
            protocols = true;
        } else {
            stacks = true;
        }
    }
    match (protocols, stacks) {
        (false, true) => "Stacks",
        (true, true) => "Protocols & stacks",
        _ => "Protocols",
    }
}

fn draw_protocol(
    f: &mut ratatui::Frame,
    browser: &Browser,
    pane: &ProtocolPane,
    block: Block,
    area: Rect,
    now: Instant,
) {
    let stack = &pane.stack;
    let mut title = vec![stack.protocol_name().to_string()];
    title.extend(stack.protocol_version().map(str::to_string));
    let mut title = format!(" {} ", title.join(" "));
    if let Some(deployment) = stack.deployment_label() {
        title = format!("{title}\u{b7} {deployment} ");
    } else if !stack.is_protocol() {
        title = format!("{title}\u{b7} stack ");
    }
    let block = block.title(Span::styled(
        title,
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ));

    let mut lines: Vec<Line> = Vec::new();
    let dim = Style::default().fg(Color::DarkGray);
    // Instances 2 (closed 2)   Activity 4   Participants —
    let mut summary = Vec::new();
    match stack.instance_counts() {
        Some(counts) if counts.closed > 0 => summary.push(format!(
            "Instances {} (closed {})",
            counts.total, counts.closed
        )),
        Some(counts) => summary.push(format!("Instances {}", counts.total)),
        None => {
            if let Some(rows) = pane.rows(INSTANCE_ENTITY) {
                summary.push(format!("Instances {rows}"));
            }
        }
    }
    if pane.primary(ACTIVITY_ENTITY).is_some() {
        summary.push(match pane.rows(ACTIVITY_ENTITY) {
            Some(rows) => format!("Activity {rows}"),
            None => "Activity \u{2026}".to_string(),
        });
    }
    if pane.primary(PARTICIPANT_ENTITY).is_some() {
        let participants = stack
            .current
            .as_ref()
            .and_then(|current| current.participants);
        summary.push(match (stack.has_participants(), participants) {
            (Some(false), _) => "Participants \u{2014}".to_string(),
            (_, Some(count)) => format!("Participants {count}"),
            _ => match pane.rows(PARTICIPANT_ENTITY) {
                Some(rows) => format!("Participants {rows}"),
                None => "Participants \u{2026}".to_string(),
            },
        });
    }
    lines.push(Line::from(summary.join("   ")));
    if let Some(notice) = browser
        .fork
        .as_ref()
        .and_then(|fork| fork.notice(stack.instance_counts()?.before_fork))
    {
        lines.push(Line::from(Span::styled(
            format!("! {notice}"),
            Style::default().fg(Color::Yellow),
        )));
    }
    lines.push(Line::from(vec![
        Span::styled(
            format!("stack {} \u{b7} {} \u{b7} ", stack.name, stack.ws_url),
            dim,
        ),
        liveness(pane, now),
    ]));
    lines.push(Line::from(""));
    let filter = match (&browser.filtering, browser.view_filter.as_str()) {
        (Some(Pane::Views), filter) => format!("  [/{filter}]"),
        (_, "") => String::new(),
        (_, filter) => format!("  [{filter}]"),
    };
    lines.push(Line::from(Span::styled(
        format!("  {:28} {:>5}  updated{filter}", "Views", "rows"),
        dim.add_modifier(Modifier::BOLD),
    )));
    let selected_row = (browser.focus == Pane::Views).then_some(browser.view_row);
    for (index, row) in pane.home_rows(&browser.view_filter).iter().enumerate() {
        let marker = if selected_row == Some(index) {
            "> "
        } else {
            "  "
        };
        let style = if selected_row == Some(index) {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let line = match row {
            HomeRow::View(view) => {
                let feed = &pane.views[*view];
                let name = format!("{} ({})", feed.entry.friendly, feed.entry.mode);
                let no_machine = feed.entry.entity == PARTICIPANT_ENTITY
                    && stack.has_participants() == Some(false);
                let (rows, updated) = if no_machine {
                    (
                        "\u{2014}".to_string(),
                        protocols::empty_reason(stack, &feed.entry),
                    )
                } else if !feed.loaded {
                    ("\u{2026}".to_string(), "loading".to_string())
                } else {
                    let rows = feed.app.entity_keys.len();
                    let updated = match feed.last_update {
                        _ if rows == 0 => protocols::empty_reason(stack, &feed.entry),
                        Some(at) => age(at, now),
                        None => "\u{2014}".to_string(),
                    };
                    (rows.to_string(), updated)
                };
                Line::from(vec![
                    Span::styled(format!("{marker}{name:28} {rows:>5}  "), style),
                    Span::styled(updated, dim),
                ])
            }
            HomeRow::Instance => Line::from(vec![
                Span::styled(
                    format!("{marker}{:28} {:>5}  ", "Instance \u{2026} (state)", ""),
                    style,
                ),
                Span::styled("pick an instance to open", dim),
            ]),
        };
        lines.push(line);
    }
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::stream::protocols::tests::protocol_stack;
    use arete_sdk::{Mode, SnapshotEntity};
    use ratatui::{backend::TestBackend, Terminal};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn listing(count: usize) -> DevListing {
        let stacks = [
            ("local-tip-jar", "localTipJar", "yurqmejlgs"),
            ("token-raise", "tokenRaise", "raise12345"),
        ];
        DevListing {
            stacks: stacks[..count]
                .iter()
                .map(|(name, protocol, short)| protocol_stack(name, protocol, short))
                .collect(),
            fork: Some(ForkMarker {
                started_at: Some("2026-10-14T14:02:00Z".to_string()),
                start_slot: Some(7),
                instances_before: 2,
            }),
        }
    }

    fn browser(count: usize) -> Browser {
        Browser::new(listing(count), None, Arc::new(AtomicU64::new(0)), true)
    }

    fn upsert(subscription: &str, key: &str, data: serde_json::Value) -> Frame {
        Frame::Upsert {
            protocol_version: 2,
            subscription_id: subscription.to_string(),
            mode: Mode::List,
            entity: "ProtocolInstance".to_string(),
            key: key.to_string(),
            data,
            append: Vec::new(),
            seq: None,
            offset: None,
        }
    }

    fn feed_id(browser: &Browser, protocol: usize, raw: &str) -> String {
        browser.protocols[protocol]
            .views
            .iter()
            .find(|feed| feed.entry.raw == raw)
            .unwrap()
            .subscription_id
            .clone()
    }

    #[test]
    fn one_protocol_starts_on_its_views() {
        let browser = browser(1);
        assert_eq!(browser.focus, Pane::Views);
        assert_eq!(
            browser.home_rows(),
            [
                HomeRow::View(0),
                HomeRow::View(1),
                HomeRow::View(2),
                HomeRow::Instance
            ]
        );
        // Keyed views are not streamed; the rest are, primaries first.
        let raws: Vec<&str> = browser.protocols[0]
            .views
            .iter()
            .map(|feed| feed.entry.raw.as_str())
            .collect();
        assert_eq!(
            raws,
            [
                "ProtocolInstance/list",
                "ProtocolActivity/list",
                "ProtocolParticipant/list",
                "ProtocolInstance/byId",
                "ProtocolActivity/byInstance",
                "ProtocolParticipant/byInstance",
            ]
        );
    }

    #[test]
    fn enter_esc_and_tab_move_between_the_levels() {
        let mut browser = browser(2);
        assert_eq!(browser.focus, Pane::Protocols);
        // Level 1: pick the second protocol.
        browser.on_key(key(KeyCode::Down));
        assert_eq!(browser.selected, 1);
        // Tab and back.
        browser.on_key(key(KeyCode::Tab));
        assert_eq!(browser.focus, Pane::Views);
        browser.on_key(key(KeyCode::BackTab));
        assert_eq!(browser.focus, Pane::Protocols);
        // Level 2.
        browser.on_key(key(KeyCode::Enter));
        assert_eq!(browser.focus, Pane::Views);
        browser.on_key(key(KeyCode::Down));
        assert_eq!(browser.view_row, 1);
        // Level 3: Activity.
        browser.on_key(key(KeyCode::Enter));
        assert!(matches!(
            browser.screen,
            Screen::View {
                protocol: 1,
                view: 1
            }
        ));
        // Esc back to level 2, again to level 1.
        browser.on_key(key(KeyCode::Esc));
        assert!(matches!(browser.screen, Screen::Home));
        assert_eq!(browser.focus, Pane::Views);
        browser.on_key(key(KeyCode::Esc));
        assert_eq!(browser.focus, Pane::Protocols);
        assert_eq!(browser.on_key(key(KeyCode::Char('q'))), [Command::Quit]);
    }

    #[test]
    fn enter_on_an_instance_opens_its_state_and_activity() {
        let mut browser = browser(1);
        let instances = feed_id(&browser, 0, "ProtocolInstance/list");
        let tag = browser.protocols[0].tag;
        let uuid = uuid::Uuid::new_v4();
        let short = bs58::encode(uuid.as_bytes()).into_string();
        browser.apply(NavEvent::Frame(
            tag,
            0,
            upsert(&instances, &short, serde_json::json!({})),
        ));

        // The Instance row leads to Instances, where Enter opens one.
        browser.view_row = 3;
        browser.on_key(key(KeyCode::Enter));
        assert!(matches!(
            browser.screen,
            Screen::View {
                protocol: 0,
                view: 0
            }
        ));
        assert_eq!(browser.on_key(key(KeyCode::Enter)), [Command::OpenInstance]);
        let Screen::Instance(instance) = &browser.screen else {
            panic!("an instance screen");
        };
        assert_eq!(instance.key.key, short);
        assert_eq!(instance.key.uuid, Some(uuid.to_string()));
        assert!(instance.state_sub.ends_with("ProtocolInstance/state"));
        let views: Vec<&str> = instance
            .feeds
            .iter()
            .map(|feed| feed.entry.raw.as_str())
            .collect();
        // The tip jar has no participant machine.
        assert_eq!(
            views,
            ["ProtocolActivity/byInstance", "ProtocolInstance/byId"]
        );
        let instance_tag = instance.tag;
        let activity = instance.feeds[0].subscription_id.clone();
        let by_id = instance.feeds[1].subscription_id.clone();

        // Its activity, and nobody else's.
        browser.apply(NavEvent::Frame(
            instance_tag,
            0,
            upsert(&activity, "sig1", serde_json::json!({ "instance": short })),
        ));
        browser.apply(NavEvent::Frame(
            instance_tag,
            0,
            upsert(
                &activity,
                "sig2",
                serde_json::json!({ "instance": "other" }),
            ),
        ));
        // Its row in Instances/byId, and nobody else's.
        browser.apply(NavEvent::Frame(
            instance_tag,
            0,
            upsert(
                &by_id,
                &short,
                serde_json::json!({ "id": { "instance": short } }),
            ),
        ));
        browser.apply(NavEvent::Frame(
            instance_tag,
            0,
            upsert(
                &by_id,
                "other",
                serde_json::json!({ "id": { "instance": "other" } }),
            ),
        ));
        // The machine state from the local API reaches the header.
        browser.apply(NavEvent::Machine(
            instance_tag,
            MachineState::from_detail(&serde_json::json!({ "state": "closed", "terminal": true })),
        ));
        let Screen::Instance(instance) = &mut browser.screen else {
            panic!("an instance screen");
        };
        assert_eq!(instance.feeds[0].app.entity_keys, ["sig1"]);
        assert_eq!(
            instance.feeds[1].app.entity_keys,
            std::slice::from_ref(&short)
        );
        // Rows carry the instance id next to the short id.
        instance.feeds[1].app.ensure_filtered_cache();
        let row: serde_json::Value =
            serde_json::from_str(&instance.feeds[1].app.selected_entity_data().unwrap()).unwrap();
        assert_eq!(row["id"]["instance_id"], uuid.to_string());
        assert_eq!(
            machine_label(instance.machine.as_ref()).0,
            "closed (terminal)"
        );

        // Tab moves between the per-instance views.
        assert!(browser.on_key(key(KeyCode::Tab)).is_empty());
        let Screen::Instance(instance) = &browser.screen else {
            panic!("an instance screen");
        };
        assert_eq!(instance.active, 1);
        browser.on_key(key(KeyCode::BackTab));

        // Esc closes it and goes back to Instances.
        assert_eq!(
            browser.on_key(key(KeyCode::Esc)),
            [Command::CloseInstance(instance_tag)]
        );
        assert!(matches!(
            browser.screen,
            Screen::View {
                protocol: 0,
                view: 0
            }
        ));
    }

    #[test]
    fn a_target_opens_its_level() {
        let mut browser = browser(2);
        let commands = browser.start_at(&DevTarget::View {
            stack: "token-raise".to_string(),
            view: "ProtocolActivity/list".to_string(),
            pinned: true,
        });
        assert!(commands.is_empty());
        assert!(matches!(
            browser.screen,
            Screen::View {
                protocol: 1,
                view: 1
            }
        ));

        let mut browser = super::tests::browser(2);
        let key = InstanceKey::parse("8c2b1c56-3f7e-4f0a-9d7e-2b1f0c6a5e11").unwrap();
        let commands = browser.start_at(&DevTarget::Instance {
            stack: "token-raise".to_string(),
            key,
            pinned: true,
        });
        assert_eq!(commands, [Command::OpenInstance]);
        // Esc from an instance opened by --instance goes to the protocol.
        let Screen::Instance(instance) = &browser.screen else {
            panic!("an instance screen");
        };
        let tag = instance.tag;
        assert_eq!(
            browser.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            [Command::CloseInstance(tag)]
        );
        assert!(matches!(browser.screen, Screen::Home));
        assert_eq!(browser.selected, 1);
        assert_eq!(browser.focus, Pane::Views);
    }

    #[test]
    fn slash_filters_the_focused_pane() {
        let mut browser = browser(2);
        for code in [
            KeyCode::Char('/'),
            KeyCode::Char('r'),
            KeyCode::Char('a'),
            KeyCode::Enter,
        ] {
            browser.on_key(key(code));
        }
        assert_eq!(browser.visible_protocols(), [1]);
        assert_eq!(browser.selected, 1);
        browser.on_key(key(KeyCode::Esc));
        assert_eq!(browser.visible_protocols(), [0, 1]);

        browser.on_key(key(KeyCode::Enter));
        for code in [KeyCode::Char('/'), KeyCode::Char('a'), KeyCode::Char('c')] {
            browser.on_key(key(code));
        }
        assert_eq!(browser.home_rows(), [HomeRow::View(1)]);
        // Esc while typing clears the filter.
        browser.on_key(key(KeyCode::Esc));
        assert_eq!(browser.home_rows().len(), 4);
    }

    #[test]
    fn instance_frames_keep_their_instances_rows() {
        let mut admitted = HashSet::new();
        let snapshot = Frame::Snapshot {
            protocol_version: 2,
            subscription_id: "s".to_string(),
            snapshot_id: "1".to_string(),
            authoritative: true,
            mode: Mode::List,
            entity: "ProtocolActivity".to_string(),
            key: None,
            data: vec![
                SnapshotEntity {
                    key: "a".to_string(),
                    data: serde_json::json!({ "instance": "mine" }),
                },
                SnapshotEntity {
                    key: "b".to_string(),
                    data: serde_json::json!({ "instance": "theirs" }),
                },
            ],
            complete: true,
        };
        let belongs = |row: &str, data: &serde_json::Value, key: &str| {
            protocols::row_of_instance(ACTIVITY_ENTITY, row, data, key)
        };
        let Some(Frame::Snapshot { data, .. }) =
            instance_frame(snapshot, "mine", &mut admitted, belongs)
        else {
            panic!("a snapshot");
        };
        assert_eq!(data.len(), 1);
        let patch = |key: &str| Frame::Patch {
            protocol_version: 2,
            subscription_id: "s".to_string(),
            mode: Mode::List,
            entity: "ProtocolActivity".to_string(),
            key: key.to_string(),
            data: serde_json::json!({ "slot": 9 }),
            append: Vec::new(),
            seq: None,
            offset: None,
        };
        assert!(instance_frame(patch("a"), "mine", &mut admitted, belongs).is_some());
        assert!(instance_frame(patch("b"), "mine", &mut admitted, belongs).is_none());
    }

    #[test]
    fn ages_read_naturally() {
        let now = Instant::now() + Duration::from_secs(7200);
        assert_eq!(age(now, now), "just now");
        assert_eq!(age(now - Duration::from_secs(12), now), "12s ago");
        assert_eq!(age(now - Duration::from_secs(150), now), "2m ago");
        assert_eq!(age(now - Duration::from_secs(7200), now), "2h ago");
    }

    #[test]
    fn the_home_screen_shows_protocols_views_and_the_restart() {
        colored::control::set_override(false);
        let mut browser = browser(2);
        let tag = browser.protocols[0].tag;
        let instances = feed_id(&browser, 0, "ProtocolInstance/list");
        browser.apply(NavEvent::Connected(tag));
        browser.apply(NavEvent::Frame(
            tag,
            0,
            upsert(&instances, "i1", serde_json::json!({})),
        ));
        browser.focus = Pane::Views;

        let mut terminal = Terminal::new(TestBackend::new(110, 16)).unwrap();
        terminal.draw(|f| draw_home(f, &browser, f.area())).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .chunks(110)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>() + "\n")
            .collect();
        assert!(
            text.contains("> localTipJar 0.1.0  \u{25cf} live   2i"),
            "{text}"
        );
        assert!(
            text.contains("tokenRaise  0.1.0  \u{25cb} starting   2i"),
            "{text}"
        );
        assert!(
            text.contains("localTipJar 0.1.0 \u{b7} deployment yurqmejlgs (active)"),
            "{text}"
        );
        assert!(
            text.contains("Instances 2 (closed 2)   Activity \u{2026}   Participants \u{2014}"),
            "{text}"
        );
        assert!(
            text.contains("! 2 instances predate this fork (restarted"),
            "{text}"
        );
        assert!(text.contains("> Instances (list)"), "{text}");
        assert!(
            text.contains("Participants (list)              \u{2014}  no participants"),
            "{text}"
        );
        assert!(text.contains("Instance \u{2026} (state)"), "{text}");
        assert!(text.contains("pick an instance to open"), "{text}");
        assert!(text.contains("Tab protocols \u{2194} views"), "{text}");
    }

    #[test]
    fn the_left_pane_names_what_it_lists() {
        assert_eq!(pane_heading([true, true].into_iter()), "Protocols");
        assert_eq!(pane_heading([false].into_iter()), "Stacks");
        assert_eq!(
            pane_heading([true, false].into_iter()),
            "Protocols & stacks"
        );
        assert_eq!(pane_heading(std::iter::empty()), "Protocols");
    }
}
