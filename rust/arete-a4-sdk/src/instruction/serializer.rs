//! Borsh-compatible instruction data serialization.
//!
//! Values arrive as [`serde_json::Value`] (typed param structs serialize into
//! JSON upstream) and are encoded against an [`ArgSchema`] slice: little-endian
//! integers, u32 length prefixes for strings/vectors/bytes/maps, a single-byte
//! prefix for options and enum variant indexes, and map entries sorted by the
//! key's UTF-8 bytes.

use serde_json::{Map, Value};

use super::types::{json_kind, ArgField, ArgSchema, ArgType, EnumVariantDef, EnumVariantKind, InstructionError};

/// Serializes the discriminator plus instruction arguments into Borsh-encoded
/// instruction data.
///
/// Option fields treat an absent key (or JSON null) as `None`; every other
/// field must be present — silently encoding zeros for a missing arg would
/// corrupt instruction data.
pub fn serialize_instruction_data(
    discriminator: &[u8],
    args: &Map<String, Value>,
    schema: &[ArgSchema],
) -> Result<Vec<u8>, InstructionError> {
    let mut out = discriminator.to_vec();
    for field in schema {
        match args.get(&field.name).filter(|value| !value.is_null()) {
            Some(value) => serialize_value(value, &field.ty, &field.name, &mut out)?,
            None if matches!(field.ty, ArgType::Option(_)) => out.push(0),
            None => {
                return Err(InstructionError::MissingArgument {
                    name: field.name.clone(),
                })
            }
        }
    }
    Ok(out)
}

/// Integer wider than i64/u64, decoded from a JSON number or decimal string.
enum WideInt {
    /// Non-negative value.
    Unsigned(u128),
    /// Negative value.
    Signed(i128),
}

impl std::fmt::Display for WideInt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WideInt::Unsigned(u) => write!(f, "{u}"),
            WideInt::Signed(i) => write!(f, "{i}"),
        }
    }
}

fn invalid(ctx: &str, message: impl Into<String>) -> InstructionError {
    InstructionError::InvalidValue {
        context: ctx.to_string(),
        message: message.into(),
    }
}

/// Small integers (u8..u32, i8..i32) accept only integral JSON numbers.
fn int_in_range(value: &Value, ctx: &str, ty: &str, min: i128, max: i128) -> Result<i128, InstructionError> {
    let Value::Number(n) = value else {
        return Err(invalid(ctx, format!("expected a number for {ty}, got {}", json_kind(value))));
    };
    let v = if let Some(u) = n.as_u64() {
        i128::from(u)
    } else if let Some(i) = n.as_i64() {
        i128::from(i)
    } else {
        return Err(invalid(ctx, format!("expected an integer for {ty}, got {n}")));
    };
    if v < min || v > max {
        return Err(invalid(ctx, format!("value {v} out of range for {ty}")));
    }
    Ok(v)
}

/// Wide integers (u64/u128/i64/i128) accept integral JSON numbers or decimal
/// strings (mirroring `BigInt(string)` in the TypeScript serializer).
fn wide_int(value: &Value, ctx: &str, ty: &str) -> Result<WideInt, InstructionError> {
    match value {
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                Ok(WideInt::Unsigned(u128::from(u)))
            } else if let Some(i) = n.as_i64() {
                Ok(WideInt::Signed(i128::from(i)))
            } else {
                Err(invalid(ctx, format!("expected an integer for {ty}, got {n}")))
            }
        }
        Value::String(s) => {
            let trimmed = s.trim();
            if let Ok(u) = trimmed.parse::<u128>() {
                Ok(WideInt::Unsigned(u))
            } else if let Ok(i) = trimmed.parse::<i128>() {
                Ok(WideInt::Signed(i))
            } else {
                Err(invalid(ctx, format!("cannot convert \"{s}\" to {ty}")))
            }
        }
        other => Err(invalid(
            ctx,
            format!("expected an integer or decimal string for {ty}, got {}", json_kind(other)),
        )),
    }
}

fn out_of_range(ctx: &str, ty: &str, value: &WideInt) -> InstructionError {
    invalid(ctx, format!("value {value} out of range for {ty}"))
}

fn float_value(value: &Value, ctx: &str, ty: &str) -> Result<f64, InstructionError> {
    match value {
        Value::Number(n) => n
            .as_f64()
            .ok_or_else(|| invalid(ctx, format!("value {n} is not representable as {ty}"))),
        other => Err(invalid(ctx, format!("expected a number for {ty}, got {}", json_kind(other)))),
    }
}

fn byte_array(value: &Value, ctx: &str) -> Result<Vec<u8>, InstructionError> {
    let Value::Array(items) = value else {
        return Err(invalid(
            ctx,
            format!("Cannot serialize bytes from value of type {}", json_kind(value)),
        ));
    };
    items
        .iter()
        .map(|item| {
            item.as_u64()
                .filter(|b| *b <= 255)
                .map(|b| b as u8)
                .ok_or_else(|| invalid(ctx, "byte array elements must be integers in 0..=255"))
        })
        .collect()
}

fn serialize_value(value: &Value, ty: &ArgType, ctx: &str, out: &mut Vec<u8>) -> Result<(), InstructionError> {
    match ty {
        ArgType::U8 => {
            let v = int_in_range(value, ctx, "u8", 0, i128::from(u8::MAX))?;
            out.push(v as u8);
        }
        ArgType::U16 => {
            let v = int_in_range(value, ctx, "u16", 0, i128::from(u16::MAX))?;
            out.extend_from_slice(&(v as u16).to_le_bytes());
        }
        ArgType::U32 => {
            let v = int_in_range(value, ctx, "u32", 0, i128::from(u32::MAX))?;
            out.extend_from_slice(&(v as u32).to_le_bytes());
        }
        ArgType::I8 => {
            let v = int_in_range(value, ctx, "i8", i128::from(i8::MIN), i128::from(i8::MAX))?;
            out.extend_from_slice(&(v as i8).to_le_bytes());
        }
        ArgType::I16 => {
            let v = int_in_range(value, ctx, "i16", i128::from(i16::MIN), i128::from(i16::MAX))?;
            out.extend_from_slice(&(v as i16).to_le_bytes());
        }
        ArgType::I32 => {
            let v = int_in_range(value, ctx, "i32", i128::from(i32::MIN), i128::from(i32::MAX))?;
            out.extend_from_slice(&(v as i32).to_le_bytes());
        }
        ArgType::U64 => match wide_int(value, ctx, "u64")? {
            WideInt::Unsigned(u) if u <= u128::from(u64::MAX) => {
                out.extend_from_slice(&(u as u64).to_le_bytes());
            }
            other => return Err(out_of_range(ctx, "u64", &other)),
        },
        ArgType::I64 => match wide_int(value, ctx, "i64")? {
            WideInt::Unsigned(u) if u <= i64::MAX as u128 => {
                out.extend_from_slice(&(u as i64).to_le_bytes());
            }
            WideInt::Signed(i) if i >= i128::from(i64::MIN) && i <= i128::from(i64::MAX) => {
                out.extend_from_slice(&(i as i64).to_le_bytes());
            }
            other => return Err(out_of_range(ctx, "i64", &other)),
        },
        ArgType::U128 => match wide_int(value, ctx, "u128")? {
            WideInt::Unsigned(u) => out.extend_from_slice(&u.to_le_bytes()),
            other => return Err(out_of_range(ctx, "u128", &other)),
        },
        ArgType::I128 => match wide_int(value, ctx, "i128")? {
            WideInt::Unsigned(u) if u <= i128::MAX as u128 => {
                out.extend_from_slice(&(u as i128).to_le_bytes());
            }
            WideInt::Signed(i) => out.extend_from_slice(&i.to_le_bytes()),
            other => return Err(out_of_range(ctx, "i128", &other)),
        },
        ArgType::F32 => {
            let v = float_value(value, ctx, "f32")?;
            out.extend_from_slice(&(v as f32).to_le_bytes());
        }
        ArgType::F64 => {
            let v = float_value(value, ctx, "f64")?;
            out.extend_from_slice(&v.to_le_bytes());
        }
        ArgType::Bool => {
            let Value::Bool(b) = value else {
                return Err(invalid(ctx, format!("expected a boolean, got {}", json_kind(value))));
            };
            out.push(u8::from(*b));
        }
        ArgType::String => {
            let Value::String(s) = value else {
                return Err(invalid(ctx, format!("expected a string, got {}", json_kind(value))));
            };
            out.extend_from_slice(&(s.len() as u32).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        ArgType::Pubkey => {
            let bytes = match value {
                Value::String(s) => {
                    let decoded = bs58::decode(s).into_vec().map_err(|_| {
                        InstructionError::InvalidPubkey(format!("'{s}' is not valid base58"))
                    })?;
                    if decoded.len() != 32 {
                        return Err(InstructionError::InvalidPubkey(format!(
                            "'{s}' decoded to {} bytes, expected 32",
                            decoded.len()
                        )));
                    }
                    decoded
                }
                Value::Array(_) => {
                    let decoded = byte_array(value, ctx).map_err(|_| {
                        InstructionError::InvalidPubkey(
                            "pubkey byte arrays must contain integers in 0..=255".to_string(),
                        )
                    })?;
                    if decoded.len() != 32 {
                        return Err(InstructionError::InvalidPubkey(format!(
                            "invalid pubkey byte length: expected 32, got {}",
                            decoded.len()
                        )));
                    }
                    decoded
                }
                other => {
                    return Err(InstructionError::InvalidPubkey(format!(
                        "cannot serialize pubkey from value of type {}",
                        json_kind(other)
                    )))
                }
            };
            out.extend_from_slice(&bytes);
        }
        ArgType::Bytes => {
            let bytes = byte_array(value, ctx)?;
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&bytes);
        }
        ArgType::Vec(inner) => {
            let Value::Array(items) = value else {
                return Err(invalid(ctx, format!("expected an array for vec, got {}", json_kind(value))));
            };
            out.extend_from_slice(&(items.len() as u32).to_le_bytes());
            for item in items {
                serialize_value(item, inner, ctx, out)?;
            }
        }
        ArgType::Option(inner) => {
            if value.is_null() {
                out.push(0);
            } else {
                out.push(1);
                serialize_value(value, inner, ctx, out)?;
            }
        }
        ArgType::Array(inner, length) => {
            let Value::Array(items) = value else {
                return Err(invalid(ctx, format!("expected an array, got {}", json_kind(value))));
            };
            if items.len() != *length {
                return Err(invalid(
                    ctx,
                    format!("Array length mismatch: expected {length}, got {}", items.len()),
                ));
            }
            for item in items {
                serialize_value(item, inner, ctx, out)?;
            }
        }
        ArgType::HashMap(key_ty, value_ty) => {
            if **key_ty != ArgType::String {
                return Err(invalid(ctx, "Instruction hashMap keys must use the 'string' schema"));
            }
            let Value::Object(map) = value else {
                return Err(invalid(
                    ctx,
                    format!("HashMap value must be a plain object, got {}", json_kind(value)),
                ));
            };
            // Rust/Borsh sorts String keys by their UTF-8 bytes before serializing.
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.as_bytes().cmp(right.as_bytes()));
            out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
            for (key, entry) in entries {
                out.extend_from_slice(&(key.len() as u32).to_le_bytes());
                out.extend_from_slice(key.as_bytes());
                serialize_value(entry, value_ty, &format!("{ctx}.{key}"), out)?;
            }
        }
        ArgType::Struct(fields) => serialize_struct(value, fields, ctx, out)?,
        ArgType::Enum(variants) => serialize_enum(value, variants, ctx, out)?,
    }
    Ok(())
}

fn serialize_struct(
    value: &Value,
    fields: &[ArgField],
    ctx: &str,
    out: &mut Vec<u8>,
) -> Result<(), InstructionError> {
    let Value::Object(obj) = value else {
        return Err(invalid(
            ctx,
            format!("Struct value must be a plain object, got {}", json_kind(value)),
        ));
    };
    for field in fields {
        match obj.get(&field.name).filter(|v| !v.is_null()) {
            Some(v) => serialize_value(v, &field.ty, &format!("{ctx}.{}", field.name), out)?,
            None if matches!(field.ty, ArgType::Option(_)) => out.push(0),
            None => {
                return Err(invalid(
                    ctx,
                    format!("Missing required struct field \"{}\"", field.name),
                ))
            }
        }
    }
    Ok(())
}

fn variant_index(variants: &[EnumVariantDef], name: &str, ctx: &str) -> Result<usize, InstructionError> {
    variants.iter().position(|v| v.name == name).ok_or_else(|| {
        let names: Vec<&str> = variants.iter().map(|v| v.name.as_str()).collect();
        invalid(
            ctx,
            format!("Unknown enum variant \"{name}\". Expected one of: {}", names.join(", ")),
        )
    })
}

fn serialize_enum(
    value: &Value,
    variants: &[EnumVariantDef],
    ctx: &str,
    out: &mut Vec<u8>,
) -> Result<(), InstructionError> {
    match value {
        // A bare variant index (unit variants only).
        Value::Number(n) => {
            let Some(index) = n.as_u64().map(|i| i as usize).filter(|i| *i < variants.len()) else {
                return Err(invalid(
                    ctx,
                    format!(
                        "Enum variant index {n} out of range (0..{})",
                        variants.len().saturating_sub(1)
                    ),
                ));
            };
            let variant = &variants[index];
            if !matches!(variant.kind, EnumVariantKind::Unit) {
                return Err(invalid(
                    ctx,
                    format!(
                        "Enum variant \"{name}\" carries data; pass {{ {name}: ... }} instead of an index",
                        name = variant.name
                    ),
                ));
            }
            out.push(index as u8);
            Ok(())
        }
        // A unit variant by name.
        Value::String(name) => {
            let index = variant_index(variants, name, ctx)?;
            if !matches!(variants[index].kind, EnumVariantKind::Unit) {
                return Err(invalid(
                    ctx,
                    format!("Enum variant \"{name}\" carries data; pass {{ {name}: ... }} instead of a bare name"),
                ));
            }
            out.push(index as u8);
            Ok(())
        }
        // { variantName: payload } for data-carrying variants.
        Value::Object(obj) => {
            let mut iter = obj.iter();
            let (Some((key, payload)), None) = (iter.next(), iter.next()) else {
                let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
                return Err(invalid(
                    ctx,
                    format!(
                        "Enum value must be a single-key object ({{ variantName: payload }}), got keys [{}]",
                        keys.join(", ")
                    ),
                ));
            };
            let index = variant_index(variants, key, ctx)?;
            match &variants[index].kind {
                EnumVariantKind::Unit => Err(invalid(
                    ctx,
                    format!("Enum variant \"{key}\" is fieldless; pass '{key}' instead of an object"),
                )),
                EnumVariantKind::Struct(fields) => {
                    out.push(index as u8);
                    serialize_struct(payload, fields, &format!("{ctx}.{key}"), out)
                }
                EnumVariantKind::Tuple(types) => {
                    let Some(items) = payload.as_array().filter(|items| items.len() == types.len()) else {
                        return Err(invalid(
                            ctx,
                            format!("Enum variant \"{key}\" expects a tuple of length {}", types.len()),
                        ));
                    };
                    out.push(index as u8);
                    for (i, (item, item_ty)) in items.iter().zip(types).enumerate() {
                        serialize_value(item, item_ty, &format!("{ctx}.{key}[{i}]"), out)?;
                    }
                    Ok(())
                }
            }
        }
        other => Err(invalid(
            ctx,
            format!("Cannot serialize enum from value of type {}", json_kind(other)),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";

    fn arg(name: &str, ty: ArgType) -> ArgSchema {
        ArgSchema { name: name.to_string(), ty }
    }

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().cloned().expect("test args must be an object")
    }

    fn ser(schema: &[ArgSchema], params: Value) -> Vec<u8> {
        serialize_instruction_data(&[], &args(params), schema).expect("serialization should succeed")
    }

    fn ser_err(schema: &[ArgSchema], params: Value) -> String {
        serialize_instruction_data(&[], &args(params), schema)
            .expect_err("serialization should fail")
            .to_string()
    }

    #[test]
    fn prefixes_the_discriminator_and_serializes_primitives() {
        let schema = [
            arg("amount", ArgType::U64),
            arg("flag", ArgType::Bool),
            arg("count", ArgType::U8),
        ];
        let data = serialize_instruction_data(
            &[0xaa, 0xbb],
            &args(json!({ "amount": 1, "flag": true, "count": 7 })),
            &schema,
        )
        .unwrap();
        assert_eq!(data, vec![0xaa, 0xbb, 1, 0, 0, 0, 0, 0, 0, 0, 1, 7]);
    }

    #[test]
    fn serializes_u64_from_decimal_strings() {
        let schema = [arg("amount", ArgType::U64)];
        assert_eq!(
            ser(&schema, json!({ "amount": "18446744073709551615" })),
            vec![0xff; 8]
        );
        assert_eq!(ser(&schema, json!({ "amount": "1" })), vec![1, 0, 0, 0, 0, 0, 0, 0]);
        assert!(ser_err(&schema, json!({ "amount": "18446744073709551616" })).contains("out of range"));
        assert!(ser_err(&schema, json!({ "amount": -1 })).contains("out of range"));
    }

    #[test]
    fn serializes_negative_i64_i128_in_twos_complement() {
        let schema = [arg("small", ArgType::I64), arg("big", ArgType::I128)];
        let data = ser(&schema, json!({ "small": -1, "big": "-1" }));
        assert_eq!(data, vec![0xff; 24]);

        let min = ser(
            &schema,
            json!({ "small": "-9223372036854775808", "big": "-170141183460469231731687303715884105728" }),
        );
        assert_eq!(&min[..8], &[0, 0, 0, 0, 0, 0, 0, 0x80]);
        assert_eq!(&min[8..24], &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80]);
    }

    #[test]
    fn serializes_large_u128_values() {
        let schema = [arg("huge", ArgType::U128)];
        assert_eq!(
            ser(&schema, json!({ "huge": "340282366920938463463374607431768211455" })),
            vec![0xff; 16]
        );
        assert_eq!(ser(&schema, json!({ "huge": 1 }))[0], 1);
    }

    #[test]
    fn serializes_strings_with_a_length_prefix() {
        let schema = [arg("s", ArgType::String)];
        assert_eq!(ser(&schema, json!({ "s": "hi" })), vec![2, 0, 0, 0, 0x68, 0x69]);
    }

    #[test]
    fn serializes_option_none_and_some() {
        let schema = [arg("maybe", ArgType::Option(Box::new(ArgType::U8)))];
        assert_eq!(ser(&schema, json!({})), vec![0]);
        assert_eq!(ser(&schema, json!({ "maybe": null })), vec![0]);
        assert_eq!(ser(&schema, json!({ "maybe": 9 })), vec![1, 9]);
    }

    #[test]
    fn serializes_vec_with_a_length_prefix() {
        let schema = [arg("v", ArgType::Vec(Box::new(ArgType::U16)))];
        assert_eq!(ser(&schema, json!({ "v": [1, 258] })), vec![2, 0, 0, 0, 1, 0, 2, 1]);
    }

    #[test]
    fn serializes_fixed_arrays_without_a_prefix_and_checks_length() {
        let schema = [arg("a", ArgType::Array(Box::new(ArgType::U8), 3))];
        assert_eq!(ser(&schema, json!({ "a": [4, 5, 6] })), vec![4, 5, 6]);
        assert!(ser_err(&schema, json!({ "a": [1] })).contains("length mismatch"));
    }

    #[test]
    fn serializes_pubkeys_from_base58_or_byte_arrays() {
        let schema = [arg("mint", ArgType::Pubkey)];
        assert_eq!(ser(&schema, json!({ "mint": SYSTEM_PROGRAM })), vec![0; 32]);
        assert_eq!(ser(&schema, json!({ "mint": vec![0u8; 32] })), vec![0; 32]);
        assert!(ser_err(&schema, json!({ "mint": "abc" })).contains("Invalid pubkey"));
        assert!(ser_err(&schema, json!({ "mint": 42 })).contains("Invalid pubkey"));
    }

    #[test]
    fn serializes_bytes_with_a_length_prefix() {
        let schema = [arg("blob", ArgType::Bytes)];
        assert_eq!(ser(&schema, json!({ "blob": [9, 8] })), vec![2, 0, 0, 0, 9, 8]);
        assert!(ser_err(&schema, json!({ "blob": "nope" })).contains("Cannot serialize bytes"));
    }

    #[test]
    fn serializes_floats_little_endian() {
        let schema = [arg("ratio", ArgType::F32), arg("price", ArgType::F64)];
        let data = ser(&schema, json!({ "ratio": 1.5, "price": 2.5 }));
        assert_eq!(&data[..4], &1.5f32.to_le_bytes());
        assert_eq!(&data[4..], &2.5f64.to_le_bytes());
    }

    #[test]
    fn serializes_string_key_maps_in_utf8_key_order() {
        let schema = [arg(
            "labels",
            ArgType::HashMap(Box::new(ArgType::String), Box::new(ArgType::U8)),
        )];
        let data = ser(&schema, json!({ "labels": { "z": 1, "a": 2 } }));
        assert_eq!(data, vec![2, 0, 0, 0, 1, 0, 0, 0, 0x61, 2, 1, 0, 0, 0, 0x7a, 1]);
    }

    #[test]
    fn serializes_string_key_maps_with_string_values() {
        let schema = [arg(
            "metadata",
            ArgType::HashMap(Box::new(ArgType::String), Box::new(ArgType::String)),
        )];
        let data = ser(&schema, json!({ "metadata": { "b": "two", "a": "one" } }));
        assert_eq!(
            data,
            vec![
                2, 0, 0, 0, //
                1, 0, 0, 0, 0x61, //
                3, 0, 0, 0, 0x6f, 0x6e, 0x65, //
                1, 0, 0, 0, 0x62, //
                3, 0, 0, 0, 0x74, 0x77, 0x6f,
            ]
        );
    }

    #[test]
    fn serializes_nested_authorization_payload_maps() {
        let schema = [arg(
            "authorizationData",
            ArgType::Struct(vec![ArgField {
                name: "payload".to_string(),
                ty: ArgType::Struct(vec![ArgField {
                    name: "map".to_string(),
                    ty: ArgType::HashMap(
                        Box::new(ArgType::String),
                        Box::new(ArgType::Enum(vec![
                            EnumVariantDef {
                                name: "Pubkey".to_string(),
                                kind: EnumVariantKind::Tuple(vec![ArgType::Pubkey]),
                            },
                            EnumVariantDef {
                                name: "Number".to_string(),
                                kind: EnumVariantKind::Tuple(vec![ArgType::U64]),
                            },
                        ])),
                    ),
                }]),
            }]),
        )];
        let data = ser(
            &schema,
            json!({
                "authorizationData": {
                    "payload": {
                        "map": {
                            "b": { "Number": [7] },
                            "a": { "Pubkey": [SYSTEM_PROGRAM] },
                        }
                    }
                }
            }),
        );
        let mut expected = vec![2, 0, 0, 0, 1, 0, 0, 0, 0x61, 0];
        expected.extend_from_slice(&[0; 32]);
        expected.extend_from_slice(&[1, 0, 0, 0, 0x62, 1, 7, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(data, expected);
    }

    #[test]
    fn rejects_invalid_map_inputs_and_key_schemas() {
        let schema = [arg(
            "labels",
            ArgType::HashMap(Box::new(ArgType::String), Box::new(ArgType::U8)),
        )];
        assert!(ser_err(&schema, json!({ "labels": ["a"] })).contains("must be a plain object"));

        let bad_key = [arg(
            "labels",
            ArgType::HashMap(Box::new(ArgType::U64), Box::new(ArgType::U8)),
        )];
        assert!(ser_err(&bad_key, json!({ "labels": { "a": 1 } })).contains("'string' schema"));
    }

    #[test]
    fn serializes_structs_in_field_order_including_nesting() {
        let schema = [arg(
            "data",
            ArgType::Struct(vec![
                ArgField { name: "amount".to_string(), ty: ArgType::U64 },
                ArgField {
                    name: "inner".to_string(),
                    ty: ArgType::Struct(vec![ArgField { name: "flag".to_string(), ty: ArgType::Bool }]),
                },
            ]),
        )];
        // Keys intentionally out of schema order.
        let data = ser(&schema, json!({ "data": { "inner": { "flag": true }, "amount": 3 } }));
        assert_eq!(data, vec![3, 0, 0, 0, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn rejects_structs_with_missing_required_fields() {
        let schema = [arg(
            "data",
            ArgType::Struct(vec![ArgField { name: "amount".to_string(), ty: ArgType::U64 }]),
        )];
        assert!(
            ser_err(&schema, json!({ "data": {} }))
                .contains("Missing required struct field \"amount\"")
        );
    }

    fn op_schema() -> [ArgSchema; 1] {
        [arg(
            "op",
            ArgType::Enum(vec![
                EnumVariantDef { name: "noop".to_string(), kind: EnumVariantKind::Unit },
                EnumVariantDef {
                    name: "transfer".to_string(),
                    kind: EnumVariantKind::Struct(vec![ArgField {
                        name: "amount".to_string(),
                        ty: ArgType::U64,
                    }]),
                },
                EnumVariantDef {
                    name: "pair".to_string(),
                    kind: EnumVariantKind::Tuple(vec![ArgType::U8, ArgType::U16]),
                },
            ]),
        )]
    }

    #[test]
    fn serializes_unit_enums_by_name_or_index() {
        let schema = [arg(
            "status",
            ArgType::Enum(vec![
                EnumVariantDef { name: "active".to_string(), kind: EnumVariantKind::Unit },
                EnumVariantDef { name: "sunset".to_string(), kind: EnumVariantKind::Unit },
            ]),
        )];
        assert_eq!(ser(&schema, json!({ "status": "sunset" })), vec![1]);
        assert_eq!(ser(&schema, json!({ "status": 0 })), vec![0]);
        assert!(ser_err(&schema, json!({ "status": "paused" })).contains("Unknown enum variant \"paused\""));
        assert!(ser_err(&schema, json!({ "status": 2 })).contains("out of range"));
    }

    #[test]
    fn serializes_data_carrying_enum_variants() {
        let schema = op_schema();
        assert_eq!(
            ser(&schema, json!({ "op": { "transfer": { "amount": 7 } } })),
            vec![1, 7, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(ser(&schema, json!({ "op": { "pair": [5, 258] } })), vec![2, 5, 2, 1]);
    }

    #[test]
    fn rejects_malformed_enum_values() {
        let schema = op_schema();
        assert!(ser_err(&schema, json!({ "op": "transfer" })).contains("carries data"));
        assert!(ser_err(&schema, json!({ "op": 1 })).contains("carries data"));
        assert!(ser_err(&schema, json!({ "op": { "noop": {} } })).contains("is fieldless"));
        assert!(ser_err(&schema, json!({ "op": { "pair": [5] } })).contains("tuple of length 2"));
        assert!(
            ser_err(&schema, json!({ "op": { "transfer": {}, "pair": [] } }))
                .contains("single-key object")
        );
        assert!(ser_err(&schema, json!({ "op": true })).contains("Cannot serialize enum"));
    }

    #[test]
    fn rejects_missing_required_arguments() {
        let schema = [arg("amount", ArgType::U64)];
        for params in [json!({}), json!({ "amount": null })] {
            let err = serialize_instruction_data(&[], &args(params), &schema).unwrap_err();
            assert_eq!(err, InstructionError::MissingArgument { name: "amount".to_string() });
        }
    }

    #[test]
    fn rejects_fractional_and_out_of_range_small_integers() {
        let schema = [arg("count", ArgType::U8)];
        assert!(ser_err(&schema, json!({ "count": 1.5 })).contains("expected an integer"));
        assert!(ser_err(&schema, json!({ "count": 256 })).contains("out of range"));
        assert!(ser_err(&schema, json!({ "count": -1 })).contains("out of range"));
    }
}
