//! Sorted view cache for maintaining ordered entity collections.
//!
//! This module provides incremental maintenance of sorted entity views,
//! enabling efficient windowed subscriptions (take/skip) with minimal
//! recomputation on updates.

use crate::shared_entity::{lookup, EntityFields, Fields, SharedEntity};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// A sortable key that combines the sort value with entity key for stable ordering.
/// Uses (sort_value, entity_key) tuple to ensure deterministic ordering even when
/// sort values are equal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    /// The extracted sort value (as comparable bytes)
    sort_value: SortValue,
    /// Entity key for tie-breaking
    entity_key: String,
    /// Direction to apply to the sort value comparison.
    order: SortOrder,
    /// A string sort value read as a decimal integer, once, rather than on
    /// every comparison. Derived from `sort_value`, so it changes nothing
    /// about equality.
    decimal: Option<DecimalKey>,
}

impl SortKey {
    fn new(sort_value: SortValue, entity_key: String, order: SortOrder) -> Self {
        Self {
            decimal: DecimalKey::of(&sort_value),
            sort_value,
            entity_key,
            order,
        }
    }

    /// Give the key another sort value.
    fn set_sort_value(&mut self, sort_value: SortValue) {
        self.decimal = DecimalKey::of(&sort_value);
        self.sort_value = sort_value;
    }

    fn ranked(&self) -> Ranked<'_> {
        Ranked {
            sort_value: &self.sort_value,
            decimal: self.decimal,
            entity_key: &self.entity_key,
            order: self.order,
        }
    }
}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ranked().cmp(&other.ranked())
    }
}

/// A [`SortKey`], borrowed: what ordering one reads, so a candidate can be
/// compared without building (and allocating) a key of its own.
struct Ranked<'a> {
    sort_value: &'a SortValue,
    decimal: Option<DecimalKey>,
    entity_key: &'a str,
    order: SortOrder,
}

impl Ranked<'_> {
    /// [`SortValue::cmp`], using the pre-parsed decimals.
    fn cmp_values(&self, other: &Self) -> Ordering {
        match (self.sort_value, other.sort_value) {
            (SortValue::String(left), SortValue::String(right)) => {
                match (&self.decimal, &other.decimal) {
                    (Some(a), Some(b)) => a.cmp(left, b, right),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => left.cmp(right),
                }
            }
            (left, right) => left.cmp(right),
        }
    }

    fn cmp(&self, other: &Self) -> Ordering {
        if self.order != other.order {
            return match (self.order, other.order) {
                (SortOrder::Asc, SortOrder::Desc) => Ordering::Less,
                (SortOrder::Desc, SortOrder::Asc) => Ordering::Greater,
                _ => Ordering::Equal,
            };
        }

        // An entity with no sort value (missing or `null`) cannot be ranked,
        // so it sorts after every entity that has one in *both* directions:
        // it must never take a window slot from a ranked entity. `desc`
        // reverses only the comparison between two present values.
        let sort_order = match (self.sort_value, other.sort_value, self.order) {
            (SortValue::Null, SortValue::Null, _) => Ordering::Equal,
            (SortValue::Null, _, _) => Ordering::Greater,
            (_, SortValue::Null, _) => Ordering::Less,
            (_, _, SortOrder::Asc) => self.cmp_values(other),
            (_, _, SortOrder::Desc) => self.cmp_values(other).reverse(),
        };

        match sort_order {
            Ordering::Equal => self.entity_key.cmp(other.entity_key),
            other => other,
        }
    }
}

/// Comparable sort value extracted from JSON
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SortValue {
    Null,
    Bool(bool),
    Integer(i64),
    Float(OrderedFloat),
    String(String),
}

impl Ord for SortValue {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (SortValue::Null, SortValue::Null) => Ordering::Equal,
            (SortValue::Null, _) => Ordering::Less,
            (_, SortValue::Null) => Ordering::Greater,
            (SortValue::Bool(a), SortValue::Bool(b)) => a.cmp(b),
            (SortValue::Integer(a), SortValue::Integer(b)) => a.cmp(b),
            (SortValue::Float(a), SortValue::Float(b)) => a.cmp(b),
            (SortValue::String(a), SortValue::String(b)) => compare_strings(a, b),
            // Cross-type comparisons: numbers < strings
            (SortValue::Integer(_), SortValue::String(_)) => Ordering::Less,
            (SortValue::String(_), SortValue::Integer(_)) => Ordering::Greater,
            (SortValue::Float(_), SortValue::String(_)) => Ordering::Less,
            (SortValue::String(_), SortValue::Float(_)) => Ordering::Greater,
            // Integer vs Float: convert to float
            (SortValue::Integer(a), SortValue::Float(b)) => OrderedFloat(*a as f64).cmp(b),
            (SortValue::Float(a), SortValue::Integer(b)) => a.cmp(&OrderedFloat(*b as f64)),
            // Bool vs others
            (SortValue::Bool(_), _) => Ordering::Less,
            (_, SortValue::Bool(_)) => Ordering::Greater,
        }
    }
}

impl PartialOrd for SortValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Wrapper for f64 that implements Ord (treats NaN as less than all values)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrderedFloat(pub f64);

impl Eq for OrderedFloat {}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.partial_cmp(&other.0).unwrap_or_else(|| {
            if self.0.is_nan() && other.0.is_nan() {
                Ordering::Equal
            } else if self.0.is_nan() {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        })
    }
}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Sort order for the cache
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Asc,
    Desc,
}

impl From<crate::materialized_view::SortOrder> for SortOrder {
    fn from(order: crate::materialized_view::SortOrder) -> Self {
        match order {
            crate::materialized_view::SortOrder::Asc => SortOrder::Asc,
            crate::materialized_view::SortOrder::Desc => SortOrder::Desc,
        }
    }
}

/// Delta representing a change to a client's windowed view
#[derive(Debug, Clone, PartialEq)]
pub enum ViewDelta {
    /// No change to the client's window
    None,
    /// Entity was added to the window
    Add { key: String, entity: Value },
    /// Entity was removed from the window
    Remove { key: String },
    /// Entity in the window was updated
    Update { key: String, entity: Value },
}

/// Sorted view cache maintaining entities in sort order.
///
/// # Bounding
///
/// The cache holds every entity it has been given. It shares an entity's
/// fields with whoever gave it (see [`SharedEntity`]) rather than copying
/// them, but it keeps them alive, so callers that feed it from a bounded
/// source (the projector and snapshot restore feed it from the LRU-capped
/// [`EntityCache`](crate::EntityCache)) should use
/// [`upsert_bounded`](Self::upsert_bounded) or
/// [`trim_to_max_entries`](Self::trim_to_max_entries) with that source's cap.
///
/// Entries are evicted from the *bottom* of the sort order, not by recency.
/// Evicting the least-recently-updated entries (mirroring the entity cache)
/// would drop a top-ranked entity that simply has not updated recently and
/// break leaderboard-style `sort` + `take` views. Evicting everything beyond
/// position `max_entries` keeps every window with `skip + take <= max_entries`
/// exact. An evicted entity re-enters on its next update, because the
/// projector re-reads the full entity from the entity cache before upserting.
///
/// Known edge: after entities are removed from (or move down out of) the top
/// of the order, a previously evicted entity that has not updated since is
/// missing from the cache until it next updates, so a window near the cap can
/// under-fill or show a lower-ranked entity in its place until then.
#[derive(Debug)]
pub struct SortedViewCache {
    /// View identifier
    view_id: String,
    /// Field path to sort by (e.g., ["id", "round_id"])
    sort_field: Vec<String>,
    /// Sort order
    order: SortOrder,
    /// Sorted entries: SortKey -> entity_key (for iteration in order)
    sorted: BTreeMap<SortKey, ()>,
    /// Entity data: entity_key -> (SortKey, entity)
    entities: HashMap<String, (SortKey, SharedEntity)>,
    /// Ordered keys cache (rebuilt on structural changes)
    keys_cache: Vec<String>,
    /// Whether keys_cache needs rebuild
    cache_dirty: bool,
    /// Stands in for an entity while it is merged in place.
    placeholder: SharedEntity,
}

impl SortedViewCache {
    pub fn new(view_id: String, sort_field: Vec<String>, order: SortOrder) -> Self {
        Self {
            view_id,
            sort_field,
            order,
            sorted: BTreeMap::new(),
            entities: HashMap::new(),
            keys_cache: Vec::new(),
            cache_dirty: true,
            placeholder: SharedEntity::new(Value::Null),
        }
    }

    pub fn view_id(&self) -> &str {
        &self.view_id
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Insert or update an entity, returns the position where it was inserted
    ///
    /// An update merges into the entity held, keeping fields the update lacks.
    /// When it lacks none, which is the case for a whole entity, the cache
    /// shares the update's fields instead of copying them.
    ///
    /// Finding the position walks the order up to it; [`Self::put`] does the
    /// same upsert without it.
    pub fn upsert(&mut self, entity_key: String, entity: impl Into<SharedEntity>) -> UpsertResult {
        let entity = entity.into();
        let sort_value = self.extract_sort_value(&entity);
        let moved = self.put_with_sort_value(Cow::Borrowed(&entity_key), entity, sort_value);
        let position = self.find_position(&entity_key);
        match moved {
            Placed::Unmoved => UpsertResult::Updated { position },
            Placed::Moved => UpsertResult::Inserted { position },
        }
    }

    /// [`Self::upsert`] without reporting where the entity landed.
    pub fn put(&mut self, entity_key: String, entity: impl Into<SharedEntity>) {
        let entity = entity.into();
        let sort_value = self.extract_sort_value(&entity);
        self.put_with_sort_value(Cow::Owned(entity_key), entity, sort_value);
    }

    /// [`Self::upsert`] given the entity's sort value.
    fn put_with_sort_value(
        &mut self,
        entity_key: Cow<'_, str>,
        entity: SharedEntity,
        sort_value: SortValue,
    ) -> Placed {
        if let Some((sort_key, held)) = self.entities.get_mut(entity_key.as_ref()) {
            // Merge incoming entity with existing to preserve fields not in the update
            let old_entity = std::mem::replace(held, self.placeholder.clone());
            *held = Self::merge_entity(old_entity, entity);

            // A missing sort value keeps the one held.
            let keeps_old_value = matches!(sort_value, SortValue::Null)
                && !matches!(sort_key.sort_value, SortValue::Null);
            if keeps_old_value || sort_key.sort_value == sort_value {
                // Sort key unchanged - position unchanged, no structural change
                return Placed::Unmoved;
            }

            // Sort key changed - need to reposition. The order's own copy of
            // the key moves, so the entity key is not copied again.
            let mut ordered = match self.sorted.remove_entry(sort_key) {
                Some((ordered, ())) => ordered,
                // Only an order that is not total can lose a held key. Drop
                // whatever copy it holds rather than stop the view.
                None => {
                    let stale = sort_key.entity_key.as_str();
                    self.sorted.retain(|held, ()| held.entity_key != stale);
                    sort_key.clone()
                }
            };
            ordered.set_sort_value(sort_value);
            sort_key.sort_value = ordered.sort_value.clone();
            sort_key.decimal = ordered.decimal;
            self.sorted.insert(ordered, ());
            self.cache_dirty = true;
            return Placed::Moved;
        }

        let entity_key = entity_key.into_owned();
        let new_sort_key = SortKey::new(sort_value, entity_key.clone(), self.order);
        self.sorted.insert(new_sort_key.clone(), ());
        self.entities.insert(entity_key, (new_sort_key, entity));
        self.cache_dirty = true;
        Placed::Moved
    }

    /// Whether upserting `entity` under `entity_key` into a cache bounded at
    /// `max_entries` would keep it: the cache already holds the key, has
    /// room, or the entity sorts before its current last entry.
    ///
    /// Lets a caller skip copying an entity that [`Self::upsert_bounded`]
    /// would evict straight away, which in a busy view is most of them.
    pub fn would_keep<E: EntityFields + ?Sized>(
        &self,
        entity_key: &str,
        entity: &E,
        max_entries: usize,
    ) -> bool {
        self.keeps(entity_key, || self.extract_sort_value(entity), max_entries)
            .0
    }

    /// [`Self::would_keep`], handing back the sort value it read, if it read
    /// one.
    fn keeps(
        &self,
        entity_key: &str,
        sort_value: impl FnOnce() -> SortValue,
        max_entries: usize,
    ) -> (bool, Option<SortValue>) {
        if self.entities.contains_key(entity_key) || self.sorted.len() < max_entries {
            return (true, None);
        }
        let Some((last, ())) = self.sorted.last_key_value() else {
            return (true, None);
        };
        let sort_value = sort_value();
        let candidate = Ranked {
            sort_value: &sort_value,
            decimal: DecimalKey::of(&sort_value),
            entity_key,
            order: self.order,
        };
        let kept = candidate.cmp(&last.ranked()).is_lt();
        (kept, Some(sort_value))
    }

    /// Upsert an entity, then evict from the bottom of the sort order so the
    /// cache holds at most `max_entries` entities.
    ///
    /// If the upserted entity itself sorts beyond `max_entries` it is evicted
    /// immediately and the returned position is `>= max_entries`.
    pub fn upsert_bounded(
        &mut self,
        entity_key: String,
        entity: impl Into<SharedEntity>,
        max_entries: usize,
    ) -> UpsertResult {
        let result = self.upsert(entity_key, entity);
        self.trim_to_max_entries(max_entries);
        result
    }

    /// [`Self::upsert_bounded`] for an entity [`Self::would_keep`] keeps,
    /// without reporting its position: the cache ends up as it would after
    /// `would_keep` and `upsert_bounded`, reading the entity's sort value
    /// once and sharing (cloning) the entity only if the cache keeps it.
    /// Returns whether it does.
    pub fn upsert_if_kept(
        &mut self,
        entity_key: &str,
        entity: &SharedEntity,
        max_entries: usize,
    ) -> bool {
        let (kept, sort_value) =
            self.keeps(entity_key, || self.extract_sort_value(entity), max_entries);
        if !kept {
            return false;
        }
        let sort_value = sort_value.unwrap_or_else(|| self.extract_sort_value(entity));
        self.put_with_sort_value(Cow::Borrowed(entity_key), entity.clone(), sort_value);
        self.trim_to_max_entries(max_entries);
        true
    }

    /// Evict entities from the bottom of the sort order until at most
    /// `max_entries` remain. Returns the number of evicted entities.
    ///
    /// Each eviction is an `O(log n)` pop from the end of the ordered index,
    /// so a batch of evictions (e.g. after a bulk rebuild) costs no more than
    /// the inserts that caused it. The ordered-keys cache is truncated in place
    /// when it is current, so trimming does not force a full rebuild.
    pub fn trim_to_max_entries(&mut self, max_entries: usize) -> usize {
        let mut evicted = 0;
        while self.sorted.len() > max_entries {
            let Some((sort_key, ())) = self.sorted.pop_last() else {
                break;
            };
            self.entities.remove(&sort_key.entity_key);
            evicted += 1;
        }
        if evicted > 0 && !self.cache_dirty {
            // Only the tail of the order was removed, so the prefix is still
            // exact.
            self.keys_cache.truncate(self.sorted.len());
        }
        evicted
    }

    /// [`Self::deep_merge`] for whole entities, `_version` included. The
    /// result shares `patch`'s fields whenever merging adds nothing to them:
    /// when `base` has no field, at any depth, that `patch` lacks.
    fn merge_entity(base: SharedEntity, patch: SharedEntity) -> SharedEntity {
        let (base_fields, base_version) = base.into_parts();
        let (patch_fields, patch_version) = patch.into_parts();
        let (Fields::Object(base_members), Fields::Object(patch_members)) =
            (&*base_fields, &*patch_fields)
        else {
            return SharedEntity::from_parts(patch_fields, patch_version);
        };
        // A top-level field both share adds nothing, so only the fields the
        // update replaced are looked into.
        let keeps = !Arc::ptr_eq(&base_fields, &patch_fields)
            && base_members.iter().any(|(key, base)| {
                patch_members
                    .get(key)
                    .is_none_or(|patch| !Arc::ptr_eq(base, patch) && keeps_fields(base, patch))
            });
        let fields = if keeps {
            let base = match Arc::try_unwrap(base_fields) {
                Ok(fields) => fields.into_value(),
                Err(shared) => shared.to_value(),
            };
            Arc::new(Fields::from_value(Self::deep_merge(
                base,
                patch_fields.to_value(),
            )))
        } else {
            patch_fields
        };
        let version = match (base_version, patch_version) {
            (Some(base), Some(patch)) => Some(Self::deep_merge(base, patch)),
            (base, None) => base,
            (None, patch) => patch,
        };
        SharedEntity::from_parts(fields, version)
    }

    fn deep_merge(base: Value, patch: Value) -> Value {
        match (base, patch) {
            (Value::Object(mut base_map), Value::Object(patch_map)) => {
                for (key, patch_value) in patch_map {
                    if let Some(base_value) = base_map.remove(&key) {
                        base_map.insert(key, Self::deep_merge(base_value, patch_value));
                    } else {
                        base_map.insert(key, patch_value);
                    }
                }
                Value::Object(base_map)
            }
            (_, patch) => patch,
        }
    }

    /// Remove an entity, returns the position it was at
    pub fn clear(&mut self) {
        self.sorted.clear();
        self.entities.clear();
        self.cache_dirty = true;
    }

    ///
    /// Finding the position walks the order up to it; [`Self::remove_key`]
    /// removes without it.
    pub fn remove(&mut self, entity_key: &str) -> Option<usize> {
        let (sort_key, _) = self.entities.get(entity_key)?;
        let position = self.find_position_by_sort_key(sort_key);
        self.remove_key(entity_key);
        Some(position)
    }

    /// Remove an entity, returning whether the cache held it.
    pub fn remove_key(&mut self, entity_key: &str) -> bool {
        let Some((sort_key, _)) = self.entities.remove(entity_key) else {
            return false;
        };
        self.sorted.remove(&sort_key);
        self.cache_dirty = true;
        true
    }

    /// Get entity by key
    pub fn get(&self, entity_key: &str) -> Option<&SharedEntity> {
        self.entities.get(entity_key).map(|(_, v)| v)
    }

    /// Get ordered keys (rebuilds cache if dirty)
    ///
    /// The other readers walk the order itself, so only this one pays for
    /// copying every key after a change.
    pub fn ordered_keys(&mut self) -> &[String] {
        if self.cache_dirty {
            self.rebuild_keys_cache();
        }
        &self.keys_cache
    }

    /// Get a window of entities, as copies.
    pub fn get_window(&mut self, skip: usize, take: usize) -> Vec<(String, Value)> {
        self.iter_ordered()
            .skip(skip)
            .take(take)
            .map(|(key, entity)| (key.to_string(), entity.to_value()))
            .collect()
    }

    /// Every entity in sort order, borrowed: a window read walks only as far
    /// as it reads, `skip` included.
    pub fn iter_ordered(&self) -> impl Iterator<Item = (&str, &SharedEntity)> + '_ {
        self.sorted.keys().filter_map(|sort_key| {
            self.entities
                .get(&sort_key.entity_key)
                .map(|(_, entity)| (sort_key.entity_key.as_str(), entity))
        })
    }

    /// Copies of every entity in deterministic sort order.
    pub fn get_all_ordered(&mut self) -> Vec<(String, Value)> {
        self.ordered_entities()
            .into_iter()
            .map(|(key, entity)| (key, entity.into_value()))
            .collect()
    }

    /// Every entity in deterministic sort order for query-side filtering,
    /// sharing their fields with the cache rather than copying them.
    pub fn ordered_entities(&mut self) -> Vec<(String, SharedEntity)> {
        self.iter_ordered()
            .map(|(key, entity)| (key.to_string(), entity.clone()))
            .collect()
    }

    /// Compute deltas for a client with a specific window
    pub fn compute_window_deltas(
        &mut self,
        old_window_keys: &[String],
        skip: usize,
        take: usize,
    ) -> Vec<ViewDelta> {
        let new_window_keys: Vec<&String> = self
            .sorted
            .keys()
            .map(|sort_key| &sort_key.entity_key)
            .skip(skip)
            .take(take)
            .collect();

        let old_set: std::collections::HashSet<&String> = old_window_keys.iter().collect();
        let new_set: std::collections::HashSet<&String> = new_window_keys.iter().cloned().collect();

        let mut deltas = Vec::new();

        // Removed from window
        for key in old_set.difference(&new_set) {
            deltas.push(ViewDelta::Remove {
                key: (*key).clone(),
            });
        }

        // Added to window
        for key in new_set.difference(&old_set) {
            if let Some((_, entity)) = self.entities.get(*key) {
                deltas.push(ViewDelta::Add {
                    key: (*key).clone(),
                    entity: entity.to_value(),
                });
            }
        }

        deltas
    }

    fn extract_sort_value<E: EntityFields + ?Sized>(&self, entity: &E) -> SortValue {
        match lookup(entity, &self.sort_field) {
            Some(value) => value_to_sort_value(&value),
            None => SortValue::Null,
        }
    }

    fn find_position(&self, entity_key: &str) -> usize {
        if let Some((sort_key, _)) = self.entities.get(entity_key) {
            self.find_position_by_sort_key(sort_key)
        } else {
            0
        }
    }

    fn find_position_by_sort_key(&self, sort_key: &SortKey) -> usize {
        self.sorted.range(..sort_key).count()
    }

    fn rebuild_keys_cache(&mut self) {
        self.keys_cache = self.sorted.keys().map(|sk| sk.entity_key.clone()).collect();
        self.cache_dirty = false;
    }
}

/// Whether an upsert moved the entity in the order (or added it).
enum Placed {
    Unmoved,
    Moved,
}

/// Result of an upsert operation
#[derive(Debug, Clone, PartialEq)]
pub enum UpsertResult {
    /// Entity was inserted at a new position
    Inserted { position: usize },
    /// Entity was updated (may or may not have moved)
    Updated { position: usize },
}

/// Whether merging `patch` into `base` keeps a field `patch` lacks: whether
/// `base` has one, in an object where `patch` has an object too.
fn keeps_fields(base: &Value, patch: &Value) -> bool {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => base
            .iter()
            .any(|(key, base)| patch.get(key).is_none_or(|patch| keeps_fields(base, patch))),
        _ => false,
    }
}

fn value_to_sort_value(v: &Value) -> SortValue {
    match v {
        Value::Null => SortValue::Null,
        Value::Bool(b) => SortValue::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                SortValue::Integer(i)
            } else if let Some(f) = n.as_f64() {
                SortValue::Float(OrderedFloat(f))
            } else {
                SortValue::Null
            }
        }
        Value::String(s) => SortValue::String(s.clone()),
        _ => SortValue::Null,
    }
}

/// A string that is a decimal integer (`-?[0-9]+`), as [`compare_decimal_strings`]
/// reads it: its sign, where its significant digits start, and their value
/// when it fits in a `u128`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DecimalKey {
    negative: bool,
    /// Where the digits start once leading zeros are dropped ("0" keeps one).
    start: usize,
    /// Those digits' value, when they fit.
    value: Option<u128>,
}

impl DecimalKey {
    fn of(sort_value: &SortValue) -> Option<Self> {
        match sort_value {
            SortValue::String(text) => Self::parse(text),
            _ => None,
        }
    }

    fn parse(text: &str) -> Option<Self> {
        let (negative, digits) = match text.strip_prefix('-') {
            Some(digits) => (true, digits),
            None => (false, text),
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let significant = digits.trim_start_matches('0');
        let significant = if significant.is_empty() {
            &digits[digits.len() - 1..]
        } else {
            significant
        };
        Some(Self {
            negative: negative && significant != "0",
            start: text.len() - significant.len(),
            value: significant.parse().ok(),
        })
    }

    /// [`compare_decimal_strings`] for `left` and `right`, whose keys these
    /// are.
    fn cmp(&self, left: &str, other: &Self, right: &str) -> Ordering {
        match (self.negative, other.negative) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => {
                let magnitude = match (self.value, other.value) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    _ => {
                        let (a, b) = (&left[self.start..], &right[other.start..]);
                        a.len().cmp(&b.len()).then_with(|| a.cmp(b))
                    }
                };
                if self.negative {
                    magnitude.reverse()
                } else {
                    magnitude
                }
            }
        }
    }
}

/// How two string sort values order: decimal integers by value, before every
/// other string, and other strings by their bytes.
///
/// Ranking decimals apart from the rest keeps the order total. Comparing a
/// decimal with any string by bytes would not: `"2" < "10"` by value, but
/// `"10" < "1a" < "2"` by bytes, and an order with a cycle loses entries.
fn compare_strings(left: &str, right: &str) -> Ordering {
    compare_decimal_strings(left, right).unwrap_or_else(|| {
        let left_decimal = DecimalKey::parse(left).is_some();
        let right_decimal = DecimalKey::parse(right).is_some();
        right_decimal
            .cmp(&left_decimal)
            .then_with(|| left.cmp(right))
    })
}

fn compare_decimal_strings(left: &str, right: &str) -> Option<Ordering> {
    fn parts(value: &str) -> Option<(bool, &str)> {
        let (negative, digits) = match value.strip_prefix('-') {
            Some(digits) => (true, digits),
            None => (false, value),
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }

        let digits = digits.trim_start_matches('0');
        let digits = if digits.is_empty() { "0" } else { digits };
        Some((negative && digits != "0", digits))
    }

    let (left_negative, left_digits) = parts(left)?;
    let (right_negative, right_digits) = parts(right)?;

    match (left_negative, right_negative) {
        (true, false) => Some(Ordering::Less),
        (false, true) => Some(Ordering::Greater),
        _ => {
            let magnitude = left_digits
                .len()
                .cmp(&right_digits.len())
                .then_with(|| left_digits.cmp(right_digits));
            Some(if left_negative {
                magnitude.reverse()
            } else {
                magnitude
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_sorted_cache_basic() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string()],
            SortOrder::Desc,
        );

        cache.upsert("a".to_string(), json!({"id": 1, "name": "first"}));
        cache.upsert("b".to_string(), json!({"id": 3, "name": "third"}));
        cache.upsert("c".to_string(), json!({"id": 2, "name": "second"}));

        let keys = cache.ordered_keys();
        // Desc order: 3, 2, 1
        assert_eq!(keys, vec!["b", "c", "a"]);
    }

    #[test]
    fn test_sorted_cache_window() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string()],
            SortOrder::Desc,
        );

        for i in 1..=10 {
            cache.upsert(format!("e{}", i), json!({"id": i}));
        }

        // Desc order: 10, 9, 8, 7, 6, 5, 4, 3, 2, 1
        let window = cache.get_window(0, 3);
        assert_eq!(window.len(), 3);
        assert_eq!(window[0].0, "e10");
        assert_eq!(window[1].0, "e9");
        assert_eq!(window[2].0, "e8");

        let window = cache.get_window(3, 3);
        assert_eq!(window[0].0, "e7");
    }

    #[test]
    fn all_ordered_preserves_stable_sort_and_tie_breaking() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );
        cache.upsert("b".to_string(), json!({"score": 10}));
        cache.upsert("a".to_string(), json!({"score": 10}));
        cache.upsert("c".to_string(), json!({"score": 9}));

        let keys: Vec<_> = cache
            .get_all_ordered()
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(keys, ["a", "b", "c"]);
    }

    #[test]
    fn test_sorted_cache_update_moves_position() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        cache.upsert("a".to_string(), json!({"score": 10}));
        cache.upsert("b".to_string(), json!({"score": 20}));
        cache.upsert("c".to_string(), json!({"score": 15}));

        // Order: b(20), c(15), a(10)
        assert_eq!(cache.ordered_keys(), vec!["b", "c", "a"]);

        // Update a to have highest score
        cache.upsert("a".to_string(), json!({"score": 25}));

        // New order: a(25), b(20), c(15)
        assert_eq!(cache.ordered_keys(), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_sorted_cache_remove() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string()],
            SortOrder::Asc,
        );

        cache.upsert("a".to_string(), json!({"id": 1}));
        cache.upsert("b".to_string(), json!({"id": 2}));
        cache.upsert("c".to_string(), json!({"id": 3}));

        assert_eq!(cache.len(), 3);

        let pos = cache.remove("b");
        assert_eq!(pos, Some(1));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.ordered_keys(), vec!["a", "c"]);
    }

    #[test]
    fn test_compute_window_deltas() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string()],
            SortOrder::Desc,
        );

        // Initial: 5, 4, 3, 2, 1
        for i in 1..=5 {
            cache.upsert(format!("e{}", i), json!({"id": i}));
        }

        let old_window: Vec<String> = vec!["e5".to_string(), "e4".to_string(), "e3".to_string()];

        // Add e6 (new top)
        cache.upsert("e6".to_string(), json!({"id": 6}));

        // New order: 6, 5, 4, 3, 2, 1
        // New top 3: e6, e5, e4
        let deltas = cache.compute_window_deltas(&old_window, 0, 3);

        assert_eq!(deltas.len(), 2);
        // e3 removed from window
        assert!(deltas
            .iter()
            .any(|d| matches!(d, ViewDelta::Remove { key } if key == "e3")));
        // e6 added to window
        assert!(deltas
            .iter()
            .any(|d| matches!(d, ViewDelta::Add { key, .. } if key == "e6")));
    }

    fn keys(cache: &mut SortedViewCache) -> Vec<String> {
        cache.ordered_keys().to_vec()
    }

    #[test]
    fn bounded_upsert_evicts_bottom_of_desc_order() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        for i in 1..=10 {
            cache.upsert_bounded(format!("e{i}"), json!({"score": i}), 4);
            assert!(cache.len() <= 4);
        }

        assert_eq!(cache.len(), 4);
        assert_eq!(keys(&mut cache), ["e10", "e9", "e8", "e7"]);
        assert!(cache.get("e1").is_none());
        assert!(cache.get("e6").is_none());
    }

    #[test]
    fn would_keep_matches_what_a_bounded_upsert_keeps() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );
        for i in 1..=10 {
            let entity = json!({"score": i});
            let expected = cache.would_keep(&format!("e{i}"), &entity, 4);
            cache.upsert_bounded(format!("e{i}"), entity, 4);
            assert!(expected, "an entity above a full cache's tail is kept");
        }
        // e7 is the tail of [e10, e9, e8, e7].
        for (key, score, kept) in [("e0", 0, false), ("e6", 6, false), ("e11", 11, true)] {
            let entity = json!({"score": score});
            assert_eq!(cache.would_keep(key, &entity, 4), kept, "{key}");
            let mut copy = SortedViewCache::new(
                "test/top".to_string(),
                vec!["score".to_string()],
                SortOrder::Desc,
            );
            for existing in keys(&mut cache) {
                let value = cache.get(&existing).unwrap().clone();
                copy.upsert_bounded(existing, value, 4);
            }
            copy.upsert_bounded(key.to_string(), entity, 4);
            assert_eq!(copy.get(key).is_some(), kept, "{key}");
        }
        // A held entity is always kept, even when its new value sorts last.
        assert!(cache.would_keep("e8", &json!({"score": -1}), 4));
    }

    #[test]
    fn bounded_upsert_evicts_bottom_of_asc_order() {
        let mut cache = SortedViewCache::new(
            "test/bottom".to_string(),
            vec!["score".to_string()],
            SortOrder::Asc,
        );

        for i in (1..=10).rev() {
            cache.upsert_bounded(format!("e{i}"), json!({"score": i}), 4);
            assert!(cache.len() <= 4);
        }

        assert_eq!(cache.len(), 4);
        assert_eq!(keys(&mut cache), ["e1", "e2", "e3", "e4"]);
        assert!(cache.get("e10").is_none());
    }

    #[test]
    fn bounded_upsert_does_not_evict_stale_top_entities() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        // The leader is inserted first and never updated again; recency-based
        // eviction would drop it.
        cache.upsert_bounded("leader".to_string(), json!({"score": 1_000}), 3);
        for i in 1..=20 {
            cache.upsert_bounded(format!("e{i}"), json!({"score": i}), 3);
        }

        assert_eq!(keys(&mut cache), ["leader", "e20", "e19"]);
    }

    #[test]
    fn windows_within_cap_match_unbounded_cache() {
        let mut bounded = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );
        let mut unbounded = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        // Scores arrive out of order within each round and every entity moves
        // up on each later round. Entities never move down, so nothing
        // evicted can belong back inside the cap (see the edge-case test
        // below for what happens when they do).
        for i in 0..200u64 {
            let key = format!("e{}", i % 60);
            let score = (i / 60) * 1_000 + (i * 37) % 101;
            let entity = json!({"score": score, "n": i});
            bounded.upsert_bounded(key.clone(), entity.clone(), 25);
            unbounded.upsert(key, entity);
        }

        assert_eq!(bounded.len(), 25);
        for (skip, take) in [(0, 25), (0, 10), (5, 20), (24, 1)] {
            assert_eq!(
                bounded.get_window(skip, take),
                unbounded.get_window(skip, take),
                "window skip={skip} take={take}"
            );
        }
    }

    #[test]
    fn evicted_entity_is_missing_after_top_moves_down_until_it_updates() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        for i in 1..=4 {
            cache.upsert_bounded(format!("e{i}"), json!({"score": i}), 3);
        }
        assert_eq!(keys(&mut cache), ["e4", "e3", "e2"]);

        // The leader drops to the bottom; e1 would now rank third but was
        // evicted and has not updated, so e4 holds third place instead.
        cache.upsert_bounded("e4".to_string(), json!({"score": 0}), 3);
        assert_eq!(keys(&mut cache), ["e3", "e2", "e4"]);

        // Once e1 updates it re-enters at its correct position.
        cache.upsert_bounded("e1".to_string(), json!({"score": 1}), 3);
        assert_eq!(keys(&mut cache), ["e3", "e2", "e1"]);
    }

    #[test]
    fn evicted_entity_reenters_when_upserted_again() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        for i in 1..=5 {
            cache.upsert_bounded(format!("e{i}"), json!({"score": i}), 3);
        }
        assert!(cache.get("e1").is_none());

        let result = cache.upsert_bounded("e1".to_string(), json!({"score": 100}), 3);
        assert_eq!(result, UpsertResult::Inserted { position: 0 });
        assert_eq!(keys(&mut cache), ["e1", "e5", "e4"]);
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn upsert_below_full_cap_is_evicted_immediately() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        for i in 10..=12 {
            cache.upsert_bounded(format!("e{i}"), json!({"score": i}), 3);
        }
        let result = cache.upsert_bounded("low".to_string(), json!({"score": 1}), 3);

        assert_eq!(result, UpsertResult::Inserted { position: 3 });
        assert!(cache.get("low").is_none());
        assert_eq!(keys(&mut cache), ["e12", "e11", "e10"]);
    }

    #[test]
    fn trim_keeps_keys_cache_and_entities_consistent() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        for i in 1..=10 {
            cache.upsert(format!("e{i}"), json!({"score": i}));
        }
        // Build the keys cache so the trim takes the in-place truncate path.
        assert_eq!(cache.ordered_keys().len(), 10);

        assert_eq!(cache.trim_to_max_entries(4), 6);
        assert_eq!(cache.trim_to_max_entries(4), 0);
        assert_eq!(cache.len(), 4);
        assert_eq!(keys(&mut cache), ["e10", "e9", "e8", "e7"]);
        assert_eq!(cache.get_all_ordered().len(), 4);
        assert_eq!(cache.remove("e7"), Some(3));
        assert_eq!(keys(&mut cache), ["e10", "e9", "e8"]);
    }

    #[test]
    fn test_nested_sort_field() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string(), "round_id".to_string()],
            SortOrder::Desc,
        );

        cache.upsert("a".to_string(), json!({"id": {"round_id": 1}}));
        cache.upsert("b".to_string(), json!({"id": {"round_id": 3}}));
        cache.upsert("c".to_string(), json!({"id": {"round_id": 2}}));

        let keys = cache.ordered_keys();
        assert_eq!(keys, vec!["b", "c", "a"]);
    }

    #[test]
    fn test_nested_decimal_string_sort_field() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string(), "round_id".to_string()],
            SortOrder::Desc,
        );

        cache.upsert("9".to_string(), json!({"id": {"round_id": "9"}}));
        cache.upsert("100".to_string(), json!({"id": {"round_id": "100"}}));
        cache.upsert("10".to_string(), json!({"id": {"round_id": "10"}}));

        assert_eq!(cache.ordered_keys(), vec!["100", "10", "9"]);
    }

    /// Decimal and other strings in one view rank decimals first, so the
    /// order stays total and moving an entity never loses it.
    #[test]
    fn test_mixed_decimal_and_text_sort_values() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["name".to_string()],
            SortOrder::Asc,
        );

        cache.upsert("two".to_string(), json!({"name": "2"}));
        cache.upsert("ten".to_string(), json!({"name": "10"}));
        cache.upsert("text".to_string(), json!({"name": "1a"}));
        assert_eq!(cache.ordered_keys(), vec!["two", "ten", "text"]);

        cache.put("ten".to_string(), json!({"name": "11"}));
        assert_eq!(cache.ordered_keys(), vec!["two", "ten", "text"]);
        cache.put("ten".to_string(), json!({"name": "1"}));
        assert_eq!(cache.ordered_keys(), vec!["ten", "two", "text"]);
        cache.put("text".to_string(), json!({"name": "0"}));
        assert_eq!(cache.ordered_keys(), vec!["text", "ten", "two"]);
        assert_eq!(cache.len(), 3);
        assert_eq!(cache.sorted.len(), 3);
    }

    /// String ordering is transitive across decimal and other strings.
    #[test]
    fn string_order_is_transitive() {
        let texts = [
            "", "-", "-1", "-10", "0", "007", "1", "10", "1a", "2", "a", "+5", "1.5",
        ];
        for a in texts {
            for b in texts {
                for c in texts {
                    if compare_strings(a, b) != Ordering::Greater
                        && compare_strings(b, c) != Ordering::Greater
                    {
                        assert_ne!(
                            compare_strings(a, c),
                            Ordering::Greater,
                            "{a:?} <= {b:?} <= {c:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_descending_string_sort_field() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["name".to_string()],
            SortOrder::Desc,
        );

        cache.upsert("a".to_string(), json!({"name": "alpha"}));
        cache.upsert("c".to_string(), json!({"name": "charlie"}));
        cache.upsert("b".to_string(), json!({"name": "bravo"}));

        assert_eq!(cache.ordered_keys(), vec!["c", "b", "a"]);
    }

    #[test]
    fn test_update_with_missing_sort_field_preserves_position() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string(), "round_id".to_string()],
            SortOrder::Desc,
        );

        cache.upsert(
            "100".to_string(),
            json!({"id": {"round_id": 100}, "data": "initial"}),
        );
        cache.upsert(
            "200".to_string(),
            json!({"id": {"round_id": 200}, "data": "initial"}),
        );
        cache.upsert(
            "300".to_string(),
            json!({"id": {"round_id": 300}, "data": "initial"}),
        );

        assert_eq!(cache.ordered_keys(), vec!["300", "200", "100"]);

        cache.upsert("200".to_string(), json!({"data": "updated_without_id"}));

        assert_eq!(
            cache.ordered_keys(),
            vec!["300", "200", "100"],
            "Entity 200 should retain its position even when updated without sort field"
        );

        let entity = cache.get("200").unwrap();
        assert_eq!(entity["data"], "updated_without_id");
    }

    #[test]
    fn test_new_entity_with_missing_sort_field_sorts_last() {
        for order in [SortOrder::Desc, SortOrder::Asc] {
            let mut cache = SortedViewCache::new(
                "test/latest".to_string(),
                vec!["id".to_string(), "round_id".to_string()],
                order,
            );

            cache.upsert("100".to_string(), json!({"id": {"round_id": 100}}));
            cache.upsert("200".to_string(), json!({"id": {"round_id": 200}}));
            cache.upsert("new".to_string(), json!({"data": "no_sort_field"}));
            cache.upsert("nil".to_string(), json!({"id": {"round_id": null}}));

            let ranked = match order {
                SortOrder::Desc => ["200", "100"],
                SortOrder::Asc => ["100", "200"],
            };
            assert_eq!(
                cache.ordered_keys(),
                [ranked[0], ranked[1], "new", "nil"],
                "{order:?}: an entity without a sort value never outranks one with it; \
                 among themselves they order by key"
            );
        }
    }

    /// The live `latest` failure: a window full of ranked entities must not
    /// admit an unranked one, since it would evict a real member.
    #[test]
    fn a_full_window_does_not_admit_an_entity_without_a_sort_value() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string(), "round_id".to_string()],
            SortOrder::Desc,
        );
        for round in [100, 200, 300] {
            cache.upsert_bounded(round.to_string(), json!({"id": {"round_id": round}}), 3);
        }
        let partial = json!({"metrics": {"checkpoint_count": 7}});
        assert!(!cache.would_keep("1", &partial, 3));
        cache.upsert_bounded("1".to_string(), partial, 3);
        assert_eq!(cache.ordered_keys(), ["300", "200", "100"]);
    }

    /// A whole entity, holding every field the cached copy has, is kept as
    /// given: the cache shares its fields instead of copying them.
    #[test]
    fn a_whole_update_shares_the_given_entity() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );
        cache.upsert(
            "a".to_string(),
            json!({"score": 1, "name": "a", "_version": "e:1"}),
        );
        let update =
            SharedEntity::new(json!({"score": 2, "name": "a", "extra": true, "_version": "e:2"}));
        cache.upsert("a".to_string(), update.clone());
        let held = cache.get("a").unwrap();
        assert!(held.shares_fields_with(&update));
        assert_eq!(*held, update);
    }

    /// An update that lacks a field the cached copy has keeps that field, in
    /// a copy of its own: the entity it was given is left as it was.
    #[test]
    fn an_update_lacking_held_fields_merges_into_its_own_copy() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );
        cache.upsert(
            "a".to_string(),
            json!({"score": 1, "nested": {"x": 1, "y": 1}, "_version": "e:1"}),
        );
        let update = SharedEntity::new(json!({"score": 2, "nested": {"x": 2}}));
        cache.upsert("a".to_string(), update.clone());
        let held = cache.get("a").unwrap();
        assert!(!held.shares_fields_with(&update));
        assert_eq!(
            held.to_value(),
            json!({"score": 2, "nested": {"x": 2, "y": 1}, "_version": "e:1"})
        );
        assert_eq!(update.to_value(), json!({"score": 2, "nested": {"x": 2}}));
    }

    /// `_version` sorts like any other field, though copies keep it apart
    /// from the fields they share.
    #[test]
    fn a_version_sort_reads_each_copys_own_version() {
        let mut cache = SortedViewCache::new(
            "test/versions".to_string(),
            vec!["_version".to_string()],
            SortOrder::Desc,
        );
        let first = SharedEntity::new(json!({"id": 1, "_version": "e:1"}));
        let (fields, _) = first.clone().into_parts();
        cache.upsert("a".to_string(), first);
        cache.upsert(
            "b".to_string(),
            SharedEntity::from_parts(fields, Some(json!("e:2"))),
        );
        assert_eq!(cache.ordered_keys(), ["b", "a"]);
    }

    /// Comparing pre-parsed decimal keys orders strings exactly as comparing
    /// the strings themselves does.
    #[test]
    fn pre_parsed_decimals_compare_as_the_strings_do() {
        let huge = "9".repeat(45);
        let huger = format!("1{}", "0".repeat(45));
        let u128_max = u128::MAX.to_string();
        let past_u128 = (u128::MAX - 1).to_string() + "0";
        let texts = [
            "0",
            "-0",
            "000",
            "-000",
            "7",
            "007",
            "-7",
            "-007",
            "10",
            "9",
            "-10",
            "-9",
            "18446744073709551616",
            "-18446744073709551616",
            &u128_max,
            &past_u128,
            &huge,
            &huger,
            "-",
            "",
            "+5",
            "5a",
            "a",
            "abc",
            " 1",
            "1.5",
            "-1.5",
            "0x10",
        ];
        for left in texts {
            for right in texts {
                let expected = compare_strings(left, right);
                for order in [SortOrder::Asc, SortOrder::Desc] {
                    let a = SortKey::new(SortValue::String(left.to_string()), "k".into(), order);
                    let b = SortKey::new(SortValue::String(right.to_string()), "k".into(), order);
                    let expected = match order {
                        SortOrder::Asc => expected,
                        SortOrder::Desc => expected.reverse(),
                    };
                    assert_eq!(a.cmp(&b), expected, "{left:?} vs {right:?} ({order:?})");
                }
            }
        }
    }

    /// The unpositioned upserts and removals the projector uses leave the
    /// cache exactly as the positioned ones do, in every order a window can
    /// read it.
    #[test]
    fn unpositioned_writes_match_positioned_ones() {
        struct Rng(u64);
        impl Rng {
            fn below(&mut self, n: u64) -> u64 {
                self.0 = self
                    .0
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (self.0 >> 33) % n
            }
        }
        for (field, order) in [
            ("amount", SortOrder::Desc),
            ("amount", SortOrder::Asc),
            ("label", SortOrder::Asc),
        ] {
            let cache = || SortedViewCache::new("test/top".into(), vec![field.into()], order);
            let (mut positioned, mut unpositioned) = (cache(), cache());
            let mut rng = Rng(7);
            for step in 0..4_000 {
                let key = format!("k{}", rng.below(60));
                let max_entries = 1 + rng.below(40) as usize;
                let value = match rng.below(6) {
                    0 => Value::Null,
                    1 => json!(rng.below(1_000)),
                    2 => json!(format!("{}", rng.below(1_000))),
                    3 => json!(format!("-{:03}", rng.below(1_000))),
                    4 => json!(format!("{}{}", "9".repeat(40), rng.below(10))),
                    _ => json!(format!("x{}", rng.below(100))),
                };
                let entity = SharedEntity::new(json!({
                    field: value,
                    "step": step,
                    "_version": format!("e:{step}"),
                }));
                match rng.below(4) {
                    0 => {
                        positioned.remove(&key);
                        unpositioned.remove_key(&key);
                    }
                    1 => {
                        positioned.upsert(key.clone(), entity.clone());
                        unpositioned.put(key, entity);
                    }
                    _ => {
                        let keeps = positioned.would_keep(&key, &entity, max_entries);
                        if keeps {
                            positioned.upsert_bounded(key.clone(), entity.clone(), max_entries);
                        }
                        assert_eq!(
                            unpositioned.upsert_if_kept(&key, &entity, max_entries),
                            keeps
                        );
                    }
                }
                let expected: Vec<(String, Value)> = positioned
                    .ordered_keys()
                    .to_vec()
                    .into_iter()
                    .map(|key| {
                        let entity = positioned.get(&key).unwrap().to_value();
                        (key, entity)
                    })
                    .collect();
                assert_eq!(unpositioned.get_all_ordered(), expected, "step {step}");
                let skip = rng.below(8) as usize;
                assert_eq!(
                    unpositioned.get_window(skip, 5),
                    expected
                        .iter()
                        .skip(skip)
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>()
                );
                assert_eq!(unpositioned.ordered_keys(), positioned.ordered_keys());

                // The order itself, from `SortValue`'s own comparison: unranked
                // entities last either way, ties broken by key.
                let mut reference: Vec<(SortValue, String)> = expected
                    .iter()
                    .map(|(key, _)| {
                        let (sort_key, _) = &unpositioned.entities[key];
                        (sort_key.sort_value.clone(), key.clone())
                    })
                    .collect();
                reference.sort_by(|(a, a_key), (b, b_key)| {
                    let by_value = match (a, b) {
                        (SortValue::Null, SortValue::Null) => Ordering::Equal,
                        (SortValue::Null, _) => Ordering::Greater,
                        (_, SortValue::Null) => Ordering::Less,
                        _ if order == SortOrder::Desc => a.cmp(b).reverse(),
                        _ => a.cmp(b),
                    };
                    by_value.then_with(|| a_key.cmp(b_key))
                });
                let reference: Vec<&String> = reference.iter().map(|(_, key)| key).collect();
                let held: Vec<&String> = expected.iter().map(|(key, _)| key).collect();
                assert_eq!(held, reference, "step {step}");
            }
        }
    }
}
