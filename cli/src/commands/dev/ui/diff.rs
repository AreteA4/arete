//! What a rebuild changed in the protocol: steps, machine states,
//! transitions (machine edges), reactions and events added, removed or
//! changed, for `build.succeeded` and the dev UI's change highlights.
//!
//! Edges come from the build's `machine-map.json` when there is one (it has
//! derivation edges too), else from the steps' `transitions`. An edge into a
//! machine from no state (a create step's start) has `from: null`, as the
//! UI's graph has it.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

/// A spec's comparable parts.
#[derive(Debug, Default)]
struct Parts {
    steps: BTreeMap<String, Value>,
    states: BTreeMap<(String, String), Value>,
    /// (machine, from (none for a start edge), to, via).
    edges: BTreeSet<(String, Option<String>, String, String)>,
    reactions: BTreeMap<String, Value>,
    events: BTreeMap<String, Value>,
}

fn object_entries(value: &Value) -> impl Iterator<Item = (&String, &Value)> {
    value.as_object().into_iter().flatten()
}

fn via(edge: &Value) -> String {
    if let Some(step) = edge["via"]["step"].as_str() {
        format!("step:{step}")
    } else if let Some(derive) = edge["via"]["derive"].as_u64() {
        format!("derive:{derive}")
    } else {
        "unknown".into()
    }
}

fn parts(spec: &Value, machine_map: Option<&Value>) -> Parts {
    let mut parts = Parts::default();
    for (name, step) in object_entries(&spec["steps"]) {
        parts.steps.insert(name.clone(), step.clone());
    }
    for machine in ["instance", "participant"] {
        for (name, state) in object_entries(&spec["machines"][machine]["states"]) {
            parts
                .states
                .insert((machine.to_string(), name.clone()), state.clone());
        }
    }
    match machine_map.filter(|map| map["machines"].is_object()) {
        Some(map) => {
            for (machine, body) in object_entries(&map["machines"]) {
                for edge in body["edges"].as_array().into_iter().flatten() {
                    parts.edges.insert((
                        machine.clone(),
                        edge["from"].as_str().map(str::to_string),
                        edge["to"].as_str().unwrap_or_default().to_string(),
                        via(edge),
                    ));
                }
            }
        }
        None => {
            for (name, step) in &parts.steps {
                let mut starts_instance = false;
                for transition in step["transitions"].as_array().into_iter().flatten() {
                    let machine = transition["machine"].as_str().unwrap_or("instance");
                    let to = transition["to"].as_str().unwrap_or_default();
                    let froms: Vec<&str> = transition["from"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect();
                    if froms.is_empty() {
                        starts_instance |= machine == "instance";
                        parts.edges.insert((
                            machine.to_string(),
                            None,
                            to.to_string(),
                            format!("step:{name}"),
                        ));
                    }
                    for from in froms.into_iter().filter(|from| *from != to) {
                        parts.edges.insert((
                            machine.to_string(),
                            Some(from.to_string()),
                            to.to_string(),
                            format!("step:{name}"),
                        ));
                    }
                }
                // A create step starts an instance in the initial state.
                if step["scope"] == "create" && !starts_instance {
                    if let Some(initial) = spec["machines"]["instance"]["initial"].as_str() {
                        parts.edges.insert((
                            "instance".to_string(),
                            None,
                            initial.to_string(),
                            format!("step:{name}"),
                        ));
                    }
                }
            }
        }
    }
    for reaction in spec["reactions"].as_array().into_iter().flatten() {
        if let Some(id) = reaction["id"].as_str() {
            parts.reactions.insert(id.to_string(), reaction.clone());
        }
    }
    for (name, event) in object_entries(&spec["events"]) {
        parts.events.insert(name.clone(), event.clone());
    }
    parts
}

/// Keys whose values differ between two JSON objects.
fn changed_fields(before: &Value, after: &Value) -> Vec<String> {
    let mut keys: BTreeSet<&String> = BTreeSet::new();
    keys.extend(
        before
            .as_object()
            .into_iter()
            .flat_map(|object| object.keys()),
    );
    keys.extend(
        after
            .as_object()
            .into_iter()
            .flat_map(|object| object.keys()),
    );
    keys.into_iter()
        .filter(|key| before.get(key.as_str()) != after.get(key.as_str()))
        .cloned()
        .collect()
}

fn keyed<K: Ord + Clone>(
    before: &BTreeMap<K, Value>,
    after: &BTreeMap<K, Value>,
    show: impl Fn(&K) -> Value,
    with_fields: bool,
) -> Value {
    let added: Vec<Value> = after
        .keys()
        .filter(|key| !before.contains_key(*key))
        .map(&show)
        .collect();
    let removed: Vec<Value> = before
        .keys()
        .filter(|key| !after.contains_key(*key))
        .map(&show)
        .collect();
    let changed: Vec<Value> = after
        .iter()
        .filter_map(|(key, value)| {
            let old = before.get(key)?;
            (old != value).then(|| {
                let mut entry = show(key);
                if with_fields {
                    if let Some(object) = entry.as_object_mut() {
                        object.insert("fields".into(), json!(changed_fields(old, value)));
                    }
                }
                entry
            })
        })
        .collect();
    json!({ "added": added, "removed": removed, "changed": changed })
}

fn is_empty(section: &Value) -> bool {
    ["added", "removed", "changed"]
        .iter()
        .all(|key| section[key].as_array().is_none_or(Vec::is_empty))
}

/// The difference between two builds. `before` is `None` for the first
/// build of a session, which is reported as everything added.
pub fn spec_diff(
    before: Option<(&Value, Option<&Value>)>,
    after: (&Value, Option<&Value>),
) -> Value {
    let old = before
        .map(|(spec, map)| parts(spec, map))
        .unwrap_or_default();
    let new = parts(after.0, after.1);
    let steps = keyed(&old.steps, &new.steps, |name| json!({ "name": name }), true);
    let states = keyed(
        &old.states,
        &new.states,
        |(machine, name)| json!({ "machine": machine, "name": name }),
        false,
    );
    let edge = |(machine, from, to, via): &(String, Option<String>, String, String)| json!({ "machine": machine, "from": from, "to": to, "via": via });
    let transitions = json!({
        "added": new.edges.difference(&old.edges).map(edge).collect::<Vec<_>>(),
        "removed": old.edges.difference(&new.edges).map(edge).collect::<Vec<_>>(),
        "changed": [],
    });
    let reactions = keyed(
        &old.reactions,
        &new.reactions,
        |id| json!({ "id": id }),
        true,
    );
    let events = keyed(
        &old.events,
        &new.events,
        |name| json!({ "name": name }),
        false,
    );
    let unchanged = [&steps, &states, &transitions, &reactions, &events]
        .iter()
        .all(|section| is_empty(section))
        && before.is_some_and(|(spec, _)| spec == after.0);
    json!({
        "initial": before.is_none(),
        "unchanged": unchanged,
        "steps": steps,
        "states": states,
        "transitions": transitions,
        "reactions": reactions,
        "events": events,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> Value {
        json!({
            "name": "jar",
            "machines": { "instance": { "initial": "open", "states": {
                "open": { "kind": "recorded", "terminal": false },
                "closed": { "kind": "recorded", "terminal": true }
            }}},
            "steps": {
                "open": { "scope": "create", "guards": [], "transitions": [] },
                "close": { "scope": "instance", "guards": [{ "code": "not_open", "expr": "true" }],
                           "transitions": [{ "machine": "instance", "from": ["open"], "to": "closed" }] }
            },
            "reactions": [{ "id": "announce", "action": { "kind": "emit", "event": "jar.closed" } }],
            "events": { "jar.closed": "instance" }
        })
    }

    #[test]
    fn the_first_build_adds_everything() {
        let spec = spec();
        let diff = spec_diff(None, (&spec, None));
        assert_eq!(diff["initial"], true);
        assert_eq!(diff["steps"]["added"].as_array().unwrap().len(), 2);
        // The create step's start edge has no source state: null, not "".
        assert_eq!(
            diff["transitions"]["added"],
            json!([
                { "machine": "instance", "from": null, "to": "open", "via": "step:open" },
                { "machine": "instance", "from": "open", "to": "closed", "via": "step:close" }
            ])
        );
        assert_eq!(diff["unchanged"], false);
    }

    #[test]
    fn reports_added_removed_and_changed_parts() {
        let before = spec();
        let mut after = spec();
        after["machines"]["instance"]["states"]["paused"] =
            json!({ "kind": "recorded", "terminal": false });
        after["steps"]["pause"] = json!({ "scope": "instance", "guards": [],
            "transitions": [{ "machine": "instance", "from": ["open"], "to": "paused" }] });
        after["steps"]["close"]["guards"] = json!([]);
        after["steps"]["close"]["transitions"] =
            json!([{ "machine": "instance", "from": ["open", "paused"], "to": "closed" }]);
        after["reactions"] = json!([]);
        let diff = spec_diff(Some((&before, None)), (&after, None));
        assert_eq!(diff["steps"]["added"], json!([{ "name": "pause" }]));
        assert_eq!(
            diff["steps"]["changed"],
            json!([{ "name": "close", "fields": ["guards", "transitions"] }])
        );
        assert_eq!(
            diff["states"]["added"],
            json!([{ "machine": "instance", "name": "paused" }])
        );
        assert_eq!(diff["transitions"]["added"].as_array().unwrap().len(), 2);
        assert_eq!(diff["reactions"]["removed"], json!([{ "id": "announce" }]));
        assert_eq!(diff["unchanged"], false);

        let same = spec_diff(Some((&before, None)), (&before, None));
        assert_eq!(same["unchanged"], true);
    }

    #[test]
    fn uses_machine_map_edges_when_present() {
        let spec = spec();
        let before_map = json!({ "machines": { "instance": { "edges": [
            { "from": "open", "to": "closed", "via": { "step": "close" } }
        ]}}});
        let after_map = json!({ "machines": { "instance": { "edges": [
            { "from": null, "to": "open", "via": { "step": "open" } },
            { "from": "open", "to": "closed", "via": { "step": "close" } },
            { "from": "open", "to": "closed", "via": { "derive": 0 } }
        ]}}});
        let diff = spec_diff(Some((&spec, Some(&before_map))), (&spec, Some(&after_map)));
        assert_eq!(
            diff["transitions"]["added"],
            json!([
                { "machine": "instance", "from": null, "to": "open", "via": "step:open" },
                { "machine": "instance", "from": "open", "to": "closed", "via": "derive:0" }
            ])
        );
    }
}
