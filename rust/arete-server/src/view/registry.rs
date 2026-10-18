use crate::row_scope::{ScopeField, DEFAULT_MAX_SCOPE_PARTITIONS};
use crate::sorted_cache::{SortOrder, SortedViewCache};
use crate::view::ViewSpec;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct ViewIndex {
    by_export: HashMap<String, Vec<ViewSpec>>,
    by_id: HashMap<String, ViewSpec>,
    sorted_caches: Arc<RwLock<HashMap<String, SortedViewCache>>>,
    /// Map from source view ID to derived view IDs
    derived_by_source: HashMap<String, Vec<String>>,
    /// Row-scoped entities (by export) and their scope fields.
    entity_scopes: HashMap<String, ScopeField>,
    /// Cap on the scope partitions of each sorted view of a scoped entity.
    max_scope_partitions: usize,
}

impl ViewIndex {
    pub fn new() -> Self {
        Self {
            by_export: HashMap::new(),
            by_id: HashMap::new(),
            sorted_caches: Arc::new(RwLock::new(HashMap::new())),
            derived_by_source: HashMap::new(),
            entity_scopes: HashMap::new(),
            max_scope_partitions: DEFAULT_MAX_SCOPE_PARTITIONS,
        }
    }

    /// Scope an entity's rows by `field` (row scoping, see
    /// [`crate::row_scope`]): every view of the entity, derived views
    /// included, then delivers a row only to sessions whose row scopes cover
    /// it, and its sorted views keep their rows per scope.
    ///
    /// Call while building the index, before it is shared.
    pub fn set_entity_scope(&mut self, export: impl Into<String>, field: ScopeField) {
        self.entity_scopes.insert(export.into(), field);
        self.partition_scoped_caches();
    }

    /// Cap the scope partitions each sorted view of a scoped entity keeps
    /// (default [`DEFAULT_MAX_SCOPE_PARTITIONS`]). See
    /// [`SortedViewCache::partition_by_scope`].
    pub fn set_max_scope_partitions(&mut self, max_partitions: usize) {
        self.max_scope_partitions = max_partitions.max(1);
        self.partition_scoped_caches();
    }

    pub fn max_scope_partitions(&self) -> usize {
        self.max_scope_partitions
    }

    /// The scope field governing a view's rows: its entity's, for the view
    /// or any view it derives from. `None` for views of unscoped entities.
    pub fn scope_field_for_view(&self, view_id: &str) -> Option<&ScopeField> {
        if self.entity_scopes.is_empty() {
            return None;
        }
        let mut current = view_id;
        // Bounded walk: a source chain never revisits a view, and a cycle
        // must not hang a subscription.
        for _ in 0..=self.by_id.len() {
            let Some(spec) = self.by_id.get(current) else {
                // An unregistered source: judge by its entity prefix.
                let entity = current.split('/').next().unwrap_or(current);
                return self.entity_scopes.get(entity);
            };
            if let Some(field) = self.entity_scopes.get(&spec.export) {
                return Some(field);
            }
            match spec.source_view.as_deref() {
                Some(source) if source != current => current = source,
                _ => return None,
            }
        }
        None
    }

    /// Reject projections that would erase the field enforcing row access.
    pub(crate) fn validate_scope_projection(&self, spec: &ViewSpec) -> anyhow::Result<()> {
        if let (Some(scope), Some(fields)) =
            (self.scope_field_for_view(&spec.id), &spec.projection.fields)
        {
            if !scope
                .path()
                .first()
                .is_some_and(|root| fields.contains(root))
            {
                anyhow::bail!(
                    "View {} projection must retain its row scope field",
                    spec.id
                );
            }
        }
        Ok(())
    }

    pub(crate) fn validate_scope_projections(&self) -> anyhow::Result<()> {
        for spec in self.by_id.values() {
            self.validate_scope_projection(spec)?;
        }
        Ok(())
    }

    /// Whether any entity is row-scoped.
    pub fn has_scoped_entities(&self) -> bool {
        !self.entity_scopes.is_empty()
    }

    /// Partition the sorted caches of every view of a scoped entity.
    fn partition_scoped_caches(&mut self) {
        let scoped: Vec<(String, ScopeField)> = self
            .by_id
            .keys()
            .filter_map(|id| {
                self.scope_field_for_view(id)
                    .map(|field| (id.clone(), field.clone()))
            })
            .collect();
        if scoped.is_empty() {
            return;
        }
        let max_partitions = self.max_scope_partitions;
        let caches = Arc::get_mut(&mut self.sorted_caches)
            .expect("Cannot partition sorted caches: Arc is shared");
        let caches = caches.get_mut();
        for (view_id, field) in scoped {
            if let Some(cache) = caches.get_mut(&view_id) {
                cache.partition_by_scope(field, max_partitions);
            }
        }
    }

    pub fn add_spec(&mut self, spec: ViewSpec) {
        if let Some(ref source) = spec.source_view {
            self.derived_by_source
                .entry(source.clone())
                .or_default()
                .push(spec.id.clone());
        }

        if let Some(ref pipeline) = spec.pipeline {
            if let Some(ref sort_config) = pipeline.sort {
                self.init_sorted_cache_sync(
                    &spec.id,
                    sort_config.field_path.clone(),
                    sort_config.order.into(),
                );
            }
        }

        // Only add non-derived views to by_export.
        // Derived views receive updates via their source_view subscription,
        // not directly from the projector.
        if !spec.is_derived() {
            self.by_export
                .entry(spec.export.clone())
                .or_default()
                .push(spec.clone());
        }
        let partition = self.has_scoped_entities();
        self.by_id.insert(spec.id.clone(), spec);
        if partition {
            self.partition_scoped_caches();
        }
    }

    pub fn by_export(&self, entity: &str) -> &[ViewSpec] {
        self.by_export
            .get(entity)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn get_view(&self, id: &str) -> Option<&ViewSpec> {
        self.by_id.get(id)
    }

    /// Stable view order for snapshot compatibility contracts.
    pub(crate) fn snapshot_specs(&self) -> Vec<&ViewSpec> {
        let mut specs = self.by_id.values().collect::<Vec<_>>();
        specs.sort_by(|left, right| left.id.cmp(&right.id));
        specs
    }

    pub fn get_derived_views(&self) -> Vec<&ViewSpec> {
        self.by_id.values().filter(|s| s.is_derived()).collect()
    }

    pub fn get_derived_views_for_source(&self, source_view_id: &str) -> Vec<&ViewSpec> {
        self.derived_by_source
            .get(source_view_id)
            .map(|ids| ids.iter().filter_map(|id| self.by_id.get(id)).collect())
            .unwrap_or_default()
    }

    pub fn sorted_caches(&self) -> Arc<RwLock<HashMap<String, SortedViewCache>>> {
        self.sorted_caches.clone()
    }

    pub async fn init_sorted_cache(
        &self,
        view_id: &str,
        sort_field: Vec<String>,
        order: SortOrder,
    ) {
        let mut caches = self.sorted_caches.write().await;
        if !caches.contains_key(view_id) {
            caches.insert(
                view_id.to_string(),
                SortedViewCache::new(view_id.to_string(), sort_field, order),
            );
        }
    }

    fn init_sorted_cache_sync(&mut self, view_id: &str, sort_field: Vec<String>, order: SortOrder) {
        let cache = SortedViewCache::new(view_id.to_string(), sort_field, order);
        let caches = Arc::get_mut(&mut self.sorted_caches)
            .expect("Cannot initialize sorted cache: Arc is shared");
        let caches = caches.get_mut();
        if !caches.contains_key(view_id) {
            caches.insert(view_id.to_string(), cache);
        }
    }
}

impl Default for ViewIndex {
    fn default() -> Self {
        Self::new()
    }
}
