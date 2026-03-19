use crate::materialized_view::{CompareOp, FilterConfig, SortConfig, SortOrder, ViewPipeline};
use crate::websocket::frame::Mode;

// # View System Architecture
//
// The view system uses hierarchical View IDs instead of simple entity names,
// enabling sophisticated filtering and organization:
//
// ## View ID Structure
// - Basic views: `EntityName/mode` (e.g., `SettlementGame/list`, `SettlementGame/state`)
// - Filtered views: `EntityName/mode/filter1/filter2/...` (e.g., `SettlementGame/list/active/large`)
//
// ## Subscription Model
// Clients subscribe using the full view ID:
// ```json
// {
//   "view": "SettlementGame/list/active/large"
// }
// ```
//
// ## Future Filter Examples
// - `SettlementGame/list/active/large` - Active games with large bets
// - `SettlementGame/list/user/123` - Games for specific user
// - `SettlementGame/list/recent` - Recently created games only

#[derive(Clone, Debug)]
pub struct ViewSpec {
    pub id: String,
    pub export: String,
    pub mode: Mode,
    pub projection: Projection,
    pub filters: Filters,
    pub delivery: Delivery,
    /// Optional pipeline for derived views
    pub pipeline: Option<ViewPipeline>,
    /// Source view ID if this is a derived view
    pub source_view: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Projection {
    pub fields: Option<Vec<String>>,
}

impl Projection {
    pub fn all() -> Self {
        Self { fields: None }
    }

    pub fn apply(&self, mut data: serde_json::Value) -> serde_json::Value {
        if let Some(ref field_list) = self.fields {
            if let Some(obj) = data.as_object_mut() {
                obj.retain(|k, _| field_list.contains(&k.to_string()));
            }
        }
        data
    }
}

#[derive(Clone, Debug, Default)]
pub struct Filters {
    pub keys: Option<Vec<String>>,
}

impl Filters {
    pub fn all() -> Self {
        Self { keys: None }
    }

    pub fn matches(&self, key: &str) -> bool {
        match &self.keys {
            None => true,
            Some(keys) => keys.iter().any(|k| k == key),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Delivery {
    pub coalesce_ms: Option<u64>,
}

impl ViewSpec {
    pub fn is_derived(&self) -> bool {
        self.pipeline.is_some()
    }

    pub fn from_view_def(view_def: &hyperstack_interpreter::ast::ViewDef, export: &str) -> Self {
        use hyperstack_interpreter::ast::{ViewOutput, ViewSource};

        let mode = match &view_def.output {
            ViewOutput::Collection => Mode::List,
            ViewOutput::Single => Mode::State,
            ViewOutput::Keyed { .. } => Mode::State,
        };

        let pipeline = Self::convert_pipeline(&view_def.pipeline);

        let source_view = match &view_def.source {
            ViewSource::Entity { name } => Some(format!("{}/list", name)),
            ViewSource::View { id } => Some(id.clone()),
        };

        ViewSpec {
            id: view_def.id.clone(),
            export: export.to_string(),
            mode,
            projection: Projection::all(),
            filters: Filters::all(),
            delivery: Delivery::default(),
            pipeline: Some(pipeline),
            source_view,
        }
    }

    fn convert_pipeline(transforms: &[hyperstack_interpreter::ast::ViewTransform]) -> ViewPipeline {
        use hyperstack_interpreter::ast::ViewTransform as VT;

        let mut pipeline = ViewPipeline {
            filter: None,
            sort: None,
            limit: None,
        };

        for transform in transforms {
            match transform {
                VT::Filter { predicate } => {
                    if let hyperstack_interpreter::ast::Predicate::Compare { field, op, value } =
                        predicate
                    {
                        use hyperstack_interpreter::ast::CompareOp as CO;
                        use hyperstack_interpreter::ast::PredicateValue;

                        let cmp_op = match op {
                            CO::Eq => CompareOp::Eq,
                            CO::Ne => CompareOp::Ne,
                            CO::Gt => CompareOp::Gt,
                            CO::Gte => CompareOp::Gte,
                            CO::Lt => CompareOp::Lt,
                            CO::Lte => CompareOp::Lte,
                        };

                        let filter_value = match value {
                            PredicateValue::Literal(v) => v.clone(),
                            PredicateValue::Dynamic(_) => serde_json::Value::Null,
                            PredicateValue::Field(_) => serde_json::Value::Null,
                        };

                        pipeline.filter = Some(FilterConfig {
                            field_path: field.segments.clone(),
                            op: cmp_op,
                            value: filter_value,
                        });
                    }
                }
                VT::Sort { key, order } => {
                    use hyperstack_interpreter::ast::SortOrder as SO;
                    pipeline.sort = Some(SortConfig {
                        field_path: key.segments.clone(),
                        order: match order {
                            SO::Asc => SortOrder::Asc,
                            SO::Desc => SortOrder::Desc,
                        },
                    });
                }
                VT::Take { count } => {
                    pipeline.limit = Some(*count);
                }
                VT::First | VT::Last | VT::MaxBy { .. } | VT::MinBy { .. } => {
                    pipeline.limit = Some(1);
                }
                VT::Skip { .. } => {}
            }
        }

        pipeline
    }
}
