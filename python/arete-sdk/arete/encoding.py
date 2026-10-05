"""Hashing and base58 helpers for extension bundles
(``docs/internal/sdk-core-api.md`` §9).

Extensions use these instead of carrying their own implementations: the
TypeScript SDK's ``encodeBase58``/``decodeBase58`` (with its error text), and
the Keccak-256 and SHA-256 digests Solana programs hash with
(``keccak::hashv``, ``hash::hashv``). Each hash takes its input as parts and
hashes their concatenation, as ``hashv`` does.

Pure Python, standard library only. ``hashlib.sha3_256`` is *not* Keccak-256:
SHA-3 changed the padding, so Keccak-256 is implemented here.
"""

from __future__ import annotations

import hashlib
from typing import List, Union

BytesLike = Union[bytes, bytearray, memoryview]

#: The base58 alphabet Solana addresses use (Bitcoin's).
BASE58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
_BASE58_INDEX = {char: index for index, char in enumerate(BASE58_ALPHABET)}


def decode_base58(text: str) -> bytes:
    """Decodes base58 text (TypeScript ``decodeBase58``). Each leading ``1``
    becomes a leading zero byte; the empty string decodes to no bytes.

    Raises ``ValueError("Invalid base58 character: <c>")``, the TypeScript
    message, for the first character outside :data:`BASE58_ALPHABET`.
    """
    if not text:
        return b""
    number = 0
    for char in text:
        value = _BASE58_INDEX.get(char)
        if value is None:
            raise ValueError("Invalid base58 character: " + char)
        number = number * 58 + value
    body = number.to_bytes((number.bit_length() + 7) // 8, "big") if number else b""
    leading_zeros = 0
    for char in text:
        if char != "1":
            break
        leading_zeros += 1
    return b"\x00" * leading_zeros + body


def encode_base58(data: BytesLike) -> str:
    """Encodes bytes as base58 (TypeScript ``encodeBase58``). Each leading zero
    byte becomes a leading ``1``; no bytes encode as the empty string."""
    data = bytes(data)
    if not data:
        return ""
    number = int.from_bytes(data, "big")
    digits = []
    while number > 0:
        number, remainder = divmod(number, 58)
        digits.append(BASE58_ALPHABET[remainder])
    leading_zeros = 0
    for byte in data:
        if byte != 0:
            break
        leading_zeros += 1
    return "1" * leading_zeros + "".join(reversed(digits))


def sha256(*parts: BytesLike) -> bytes:
    """SHA-256 of the concatenation of ``parts`` (Solana's ``hash::hashv``)."""
    hasher = hashlib.sha256()
    for part in parts:
        hasher.update(part)
    return hasher.digest()


# Keccak-f[1600] over 64-bit lanes, with the original Keccak padding (0x01).
_LANE_MASK = (1 << 64) - 1
_KECCAK256_RATE_BYTES = 136
_KECCAK_ROUND_CONSTANTS = (
    0x0000000000000001, 0x0000000000008082, 0x800000000000808A, 0x8000000080008000,
    0x000000000000808B, 0x0000000080000001, 0x8000000080008081, 0x8000000000008009,
    0x000000000000008A, 0x0000000000000088, 0x0000000080008009, 0x000000008000000A,
    0x000000008000808B, 0x800000000000008B, 0x8000000000008089, 0x8000000000008003,
    0x8000000000008002, 0x8000000000000080, 0x000000000000800A, 0x800000008000000A,
    0x8000000080008081, 0x8000000000008080, 0x0000000080000001, 0x8000000080008008,
)
# Rotation offset of lane (x, y), indexed x + 5y.
_KECCAK_ROTATIONS = (
    0, 1, 62, 28, 27,
    36, 44, 6, 55, 20,
    3, 10, 43, 25, 39,
    41, 45, 15, 21, 8,
    18, 2, 61, 56, 14,
)


def _rotate_left(lane: int, shift: int) -> int:
    if shift == 0:
        return lane
    return ((lane << shift) | (lane >> (64 - shift))) & _LANE_MASK


def _keccak_f1600(state: List[int]) -> None:
    columns = [0] * 5
    rotated = [0] * 25
    for round_constant in _KECCAK_ROUND_CONSTANTS:
        for x in range(5):
            columns[x] = state[x] ^ state[x + 5] ^ state[x + 10] ^ state[x + 15] ^ state[x + 20]
        for x in range(5):
            delta = columns[(x + 4) % 5] ^ _rotate_left(columns[(x + 1) % 5], 1)
            for y in range(0, 25, 5):
                state[x + y] ^= delta
        for x in range(5):
            for y in range(5):
                rotated[y + 5 * ((2 * x + 3 * y) % 5)] = _rotate_left(
                    state[x + 5 * y], _KECCAK_ROTATIONS[x + 5 * y]
                )
        for y in range(0, 25, 5):
            for x in range(5):
                state[x + y] = rotated[x + y] ^ (
                    ~rotated[((x + 1) % 5) + y] & _LANE_MASK & rotated[((x + 2) % 5) + y]
                )
        state[0] ^= round_constant


def keccak256(*parts: BytesLike) -> bytes:
    """Keccak-256 of the concatenation of ``parts``: the original Keccak
    padding that Solana's ``keccak::hashv`` (and Ethereum) use, which is not
    ``hashlib.sha3_256``."""
    message = b"".join(bytes(part) for part in parts)
    length = len(message)
    rate = _KECCAK256_RATE_BYTES
    padded = bytearray(((length + rate) // rate) * rate)
    padded[:length] = message
    padded[length] ^= 0x01
    padded[-1] ^= 0x80

    state = [0] * 25
    for block in range(0, len(padded), rate):
        for lane in range(rate // 8):
            offset = block + lane * 8
            state[lane] ^= int.from_bytes(padded[offset:offset + 8], "little")
        _keccak_f1600(state)

    return b"".join(state[lane].to_bytes(8, "little") for lane in range(4))


__all__ = [
    "BASE58_ALPHABET",
    "decode_base58",
    "encode_base58",
    "keccak256",
    "sha256",
]
