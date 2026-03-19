//! Serde helpers for deserializing integers that may arrive as JSON strings.
//!
//! The HyperStack server converts u64 values exceeding JavaScript's
//! `Number.MAX_SAFE_INTEGER` (2^53 - 1) to strings for JSON transport.
//! These helpers allow the Rust SDK to transparently parse both formats.
//!
//! Each function is designed for use with `#[serde(deserialize_with = "...")]`.

use serde::de::{self, Deserializer, SeqAccess, Visitor};
use std::fmt;

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
}
