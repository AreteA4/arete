//! Protocol summaries for `explore_protocol` and `a4 explore protocol`.
//!
//! A protocol install descriptor (`GET /api/registry/protocols/{ref}/install`)
//! is the resolver's protocol entry: the package release, the
//! `arete.protocol-spec/v1` document, the deployment its client calls, and
//! the stacks and programs it hoists. The summary keeps what an app or agent
//! needs to use the protocol: machines and their states, each step's scope,
//! authority, access and rejection codes, events, metadata fields, and the
//! hoisted stacks and programs. It works on raw JSON so it never lags the
//! spec model.

use serde_json::{json, Map, Value};

/// The access rejection codes every prepare may return, beside a step's own
/// guard codes.
pub const ACCESS_REJECTION_CODES: [&str; 3] = [
    "step_requires_secret_key",
    "step_not_in_session_scope",
    "step_not_client_callable",
];

/// The state rejection codes every instance or participant step may return:
/// the instance, or the preparing wallet's participant, is in a terminal
/// state. `detail.state` names it.
pub const STATE_REJECTION_CODES: [&str; 2] = ["instance_terminal", "participant_terminal"];

/// Summarise a protocol install descriptor. The spec may be absent when the
/// descriptor names only its hash; the summary then says where to read it.
pub fn protocol_install_summary(install: &Value) -> Value {
    let protocol = &install["protocol"];
    let mut summary = match protocol.get("protocolSpec").filter(|spec| spec.is_object()) {
        Some(spec) => protocol_spec_summary(spec),
        None => json!({
            "spec": format!(
                "not inlined; read it with resolve_artifact kind `protocol-spec` hash `{}`",
                protocol["protocolSpecHash"].as_str().unwrap_or_default()
            )
        }),
    };
    let object = summary.as_object_mut().expect("summary is an object");
    let mut identity = Map::new();
    for key in ["package", "version", "packageReleaseHash"] {
        if let Some(value) = install.get(key) {
            identity.insert(key.into(), value.clone());
        }
    }
    for key in ["protocolReleaseHash", "protocolSpecHash"] {
        if let Some(value) = protocol.get(key) {
            identity.insert(key.into(), value.clone());
        }
    }
    object.insert("release".into(), Value::Object(identity));
    if let Some(deployment) = protocol.get("deployment").filter(|value| value.is_object()) {
        object.insert(
            "deployment".into(),
            json!({
                "deploymentId": deployment["deploymentId"],
                "access": deployment["access"],
            }),
        );
    }
    if let Some(live) = protocol.get("liveStack").filter(|value| value.is_object()) {
        object.insert("liveStack".into(), live.clone());
    }
    let hoisted_stacks = install["stacks"]
        .as_array()
        .map(|stacks| {
            stacks
                .iter()
                .map(|stack| {
                    json!({
                        "key": stack["alias"],
                        "package": stack["package"],
                        "version": stack["version"],
                        "stackManifestHash": stack["stackManifestHash"],
                        "programs": stack["programs"]
                            .as_array()
                            .map(|programs| programs.iter().map(program_identity).collect::<Vec<_>>())
                            .unwrap_or_default(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    object.insert("hoistedStacks".into(), Value::Array(hoisted_stacks));
    let direct_programs = install["programs"]
        .as_array()
        .map(|programs| programs.iter().map(program_identity).collect::<Vec<_>>())
        .unwrap_or_default();
    object.insert("directPrograms".into(), Value::Array(direct_programs));
    summary
}

fn program_identity(program: &Value) -> Value {
    json!({
        "installName": program["installName"],
        "programId": program["definition"]["programId"],
        "package": program["programPackage"]["package"],
        "version": program["programPackage"]["version"],
    })
}

/// Summarise an `arete.protocol-spec/v1` document.
pub fn protocol_spec_summary(spec: &Value) -> Value {
    let machines = ["instance", "participant"]
        .into_iter()
        .filter_map(|kind| {
            let machine = spec["machines"]
                .get(kind)
                .filter(|value| value.is_object())?;
            let states = machine["states"]
                .as_object()
                .map(|states| {
                    states
                        .iter()
                        .map(|(name, state)| {
                            let mut entry = json!({
                                "name": name,
                                "kind": state["kind"],
                                "terminal": state["terminal"],
                            });
                            copy_optional(state, &mut entry, "description");
                            entry
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Some((
                kind.to_string(),
                json!({
                    "initial": machine["initial"],
                    "states": states,
                    "derive": machine["derive"],
                }),
            ))
        })
        .collect::<Map<_, _>>();

    let steps = spec["steps"]
        .as_object()
        .map(|steps| {
            steps
                .iter()
                .map(|(name, step)| step_summary(name, step))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let events = spec["events"]
        .as_object()
        .map(|events| {
            events
                .iter()
                .map(|(name, event)| {
                    let mut entry = json!({ "name": name, "machine": event["machine"] });
                    copy_optional(event, &mut entry, "description");
                    entry
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let metadata = ["instance", "participant"]
        .into_iter()
        .filter_map(|kind| {
            let schema = spec["metadata"]
                .get(kind)
                .filter(|value| value.is_object())?;
            let fields = schema["fields"]
                .as_object()
                .map(|fields| {
                    fields
                        .iter()
                        .map(|(name, field)| {
                            json!({
                                "name": name,
                                "type": field_type(&field["schema"]),
                                "writer": field["writer"],
                                "filterable": field["filterable"],
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Some((
                kind.to_string(),
                json!({ "fields": fields, "maxBytes": schema["maxBytes"] }),
            ))
        })
        .collect::<Map<_, _>>();

    let roles = spec["roles"]
        .as_object()
        .map(|roles| {
            roles
                .iter()
                .map(|(name, role)| {
                    let mut entry = json!({ "name": name });
                    copy_optional(role, &mut entry, "description");
                    entry
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let stacks = spec["stacks"]
        .as_object()
        .map(|stacks| {
            stacks
                .iter()
                .map(|(key, stack)| {
                    let mut entry = json!({
                        "key": key,
                        "stackManifestHash": stack["stackManifestHash"],
                    });
                    copy_optional(stack, &mut entry, "package");
                    entry
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let programs = spec["programs"]
        .as_object()
        .map(|programs| {
            programs
                .iter()
                .map(|(key, program)| match program["kind"].as_str() {
                    Some("builtin") => json!({ "key": key, "builtin": program["builtin"] }),
                    _ => json!({ "key": key, "programId": program["programId"] }),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut summary = json!({
        "kind": "protocol",
        "name": spec["name"],
        "version": spec["version"],
        "instanceVisibility": spec["instanceVisibility"],
        "machines": machines,
        "steps": steps,
        "accessRejectionCodes": ACCESS_REJECTION_CODES,
        "stateRejectionCodes": STATE_REJECTION_CODES,
        "events": events,
        "metadata": metadata,
        "roles": roles,
        "params": object_fields(&spec["params"]),
        "stacks": stacks,
        "programs": programs,
    });
    copy_optional(spec, &mut summary, "description");
    summary
}

fn step_summary(name: &str, step: &Value) -> Value {
    let authority = &step["authority"];
    let authority_name = match authority["kind"].as_str() {
        Some("role") => format!("role:{}", authority["role"].as_str().unwrap_or_default()),
        Some(kind) => kind.to_string(),
        None => String::new(),
    };
    let client_callable = matches!(
        authority["kind"].as_str(),
        Some("creator" | "participant" | "role")
    );
    let guards = step["guards"]
        .as_array()
        .map(|guards| {
            guards
                .iter()
                .map(|guard| {
                    let mut entry = json!({ "code": guard["code"], "expr": guard["expr"] });
                    copy_optional(guard, &mut entry, "message");
                    entry
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut entry = json!({
        "name": name,
        "scope": step["scope"],
        "authority": authority_name,
        "clientCallable": client_callable,
        "access": step.get("access").cloned().unwrap_or(Value::Null),
        "args": object_fields(&step["args"]),
        "guards": guards,
        "transitions": step["transitions"],
        "build": step["build"]["kind"],
    });
    copy_optional(step, &mut entry, "title");
    copy_optional(step, &mut entry, "description");
    entry
}

/// The properties of an object schema: name, type and whether required.
fn object_fields(schema: &Value) -> Vec<Value> {
    let required = schema["required"]
        .as_array()
        .map(|required| {
            required
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    schema["properties"]
        .as_object()
        .map(|properties| {
            properties
                .iter()
                .map(|(name, field)| {
                    json!({
                        "name": name,
                        "type": field_type(field),
                        "required": required.contains(&name.as_str()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `string`, `string(u64)`, `array<string(pubkey)>`, … for one field schema.
fn field_type(field: &Value) -> String {
    let base = field["type"].as_str().unwrap_or("unknown");
    match base {
        "array" => format!("array<{}>", field_type(&field["items"])),
        _ => match field["format"].as_str() {
            Some(format) => format!("{base}({format})"),
            None => match field["enum"].as_array() {
                Some(values) => format!(
                    "{base}({})",
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("|")
                ),
                None => base.to_string(),
            },
        },
    }
}

fn copy_optional(from: &Value, to: &mut Value, key: &str) {
    if let Some(value) = from.get(key).filter(|value| !value.is_null()) {
        to[key] = value.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_raise() -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-vectors/protocol/token-raise.protocol-spec.json");
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn the_spec_summary_lists_steps_with_access_and_rejection_codes() {
        let summary = protocol_spec_summary(&token_raise());
        assert_eq!(summary["name"], "tokenRaise");
        let steps = summary["steps"].as_array().unwrap();
        let contribute = steps
            .iter()
            .find(|step| step["name"] == "contribute")
            .unwrap();
        assert_eq!(contribute["access"], "public");
        assert_eq!(contribute["scope"], "participant");
        assert_eq!(contribute["clientCallable"], true);
        assert_eq!(
            contribute["guards"]
                .as_array()
                .unwrap()
                .iter()
                .map(|guard| guard["code"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "raise_not_open",
                "deadline_passed",
                "wallet_cap_exceeded",
                "hard_cap_exceeded"
            ]
        );
        assert_eq!(contribute["args"][0]["type"], "string(u64)");
        let finalize = steps
            .iter()
            .find(|step| step["name"] == "finalize")
            .unwrap();
        assert_eq!(finalize["clientCallable"], false);
        assert_eq!(finalize["access"], Value::Null);
        assert_eq!(finalize["authority"], "squadsProposer");
        let create = steps.iter().find(|step| step["name"] == "create").unwrap();
        assert_eq!(create["access"], "secret");
        assert_eq!(create["args"][2]["name"], "quoteMint");
        assert_eq!(create["args"][2]["type"], "string(pubkey)");
        assert_eq!(summary["machines"]["instance"]["initial"], "open");
        assert_eq!(summary["events"].as_array().unwrap().len(), 3);
        assert_eq!(summary["metadata"]["instance"]["fields"][0]["name"], "name");
        assert_eq!(summary["stacks"][0]["key"], "meteoraPresale");
        assert_eq!(summary["accessRejectionCodes"].as_array().unwrap().len(), 3);
        assert_eq!(
            summary["stateRejectionCodes"],
            json!(["instance_terminal", "participant_terminal"])
        );
    }

    #[test]
    fn the_install_summary_adds_release_deployment_and_hoisted_packages() {
        let install = json!({
            "kind": "protocol",
            "alias": "token-raise",
            "package": "token-raise",
            "version": "1.0.0",
            "packageReleaseHash": "arete:registry-package-release:v2:sha256:00",
            "protocol": {
                "protocolReleaseHash": "arete:h1:protocol-release:sha256:11",
                "protocolSpecHash": "arete:h1:protocol-spec:sha256:22",
                "protocolSpec": token_raise(),
                "deployment": {
                    "deploymentId": "d1",
                    "access": "public",
                    "apiUrl": "https://api.example.test/api",
                    "sessionEndpoint": "https://api.example.test/ws/sessions"
                }
            },
            "stacks": [{"alias": "meteoraPresale", "package": "meteora-presale", "version": "1.0.0", "stackManifestHash": "h", "programs": []}],
            "programs": []
        });
        let summary = protocol_install_summary(&install);
        assert_eq!(
            summary["deployment"],
            json!({"deploymentId": "d1", "access": "public"})
        );
        assert_eq!(
            summary["release"]["protocolSpecHash"],
            "arete:h1:protocol-spec:sha256:22"
        );
        assert_eq!(summary["hoistedStacks"][0]["key"], "meteoraPresale");
        assert!(summary.get("apiUrl").is_none());

        let mut by_hash = install.clone();
        by_hash["protocol"]
            .as_object_mut()
            .unwrap()
            .remove("protocolSpec");
        let summary = protocol_install_summary(&by_hash);
        assert!(summary["spec"].as_str().unwrap().contains("protocol-spec"));
    }
}
