"""Tests for arete.spl (port of typescript/core/src/spl.test.ts).

ATA derivation depends on the ``arete.instructions.pda`` seam and is skipped
until that module lands (``pytest.importorskip``).
"""

from __future__ import annotations

from typing import Optional

import pytest

from arete.chain import MintAccountInfo
from arete.spl import (
    ASSOCIATED_TOKEN_PROGRAM_ADDRESS,
    SPL_TOKEN_PROGRAM_ADDRESS,
    SYSTEM_PROGRAM_ADDRESS,
    TOKEN_2022_PROGRAM_ADDRESS,
    derive_associated_token_account,
    resolve_token_program_address,
)


def test_program_address_constants():
    assert SPL_TOKEN_PROGRAM_ADDRESS == "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
    assert TOKEN_2022_PROGRAM_ADDRESS == "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
    assert ASSOCIATED_TOKEN_PROGRAM_ADDRESS == "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"
    assert SYSTEM_PROGRAM_ADDRESS == "11111111111111111111111111111111"


class ChainStub:
    def __init__(self, mint_result: Optional[MintAccountInfo], fail: bool = False) -> None:
        self._mint_result = mint_result
        self._fail = fail

    async def mint(self, address: str) -> Optional[MintAccountInfo]:
        if self._fail:
            raise AssertionError("unexpected read")
        return self._mint_result


def mint_owned_by(owner_program: str) -> MintAccountInfo:
    return MintAccountInfo(address="mint", owner_program=owner_program)


class TestResolveTokenProgramAddress:
    @pytest.mark.asyncio
    async def test_returns_explicit_override_without_reading_the_mint(self):
        chain = ChainStub(None, fail=True)
        assert (
            await resolve_token_program_address(chain, "mint", "custom-program")
            == "custom-program"
        )

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "owner_program", [SPL_TOKEN_PROGRAM_ADDRESS, TOKEN_2022_PROGRAM_ADDRESS]
    )
    async def test_infers_supported_mint_owner(self, owner_program):
        chain = ChainStub(mint_owned_by(owner_program))
        assert await resolve_token_program_address(chain, "mint") == owner_program

    @pytest.mark.asyncio
    async def test_rejects_missing_mints_and_unsupported_owners(self):
        with pytest.raises(ValueError, match="Mint account not found"):
            await resolve_token_program_address(ChainStub(None), "missing")
        with pytest.raises(ValueError, match="unsupported token program"):
            await resolve_token_program_address(
                ChainStub(mint_owned_by("unsupported")), "mint"
            )


class TestDeriveAssociatedTokenAccount:
    def test_matches_a_known_mainnet_usdc_ata_derivation(self):
        pytest.importorskip("arete.instructions.pda")
        assert (
            derive_associated_token_account(
                owner="So11111111111111111111111111111111111111112",
                mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            )
            == "DHe62eeQVEnNK7vg5xUpDkJm7tuqHadjhvmPRFBG9UPo"
        )

    def test_uses_the_token_program_in_the_pda_seeds(self):
        pytest.importorskip("arete.instructions.pda")
        owner = "So11111111111111111111111111111111111111112"
        mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
        assert derive_associated_token_account(
            owner=owner, mint=mint, token_program=SPL_TOKEN_PROGRAM_ADDRESS
        ) != derive_associated_token_account(
            owner=owner, mint=mint, token_program=TOKEN_2022_PROGRAM_ADDRESS
        )

    def test_rejects_invalid_base58_addresses(self):
        pytest.importorskip("arete.instructions.pda")
        with pytest.raises(ValueError, match="base58"):
            derive_associated_token_account(owner="not base58 0OIl", mint="mint")
