//! Row scoping: rows of a scoped entity reach only the sessions entitled to
//! them.
//!
//! An entity may name one string field as its **scope field** (the stack's
//! LiveSpec `rowScopes`, carried on the entity spec as `scope_field`). Each
//! row's value in that field is its scope id:
//!
//! - an absent, `null` or empty value makes the row **public**: every session
//!   receives it;
//! - any other string makes it **scoped**: a session receives it only when
//!   its verified token's `row_scopes` claim lists that id;
//! - any other value makes it visible to nobody, so a malformed row fails
//!   closed.
//!
//! The scopes a session may read come only from its verified token, never
//! from the subscription request. A client's own `partition` and `filters`
//! still apply, but only on top of the scope check, so they can narrow what a
//! session receives and never widen it. Sessions without the claim, and
//! servers without authentication, receive only public rows of scoped
//! entities. Entities without a scope field are unaffected.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::{RwLock, RwLockReadGuard};

use crate::shared_entity::{lookup, EntityFields};
use crate::websocket::auth::AuthContext;

/// Default cap on the scope partitions one sorted view keeps (see
/// [`SortedViewCache::partition_by_scope`](crate::sorted_cache::SortedViewCache::partition_by_scope)).
pub const DEFAULT_MAX_SCOPE_PARTITIONS: usize = 10_000;

/// The field of a scoped entity's rows that holds their scope id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeField {
    path: Arc<[String]>,
}

impl ScopeField {
    /// A scope field from its dotted path (`meta.scope`).
    pub fn new(dotted_path: &str) -> Self {
        Self {
            path: dotted_path.split('.').map(str::to_string).collect(),
        }
    }

    /// The field's path segments.
    pub fn path(&self) -> &[String] {
        &self.path
    }

    /// The scope of a whole row: an absent field is public.
    pub fn scope_of<E: EntityFields + ?Sized>(&self, row: &E) -> RowScope {
        match lookup(row, self.path.iter()) {
            Some(value) => RowScope::of_value(&value),
            None => RowScope::Public,
        }
    }

    /// The scope a frame's data sets, or `None` when the data does not carry
    /// the field (a patch that leaves the scope as it was).
    pub fn scope_in<E: EntityFields + ?Sized>(&self, data: &E) -> Option<RowScope> {
        lookup(data, self.path.iter()).map(|value| RowScope::of_value(&value))
    }
}

/// A row's scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RowScope {
    /// No scope: every session receives the row.
    Public,
    /// Only sessions whose row scopes list this id receive the row.
    Scoped(String),
    /// The scope field holds something other than a string: nobody receives
    /// the row.
    Invalid,
}

impl RowScope {
    fn of_value(value: &Value) -> Self {
        match value {
            Value::Null => Self::Public,
            Value::String(scope) if scope.is_empty() => Self::Public,
            Value::String(scope) => Self::Scoped(scope.clone()),
            _ => Self::Invalid,
        }
    }

    /// The partition a sorted view keeps the row in: `""` for public rows,
    /// the scope id otherwise, and none for an invalid scope.
    pub(crate) fn partition(&self) -> Option<&str> {
        match self {
            Self::Public => Some(""),
            Self::Scoped(scope) => Some(scope),
            Self::Invalid => None,
        }
    }
}

/// The row scopes a verified session may read: its `row_scopes` claim,
/// without empty ids. Empty for a session without the claim or without
/// authentication.
pub fn session_row_scopes(auth: Option<&AuthContext>) -> BTreeSet<String> {
    auth.and_then(|auth| auth.row_scopes.as_ref())
        .map(|scopes| {
            scopes
                .iter()
                .filter(|scope| !scope.is_empty())
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Which rows of one scoped view one subscription may receive: the view's
/// scope field and the session's row scopes, taken from its verified token
/// when the subscription attached.
///
/// It also carries the connection's row-scope generation. A token refresh
/// that changes the session's row scopes bumps the generation (under the
/// write half of the connection's lock) before it resnapshots, so a
/// subscription task still running on the old scopes can no longer send:
/// see [`RowFilter::hold`].
#[derive(Debug, Clone)]
pub(crate) struct RowFilter {
    field: ScopeField,
    scopes: Arc<BTreeSet<String>>,
    lock: Arc<RwLock<u64>>,
    generation: u64,
}

impl RowFilter {
    pub(crate) fn new(
        field: ScopeField,
        scopes: BTreeSet<String>,
        lock: Arc<RwLock<u64>>,
        generation: u64,
    ) -> Self {
        Self {
            field,
            scopes: Arc::new(scopes),
            lock,
            generation,
        }
    }

    pub(crate) fn field(&self) -> &ScopeField {
        &self.field
    }

    /// The session's row scopes.
    pub(crate) fn scopes(&self) -> &BTreeSet<String> {
        &self.scopes
    }

    pub(crate) fn allows_scope(&self, scope: &RowScope) -> bool {
        match scope {
            RowScope::Public => true,
            RowScope::Scoped(scope) => self.scopes.contains(scope),
            RowScope::Invalid => false,
        }
    }

    /// Whether the session may receive this whole row.
    pub(crate) fn allows<E: EntityFields + ?Sized>(&self, row: &E) -> bool {
        self.allows_scope(&self.field.scope_of(row))
    }

    /// Whether frame data that carries the scope field moves the row out of
    /// this session's reach. `false` when the data leaves the scope as it was.
    pub(crate) fn denies_frame_data<E: EntityFields + ?Sized>(&self, data: &E) -> bool {
        self.field
            .scope_in(data)
            .is_some_and(|scope| !self.allows_scope(&scope))
    }

    /// Hold the right to send on these scopes, without waiting. `None` once a
    /// refresh changed the session's row scopes, or while one is changing
    /// them: the subscription must stop.
    pub(crate) fn try_hold(&self) -> Option<RwLockReadGuard<'_, u64>> {
        self.lock
            .try_read()
            .ok()
            .filter(|generation| **generation == self.generation)
    }

    /// [`Self::try_hold`], waiting out a refresh in progress.
    pub(crate) async fn hold(&self) -> Option<RwLockReadGuard<'_, u64>> {
        let generation = self.lock.read().await;
        (*generation == self.generation).then_some(generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn filter(scopes: &[&str]) -> RowFilter {
        RowFilter::new(
            ScopeField::new("meta.scope"),
            scopes.iter().map(|scope| scope.to_string()).collect(),
            Arc::new(RwLock::new(0)),
            0,
        )
    }

    #[test]
    fn absent_null_and_empty_scopes_are_public() {
        let field = ScopeField::new("meta.scope");
        assert_eq!(field.scope_of(&json!({})), RowScope::Public);
        assert_eq!(field.scope_of(&json!({"meta": {}})), RowScope::Public);
        assert_eq!(
            field.scope_of(&json!({"meta": {"scope": null}})),
            RowScope::Public
        );
        assert_eq!(
            field.scope_of(&json!({"meta": {"scope": ""}})),
            RowScope::Public
        );
        assert_eq!(
            field.scope_of(&json!({"meta": {"scope": "a"}})),
            RowScope::Scoped("a".into())
        );
        assert_eq!(
            field.scope_of(&json!({"meta": {"scope": 7}})),
            RowScope::Invalid
        );
    }

    #[test]
    fn a_session_reads_public_rows_and_its_own_scopes_only() {
        let scoped_to_a = filter(&["a"]);
        assert!(scoped_to_a.allows(&json!({"meta": {"scope": "a"}})));
        assert!(scoped_to_a.allows(&json!({"name": "public"})));
        assert!(!scoped_to_a.allows(&json!({"meta": {"scope": "b"}})));
        assert!(!scoped_to_a.allows(&json!({"meta": {"scope": ["a"]}})));

        let unscoped = filter(&[]);
        assert!(unscoped.allows(&json!({"meta": {"scope": ""}})));
        assert!(!unscoped.allows(&json!({"meta": {"scope": "a"}})));
    }

    #[test]
    fn frame_data_without_the_field_leaves_the_scope_alone() {
        let scoped_to_a = filter(&["a"]);
        assert!(!scoped_to_a.denies_frame_data(&json!({"count": 2})));
        assert!(!scoped_to_a.denies_frame_data(&json!({"meta": {"scope": "a"}})));
        assert!(scoped_to_a.denies_frame_data(&json!({"meta": {"scope": "b"}})));
    }

    #[test]
    fn session_row_scopes_come_from_the_claim_without_empty_ids() {
        let claims = arete_auth::SessionClaims::builder("issuer", "subject", "audience")
            .with_row_scopes(["a", "", "b"])
            .build();
        let context = AuthContext::from_claims(claims);
        assert_eq!(
            session_row_scopes(Some(&context)),
            ["a".to_string(), "b".to_string()].into_iter().collect()
        );
        assert!(session_row_scopes(None).is_empty());
    }

    #[tokio::test]
    async fn a_bumped_generation_stops_the_old_filter() {
        let lock = Arc::new(RwLock::new(0));
        let old = RowFilter::new(ScopeField::new("scope"), BTreeSet::new(), lock.clone(), 0);
        assert!(old.try_hold().is_some());
        *lock.write().await += 1;
        assert!(old.try_hold().is_none());
        assert!(old.hold().await.is_none());
    }
}
