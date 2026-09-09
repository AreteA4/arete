"""Tests for arete.transactions (port of transactions.test.ts plus the
remaining route shapes)."""

from __future__ import annotations

import json
from typing import Callable, List

import httpx
import pytest

from arete.http import UPSTREAM_ATTEMPTED_HEADER, HttpAuthClient
from arete.transactions import (
    HttpTransactionTransport,
    LatestBlockhashResult,
    TransactionFeeResult,
    TransactionSendResult,
    TransactionSignatureStatus,
    TransactionSimulationResult,
    TransactionTransportError,
)

BASE = "https://stack.example/"


def make_transport(handler: Callable[[httpx.Request], httpx.Response]):
    requests: List[httpx.Request] = []

    def recording(request: httpx.Request) -> httpx.Response:
        requests.append(request)
        return handler(request)

    auth_client = HttpAuthClient(
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(recording))
    )
    return HttpTransactionTransport(BASE, auth_client), requests


@pytest.mark.asyncio
async def test_latest_blockhash_serializes_request_and_parses_u64():
    def handler(request: httpx.Request) -> httpx.Response:
        assert str(request.url) == "https://stack.example/transactions/v1/latest-blockhash"
        assert json.loads(request.content) == {
            "commitment": "confirmed",
            "minContextSlot": "42",
        }
        return httpx.Response(
            200,
            json={"blockhash": "blockhash", "contextSlot": "43", "lastValidBlockHeight": "99"},
        )

    transport, _ = make_transport(handler)
    result = await transport.get_latest_blockhash(commitment="confirmed", min_context_slot=42)
    assert result == LatestBlockhashResult(
        blockhash="blockhash", context_slot=43, last_valid_block_height=99
    )


@pytest.mark.asyncio
async def test_send_is_send_scoped_and_not_retried_internally():
    transport, requests = make_transport(
        lambda r: httpx.Response(200, json={"signature": "sig"})
    )
    result = await transport.send_transaction("signed-base64", skip_preflight=True)
    assert result == TransactionSendResult(signature="sig")
    assert len(requests) == 1
    request = requests[0]
    assert str(request.url) == "https://stack.example/transactions/v1/send"
    assert json.loads(request.content) == {
        "transaction": "signed-base64",
        "skipPreflight": True,
    }
    # The predispatch marker is a response header the server sets; the client
    # never sends it (matching TS client.ts:843 / Rust http.rs:909).
    assert UPSTREAM_ATTEMPTED_HEADER.lower() not in request.headers


@pytest.mark.asyncio
async def test_inspect_routes_do_not_carry_the_predispatch_marker():
    transport, requests = make_transport(
        lambda r: httpx.Response(200, json={"blockHeight": "7"})
    )
    assert await transport.get_block_height() == 7
    assert UPSTREAM_ATTEMPTED_HEADER.lower() not in requests[0].headers
    assert json.loads(requests[0].content) == {}


@pytest.mark.asyncio
async def test_transport_error_metadata_is_stable():
    transport, _ = make_transport(
        lambda r: httpx.Response(
            504,
            json={
                "code": "upstream_timeout",
                "message": "Submission outcome is unknown",
                "retryable": False,
                "requestId": "req-1",
                "submissionState": "unknown",
                "signature": "local-sig",
            },
        )
    )
    with pytest.raises(TransactionTransportError) as info:
        await transport.send_transaction("signed")
    error = info.value
    assert error.status == 504
    assert error.code == "upstream_timeout"
    assert error.message == "Submission outcome is unknown"
    assert error.retryable is False
    assert error.request_id == "req-1"
    assert error.submission_state == "unknown"
    assert error.signature == "local-sig"


@pytest.mark.asyncio
async def test_transport_error_synthesizes_defaults_without_reflecting_bodies():
    transport, _ = make_transport(lambda r: httpx.Response(502, text="<html>bad gateway</html>"))
    with pytest.raises(TransactionTransportError) as info:
        await transport.get_latest_blockhash()
    error = info.value
    assert error.status == 502
    assert error.code == "transaction_transport_error"
    assert error.message == "Transaction request failed (502)"
    assert error.retryable is False


@pytest.mark.asyncio
async def test_fee_parses_null_and_decimal_values():
    transport, requests = make_transport(
        lambda r: httpx.Response(200, json={"feeLamports": None, "contextSlot": "10"})
    )
    result = await transport.get_fee_for_message("msg")
    assert result == TransactionFeeResult(fee_lamports=None, context_slot=10)
    assert json.loads(requests[0].content) == {"message": "msg"}

    transport, _ = make_transport(
        lambda r: httpx.Response(200, json={"feeLamports": "5000", "contextSlot": "10"})
    )
    result = await transport.get_fee_for_message("msg", commitment="finalized")
    assert result.fee_lamports == 5000


@pytest.mark.asyncio
async def test_simulate_wraps_accounts_and_parses_result():
    def handler(request: httpx.Request) -> httpx.Response:
        assert str(request.url) == "https://stack.example/transactions/v1/simulate"
        assert json.loads(request.content) == {
            "transaction": "tx",
            "commitment": "processed",
            "accounts": {"addresses": ["a", "b"]},
            "innerInstructions": True,
        }
        return httpx.Response(
            200,
            json={
                "contextSlot": "77",
                "err": None,
                "logs": ["log1"],
                "unitsConsumed": "1200",
                "accounts": [None],
            },
        )

    transport, _ = make_transport(handler)
    result = await transport.simulate_transaction(
        "tx", commitment="processed", accounts=["a", "b"], inner_instructions=True
    )
    assert result == TransactionSimulationResult(
        context_slot=77, err=None, logs=["log1"], units_consumed=1200, accounts=[None]
    )


@pytest.mark.asyncio
async def test_v1_contract_simulate_preserves_loaded_accounts_data_size():
    """Absent, explicit null, "0" and a positive size are four distinct
    outcomes: V1 budget estimation cannot tell "unreported" from "zero"
    unless the distinction survives parsing (contract §1)."""
    for payload, expected in (
        ({}, None),
        ({"loadedAccountsDataSize": None}, None),
        ({"loadedAccountsDataSize": "0"}, 0),
        ({"loadedAccountsDataSize": "65536"}, 65536),
    ):
        transport, _ = make_transport(
            lambda r, payload=payload: httpx.Response(
                200, json={"contextSlot": "77", "unitsConsumed": "1200", **payload}
            )
        )
        result = await transport.simulate_transaction("tx")
        assert result.loaded_accounts_data_size == expected
        assert result.units_consumed == 1200


@pytest.mark.asyncio
async def test_v1_contract_malformed_loaded_accounts_data_size_raises():
    """Malformed values raise the same typed error as a bad unitsConsumed —
    a JSON number, a non-numeric string and a negative are all rejected
    rather than silently dropped."""
    for bad in (65536, "-1", "64k", "", 1.5, True):
        transport, _ = make_transport(
            lambda r, bad=bad: httpx.Response(
                200, json={"contextSlot": "77", "loadedAccountsDataSize": bad}
            )
        )
        with pytest.raises(TransactionTransportError, match="loadedAccountsDataSize"):
            await transport.simulate_transaction("tx")


@pytest.mark.asyncio
async def test_signature_status_null_and_parsed():
    transport, requests = make_transport(
        lambda r: httpx.Response(200, json={"status": None})
    )
    assert await transport.get_signature_status("sig") is None
    assert json.loads(requests[0].content) == {"signature": "sig"}

    transport, _ = make_transport(
        lambda r: httpx.Response(
            200,
            json={"status": {"slot": "55", "confirmationStatus": "confirmed", "err": None}},
        )
    )
    result = await transport.get_signature_status("sig", search_transaction_history=True)
    assert result == TransactionSignatureStatus(
        signature="sig", slot=55, confirmation_status="confirmed", err=None
    )


@pytest.mark.asyncio
async def test_invalid_decimal_fields_raise_transport_error():
    transport, _ = make_transport(
        lambda r: httpx.Response(200, json={"blockHeight": 7})
    )
    with pytest.raises(TransactionTransportError, match="blockHeight"):
        await transport.get_block_height()
