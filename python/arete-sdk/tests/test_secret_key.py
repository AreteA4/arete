"""Tests for the server-side ``secret_key`` option and ``ARETE_API_KEY``."""

from __future__ import annotations

import logging

import httpx
import pytest

import arete.auth as auth_module
from arete.auth import (
    ARETE_API_KEY_ENV,
    AuthConfig,
    AuthState,
    classify_api_key,
    resolve_auth_config,
)
from arete.errors import AreteError
from arete.http import HttpAuthClient

SECRET = "a4_sk_supersecretvalue"
AGENT = "a4_ak_agentsecretvalue"
PUBLISHABLE = "a4_pk_publicvalue"


@pytest.fixture(autouse=True)
def _isolate(monkeypatch):
    monkeypatch.delenv(ARETE_API_KEY_ENV, raising=False)
    auth_module._warned.clear()
    yield
    auth_module._warned.clear()


def _assert_no_key_material(text: str) -> None:
    for key in (SECRET, AGENT, PUBLISHABLE):
        assert key not in text
        assert key[6:] not in text


def test_classify_api_key():
    assert classify_api_key(PUBLISHABLE) == "publishable"
    assert classify_api_key("hspk_legacy") == "publishable"
    assert classify_api_key(SECRET) == "secret"
    assert classify_api_key(AGENT) == "secret"
    assert classify_api_key("hsk_legacy") == "secret"
    assert classify_api_key("custom-key") == "unknown"


def test_secret_key_is_the_bearer_credential():
    assert AuthConfig(secret_key=SECRET).api_key == SECRET
    assert AuthConfig(publishable_key=PUBLISHABLE).api_key == PUBLISHABLE
    assert AuthConfig().api_key is None


def test_from_api_key_routes_by_prefix():
    assert AuthConfig.from_api_key(AGENT).secret_key == AGENT
    assert AuthConfig.from_api_key(SECRET).secret_key == SECRET
    assert AuthConfig.from_api_key(PUBLISHABLE).publishable_key == PUBLISHABLE
    assert AuthConfig.from_api_key(PUBLISHABLE).secret_key is None


def test_env_fallback_when_no_auth_is_configured(monkeypatch):
    monkeypatch.setenv(ARETE_API_KEY_ENV, f"  {AGENT}\n")
    assert resolve_auth_config(None).secret_key == AGENT
    resolved = resolve_auth_config(AuthConfig(token_endpoint_headers={"X-A": "1"}))
    assert resolved.secret_key == AGENT
    assert resolved.token_endpoint_headers == {"X-A": "1"}


def test_no_env_and_no_auth_stays_unconfigured():
    assert resolve_auth_config(None) is None


@pytest.mark.parametrize(
    "config",
    [
        AuthConfig(secret_key=SECRET),
        AuthConfig(publishable_key=PUBLISHABLE),
        AuthConfig(token="static"),
        AuthConfig(token_endpoint="https://auth.example.com/token"),
    ],
)
def test_explicit_auth_wins_over_env(monkeypatch, config):
    monkeypatch.setenv(ARETE_API_KEY_ENV, AGENT)
    assert resolve_auth_config(config) is config


def test_publishable_key_in_env_is_ignored_with_a_warning(monkeypatch, caplog):
    monkeypatch.setenv(ARETE_API_KEY_ENV, PUBLISHABLE)
    with caplog.at_level(logging.WARNING, logger="arete.auth"):
        assert resolve_auth_config(None) is None
    assert "auth.publishable_key" in caplog.text
    _assert_no_key_material(caplog.text)


def test_publishable_key_as_secret_key_is_refused():
    with pytest.raises(AreteError) as info:
        AuthConfig(secret_key=PUBLISHABLE)
    assert info.value.code == "INVALID_CONFIG"
    assert "Pass it as auth.publishable_key" in info.value.message
    _assert_no_key_material(str(info.value))


def test_empty_and_conflicting_keys_are_refused():
    with pytest.raises(AreteError):
        AuthConfig(secret_key="  ")
    with pytest.raises(AreteError) as info:
        AuthConfig(secret_key=SECRET, publishable_key=PUBLISHABLE)
    assert "not both" in info.value.message
    _assert_no_key_material(str(info.value))


def test_secret_class_key_as_publishable_key_warns_once(caplog):
    with caplog.at_level(logging.WARNING, logger="arete.auth"):
        config = AuthConfig(publishable_key=SECRET)
        AuthConfig(publishable_key=AGENT)
    assert config.api_key == SECRET
    messages = [r.getMessage() for r in caplog.records if "secret-class" in r.getMessage()]
    assert len(messages) == 1
    assert "auth.secret_key" in messages[0]
    assert "a4 auth keys create-publishable" in messages[0]
    _assert_no_key_material(caplog.text)


def _token_handler(seen):
    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(200, json={"token": "minted", "expires_at": 4102444800})

    return handler


async def _websocket_auth_header(config):
    seen = []
    state = AuthState("wss://demo.stack.arete.run", config)
    state._http_session = httpx.AsyncClient(
        transport=httpx.MockTransport(_token_handler(seen))
    )
    try:
        assert await state.resolve_token() == "minted"
    finally:
        await state.close()
    assert str(seen[0].url) == "https://api.arete.run/ws/sessions"
    assert "origin" not in seen[0].headers
    return seen[0].headers.get("authorization")


@pytest.mark.asyncio
async def test_secret_key_mints_hosted_websocket_tokens():
    assert await _websocket_auth_header(AuthConfig(secret_key=SECRET)) == f"Bearer {SECRET}"


@pytest.mark.asyncio
async def test_env_key_mints_hosted_websocket_tokens(monkeypatch):
    monkeypatch.setenv(ARETE_API_KEY_ENV, AGENT)
    assert await _websocket_auth_header(None) == f"Bearer {AGENT}"


@pytest.mark.asyncio
async def test_explicit_secret_key_beats_env(monkeypatch):
    monkeypatch.setenv(ARETE_API_KEY_ENV, AGENT)
    assert await _websocket_auth_header(AuthConfig(secret_key=SECRET)) == f"Bearer {SECRET}"


@pytest.mark.asyncio
async def test_http_auth_client_uses_env_key(monkeypatch):
    monkeypatch.setenv(ARETE_API_KEY_ENV, AGENT)
    seen = []
    client = HttpAuthClient(
        auth=None,
        websocket_url="wss://demo.stack.arete.run",
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(_token_handler(seen))),
    )
    assert await client.get_token() == "minted"
    assert seen[0].headers["authorization"] == f"Bearer {AGENT}"
