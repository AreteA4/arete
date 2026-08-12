"""Tests for arete.auth: existing public API + targeted-token endpoint hooks."""

from __future__ import annotations

import json
import time

import httpx
import pytest

from arete.auth import (
    DEFAULT_HOSTED_TOKEN_ENDPOINT,
    AuthConfig,
    AuthErrorCode,
    AuthToken,
    TokenTransport,
    build_token_endpoint_request_body,
    build_websocket_url,
    is_hosted_arete_websocket_url,
    parse_jwt_expiry,
    request_token_from_endpoint,
    resolve_token_endpoint,
    should_refresh_token,
)
from arete.errors import AuthError


def test_public_api_is_intact():
    config = AuthConfig(publishable_key="a4_pk_x")
    assert config.token_transport is TokenTransport.QUERY
    assert AuthConfig.from_api_key("a4_sk_x").publishable_key == "a4_sk_x"
    token = AuthToken(token="t", expires_at=123)
    assert token.scopes is None  # new optional field defaults preserved
    assert AuthErrorCode.from_wire("token-expired") is AuthErrorCode.TOKEN_EXPIRED
    assert should_refresh_token(AuthErrorCode.TOKEN_EXPIRED)
    assert not should_refresh_token(AuthErrorCode.ORIGIN_MISMATCH)


def test_auth_token_expiry_check():
    assert not AuthToken(token="t").is_expiring()
    assert AuthToken(token="t", expires_at=int(time.time()) + 30).is_expiring()
    assert not AuthToken(token="t", expires_at=int(time.time()) + 3600).is_expiring()


def test_build_websocket_url_query_transport():
    url = build_websocket_url("wss://host/socket", token="tok")
    assert "hs_token=tok" in url
    assert build_websocket_url("wss://host/socket", token="tok", transport=TokenTransport.BEARER) == "wss://host/socket"


def test_resolve_token_endpoint_strategy():
    explicit = AuthConfig(token_endpoint="https://auth.example/token")
    assert (
        resolve_token_endpoint(explicit, "wss://x.stack.arete.run/s")
        == "https://auth.example/token"
    )
    hosted = AuthConfig(publishable_key="a4_pk_x")
    assert (
        resolve_token_endpoint(hosted, "wss://x.stack.arete.run/s")
        == DEFAULT_HOSTED_TOKEN_ENDPOINT
    )
    # Hosted default applies even without a config (anonymous minting).
    assert (
        resolve_token_endpoint(None, "wss://x.stack.arete.run/s")
        == DEFAULT_HOSTED_TOKEN_ENDPOINT
    )
    assert resolve_token_endpoint(hosted, "wss://self.hosted.example/s") is None
    assert resolve_token_endpoint(None, None) is None
    assert is_hosted_arete_websocket_url("wss://x.stack.arete.run/s")


def test_build_token_endpoint_request_body_untargeted():
    assert build_token_endpoint_request_body(
        websocket_url="wss://host/socket", scopes=["read"]
    ) == {"websocket_url": "wss://host/socket", "scopes": ["read"]}
    assert build_token_endpoint_request_body(websocket_url=None, scopes=[]) == {
        "websocket_url": "",
        "scopes": [],
    }


def test_build_token_endpoint_request_body_targeted():
    assert build_token_endpoint_request_body(
        websocket_url="wss://host/socket",
        scopes=["read"],
        target_kind="program-read-binding",
        target_id="prb_1",
        program_release_hash="hash-1",
    ) == {
        "targetKind": "program-read-binding",
        "targetId": "prb_1",
        "scopes": ["read"],
        "programReleaseHash": "hash-1",
    }
    assert build_token_endpoint_request_body(
        websocket_url="wss://host/socket",
        scopes=["transaction:send"],
        target_kind="solana-gateway-binding",
        target_id="sgb_1",
    ) == {
        "targetKind": "solana-gateway-binding",
        "targetId": "sgb_1",
        "scopes": ["transaction:send"],
    }


def test_parse_jwt_expiry():
    import base64

    def b64(raw: str) -> str:
        return base64.urlsafe_b64encode(raw.encode()).rstrip(b"=").decode()

    token = f"{b64('{}')}.{b64(json.dumps({'exp': 1234567890}))}.sig"
    assert parse_jwt_expiry(token) == 1234567890
    assert parse_jwt_expiry("not-a-jwt") is None


def _client(handler) -> httpx.AsyncClient:
    return httpx.AsyncClient(transport=httpx.MockTransport(handler))


@pytest.mark.asyncio
async def test_request_token_from_endpoint_success():
    seen = {}

    def handler(request: httpx.Request) -> httpx.Response:
        seen["headers"] = dict(request.headers)
        seen["body"] = json.loads(request.content)
        return httpx.Response(
            200,
            json={"token": "minted", "expiresAt": 4102444800, "scopes": ["read"]},
        )

    config = AuthConfig(
        publishable_key="a4_pk_test",
        token_endpoint_headers={"x-custom": "v"},
    )
    async with _client(handler) as http_client:
        token = await request_token_from_endpoint(
            http_client,
            "https://auth.example/token",
            config,
            {"websocket_url": "wss://host/socket", "scopes": ["read"]},
        )
    assert token == AuthToken(token="minted", expires_at=4102444800, scopes=["read"])
    assert seen["headers"]["authorization"] == "Bearer a4_pk_test"
    assert seen["headers"]["x-custom"] == "v"
    assert seen["body"] == {"websocket_url": "wss://host/socket", "scopes": ["read"]}


@pytest.mark.asyncio
async def test_request_token_from_endpoint_error_paths():
    def unauthorized(request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            401,
            headers={"X-Error-Code": "invalid-api-key"},
            json={"error": "bad key", "code": "invalid-api-key"},
        )

    async with _client(unauthorized) as http_client:
        with pytest.raises(AuthError) as info:
            await request_token_from_endpoint(
                http_client, "https://auth.example/token", None, {}
            )
    assert info.value.code is AuthErrorCode.INVALID_API_KEY
    assert "bad key" in str(info.value)

    def throttled(request: httpx.Request) -> httpx.Response:
        return httpx.Response(429, text="slow down")

    async with _client(throttled) as http_client:
        with pytest.raises(AuthError) as info:
            await request_token_from_endpoint(
                http_client, "https://auth.example/token", None, {}
            )
    assert info.value.code is AuthErrorCode.QUOTA_EXCEEDED

    def tokenless(request: httpx.Request) -> httpx.Response:
        return httpx.Response(200, json={"expires_at": 1})

    async with _client(tokenless) as http_client:
        with pytest.raises(AuthError) as info:
            await request_token_from_endpoint(
                http_client, "https://auth.example/token", None, {}
            )
    assert info.value.code is AuthErrorCode.TOKEN_INVALID_FORMAT
