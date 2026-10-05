"""Tests for arete.amounts (port of typescript/core/src/amounts.test.ts)."""

from __future__ import annotations

from typing import List, Optional

import pytest

import arete
from arete.amounts import (
    ResolvedAmount,
    decode_amount_input,
    format_raw_to_ui,
    get_mint_decimals,
    parse_ui_amount_to_raw,
    resolve_amount,
    resolve_amount_to_raw,
    resolve_amounts_to_raw,
    to_raw_amount,
)
from arete.chain import MintAccountInfo


class FakeChain:
    def __init__(self, decimals: Optional[int]) -> None:
        self.decimals = decimals
        self.calls: List[str] = []

    async def mint(self, address: str) -> Optional[MintAccountInfo]:
        self.calls.append(address)
        return MintAccountInfo(
            address=address,
            owner_program="TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
            decimals=self.decimals,
        )


class TestParseUiAmountToRaw:
    def test_converts_decimal_strings_without_float_math(self):
        assert parse_ui_amount_to_raw("1.5", 6) == 1_500_000
        assert parse_ui_amount_to_raw("0.000001", 6) == 1
        assert parse_ui_amount_to_raw("100", 6) == 100_000_000
        assert parse_ui_amount_to_raw(0, 6) == 0
        assert parse_ui_amount_to_raw("12345678901234567890", 0) == 12345678901234567890

    def test_accepts_trailing_zero_fraction_digits(self):
        assert parse_ui_amount_to_raw("1.120000000", 6) == 1_120_000

    def test_rejects_malformed_and_negative_inputs(self):
        for bad in ("1.2.3", "abc", "-1", ""):
            with pytest.raises(ValueError, match="Invalid UI amount"):
                parse_ui_amount_to_raw(bad, 6)

    def test_rejects_nonzero_digits_below_mint_precision(self):
        with pytest.raises(ValueError, match="more fractional digits"):
            parse_ui_amount_to_raw("1.1234567", 6)


class TestFormatRawToUi:
    def test_is_the_inverse_of_parse(self):
        assert format_raw_to_ui(1_500_000, 6) == "1.5"
        assert format_raw_to_ui(1, 6) == "0.000001"
        assert format_raw_to_ui(0, 6) == "0"
        assert format_raw_to_ui(100_000_000, 6) == "100"
        assert format_raw_to_ui("2500000", 6) == "2.5"

    def test_handles_zero_decimals_and_negatives(self):
        assert format_raw_to_ui(5, 0) == "5"
        assert format_raw_to_ui(-1_500_000, 6) == "-1.5"


class TestToRawAmount:
    def test_passes_raw_inputs_through(self):
        assert to_raw_amount(42, 6) == 42
        assert to_raw_amount({"raw": "25"}, 6) == 25
        assert to_raw_amount({"raw": 7}, 6) == 7

    def test_scales_ui_inputs(self):
        assert to_raw_amount({"ui": 2}, 6) == 2_000_000
        assert to_raw_amount({"ui": "0.25"}, 8) == 25_000_000

    def test_rejects_invalid_inputs(self):
        with pytest.raises(ValueError):
            to_raw_amount({"ui": "1.2.3"}, 6)
        with pytest.raises(ValueError):
            to_raw_amount({"ui": "1.0000001"}, 6)
        with pytest.raises(ValueError):
            to_raw_amount({"raw": "not-an-integer"}, 6)
        with pytest.raises(ValueError):
            to_raw_amount({"neither": 1}, 6)


class TestUiFloatsReadAsJavaScriptText:
    def test_parse_reads_floats_as_javascript_string_number(self):
        # Python's repr would give "1e+16" and "1e-07".
        assert parse_ui_amount_to_raw(1e16, 0) == 10**16
        assert parse_ui_amount_to_raw(1.5, 9) == 1_500_000_000
        with pytest.raises(ValueError) as raised:
            parse_ui_amount_to_raw(1e-7, 9)
        assert str(raised.value) == "Invalid UI amount: 1e-7"
        with pytest.raises(ValueError) as raised:
            parse_ui_amount_to_raw(1e21, 0)
        assert str(raised.value) == "Invalid UI amount: 1e+21"

    def test_parse_accepts_ascii_digits_only(self):
        # TypeScript's /\d/ is ASCII; Python's matches every Unicode digit.
        for text in ("\uff11", "\u0661.5"):
            with pytest.raises(ValueError, match="Invalid UI amount"):
                parse_ui_amount_to_raw(text, 6)


class TestDecodeAmountInput:
    def test_a_bare_int_is_raw(self):
        assert decode_amount_input(0) == {"raw": 0}
        assert decode_amount_input(5_000_000) == {"raw": 5_000_000}
        assert decode_amount_input(2**128) == {"raw": 2**128}
        # A TypeScript bigint may be negative; Rust's unsigned AmountInput
        # rejects it.
        assert decode_amount_input(-1) == {"raw": -1}

    def test_raw_values_read_as_javascript_bigint_reads_them(self):
        cases = [
            (7, 7),
            ("25", 25),
            ("  42\n", 42),
            ("\ufeff42\u00a0", 42),
            ("+9", 9),
            ("-0", 0),
            ("-5", -5),
            ("007", 7),
            ("0x1F", 31),
            ("0O17", 15),
            ("0b101", 5),
            ("", 0),
            (" ", 0),
            ("340282366920938463463374607431768211455", 2**128 - 1),
            (5.0, 5),
            (1e21, 10**21),
        ]
        for value, expected in cases:
            assert decode_amount_input({"raw": value}) == {"raw": expected}, value

    @pytest.mark.parametrize(
        "value, message",
        [
            ("abc", "Cannot convert abc to a BigInt"),
            ("1.5", "Cannot convert 1.5 to a BigInt"),
            ("1e3", "Cannot convert 1e3 to a BigInt"),
            ("1_000", "Cannot convert 1_000 to a BigInt"),
            ("0x", "Cannot convert 0x to a BigInt"),
            ("-0x1", "Cannot convert -0x1 to a BigInt"),
            ("0b2", "Cannot convert 0b2 to a BigInt"),
            ("+", "Cannot convert + to a BigInt"),
            ("\uff11", "Cannot convert \uff11 to a BigInt"),
            (1.5, "The number 1.5 cannot be converted to a BigInt because it is not an integer"),
            (1e-7, "The number 1e-7 cannot be converted to a BigInt because it is not an integer"),
            (float("nan"), "The number NaN cannot be converted to a BigInt because it is not an integer"),
            (float("inf"), "The number Infinity cannot be converted to a BigInt because it is not an integer"),
        ],
    )
    def test_raw_values_bigint_rejects_raise_its_message(self, value, message):
        with pytest.raises(ValueError) as raised:
            decode_amount_input({"raw": value})
        assert str(raised.value) == message

    def test_ui_values_keep_strings_and_read_numbers_as_javascript_text(self):
        cases = [
            ("1.5", "1.5"),
            (" 0.25 ", " 0.25 "),
            ("one", "one"),
            (2, "2"),
            (-3, "-3"),
            (1.5, "1.5"),
            (0.000001, "0.000001"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (1e16, "10000000000000000"),
            (1e21, "1e+21"),
            (1.2345e25, "1.2345e+25"),
            (-2.0, "-2"),
            (100.0, "100"),
        ]
        for value, text in cases:
            assert decode_amount_input({"ui": value}) == {"ui": text}, value

    def test_ui_text_then_fails_as_typescript_does(self):
        decoded = decode_amount_input({"ui": 1e-7})
        with pytest.raises(ValueError) as raised:
            to_raw_amount(decoded, 9)
        assert str(raised.value) == "Invalid UI amount: 1e-7"

    @pytest.mark.parametrize(
        "value",
        [
            "5",
            1.5,
            True,
            None,
            [1],
            {},
            {"amount": 1},
            {"raw": 1, "ui": "1"},
            {"ui": "1", "decimals": 9},
            {"raw": None},
            {"raw": True},
            {"raw": [1]},
            {"ui": None},
            {"ui": False},
            {"ui": {"value": "1"}},
        ],
    )
    def test_rejects_shapes_typescript_does_not_accept(self, value):
        with pytest.raises(TypeError):
            decode_amount_input(value)

    def test_null_values_render_as_javascript_does(self):
        with pytest.raises(TypeError) as raised:
            decode_amount_input({"raw": None})
        assert str(raised.value) == "Cannot convert null to a BigInt"
        with pytest.raises(TypeError) as raised:
            decode_amount_input({"ui": None})
        assert str(raised.value) == "Invalid UI amount: null"

    def test_decoded_inputs_resolve_with_every_amount_helper(self):
        assert to_raw_amount(decode_amount_input({"ui": "0.005"}), 9) == 5_000_000
        assert to_raw_amount(decode_amount_input({"raw": "0x10"}), 9) == 16
        assert to_raw_amount(decode_amount_input(77), 9) == 77

    def test_is_exported_from_arete(self):
        assert arete.decode_amount_input is decode_amount_input
        assert "decode_amount_input" in arete.__all__
        assert "AmountInput" in arete.__all__


class TestGetMintDecimals:
    @pytest.mark.asyncio
    async def test_returns_decimals_from_the_chain_read(self):
        assert await get_mint_decimals(FakeChain(9), "MintA") == 9

    @pytest.mark.asyncio
    async def test_raises_when_the_mint_has_no_decimals(self):
        with pytest.raises(ValueError, match="missing decimals"):
            await get_mint_decimals(FakeChain(None), "MintA")


class TestResolveAmount:
    @pytest.mark.asyncio
    async def test_never_fetches_when_decimals_are_provided(self):
        chain = FakeChain(6)
        result = await resolve_amount(chain, mint="MintA", amount={"ui": "1.5"}, decimals=6)
        assert result == ResolvedAmount(raw=1_500_000, decimals=6)
        assert chain.calls == []

    @pytest.mark.asyncio
    async def test_fetches_decimals_once_for_ui_inputs_when_unknown(self):
        chain = FakeChain(6)
        result = await resolve_amount(chain, mint="MintA", amount={"ui": "1.5"})
        assert result == ResolvedAmount(raw=1_500_000, decimals=6)
        assert chain.calls == ["MintA"]

    @pytest.mark.asyncio
    async def test_resolves_raw_inputs_without_needing_conversion(self):
        chain = FakeChain(6)
        assert await resolve_amount(
            chain, mint="MintA", amount=123, decimals=6
        ) == ResolvedAmount(raw=123, decimals=6)
        assert await resolve_amount(
            chain, mint="MintA", amount={"raw": "456"}, decimals=6
        ) == ResolvedAmount(raw=456, decimals=6)
        assert chain.calls == []


class TestResolveAmountToRaw:
    @pytest.mark.asyncio
    async def test_never_fetches_when_the_input_is_already_raw(self):
        chain = FakeChain(6)
        assert await resolve_amount_to_raw(chain, mint="MintA", amount=123) == 123
        assert await resolve_amount_to_raw(chain, mint="MintA", amount={"raw": "456"}) == 456
        assert chain.calls == []

    @pytest.mark.asyncio
    async def test_fetches_decimals_for_ui_inputs_when_unknown(self):
        chain = FakeChain(6)
        assert await resolve_amount_to_raw(chain, mint="MintA", amount={"ui": "1.5"}) == 1_500_000
        assert chain.calls == ["MintA"]


class TestResolveAmountsToRaw:
    @pytest.mark.asyncio
    async def test_resolves_named_amounts_and_preserves_keys(self):
        chain = FakeChain(6)
        result = await resolve_amounts_to_raw(
            chain,
            {
                "amount_in": {"mint": "MintA", "amount": {"ui": "1.5"}},
                "minimum_amount_out": {"mint": "MintB", "amount": 42},
            },
        )
        assert result == {"amount_in": 1_500_000, "minimum_amount_out": 42}
        assert chain.calls == ["MintA"]

    @pytest.mark.asyncio
    async def test_returns_an_empty_dict_for_empty_inputs(self):
        chain = FakeChain(6)
        assert await resolve_amounts_to_raw(chain, {}) == {}
        assert chain.calls == []
