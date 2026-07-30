use serde::de::{self, Deserialize, Deserializer, Error as _, MapAccess, SeqAccess, Visitor};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::fmt;

use crate::{
    hash_canonical_payload, push_framed_bytes, require_profile, CanonicalizationProfile, HashError,
    HashId, Kind,
};

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const MAX_SAFE_INTEGER_DIGITS: &str = "9007199254740991";
const DUPLICATE_KEY_MARKER: &str = "__ARETE_DUPLICATE_KEY__:";
const UNSAFE_INTEGER_MARKER: &str = "__ARETE_UNSAFE_INTEGER__:";
const NON_FINITE_MARKER: &str = "__ARETE_NON_FINITE_NUMBER__";

pub fn hash_raw_bytes<K: Kind>(bytes: &[u8]) -> Result<HashId<K>, HashError> {
    require_profile::<K>(CanonicalizationProfile::RawBytesV1)?;
    Ok(hash_canonical_payload(bytes))
}

/// Parse JSON bytes without first converting through a JavaScript number.
///
/// This rejects malformed UTF-8, duplicate object keys, non-finite/out-of-range
/// numbers, and integer tokens outside JavaScript's inclusive safe range.
pub fn parse_json_bytes_strict(bytes: &[u8]) -> Result<Value, HashError> {
    reject_unsafe_integer_tokens(bytes)?;
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValue::deserialize(&mut deserializer)
        .map_err(|error| classify_json_error(error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| classify_json_error(error.to_string()))?;
    Ok(value.0)
}

/// RFC 8785 treats every number as an IEEE-754 double, but the Arete profile
/// rejects integer *tokens* outside the inclusive safe range before they can
/// be converted through a double. serde_json silently falls back to `f64` for
/// integer literals beyond `u64`/`i64`, so token-level validation must happen
/// lexically, before deserialization.
fn reject_unsafe_integer_tokens(bytes: &[u8]) -> Result<(), HashError> {
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                index = skip_string_token(bytes, index);
            }
            b'-' | b'0'..=b'9' => {
                if let Some((token_end, integer_digits)) = scan_number_token(bytes, index) {
                    if let Some(digits) = integer_digits {
                        let magnitude = digits.trim_start_matches('-');
                        let unsafe_integer = magnitude.len() > MAX_SAFE_INTEGER_DIGITS.len()
                            || (magnitude.len() == MAX_SAFE_INTEGER_DIGITS.len()
                                && magnitude > MAX_SAFE_INTEGER_DIGITS);
                        if unsafe_integer {
                            let token =
                                std::string::String::from_utf8_lossy(&bytes[index..token_end])
                                    .into_owned();
                            return Err(HashError::UnsafeJsonInteger(token));
                        }
                    }
                    index = token_end;
                } else {
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }
    Ok(())
}

/// Skip a string token starting at the opening quote. Escaped code units never
/// contain a raw `"` or `\` byte, so skipping `\` plus one byte is exact for
/// scanning; malformed escapes are rejected later by the real parser.
fn skip_string_token(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return index + 1,
            _ => index += 1,
        }
    }
    index
}

/// Scan a JSON number token starting at `start`. Returns the token end and,
/// for integer-form tokens (no fraction or exponent), the digit text. Returns
/// `None` when the bytes do not form a complete, well-delimited JSON number;
/// the real parser reports those as syntax errors.
fn scan_number_token(bytes: &[u8], start: usize) -> Option<(usize, Option<String>)> {
    let mut index = start;
    if bytes.get(index) == Some(&b'-') {
        index += 1;
    }
    match bytes.get(index) {
        Some(b'0') => {
            index += 1;
            if bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
                return None;
            }
        }
        Some(b'1'..=b'9') => {
            while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
                index += 1;
            }
        }
        _ => return None,
    }
    let integer_digits = std::str::from_utf8(&bytes[start..index]).ok()?.to_string();

    let mut is_integer = true;
    if bytes.get(index) == Some(&b'.') {
        is_integer = false;
        index += 1;
        let fraction_start = index;
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        if index == fraction_start {
            return None;
        }
    }
    if matches!(bytes.get(index), Some(b'e') | Some(b'E')) {
        is_integer = false;
        index += 1;
        if matches!(bytes.get(index), Some(b'+') | Some(b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        if index == exponent_start {
            return None;
        }
    }

    let complete = match bytes.get(index) {
        None => true,
        Some(byte) => matches!(byte, b',' | b']' | b'}' | b' ' | b'\t' | b'\n' | b'\r'),
    };
    if !complete {
        return None;
    }
    Some((index, is_integer.then_some(integer_digits)))
}

pub fn canonicalize_json_bytes(bytes: &[u8]) -> Result<Vec<u8>, HashError> {
    let value = parse_json_bytes_strict(bytes)?;
    canonicalize_json_value(&value)
}

pub fn canonicalize_jcs<T: Serialize>(value: &T) -> Result<Vec<u8>, HashError> {
    // serde_json converts non-finite floats to null when building a Value. Run
    // the RFC 8785 serializer first so NaN and infinities fail before that
    // lossy conversion, then strictly parse its output to validate integer
    // bounds without canonicalizing a second time.
    let canonical = serde_json_canonicalizer::to_vec(value).map_err(|error| {
        let message = error.to_string();
        if message.contains("NaN") || message.contains("Infinity") || message.contains("finite") {
            HashError::NonFiniteNumber
        } else {
            HashError::Serialization(message)
        }
    })?;
    parse_json_bytes_strict(&canonical)?;
    Ok(canonical)
}

pub fn canonicalize_json_value(value: &Value) -> Result<Vec<u8>, HashError> {
    validate_json_value(value)?;
    serde_json_canonicalizer::to_vec(value)
        .map_err(|error| HashError::Serialization(error.to_string()))
}

pub fn hash_json_bytes<K: Kind>(bytes: &[u8]) -> Result<HashId<K>, HashError> {
    require_profile::<K>(CanonicalizationProfile::AreteJcsV1)?;
    let payload = canonicalize_json_bytes(bytes)?;
    Ok(hash_canonical_payload(&payload))
}

pub fn hash_jcs<K: Kind, T: Serialize>(value: &T) -> Result<HashId<K>, HashError> {
    require_profile::<K>(CanonicalizationProfile::AreteJcsV1)?;
    let payload = canonicalize_jcs(value)?;
    Ok(hash_canonical_payload(&payload))
}

#[derive(Debug, Clone, Copy)]
pub struct TupleField<'a> {
    pub label: &'a str,
    pub value: &'a [u8],
}

impl<'a> TupleField<'a> {
    pub const fn new(label: &'a str, value: &'a [u8]) -> Self {
        Self { label, value }
    }
}

pub fn framed_tuple_payload(fields: &[TupleField<'_>]) -> Result<Vec<u8>, HashError> {
    let mut labels = HashSet::with_capacity(fields.len());
    let mut payload = Vec::new();
    payload.extend_from_slice(&(fields.len() as u64).to_be_bytes());
    for field in fields {
        if !labels.insert(field.label) {
            return Err(HashError::DuplicateTupleLabel(field.label.to_string()));
        }
        push_framed_bytes(&mut payload, field.label.as_bytes());
        push_framed_bytes(&mut payload, field.value);
    }
    Ok(payload)
}

pub fn hash_framed_tuple<K: Kind>(fields: &[TupleField<'_>]) -> Result<HashId<K>, HashError> {
    require_profile::<K>(CanonicalizationProfile::FramedTupleV1)?;
    let payload = framed_tuple_payload(fields)?;
    Ok(hash_canonical_payload(&payload))
}

fn validate_json_value(value: &Value) -> Result<(), HashError> {
    match value {
        Value::Null | Value::Bool(_) | Value::String(_) => Ok(()),
        Value::Array(values) => values.iter().try_for_each(validate_json_value),
        Value::Object(values) => values.values().try_for_each(validate_json_value),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&integer) {
                    return Err(HashError::UnsafeJsonInteger(integer.to_string()));
                }
            } else if let Some(integer) = number.as_u64() {
                if integer > MAX_SAFE_INTEGER as u64 {
                    return Err(HashError::UnsafeJsonInteger(integer.to_string()));
                }
            } else if !number.as_f64().is_some_and(f64::is_finite) {
                return Err(HashError::NonFiniteNumber);
            }
            Ok(())
        }
    }
}

fn classify_json_error(message: String) -> HashError {
    if let Some(value) = marker_value(&message, DUPLICATE_KEY_MARKER) {
        return HashError::DuplicateJsonKey(value);
    }
    if let Some(value) = marker_value(&message, UNSAFE_INTEGER_MARKER) {
        return HashError::UnsafeJsonInteger(value);
    }
    if marker_value(&message, NON_FINITE_MARKER).is_some() {
        return HashError::NonFiniteNumber;
    }
    if message.contains("number out of range") {
        return HashError::NonFiniteNumber;
    }
    HashError::InvalidJson(message)
}

fn marker_value(message: &str, marker: &str) -> Option<String> {
    let rest = message.split_once(marker)?.1;
    Some(rest.split(" at line ").next().unwrap_or(rest).to_string())
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a strict JSON value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictValue(Value::Null))
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
            return Err(E::custom(format!("{UNSAFE_INTEGER_MARKER}{value}")));
        }
        Ok(StrictValue(Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value > MAX_SAFE_INTEGER as u64 {
            return Err(E::custom(format!("{UNSAFE_INTEGER_MARKER}{value}")));
        }
        Ok(StrictValue(Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if !value.is_finite() {
            return Err(E::custom(format!("{NON_FINITE_MARKER}number")));
        }
        let number = serde_json::Number::from_f64(value)
            .ok_or_else(|| E::custom(format!("{NON_FINITE_MARKER}number")))?;
        Ok(StrictValue(Value::Number(number)))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictValue(Value::String(value.to_string())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictValue(Value::String(value)))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<StrictValue>()? {
            values.push(value.0);
        }
        Ok(StrictValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom(format!("{DUPLICATE_KEY_MARKER}{key}")));
            }
            let value = object.next_value::<StrictValue>()?;
            values.insert(key, value.0);
        }
        Ok(StrictValue(Value::Object(values)))
    }
}
