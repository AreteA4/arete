"""Account credentials only go to trusted Arete API origins.

A secret or agent key, whether passed in code, read from ``ARETE_API_KEY`` or
taken from the ``a4`` login, is never sent to a session endpoint a descriptor
names on another origin. Publishable keys and session tokens are unaffected.
"""

from __future__ import annotations

import dataclasses
from pathlib import Path
from typing import List

import httpx
import pytest

import arete.auth as auth_module
from arete._credential_destination import credential_may_reach, url_origin
from arete.auth import AuthConfig, AuthState
from arete.errors import AuthError
from arete.gateway import create_hosted_solana_gateway_transports
from arete.protocol import ProtocolClient

from test_gateway import valid_bindings
from test_protocol import DEFINITION

SECRET = "a4_sk_explicitsecretvalue"
ENV_KEY = "a4_ak_environmentvalue"
LOGIN_KEY = "a4_ak_loginvalue"
PUBLISHABLE = "a4_pk_publicvalue"
FOREIGN = "https://api.example.test/ws/sessions"
ARETE = "https://api.arete.run/ws/sessions"
CODE = "UNTRUSTED_CREDENTIAL_DESTINATION"


@pytest.fixture(autouse=True)
def _isolate(monkeypatch, tmp_path):
    monkeypatch.delenv("ARETE_API_KEY", raising=False)
    monkeypatch.delenv("ARETE_API_URL", raising=False)
    monkeypatch.delenv("ARETE_PROFILE", raising=False)
    monkeypatch.setenv("HOME", str(tmp_path / "home"))
    monkeypatch.chdir(tmp_path)
    auth_module._warned.clear()


def _use_env_key(monkeypatch) -> None:
    monkeypatch.setenv("ARETE_API_KEY", ENV_KEY)


def _use_a4_login(monkeypatch, tmp_path: Path) -> None:
    path = tmp_path / "credentials.toml"
    path.write_text(f'[profiles.agent.keys]\n"https://api.arete.run" = "{LOGIN_KEY}"\n')
    monkeypatch.setenv("ARETE_CREDENTIALS_PATH", str(path))


CREDENTIALS = {
    # name: (auth option, setup, key sent, credential label)
    "secret_key": (lambda: AuthConfig(secret_key=SECRET), None, SECRET, "secret key"),
    "ARETE_API_KEY": (lambda: None, _use_env_key, ENV_KEY, "ARETE_API_KEY"),
    "a4_login": (lambda: None, _use_a4_login, LOGIN_KEY, "a4 login"),
}


def _setup(name, monkeypatch, tmp_path):
    make_auth, setup, key, label = CREDENTIALS[name]
    if setup is _use_env_key:
        setup(monkeypatch)
    elif setup is _use_a4_login:
        setup(monkeypatch, tmp_path)
    return make_auth(), key, label


def _bindings(session_endpoint: str):
    bindings = valid_bindings()
    return dataclasses.replace(
        bindings,
        **{
            capability: dataclasses.replace(
                getattr(bindings, capability),
                auth=dataclasses.replace(
                    getattr(bindings, capability).auth, session_endpoint=session_endpoint
                ),
            )
            for capability in ("chain", "transactions")
        },
    )


def _gateway_handler(seen: List[httpx.Request]):
    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        if request.url.path.endswith("/ws/sessions"):
            return httpx.Response(200, json={"token": "gateway-token", "expires_at": 4102444800})
        return httpx.Response(200, json={"slot": 1, "epoch": 1, "unixTimestamp": 1})

    return handler


def _assert_refusal(error: AuthError, label: str, key: str, origin: str) -> None:
    assert error.code == CODE
    assert error.details == {"destination_origin": origin, "credential": label}
    assert origin in error.message and label in error.message
    assert key not in str(error) and key[6:] not in str(error)


@pytest.mark.asyncio
@pytest.mark.parametrize("name", list(CREDENTIALS))
async def test_gateway_binding_on_another_origin_is_refused(name, monkeypatch, tmp_path):
    auth, key, label = _setup(name, monkeypatch, tmp_path)
    seen: List[httpx.Request] = []
    transports = create_hosted_solana_gateway_transports(
        _bindings(FOREIGN),
        auth=auth,
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(_gateway_handler(seen))),
    )
    with pytest.raises(AuthError) as refused:
        await transports.chain.clock()
    _assert_refusal(refused.value, label, key, "https://api.example.test")
    assert seen == []


@pytest.mark.asyncio
@pytest.mark.parametrize("name", list(CREDENTIALS))
async def test_gateway_binding_on_the_arete_api_gets_the_key(name, monkeypatch, tmp_path):
    auth, key, _ = _setup(name, monkeypatch, tmp_path)
    seen: List[httpx.Request] = []
    transports = create_hosted_solana_gateway_transports(
        _bindings(ARETE),
        auth=auth,
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(_gateway_handler(seen))),
    )
    await transports.chain.clock()
    assert str(seen[0].url) == ARETE
    assert seen[0].headers["authorization"] == f"Bearer {key}"


def _protocol_handler(seen: List[httpx.Request]):
    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        if request.url.path == "/ws/sessions":
            return httpx.Response(
                200, json={"token": "s", "expires_at": 4_000_000_000, "scopes": ["protocol:read"]}
            )
        return httpx.Response(200, json={"instances": [], "nextCursor": None})

    return handler


@pytest.mark.asyncio
@pytest.mark.parametrize("name", list(CREDENTIALS))
async def test_protocol_descriptor_session_endpoint_is_refused(name, monkeypatch, tmp_path):
    # The generated deployment names api.example.test: descriptor data, not
    # configuration.
    auth, key, label = _setup(name, monkeypatch, tmp_path)
    seen: List[httpx.Request] = []
    client = ProtocolClient(
        DEFINITION,
        auth=auth,
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(_protocol_handler(seen))),
    )
    with pytest.raises(AuthError) as refused:
        await client.list_instances()
    _assert_refusal(refused.value, label, key, "https://api.example.test")
    assert seen == []


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "name, allowed", [("secret_key", True), ("ARETE_API_KEY", False), ("a4_login", False)]
)
async def test_explicit_api_url_widens_only_an_explicit_key(name, allowed, monkeypatch, tmp_path):
    auth, key, label = _setup(name, monkeypatch, tmp_path)
    seen: List[httpx.Request] = []
    client = ProtocolClient(
        DEFINITION,
        auth=auth,
        api_url="http://127.0.0.1:4100/api",
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(_protocol_handler(seen))),
    )
    if allowed:
        await client.list_instances()
        assert str(seen[0].url) == "http://127.0.0.1:4100/ws/sessions"
        assert seen[0].headers["authorization"] == f"Bearer {key}"
    else:
        with pytest.raises(AuthError) as refused:
            await client.list_instances()
        _assert_refusal(refused.value, label, key, "http://127.0.0.1:4100")
        assert seen == []


@pytest.mark.asyncio
async def test_explicit_api_url_does_not_trust_another_session_endpoint():
    client = ProtocolClient(
        DEFINITION,
        auth=AuthConfig(secret_key=SECRET),
        api_url="http://127.0.0.1:4100/api",
        session_endpoint=FOREIGN,
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(_protocol_handler([]))),
    )
    with pytest.raises(AuthError) as refused:
        await client.list_instances()
    assert refused.value.code == CODE


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "name, allowed", [("secret_key", True), ("ARETE_API_KEY", True), ("a4_login", False)]
)
async def test_arete_api_url_origin_is_trusted_except_for_the_a4_login(
    name, allowed, monkeypatch, tmp_path
):
    monkeypatch.setenv("ARETE_API_URL", "http://127.0.0.1:48705")
    auth, key, label = _setup(name, monkeypatch, tmp_path)
    seen: List[httpx.Request] = []
    client = ProtocolClient(
        DEFINITION,
        auth=auth,
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(_protocol_handler(seen))),
    )
    if allowed:
        await client.list_instances()
        assert seen[0].headers["authorization"] == f"Bearer {key}"
    else:
        with pytest.raises(AuthError) as refused:
            await client.list_instances()
        _assert_refusal(refused.value, label, key, "http://127.0.0.1:48705")
        assert "pass the key explicitly" in refused.value.message


@pytest.mark.asyncio
async def test_custom_websocket_token_endpoint_refuses_a_secret_key():
    seen: List[httpx.Request] = []
    state = AuthState(
        "wss://demo.stack.arete.run",
        AuthConfig(secret_key=SECRET, token_endpoint="https://auth.example.test/token"),
    )
    state._http_session = httpx.AsyncClient(transport=httpx.MockTransport(_gateway_handler(seen)))
    try:
        with pytest.raises(AuthError) as refused:
            await state.resolve_token()
    finally:
        await state.close()
    _assert_refusal(refused.value, "secret key", SECRET, "https://auth.example.test")
    assert seen == []


@pytest.mark.asyncio
async def test_publishable_keys_and_session_tokens_are_unaffected():
    seen: List[httpx.Request] = []
    transports = create_hosted_solana_gateway_transports(
        _bindings(FOREIGN),
        auth=AuthConfig(publishable_key=PUBLISHABLE),
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(_gateway_handler(seen))),
    )
    await transports.chain.clock()
    assert str(seen[0].url) == FOREIGN
    assert seen[0].headers["authorization"] == f"Bearer {PUBLISHABLE}"

    seen.clear()
    transports = create_hosted_solana_gateway_transports(
        _bindings(FOREIGN),
        auth=AuthConfig(token="session-token"),
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(_gateway_handler(seen))),
    )
    await transports.chain.clock()
    assert [str(request.url) for request in seen] == [
        "https://solana.example.test/gateway/chain/clock"
    ]
    assert seen[0].headers["authorization"] == "Bearer session-token"


@pytest.mark.asyncio
async def test_a_secret_key_given_as_publishable_is_still_refused():
    seen: List[httpx.Request] = []
    misplaced = "a4_sk_misplacedvalue"
    transports = create_hosted_solana_gateway_transports(
        _bindings(FOREIGN),
        auth=AuthConfig(publishable_key=misplaced),
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(_gateway_handler(seen))),
    )
    with pytest.raises(AuthError) as refused:
        await transports.chain.clock()
    _assert_refusal(refused.value, "secret key", misplaced, "https://api.example.test")
    assert seen == []


def test_origin_matching():
    assert url_origin("https://API.arete.run.:443/ws") == "https://api.arete.run:443"
    assert url_origin("http://127.0.0.1/x") == "http://127.0.0.1:80"
    for url in ["https://user@api.arete.run/", "https://:pw@api.arete.run/", "not a url",
                "ftp://api.arete.run/", "https://api.arete.run:notaport/", ""]:
        assert url_origin(url) is None, url
    assert credential_may_reach("https://api.arete.run/ws/sessions", "a4 login")
    assert not credential_may_reach("https://api.arete.run:8443/ws", "secret key")
    assert not credential_may_reach("https://api.arete.run.evil.example/", "secret key")
    assert not credential_may_reach("https://user@api.arete.run/", "secret key")
    env = {"ARETE_API_URL": "http://localhost:3000/"}
    assert credential_may_reach("http://LOCALHOST:3000/ws", "ARETE_API_KEY", environ=env)
    assert not credential_may_reach("http://localhost:3000/ws", "a4 login", environ=env)
    assert credential_may_reach(
        "https://dev.example.test/ws", "secret key", api_url="https://dev.example.test/api"
    )
    assert not credential_may_reach(
        "https://dev.example.test/ws", "ARETE_API_KEY", api_url="https://dev.example.test/api"
    )
