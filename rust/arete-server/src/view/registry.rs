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
}

impl ViewIndex {
    pub fn new() -> Self {
        Self {
            by_export: HashMap::new(),
            by_id: HashMap::new(),
            sorted_caches: Arc::new(RwLock::new(HashMap::new())),
            derived_by_source: HashMap::new(),
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
        self.by_id.insert(spec.id.clone(), spec);
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
