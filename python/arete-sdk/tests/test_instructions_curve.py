"""Vendored base58 + ed25519 on-curve check.

Ported from typescript/core/src/instructions/instructions.test.ts (base58
suite) with golden on-curve/off-curve vectors generated from the TS reference
stack (@noble/ed25519 + bs58).
"""

from __future__ import annotations

import pytest

from arete.instructions import decode_base58, encode_base58, is_on_curve

SYSTEM_PROGRAM = "11111111111111111111111111111111"
TOKEN_PROGRAM = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
WSOL_MINT = "So11111111111111111111111111111111111111112"
ATA_PROGRAM = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"

# Real keypair-derived program ids: all lie ON the ed25519 curve.
ON_CURVE_ADDRESSES = [SYSTEM_PROGRAM, TOKEN_PROGRAM, WSOL_MINT, ATA_PROGRAM]

# Derived PDAs (verified against @noble/ed25519): all lie OFF the curve.
OFF_CURVE_ADDRESSES = [
    "GLCzvhmavoYmT1Z4u8afD3G2yo5ro9xcPRnNEYxrtutP",  # ["treasury"] @ TOKEN
    "HqK1X4NqLXxDwgMhTiHwMnfSMh19jdZj2VBmUdPAdBeS",  # ["state", WSOL] @ TOKEN
    "5AtDnwsRPbCgdHszHDXZ6qDKxhqnKgFmFAN963EfKNZV",  # [[1,2,255]] @ TOKEN
    "Cu7NwqCXSmsR5vgGA3Vw9uYVViPi3kQvkbKByVQ8nPY9",  # [] @ SYSTEM
]


class TestBase58:
    def test_decodes_the_system_program_to_32_zero_bytes(self):
        decoded = decode_base58(SYSTEM_PROGRAM)
        assert decoded == b"\x00" * 32

    def test_round_trips_known_program_addresses(self):
        for address in [TOKEN_PROGRAM, WSOL_MINT, SYSTEM_PROGRAM, ATA_PROGRAM]:
            decoded = decode_base58(address)
            assert len(decoded) == 32
            assert encode_base58(decoded) == address

    def test_rejects_invalid_base58_characters(self):
        with pytest.raises(ValueError, match="Invalid base58"):
            decode_base58("0OIl")

    def test_empty_input(self):
        assert decode_base58("") == b""
        assert encode_base58(b"") == ""

    def test_leading_zero_handling(self):
        assert decode_base58("1") == b"\x00"
        assert encode_base58(b"\x00") == "1"
        assert encode_base58(b"\x00\x00\x01") == "112"
        assert decode_base58("112") == b"\x00\x00\x01"


class TestIsOnCurve:
    def test_real_public_keys_are_on_curve(self):
        for address in ON_CURVE_ADDRESSES:
            assert is_on_curve(decode_base58(address)), address

    def test_derived_pdas_are_off_curve(self):
        for address in OFF_CURVE_ADDRESSES:
            assert not is_on_curve(decode_base58(address)), address

    def test_wrong_length_is_not_on_curve(self):
        assert not is_on_curve(b"\x00" * 31)
        assert not is_on_curve(b"\x00" * 33)
        assert not is_on_curve(b"")

    def test_non_canonical_y_is_rejected(self):
        # y = 2^255 - 19 (== p) with the sign bit clear: >= p is non-canonical.
        p = 2**255 - 19
        encoded = p.to_bytes(32, "little")
        assert not is_on_curve(encoded)
