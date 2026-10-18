//! Protocol-first names for `a4 stream --dev`.
//!
//! A protocol's generated stack serves `ProtocolInstance`,
//! `ProtocolActivity` and `ProtocolParticipant` rows. Here they are the
//! protocol's **Instances**, **Activity** and **Participants**: one
//! top-level view each (its `list` view, else its first collection), while
//! the keyed `<Entity>/state` views and the `byInstance` collections are
//! per-instance drill-ins. Other entities keep their own names.
//!
//! The project's own stacks (no protocol deployment records them) are
//! listed after the protocols, by stack name, with their entities' views.
//!
//! Protocols are matched by the spec's name (`localTipJar`), the protocol's
//! slug, the stack's name or the deployment's short id; views by friendly
//! name (`Activity`), `Friendly/mode` (`Activity/byInstance`), entity name
//! or raw view id (`ProtocolActivity/list`). An instance is the id
//! `a4 dev step` prints (a UUID) or the short id the stream keys rows by
//! (base58 of the UUID's bytes).

use anyhow::{bail, Result};
use arete_mcp::dev::DevSession;
use colored::Colorize;

use super::dev::{select_stack, DevListing, DevStack};

pub const INSTANCE_ENTITY: &str = "ProtocolInstance";
pub const ACTIVITY_ENTITY: &str = "ProtocolActivity";
pub const PARTICIPANT_ENTITY: &str = "ProtocolParticipant";

/// The friendly name of a protocol stack entity.
pub fn friendly_entity(entity: &str) -> Option<&'static str> {
    match entity {
        INSTANCE_ENTITY => Some("Instances"),
        ACTIVITY_ENTITY => Some("Activity"),
        PARTICIPANT_ENTITY => Some("Participants"),
        _ => None,
    }
}

/// `localTipJar`, `local-tip-jar`, `Local Tip Jar` all compare equal.
fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

impl DevStack {
    /// The protocol's name from its spec, else its slug, else the stack's
    /// name.
    pub fn protocol_name(&self) -> &str {
        self.current
            .as_ref()
            .and_then(|current| current.protocol_name.as_deref())
            .or(self.protocol.as_deref())
            .or_else(|| {
                self.current
                    .as_ref()
                    .and_then(|current| current.protocol.as_deref())
            })
            .unwrap_or(&self.name)
    }

    pub fn protocol_version(&self) -> Option<&str> {
        self.current.as_ref()?.protocol_version.as_deref()
    }

    /// Whether a protocol deployment records the stack.
    pub fn is_protocol(&self) -> bool {
        self.current.is_some() || self.protocol.is_some()
    }

    /// The deployment it shows: its short id and status.
    pub fn deployment_label(&self) -> Option<String> {
        if let Some(current) = &self.current {
            let id = current
                .short_id
                .clone()
                .unwrap_or_else(|| current.deployment_id.clone());
            return Some(match current.status.as_str() {
                "" => format!("deployment {id}"),
                status => format!("deployment {id} ({status})"),
            });
        }
        self.short_id
            .as_ref()
            .map(|short_id| format!("deployment {short_id} (active)"))
    }

    /// `false` when the spec declares no participant machine.
    pub fn has_participants(&self) -> Option<bool> {
        self.current.as_ref()?.has_participants
    }

    /// Instances of the deployment it shows, in the local database.
    pub fn instance_counts(&self) -> Option<super::dev::InstanceCounts> {
        self.current.as_ref()?.instances
    }

    /// Whether `input` names this stack's protocol.
    fn answers_to(&self, input: &str) -> bool {
        let wanted = normalize(input);
        if wanted.is_empty() {
            return false;
        }
        let mut names = vec![self.name.as_str(), self.protocol_name()];
        names.extend(self.protocol.as_deref());
        names.extend(self.short_id.as_deref());
        names.extend(self.deployment_id.as_deref());
        if let Some(current) = &self.current {
            names.extend(current.protocol.as_deref());
            names.extend(current.short_id.as_deref());
            names.push(current.deployment_id.as_str());
        }
        names.iter().any(|name| normalize(name) == wanted)
    }

    /// The stack's views, as the protocol presents them.
    pub fn catalog(&self) -> Vec<ViewEntry> {
        catalog(&self.views)
    }
}

/// How a view sits in a protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewRole {
    /// The entity's top-level view.
    Primary,
    /// Another collection of the same rows (`ProtocolInstance/byId`).
    Alternate,
    /// One row by key (`<Entity>/state`): `ProtocolInstance/state` is an
    /// instance's state.
    Keyed,
    /// Rows to filter by instance (`ProtocolActivity/byInstance`).
    PerInstance,
}

/// One view of a protocol's stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewEntry {
    /// `ProtocolActivity/list`.
    pub raw: String,
    pub entity: String,
    pub mode: String,
    /// `Activity` for a primary view, `Activity/byInstance` for the others.
    pub friendly: String,
    pub role: ViewRole,
}

impl ViewEntry {
    /// The entity's friendly name, or the entity.
    pub fn entity_title(&self) -> &str {
        friendly_entity(&self.entity).unwrap_or(&self.entity)
    }

    /// Whether `--all` and the TUI can subscribe to it without a key.
    pub fn streamable(&self) -> bool {
        self.role != ViewRole::Keyed
    }
}

/// The protocol's views: each entity's primary view first (Instances,
/// Activity, Participants, then the others in listing order), then the
/// rest.
pub fn catalog(views: &[String]) -> Vec<ViewEntry> {
    let mut entities: Vec<&str> = Vec::new();
    for view in views {
        let entity = view.split_once('/').map_or(view.as_str(), |(e, _)| e);
        if !entities.contains(&entity) {
            entities.push(entity);
        }
    }
    let rank = |entity: &str| match entity {
        INSTANCE_ENTITY => 0,
        ACTIVITY_ENTITY => 1,
        PARTICIPANT_ENTITY => 2,
        _ => 3,
    };
    entities.sort_by_key(|entity| rank(entity));

    let mut primaries = Vec::new();
    let mut rest = Vec::new();
    for entity in entities {
        let modes: Vec<&str> = views
            .iter()
            .filter_map(|view| view.split_once('/'))
            .filter(|(e, _)| *e == entity)
            .map(|(_, mode)| mode)
            .collect();
        let primary = modes
            .iter()
            .find(|mode| **mode == "list")
            .or_else(|| {
                modes
                    .iter()
                    .find(|mode| **mode != "state" && **mode != "byInstance")
            })
            .copied();
        let title = friendly_entity(entity).unwrap_or(entity);
        for mode in modes {
            let role = if Some(mode) == primary {
                ViewRole::Primary
            } else if mode == "state" {
                ViewRole::Keyed
            } else if mode == "byInstance" {
                ViewRole::PerInstance
            } else {
                ViewRole::Alternate
            };
            let entry = ViewEntry {
                raw: format!("{entity}/{mode}"),
                entity: entity.to_string(),
                mode: mode.to_string(),
                friendly: if role == ViewRole::Primary {
                    title.to_string()
                } else {
                    format!("{title}/{mode}")
                },
                role,
            };
            if role == ViewRole::Primary {
                primaries.push(entry);
            } else {
                rest.push(entry);
            }
        }
    }
    primaries.extend(rest);
    primaries
}

/// The view `input` names among `catalog`.
pub fn resolve_view<'a>(catalog: &'a [ViewEntry], input: &str) -> Result<&'a ViewEntry> {
    if let Some(entry) = catalog.iter().find(|entry| entry.raw == input) {
        return Ok(entry);
    }
    let wanted = normalize(input);
    let singular = |name: &str| normalize(name.trim_end_matches('s'));
    let found = catalog
        .iter()
        .find(|entry| normalize(&entry.raw) == wanted || normalize(&entry.friendly) == wanted)
        .or_else(|| {
            // An entity, plural or singular: its primary view.
            catalog.iter().find(|entry| {
                entry.role == ViewRole::Primary
                    && (normalize(&entry.entity) == wanted
                        || singular(entry.entity_title()) == singular(input))
            })
        })
        .or_else(|| {
            // `Instance/state`, `Activities/byInstance`.
            let (entity, mode) = input.split_once('/')?;
            catalog.iter().find(|entry| {
                entry.mode == mode && singular(entry.entity_title()) == singular(entity)
            })
        })
        .or_else(|| {
            // A view by its own name (`largest`), when one entity has it.
            let mut named = catalog
                .iter()
                .filter(|entry| entry.role != ViewRole::Keyed && normalize(&entry.mode) == wanted);
            let first = named.next()?;
            named.next().is_none().then_some(first)
        });
    match found {
        Some(entry) => Ok(entry),
        None => bail!(
            "No view named '{input}'.\n{}",
            describe_views(catalog).join("\n")
        ),
    }
}

/// `  Activity       ProtocolActivity/list` lines.
fn describe_views(catalog: &[ViewEntry]) -> Vec<String> {
    let width = catalog
        .iter()
        .map(|entry| entry.friendly.len())
        .max()
        .unwrap_or(0);
    let mut lines = vec!["Views:".to_string()];
    for entry in catalog {
        lines.push(format!(
            "  {:width$}  {}",
            entry.friendly,
            entry.raw,
            width = width
        ));
    }
    lines
}

/// The stack serving the protocol `input` names.
pub fn resolve_protocol<'a>(stacks: &'a [DevStack], input: &str) -> Result<&'a DevStack> {
    match find_protocol(stacks, input) {
        Some(stack) => Ok(stack),
        None => bail!(
            "No protocol or stack named '{input}' in the a4 dev session.\n{}",
            available_protocols(stacks)
        ),
    }
}

/// The stack whose protocol `input` names, if exactly one does.
pub fn find_protocol<'a>(stacks: &'a [DevStack], input: &str) -> Option<&'a DevStack> {
    let mut matches = stacks.iter().filter(|stack| stack.answers_to(input));
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn available_protocols(stacks: &[DevStack]) -> String {
    if stacks.is_empty() {
        return "No stacks are loaded.".to_string();
    }
    let protocols: Vec<&str> = stacks
        .iter()
        .filter(|stack| stack.is_protocol())
        .map(DevStack::protocol_name)
        .collect();
    let plain: Vec<&str> = stacks
        .iter()
        .filter(|stack| !stack.is_protocol())
        .map(|stack| stack.name.as_str())
        .collect();
    let mut parts = Vec::new();
    if !protocols.is_empty() {
        parts.push(format!("Protocols: {}", protocols.join(", ")));
    }
    if !plain.is_empty() {
        parts.push(format!("Stacks: {}", plain.join(", ")));
    }
    parts.join("\n")
}

/// An instance as the stream keys it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceKey {
    /// The `ProtocolInstance` row key and the `instance` field of its
    /// activity rows.
    pub key: String,
    /// The instance id, when `key` is one in its short form.
    pub uuid: Option<String>,
}

impl InstanceKey {
    /// `a4 dev step`'s UUID becomes the short id the stack keys rows by
    /// (base58 of its 16 bytes); a short id is taken as it is, and so is
    /// any other key (an interim-profile stack keys instances by account).
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            bail!("--instance needs an instance id");
        }
        // Hyphenated only: 32 bare hex digits can be an account address.
        if let Some(uuid) = uuid::Uuid::try_parse(input)
            .ok()
            .filter(|_| input.len() == 36)
        {
            return Ok(Self {
                key: bs58::encode(uuid.as_bytes()).into_string(),
                uuid: Some(uuid.to_string()),
            });
        }
        let uuid = bs58::decode(input)
            .into_vec()
            .ok()
            .and_then(|bytes| uuid::Uuid::from_slice(&bytes).ok())
            .map(|uuid| uuid.to_string());
        Ok(Self {
            key: input.to_string(),
            uuid,
        })
    }

    /// `<short id> (instance <uuid>)`.
    pub fn describe(&self) -> String {
        match &self.uuid {
            Some(uuid) => format!("{} (instance {uuid})", self.key),
            None => self.key.clone(),
        }
    }
}

/// Whether an activity row belongs to the instance keyed `key`: its short
/// id, or the account the activity names.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub fn activity_of(row: &serde_json::Value, key: &str) -> bool {
    ["instance", "activity_account"]
        .iter()
        .any(|field| row.get(field).and_then(serde_json::Value::as_str) == Some(key))
}

/// The instance id (a UUID) of a short id: base58 of the UUID's 16 bytes.
pub fn instance_uuid(short_id: &str) -> Option<String> {
    let bytes = bs58::decode(short_id).into_vec().ok()?;
    let uuid = uuid::Uuid::from_slice(&bytes).ok()?;
    Some(uuid.to_string())
}

/// Add the instance id (`instance_id`, a UUID) next to the short id a
/// protocol row carries, so stream rows line up with the API's instances
/// and `a4 dev step`'s output. The stack cannot do it: its expression
/// language has no UUID formatting. `ProtocolInstance` rows carry the short
/// id at `id.instance` and get `id.instance_id`; `ProtocolActivity` and
/// `ProtocolParticipant` rows carry it at `instance` and get `instance_id`.
/// Rows of other entities, rows without a short id (a patch that leaves it
/// out) and interim-profile rows (keyed by account) are left alone.
pub fn add_instance_id(view: &str, data: &mut serde_json::Value) {
    let entity = view.split('/').next().unwrap_or_default();
    let target = match entity {
        INSTANCE_ENTITY => data.get_mut("id"),
        ACTIVITY_ENTITY | PARTICIPANT_ENTITY => Some(data),
        _ => None,
    };
    let Some(serde_json::Value::Object(row)) = target else {
        return;
    };
    let Some(uuid) = row
        .get("instance")
        .and_then(serde_json::Value::as_str)
        .and_then(instance_uuid)
    else {
        return;
    };
    row.insert("instance_id".into(), serde_json::Value::String(uuid));
}

/// [`add_instance_id`] for every row a frame carries.
pub fn add_instance_ids(view: &str, frame: &mut arete_sdk::Frame) {
    use arete_sdk::Frame;
    match frame {
        Frame::Snapshot { data, .. } => {
            for row in data {
                add_instance_id(view, &mut row.data);
            }
        }
        Frame::Upsert { data, .. } | Frame::Patch { data, .. } => add_instance_id(view, data),
        _ => {}
    }
}

/// What `a4 stream --dev` streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevTarget {
    /// No view: list the protocols (or open the TUI on them).
    Overview,
    /// One protocol (or `--stack`): its views.
    Protocol { stack: String, pinned: bool },
    /// One view of a stack.
    View {
        stack: String,
        view: String,
        pinned: bool,
    },
    /// `--all`: every view of a stack but the keyed ones.
    All { stack: String, pinned: bool },
    /// `--instance`: one instance's state and activity.
    Instance {
        stack: String,
        key: InstanceKey,
        pinned: bool,
    },
}

impl DevTarget {
    /// The stack it reads.
    pub fn stack(&self) -> Option<&str> {
        match self {
            Self::Overview => None,
            Self::Protocol { stack, .. }
            | Self::View { stack, .. }
            | Self::All { stack, .. }
            | Self::Instance { stack, .. } => Some(stack),
        }
    }

    /// Whether the user named the stack (by protocol or `--stack`), so the
    /// stream follows it by name rather than whichever stack the active
    /// deployment moves to.
    pub fn pinned(&self) -> bool {
        match self {
            Self::Overview => false,
            Self::Protocol { pinned, .. }
            | Self::View { pinned, .. }
            | Self::All { pinned, .. }
            | Self::Instance { pinned, .. } => *pinned,
        }
    }
}

/// What the user asked `a4 stream --dev` for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DevRequest {
    /// The positionals: `[PROTOCOL] [VIEW]`, or a view of the default
    /// protocol.
    pub first: Option<String>,
    pub second: Option<String>,
    pub stack: Option<String>,
    pub all: bool,
    pub instance: Option<String>,
}

/// Resolve `request` against the session's stacks.
///
/// - `--stack <name>` picks the stack; a positional is then its view.
/// - Otherwise the first positional is a protocol, then its view; failing
///   that, a view of the default protocol (the only one, or the active
///   deployment's), as before protocols were named.
pub fn resolve_target(
    request: &DevRequest,
    stacks: &[DevStack],
    session: &DevSession,
) -> Result<DevTarget> {
    if request.all && request.instance.is_some() {
        bail!("--all and --instance stream different things; choose one.");
    }
    let (stack, view, pinned) = if let Some(name) = &request.stack {
        if request.second.is_some() {
            bail!(
                "With --stack, name one view: a4 stream --dev --stack {name} <VIEW> (protocols \
                 are named without --stack)."
            );
        }
        let stack = select_stack(stacks, Some(name), session)?;
        (Some(stack), request.first.clone(), true)
    } else if let Some(first) = &request.first {
        match find_protocol(stacks, first) {
            Some(stack) => (Some(stack), request.second.clone(), true),
            None if request.second.is_none() => {
                // A view of the default protocol, if it has one by that name.
                let stack = select_stack(stacks, None, session).ok();
                match stack.filter(|stack| resolve_view(&stack.catalog(), first).is_ok()) {
                    Some(stack) => (Some(stack), Some(first.clone()), false),
                    None => bail!(
                        "No protocol, stack or view named '{first}' in the a4 dev session.\n{}",
                        available_protocols(stacks)
                    ),
                }
            }
            None => {
                resolve_protocol(stacks, first)?;
                unreachable!("resolve_protocol fails when find_protocol finds nothing")
            }
        }
    } else {
        (None, None, false)
    };

    let stack = match stack {
        Some(stack) => stack,
        None if request.all || request.instance.is_some() => select_stack(stacks, None, session)?,
        None => return Ok(DevTarget::Overview),
    };
    let name = stack.name.clone();
    if let Some(view) = view {
        if request.all {
            bail!("--all streams every view; drop '{view}' or --all.");
        }
        if request.instance.is_some() {
            bail!("--instance streams the instance's state and activity; drop the view '{view}'.");
        }
        let view = resolve_view(&stack.catalog(), &view)?.raw.clone();
        return Ok(DevTarget::View {
            stack: name,
            view,
            pinned,
        });
    }
    if let Some(instance) = &request.instance {
        return Ok(DevTarget::Instance {
            stack: name,
            key: InstanceKey::parse(instance)?,
            pinned,
        });
    }
    if request.all {
        return Ok(DevTarget::All {
            stack: name,
            pinned,
        });
    }
    Ok(DevTarget::Protocol {
        stack: name,
        pinned,
    })
}

/// The views a protocol's merged feed (`a4 stream --dev <protocol>`)
/// subscribes to: each entity's primary view.
pub fn primary_views(stack: &DevStack) -> Vec<ViewEntry> {
    stack
        .catalog()
        .into_iter()
        .filter(|entry| entry.role == ViewRole::Primary)
        .collect()
}

/// The `ProtocolInstance/state` and activity views an instance reads.
pub fn instance_views(stack: &DevStack) -> (String, Option<String>) {
    let catalog = stack.catalog();
    let state = catalog
        .iter()
        .find(|entry| entry.entity == INSTANCE_ENTITY && entry.mode == "state")
        .map(|entry| entry.raw.clone())
        .unwrap_or_else(|| format!("{INSTANCE_ENTITY}/state"));
    let activity = ["byInstance", "list"].iter().find_map(|mode| {
        catalog
            .iter()
            .find(|entry| entry.entity == ACTIVITY_ENTITY && entry.mode == *mode)
            .map(|entry| entry.raw.clone())
    });
    (state, activity)
}

/// The collections an instance screen shows filtered to one instance:
/// each entity's `byInstance` view (Activity, then Participants unless the
/// spec declares no participant machine) and `ProtocolInstance/byId`. A
/// stack without per-instance views falls back to the activity list.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub fn instance_collections(stack: &DevStack) -> Vec<ViewEntry> {
    let catalog = stack.catalog();
    let mut collections: Vec<ViewEntry> = catalog
        .iter()
        .filter(|entry| entry.role == ViewRole::PerInstance)
        .filter(|entry| {
            entry.entity != PARTICIPANT_ENTITY || stack.has_participants() != Some(false)
        })
        .cloned()
        .collect();
    collections.extend(
        catalog
            .iter()
            .filter(|entry| entry.entity == INSTANCE_ENTITY && entry.mode == "byId")
            .cloned(),
    );
    if !collections
        .iter()
        .any(|entry| entry.entity == ACTIVITY_ENTITY)
    {
        if let Some(list) = catalog
            .iter()
            .find(|entry| entry.entity == ACTIVITY_ENTITY && entry.mode == "list")
        {
            collections.insert(0, list.clone());
        }
    }
    collections
}

/// Whether the row `row_key` (`data`) of `entity` belongs to the instance
/// keyed `key`: an instance row by its key or `id.instance`, any other row
/// by its `instance` (or the account its activity names).
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub fn row_of_instance(entity: &str, row_key: &str, data: &serde_json::Value, key: &str) -> bool {
    if entity == INSTANCE_ENTITY {
        return row_key == key
            || data
                .pointer("/id/instance")
                .or_else(|| data.pointer("/id/address"))
                .and_then(serde_json::Value::as_str)
                == Some(key);
    }
    activity_of(data, key)
}

/// Why a view of the protocol shows no rows.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub fn empty_reason(stack: &DevStack, entry: &ViewEntry) -> String {
    let before = stack
        .instance_counts()
        .map_or(0, |counts| counts.before_fork);
    match entry.entity.as_str() {
        PARTICIPANT_ENTITY if stack.has_participants() == Some(false) => "no participants".into(),
        INSTANCE_ENTITY | ACTIVITY_ENTITY if before > 0 => {
            format!("none on this fork ({before} predate it)")
        }
        PARTICIPANT_ENTITY => "none yet".into(),
        INSTANCE_ENTITY => "none yet: run a create step".into(),
        ACTIVITY_ENTITY => "no attested activity yet".into(),
        _ => "no rows yet".into(),
    }
}

/// The protocol's restart notice, when instances predate the fork.
pub fn fork_notice(listing: &DevListing, stack: &DevStack) -> Option<String> {
    let fork = listing.fork.as_ref()?;
    fork.notice(stack.instance_counts()?.before_fork)
}

/// `Instances 2 (closed 2) · Participants —` from the database's counts.
pub fn counts_summary(stack: &DevStack) -> Option<String> {
    let counts = stack.instance_counts()?;
    let mut instances = format!("Instances {}", counts.total);
    if counts.closed > 0 {
        instances.push_str(&format!(" (closed {})", counts.closed));
    }
    let participants = match (
        stack.has_participants(),
        stack.current.as_ref().and_then(|c| c.participants),
    ) {
        (Some(false), _) => "Participants \u{2014}".to_string(),
        (_, Some(count)) => format!("Participants {count}"),
        _ => return Some(instances),
    };
    Some(format!("{instances} \u{b7} {participants}"))
}

/// The listing as `a4 stream --dev` prints it without a view: each
/// protocol, its deployment, counts and views (friendly and raw names), then
/// the project's own stacks with theirs.
pub fn render_listing(listing: &DevListing, session: &DevSession) -> String {
    let stacks = &listing.stacks;
    let mut lines = Vec::new();
    if stacks.is_empty() {
        lines.push(super::dev::no_stacks(session));
        return lines.join("\n");
    }
    let (protocols, plain): (Vec<&DevStack>, Vec<&DevStack>) =
        stacks.iter().partition(|stack| stack.is_protocol());
    for stack in &protocols {
        let mut head = vec![stack.protocol_name().bold().to_string()];
        head.extend(stack.protocol_version().map(str::to_string));
        head.extend(stack.deployment_label());
        head.push(readiness(stack));
        lines.push(head.join("  "));
        let mut facts = vec![format!("stack {}", stack.name)];
        if let Some(generation) = stack.generation {
            facts.push(format!("gen {generation}"));
        }
        facts.push(stack.ws_url.clone());
        lines.push(format!("  {}", facts.join(" \u{b7} ").dimmed()));
        if let Some(summary) = counts_summary(stack) {
            lines.push(format!("  {summary}"));
        }
        if let Some(notice) = fork_notice(listing, stack) {
            lines.push(format!("  {}", format!("! {notice}").yellow()));
        }
        lines.extend(view_lines(stack));
    }
    for stack in &plain {
        let head = [stack.name.bold().to_string(), readiness(stack)];
        lines.push(head.join("  "));
        let mut facts = vec!["stack".to_string()];
        if let Some(generation) = stack.generation {
            facts.push(format!("gen {generation}"));
        }
        facts.push(stack.ws_url.clone());
        lines.push(format!("  {}", facts.join(" \u{b7} ").dimmed()));
        lines.extend(view_lines(stack));
    }
    lines.push(String::new());
    if let Some(example) = select_stack(stacks, None, session)
        .ok()
        .filter(|stack| stack.is_protocol())
        .or_else(|| protocols.first().copied())
    {
        let name = example.protocol_name();
        let view = primary_views(example)
            .into_iter()
            .find(|entry| entry.entity == ACTIVITY_ENTITY)
            .map(|entry| entry.friendly)
            .unwrap_or_else(|| "<View>".to_string());
        lines.push(format!(
            "Stream a protocol:  a4 stream --dev {name}            its views, one feed"
        ));
        lines.push(format!("One view:           a4 stream --dev {name} {view}"));
        lines.push(format!(
            "One instance:       a4 stream --dev {name} --instance <id>"
        ));
    }
    if let Some(example) = plain.first() {
        let view = primary_views(example)
            .into_iter()
            .next()
            .map(|entry| entry.friendly)
            .unwrap_or_else(|| "<View>".to_string());
        lines.push(format!(
            "Stream a stack:     a4 stream --dev {}            its views, one feed",
            example.name
        ));
        lines.push(format!(
            "One of its views:   a4 stream --dev {} {view}",
            example.name
        ));
    }
    lines.push("Browse them:        a4 stream --dev --tui".to_string());
    lines.join("\n")
}

fn readiness(stack: &DevStack) -> String {
    if stack.ready {
        "ready".green().to_string()
    } else {
        "not ready".yellow().to_string()
    }
}

/// A stack's views, one line each: friendly name, raw id, how to read it.
fn view_lines(stack: &DevStack) -> Vec<String> {
    let mut lines = Vec::new();
    let catalog = stack.catalog();
    if catalog.is_empty() {
        lines.push("  (no views)".dimmed().to_string());
    }
    let width = catalog
        .iter()
        .map(|entry| entry.friendly.len())
        .max()
        .unwrap_or(0);
    let raw_width = catalog
        .iter()
        .map(|entry| entry.raw.len())
        .max()
        .unwrap_or(0);
    for entry in &catalog {
        let note = match entry.role {
            ViewRole::Keyed if entry.entity == INSTANCE_ENTITY => "  --instance <id>",
            ViewRole::Keyed => "  --key <key>",
            ViewRole::PerInstance => "  per instance: --instance <id>",
            _ if entry.entity == PARTICIPANT_ENTITY && stack.has_participants() == Some(false) => {
                "  (no participant machine)"
            }
            _ => "",
        };
        let line = format!(
            "  {:width$}  {}{}",
            entry.friendly,
            format!("{:raw_width$}", entry.raw).dimmed(),
            note.dimmed(),
            width = width
        );
        lines.push(line.trim_end().to_string());
    }
    lines
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::commands::stream::dev::{DeploymentRef, ForkMarker, InstanceCounts};

    const TIP_JAR_VIEWS: [&str; 8] = [
        "ProtocolActivity/byInstance",
        "ProtocolActivity/list",
        "ProtocolActivity/state",
        "ProtocolInstance/byId",
        "ProtocolInstance/list",
        "ProtocolInstance/state",
        "ProtocolParticipant/byInstance",
        "ProtocolParticipant/list",
    ];

    pub(crate) fn protocol_stack(name: &str, protocol: &str, short_id: &str) -> DevStack {
        DevStack {
            name: name.to_string(),
            ws_url: "ws://127.0.0.1:49301".to_string(),
            ready: true,
            generation: Some(1),
            views: TIP_JAR_VIEWS.iter().map(|view| view.to_string()).collect(),
            deployment_id: Some(format!("{short_id}-uuid")),
            short_id: Some(short_id.to_string()),
            protocol: Some(name.to_string()),
            current: Some(DeploymentRef {
                deployment_id: format!("{short_id}-uuid"),
                status: "active".to_string(),
                short_id: Some(short_id.to_string()),
                protocol: Some(name.to_string()),
                protocol_name: Some(protocol.to_string()),
                protocol_version: Some("0.1.0".to_string()),
                has_participants: Some(false),
                instances: Some(InstanceCounts {
                    total: 2,
                    closed: 2,
                    before_fork: 2,
                }),
                participants: Some(0),
            }),
            deployments: Vec::new(),
        }
    }

    fn session() -> DevSession {
        let mut session = DevSession::offline(&arete_mcp::dev_env::EnvSelection::new(
            "denv_01hzx3k8m9q2r4s5t6v7w8x9yz",
            "app",
            "http://127.0.0.1:9",
        ));
        session.stack = Some("local-tip-jar".into());
        session
    }

    fn stacks() -> Vec<DevStack> {
        vec![
            protocol_stack("local-tip-jar", "localTipJar", "yurqmejlgs"),
            protocol_stack("token-raise", "tokenRaise", "raise12345"),
        ]
    }

    #[test]
    fn protocol_entities_get_friendly_names_and_roles() {
        let views: Vec<String> = TIP_JAR_VIEWS.iter().map(|v| v.to_string()).collect();
        let catalog = catalog(&views);
        let described: Vec<(&str, &str, ViewRole)> = catalog
            .iter()
            .map(|entry| (entry.friendly.as_str(), entry.raw.as_str(), entry.role))
            .collect();
        assert_eq!(
            described,
            [
                ("Instances", "ProtocolInstance/list", ViewRole::Primary),
                ("Activity", "ProtocolActivity/list", ViewRole::Primary),
                (
                    "Participants",
                    "ProtocolParticipant/list",
                    ViewRole::Primary
                ),
                (
                    "Instances/byId",
                    "ProtocolInstance/byId",
                    ViewRole::Alternate
                ),
                ("Instances/state", "ProtocolInstance/state", ViewRole::Keyed),
                (
                    "Activity/byInstance",
                    "ProtocolActivity/byInstance",
                    ViewRole::PerInstance
                ),
                ("Activity/state", "ProtocolActivity/state", ViewRole::Keyed),
                (
                    "Participants/byInstance",
                    "ProtocolParticipant/byInstance",
                    ViewRole::PerInstance
                ),
            ]
        );
        // Without a list view the first collection is the primary one, and
        // other entities keep their names.
        let catalog = super::catalog(&[
            "ProtocolInstance/byId".to_string(),
            "ProtocolInstance/state".to_string(),
            "TipJar/latest".to_string(),
        ]);
        assert_eq!(catalog[0].friendly, "Instances");
        assert_eq!(catalog[0].raw, "ProtocolInstance/byId");
        assert_eq!(catalog[1].friendly, "TipJar");
        assert_eq!(catalog[1].raw, "TipJar/latest");
    }

    #[test]
    fn views_resolve_by_friendly_entity_or_raw_name() {
        let views: Vec<String> = TIP_JAR_VIEWS.iter().map(|v| v.to_string()).collect();
        let catalog = catalog(&views);
        let raw = |input: &str| resolve_view(&catalog, input).map(|entry| entry.raw.clone());
        assert_eq!(raw("Activity").unwrap(), "ProtocolActivity/list");
        assert_eq!(raw("activity").unwrap(), "ProtocolActivity/list");
        assert_eq!(raw("Instances").unwrap(), "ProtocolInstance/list");
        assert_eq!(raw("instance").unwrap(), "ProtocolInstance/list");
        assert_eq!(raw("Participants").unwrap(), "ProtocolParticipant/list");
        assert_eq!(raw("ProtocolActivity").unwrap(), "ProtocolActivity/list");
        assert_eq!(
            raw("ProtocolActivity/list").unwrap(),
            "ProtocolActivity/list"
        );
        assert_eq!(
            raw("protocolactivity/byinstance").unwrap(),
            "ProtocolActivity/byInstance"
        );
        assert_eq!(
            raw("Activity/byInstance").unwrap(),
            "ProtocolActivity/byInstance"
        );
        assert_eq!(raw("Instance/state").unwrap(), "ProtocolInstance/state");
        let error = raw("Rounds").unwrap_err().to_string();
        assert!(error.contains("No view named 'Rounds'"), "{error}");
        assert!(error.contains("Activity"), "{error}");
        assert!(error.contains("ProtocolActivity/list"), "{error}");
    }

    #[test]
    fn protocols_resolve_by_spec_name_slug_stack_or_short_id() {
        let stacks = stacks();
        for input in ["localTipJar", "local-tip-jar", "LOCALTIPJAR", "yurqmejlgs"] {
            assert_eq!(
                resolve_protocol(&stacks, input).unwrap().name,
                "local-tip-jar"
            );
        }
        assert_eq!(
            resolve_protocol(&stacks, "tokenRaise").unwrap().name,
            "token-raise"
        );
        let error = resolve_protocol(&stacks, "escrow").unwrap_err().to_string();
        assert!(
            error.contains("No protocol or stack named 'escrow'"),
            "{error}"
        );
        assert!(error.contains("localTipJar, tokenRaise"), "{error}");
    }

    #[test]
    fn instances_are_keyed_by_the_short_id_of_their_uuid() {
        let uuid = "8c2b1c56-3f7e-4f0a-9d7e-2b1f0c6a5e11";
        let key = InstanceKey::parse(uuid).unwrap();
        let bytes = uuid::Uuid::parse_str(uuid).unwrap();
        assert_eq!(key.key, bs58::encode(bytes.as_bytes()).into_string());
        assert_eq!(key.uuid.as_deref(), Some(uuid));
        // The short id is taken as is, and names the same instance.
        let short = InstanceKey::parse(&key.key).unwrap();
        assert_eq!(short, key);
        assert!(key.describe().ends_with(&format!("(instance {uuid})")));
        // Anything else is a key of its own (an account address).
        let account = InstanceKey::parse("11111111111111111111111111111111").unwrap();
        assert_eq!(account.key, "11111111111111111111111111111111");
        assert_eq!(account.uuid, None);
        assert!(InstanceKey::parse(" ").is_err());
    }

    #[test]
    fn activity_rows_belong_to_their_instance() {
        let row = serde_json::json!({ "instance": "abc", "step": "open" });
        assert!(activity_of(&row, "abc"));
        assert!(!activity_of(&row, "abd"));
        assert!(activity_of(
            &serde_json::json!({ "activity_account": "acct" }),
            "acct"
        ));
    }

    #[test]
    fn an_instance_shows_its_per_instance_collections() {
        let mut stack = protocol_stack("local-tip-jar", "localTipJar", "dev1");
        let raw = |stack: &DevStack| {
            instance_collections(stack)
                .into_iter()
                .map(|entry| entry.raw)
                .collect::<Vec<_>>()
        };
        // The tip jar declares no participant machine.
        assert_eq!(
            raw(&stack),
            ["ProtocolActivity/byInstance", "ProtocolInstance/byId"]
        );
        stack.current.as_mut().unwrap().has_participants = Some(true);
        assert_eq!(
            raw(&stack),
            [
                "ProtocolActivity/byInstance",
                "ProtocolParticipant/byInstance",
                "ProtocolInstance/byId"
            ]
        );
        stack.views = vec![
            "ProtocolActivity/list".into(),
            "ProtocolInstance/state".into(),
        ];
        assert_eq!(raw(&stack), ["ProtocolActivity/list"]);

        let row = serde_json::json!({ "id": { "instance": "k" } });
        assert!(row_of_instance("ProtocolInstance", "k", &row, "k"));
        assert!(row_of_instance("ProtocolInstance", "other-key", &row, "k"));
        assert!(!row_of_instance(
            "ProtocolInstance",
            "x",
            &serde_json::json!({}),
            "k"
        ));
        assert!(row_of_instance(
            "ProtocolParticipant",
            "p",
            &serde_json::json!({ "instance": "k" }),
            "k"
        ));
    }

    #[test]
    fn protocol_rows_carry_the_instance_id_next_to_the_short_id() {
        let uuid = "8c2b1c56-3f7e-4f0a-9d7e-2b1f0c6a5e11";
        let short = bs58::encode(uuid::Uuid::parse_str(uuid).unwrap().as_bytes()).into_string();
        assert_eq!(instance_uuid(&short).as_deref(), Some(uuid));

        let mut activity = serde_json::json!({ "instance": short, "step": "open" });
        add_instance_id("ProtocolActivity/list", &mut activity);
        assert_eq!(activity["instance_id"], uuid);

        let mut participant = serde_json::json!({ "instance": short });
        add_instance_id("ProtocolParticipant/byInstance", &mut participant);
        assert_eq!(participant["instance_id"], uuid);

        let mut instance = serde_json::json!({ "id": { "instance": short } });
        add_instance_id("ProtocolInstance/state", &mut instance);
        assert_eq!(instance["id"]["instance_id"], uuid);

        // An account (interim profile), a patch without the id and other
        // entities stay as they are.
        for (view, row) in [
            (
                "ProtocolActivity/list",
                serde_json::json!({ "instance": "11111111111111111111111111111111" }),
            ),
            (
                "ProtocolInstance/list",
                serde_json::json!({ "activity": {} }),
            ),
            ("Other/list", serde_json::json!({ "instance": short })),
        ] {
            let mut changed = row.clone();
            add_instance_id(view, &mut changed);
            assert_eq!(changed, row, "{view}");
        }
    }

    fn request(first: Option<&str>, second: Option<&str>) -> DevRequest {
        DevRequest {
            first: first.map(str::to_string),
            second: second.map(str::to_string),
            ..DevRequest::default()
        }
    }

    #[test]
    fn positionals_name_a_protocol_then_a_view() {
        let stacks = stacks();
        let session = session();
        let target = |request: DevRequest| resolve_target(&request, &stacks, &session);

        assert_eq!(target(request(None, None)).unwrap(), DevTarget::Overview);
        assert_eq!(
            target(request(Some("tokenRaise"), None)).unwrap(),
            DevTarget::Protocol {
                stack: "token-raise".to_string(),
                pinned: true
            }
        );
        assert_eq!(
            target(request(Some("tokenRaise"), Some("Activity"))).unwrap(),
            DevTarget::View {
                stack: "token-raise".to_string(),
                view: "ProtocolActivity/list".to_string(),
                pinned: true
            }
        );
        // A view alone is the default protocol's, as before.
        assert_eq!(
            target(request(Some("ProtocolActivity/list"), None)).unwrap(),
            DevTarget::View {
                stack: "local-tip-jar".to_string(),
                view: "ProtocolActivity/list".to_string(),
                pinned: false
            }
        );
        assert_eq!(
            target(request(Some("Activity"), None)).unwrap(),
            DevTarget::View {
                stack: "local-tip-jar".to_string(),
                view: "ProtocolActivity/list".to_string(),
                pinned: false
            }
        );
        let error = target(request(Some("nope"), None)).unwrap_err().to_string();
        assert!(
            error.contains("No protocol, stack or view named 'nope'"),
            "{error}"
        );
        let error = target(request(Some("nope"), Some("Activity")))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("No protocol or stack named 'nope'"),
            "{error}"
        );
        let error = target(request(Some("tokenRaise"), Some("Rounds")))
            .unwrap_err()
            .to_string();
        assert!(error.contains("No view named 'Rounds'"), "{error}");
    }

    #[test]
    fn instance_all_and_stack_requests() {
        let stacks = stacks();
        let session = session();
        let target = |request: DevRequest| resolve_target(&request, &stacks, &session);
        let uuid = "8c2b1c56-3f7e-4f0a-9d7e-2b1f0c6a5e11";

        let instance = target(DevRequest {
            instance: Some(uuid.to_string()),
            ..request(Some("tokenRaise"), None)
        })
        .unwrap();
        assert!(matches!(
            &instance,
            DevTarget::Instance { stack, key, pinned: true }
                if stack == "token-raise" && key.uuid.as_deref() == Some(uuid)
        ));
        // Without a protocol, the default one's.
        let instance = target(DevRequest {
            instance: Some(uuid.to_string()),
            ..DevRequest::default()
        })
        .unwrap();
        assert_eq!(instance.stack(), Some("local-tip-jar"));
        assert!(!instance.pinned());

        assert_eq!(
            target(DevRequest {
                all: true,
                ..DevRequest::default()
            })
            .unwrap(),
            DevTarget::All {
                stack: "local-tip-jar".to_string(),
                pinned: false
            }
        );
        assert!(target(DevRequest {
            all: true,
            instance: Some(uuid.to_string()),
            ..DevRequest::default()
        })
        .is_err());
        assert!(target(DevRequest {
            all: true,
            ..request(Some("tokenRaise"), Some("Activity"))
        })
        .is_err());

        // --stack is the low-level form: the positional is its view.
        assert_eq!(
            target(DevRequest {
                stack: Some("token-raise".to_string()),
                ..request(Some("ProtocolActivity/byInstance"), None)
            })
            .unwrap(),
            DevTarget::View {
                stack: "token-raise".to_string(),
                view: "ProtocolActivity/byInstance".to_string(),
                pinned: true
            }
        );
        assert!(target(DevRequest {
            stack: Some("token-raise".to_string()),
            ..request(Some("a"), Some("b"))
        })
        .is_err());
    }

    #[test]
    fn the_listing_names_protocols_their_views_and_the_restart() {
        colored::control::set_override(false);
        let listing = DevListing {
            stacks: vec![protocol_stack("local-tip-jar", "localTipJar", "yurqmejlgs")],
            fork: Some(ForkMarker {
                started_at: Some("2026-10-14T14:02:00Z".to_string()),
                start_slot: Some(1),
                instances_before: 2,
            }),
        };
        let text = render_listing(&listing, &session());
        assert!(
            text.starts_with("localTipJar  0.1.0  deployment yurqmejlgs (active)  ready"),
            "{text}"
        );
        assert!(
            text.contains("Instances 2 (closed 2) \u{b7} Participants \u{2014}"),
            "{text}"
        );
        assert!(
            text.contains("! 2 instances predate this fork (restarted "),
            "{text}"
        );
        assert!(
            text.contains("their chain history isn't on this fork"),
            "{text}"
        );
        assert!(
            text.contains("  Activity                 ProtocolActivity/list\n"),
            "{text}"
        );
        assert!(
            text.contains(
                "Participants             ProtocolParticipant/list        (no participant machine)"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "Instances/state          ProtocolInstance/state          --instance <id>"
            ),
            "{text}"
        );
        assert!(
            text.contains("a4 stream --dev localTipJar Activity"),
            "{text}"
        );
        assert!(
            text.contains("a4 stream --dev localTipJar --instance <id>"),
            "{text}"
        );
    }

    #[test]
    fn plain_stacks_are_listed_by_name_after_the_protocols() {
        colored::control::set_override(false);
        let plain = DevStack {
            name: "transfers".into(),
            ws_url: "ws://127.0.0.1:7".into(),
            ready: true,
            generation: Some(3),
            views: vec![
                "Transfer/list".into(),
                "Transfer/state".into(),
                "Transfer/largest".into(),
            ],
            deployment_id: None,
            short_id: None,
            protocol: None,
            current: None,
            deployments: Vec::new(),
        };
        let listing = DevListing {
            stacks: vec![
                protocol_stack("local-tip-jar", "localTipJar", "yurqmejlgs"),
                plain.clone(),
            ],
            fork: None,
        };
        let session = session();
        let text = render_listing(&listing, &session);
        let transfers = text
            .find("transfers  ready")
            .unwrap_or_else(|| panic!("{text}"));
        assert!(text.find("localTipJar").unwrap() < transfers, "{text}");
        assert!(
            text.contains("  stack \u{b7} gen 3 \u{b7} ws://127.0.0.1:7"),
            "{text}"
        );
        assert!(
            text.contains("  Transfer          Transfer/list\n"),
            "{text}"
        );
        assert!(
            text.contains("Transfer/state    Transfer/state    --key <key>"),
            "{text}"
        );
        assert!(
            text.contains("a4 stream --dev localTipJar Activity"),
            "{text}"
        );
        assert!(
            text.contains("a4 stream --dev transfers Transfer"),
            "{text}"
        );

        // Plain stacks resolve by name; a view of theirs by entity.
        let stacks = listing.stacks.clone();
        let target = resolve_target(
            &request(Some("transfers"), Some("largest")),
            &stacks,
            &session,
        )
        .unwrap();
        assert_eq!(
            target,
            DevTarget::View {
                stack: "transfers".into(),
                view: "Transfer/largest".into(),
                pinned: true
            }
        );
        let error = resolve_target(&request(Some("nope"), Some("x")), &stacks, &session)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Protocols: localTipJar\nStacks: transfers"),
            "{error}"
        );

        // Only plain stacks: no protocol examples.
        let only = DevListing {
            stacks: vec![plain],
            fork: None,
        };
        let text = render_listing(&only, &session);
        assert!(!text.contains("Stream a protocol"), "{text}");
        assert!(
            text.contains("Stream a stack:     a4 stream --dev transfers"),
            "{text}"
        );
    }

    #[test]
    fn the_fork_notice_counts_instances() {
        let fork = ForkMarker {
            started_at: Some("2026-10-14T14:02:00Z".to_string()),
            start_slot: None,
            instances_before: 1,
        };
        let one = fork.notice(1).unwrap();
        assert!(
            one.starts_with("1 instance predates this fork (restarted "),
            "{one}"
        );
        assert!(
            one.ends_with("; its chain history isn't on this fork"),
            "{one}"
        );
        assert_eq!(fork.notice(0), None);
        let unknown = ForkMarker::default();
        assert_eq!(
            unknown.notice(3).unwrap(),
            "3 instances predate this fork; their chain history isn't on this fork"
        );
    }

    #[test]
    fn empty_views_say_why() {
        let stack = protocol_stack("local-tip-jar", "localTipJar", "yurqmejlgs");
        let catalog = stack.catalog();
        let reason = |raw: &str| {
            empty_reason(
                &stack,
                catalog.iter().find(|entry| entry.raw == raw).unwrap(),
            )
        };
        assert_eq!(reason("ProtocolParticipant/list"), "no participants");
        // Instances from an earlier fork: none of their rows are on this one.
        assert_eq!(
            reason("ProtocolInstance/list"),
            "none on this fork (2 predate it)"
        );
        let mut fresh = stack.clone();
        if let Some(current) = fresh.current.as_mut() {
            current.instances = Some(InstanceCounts::default());
        }
        let instances = &catalog[0];
        assert_eq!(
            empty_reason(&fresh, instances),
            "none yet: run a create step"
        );
        let (state, activity) = instance_views(&stack);
        assert_eq!(state, "ProtocolInstance/state");
        assert_eq!(activity.as_deref(), Some("ProtocolActivity/byInstance"));
    }
}
