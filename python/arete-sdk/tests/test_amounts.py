"""Tests for arete.amounts (port of typescript/core/src/amounts.test.ts)."""

from __future__ import annotations

from typing import List, Optional

import pytest

from arete.amounts import (
    ResolvedAmount,
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
