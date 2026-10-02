"""Tests for arete.encoding: Keccak-256, SHA-256 and base58 (the Rust
``arete_sdk::encoding`` vectors)."""

from __future__ import annotations

import hashlib

import pytest

import arete
from arete.encoding import (
    BASE58_ALPHABET,
    decode_base58,
    encode_base58,
    keccak256,
    sha256,
)


def _message(length: int) -> bytes:
    return bytes(index % 256 for index in range(length))


class TestKeccak256:
    def test_matches_known_vectors(self):
        empty = "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        assert keccak256().hex() == empty
        assert keccak256(b"").hex() == empty
        assert keccak256(b"abc").hex() == (
            "4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45"
        )
        assert keccak256(b"The quick brown fox jumps over the lazy dog").hex() == (
            "4d741b6f1eb29cb2a9b9911c82f56fa8d73b04959d3d9d222895df6c0b28aa15"
        )

    @pytest.mark.parametrize(
        "length, digest",
        [
            # Around the 136-byte rate (@noble/hashes keccak_256).
            (135, "cbdfd9dee5faad3818d6b06f95a219fd290b0e1706f6a82e5a595b9ce9faca62"),
            (136, "7ce759f1ab7f9ce437719970c26b0a66ff11fe3e38e17df89cf5d29c7d7f807e"),
            (137, "ac73d4fae68b8453f764007c1a20ce95994187861f0c3227a3a8e99a73a3b1db"),
            (200, "bfb0aa97863e797943cf7c33bb7e880bb4543f3d2703c0923c6901c2af57b890"),
            (272, "fdf2ec49e749960d3c8521a0219af8d03e30e2b3bf19bd16150ee0eaf133d66e"),
            (300, "a679e749a6af300c36e7ff2255d220864eab27b382f9cfdc5aa4d13563ba36ff"),
        ],
    )
    def test_matches_multi_block_vectors(self, length, digest):
        assert keccak256(_message(length)).hex() == digest

    def test_is_not_hashlib_sha3_256(self):
        assert keccak256(b"").hex() != hashlib.sha3_256(b"").hexdigest()

    def test_hashes_the_concatenation_of_its_parts(self):
        message = _message(300)
        for split in (0, 1, 135, 136, 137, 272, 300):
            assert keccak256(message[:split], message[split:]) == keccak256(message)
        assert keccak256(b"The quick ", bytearray(b"brown fox "), memoryview(b"jumps over the lazy dog")) == (
            keccak256(b"The quick brown fox jumps over the lazy dog")
        )

    def test_returns_32_bytes(self):
        digest = keccak256(b"abc")
        assert isinstance(digest, bytes)
        assert len(digest) == 32


class TestSha256:
    def test_matches_known_vectors(self):
        assert sha256().hex() == (
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        )
        assert sha256(b"abc").hex() == (
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        )
        assert sha256(b"a", b"b", b"c") == sha256(b"abc")
        assert sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq").hex() == (
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        )


# Bitcoin Core's base58_encode_decode.json vectors.
BASE58_VECTORS = [
    ("", ""),
    ("61", "2g"),
    ("626262", "a3gV"),
    ("636363", "aPEr"),
    ("73696d706c792061206c6f6e6720737472696e67", "2cFupjhnEsSn59qHXstmK2ffpLv2"),
    ("00eb15231dfceb60925886b67d065299925915aeb172c06647", "1NS17iag9jJgTHD1VXjvLCEnZuQ3rJDE9L"),
    ("516b6fcd0f", "ABnLTmg"),
    ("bf4f89001e670274dd", "3SEo3LWLoPntC"),
    ("572e4794", "3EFU7m"),
    ("ecac89cad93923c02321", "EJDM8drfXA6uyA"),
    ("10c8511e", "Rt5zm"),
    ("00000000000000000000", "1111111111"),
]


class TestBase58:
    @pytest.mark.parametrize("data, text", BASE58_VECTORS)
    def test_round_trips_known_vectors(self, data, text):
        assert encode_base58(bytes.fromhex(data)) == text
        assert decode_base58(text) == bytes.fromhex(data)

    def test_round_trips_solana_addresses(self):
        assert encode_base58(bytes(32)) == "1" * 32
        assert decode_base58("1" * 32) == bytes(32)
        token_program = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
        decoded = decode_base58(token_program)
        assert len(decoded) == 32
        assert encode_base58(decoded) == token_program

    @pytest.mark.parametrize(
        "text, character",
        [("0", "0"), ("abc0O", "0"), ("I", "I"), ("l1", "l"), ("1é0", "é"), ("11+", "+"), (" 1", " ")],
    )
    def test_reports_the_first_invalid_character_as_typescript_does(self, text, character):
        with pytest.raises(ValueError) as raised:
            decode_base58(text)
        assert str(raised.value) == f"Invalid base58 character: {character}"

    def test_alphabet_is_bitcoins(self):
        assert len(BASE58_ALPHABET) == 58
        assert not set("0OIl") & set(BASE58_ALPHABET)


def test_helpers_are_exported_from_arete():
    assert arete.keccak256 is keccak256
    assert arete.sha256 is sha256
    assert arete.encode_base58 is encode_base58
    assert arete.decode_base58 is decode_base58
    for name in ("keccak256", "sha256", "encode_base58", "decode_base58"):
        assert name in arete.__all__


def test_instruction_runtime_shares_the_base58_helpers():
    from arete import instructions

    assert instructions.decode_base58 is decode_base58
    assert instructions.encode_base58 is encode_base58
