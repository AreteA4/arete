"""Tests for arete.rpc, the direct Solana JSON-RPC transport.

Base-install tests: nothing here imports solders. What they pin is the wire
mapping the relay otherwise hides -- JSON numbers instead of decimal strings,
``{context, value}`` envelopes, and error members inside a 200 body.
"""

from __future__ import annotations

import json
from typing import Any, Callable, Dict, List

import httpx
import pytest

from arete.rpc import RpcTransactionTransport
from arete.transactions import (
    LatestBlockhashResult,
    TransactionTransportError,
)

URL = "https://node.example/rpc"
U64_MAX = 0xFFFF_FFFF_FFFF_FFFF


def make_transport(
    results: Dict[str, Any],
    *,
    errors: Dict[str, Any] = None,
    status: int = 200,
    headers: Dict[str, str] = None,
):
    """Transport against a mock node answering by method name."""
    requests: List[Dict[str, Any]] = []
    errors = errors or {}

    def handler(request: httpx.Request) -> httpx.Response:
        body = json.loads(request.content)
        requests.append({**body, "headers": dict(request.headers)})
        method = body["method"]
        if method in errors:
            return httpx.Response(
                status, json={"jsonrpc": "2.0", "id": body["id"], "error": errors[method]}
            )
        if method not in results:
            raise AssertionError(f"unexpected RPC method {method!r}")
        return httpx.Response(
            status, json={"jsonrpc": "2.0", "id": body["id"], "result": results[method]}
        )

    transport = RpcTransactionTransport(
        URL,
        headers=headers,
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(handler)),
    )
    return transport, requests


def params(requests: List[Dict[str, Any]], method: str) -> Any:
    for request in requests:
        if request["method"] == method:
            return request["params"]
    raise AssertionError(f"{method!r} was never called")


@pytest.mark.asyncio
async def test_reads_unwrap_the_context_envelope_and_send_json_numbers():
    transport, requests = make_transport(
        {
            "getLatestBlockhash": {
                "context": {"slot": 43},
                "value": {"blockhash": "Bh1", "lastValidBlockHeight": 99},
            },
            "getBlockHeight": 1234,
        }
    )

    assert await transport.get_latest_blockhash(
        commitment="confirmed", min_context_slot=42
    ) == LatestBlockhashResult(
        blockhash="Bh1", context_slot=43, last_valid_block_height=99
    )
    # Numbers, not the relay's decimal strings.
    assert params(requests, "getLatestBlockhash") == [
        {"commitment": "confirmed", "minContextSlot": 42}
    ]
    assert await transport.get_block_height() == 1234
    assert params(requests, "getBlockHeight") == [{}]


@pytest.mark.asyncio
async def test_u64_results_keep_full_precision():
    transport, _ = make_transport(
        {
            "getFeeForMessage": {"context": {"slot": 1}, "value": U64_MAX},
            "getBlockHeight": U64_MAX - 1,
        }
    )

    fee = await transport.get_fee_for_message("bQ==")
    assert fee.fee_lamports == U64_MAX
    assert await transport.get_block_height() == U64_MAX - 1


@pytest.mark.asyncio
async def test_a_null_fee_is_not_a_zero_fee():
    transport, requests = make_transport(
        {"getFeeForMessage": {"context": {"slot": 1}, "value": None}}
    )

    result = await transport.get_fee_for_message("bWVzc2FnZQ==", commitment="finalized")

    assert result.fee_lamports is None
    assert result.context_slot == 1
    assert params(requests, "getFeeForMessage") == [
        "bWVzc2FnZQ==",
        {"commitment": "finalized"},
    ]


@pytest.mark.asyncio
async def test_simulation_runs_unsigned_and_keeps_absent_metrics_absent():
    transport, requests = make_transport(
        {
            "simulateTransaction": {
                "context": {"slot": 7},
                "value": {
                    "err": None,
                    "logs": ["Program log: hi"],
                    "unitsConsumed": 0,
                },
            }
        }
    )

    result = await transport.simulate_transaction("dHg=", commitment="processed")

    assert params(requests, "simulateTransaction") == [
        "dHg=",
        {"encoding": "base64", "sigVerify": False, "commitment": "processed"},
    ], "inspection simulates placeholder signatures, so verification stays off"
    assert result.context_slot == 7
    assert result.logs == ["Program log: hi"]
    assert result.units_consumed == 0, "a measured zero is a measurement"
    assert result.loaded_accounts_data_size is None, (
        "an unreported metric is absent, never an invented zero"
    )


@pytest.mark.asyncio
async def test_a_decimal_string_metric_is_a_malformed_node_response():
    """The relay encodes u64 as decimal strings; a node does not. Reusing
    that parsing would turn a malformed response into a plausible budget."""
    transport, _ = make_transport(
        {
            "simulateTransaction": {
                "context": {"slot": 7},
                "value": {"unitsConsumed": "1200"},
            }
        }
    )

    with pytest.raises(TransactionTransportError, match="unitsConsumed"):
        await transport.simulate_transaction("dHg=")


@pytest.mark.asyncio
async def test_statuses_align_positionally_and_map_unseen_to_none():
    transport, requests = make_transport(
        {
            "getSignatureStatuses": {
                "context": {"slot": 9},
                "value": [
                    {
                        "slot": 8,
                        "confirmations": None,
                        "err": None,
                        "confirmationStatus": "finalized",
                    },
                    None,
                ],
            }
        }
    )

    statuses = await transport.get_signature_statuses(
        ["sigA", "sigB"], search_transaction_history=True
    )

    assert params(requests, "getSignatureStatuses") == [
        ["sigA", "sigB"],
        {"searchTransactionHistory": True},
    ]
    assert statuses[0].signature == "sigA"
    assert statuses[0].slot == 8
    assert statuses[0].confirmation_status == "finalized"
    assert statuses[1] is None


@pytest.mark.asyncio
async def test_a_single_status_lookup_reads_the_batch_route():
    transport, requests = make_transport(
        {"getSignatureStatuses": {"context": {"slot": 9}, "value": [None]}}
    )

    assert await transport.get_signature_status("sigA") is None
    assert params(requests, "getSignatureStatuses")[0] == ["sigA"]


@pytest.mark.asyncio
async def test_a_length_mismatch_is_refused_rather_than_misattributed():
    transport, _ = make_transport(
        {"getSignatureStatuses": {"context": {"slot": 9}, "value": [None]}}
    )

    with pytest.raises(TransactionTransportError, match="expected 2 statuses"):
        await transport.get_signature_statuses(["sigA", "sigB"])


@pytest.mark.asyncio
async def test_a_send_serializes_its_options_and_returns_the_signature():
    transport, requests = make_transport({"sendTransaction": "sigZ"})

    result = await transport.send_transaction(
        "dHg=", skip_preflight=True, preflight_commitment="confirmed"
    )

    assert result.signature == "sigZ"
    assert params(requests, "sendTransaction") == [
        "dHg=",
        {
            "encoding": "base64",
            "skipPreflight": True,
            "preflightCommitment": "confirmed",
        },
    ]


@pytest.mark.asyncio
async def test_a_send_the_node_refused_before_dispatch_is_typed_not_submitted():
    transport, _ = make_transport(
        {},
        errors={
            "sendTransaction": {
                "code": -32002,
                "message": "Transaction simulation failed",
                "data": {"logs": []},
            }
        },
    )

    with pytest.raises(TransactionTransportError) as caught:
        await transport.send_transaction("dHg=")

    assert caught.value.submission_state == "not_submitted"
    assert caught.value.code == "rpc_-32002"
    assert caught.value.details == {"logs": []}
    assert caught.value.signature is None, (
        "a node never names a signature the caller did not derive"
    )


@pytest.mark.asyncio
async def test_an_unrecognized_send_failure_leaves_the_submission_unknown():
    transport, _ = make_transport(
        {}, errors={"sendTransaction": {"code": -32005, "message": "Node is unhealthy"}}
    )

    with pytest.raises(TransactionTransportError) as caught:
        await transport.send_transaction("dHg=")

    assert caught.value.submission_state is None, (
        "unknown keeps the locally derived signature authoritative upstream"
    )


@pytest.mark.asyncio
async def test_a_gateway_failure_without_an_envelope_is_retryable_and_unknown():
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(503, text="upstream unavailable")

    transport = RpcTransactionTransport(
        URL, http_client=httpx.AsyncClient(transport=httpx.MockTransport(handler))
    )

    with pytest.raises(TransactionTransportError) as caught:
        await transport.send_transaction("dHg=")

    assert caught.value.status == 503
    assert caught.value.code == "rpc_http_error"
    assert caught.value.retryable is True
    assert caught.value.submission_state is None


@pytest.mark.asyncio
async def test_provider_headers_go_to_the_node():
    transport, requests = make_transport(
        {"getBlockHeight": 1}, headers={"x-api-key": "secret"}
    )

    await transport.get_block_height()

    assert requests[0]["headers"]["x-api-key"] == "secret"
