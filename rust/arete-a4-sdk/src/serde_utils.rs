//! Serde helpers for deserializing integers that may arrive as JSON strings.
//!
//! The Arete server converts u64 values exceeding JavaScript's
//! `Number.MAX_SAFE_INTEGER` (2^53 - 1) to strings for JSON transport.
//! These helpers allow the Rust SDK to transparently parse both formats.
//!
//! Each function is designed for use with `#[serde(deserialize_with = "...")]`.

use serde::de::{self, Deserializer, SeqAccess, Visitor};
use std::fmt;

/// Exact IDL integer decoding, including u128/i128, from strings or integral JSON numbers.
/// Fractional values and overflows fail; account keys and strings never pass through this helper.
pub fn deserialize_integer<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    use serde::Deserialize;
    let value = serde_json::Value::deserialize(deserializer)?;
    let text = match value {
        serde_json::Value::String(text) => text,
        serde_json::Value::Number(number) if number.is_u64() || number.is_i64() => {
            number.to_string()
        }
        _ => {
            return Err(de::Error::custom(
                "expected an exact integer or decimal string",
            ))
        }
    };
    text.parse().map_err(de::Error::custom)
}

pub fn deserialize_integer_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    use serde::Deserialize;
    let values = Vec::<serde_json::Value>::deserialize(deserializer)?;
    values
        .into_iter()
        .map(|value| deserialize_integer(value).map_err(de::Error::custom))
        .collect()
}

pub fn deserialize_optional_integer<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    use serde::Deserialize;
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    value
        .map(|value| deserialize_integer(value).map_err(de::Error::custom))
        .transpose()
}

// ─── Core visitors ──────────────────────────────────────────────────────────

struct U64OrStringVisitor;

impl<'de> Visitor<'de> for U64OrStringVisitor {
    type Value = u64;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("u64 or string-encoded u64")
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<u64, E> {
        Ok(v)
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<u64, E> {
        u64::try_from(v).map_err(|_| E::custom(format!("negative value {v} cannot be u64")))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<u64, E> {
        if v >= 0.0 && v <= u64::MAX as f64 {
            Ok(v as u64)
        } else {
            Err(E::custom(format!("f64 {v} out of u64 range")))
        }
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<u64, E> {
        v.parse().map_err(E::custom)
    }
}

struct I64OrStringVisitor;

impl<'de> Visitor<'de> for I64OrStringVisitor {
    type Value = i64;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("i64 or string-encoded i64")
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<i64, E> {
        i64::try_from(v).map_err(|_| E::custom(format!("u64 {v} overflows i64")))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<i64, E> {
        Ok(v)
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<i64, E> {
        if v >= i64::MIN as f64 && v <= i64::MAX as f64 {
            Ok(v as i64)
        } else {
            Err(E::custom(format!("f64 {v} out of i64 range")))
        }
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<i64, E> {
        v.parse().map_err(E::custom)
    }
}

// ─── Bare types ─────────────────────────────────────────────────────────────

/// Deserialize a bare `u64` from a JSON number or string.
pub fn deserialize_u64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    d.deserialize_any(U64OrStringVisitor)
}

/// Deserialize a bare `i64` from a JSON number or string.
pub fn deserialize_i64<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    d.deserialize_any(I64OrStringVisitor)
}

// ─── Option<T> ──────────────────────────────────────────────────────────────
// Used for non-optional spec fields. `None` = not yet received in any patch.
// With `#[serde(default)]`, missing fields → None. This function is only
// called when the field IS present in the JSON (null, number, or string).

/// Deserialize `Option<u64>` from null / number / string.
pub fn deserialize_option_u64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<u64>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("null, u64, or string-encoded u64")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Option<u64>, E> {
            Ok(None)
        }
        fn visit_none<E: de::Error>(self) -> Result<Option<u64>, E> {
            Ok(None)
        }
        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Option<u64>, D2::Error> {
            deserialize_u64(d).map(Some)
        }
    }
    d.deserialize_option(V)
}

/// Deserialize `Option<i64>` from null / number / string.
pub fn deserialize_option_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<i64>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("null, i64, or string-encoded i64")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Option<i64>, E> {
            Ok(None)
        }
        fn visit_none<E: de::Error>(self) -> Result<Option<i64>, E> {
            Ok(None)
        }
        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Option<i64>, D2::Error> {
            deserialize_i64(d).map(Some)
        }
    }
    d.deserialize_option(V)
}

// ─── Option<Option<T>> ─────────────────────────────────────────────────────
// Used for optional spec fields (patch semantics):
//   None         = field not present in patch (handled by #[serde(default)])
//   Some(None)   = field explicitly set to null
//   Some(Some(v))= field has value
//
// This function is only called when the field IS present, so:
//   JSON null   → Some(None)
//   JSON number → Some(Some(n))
//   JSON string → Some(Some(parse(s)))

/// Deserialize `Option<Option<u64>>` for patch semantics.
pub fn deserialize_option_option_u64<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<u64>>, D::Error> {
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<Option<u64>>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("null, u64, or string-encoded u64")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Option<Option<u64>>, E> {
            Ok(Some(None))
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Option<Option<u64>>, E> {
            Ok(Some(Some(v)))
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Option<Option<u64>>, E> {
            u64::try_from(v)
                .map(|v| Some(Some(v)))
                .map_err(|_| E::custom(format!("negative value {v} cannot be u64")))
        }
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Option<Option<u64>>, E> {
            if v >= 0.0 && v < (u64::MAX as f64) {
                Ok(Some(Some(v as u64)))
            } else {
                Err(E::custom(format!("f64 {v} out of u64 range")))
            }
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Option<Option<u64>>, E> {
            v.parse().map(|v| Some(Some(v))).map_err(E::custom)
        }
    }
    d.deserialize_any(V)
}

/// Deserialize `Option<Option<i64>>` for patch semantics.
pub fn deserialize_option_option_i64<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<i64>>, D::Error> {
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<Option<i64>>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("null, i64, or string-encoded i64")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Option<Option<i64>>, E> {
            Ok(Some(None))
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Option<Option<i64>>, E> {
            i64::try_from(v)
                .map(|v| Some(Some(v)))
                .map_err(|_| E::custom(format!("u64 {v} overflows i64")))
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Option<Option<i64>>, E> {
            Ok(Some(Some(v)))
        }
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Option<Option<i64>>, E> {
            Ok(Some(Some(v as i64)))
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Option<Option<i64>>, E> {
            v.parse().map(|v| Some(Some(v))).map_err(E::custom)
        }
    }
    d.deserialize_any(V)
}

// ─── Vec<T> variants ────────────────────────────────────────────────────────
// For array fields where elements may be numbers or strings.

/// Deserialize `Option<Vec<u64>>` where each element may be a number or string.
pub fn deserialize_option_vec_u64<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Vec<u64>>, D::Error> {
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<Vec<u64>>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("null or array of u64/string-encoded u64")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Option<Vec<u64>>, E> {
            Ok(None)
        }
        fn visit_none<E: de::Error>(self) -> Result<Option<Vec<u64>>, E> {
            Ok(None)
        }
        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Option<Vec<u64>>, D2::Error> {
            struct SeqV;
            impl<'de> Visitor<'de> for SeqV {
                type Value = Vec<u64>;
                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("array of u64/string-encoded u64")
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<u64>, A::Error> {
                    let mut vec = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                    while let Some(elem) = seq.next_element::<serde_json::Value>()? {
                        let n = match &elem {
                            serde_json::Value::Number(n) => n
                                .as_u64()
                                .or_else(|| n.as_i64().and_then(|i| u64::try_from(i).ok()))
                                .ok_or_else(|| {
                                    de::Error::custom(format!("cannot convert {n} to u64"))
                                })?,
                            serde_json::Value::String(s) => {
                                s.parse::<u64>().map_err(de::Error::custom)?
                            }
                            other => {
                                return Err(de::Error::custom(format!(
                                    "expected number or string in array, got {other}"
                                )));
                            }
                        };
                        vec.push(n);
                    }
                    Ok(vec)
                }
            }
            d.deserialize_seq(SeqV).map(Some)
        }
    }
    d.deserialize_option(V)
}

/// Deserialize `Option<Option<Vec<u64>>>` for optional array fields (patch semantics).
pub fn deserialize_option_option_vec_u64<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<Vec<u64>>>, D::Error> {
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<Option<Vec<u64>>>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("null or array of u64/string-encoded u64")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Option<Option<Vec<u64>>>, E> {
            Ok(Some(None))
        }
        fn visit_seq<A: SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Option<Option<Vec<u64>>>, A::Error> {
            let mut vec = Vec::with_capacity(seq.size_hint().unwrap_or(0));
            while let Some(elem) = seq.next_element::<serde_json::Value>()? {
                let n = match &elem {
                    serde_json::Value::Number(n) => n
                        .as_u64()
                        .or_else(|| n.as_i64().and_then(|i| u64::try_from(i).ok()))
                        .ok_or_else(|| de::Error::custom(format!("cannot convert {n} to u64")))?,
                    serde_json::Value::String(s) => s.parse::<u64>().map_err(de::Error::custom)?,
                    other => {
                        return Err(de::Error::custom(format!(
                            "expected number or string in array, got {other}"
                        )));
                    }
                };
                vec.push(n);
            }
            Ok(Some(Some(vec)))
        }
    }
    d.deserialize_any(V)
}

/// Deserialize `Option<Vec<i64>>` where each element may be a number or string.
pub fn deserialize_option_vec_i64<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Vec<i64>>, D::Error> {
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<Vec<i64>>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("null or array of i64/string-encoded i64")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Option<Vec<i64>>, E> {
            Ok(None)
        }
        fn visit_none<E: de::Error>(self) -> Result<Option<Vec<i64>>, E> {
            Ok(None)
        }
        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Option<Vec<i64>>, D2::Error> {
            struct SeqV;
            impl<'de> Visitor<'de> for SeqV {
                type Value = Vec<i64>;
                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("array of i64/string-encoded i64")
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<i64>, A::Error> {
                    let mut vec = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                    while let Some(elem) = seq.next_element::<serde_json::Value>()? {
                        let n = match &elem {
                            serde_json::Value::Number(n) => n.as_i64().ok_or_else(|| {
                                de::Error::custom(format!("cannot convert {n} to i64"))
                            })?,
                            serde_json::Value::String(s) => {
                                s.parse::<i64>().map_err(de::Error::custom)?
                            }
                            other => {
                                return Err(de::Error::custom(format!(
                                    "expected number or string in array, got {other}"
                                )));
                            }
                        };
                        vec.push(n);
                    }
                    Ok(vec)
                }
            }
            d.deserialize_seq(SeqV).map(Some)
        }
    }
    d.deserialize_option(V)
}

/// Deserialize `Option<Option<Vec<i64>>>` for optional array fields (patch semantics).
pub fn deserialize_option_option_vec_i64<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<Vec<i64>>>, D::Error> {
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<Option<Vec<i64>>>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("null or array of i64/string-encoded i64")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Option<Option<Vec<i64>>>, E> {
            Ok(Some(None))
        }
        fn visit_seq<A: SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Option<Option<Vec<i64>>>, A::Error> {
            let mut vec = Vec::with_capacity(seq.size_hint().unwrap_or(0));
            while let Some(elem) = seq.next_element::<serde_json::Value>()? {
                let n = match &elem {
                    serde_json::Value::Number(n) => n
                        .as_i64()
                        .ok_or_else(|| de::Error::custom(format!("cannot convert {n} to i64")))?,
                    serde_json::Value::String(s) => s.parse::<i64>().map_err(de::Error::custom)?,
                    other => {
                        return Err(de::Error::custom(format!(
                            "expected number or string in array, got {other}"
                        )));
                    }
                };
                vec.push(n);
            }
            Ok(Some(Some(vec)))
        }
    }
    d.deserialize_any(V)
}

// ─── 32-bit narrowing helpers ───────────────────────────────────────────────
// Delegate to the 64-bit deserializers above, then narrow via TryFrom.
// This avoids duplicating all the visitor boilerplate for i32/u32.

fn narrow_opt<W, N, E: de::Error>(opt: Option<W>) -> Result<Option<N>, E>
where
    N: TryFrom<W>,
    N::Error: fmt::Display,
{
    opt.map(|v| N::try_from(v).map_err(E::custom)).transpose()
}

fn narrow_opt_opt<W, N, E: de::Error>(opt: Option<Option<W>>) -> Result<Option<Option<N>>, E>
where
    N: TryFrom<W>,
    N::Error: fmt::Display,
{
    match opt {
        None => Ok(None),
        Some(None) => Ok(Some(None)),
        Some(Some(v)) => N::try_from(v).map(|n| Some(Some(n))).map_err(E::custom),
    }
}

fn narrow_opt_vec<W, N, E: de::Error>(opt: Option<Vec<W>>) -> Result<Option<Vec<N>>, E>
where
    N: TryFrom<W>,
    N::Error: fmt::Display,
{
    opt.map(|vec| {
        vec.into_iter()
            .map(|v| N::try_from(v).map_err(E::custom))
            .collect()
    })
    .transpose()
}

fn narrow_opt_opt_vec<W, N, E: de::Error>(
    opt: Option<Option<Vec<W>>>,
) -> Result<Option<Option<Vec<N>>>, E>
where
    N: TryFrom<W>,
    N::Error: fmt::Display,
{
    match opt {
        None => Ok(None),
        Some(None) => Ok(Some(None)),
        Some(Some(vec)) => vec
            .into_iter()
            .map(|v| N::try_from(v).map_err(E::custom))
            .collect::<Result<Vec<N>, E>>()
            .map(|v| Some(Some(v))),
    }
}

// ─── Option<u32/i32> ────────────────────────────────────────────────────────

pub fn deserialize_option_u32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    narrow_opt(deserialize_option_u64(d)?)
}

pub fn deserialize_option_i32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i32>, D::Error> {
    narrow_opt(deserialize_option_i64(d)?)
}

// ─── Option<Option<u32/i32>> ────────────────────────────────────────────────

pub fn deserialize_option_option_u32<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<u32>>, D::Error> {
    narrow_opt_opt(deserialize_option_option_u64(d)?)
}

pub fn deserialize_option_option_i32<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<i32>>, D::Error> {
    narrow_opt_opt(deserialize_option_option_i64(d)?)
}

// ─── Option<Vec<u32/i32>> ───────────────────────────────────────────────────

pub fn deserialize_option_vec_u32<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Vec<u32>>, D::Error> {
    narrow_opt_vec(deserialize_option_vec_u64(d)?)
}

pub fn deserialize_option_vec_i32<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Vec<i32>>, D::Error> {
    narrow_opt_vec(deserialize_option_vec_i64(d)?)
}

// ─── Option<Option<Vec<u32/i32>>> ───────────────────────────────────────────

pub fn deserialize_option_option_vec_u32<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<Vec<u32>>>, D::Error> {
    narrow_opt_opt_vec(deserialize_option_option_vec_u64(d)?)
}

pub fn deserialize_option_option_vec_i32<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<Vec<i32>>>, D::Error> {
    narrow_opt_opt_vec(deserialize_option_option_vec_i64(d)?)
}

// ─── 128-bit integers ───────────────────────────────────────────────────────
// `u128`/`i128` values travel as decimal strings (they exceed every JSON
// number type); small values may still arrive as JSON numbers. Generated
// types use these for IDL `u128`/`i128` fields.

fn wide_from_json<T, E>(value: &serde_json::Value, kind: &str) -> Result<T, E>
where
    T: std::str::FromStr,
    T::Err: fmt::Display,
    E: de::Error,
{
    let text = match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Number(number) if number.is_u64() || number.is_i64() => {
            number.to_string()
        }
        other => {
            return Err(E::custom(format!(
                "expected {kind} or string-encoded {kind}, got {other}"
            )))
        }
    };
    text.parse::<T>()
        .map_err(|error| E::custom(format!("invalid {kind} {text:?}: {error}")))
}

fn wide_opt<'de, D, T>(d: D, kind: &str) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    match <Option<serde_json::Value> as serde::Deserialize>::deserialize(d)? {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => wide_from_json(&value, kind).map(Some),
    }
}

fn wide_opt_opt<'de, D, T>(d: D, kind: &str) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    match <serde_json::Value as serde::Deserialize>::deserialize(d)? {
        serde_json::Value::Null => Ok(Some(None)),
        value => wide_from_json(&value, kind).map(|value| Some(Some(value))),
    }
}

fn wide_vec<T, E>(value: serde_json::Value, kind: &str) -> Result<Vec<T>, E>
where
    T: std::str::FromStr,
    T::Err: fmt::Display,
    E: de::Error,
{
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| wide_from_json(item, kind))
            .collect(),
        other => Err(E::custom(format!(
            "expected an array of {kind} values, got {other}"
        ))),
    }
}

fn wide_opt_vec<'de, D, T>(d: D, kind: &str) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    match <Option<serde_json::Value> as serde::Deserialize>::deserialize(d)? {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => wide_vec(value, kind).map(Some),
    }
}

fn wide_opt_opt_vec<'de, D, T>(d: D, kind: &str) -> Result<Option<Option<Vec<T>>>, D::Error>
where
    D: Deserializer<'de>,
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    match <serde_json::Value as serde::Deserialize>::deserialize(d)? {
        serde_json::Value::Null => Ok(Some(None)),
        value => wide_vec(value, kind).map(|value| Some(Some(value))),
    }
}

/// Deserialize `Option<u128>` from null / number / decimal string.
pub fn deserialize_option_u128<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u128>, D::Error> {
    wide_opt(d, "u128")
}

/// Deserialize `Option<i128>` from null / number / decimal string.
pub fn deserialize_option_i128<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i128>, D::Error> {
    wide_opt(d, "i128")
}

/// Deserialize `Option<Option<u128>>` for patch semantics.
pub fn deserialize_option_option_u128<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<u128>>, D::Error> {
    wide_opt_opt(d, "u128")
}

/// Deserialize `Option<Option<i128>>` for patch semantics.
pub fn deserialize_option_option_i128<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<i128>>, D::Error> {
    wide_opt_opt(d, "i128")
}

/// Deserialize `Option<Vec<u128>>` where each element may be a number or string.
pub fn deserialize_option_vec_u128<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Vec<u128>>, D::Error> {
    wide_opt_vec(d, "u128")
}

/// Deserialize `Option<Vec<i128>>` where each element may be a number or string.
pub fn deserialize_option_vec_i128<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Vec<i128>>, D::Error> {
    wide_opt_vec(d, "i128")
}

/// Deserialize `Option<Option<Vec<u128>>>` for optional array fields.
pub fn deserialize_option_option_vec_u128<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<Vec<u128>>>, D::Error> {
    wide_opt_opt_vec(d, "u128")
}

/// Deserialize `Option<Option<Vec<i128>>>` for optional array fields.
pub fn deserialize_option_option_vec_i128<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<Vec<i128>>>, D::Error> {
    wide_opt_opt_vec(d, "i128")
}

// ─── Nested shapes ──────────────────────────────────────────────────────────
// Program account models type every IDL value, so an integer can sit inside
// options, vectors, fixed arrays, tuples and maps (`Vec<Option<u64>>`,
// `(Pubkey, u128)`, ...). The fixed helpers above cover the flat shapes; for
// the rest, generated models name the value's shape with the markers in
// [`wire`] and decode it with [`deserialize_wire`],
// [`deserialize_wire_option`] (for a patch field), or
// [`deserialize_wire_option_option`] (for an IDL `Option` patch field):
//
// ```ignore
// #[serde(default, deserialize_with = "serde_utils::deserialize_wire_option::<_, _, serde_utils::wire::List<serde_utils::wire::Opt<serde_utils::wire::Int>>>")]
// pub amounts: Option<Vec<Option<u64>>>,
// ```

/// Shape markers for [`deserialize_wire_option`]: how a JSON value decodes
/// into a Rust value, with every integer accepting a JSON number or a
/// decimal string.
pub mod wire {
    use serde::de::DeserializeOwned;
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::marker::PhantomData;

    /// Decodes a JSON value into `T`, following the shape `Self` names.
    pub trait Decode<T> {
        fn decode(value: Value) -> Result<T, String>;
    }

    /// An integer: a JSON number or a decimal string (wide integers travel
    /// as strings).
    pub enum Int {}

    /// A value decoded by its own `Deserialize` impl (strings, floats,
    /// booleans, generated models, ...).
    pub enum Plain {}

    /// `Option<T>`: `null` is `None`, anything else `Some` of `M`'s shape.
    pub struct Opt<M>(PhantomData<M>);

    /// `Vec<T>` from a JSON array (IDL vectors and fixed arrays).
    pub struct List<M>(PhantomData<M>);

    /// `BTreeMap<String, T>` from a JSON object (IDL maps; keys are strings
    /// on the wire).
    pub struct Map<M>(PhantomData<M>);

    fn integer<T>(value: &Value, kind: &str) -> Result<T, String>
    where
        T: std::str::FromStr + TryFrom<u64> + TryFrom<i64> + TryFrom<i128>,
        <T as std::str::FromStr>::Err: std::fmt::Display,
    {
        let out_of_range = || format!("{value} is out of {kind} range");
        match value {
            Value::String(text) => text
                .parse::<T>()
                .map_err(|error| format!("invalid {kind} {text:?}: {error}")),
            Value::Number(number) => {
                if let Some(unsigned) = number.as_u64() {
                    T::try_from(unsigned).map_err(|_| out_of_range())
                } else if let Some(signed) = number.as_i64() {
                    T::try_from(signed).map_err(|_| out_of_range())
                } else {
                    match number.as_f64() {
                        Some(float)
                            if float.is_finite()
                                && float.fract() == 0.0
                                && float >= i128::MIN as f64
                                && float <= i128::MAX as f64 =>
                        {
                            T::try_from(float as i128).map_err(|_| out_of_range())
                        }
                        _ => Err(format!("expected {kind}, got {value}")),
                    }
                }
            }
            other => Err(format!(
                "expected {kind} or string-encoded {kind}, got {other}"
            )),
        }
    }

    macro_rules! decode_integers {
        ($($kind:ty),* $(,)?) => {
            $(
                impl Decode<$kind> for Int {
                    fn decode(value: Value) -> Result<$kind, String> {
                        integer(&value, stringify!($kind))
                    }
                }
            )*
        };
    }

    decode_integers!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

    impl<T: DeserializeOwned> Decode<T> for Plain {
        fn decode(value: Value) -> Result<T, String> {
            serde_json::from_value(value).map_err(|error| error.to_string())
        }
    }

    impl<T, M: Decode<T>> Decode<Option<T>> for Opt<M> {
        fn decode(value: Value) -> Result<Option<T>, String> {
            match value {
                Value::Null => Ok(None),
                value => M::decode(value).map(Some),
            }
        }
    }

    impl<T, M: Decode<T>> Decode<Vec<T>> for List<M> {
        fn decode(value: Value) -> Result<Vec<T>, String> {
            match value {
                Value::Array(items) => items.into_iter().map(M::decode).collect(),
                other => Err(format!("expected an array, got {other}")),
            }
        }
    }

    impl<T, M: Decode<T>> Decode<BTreeMap<String, T>> for Map<M> {
        fn decode(value: Value) -> Result<BTreeMap<String, T>, String> {
            match value {
                Value::Object(entries) => entries
                    .into_iter()
                    .map(|(key, item)| M::decode(item).map(|item| (key, item)))
                    .collect(),
                other => Err(format!("expected an object, got {other}")),
            }
        }
    }

    macro_rules! decode_tuples {
        ($(($len:expr; $($value:ident $marker:ident),+)),* $(,)?) => {
            $(
                impl<$($value, $marker: Decode<$value>),+> Decode<($($value,)+)> for ($($marker,)+) {
                    fn decode(value: Value) -> Result<($($value,)+), String> {
                        let items = match value {
                            Value::Array(items) if items.len() == $len => items,
                            other => {
                                return Err(format!("expected a {}-element array, got {other}", $len))
                            }
                        };
                        let mut items = items.into_iter();
                        Ok(($($marker::decode(items.next().expect("length checked"))?,)+))
                    }
                }
            )*
        };
    }

    decode_tuples!(
        (1; T0 M0),
        (2; T0 M0, T1 M1),
        (3; T0 M0, T1 M1, T2 M2),
        (4; T0 M0, T1 M1, T2 M2, T3 M3),
        (5; T0 M0, T1 M1, T2 M2, T3 M3, T4 M4),
        (6; T0 M0, T1 M1, T2 M2, T3 M3, T4 M4, T5 M5),
        (7; T0 M0, T1 M1, T2 M2, T3 M3, T4 M4, T5 M5, T6 M6),
        (8; T0 M0, T1 M1, T2 M2, T3 M3, T4 M4, T5 M5, T6 M6, T7 M7),
    );
}

/// Deserialize a required `T` whose value has the nested shape `M` (see
/// [`wire`]). Generated enum payloads use this because their fields are not
/// patch fields and therefore must not gain an outer `Option`.
pub fn deserialize_wire<'de, D, T, M>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    M: wire::Decode<T>,
{
    use serde::Deserialize;
    let value = serde_json::Value::deserialize(d)?;
    M::decode(value).map_err(de::Error::custom)
}

/// Deserialize `Option<T>` whose value has the nested shape `M` (see
/// [`wire`]): `null` is `None`.
pub fn deserialize_wire_option<'de, D, T, M>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    M: wire::Decode<T>,
{
    match <Option<serde_json::Value> as serde::Deserialize>::deserialize(d)? {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => M::decode(value).map(Some).map_err(de::Error::custom),
    }
}

/// Deserialize `Option<Option<T>>` (patch semantics, IDL `Option` fields)
/// whose value has the nested shape `M`: JSON `null` is `Some(None)`.
pub fn deserialize_wire_option_option<'de, D, T, M>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    M: wire::Decode<T>,
{
    match <serde_json::Value as serde::Deserialize>::deserialize(d)? {
        serde_json::Value::Null => Ok(Some(None)),
        value => M::decode(value)
            .map(|value| Some(Some(value)))
            .map_err(de::Error::custom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, Debug, PartialEq)]
    struct TestBare {
        #[serde(deserialize_with = "deserialize_u64")]
        balance: u64,
        #[serde(deserialize_with = "deserialize_i64")]
        timestamp: i64,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct TestOption {
        #[serde(default, deserialize_with = "deserialize_option_u64")]
        balance: Option<u64>,
        #[serde(default, deserialize_with = "deserialize_option_i64")]
        timestamp: Option<i64>,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct TestOptionOption {
        #[serde(default, deserialize_with = "deserialize_option_option_u64")]
        balance: Option<Option<u64>>,
        #[serde(default, deserialize_with = "deserialize_option_option_i64")]
        timestamp: Option<Option<i64>>,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct TestVec {
        #[serde(default, deserialize_with = "deserialize_option_vec_u64")]
        values: Option<Vec<u64>>,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct TestOptionOptionVec {
        #[serde(default, deserialize_with = "deserialize_option_option_vec_u64")]
        values: Option<Option<Vec<u64>>>,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct TestVecI64 {
        #[serde(default, deserialize_with = "deserialize_option_vec_i64")]
        values: Option<Vec<i64>>,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct TestOptionOptionVecI64 {
        #[serde(default, deserialize_with = "deserialize_option_option_vec_i64")]
        values: Option<Option<Vec<i64>>>,
    }

    // ── Bare types ──

    #[test]
    fn bare_u64_from_number() {
        let v: TestBare = serde_json::from_str(r#"{"balance": 42, "timestamp": -100}"#).unwrap();
        assert_eq!(v.balance, 42);
        assert_eq!(v.timestamp, -100);
    }

    #[test]
    fn bare_u64_from_string() {
        let v: TestBare =
            serde_json::from_str(r#"{"balance": "9007199254740992", "timestamp": "-100"}"#)
                .unwrap();
        assert_eq!(v.balance, 9007199254740992);
        assert_eq!(v.timestamp, -100);
    }

    // ── Option<T> ──

    #[test]
    fn option_from_number() {
        let v: TestOption = serde_json::from_str(r#"{"balance": 42, "timestamp": -100}"#).unwrap();
        assert_eq!(v.balance, Some(42));
        assert_eq!(v.timestamp, Some(-100));
    }

    #[test]
    fn option_from_string() {
        let v: TestOption =
            serde_json::from_str(r#"{"balance": "9007199254740992", "timestamp": "123"}"#).unwrap();
        assert_eq!(v.balance, Some(9007199254740992));
        assert_eq!(v.timestamp, Some(123));
    }

    #[test]
    fn option_from_null() {
        let v: TestOption =
            serde_json::from_str(r#"{"balance": null, "timestamp": null}"#).unwrap();
        assert_eq!(v.balance, None);
        assert_eq!(v.timestamp, None);
    }

    #[test]
    fn option_missing_field() {
        let v: TestOption = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(v.balance, None);
        assert_eq!(v.timestamp, None);
    }

    // ── Option<Option<T>> ──

    #[test]
    fn option_option_from_number() {
        let v: TestOptionOption =
            serde_json::from_str(r#"{"balance": 42, "timestamp": -100}"#).unwrap();
        assert_eq!(v.balance, Some(Some(42)));
        assert_eq!(v.timestamp, Some(Some(-100)));
    }

    #[test]
    fn option_option_from_string() {
        let v: TestOptionOption =
            serde_json::from_str(r#"{"balance": "9007199254740992", "timestamp": "123"}"#).unwrap();
        assert_eq!(v.balance, Some(Some(9007199254740992)));
        assert_eq!(v.timestamp, Some(Some(123)));
    }

    #[test]
    fn option_option_null_means_explicit_null() {
        let v: TestOptionOption =
            serde_json::from_str(r#"{"balance": null, "timestamp": null}"#).unwrap();
        assert_eq!(v.balance, Some(None)); // explicitly null
        assert_eq!(v.timestamp, Some(None));
    }

    #[test]
    fn option_option_missing_means_not_received() {
        let v: TestOptionOption = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(v.balance, None); // not in patch
        assert_eq!(v.timestamp, None);
    }

    // ── Vec variants ──

    #[test]
    fn vec_mixed_numbers_and_strings() {
        let v: TestVec = serde_json::from_str(r#"{"values": [1, "9007199254740992", 3]}"#).unwrap();
        assert_eq!(v.values, Some(vec![1, 9007199254740992, 3]));
    }

    #[test]
    fn vec_null() {
        let v: TestVec = serde_json::from_str(r#"{"values": null}"#).unwrap();
        assert_eq!(v.values, None);
    }

    #[test]
    fn vec_missing() {
        let v: TestVec = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(v.values, None);
    }

    #[test]
    fn option_option_vec_from_array() {
        let v: TestOptionOptionVec =
            serde_json::from_str(r#"{"values": [1, "9007199254740992"]}"#).unwrap();
        assert_eq!(v.values, Some(Some(vec![1, 9007199254740992])));
    }

    #[test]
    fn option_option_vec_null() {
        let v: TestOptionOptionVec = serde_json::from_str(r#"{"values": null}"#).unwrap();
        assert_eq!(v.values, Some(None));
    }

    #[test]
    fn option_option_vec_missing() {
        let v: TestOptionOptionVec = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(v.values, None);
    }

    // ── Vec<i64> variants ──

    #[test]
    fn vec_i64_mixed_numbers_and_strings() {
        let v: TestVecI64 =
            serde_json::from_str(r#"{"values": [-1, "9007199254740992", 3]}"#).unwrap();
        assert_eq!(v.values, Some(vec![-1, 9007199254740992, 3]));
    }

    #[test]
    fn vec_i64_null() {
        let v: TestVecI64 = serde_json::from_str(r#"{"values": null}"#).unwrap();
        assert_eq!(v.values, None);
    }

    #[test]
    fn vec_i64_missing() {
        let v: TestVecI64 = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(v.values, None);
    }

    #[test]
    fn option_option_vec_i64_from_array() {
        let v: TestOptionOptionVecI64 =
            serde_json::from_str(r#"{"values": [-1, "9007199254740992"]}"#).unwrap();
        assert_eq!(v.values, Some(Some(vec![-1, 9007199254740992])));
    }

    #[test]
    fn option_option_vec_i64_null() {
        let v: TestOptionOptionVecI64 = serde_json::from_str(r#"{"values": null}"#).unwrap();
        assert_eq!(v.values, Some(None));
    }

    #[test]
    fn option_option_vec_i64_missing() {
        let v: TestOptionOptionVecI64 = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(v.values, None);
    }

    // ── Edge cases ──

    #[test]
    fn large_u64_from_string() {
        let v: TestOption = serde_json::from_str(r#"{"balance": "18446744073709551615"}"#).unwrap();
        assert_eq!(v.balance, Some(u64::MAX));
    }

    #[test]
    fn u64_from_float() {
        let v: TestOption = serde_json::from_str(r#"{"balance": 42.0}"#).unwrap();
        assert_eq!(v.balance, Some(42));
    }

    // ── 32-bit narrowing ──

    #[derive(Deserialize, Debug, PartialEq)]
    struct TestOption32 {
        #[serde(default, deserialize_with = "deserialize_option_u32")]
        balance: Option<u32>,
        #[serde(default, deserialize_with = "deserialize_option_i32")]
        timestamp: Option<i32>,
    }

    #[test]
    fn option_u32_from_number() {
        let v: TestOption32 =
            serde_json::from_str(r#"{"balance": 42, "timestamp": -100}"#).unwrap();
        assert_eq!(v.balance, Some(42));
        assert_eq!(v.timestamp, Some(-100));
    }

    #[test]
    fn option_u32_from_string() {
        let v: TestOption32 =
            serde_json::from_str(r#"{"balance": "1000", "timestamp": "-50"}"#).unwrap();
        assert_eq!(v.balance, Some(1000));
        assert_eq!(v.timestamp, Some(-50));
    }

    #[test]
    fn option_u32_overflow_rejected() {
        let r = serde_json::from_str::<TestOption32>(r#"{"balance": 4294967296}"#);
        assert!(r.is_err(), "u32 overflow should be rejected");
    }

    #[test]
    fn option_i32_overflow_rejected() {
        let r = serde_json::from_str::<TestOption32>(r#"{"timestamp": 2147483648}"#);
        assert!(r.is_err(), "i32 overflow should be rejected");
    }

    #[test]
    fn option_u32_null_and_missing() {
        let v: TestOption32 =
            serde_json::from_str(r#"{"balance": null, "timestamp": null}"#).unwrap();
        assert_eq!(v.balance, None);
        assert_eq!(v.timestamp, None);
        let v: TestOption32 = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(v.balance, None);
        assert_eq!(v.timestamp, None);
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct TestOptionOption32 {
        #[serde(default, deserialize_with = "deserialize_option_option_u32")]
        balance: Option<Option<u32>>,
    }

    #[test]
    fn option_option_u32_patch_semantics() {
        let v: TestOptionOption32 = serde_json::from_str(r#"{"balance": 42}"#).unwrap();
        assert_eq!(v.balance, Some(Some(42)));
        let v: TestOptionOption32 = serde_json::from_str(r#"{"balance": null}"#).unwrap();
        assert_eq!(v.balance, Some(None));
        let v: TestOptionOption32 = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(v.balance, None);
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct TestVec32 {
        #[serde(default, deserialize_with = "deserialize_option_vec_u32")]
        values: Option<Vec<u32>>,
    }

    #[test]
    fn vec_u32_mixed() {
        let v: TestVec32 = serde_json::from_str(r#"{"values": [1, "2", 3]}"#).unwrap();
        assert_eq!(v.values, Some(vec![1, 2, 3]));
    }

    #[test]
    fn vec_u32_overflow_rejected() {
        let r = serde_json::from_str::<TestVec32>(r#"{"values": [1, 4294967296]}"#);
        assert!(r.is_err());
    }

    // ── 128-bit ──

    #[derive(Deserialize, Debug, PartialEq, Default)]
    struct Test128 {
        #[serde(default, deserialize_with = "deserialize_option_u128")]
        sqrt_price: Option<u128>,
        #[serde(default, deserialize_with = "deserialize_option_i128")]
        delta: Option<i128>,
        #[serde(default, deserialize_with = "deserialize_option_option_u128")]
        patched: Option<Option<u128>>,
        #[serde(default, deserialize_with = "deserialize_option_vec_u128")]
        rewards: Option<Vec<u128>>,
        #[serde(default, deserialize_with = "deserialize_option_option_vec_i128")]
        growth: Option<Option<Vec<i128>>>,
    }

    #[test]
    fn wide_integers_from_decimal_strings_and_numbers() {
        let v: Test128 = serde_json::from_str(
            r#"{
                "sqrt_price": "340282366920938463463374607431768211455",
                "delta": "-170141183460469231731687303715884105728",
                "patched": 7,
                "rewards": ["18446744073709551616", 2],
                "growth": [-1, "-18446744073709551617"]
            }"#,
        )
        .unwrap();
        assert_eq!(v.sqrt_price, Some(u128::MAX));
        assert_eq!(v.delta, Some(i128::MIN));
        assert_eq!(v.patched, Some(Some(7)));
        assert_eq!(v.rewards, Some(vec![u64::MAX as u128 + 1, 2]));
        assert_eq!(v.growth, Some(Some(vec![-1, -(u64::MAX as i128) - 2])));
    }

    #[test]
    fn wide_integers_null_missing_and_invalid() {
        let v: Test128 =
            serde_json::from_str(r#"{"sqrt_price": null, "patched": null, "growth": null}"#)
                .unwrap();
        assert_eq!(v.sqrt_price, None);
        assert_eq!(v.patched, Some(None));
        assert_eq!(v.growth, Some(None));
        let v: Test128 = serde_json::from_str("{}").unwrap();
        assert_eq!(v, Test128::default());
        assert!(serde_json::from_str::<Test128>(r#"{"sqrt_price": "-1"}"#).is_err());
        assert!(serde_json::from_str::<Test128>(r#"{"sqrt_price": 1.5}"#).is_err());
        assert!(serde_json::from_str::<Test128>(r#"{"rewards": "1"}"#).is_err());
    }

    /// The nested shapes generated program account models use (the
    /// attributes are the generator's, relative to this module).
    #[derive(Deserialize, Debug, PartialEq, Default)]
    struct TestWire {
        #[serde(
            default,
            deserialize_with = "deserialize_wire_option::<_, _, wire::List<wire::Opt<wire::Int>>>"
        )]
        amounts: Option<Vec<Option<u64>>>,
        #[serde(
            default,
            deserialize_with = "deserialize_wire_option::<_, _, (wire::Int, wire::Plain)>"
        )]
        pair: Option<(u64, String)>,
        #[serde(
            default,
            deserialize_with = "deserialize_wire_option::<_, _, wire::Map<wire::Int>>"
        )]
        balances: Option<std::collections::BTreeMap<String, u128>>,
        #[serde(
            default,
            deserialize_with = "deserialize_wire_option::<_, _, wire::List<wire::List<wire::Int>>>"
        )]
        grid: Option<Vec<Vec<i64>>>,
        #[serde(
            default,
            deserialize_with = "deserialize_wire_option_option::<_, _, wire::List<(wire::Int, wire::Plain)>>"
        )]
        flags: Option<Option<Vec<(u64, bool)>>>,
        #[serde(
            default,
            deserialize_with = "deserialize_wire_option::<_, _, (wire::Int,)>"
        )]
        single: Option<(i32,)>,
    }

    #[test]
    fn nested_integers_decode_at_any_depth() {
        let v: TestWire = serde_json::from_str(
            r#"{
                "amounts": ["1", null, 3],
                "pair": ["18446744073709551615", "key"],
                "balances": {"a": "340282366920938463463374607431768211455", "b": 2},
                "grid": [["-1", 2], []],
                "flags": [[1, true], ["2", false]],
                "single": ["-5"]
            }"#,
        )
        .unwrap();
        assert_eq!(v.amounts, Some(vec![Some(1), None, Some(3)]));
        assert_eq!(v.pair, Some((u64::MAX, "key".to_string())));
        assert_eq!(
            v.balances,
            Some(std::collections::BTreeMap::from([
                ("a".to_string(), u128::MAX),
                ("b".to_string(), 2),
            ]))
        );
        assert_eq!(v.grid, Some(vec![vec![-1, 2], vec![]]));
        assert_eq!(v.flags, Some(Some(vec![(1, true), (2, false)])));
        assert_eq!(v.single, Some((-5,)));
    }

    #[test]
    fn nested_shapes_null_missing_and_invalid() {
        let v: TestWire = serde_json::from_str(r#"{"amounts": null, "flags": null}"#).unwrap();
        assert_eq!(v.amounts, None);
        assert_eq!(v.flags, Some(None));
        assert_eq!(
            serde_json::from_str::<TestWire>("{}").unwrap(),
            TestWire::default()
        );
        for invalid in [
            r#"{"amounts": [1.5]}"#,
            r#"{"amounts": "1"}"#,
            r#"{"pair": [1]}"#,
            r#"{"pair": [1, "key", 2]}"#,
            r#"{"balances": [1]}"#,
            r#"{"single": ["2147483648"]}"#,
            r#"{"grid": [[-1], ["x"]]}"#,
        ] {
            assert!(
                serde_json::from_str::<TestWire>(invalid).is_err(),
                "accepted {invalid}"
            );
        }
    }

    /// Program account enums are externally tagged, the Program Read wire
    /// shape: a unit variant is its name, a data variant a one-key object
    /// (tuple fields keyed `field_<index>`).
    #[derive(Deserialize, Debug, PartialEq)]
    enum TestLevel {
        Partial {
            #[serde(default, deserialize_with = "deserialize_option_u64")]
            #[serde(alias = "numSignatures")]
            num_signatures: Option<u64>,
        },
        Address {
            #[serde(default)]
            field_0: Option<String>,
        },
        Full,
    }

    #[test]
    fn enums_decode_the_program_read_wire_tags() {
        let decode = |json: &str| serde_json::from_str::<TestLevel>(json);
        assert_eq!(decode(r#""Full""#).unwrap(), TestLevel::Full);
        assert_eq!(
            decode(r#"{"Partial": {"numSignatures": "5"}}"#).unwrap(),
            TestLevel::Partial {
                num_signatures: Some(5)
            }
        );
        assert_eq!(
            decode(r#"{"Address": {"field_0": "key"}}"#).unwrap(),
            TestLevel::Address {
                field_0: Some("key".to_string())
            }
        );
        assert!(decode(r#""Partial""#).is_err());
        assert!(decode("1").is_err());
    }
}
