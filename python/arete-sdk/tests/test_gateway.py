"""Tests for arete.gateway (port of solana-gateway.test.ts + Rust gateway
validation cases)."""

from __future__ import annotations

import dataclasses
import json
from typing import Any, Dict, List, Optional

import httpx
import pytest

from arete.auth import AuthConfig
from arete.errors import AreteError
from arete.gateway import (
    HostedSolanaGatewayBindings,
    HostedSolanaGatewayCapabilityBinding,
    SolanaGatewayAuthMetadata,
    create_hosted_solana_gateway_transports,
)
from arete.http import UPSTREAM_ATTEMPTED_HEADER
from arete.transactions import TransactionTransportError

GATEWAY_ID = "sgb_00000000000000000000000000000001"
ENDPOINT = "https://solana.example.test/gateway/"
SESSION_ENDPOINT = "https://api.example.test/ws/sessions"


def metadata(scopes, transaction_entitlement_required=False) -> SolanaGatewayAuthMetadata:
    return SolanaGatewayAuthMetadata(
        required=True,
        mode="signed_session",
        session_endpoint=SESSION_ENDPOINT,
        jwks_url="https://api.example.test/.well-known/jwks.json",
        token_transport="bearer",
        audience="arete:solana-gateway",
        target_kind="solana-gateway-binding",
        target_id=GATEWAY_ID,
        scopes=tuple(scopes),
        accepted_key_classes=("publishable", "secret"),
        transaction_entitlement_required=transaction_entitlement_required,
    )


def binding(scopes, transaction_entitlement_required=False) -> HostedSolanaGatewayCapabilityBinding:
    return HostedSolanaGatewayCapabilityBinding(
        endpoint=ENDPOINT,
        auth_policy="signed_session",
        solana_gateway_binding_id=GATEWAY_ID,
        cluster="mainnet-beta",
        region="us-west-1",
        auth=metadata(scopes, transaction_entitlement_required),
    )


def valid_bindings() -> HostedSolanaGatewayBindings:
    return HostedSolanaGatewayBindings(
        chain=binding(["read"]),
        transactions=binding(["transaction:inspect", "transaction:send"], True),
    )


def mutated(bindings: HostedSolanaGatewayBindings, capability: str, **changes) -> HostedSolanaGatewayBindings:
    target = getattr(bindings, capability)
    auth_changes = changes.pop("auth", None)
    if auth_changes:
        changes["auth"] = dataclasses.replace(target.auth, **auth_changes)
    return dataclasses.replace(bindings, **{capability: dataclasses.replace(target, **changes)})


def assert_invalid(bindings: HostedSolanaGatewayBindings) -> None:
    with pytest.raises(AreteError, match="Hosted Solana gateway"):
        create_hosted_solana_gateway_transports(bindings)


def test_valid_bindings_construct_transports():
    transports = create_hosted_solana_gateway_transports(valid_bindings())
    assert transports.chain is not None
    assert transports.transactions is not None


def test_validation_rejects_inconsistent_bindings():
    assert_invalid(mutated(valid_bindings(), "chain", endpoint="http://gateway.example/chain"))
    assert_invalid(
        mutated(
            valid_bindings(),
            "chain",
            solana_gateway_binding_id="sgb_short",
            auth={"target_id": "sgb_short"},
        )
    )
    assert_invalid(mutated(valid_bindings(), "transactions", cluster="  "))
    assert_invalid(mutated(valid_bindings(), "chain", auth={"mode": "jwt"}))
    assert_invalid(
        mutated(valid_bindings(), "chain", auth={"session_endpoint": "http://api.example.test/x"})
    )
    assert_invalid(
        mutated(valid_bindings(), "transactions", auth={"jwks_url": "ftp://api.example.test/jwks"})
    )
    assert_invalid(mutated(valid_bindings(), "chain", auth={"token_transport": "query"}))
    assert_invalid(mutated(valid_bindings(), "chain", auth={"audience": "arete:other"}))
    assert_invalid(mutated(valid_bindings(), "chain", auth={"target_kind": "program-read-binding"}))
    assert_invalid(
        mutated(valid_bindings(), "transactions", auth={"target_id": "sgb_00000000000000000000000000000002"})
    )
    assert_invalid(
        mutated(valid_bindings(), "transactions", auth={"scopes": ("transaction:inspect",)})
    )
    assert_invalid(mutated(valid_bindings(), "chain", auth={"scopes": ("transaction:inspect",)}))


def test_loopback_endpoints_are_accepted():
    bindings = valid_bindings()
    bindings = mutated(bindings, "chain", endpoint="http://127.0.0.1:9/chain")
    bindings = mutated(bindings, "transactions", endpoint="http://localhost:9/transactions")
    transports = create_hosted_solana_gateway_transports(bindings)
    assert transports.chain is not None


def test_bindings_deserialize_from_ts_generated_json():
    payload = {
        "chain": {
            "endpoint": ENDPOINT,
            "authPolicy": "signed_session",
            "solanaGatewayBindingId": GATEWAY_ID,
            "cluster": "mainnet-beta",
            "region": "us-west-1",
            "auth": {
                "required": True,
                "mode": "signed_session",
                "sessionEndpoint": SESSION_ENDPOINT,
                "jwksUrl": "https://api.example.test/.well-known/jwks.json",
                "tokenTransport": "bearer",
                "audience": "arete:solana-gateway",
                "targetKind": "solana-gateway-binding",
                "targetId": GATEWAY_ID,
                "scopes": ["read"],
                "acceptedKeyClasses": ["publishable", "secret"],
                "transactionEntitlementRequired": False,
            },
        },
        "transactions": {
            "endpoint": ENDPOINT,
            "authPolicy": "signed_session",
            "solanaGatewayBindingId": GATEWAY_ID,
            "cluster": "mainnet-beta",
            "region": "us-west-1",
            "auth": {
                "required": True,
                "mode": "signed_session",
                "sessionEndpoint": SESSION_ENDPOINT,
                "jwksUrl": "https://api.example.test/.well-known/jwks.json",
                "tokenTransport": "bearer",
                "audience": "arete:solana-gateway",
                "targetKind": "solana-gateway-binding",
                "targetId": GATEWAY_ID,
                "scopes": ["transaction:inspect", "transaction:send"],
                "acceptedKeyClasses": ["publishable", "secret"],
                "transactionEntitlementRequired": True,
            },
        },
    }
    assert HostedSolanaGatewayBindings.from_dict(payload) == valid_bindings()


def request_aware_provider(token_requests: List[Dict[str, Any]], tokens=None):
    calls = {"n": 0}

    async def get_token(request: Dict[str, Any]):
        token_requests.append(request)
        calls["n"] += 1
        if tokens is not None:
            return {"token": tokens[min(calls["n"], len(tokens)) - 1], "scopes": request["scopes"]}
        return {"token": f"token-{'+'.join(request['scopes'])}", "scopes": request["scopes"]}

    return get_token


@pytest.mark.asyncio
async def test_isolates_exact_target_tokens_per_scope():
    token_requests: List[Dict[str, Any]] = []
    gateway_requests: List[Dict[str, Optional[str]]] = []

    def handler(request: httpx.Request) -> httpx.Response:
        url = str(request.url)
        gateway_requests.append(
            {"url": url, "authorization": request.headers.get("authorization")}
        )
        if url.endswith("/chain/exists/account"):
            return httpx.Response(200, json={"exists": True})
        if url.endswith("/transactions/v1/latest-blockhash"):
            return httpx.Response(
                200,
                json={"blockhash": "blockhash", "contextSlot": "42", "lastValidBlockHeight": "99"},
            )
        if url.endswith("/transactions/v1/send"):
            return httpx.Response(200, json={"signature": "signature"})
        raise AssertionError(f"Unexpected gateway request: {url}")

    transports = create_hosted_solana_gateway_transports(
        valid_bindings(),
        auth=AuthConfig(get_token=request_aware_provider(token_requests)),
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(handler)),
    )

    assert await transports.chain.exists("account") is True
    assert await transports.chain.exists("account") is True
    blockhash = await transports.transactions.get_latest_blockhash()
    assert blockhash.blockhash == "blockhash"
    assert blockhash.context_slot == 42
    await transports.transactions.get_latest_blockhash()
    send = await transports.transactions.send_transaction("signed")
    assert send.signature == "signature"

    assert token_requests == [
        {"scopes": ["read"], "targetKind": "solana-gateway-binding", "targetId": GATEWAY_ID},
        {
            "scopes": ["transaction:inspect"],
            "targetKind": "solana-gateway-binding",
            "targetId": GATEWAY_ID,
        },
        {
            "scopes": ["transaction:send"],
            "targetKind": "solana-gateway-binding",
            "targetId": GATEWAY_ID,
        },
    ]
    assert [entry["url"] for entry in gateway_requests] == [
        f"{ENDPOINT}chain/exists/account",
        f"{ENDPOINT}chain/exists/account",
        f"{ENDPOINT}transactions/v1/latest-blockhash",
        f"{ENDPOINT}transactions/v1/latest-blockhash",
        f"{ENDPOINT}transactions/v1/send",
    ]
    assert [entry["authorization"] for entry in gateway_requests] == [
        "Bearer token-read",
        "Bearer token-read",
        "Bearer token-transaction:inspect",
        "Bearer token-transaction:inspect",
        "Bearer token-transaction:send",
    ]


@pytest.mark.asyncio
async def test_refreshes_once_only_when_dispatch_is_explicitly_safe_to_replay():
    token_requests: List[Dict[str, Any]] = []
    responses = [
        httpx.Response(
            401,
            headers={"X-Error-Code": "token-expired", UPSTREAM_ATTEMPTED_HEADER: "false"},
            json={"code": "token-expired"},
        ),
        httpx.Response(200, json={"signature": "signature"}),
    ]
    seen: List[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return responses[len(seen) - 1]

    transports = create_hosted_solana_gateway_transports(
        valid_bindings(),
        auth=AuthConfig(get_token=request_aware_provider(token_requests, tokens=["stale", "fresh"])),
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(handler)),
    )

    result = await transports.transactions.send_transaction("signed")
    assert result.signature == "signature"
    assert len(token_requests) == 2
    assert len(seen) == 2
    assert seen[0].headers["authorization"] == "Bearer stale"
    assert seen[1].headers["authorization"] == "Bearer fresh"


@pytest.mark.asyncio
async def test_never_refreshes_after_upstream_dispatch_may_have_started():
    token_requests: List[Dict[str, Any]] = []
    seen: List[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(
            401,
            headers={"X-Error-Code": "token-expired", UPSTREAM_ATTEMPTED_HEADER: "true"},
            json={"code": "token-expired"},
        )

    transports = create_hosted_solana_gateway_transports(
        valid_bindings(),
        auth=AuthConfig(get_token=request_aware_provider(token_requests, tokens=["stale"])),
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(handler)),
    )

    with pytest.raises(TransactionTransportError) as info:
        await transports.transactions.send_transaction("signed")
    assert info.value.status == 401
    assert len(token_requests) == 1
    assert len(seen) == 1


@pytest.mark.asyncio
async def test_session_endpoint_flow_mints_targeted_tokens():
    session_bodies: List[Dict[str, Any]] = []
    chain_auth_headers: List[Optional[str]] = []

    def handler(request: httpx.Request) -> httpx.Response:
        url = str(request.url)
        if url == SESSION_ENDPOINT:
            session_bodies.append(json.loads(request.content))
            assert request.headers["authorization"] == "Bearer a4_pk_test"
            return httpx.Response(200, json={"token": "gateway-token", "expires_at": 4102444800})
        if url.endswith("/chain/clock"):
            chain_auth_headers.append(request.headers.get("authorization"))
            return httpx.Response(200, json={"slot": 123, "epoch": 5, "unixTimestamp": 1700000000})
        raise AssertionError(f"Unexpected request: {url}")

    # No runtime strategy: tokens are minted from the binding session endpoint
    # with the exact solana-gateway-binding target.
    transports = create_hosted_solana_gateway_transports(
        valid_bindings(),
        auth=AuthConfig(publishable_key="a4_pk_test"),
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(handler)),
    )

    clock = await transports.chain.clock()
    assert clock.slot == 123
    assert session_bodies == [
        {
            "targetKind": "solana-gateway-binding",
            "targetId": GATEWAY_ID,
            "scopes": ["read"],
        }
    ]
    assert chain_auth_headers == ["Bearer gateway-token"]


@pytest.mark.asyncio
async def test_static_runtime_token_is_used_directly():
    seen: List[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        assert "/ws/sessions" not in str(request.url)
        return httpx.Response(200, json={"exists": False})

    transports = create_hosted_solana_gateway_transports(
        valid_bindings(),
        auth=AuthConfig(token="static-token"),
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(handler)),
    )
    assert await transports.chain.exists("acct") is False
    assert seen[0].headers["authorization"] == "Bearer static-token"
