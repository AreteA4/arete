//! Entity rows packed into one compact allocation.
//!
//! A `serde_json::Value` tree costs several times the JSON it stands for.
//! Every object is a B-tree leaf of fixed size (632 bytes for up to 11
//! fields, whether it has 1 or 11). Every key is its own heap `String`,
//! repeated in every object that has it. Every value takes a 32-byte slot.
//! An object of four small fields, as in an array of per-bin fee records,
//! costs about 750 bytes of heap for under 100 bytes of JSON.
//!
//! A state table holds thousands of rows, and a handler works on one at a
//! time. So the table keeps each row as a [`PackedRow`]: one byte buffer,
//! about the size of the row's JSON or smaller, unpacked into a `Value` only
//! while a handler works on it.
//!
//! Packing is lossless. [`PackedRow::unpack`] returns a value equal to the
//! one packed: the same fields in the same order, the same strings, and
//! numbers with the same representation (unsigned, negative or float). It
//! therefore serializes to exactly the same bytes.
//!
//! The format exists only in memory and nothing persists it, so it can change
//! freely.
//!
//! # Layout
//!
//! - `u32` little-endian: offset of the key dictionary.
//! - The value.
//! - The key dictionary: a varint count, then each key as a varint length and
//!   its UTF-8 bytes. Objects refer to keys by their index here, so a key
//!   repeated in every element of an array is stored once.
//!
//! A value is a tag byte, then:
//!
//! - null, false, true, and unsigned integers below 240 (in the tag itself):
//!   nothing.
//! - Other unsigned integers: a LEB128 varint.
//! - Negative integers: the varint of the bitwise complement.
//! - Floats: 8 bytes, little-endian.
//! - A number that none of those reproduces exactly (only possible with
//!   serde_json's `arbitrary_precision`): its text, length-prefixed.
//! - Strings: a varint length and the UTF-8 bytes.
//! - Strings of three or more ASCII digits (integers carried as strings, such
//!   as u64 and u128 amounts): a varint digit count, then two digits per
//!   byte.
//! - Arrays: a varint count, then the items.
//! - Objects: a varint count, then each field as a varint key index and its
//!   value, in the order the map iterates them.
//!
//! Integers are LEB128 varints: seven bits per byte, low bits first, the high
//! bit set on every byte but the last.

use std::collections::HashMap;
use std::str::FromStr;

use serde_json::{Map, Number, Value};

const NULL: u8 = 0;
const FALSE: u8 = 1;
const TRUE: u8 = 2;
const UNSIGNED: u8 = 3;
const NEGATIVE: u8 = 4;
const FLOAT: u8 = 5;
const NUMBER_TEXT: u8 = 6;
const STRING: u8 = 7;
const DIGITS: u8 = 8;
const ARRAY: u8 = 9;
const OBJECT: u8 = 10;
/// Tags from here up are the unsigned integers below
/// [`SMALL_UNSIGNED_LIMIT`]: `tag - SMALL_UNSIGNED`.
const SMALL_UNSIGNED: u8 = 16;
const SMALL_UNSIGNED_LIMIT: u64 = 256 - SMALL_UNSIGNED as u64;

/// Shortest all-digit string worth packing two digits to a byte: shorter
/// ones take no more space as plain strings.
const MIN_PACKED_DIGITS: usize = 3;

const HEADER: usize = 4;

/// One entity row as a single byte buffer. See the [module docs](self).
#[derive(Clone)]
pub(crate) struct PackedRow(Box<[u8]>);

impl PackedRow {
    /// Pack `value`.
    pub(crate) fn pack(value: &Value) -> Self {
        let mut packer = Packer::new();
        packer.value(value);
        PackedRow(packer.finish())
    }

    /// The value packed, rebuilt.
    pub(crate) fn unpack(&self) -> Value {
        let mut reader = Reader::new(&self.0);
        reader.value()
    }

    /// Heap bytes the row takes.
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

impl std::fmt::Debug for PackedRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("PackedRow").field(&self.unpack()).finish()
    }
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

struct Packer<'a> {
    out: Vec<u8>,
    keys: Vec<&'a str>,
    key_ids: HashMap<&'a str, u32>,
}

impl<'a> Packer<'a> {
    fn new() -> Self {
        let mut out = Vec::with_capacity(256);
        out.extend_from_slice(&[0; HEADER]);
        Packer {
            out,
            keys: Vec::new(),
            key_ids: HashMap::new(),
        }
    }

    fn key_id(&mut self, key: &'a str) -> u32 {
        if let Some(id) = self.key_ids.get(key) {
            return *id;
        }
        let id = self.keys.len() as u32;
        self.keys.push(key);
        self.key_ids.insert(key, id);
        id
    }

    fn value(&mut self, value: &'a Value) {
        match value {
            Value::Null => self.out.push(NULL),
            Value::Bool(false) => self.out.push(FALSE),
            Value::Bool(true) => self.out.push(TRUE),
            Value::Number(number) => self.number(number),
            Value::String(string) => self.string(string),
            Value::Array(items) => {
                self.out.push(ARRAY);
                put_varint(&mut self.out, items.len() as u64);
                // Elements of one array are usually objects of one shape, so
                // each predicts the next one's keys.
                let mut shape = Vec::new();
                for item in items {
                    match item {
                        Value::Object(fields) => self.object(fields, Some(&mut shape)),
                        _ => self.value(item),
                    }
                }
            }
            Value::Object(fields) => self.object(fields, None),
        }
    }

    /// An object. In an array, `shape` holds the keys of the object packed
    /// before it: their ids are reused where the keys match, and this
    /// object's keys are left there for the next.
    fn object(
        &mut self,
        fields: &'a Map<String, Value>,
        mut shape: Option<&mut Vec<(&'a str, u32)>>,
    ) {
        self.out.push(OBJECT);
        put_varint(&mut self.out, fields.len() as u64);
        for (index, (key, value)) in fields.iter().enumerate() {
            let id = match shape.as_deref_mut() {
                None => self.key_id(key),
                Some(shape) => match shape.get(index) {
                    Some((predicted, id)) if *predicted == key.as_str() => *id,
                    _ => {
                        let id = self.key_id(key);
                        shape.truncate(index);
                        shape.push((key, id));
                        id
                    }
                },
            };
            put_varint(&mut self.out, u64::from(id));
            self.value(value);
        }
        if let Some(shape) = shape {
            shape.truncate(fields.len());
        }
    }

    fn number(&mut self, number: &Number) {
        // Each form is used only if it gives back the very same number, so
        // packing stays exact whatever serde_json features are enabled.
        if let Some(unsigned) = number.as_u64().filter(|u| Number::from(*u) == *number) {
            if unsigned < SMALL_UNSIGNED_LIMIT {
                self.out.push(SMALL_UNSIGNED + unsigned as u8);
            } else {
                self.out.push(UNSIGNED);
                put_varint(&mut self.out, unsigned);
            }
        } else if let Some(negative) = number
            .as_i64()
            .filter(|i| *i < 0 && Number::from(*i) == *number)
        {
            self.out.push(NEGATIVE);
            put_varint(&mut self.out, !(negative as u64));
        } else if let Some(float) = number
            .as_f64()
            .filter(|f| Number::from_f64(*f).as_ref() == Some(number))
        {
            self.out.push(FLOAT);
            self.out.extend_from_slice(&float.to_le_bytes());
        } else {
            let text = number.to_string();
            self.out.push(NUMBER_TEXT);
            put_varint(&mut self.out, text.len() as u64);
            self.out.extend_from_slice(text.as_bytes());
        }
    }

    fn string(&mut self, string: &str) {
        let bytes = string.as_bytes();
        if bytes.len() >= MIN_PACKED_DIGITS && bytes.iter().all(u8::is_ascii_digit) {
            self.out.push(DIGITS);
            put_varint(&mut self.out, bytes.len() as u64);
            let (pairs, last) = bytes.as_chunks::<2>();
            for [high, low] in pairs {
                self.out.push((high - b'0') << 4 | (low - b'0'));
            }
            if let [last] = last {
                self.out.push((last - b'0') << 4);
            }
        } else {
            self.out.push(STRING);
            put_varint(&mut self.out, bytes.len() as u64);
            self.out.extend_from_slice(bytes);
        }
    }

    fn finish(mut self) -> Box<[u8]> {
        let dictionary = u32::try_from(self.out.len()).expect("a packed row is under 4 GiB");
        self.out[..HEADER].copy_from_slice(&dictionary.to_le_bytes());
        put_varint(&mut self.out, self.keys.len() as u64);
        for key in &self.keys {
            put_varint(&mut self.out, key.len() as u64);
            self.out.extend_from_slice(key.as_bytes());
        }
        self.out.into_boxed_slice()
    }
}

/// Reads bytes written by [`Packer`]. Anything else is a bug, so a
/// malformed row panics rather than returning an error.
struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
    keys: Vec<&'a str>,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        let mut header = [0; HEADER];
        header.copy_from_slice(&bytes[..HEADER]);
        let mut reader = Reader {
            bytes,
            position: u32::from_le_bytes(header) as usize,
            keys: Vec::new(),
        };
        let count = reader.length();
        reader.keys.reserve_exact(count);
        for _ in 0..count {
            let key = reader.text();
            reader.keys.push(key);
        }
        reader.position = HEADER;
        reader
    }

    fn byte(&mut self) -> u8 {
        let byte = self.bytes[self.position];
        self.position += 1;
        byte
    }

    fn varint(&mut self) -> u64 {
        let mut value = 0;
        let mut shift = 0;
        loop {
            let byte = self.byte();
            value |= u64::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return value;
            }
            shift += 7;
        }
    }

    fn length(&mut self) -> usize {
        self.varint() as usize
    }

    fn take(&mut self, length: usize) -> &'a [u8] {
        let taken = &self.bytes[self.position..self.position + length];
        self.position += length;
        taken
    }

    fn text(&mut self) -> &'a str {
        let length = self.length();
        std::str::from_utf8(self.take(length)).expect("packed text is UTF-8")
    }

    fn digits(&mut self) -> String {
        let count = self.length();
        let mut digits = Vec::with_capacity(count);
        for pair in self.take(count.div_ceil(2)) {
            digits.push(b'0' + (pair >> 4));
            if digits.len() < count {
                digits.push(b'0' + (pair & 0x0f));
            }
        }
        String::from_utf8(digits).expect("ASCII digits are UTF-8")
    }

    fn value(&mut self) -> Value {
        match self.byte() {
            NULL => Value::Null,
            FALSE => Value::Bool(false),
            TRUE => Value::Bool(true),
            UNSIGNED => Value::from(self.varint()),
            NEGATIVE => Value::from(!self.varint() as i64),
            FLOAT => {
                let mut bits = [0; 8];
                bits.copy_from_slice(self.take(8));
                let float = f64::from_le_bytes(bits);
                Value::Number(Number::from_f64(float).expect("packed floats are finite"))
            }
            NUMBER_TEXT => {
                let text = self.text();
                Value::Number(Number::from_str(text).expect("packed number text parses"))
            }
            STRING => Value::String(self.text().to_owned()),
            DIGITS => Value::String(self.digits()),
            ARRAY => {
                let count = self.length();
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.value());
                }
                Value::Array(items)
            }
            OBJECT => {
                let count = self.length();
                let mut fields = Vec::with_capacity(count);
                for _ in 0..count {
                    let id = self.length();
                    let key = self.keys[id].to_owned();
                    fields.push((key, self.value()));
                }
                // The fields come in the order they were packed, which is
                // the map's own order, so building from them in one go
                // spares a search per field.
                Value::Object(fields.into_iter().collect::<Map<_, _>>())
            }
            tag if tag >= SMALL_UNSIGNED => Value::from(u64::from(tag - SMALL_UNSIGNED)),
            tag => unreachable!("unknown packed row tag {tag}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn assert_round_trip(value: &Value) -> PackedRow {
        let packed = PackedRow::pack(value);
        let unpacked = packed.unpack();
        assert_eq!(&unpacked, value);
        assert_eq!(
            serde_json::to_string(&unpacked).unwrap(),
            serde_json::to_string(value).unwrap(),
            "the unpacked value serializes byte for byte as the original"
        );
        packed
    }

    #[test]
    fn scalars_round_trip_exactly() {
        for value in [
            json!(null),
            json!(true),
            json!(false),
            json!(0),
            json!(239),
            json!(240),
            json!(255),
            json!(u64::MAX),
            json!(-1),
            json!(i64::MIN),
            json!(0.0),
            json!(-0.0),
            json!(1.5),
            json!(1e300),
            json!(5e-324),
            json!(f64::MAX),
            json!(""),
            json!("a"),
            json!("12"),
            json!("123"),
            json!("0123"),
            json!("000"),
            json!("340282366920938463463374607431768211455"),
            json!("1".repeat(1000)),
            json!("12a"),
            json!("-123"),
            json!("1.5"),
            json!("quote \" backslash \\ newline \n tab \t nul \u{0} é 🦀"),
        ] {
            assert_round_trip(&value);
        }
    }

    #[test]
    fn numbers_keep_their_representation() {
        // Equal as numbers but not as JSON values: an integer never comes
        // back as a float, nor the reverse.
        let float_one = PackedRow::pack(&json!(1.0)).unpack();
        assert!(float_one.is_f64());
        assert_eq!(serde_json::to_string(&float_one).unwrap(), "1.0");
        let integer_one = PackedRow::pack(&json!(1)).unpack();
        assert!(integer_one.is_u64());
        let negative = PackedRow::pack(&json!(-5)).unpack();
        assert!(negative.is_i64() && !negative.is_u64());
    }

    #[test]
    fn containers_round_trip_with_their_order() {
        assert_round_trip(&json!({}));
        assert_round_trip(&json!([]));
        assert_round_trip(&json!([[], {}, [[]], {"": {"": []}}]));
        assert_round_trip(&json!({
            "b": 1,
            "a": [1, "two", {"z": null, "y": [true, false]}],
            "c": {"nested": {"deeper": {"deepest": "123456789"}}},
        }));
        // Objects of varying shapes in one array: key prediction must miss
        // cleanly when the shape changes.
        assert_round_trip(&json!([
            {"a": 1, "b": 2},
            {"a": 1, "b": 2},
            {"a": 1, "c": 3},
            {"b": 2},
            {"a": 1, "b": 2, "c": 3, "d": 4},
            {},
            5,
            {"a": 1, "b": 2},
        ]));
    }

    #[test]
    fn repeated_keys_are_stored_once() {
        let records: Vec<Value> = (0..70)
            .map(|index| {
                json!({
                    "fee_x": format!("{}", 1_000_000_007u64 * index),
                    "fee_y": "0",
                    "pending_x": index,
                    "pending_y": index * 2,
                })
            })
            .collect();
        let row = json!({ "fees": records });
        let packed = assert_round_trip(&row);
        let json = serde_json::to_vec(&row).unwrap();
        assert!(
            packed.0.len() * 3 < json.len(),
            "{} packed bytes for {} bytes of JSON",
            packed.0.len(),
            json.len()
        );
        let occurrences = packed
            .0
            .windows("pending_x".len())
            .filter(|window| *window == b"pending_x")
            .count();
        assert_eq!(occurrences, 1);
    }
}
