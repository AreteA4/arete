"""Tests for arete.chain (port of typescript/core/src/chain.test.ts plus
route-shape coverage for the ten chain routes)."""

from __future__ import annotations

import base64
import json
from typing import Any, Callable, List

import httpx
import pytest

from arete.chain import (
    ChainClock,
    ChainError,
    HttpChainClient,
    MintAccountInfo,
    NativeBalanceInfo,
    RawAccountInfo,
    TokenAccountInfo,
    TokenBalanceInfo,
)
from arete.http import HttpAuthClient

BASE = "https://example.invalid"


def make_chain(handler: Callable[[httpx.Request], httpx.Response]):
    requests: List[httpx.Request] = []

    def recording(request: httpx.Request) -> httpx.Response:
        requests.append(request)
        return handler(request)

    auth_client = HttpAuthClient(
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(recording))
    )
    return HttpChainClient(BASE, auth_client), requests


@pytest.mark.asyncio
async def test_lamports_keeps_numeric_api_and_route():
    chain, requests = make_chain(lambda r: httpx.Response(200, json={"lamports": 1_461_600}))
    assert await chain.lamports("owner") == 1_461_600
    assert str(requests[0].url) == f"{BASE}/chain/lamports/owner"
    assert requests[0].method == "GET"


@pytest.mark.asyncio
async def test_exists_route():
    chain, requests = make_chain(lambda r: httpx.Response(200, json={"exists": True}))
    assert await chain.exists("acc/ount") is True
    assert str(requests[0].url) == f"{BASE}/chain/exists/acc%2Fount"


@pytest.mark.asyncio
async def test_native_balance_serializes_min_context_slot_and_parses_u64_strings():
    def handler(request: httpx.Request) -> httpx.Response:
        assert json.loads(request.content) == {
            "address": "owner",
            "minContextSlot": "9007199254740997",
        }
        return httpx.Response(
            200,
            json={"lamports": "9007199254740993", "contextSlot": "9007199254740995"},
        )

    chain, requests = make_chain(handler)
    result = await chain.native_balance("owner", min_context_slot=9_007_199_254_740_997)
    assert result == NativeBalanceInfo(
        lamports=9_007_199_254_740_993, context_slot=9_007_199_254_740_995
    )
    assert str(requests[0].url) == f"{BASE}/chain/native-balance"
    assert requests[0].method == "POST"
    assert requests[0].headers["content-type"] == "application/json"


@pytest.mark.asyncio
async def test_balance_preserves_raw_amount_and_parses_context_slot():
    def handler(request: httpx.Request) -> httpx.Response:
        assert json.loads(request.content) == {
            "owner": "owner",
            "mint": "mint",
            "tokenProgram": "token-program",
            "minContextSlot": "9007199254740997",
        }
        return httpx.Response(
            200,
            json={
                "exists": True,
                "address": "token-account",
                "owner": "owner",
                "mint": "mint",
                "tokenProgram": "token-program",
                "amount": "18446744073709551615",
                "decimals": 9,
                "uiAmountString": "18446744073.709551615",
                "contextSlot": "9007199254740999",
            },
        )

    chain, _ = make_chain(handler)
    result = await chain.balance(
        owner="owner",
        mint="mint",
        token_program="token-program",
        min_context_slot=9_007_199_254_740_997,
    )
    assert result == TokenBalanceInfo(
        exists=True,
        address="token-account",
        owner="owner",
        mint="mint",
        token_program="token-program",
        amount="18446744073709551615",
        decimals=9,
        ui_amount_string="18446744073.709551615",
        context_slot=9_007_199_254_740_999,
    )


@pytest.mark.asyncio
async def test_invalid_min_context_slot_is_rejected_before_fetching():
    chain, requests = make_chain(lambda r: httpx.Response(200, json={}))
    with pytest.raises(ValueError, match="minContextSlot"):
        await chain.native_balance("owner", min_context_slot=-1)
    with pytest.raises(ValueError, match="minContextSlot"):
        await chain.native_balance("owner", min_context_slot=2**64)
    assert requests == []


@pytest.mark.asyncio
async def test_rent_exemption_route():
    chain, requests = make_chain(lambda r: httpx.Response(200, json={"lamports": 890880}))
    assert await chain.minimum_balance_for_rent_exemption(128) == 890880
    assert str(requests[0].url) == f"{BASE}/chain/rent-exemption/128"


@pytest.mark.asyncio
async def test_clock_route():
    chain, requests = make_chain(
        lambda r: httpx.Response(
            200,
            json={"slot": 123, "epoch": 5, "leaderScheduleEpoch": 6, "unixTimestamp": 1_700_000_000},
        )
    )
    assert await chain.clock() == ChainClock(
        slot=123, epoch=5, leader_schedule_epoch=6, unix_timestamp=1_700_000_000
    )
    assert str(requests[0].url) == f"{BASE}/chain/clock"

    chain, _ = make_chain(
        lambda r: httpx.Response(200, json={"slot": 1, "unixTimestamp": 2})
    )
    assert await chain.clock() == ChainClock(slot=1, unix_timestamp=2)


@pytest.mark.asyncio
async def test_account_decodes_base64_and_null():
    data = bytes(range(8))
    encoded = base64.b64encode(data).decode()
    chain, requests = make_chain(
        lambda r: httpx.Response(
            200,
            json={
                "address": "addr",
                "ownerProgram": "owner-prog",
                "lamports": 5,
                "executable": False,
                "data": encoded,
            },
        )
    )
    assert await chain.account("addr") == RawAccountInfo(
        address="addr", owner_program="owner-prog", lamports=5, executable=False, data=data
    )
    assert str(requests[0].url) == f"{BASE}/chain/accounts/addr"

    chain, _ = make_chain(lambda r: httpx.Response(200, content=b"null"))
    assert await chain.account("missing") is None

    chain, _ = make_chain(
        lambda r: httpx.Response(
            200,
            json={
                "address": "a",
                "ownerProgram": "p",
                "lamports": 0,
                "executable": False,
                "data": "!!!not-base64!!!",
            },
        )
    )
    with pytest.raises(ChainError, match="base64"):
        await chain.account("a")


@pytest.mark.asyncio
async def test_accounts_batch_maps_items_positionally():
    data = bytes(range(4))
    encoded = base64.b64encode(data).decode()

    def handler(request: httpx.Request) -> httpx.Response:
        assert json.loads(request.content) == {"addresses": ["addr", "missing"]}
        return httpx.Response(
            200,
            json={
                "items": [
                    {
                        "address": "addr",
                        "ownerProgram": "owner-prog",
                        "lamports": 7,
                        "executable": False,
                        "data": encoded,
                    },
                    None,
                ]
            },
        )

    chain, requests = make_chain(handler)
    assert await chain.accounts(["addr", "missing"]) == [
        RawAccountInfo(
            address="addr",
            owner_program="owner-prog",
            lamports=7,
            executable=False,
            data=data,
        ),
        None,
    ]
    assert str(requests[0].url) == f"{BASE}/chain/accounts"
    assert requests[0].method == "POST"
    assert requests[0].headers["content-type"] == "application/json"


@pytest.mark.asyncio
async def test_accounts_batch_bounds_are_resolved_before_fetching():
    chain, requests = make_chain(lambda r: httpx.Response(200, json={"items": []}))
    assert await chain.accounts([]) == []
    with pytest.raises(ValueError, match="100-address"):
        await chain.accounts([f"addr{i}" for i in range(101)])
    assert requests == []


@pytest.mark.asyncio
async def test_accounts_batch_rejects_a_cardinality_mismatch():
    chain, _ = make_chain(lambda r: httpx.Response(200, json={"items": [None]}))
    with pytest.raises(ChainError, match="expected 2 items, got 1"):
        await chain.accounts(["addr1", "addr2"])


@pytest.mark.asyncio
async def test_mint_and_token_account_routes():
    chain, requests = make_chain(
        lambda r: httpx.Response(
            200,
            json={
                "address": "mint",
                "ownerProgram": "tok",
                "decimals": 6,
                "supply": "1000",
                "mintAuthority": None,
                "freezeAuthority": None,
            },
        )
    )
    assert await chain.mint("mint") == MintAccountInfo(
        address="mint", owner_program="tok", decimals=6, supply="1000"
    )
    assert str(requests[0].url) == f"{BASE}/chain/mints/mint"

    chain, _ = make_chain(lambda r: httpx.Response(200, content=b"null"))
    assert await chain.mint("missing") is None

    chain, requests = make_chain(
        lambda r: httpx.Response(
            200,
            json={
                "address": "ta",
                "ownerProgram": "tok",
                "mint": "mint",
                "owner": "owner",
                "amount": "42",
                "uiAmountString": "0.000042",
            },
        )
    )
    assert await chain.token_account("ta") == TokenAccountInfo(
        address="ta",
        owner_program="tok",
        mint="mint",
        owner="owner",
        amount="42",
        ui_amount_string="0.000042",
    )
    assert str(requests[0].url) == f"{BASE}/chain/token-accounts/ta"

    chain, _ = make_chain(lambda r: httpx.Response(200, content=b"null"))
    assert await chain.token_account("missing") is None


@pytest.mark.asyncio
async def test_http_failures_raise_chain_error_with_status():
    chain, _ = make_chain(
        lambda r: httpx.Response(500, json={"code": "internal-error", "message": "boom"})
    )
    with pytest.raises(ChainError) as info:
        await chain.clock()
    assert info.value.status == 500
    assert info.value.path == "/chain/clock"


@pytest.mark.asyncio
async def test_invalid_u64_strings_raise_chain_error():
    chain, _ = make_chain(
        lambda r: httpx.Response(200, json={"lamports": "12x", "contextSlot": "1"})
    )
    with pytest.raises(ChainError, match="decimal u64"):
        await chain.native_balance("owner")

    chain, _ = make_chain(
        lambda r: httpx.Response(
            200, json={"lamports": str(2**64), "contextSlot": "1"}
        )
    )
    with pytest.raises(ChainError, match="exceeds u64"):
        await chain.native_balance("owner")
