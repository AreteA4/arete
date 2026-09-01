"""Borsh argument serializer byte vectors.

Ported from typescript/core/src/instructions/instructions.test.ts
(serializeInstructionData suite) and
rust/arete-a4-sdk/src/instruction/serializer.rs tests, so all three
serializers are proven byte-identical.
"""

from __future__ import annotations

import struct

import pytest

from arete.instructions import ArgSchema, InstructionError, serialize_args

SYSTEM_PROGRAM = "11111111111111111111111111111111"


def ser(schema, args):
    return serialize_args(b"", args, schema)


class TestPrimitives:
    def test_prefixes_the_discriminator_and_serializes_primitives(self):
        schema = [
            ArgSchema("amount", "u64"),
            ArgSchema("flag", "bool"),
            ArgSchema("count", "u8"),
        ]
        data = serialize_args(
            bytes([0xAA, 0xBB]), {"amount": 1, "flag": True, "count": 7}, schema
        )
        assert list(data) == [0xAA, 0xBB, 1, 0, 0, 0, 0, 0, 0, 0, 1, 7]

    def test_serializes_u64_from_decimal_strings(self):
        schema = [ArgSchema("amount", "u64")]
        assert ser(schema, {"amount": "18446744073709551615"}) == b"\xff" * 8
        assert list(ser(schema, {"amount": "1"})) == [1, 0, 0, 0, 0, 0, 0, 0]
        with pytest.raises(InstructionError, match="out of range"):
            ser(schema, {"amount": "18446744073709551616"})
        with pytest.raises(InstructionError, match="out of range"):
            ser(schema, {"amount": -1})

    def test_serializes_negative_i64_i128_in_twos_complement(self):
        schema = [ArgSchema("small", "i64"), ArgSchema("big", "i128")]
        data = ser(schema, {"small": -1, "big": "-1"})
        assert data == b"\xff" * 24

        low = ser(
            schema,
            {"small": -(2**63), "big": -(2**127)},
        )
        assert list(low[:8]) == [0, 0, 0, 0, 0, 0, 0, 0x80]
        assert list(low[8:24]) == [0] * 15 + [0x80]

    def test_serializes_large_u128_values(self):
        schema = [ArgSchema("huge", "u128")]
        assert ser(schema, {"huge": (1 << 128) - 1}) == b"\xff" * 16
        assert ser(schema, {"huge": "340282366920938463463374607431768211455"}) == b"\xff" * 16
        assert ser(schema, {"huge": 1})[0] == 1

    def test_serializes_strings_with_a_length_prefix(self):
        schema = [ArgSchema("s", "string")]
        assert list(ser(schema, {"s": "hi"})) == [2, 0, 0, 0, 0x68, 0x69]

    def test_serializes_f32_f64_little_endian_and_bytes_with_length_prefix(self):
        schema = [
            ArgSchema("ratio", "f32"),
            ArgSchema("price", "f64"),
            ArgSchema("blob", "bytes"),
        ]
        data = ser(schema, {"ratio": 1.5, "price": 2.5, "blob": bytes([9, 8])})
        assert data[:4] == struct.pack("<f", 1.5)
        assert data[4:12] == struct.pack("<d", 2.5)
        assert list(data[12:]) == [2, 0, 0, 0, 9, 8]  # u32 len + raw

        # bytes accepts plain integer lists too.
        from_list = ser(schema, {"ratio": 0, "price": 0, "blob": [1]})
        assert list(from_list[12:]) == [1, 0, 0, 0, 1]

    def test_rejects_invalid_bytes_values(self):
        schema = [ArgSchema("blob", "bytes")]
        with pytest.raises(InstructionError, match="Cannot serialize bytes"):
            ser(schema, {"blob": "nope"})
        with pytest.raises(InstructionError, match="0..=255"):
            ser(schema, {"blob": [256]})

    def test_serializes_bools_strictly(self):
        schema = [ArgSchema("flag", "bool")]
        assert list(ser(schema, {"flag": False})) == [0]
        with pytest.raises(InstructionError, match="expected a boolean"):
            ser(schema, {"flag": 1})

    def test_rejects_fractional_and_out_of_range_small_integers(self):
        schema = [ArgSchema("count", "u8")]
        with pytest.raises(InstructionError, match="expected an integer"):
            ser(schema, {"count": 1.5})
        with pytest.raises(InstructionError, match="expected an integer"):
            ser(schema, {"count": True})
        with pytest.raises(InstructionError, match="out of range"):
            ser(schema, {"count": 256})
        with pytest.raises(InstructionError, match="out of range"):
            ser(schema, {"count": -1})


class TestPubkey:
    def test_serializes_a_pubkey_from_a_base58_string_into_32_bytes(self):
        schema = [ArgSchema("mint", "pubkey")]
        assert ser(schema, {"mint": SYSTEM_PROGRAM}) == b"\x00" * 32

    def test_serializes_pubkeys_from_raw_bytes_or_integer_lists(self):
        schema = [ArgSchema("mint", "pubkey")]
        assert ser(schema, {"mint": bytes(32)}) == b"\x00" * 32
        assert ser(schema, {"mint": [0] * 32}) == b"\x00" * 32

    def test_rejects_pubkeys_that_do_not_decode_to_32_bytes(self):
        schema = [ArgSchema("mint", "pubkey")]
        with pytest.raises(InstructionError, match="Invalid pubkey"):
            ser(schema, {"mint": "abc"})
        with pytest.raises(InstructionError, match="expected 32"):
            ser(schema, {"mint": bytes(31)})
        with pytest.raises(InstructionError, match="Invalid pubkey"):
            ser(schema, {"mint": 42})


class TestContainers:
    def test_serializes_option_none_and_some(self):
        schema = [ArgSchema("maybe", {"option": "u8"})]
        assert list(ser(schema, {})) == [0]
        assert list(ser(schema, {"maybe": None})) == [0]
        assert list(ser(schema, {"maybe": 9})) == [1, 9]

    def test_serializes_vec_with_a_length_prefix(self):
        schema = [ArgSchema("v", {"vec": "u8"})]
        assert list(ser(schema, {"v": [1, 2]})) == [2, 0, 0, 0, 1, 2]

        u16s = [ArgSchema("v", {"vec": "u16"})]
        assert list(ser(u16s, {"v": [1, 258]})) == [2, 0, 0, 0, 1, 0, 2, 1]

    def test_serializes_vec_u64_len_with_an_eight_byte_prefix(self):
        # Pinned to the Rust and TypeScript tests of the same shape.
        schema = [ArgSchema("v", {"vecU64Len": "u16"})]
        assert list(ser(schema, {"v": [1, 258]})) == [2, 0, 0, 0, 0, 0, 0, 0, 1, 0, 2, 1]
        assert list(ser(schema, {"v": []})) == [0, 0, 0, 0, 0, 0, 0, 0]

    def test_serializes_fixed_arrays_without_a_prefix_and_checks_length(self):
        schema = [ArgSchema("a", {"array": ("u8", 3)})]
        assert list(ser(schema, {"a": [4, 5, 6]})) == [4, 5, 6]
        with pytest.raises(InstructionError, match="length mismatch"):
            ser(schema, {"a": [1]})

    def test_serializes_tuples_without_a_prefix_and_checks_shape(self):
        schema = [ArgSchema("pair", {"tuple": ("u8", "u16")})]
        assert list(ser(schema, {"pair": [5, 258]})) == [5, 2, 1]
        assert list(ser(schema, {"pair": (5, 258)})) == [5, 2, 1]
        with pytest.raises(InstructionError, match="Tuple length mismatch"):
            ser(schema, {"pair": [5]})
        with pytest.raises(InstructionError, match="expected an array for tuple"):
            ser(schema, {"pair": {"first": 5}})

    def test_serializes_tuples_nested_inside_vectors(self):
        schema = [
            ArgSchema(
                "checks",
                {
                    "vec": {
                        "tuple": (
                            "u8",
                            {"struct": [{"name": "flags", "type": "u32"}]},
                        )
                    }
                },
            )
        ]
        assert list(ser(schema, {"checks": [[1, {"flags": 0x01020304}]]})) == [
            1,
            0,
            0,
            0,
            1,
            4,
            3,
            2,
            1,
        ]

    def test_serializes_structs_in_field_order_including_nesting(self):
        schema = [
            ArgSchema(
                "data",
                {
                    "struct": [
                        {"name": "amount", "type": "u64"},
                        {
                            "name": "inner",
                            "type": {"struct": [{"name": "flag", "type": "bool"}]},
                        },
                    ]
                },
            )
        ]
        # Keys intentionally out of schema order.
        data = ser(schema, {"data": {"inner": {"flag": True}, "amount": 3}})
        assert list(data) == [3, 0, 0, 0, 0, 0, 0, 0, 1]

    def test_rejects_structs_with_missing_required_fields(self):
        schema = [ArgSchema("data", {"struct": [{"name": "amount", "type": "u64"}]})]
        with pytest.raises(
            InstructionError, match='Missing required struct field "amount"'
        ):
            ser(schema, {"data": {}})

    def test_option_struct_fields_may_be_omitted(self):
        schema = [
            ArgSchema(
                "data",
                {"struct": [{"name": "maybe", "type": {"option": "u8"}}]},
            )
        ]
        assert list(ser(schema, {"data": {}})) == [0]


class TestHashMap:
    def test_serializes_string_key_maps_in_deterministic_key_order(self):
        schema = [ArgSchema("labels", {"hashMap": ("string", "u8")})]
        left = ser(schema, {"labels": {"z": 1, "a": 2}})
        right = ser(schema, {"labels": {"a": 2, "z": 1}})
        assert left == right
        assert list(left) == [2, 0, 0, 0, 1, 0, 0, 0, 0x61, 2, 1, 0, 0, 0, 0x7A, 1]

    def test_serializes_string_key_maps_with_string_values(self):
        schema = [ArgSchema("metadata", {"hashMap": ("string", "string")})]
        data = ser(schema, {"metadata": {"b": "two", "a": "one"}})
        assert list(data) == [
            2, 0, 0, 0,
            1, 0, 0, 0, 0x61,
            3, 0, 0, 0, 0x6F, 0x6E, 0x65,
            1, 0, 0, 0, 0x62,
            3, 0, 0, 0, 0x74, 0x77, 0x6F,
        ]

    def test_serializes_nested_metaplex_style_authorization_payload_maps(self):
        schema = [
            ArgSchema(
                "authorizationData",
                {
                    "struct": [
                        {
                            "name": "payload",
                            "type": {
                                "struct": [
                                    {
                                        "name": "map",
                                        "type": {
                                            "hashMap": (
                                                "string",
                                                {
                                                    "enum": [
                                                        {"name": "Pubkey", "tuple": ["pubkey"]},
                                                        {"name": "Number", "tuple": ["u64"]},
                                                    ]
                                                },
                                            )
                                        },
                                    }
                                ]
                            },
                        }
                    ]
                },
            )
        ]
        data = ser(
            schema,
            {
                "authorizationData": {
                    "payload": {
                        "map": {
                            "b": {"Number": [7]},
                            "a": {"Pubkey": [SYSTEM_PROGRAM]},
                        }
                    }
                }
            },
        )
        expected = [2, 0, 0, 0, 1, 0, 0, 0, 0x61, 0]
        expected += [0] * 32
        expected += [1, 0, 0, 0, 0x62, 1, 7, 0, 0, 0, 0, 0, 0, 0]
        assert list(data) == expected

    def test_rejects_invalid_map_inputs_and_unsupported_key_schemas(self):
        schema = [ArgSchema("labels", {"hashMap": ("string", "u8")})]
        with pytest.raises(InstructionError, match="must be a plain object"):
            ser(schema, {"labels": ["a"]})

        bad_key = [ArgSchema("labels", {"hashMap": ("u64", "u8")})]
        with pytest.raises(InstructionError, match="'string' schema"):
            ser(bad_key, {"labels": {"a": 1}})


class TestEnum:
    OP_SCHEMA = [
        ArgSchema(
            "op",
            {
                "enum": [
                    "noop",
                    {"name": "transfer", "fields": [{"name": "amount", "type": "u64"}]},
                    {"name": "pair", "tuple": ["u8", "u16"]},
                ]
            },
        )
    ]

    def test_serializes_fieldless_enums_by_name_or_index(self):
        schema = [ArgSchema("status", {"enum": ["active", "sunset"]})]
        assert list(ser(schema, {"status": "sunset"})) == [1]
        assert list(ser(schema, {"status": 0})) == [0]
        with pytest.raises(InstructionError, match='Unknown enum variant "paused"'):
            ser(schema, {"status": "paused"})
        with pytest.raises(InstructionError, match="out of range"):
            ser(schema, {"status": 2})

    def test_serializes_data_carrying_enum_variants(self):
        assert list(ser(self.OP_SCHEMA, {"op": {"transfer": {"amount": 7}}})) == [
            1, 7, 0, 0, 0, 0, 0, 0, 0,
        ]
        assert list(ser(self.OP_SCHEMA, {"op": {"pair": [5, 258]}})) == [2, 5, 2, 1]

    def test_rejects_malformed_enum_values(self):
        with pytest.raises(InstructionError, match="carries data"):
            ser(self.OP_SCHEMA, {"op": "transfer"})
        with pytest.raises(InstructionError, match="carries data"):
            ser(self.OP_SCHEMA, {"op": 1})
        with pytest.raises(InstructionError, match="is fieldless"):
            ser(self.OP_SCHEMA, {"op": {"noop": {}}})
        with pytest.raises(InstructionError, match="tuple of length 2"):
            ser(self.OP_SCHEMA, {"op": {"pair": [5]}})
        with pytest.raises(InstructionError, match="single-key object"):
            ser(self.OP_SCHEMA, {"op": {"transfer": {}, "pair": []}})
        with pytest.raises(InstructionError, match="Cannot serialize enum"):
            ser(self.OP_SCHEMA, {"op": True})


class TestFailClosed:
    def test_rejects_missing_required_arguments(self):
        schema = [ArgSchema("amount", "u64")]
        for params in [{}, {"amount": None}]:
            with pytest.raises(
                InstructionError, match='Missing required argument "amount"'
            ):
                ser(schema, params)
