//! A small structural walker over `serde_json::Value`.
//!
//! Each contract validator checks a document with these helpers before it is
//! deserialized into a typed model. The TypeScript package mirrors the same
//! check order (unknown fields first, then declared fields in order), so both
//! languages report the same first error for a document with one defect.

use serde_json::{Map, Value};

use super::error::{ProtocolContractError, ProtocolResult};

pub(crate) const MAX_IDENTIFIER_BYTES: usize = 64;
pub(crate) const MAX_DESCRIPTION_BYTES: usize = 1024;
pub(crate) const MAX_EXPRESSION_BYTES: usize = 2048;

pub(crate) fn child(path: &str, key: &str) -> String {
    let escaped = key.replace('~', "~0").replace('/', "~1");
    format!("{path}/{escaped}")
}

pub(crate) fn index(path: &str, index: usize) -> String {
    format!("{path}/{index}")
}

pub(crate) fn err(
    code: &'static str,
    path: &str,
    message: impl Into<String>,
) -> ProtocolContractError {
    ProtocolContractError::new(code, path, message)
}

/// Reject floats and unsafe integers anywhere in a document. Contracts carry integer base units
/// as decimal strings and small counts as safe JSON integers only.
pub(crate) fn reject_floats(value: &Value, path: &str) -> ProtocolResult<()> {
    match value {
        Value::Number(number) => {
            if number.is_f64() {
                return Err(err(
                    "float-not-allowed",
                    path,
                    "floating-point numbers are not allowed",
                ));
            }
            const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
            if number.as_u64().is_some_and(|n| n > MAX_SAFE_INTEGER)
                || number
                    .as_i64()
                    .is_some_and(|n| n < -(MAX_SAFE_INTEGER as i64))
            {
                return Err(err(
                    "unsafe-json-integer",
                    path,
                    "integer exceeds the safe JSON range",
                ));
            }
            Ok(())
        }
        Value::Array(values) => values
            .iter()
            .enumerate()
            .try_for_each(|(i, value)| reject_floats(value, &index(path, i))),
        Value::Object(values) => {
            let mut keys: Vec<&String> = values.keys().collect();
            keys.sort();
            keys.into_iter()
                .try_for_each(|key| reject_floats(&values[key], &child(path, key)))
        }
        _ => Ok(()),
    }
}

pub(crate) struct Obj<'a> {
    pub path: String,
    map: &'a Map<String, Value>,
}

impl<'a> Obj<'a> {
    /// Open an object, rejecting the lexicographically first key that is not
    /// in `allowed`.
    pub fn new(value: &'a Value, path: &str, allowed: &[&str]) -> ProtocolResult<Self> {
        let map = value
            .as_object()
            .ok_or_else(|| err("invalid-type", path, "expected an object"))?;
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        if let Some(unknown) = keys
            .into_iter()
            .find(|key| !allowed.contains(&key.as_str()))
        {
            return Err(err(
                "unknown-field",
                &child(path, unknown),
                format!("unknown field '{unknown}'"),
            ));
        }
        Ok(Self {
            path: path.to_string(),
            map,
        })
    }

    pub fn path_of(&self, key: &str) -> String {
        child(&self.path, key)
    }

    /// A required, non-null field.
    pub fn req(&self, key: &str) -> ProtocolResult<&'a Value> {
        match self.map.get(key) {
            None => Err(err(
                "missing-field",
                &self.path_of(key),
                format!("missing field '{key}'"),
            )),
            Some(Value::Null) => Err(err(
                "null-not-allowed",
                &self.path_of(key),
                format!("field '{key}' must not be null"),
            )),
            Some(value) => Ok(value),
        }
    }

    /// An optional field. Absent means "not set"; `null` is never accepted, so
    /// every document has exactly one canonical form.
    pub fn opt(&self, key: &str) -> ProtocolResult<Option<&'a Value>> {
        match self.map.get(key) {
            None => Ok(None),
            Some(Value::Null) => Err(err(
                "null-not-allowed",
                &self.path_of(key),
                format!("optional field '{key}' must be omitted, not null"),
            )),
            Some(value) => Ok(Some(value)),
        }
    }

    /// A field that must be present and may be `null`.
    pub fn nullable(&self, key: &str) -> ProtocolResult<Option<&'a Value>> {
        match self.map.get(key) {
            None => Err(err(
                "missing-field",
                &self.path_of(key),
                format!("missing field '{key}'"),
            )),
            Some(Value::Null) => Ok(None),
            Some(value) => Ok(Some(value)),
        }
    }

    pub fn has(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }
}

pub(crate) fn as_str<'a>(value: &'a Value, path: &str) -> ProtocolResult<&'a str> {
    value
        .as_str()
        .ok_or_else(|| err("invalid-type", path, "expected a string"))
}

pub(crate) fn as_bool(value: &Value, path: &str) -> ProtocolResult<bool> {
    value
        .as_bool()
        .ok_or_else(|| err("invalid-type", path, "expected a boolean"))
}

pub(crate) fn as_array<'a>(
    value: &'a Value,
    path: &str,
    max: usize,
) -> ProtocolResult<&'a Vec<Value>> {
    let values = value
        .as_array()
        .ok_or_else(|| err("invalid-type", path, "expected an array"))?;
    if values.len() > max {
        return Err(err(
            "array-too-large",
            path,
            format!("at most {max} entries are allowed"),
        ));
    }
    Ok(values)
}

/// A map whose keys are validated with `key_check`, returned in sorted order.
pub(crate) fn as_map<'a>(
    value: &'a Value,
    path: &str,
    max: usize,
    key_check: fn(&str, &str) -> ProtocolResult<()>,
) -> ProtocolResult<Vec<(&'a String, &'a Value, String)>> {
    let map = value
        .as_object()
        .ok_or_else(|| err("invalid-type", path, "expected an object"))?;
    if map.len() > max {
        return Err(err(
            "map-too-large",
            path,
            format!("at most {max} entries are allowed"),
        ));
    }
    let mut entries: Vec<(&String, &Value)> = map.iter().collect();
    entries.sort_by(|left, right| left.0.cmp(right.0));
    entries
        .into_iter()
        .map(|(key, value)| {
            let entry_path = child(path, key);
            key_check(key, &entry_path)?;
            Ok((key, value, entry_path))
        })
        .collect()
}

pub(crate) fn as_integer(value: &Value, path: &str, min: i64, max: i64) -> ProtocolResult<i64> {
    let integer = value
        .as_i64()
        .ok_or_else(|| err("invalid-type", path, "expected an integer"))?;
    if integer < min || integer > max {
        return Err(err(
            "integer-out-of-range",
            path,
            format!("expected an integer in {min}..={max}"),
        ));
    }
    Ok(integer)
}

pub(crate) fn bounded_str<'a>(
    value: &'a Value,
    path: &str,
    max_bytes: usize,
) -> ProtocolResult<&'a str> {
    let text = as_str(value, path)?;
    if text.len() > max_bytes {
        return Err(err(
            "string-too-long",
            path,
            format!("at most {max_bytes} bytes are allowed"),
        ));
    }
    Ok(text)
}

pub(crate) fn description(value: &Value, path: &str) -> ProtocolResult<()> {
    let text = bounded_str(value, path, MAX_DESCRIPTION_BYTES)?;
    if text.chars().any(|c| c.is_control() && c != '\n') {
        return Err(err(
            "invalid-value",
            path,
            "control characters are not allowed",
        ));
    }
    Ok(())
}

pub(crate) fn one_of<'a>(
    value: &'a Value,
    path: &str,
    allowed: &[&str],
) -> ProtocolResult<&'a str> {
    let text = as_str(value, path)?;
    if !allowed.contains(&text) {
        return Err(err(
            "invalid-value",
            path,
            format!("expected one of {}", allowed.join(", ")),
        ));
    }
    Ok(text)
}

pub(crate) fn schema_tag(value: &Value, path: &str, expected: &str) -> ProtocolResult<()> {
    let text = as_str(value, path)?;
    if text != expected {
        return Err(err(
            "invalid-schema-tag",
            path,
            format!("expected '{expected}'"),
        ));
    }
    Ok(())
}

fn ascii_matches(text: &str, first: fn(u8) -> bool, rest: fn(u8) -> bool) -> bool {
    let bytes = text.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_IDENTIFIER_BYTES
        && first(bytes[0])
        && bytes[1..].iter().all(|byte| rest(*byte))
}

fn is_lower(byte: u8) -> bool {
    byte.is_ascii_lowercase()
}

fn is_alpha(byte: u8) -> bool {
    byte.is_ascii_alphabetic()
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_alnum(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}

fn is_snake(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
}

/// `^[A-Za-z][A-Za-z0-9_]{0,63}$`: steps, reads, roles, program aliases,
/// argument and account names (IDL names may be snake_case or camelCase).
pub(crate) fn identifier(text: &str, path: &str) -> ProtocolResult<()> {
    if ascii_matches(text, is_alpha, is_word) {
        Ok(())
    } else {
        Err(err(
            "invalid-identifier",
            path,
            format!("'{text}' must match ^[A-Za-z][A-Za-z0-9_]{{0,63}}$"),
        ))
    }
}

/// `^[a-z][A-Za-z0-9]{0,63}$`: the protocol name, used as
/// `arete.protocols.<name>`.
pub(crate) fn protocol_name(text: &str, path: &str) -> ProtocolResult<()> {
    if ascii_matches(text, is_lower, is_alnum) {
        Ok(())
    } else {
        Err(err(
            "invalid-identifier",
            path,
            format!("'{text}' must match ^[a-z][A-Za-z0-9]{{0,63}}$"),
        ))
    }
}

/// `^[a-z][a-z0-9_]{0,63}$`: machine states and guard codes.
pub(crate) fn snake_identifier(text: &str, path: &str) -> ProtocolResult<()> {
    if ascii_matches(text, is_lower, is_snake) {
        Ok(())
    } else {
        Err(err(
            "invalid-identifier",
            path,
            format!("'{text}' must match ^[a-z][a-z0-9_]{{0,63}}$"),
        ))
    }
}

/// Dotted event names such as `raise.succeeded`: one to four segments, each
/// `[a-z][A-Za-z0-9_]*`, at most 64 bytes in total.
pub(crate) fn event_name(text: &str, path: &str) -> ProtocolResult<()> {
    let valid = !text.is_empty()
        && text.len() <= MAX_IDENTIFIER_BYTES
        && text.split('.').count() <= 4
        && text.split('.').all(|segment| {
            let bytes = segment.as_bytes();
            !bytes.is_empty() && is_lower(bytes[0]) && bytes[1..].iter().all(|b| is_word(*b))
        });
    if valid {
        Ok(())
    } else {
        Err(err(
            "invalid-identifier",
            path,
            format!("'{text}' must be dotted lower-camel segments"),
        ))
    }
}

pub(crate) fn key_identifier(key: &str, path: &str) -> ProtocolResult<()> {
    identifier(key, path)
}

pub(crate) fn key_snake(key: &str, path: &str) -> ProtocolResult<()> {
    snake_identifier(key, path)
}

pub(crate) fn key_event(key: &str, path: &str) -> ProtocolResult<()> {
    event_name(key, path)
}

pub(crate) fn identifier_value<'a>(value: &'a Value, path: &str) -> ProtocolResult<&'a str> {
    let text = as_str(value, path)?;
    identifier(text, path)?;
    Ok(text)
}

pub(crate) fn snake_value<'a>(value: &'a Value, path: &str) -> ProtocolResult<&'a str> {
    let text = as_str(value, path)?;
    snake_identifier(text, path)?;
    Ok(text)
}

/// An expression source string. The platform's expression checker parses and
/// type-checks it; contracts only bound its size and reject control
/// characters.
pub(crate) fn expression<'a>(value: &'a Value, path: &str) -> ProtocolResult<&'a str> {
    let text = as_str(value, path)?;
    if text.trim().is_empty() {
        return Err(err(
            "invalid-expression",
            path,
            "expressions must not be empty",
        ));
    }
    if text.len() > MAX_EXPRESSION_BYTES {
        return Err(err(
            "string-too-long",
            path,
            format!("expressions are at most {MAX_EXPRESSION_BYTES} bytes"),
        ));
    }
    if text.chars().any(|c| c.is_control() && c != '\n') {
        return Err(err(
            "invalid-expression",
            path,
            "control characters are not allowed in expressions",
        ));
    }
    Ok(text)
}

/// `^(0|[1-9][0-9]*)$` with an upper bound.
pub(crate) fn decimal(value: &Value, path: &str, max: u128) -> ProtocolResult<u128> {
    let text = as_str(value, path)?;
    let well_formed = !text.is_empty()
        && text.len() <= 39
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    let parsed = if well_formed {
        text.parse::<u128>().ok()
    } else {
        None
    };
    match parsed {
        Some(number) if number <= max => Ok(number),
        _ => Err(err(
            "invalid-decimal",
            path,
            format!("expected a canonical decimal string at most {max}"),
        )),
    }
}

pub(crate) fn pubkey<'a>(value: &'a Value, path: &str) -> ProtocolResult<&'a str> {
    let text = as_str(value, path)?;
    let decoded = bs58::decode(text).into_vec().ok();
    match decoded {
        Some(bytes) if bytes.len() == 32 && text.len() <= 44 => Ok(text),
        _ => Err(err(
            "invalid-pubkey",
            path,
            "expected a base58 32-byte public key",
        )),
    }
}

pub(crate) fn hash_id<K: crate::Kind>(
    value: &Value,
    path: &str,
) -> ProtocolResult<crate::HashId<K>> {
    let text = as_str(value, path)?;
    text.parse::<crate::HashId<K>>().map_err(|_| {
        err(
            "invalid-hash-id",
            path,
            format!("expected an arete:h1:{}:sha256 identifier", K::NAME),
        )
    })
}

/// A lowercase hex SHA-256 digest.
pub(crate) fn sha256_hex<'a>(value: &'a Value, path: &str) -> ProtocolResult<&'a str> {
    let text = as_str(value, path)?;
    if text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(text)
    } else {
        Err(err(
            "invalid-value",
            path,
            "expected a lowercase hex SHA-256 digest",
        ))
    }
}

/// `MAJOR.MINOR.PATCH` with an optional pre-release; no build metadata.
pub(crate) fn semver<'a>(value: &'a Value, path: &str) -> ProtocolResult<&'a str> {
    let text = as_str(value, path)?;
    let (core, pre) = match text.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (text, None),
    };
    let numeric = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'))
    };
    let parts: Vec<&str> = core.split('.').collect();
    let pre_ok = pre.is_none_or(|pre| {
        !pre.is_empty()
            && pre.split('.').all(|id| {
                !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
    });
    if text.len() <= 64 && parts.len() == 3 && parts.iter().all(|part| numeric(part)) && pre_ok {
        Ok(text)
    } else {
        Err(err(
            "invalid-semver",
            path,
            "expected MAJOR.MINOR.PATCH[-pre]",
        ))
    }
}

/// A list of unique strings, each validated by `check`.
pub(crate) fn unique_strings<'a>(
    value: &'a Value,
    path: &str,
    max: usize,
    check: fn(&'a Value, &str) -> ProtocolResult<&'a str>,
) -> ProtocolResult<Vec<&'a str>> {
    let values = as_array(value, path, max)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(values.len());
    for (i, item) in values.iter().enumerate() {
        let item_path = index(path, i);
        let text = check(item, &item_path)?;
        if !seen.insert(text) {
            return Err(err(
                "duplicate-entry",
                &item_path,
                format!("'{text}' is listed more than once"),
            ));
        }
        out.push(text);
    }
    Ok(out)
}
