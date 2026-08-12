"""Tests for arete.http: token strategy order, targeted-token LRU,
refresh-replay-once, predispatch marker, derive_http_endpoint."""

from __future__ import annotations

import base64
import json
import time
from typing import Any, Dict, List, Optional

import httpx
import pytest

from arete.auth import DEFAULT_HOSTED_TOKEN_ENDPOINT, AuthConfig, AuthToken
from arete.errors import AuthError, HttpRequestError
from arete.http import (
    MAX_HTTP_AUTH_TOKEN_STATES,
    UPSTREAM_ATTEMPTED_HEADER,
    AuthTokenTarget,
    HttpAuthClient,
    derive_http_endpoint,
    normalize_scopes,
)


def _b64url(raw: str) -> str:
    return base64.urlsafe_b64encode(raw.encode()).rstrip(b"=").decode()


def jwt_with_exp(exp: int) -> str:
    return f"{_b64url(json.dumps({'alg': 'none'}))}.{_b64url(json.dumps({'exp': exp}))}.sig"


class TokenEndpointServer:
    """MockTransport handler standing in for a token endpoint + JSON API."""

    def __init__(self) -> None:
        self.token_bodies: List[Dict[str, Any]] = []
        self.token_headers: List[Dict[str, str]] = []
        self.token_urls: List[str] = []
        self.api_requests: List[httpx.Request] = []
        self.scopes_override: Optional[List[str]] = None
        self.api_responder = lambda request, index: httpx.Response(200, json={"ok": True})

    @property
    def mints(self) -> int:
        return len(self.token_bodies)

    def handler(self, request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/token") or str(request.url) == DEFAULT_HOSTED_TOKEN_ENDPOINT:
            self.token_urls.append(str(request.url))
            self.token_bodies.append(json.loads(request.content))
            self.token_headers.append(dict(request.headers))
            payload: Dict[str, Any] = {
                "token": f"token-{self.mints - 1}",
                "expires_at": int(time.time()) + 3600,
            }
            if self.scopes_override is not None:
                payload["scopes"] = self.scopes_override
        else:
            self.api_requests.append(request)
            return self.api_responder(request, len(self.api_requests) - 1)
        return httpx.Response(200, json=payload)

    def http_client(self) -> httpx.AsyncClient:
        return httpx.AsyncClient(transport=httpx.MockTransport(self.handler))


def endpoint_client(server: TokenEndpointServer, **kwargs: Any) -> HttpAuthClient:
    auth = kwargs.pop(
        "auth",
        AuthConfig(
            publishable_key="a4_pk_test",
            token_endpoint="https://auth.example/token",
            token_endpoint_headers={"x-custom": "custom-value"},
        ),
    )
    return HttpAuthClient(
        auth=auth,
        websocket_url=kwargs.pop("websocket_url", "wss://stack.example/socket"),
        http_client=server.http_client(),
    )


def prb(target_id: str, release_hash: str = "release-hash") -> AuthTokenTarget:
    return AuthTokenTarget(
        kind="program-read-binding", target_id=target_id, release_hash=release_hash
    )


# ---------------------------------------------------------------------------
# derive_http_endpoint


def test_derive_http_endpoint_maps_schemes_and_strips_trailing_slash():
    assert derive_http_endpoint("ws://host:8080/path") == "http://host:8080/path"
    assert derive_http_endpoint("wss://host/socket") == "https://host/socket"
    assert derive_http_endpoint("wss://host/") == "https://host"
    assert derive_http_endpoint("wss://host") == "https://host"
    assert derive_http_endpoint("https://host/x") == "https://host/x"


def test_derive_http_endpoint_falls_back_to_string_surgery():
    assert derive_http_endpoint("wss:not-a-url") == "https:not-a-url"
    assert derive_http_endpoint("WS:oddball") == "http:oddball"
    assert derive_http_endpoint("garbage") == "garbage"


def test_normalize_scopes_dedupes_and_sorts():
    assert normalize_scopes(["write", "read", "read"]) == ("read", "write")
    assert normalize_scopes(None) == ()


# ---------------------------------------------------------------------------
# strategy order


@pytest.mark.asyncio
async def test_no_auth_config_yields_no_token():
    server = TokenEndpointServer()
    client = HttpAuthClient(auth=None, http_client=server.http_client())
    assert await client.get_token() is None
    assert server.mints == 0


@pytest.mark.asyncio
async def test_static_token_wins_over_provider_and_endpoint():
    server = TokenEndpointServer()

    async def provider():
        raise AssertionError("provider must not be called")

    client = endpoint_client(
        server,
        auth=AuthConfig(
            token="static-token",
            get_token=provider,
            token_endpoint="https://auth.example/token",
        ),
    )
    assert await client.get_token() == "static-token"
    assert server.mints == 0


@pytest.mark.asyncio
async def test_provider_wins_over_endpoint():
    server = TokenEndpointServer()

    async def provider():
        return AuthToken(token="provider-token")

    client = endpoint_client(
        server,
        auth=AuthConfig(get_token=provider, token_endpoint="https://auth.example/token"),
    )
    assert await client.get_token() == "provider-token"
    assert server.mints == 0


@pytest.mark.asyncio
async def test_endpoint_flow_sends_untargeted_body_and_headers():
    server = TokenEndpointServer()
    client = endpoint_client(server)

    token = await client.get_token(scopes=["read", "read", "transaction:inspect"])
    assert token == "token-0"
    assert server.token_bodies == [
        {
            "websocket_url": "wss://stack.example/socket",
            "scopes": ["read", "transaction:inspect"],
        }
    ]
    headers = server.token_headers[0]
    assert headers["authorization"] == "Bearer a4_pk_test"
    assert headers["x-custom"] == "custom-value"


@pytest.mark.asyncio
async def test_hosted_default_endpoint_is_used_for_hosted_stacks():
    server = TokenEndpointServer()
    client = HttpAuthClient(
        auth=AuthConfig(publishable_key="a4_pk_test"),
        websocket_url="wss://demo.stack.arete.run/socket",
        http_client=server.http_client(),
    )
    assert await client.get_token() == "token-0"
    assert server.token_urls == [DEFAULT_HOSTED_TOKEN_ENDPOINT]


@pytest.mark.asyncio
async def test_expired_static_jwt_is_rejected():
    client = HttpAuthClient(
        auth=AuthConfig(token=jwt_with_exp(int(time.time()) - 10)),
        http_client=TokenEndpointServer().http_client(),
    )
    with pytest.raises(AuthError):
        await client.get_token()

    valid = jwt_with_exp(int(time.time()) + 3600)
    client = HttpAuthClient(
        auth=AuthConfig(token=valid),
        http_client=TokenEndpointServer().http_client(),
    )
    assert await client.get_token() == valid


# ---------------------------------------------------------------------------
# shared (untargeted) token state


@pytest.mark.asyncio
async def test_untargeted_tokens_are_cached_and_scopes_accumulate():
    server = TokenEndpointServer()
    client = endpoint_client(server)

    first = await client.get_token()
    cached = await client.get_token()
    assert first == cached
    assert server.mints == 1

    widened = await client.get_token(scopes=["transaction:inspect"])
    assert widened == "token-1"
    assert server.mints == 2
    assert server.token_bodies[1]["scopes"] == ["read", "transaction:inspect"]


@pytest.mark.asyncio
async def test_force_refresh_mints_a_new_token():
    server = TokenEndpointServer()
    client = endpoint_client(server)
    first = await client.get_token()
    forced = await client.get_token(force_refresh=True)
    assert first != forced
    assert server.mints == 2


@pytest.mark.asyncio
async def test_scope_coverage_failure_is_an_auth_error():
    server = TokenEndpointServer()
    server.scopes_override = ["read"]
    client = endpoint_client(server)
    with pytest.raises(AuthError, match="not granted required scopes"):
        await client.get_token(scopes=["transaction:send"])


# ---------------------------------------------------------------------------
# targeted tokens


@pytest.mark.asyncio
async def test_targeted_body_includes_target_identity():
    server = TokenEndpointServer()
    client = endpoint_client(server)

    await client.get_token(target=prb("prb_1", "hash-1"))
    await client.get_token(
        target=AuthTokenTarget(kind="solana-gateway-binding", target_id="sgb_1"),
        scopes=["transaction:send"],
    )

    assert server.token_bodies[0] == {
        "targetKind": "program-read-binding",
        "targetId": "prb_1",
        "scopes": ["read"],
        "programReleaseHash": "hash-1",
    }
    assert server.token_bodies[1] == {
        "targetKind": "solana-gateway-binding",
        "targetId": "sgb_1",
        "scopes": ["transaction:send"],
    }


@pytest.mark.asyncio
async def test_targeted_cache_identity_normalizes_scopes():
    server = TokenEndpointServer()
    client = endpoint_client(server)

    a = await client.get_token(target=prb("prb_1"), scopes=["write", "read", "read"])
    b = await client.get_token(target=prb("prb_1"), scopes=["read", "write"])
    assert a == b
    assert server.mints == 1

    other = await client.get_token(target=prb("prb_2"), scopes=["read", "write"])
    assert other != a
    assert server.mints == 2


@pytest.mark.asyncio
async def test_incomplete_targets_are_rejected():
    client = HttpAuthClient(http_client=TokenEndpointServer().http_client())
    with pytest.raises(Exception, match="complete supported target identity"):
        await client.get_token(
            target=AuthTokenTarget(kind="program-read-binding", target_id="prb_1")
        )
    with pytest.raises(Exception, match="complete supported target identity"):
        await client.get_token(
            target=AuthTokenTarget(
                kind="solana-gateway-binding",
                target_id="sgb_1",
                release_hash="unexpected",
            )
        )


@pytest.mark.asyncio
async def test_targeted_cache_evicts_oldest_beyond_cap():
    server = TokenEndpointServer()
    client = endpoint_client(server)

    for i in range(MAX_HTTP_AUTH_TOKEN_STATES + 1):
        await client.get_token(target=prb(f"prb_{i}"))
    assert server.mints == MAX_HTTP_AUTH_TOKEN_STATES + 1

    # The newest entry is still cached…
    await client.get_token(target=prb(f"prb_{MAX_HTTP_AUTH_TOKEN_STATES}"))
    assert server.mints == MAX_HTTP_AUTH_TOKEN_STATES + 1

    # …but the oldest was evicted and re-mints.
    await client.get_token(target=prb("prb_0"))
    assert server.mints == MAX_HTTP_AUTH_TOKEN_STATES + 2


# ---------------------------------------------------------------------------
# request_json


@pytest.mark.asyncio
async def test_request_json_decodes_bodies_and_null():
    server = TokenEndpointServer()

    def responder(request: httpx.Request, index: int) -> httpx.Response:
        if request.url.path == "/null":
            return httpx.Response(200, content=b"null")
        return httpx.Response(200, json={"value": 7})

    server.api_responder = responder
    client = endpoint_client(server)

    assert await client.request_json("GET", "https://api.example/value") == {"value": 7}
    assert await client.request_json("GET", "https://api.example/null") is None
    assert server.api_requests[0].headers["authorization"] == "Bearer token-0"


@pytest.mark.asyncio
async def test_refresh_and_replay_exactly_once_on_401():
    server = TokenEndpointServer()

    def responder(request: httpx.Request, index: int) -> httpx.Response:
        if request.headers.get("authorization") == "Bearer token-0":
            return httpx.Response(
                401,
                headers={"X-Error-Code": "token-expired"},
                json={"code": "token-expired"},
            )
        return httpx.Response(200, json={"ok": True})

    server.api_responder = responder
    client = endpoint_client(server)

    assert await client.request_json("GET", "https://api.example/x") == {"ok": True}
    assert len(server.api_requests) == 2
    assert server.mints == 2
    assert server.api_requests[1].headers["authorization"] == "Bearer token-1"


@pytest.mark.asyncio
async def test_replay_happens_at_most_once():
    server = TokenEndpointServer()
    server.api_responder = lambda request, index: httpx.Response(
        401, headers={"X-Error-Code": "token-expired"}, json={"code": "token-expired"}
    )
    client = endpoint_client(server)

    with pytest.raises(HttpRequestError) as info:
        await client.request_json("GET", "https://api.example/x")
    assert len(server.api_requests) == 2
    assert info.value.status == 401


@pytest.mark.asyncio
async def test_non_refreshable_errors_do_not_replay():
    server = TokenEndpointServer()
    server.api_responder = lambda request, index: httpx.Response(
        403, json={"code": "deployment-access-denied", "message": "nope"}
    )
    client = endpoint_client(server)

    with pytest.raises(HttpRequestError):
        await client.request_json("GET", "https://api.example/x")
    assert len(server.api_requests) == 1


@pytest.mark.asyncio
async def test_structured_error_body_is_attached():
    server = TokenEndpointServer()
    server.api_responder = lambda request, index: httpx.Response(
        429,
        json={
            "code": "rate_limit_exceeded",
            "message": "Too many requests",
            "retryable": True,
            "retry_after": 1.5,
            "suggested_action": "back off",
            "docs_url": "https://docs.arete.run/errors#rate-limit",
        },
    )
    client = endpoint_client(server)

    with pytest.raises(HttpRequestError) as info:
        await client.request_json("GET", "https://api.example/x")
    error = info.value
    assert error.status == 429
    assert error.code == "rate_limit_exceeded"
    assert error.message == "Too many requests"
    assert error.retryable is True
    assert error.retry_after == 1.5
    assert error.suggested_action == "back off"
    assert error.docs_url == "https://docs.arete.run/errors#rate-limit"


@pytest.mark.asyncio
async def test_predispatch_marker_is_never_sent_outbound():
    # Finding 8: the marker is a response-only mechanism. The server sets it
    # (rust/arete-server/src/http/transactions.rs `transaction_response`,
    # http_health.rs:428) and only exposes it via Access-Control-Expose-Headers;
    # Access-Control-Allow-Headers is just "Authorization, Content-Type", so a
    # browser preflight would reject it as a request header. TS
    # (client.ts:843, solana-gateway.ts:182) and Rust (http.rs:909) only read
    # it off the response. No scope may add it to an outbound request.
    server = TokenEndpointServer()
    client = endpoint_client(server)

    await client.request_json(
        "POST", "https://api.example/send", json_body={}, scopes=["transaction:send"]
    )
    await client.request_json("GET", "https://api.example/read", scopes=["read"])

    send_request, read_request = server.api_requests
    assert UPSTREAM_ATTEMPTED_HEADER.lower() not in send_request.headers
    assert UPSTREAM_ATTEMPTED_HEADER.lower() not in read_request.headers


@pytest.mark.asyncio
async def test_caller_supplied_headers_are_still_forwarded_verbatim():
    # Removing the injected marker must not disturb explicit headers.
    server = TokenEndpointServer()
    client = endpoint_client(server)

    await client.request_json(
        "POST",
        "https://api.example/send",
        json_body={},
        headers={"X-Custom": "kept"},
        scopes=["transaction:send"],
    )
    request = server.api_requests[0]
    assert request.headers["X-Custom"] == "kept"
    assert request.headers["Authorization"].startswith("Bearer ")
    assert UPSTREAM_ATTEMPTED_HEADER.lower() not in request.headers


@pytest.mark.asyncio
async def test_send_replay_requires_predispatch_marker_false():
    # Marker "true": the upstream dispatch may have started; never replay.
    server = TokenEndpointServer()
    server.api_responder = lambda request, index: httpx.Response(
        401,
        headers={"X-Error-Code": "token-expired", UPSTREAM_ATTEMPTED_HEADER: "true"},
        json={"code": "token-expired"},
    )
    client = endpoint_client(server)
    with pytest.raises(HttpRequestError):
        await client.request_json(
            "POST", "https://api.example/send", json_body={}, scopes=["transaction:send"]
        )
    assert len(server.api_requests) == 1

    # Marker "false": provably not dispatched; replay once.
    server = TokenEndpointServer()

    def responder(request: httpx.Request, index: int) -> httpx.Response:
        if index == 0:
            return httpx.Response(
                401,
                headers={
                    "X-Error-Code": "token-expired",
                    UPSTREAM_ATTEMPTED_HEADER: "false",
                },
                json={"code": "token-expired"},
            )
        return httpx.Response(200, json={"signature": "sig"})

    server.api_responder = responder
    client = endpoint_client(server)
    result = await client.request_json(
        "POST", "https://api.example/send", json_body={}, scopes=["transaction:send"]
    )
    assert result == {"signature": "sig"}
    assert len(server.api_requests) == 2


@pytest.mark.asyncio
async def test_aclose_closes_owned_client_only():
    client = HttpAuthClient()
    await client.aclose()

    injected = TokenEndpointServer().http_client()
    client = HttpAuthClient(http_client=injected)
    await client.aclose()
    assert not injected.is_closed
    await injected.aclose()
