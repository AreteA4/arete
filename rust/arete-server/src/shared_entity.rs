//! Cached entities that share their fields between the views holding them.
//!
//! Every view of an export receives the same patches, and a derived view holds
//! the entity its source view holds, so most cached copies of one entity have
//! the same fields. Each view's frames carry their own `_version` (see the
//! projector), which a view's copy keeps, so the copies differ in that one
//! field. A [`SharedEntity`] keeps every other field behind an [`Arc`] that the
//! copies share, and `_version` beside it, per copy. A change replaces or
//! copies the shared fields before it writes (copy-on-write), so it never
//! reaches another holder's copy.

use std::borrow::Cow;
use std::ops::Index;
use std::sync::Arc;

use serde_json::Value;

/// The field each view's copy of an entity stamps with its own frame version.
pub const VERSION_FIELD: &str = "_version";

/// One copy of a cached entity: fields shared with every other copy that has
/// the same ones, plus this copy's `_version`.
///
/// It reads and compares as the whole entity it stands for, `_version`
/// included: [`Self::to_value`] rebuilds that entity, and equality, field
/// access ([`EntityFields`]) and indexing see it too.
#[derive(Clone, Debug)]
pub struct SharedEntity {
    /// Every field but `_version`. Never has a top-level `_version`.
    fields: Arc<Value>,
    /// `_version`, if the entity is an object that has one.
    version: Option<Value>,
}

impl SharedEntity {
    /// Hold `entity` unshared.
    pub fn new(entity: Value) -> Self {
        let (fields, version) = split_version(entity);
        Self {
            fields: Arc::new(fields),
            version,
        }
    }

    /// Hold `fields`, shared with whoever else holds them, under `version`.
    ///
    /// `fields` must not have a top-level `_version`, and only an object
    /// carries one.
    pub(crate) fn from_parts(fields: Arc<Value>, version: Option<Value>) -> Self {
        debug_assert!(
            fields
                .as_object()
                .is_none_or(|map| !map.contains_key(VERSION_FIELD)),
            "shared fields must not carry `_version`"
        );
        debug_assert!(
            fields.is_object() || version.is_none(),
            "only an object carries `_version`"
        );
        Self { fields, version }
    }

    pub(crate) fn into_parts(self) -> (Arc<Value>, Option<Value>) {
        (self.fields, self.version)
    }

    pub(crate) fn fields(&self) -> &Arc<Value> {
        &self.fields
    }

    pub(crate) fn fields_mut(&mut self) -> &mut Arc<Value> {
        &mut self.fields
    }

    pub(crate) fn version_mut(&mut self) -> &mut Option<Value> {
        &mut self.version
    }

    /// The entity's `_version`, if it has one.
    pub fn version(&self) -> Option<&Value> {
        self.version.as_ref()
    }

    /// Whether `other` shares this entity's fields rather than holding a copy
    /// of its own. Sharing says nothing about `_version`.
    pub fn shares_fields_with(&self, other: &SharedEntity) -> bool {
        Arc::ptr_eq(&self.fields, &other.fields)
    }

    /// The whole entity, as an owned value.
    pub fn to_value(&self) -> Value {
        with_version(Value::clone(&self.fields), self.version.clone())
    }

    /// The whole entity, copying the fields only if another holder shares
    /// them.
    pub fn into_value(self) -> Value {
        let fields = Arc::try_unwrap(self.fields).unwrap_or_else(|shared| Value::clone(&shared));
        with_version(fields, self.version)
    }
}

impl From<Value> for SharedEntity {
    fn from(entity: Value) -> Self {
        Self::new(entity)
    }
}

impl From<SharedEntity> for Value {
    fn from(entity: SharedEntity) -> Self {
        entity.into_value()
    }
}

/// Equal when the whole entities are: `_version` and every other field.
impl PartialEq for SharedEntity {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
            && (Arc::ptr_eq(&self.fields, &other.fields) || self.fields == other.fields)
    }
}

/// Reads a field the way indexing a [`Value`] does: `Null` when it is absent.
impl Index<&str> for SharedEntity {
    type Output = Value;

    fn index(&self, field: &str) -> &Value {
        static NULL: Value = Value::Null;
        self.field(field).unwrap_or(&NULL)
    }
}

/// Field access shared by plain values and [`SharedEntity`] copies.
pub trait EntityFields {
    /// The top-level field `name`, if the entity is an object that has it.
    fn field(&self, name: &str) -> Option<&Value>;

    /// The whole entity. Borrowed whenever it can be.
    fn whole(&self) -> Cow<'_, Value>;
}

impl EntityFields for Value {
    fn field(&self, name: &str) -> Option<&Value> {
        self.get(name)
    }

    fn whole(&self) -> Cow<'_, Value> {
        Cow::Borrowed(self)
    }
}

impl EntityFields for SharedEntity {
    fn field(&self, name: &str) -> Option<&Value> {
        if name == VERSION_FIELD {
            self.version.as_ref()
        } else {
            self.fields.get(name)
        }
    }

    fn whole(&self) -> Cow<'_, Value> {
        match self.version {
            None => Cow::Borrowed(&self.fields),
            Some(_) => Cow::Owned(self.to_value()),
        }
    }
}

/// The value at `path`, field names from the top of `entity`; the whole entity
/// for an empty path. `None` as soon as a step is missing.
pub fn lookup<'a, E, I, S>(entity: &'a E, path: I) -> Option<Cow<'a, Value>>
where
    E: EntityFields + ?Sized,
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut path = path.into_iter();
    let Some(first) = path.next() else {
        return Some(entity.whole());
    };
    let mut current = entity.field(first.as_ref())?;
    for segment in path {
        current = current.get(segment.as_ref())?;
    }
    Some(Cow::Borrowed(current))
}

/// Take `_version` out of `value`, if it is an object that has one.
pub(crate) fn split_version(mut value: Value) -> (Value, Option<Value>) {
    let version = match &mut value {
        Value::Object(map) => map.remove(VERSION_FIELD),
        _ => None,
    };
    (value, version)
}

/// Put `version` back into `fields` as `_version`. Only an object takes one.
pub(crate) fn with_version(mut fields: Value, version: Option<Value>) -> Value {
    if let (Value::Object(map), Some(version)) = (&mut fields, version) {
        map.insert(VERSION_FIELD.to_string(), version);
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn it_reads_compares_and_rebuilds_as_the_whole_entity() {
        let whole = json!({"a": 1, "_seq": "1:0", "_version": "e:7", "nested": {"b": [1, 2]}});
        let entity = SharedEntity::new(whole.clone());
        assert_eq!(entity.version(), Some(&json!("e:7")));
        assert!(entity.fields().get(VERSION_FIELD).is_none());
        assert_eq!(entity.field("_version"), Some(&json!("e:7")));
        assert_eq!(entity["a"], 1);
        assert_eq!(entity["missing"], Value::Null);
        assert_eq!(
            lookup(&entity, ["nested", "b"]).as_deref(),
            Some(&json!([1, 2]))
        );
        assert_eq!(
            lookup(&entity, ["_version"]).as_deref(),
            Some(&json!("e:7"))
        );
        assert_eq!(lookup(&entity, Vec::<&str>::new()).as_deref(), Some(&whole));
        assert_eq!(entity.to_value(), whole);
        assert_eq!(
            serde_json::to_string(&entity.clone().into_value()).unwrap(),
            serde_json::to_string(&whole).unwrap(),
            "the rebuilt entity serializes byte for byte as the original"
        );
        assert_eq!(entity, SharedEntity::new(whole));
    }

    #[test]
    fn copies_that_differ_only_in_version_are_not_equal() {
        let list = SharedEntity::new(json!({"a": 1, "_version": "e:1"}));
        let (fields, _) = list.clone().into_parts();
        let state = SharedEntity::from_parts(fields, Some(json!("e:2")));
        assert!(state.shares_fields_with(&list));
        assert_ne!(state, list);
        assert_eq!(state.to_value(), json!({"a": 1, "_version": "e:2"}));
    }

    #[test]
    fn a_value_that_is_not_an_object_has_no_version() {
        let entity = SharedEntity::new(json!([1, 2, 3]));
        assert_eq!(entity.version(), None);
        assert_eq!(entity.field("_version"), None);
        assert_eq!(entity.into_value(), json!([1, 2, 3]));
    }
}
