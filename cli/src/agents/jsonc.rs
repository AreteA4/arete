//! Comment- and order-preserving JSON/JSONC editing on top of the
//! `jsonc-parser` CST. Used for every JSON config file `init` touches so a
//! user's key order, indentation and comments survive the upsert.

use anyhow::{anyhow, Result};
use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
use jsonc_parser::ParseOptions;
use serde_json::Value;

pub struct JsonDoc {
    root: CstRootNode,
}

impl JsonDoc {
    /// Parse JSON or JSONC text. Empty text parses to an empty document.
    pub fn parse(text: &str) -> Result<Self> {
        let root = CstRootNode::parse(text, &ParseOptions::default())
            .map_err(|error| anyhow!("invalid JSON: {error}"))?;
        Ok(Self { root })
    }

    /// The root object, replacing a non-object root.
    fn root_object(&self) -> CstObject {
        self.root.object_value_or_set()
    }

    /// Whether the root value is an object (or absent).
    pub fn root_is_object(&self) -> bool {
        match self.root.value() {
            None => true,
            Some(node) => node.as_object().is_some(),
        }
    }

    /// Value at `path` (each element a literal key), if present.
    pub fn get(&self, path: &[&str]) -> Option<Value> {
        let mut object = self.root.object_value()?;
        let (last, parents) = path.split_last()?;
        for key in parents {
            object = object.object_value(key)?;
        }
        node_to_value(&object.get(last)?.value()?)
    }

    /// Set `path` to `value`, creating intermediate objects and replacing
    /// non-object intermediates.
    pub fn set(&self, path: &[&str], value: &Value) {
        let mut object = self.root_object();
        let Some((last, parents)) = path.split_last() else {
            return;
        };
        for key in parents {
            object = object.object_value_or_set(key);
        }
        let input = to_input(value);
        match object.get(last) {
            Some(prop) => prop.set_value(input),
            None => {
                object.append(last, input);
            }
        }
    }

    /// Remove only the named leaf, retaining all unrelated syntax.
    pub fn remove(&self, path: &[&str]) {
        let Some(mut object) = self.root.object_value() else {
            return;
        };
        let Some((last, parents)) = path.split_last() else {
            return;
        };
        for key in parents {
            let Some(child) = object.object_value(key) else {
                return;
            };
            object = child;
        }
        if let Some(property) = object.get(last) {
            property.remove();
        }
    }

    /// Serialised text with a trailing newline.
    pub fn render(&self) -> String {
        let mut text = self.root.to_string();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text
    }
}

/// Convert a CST node to a `serde_json::Value` (the crate's own
/// `serde_json` feature is not enabled, so go through its value parser).
fn node_to_value(node: &jsonc_parser::cst::CstNode) -> Option<Value> {
    let text = node.to_string();
    let parsed = jsonc_parser::parse_to_value(&text, &ParseOptions::default()).ok()??;
    Some(json_value_to_serde(parsed))
}

fn json_value_to_serde(value: jsonc_parser::JsonValue<'_>) -> Value {
    use jsonc_parser::JsonValue;
    match value {
        JsonValue::Null => Value::Null,
        JsonValue::Boolean(b) => Value::Bool(b),
        JsonValue::Number(n) => serde_json::from_str(n).unwrap_or(Value::String(n.to_string())),
        JsonValue::String(s) => Value::String(s.into_owned()),
        JsonValue::Array(items) => {
            Value::Array(items.into_iter().map(json_value_to_serde).collect())
        }
        JsonValue::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (key.into_owned(), json_value_to_serde(value)))
                .collect(),
        ),
    }
}

fn to_input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(b) => CstInputValue::Bool(*b),
        Value::Number(n) => CstInputValue::Number(n.to_string()),
        Value::String(s) => CstInputValue::String(s.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(to_input).collect()),
        Value::Object(map) => CstInputValue::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), to_input(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_text_becomes_a_pretty_object() {
        let doc = JsonDoc::parse("").unwrap();
        assert!(doc.root_is_object());
        doc.set(
            &["mcpServers", "arete"],
            &json!({"command": "a4", "args": ["mcp"]}),
        );
        let text = doc.render();
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["mcpServers"]["arete"]["command"], "a4");
        assert!(text.ends_with("}\n"));
        assert!(
            text.contains("\n  \"mcpServers\""),
            "two-space indent: {text}"
        );
    }

    #[test]
    fn comments_and_other_keys_survive_and_updates_replace_in_place() {
        let source = r#"{
  // keep me
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "other": { "type": "local", "command": ["x"] }, // trailing
    "arete": { "type": "local", "command": ["old", "mcp"], "enabled": false }
  },
  "theme": "dark"
}
"#;
        let doc = JsonDoc::parse(source).unwrap();
        assert_eq!(doc.get(&["mcp", "arete", "enabled"]), Some(json!(false)));
        doc.set(
            &["mcp", "arete"],
            &json!({"type": "local", "command": ["a4", "mcp"], "enabled": true}),
        );
        doc.set(
            &["mcp", "arete-docs"],
            &json!({"type": "remote", "url": "u", "enabled": true}),
        );
        let text = doc.render();
        assert!(text.contains("// keep me"));
        assert!(text.contains("// trailing"));
        assert!(text.contains("\"theme\": \"dark\""));
        let schema_pos = text.find("$schema").unwrap();
        let mcp_pos = text.find("\"mcp\"").unwrap();
        let theme_pos = text.find("theme").unwrap();
        assert!(
            schema_pos < mcp_pos && mcp_pos < theme_pos,
            "key order preserved"
        );
        assert_eq!(doc.get(&["mcp", "arete", "enabled"]), Some(json!(true)));
        assert_eq!(doc.get(&["mcp", "arete-docs", "url"]), Some(json!("u")));
    }

    #[test]
    fn literal_dotted_keys_are_not_paths() {
        let doc = JsonDoc::parse("{}").unwrap();
        doc.set(&["amp.mcpServers", "arete"], &json!({"command": "a4"}));
        let parsed: Value = serde_json::from_str(&doc.render()).unwrap();
        assert_eq!(parsed["amp.mcpServers"]["arete"]["command"], "a4");
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(JsonDoc::parse("{ nope").is_err());
    }
}
