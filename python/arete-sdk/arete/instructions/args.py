"""Borsh-compatible instruction argument serialization.

Python port of the TypeScript serializer (``instructions/serializer.ts``) and
its Rust sibling, with byte-for-byte identical wire output: little-endian
integers, u32 length prefixes for strings/vectors/bytes/maps, a single-byte
prefix for options and enum variant indexes, and map entries sorted by the
key's UTF-8 bytes.

Type schema mirrors the TS ``ArgType`` shape: primitives are strings
(``"u8"`` ... ``"u128"``, ``"i8"`` ... ``"i128"``, ``"f32"``, ``"f64"``,
``"bool"``, ``"string"``, ``"pubkey"``, ``"bytes"``); composites are
single-key dicts (``{"vec": t}`` with a u32 count, ``{"vecU64Len": t}`` with a
u64 count, ``{"option": t}``, ``{"array": (t, n)}``,
``{"tuple": (t, ...)}``, ``{"hashMap": (k, v)}``,
``{"struct": [{"name", "type"}, ...]}``,
``{"enum": [variant, ...]}``). Enum variants are bare strings (fieldless) or
``{"name": ..., "fields": [...]}`` / ``{"name": ..., "tuple": [...]}``.
"""

from __future__ import annotations

import struct
from dataclasses import dataclass
from typing import Any, Mapping, Sequence, Union

from ._curve import decode_base58
from .errors import InstructionError

ArgType = Union[str, Mapping[str, Any]]


@dataclass(frozen=True)
class ArgSchema:
    """One instruction argument: name plus type, in serialization order."""

    name: str
    type: ArgType


_INT_WIDTHS = {
    "u8": (1, False),
    "u16": (2, False),
    "u32": (4, False),
    "u64": (8, False),
    "u128": (16, False),
    "i8": (1, True),
    "i16": (2, True),
    "i32": (4, True),
    "i64": (8, True),
    "i128": (16, True),
}
# Wide integers additionally accept decimal strings (mirror of BigInt(string)
# in TS and the Rust string path).
_WIDE_INTS = frozenset(["u64", "u128", "i64", "i128"])


def serialize_args(
    discriminator: Union[bytes, bytearray, Sequence[int]],
    args: Mapping[str, Any],
    schema: Sequence[ArgSchema],
) -> bytes:
    """Serializes the discriminator plus arguments into instruction data.

    Option fields treat an absent key (or ``None``) as ``None``; every other
    field must be present — silently encoding zeros for a missing arg would
    corrupt instruction data. Unknown-parameter rejection lives in the
    handler's param splitting, not here.
    """
    out = bytearray(bytes(discriminator))
    for field in schema:
        value = args.get(field.name)
        if value is None:
            if _is_option(field.type):
                out.append(0)
                continue
            raise InstructionError(
                f'Missing required argument "{field.name}" (type {field.type!r})'
            )
        _serialize_value(value, field.type, field.name, out)
    return bytes(out)


def _is_option(arg_type: ArgType) -> bool:
    return isinstance(arg_type, Mapping) and "option" in arg_type


def _type_name(value: Any) -> str:
    return type(value).__name__


def _int_value(value: Any, ctx: str, ty: str, wide: bool) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        if wide and isinstance(value, str):
            try:
                return int(value.strip(), 10)
            except ValueError:
                raise InstructionError(
                    f'Invalid value for "{ctx}": cannot convert "{value}" to {ty}'
                ) from None
        raise InstructionError(
            f'Invalid value for "{ctx}": expected an integer for {ty}, '
            f"got {_type_name(value)}"
        )
    return value


def _serialize_int(value: Any, ty: str, ctx: str, out: bytearray) -> None:
    size, signed = _INT_WIDTHS[ty]
    number = _int_value(value, ctx, ty, wide=ty in _WIDE_INTS)
    try:
        out += number.to_bytes(size, "little", signed=signed)
    except OverflowError:
        raise InstructionError(
            f'Invalid value for "{ctx}": value {number} out of range for {ty}'
        ) from None


def _byte_values(value: Any, ctx: str) -> bytes:
    if isinstance(value, (bytes, bytearray)):
        return bytes(value)
    if isinstance(value, (list, tuple)):
        try:
            return bytes(value)
        except (ValueError, TypeError):
            raise InstructionError(
                f'Invalid value for "{ctx}": byte array elements must be '
                "integers in 0..=255"
            ) from None
    raise InstructionError(
        f'Invalid value for "{ctx}": Cannot serialize bytes from value of type '
        f"{_type_name(value)}"
    )


def _serialize_pubkey(value: Any, ctx: str, out: bytearray) -> None:
    if isinstance(value, str):
        try:
            decoded = decode_base58(value)
        except ValueError:
            raise InstructionError(
                f"Invalid pubkey: '{value}' is not valid base58"
            ) from None
        if len(decoded) != 32:
            raise InstructionError(
                f"Invalid pubkey: '{value}' decoded to {len(decoded)} bytes, expected 32"
            )
        out += decoded
        return
    if isinstance(value, (bytes, bytearray, list, tuple)):
        try:
            raw = _byte_values(value, ctx)
        except InstructionError:
            raise InstructionError(
                "Invalid pubkey: pubkey byte arrays must contain integers in 0..=255"
            ) from None
        if len(raw) != 32:
            raise InstructionError(
                f"Invalid pubkey byte length: expected 32, got {len(raw)}"
            )
        out += raw
        return
    raise InstructionError(
        f"Invalid pubkey: cannot serialize pubkey from value of type {_type_name(value)}"
    )


def _sequence_items(value: Any, ctx: str, what: str) -> Sequence[Any]:
    if isinstance(value, (list, tuple)):
        return value
    raise InstructionError(
        f'Invalid value for "{ctx}": expected an array for {what}, '
        f"got {_type_name(value)}"
    )


def _serialize_value(value: Any, arg_type: ArgType, ctx: str, out: bytearray) -> None:
    if isinstance(arg_type, str):
        _serialize_primitive(value, arg_type, ctx, out)
        return
    if not isinstance(arg_type, Mapping):
        raise InstructionError(f"Unknown type: {arg_type!r}")

    if "vec" in arg_type:
        items = _sequence_items(value, ctx, "vec")
        out += struct.pack("<I", len(items))
        for item in items:
            _serialize_value(item, arg_type["vec"], ctx, out)
    elif "vecU64Len" in arg_type:
        items = _sequence_items(value, ctx, "vec")
        out += struct.pack("<Q", len(items))
        for item in items:
            _serialize_value(item, arg_type["vecU64Len"], ctx, out)
    elif "option" in arg_type:
        if value is None:
            out.append(0)
        else:
            out.append(1)
            _serialize_value(value, arg_type["option"], ctx, out)
    elif "array" in arg_type:
        element_type, length = arg_type["array"]
        items = _sequence_items(value, ctx, "array")
        if len(items) != length:
            raise InstructionError(
                f"Array length mismatch: expected {length}, got {len(items)}"
            )
        for item in items:
            _serialize_value(item, element_type, ctx, out)
    elif "tuple" in arg_type:
        element_types = arg_type["tuple"]
        items = _sequence_items(value, ctx, "tuple")
        if len(items) != len(element_types):
            raise InstructionError(
                f"Tuple length mismatch: expected {len(element_types)}, got {len(items)}"
            )
        for position, (item, element_type) in enumerate(zip(items, element_types)):
            _serialize_value(item, element_type, f"{ctx}[{position}]", out)
    elif "hashMap" in arg_type:
        key_type, value_type = arg_type["hashMap"]
        _serialize_hash_map(value, key_type, value_type, ctx, out)
    elif "struct" in arg_type:
        _serialize_struct(value, arg_type["struct"], ctx, out)
    elif "enum" in arg_type:
        _serialize_enum(value, arg_type["enum"], ctx, out)
    else:
        raise InstructionError(f"Unknown type: {arg_type!r}")


def _serialize_primitive(value: Any, ty: str, ctx: str, out: bytearray) -> None:
    if ty in _INT_WIDTHS:
        _serialize_int(value, ty, ctx, out)
    elif ty in ("f32", "f64"):
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise InstructionError(
                f'Invalid value for "{ctx}": expected a number for {ty}, '
                f"got {_type_name(value)}"
            )
        out += struct.pack("<f" if ty == "f32" else "<d", float(value))
    elif ty == "bool":
        if not isinstance(value, bool):
            raise InstructionError(
                f'Invalid value for "{ctx}": expected a boolean, '
                f"got {_type_name(value)}"
            )
        out.append(1 if value else 0)
    elif ty == "string":
        if not isinstance(value, str):
            raise InstructionError(
                f'Invalid value for "{ctx}": expected a string, '
                f"got {_type_name(value)}"
            )
        encoded = value.encode("utf-8")
        out += struct.pack("<I", len(encoded))
        out += encoded
    elif ty == "pubkey":
        _serialize_pubkey(value, ctx, out)
    elif ty == "bytes":
        raw = _byte_values(value, ctx)
        out += struct.pack("<I", len(raw))
        out += raw
    else:
        raise InstructionError(f"Unknown primitive type: {ty}")


def _serialize_hash_map(
    value: Any, key_type: ArgType, value_type: ArgType, ctx: str, out: bytearray
) -> None:
    if key_type != "string":
        raise InstructionError(
            f"Instruction hashMap keys must use the 'string' schema, got {key_type!r}"
        )
    if not isinstance(value, Mapping):
        raise InstructionError(
            f"HashMap value must be a plain object, got {_type_name(value)}"
        )
    # Rust/Borsh sorts String keys by their UTF-8 bytes before serializing.
    entries = sorted(value.items(), key=lambda item: str(item[0]).encode("utf-8"))
    out += struct.pack("<I", len(entries))
    for key, entry in entries:
        _serialize_primitive(key, "string", ctx, out)
        _serialize_value(entry, value_type, f"{ctx}.{key}", out)


def _serialize_struct(
    value: Any, fields: Sequence[Mapping[str, Any]], ctx: str, out: bytearray
) -> None:
    if not isinstance(value, Mapping):
        raise InstructionError(
            f"Struct value must be a plain object, got {_type_name(value)}"
        )
    for field in fields:
        name = field["name"]
        field_type = field["type"]
        field_value = value.get(name)
        if field_value is None:
            if _is_option(field_type):
                out.append(0)
                continue
            raise InstructionError(f'Missing required struct field "{name}"')
        _serialize_value(field_value, field_type, f"{ctx}.{name}", out)


def _variant_name(variant: Any) -> str:
    return variant if isinstance(variant, str) else variant["name"]


def _serialize_enum(value: Any, variants: Sequence[Any], ctx: str, out: bytearray) -> None:
    names = [_variant_name(variant) for variant in variants]

    # A bare variant index (fieldless variants only).
    if isinstance(value, int) and not isinstance(value, bool):
        if value < 0 or value >= len(variants):
            raise InstructionError(
                f"Enum variant index {value} out of range (0..{len(variants) - 1})"
            )
        variant = variants[value]
        if not isinstance(variant, str):
            name = _variant_name(variant)
            raise InstructionError(
                f'Enum variant "{name}" carries data; pass {{ {name}: ... }} '
                "instead of an index"
            )
        out.append(value)
        return

    # A fieldless variant by name.
    if isinstance(value, str):
        if value not in names:
            raise InstructionError(
                f'Unknown enum variant "{value}". Expected one of: {", ".join(names)}'
            )
        index = names.index(value)
        if not isinstance(variants[index], str):
            raise InstructionError(
                f'Enum variant "{value}" carries data; pass {{ {value}: ... }} '
                "instead of a bare name"
            )
        out.append(index)
        return

    # { variantName: payload } for data-carrying variants.
    if isinstance(value, Mapping):
        keys = list(value.keys())
        if len(keys) != 1:
            raise InstructionError(
                "Enum value must be a single-key object ({ variantName: payload }), "
                f"got keys [{', '.join(keys)}]"
            )
        key = keys[0]
        payload = value[key]
        if key not in names:
            raise InstructionError(
                f'Unknown enum variant "{key}". Expected one of: {", ".join(names)}'
            )
        index = names.index(key)
        variant = variants[index]
        if isinstance(variant, str):
            raise InstructionError(
                f'Enum variant "{key}" is fieldless; pass \'{key}\' instead of an object'
            )
        out.append(index)
        if "fields" in variant:
            _serialize_struct(payload, variant["fields"], f"{ctx}.{key}", out)
            return
        types = variant["tuple"]
        if not isinstance(payload, (list, tuple)) or len(payload) != len(types):
            raise InstructionError(
                f'Enum variant "{key}" expects a tuple of length {len(types)}'
            )
        for position, (item, item_type) in enumerate(zip(payload, types)):
            _serialize_value(item, item_type, f"{ctx}.{key}[{position}]", out)
        return

    raise InstructionError(
        f"Cannot serialize enum from value of type {_type_name(value)}"
    )
