"""Tests for arete.protocol.ProtocolClient (port of
typescript/core/src/protocol/client.test.ts and session.test.ts)."""

from __future__ import annotations

import base64
import json
from typing import Callable, Dict, List

import httpx
import pytest

import arete.protocol.client as protocol_client
from arete import AuthConfig
from arete.encoding import decode_base58
from arete.errors import HttpRequestError
from arete.protocol import (
    ProtocolClient,
    ProtocolDefinition,
    ProtocolDeployment,
    ProtocolMetadataFieldDefinition,
    ProtocolPrepared,
    ProtocolRejection,
    ProtocolStepDefinition,
    PROTOCOL_STATE_REJECTION_CODES,
    SQUADS_PROPOSER_MISSING_EVENT,
    decimal_args,
    is_protocol_access_rejection_code,
    is_protocol_state_rejection_code,
)

API = "https://api.example.test/api"
SESSIONS = "https://api.example.test/ws/sessions"
DEPLOYMENT = "7d0c5a52-2f8e-4c1b-9a37-1b6f3c2a9e10"
WALLET = "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf"

DEFINITION = ProtocolDefinition(
    name="tokenRaise",
    version="1.0.0",
    package="token-raise",
    package_release_hash="arete:h1:package-release:sha256:" + "9" * 64,
    protocol_spec_hash="arete:h1:protocol-spec:sha256:" + "a" * 64,
    protocol_release_hash="arete:h1:protocol-release:sha256:" + "f" * 64,
    deployment=ProtocolDeployment(
        deployment_id=DEPLOYMENT,
        access="public",
        api_url=API + "/",
        session_endpoint=SESSIONS,
    ),
    instance_visibility="shared",
    steps={
        "create": ProtocolStepDefinition(scope="create", access="secret", args=("deadline",)),
        "contribute": ProtocolStepDefinition(
            scope="instance",
            access="public",
            guards=("raise_not_open", "hard_cap_exceeded"),
            args=("amount",),
        ),
        "refund": ProtocolStepDefinition(scope="participant", access="public"),
    },
    events={"raise.succeeded": "instance"},
    instance_metadata={
        "title": ProtocolMetadataFieldDefinition(writer="creator", filterable=True),
        "notes": ProtocolMetadataFieldDefinition(writer="creator"),
    },
)

INTENT = {"schema": "arete.execution-intent/v1", "executionId": "exec_1", "step": "contribute"}


class Server:
    """A MockTransport handler: mints sessions and routes the rest."""

    def __init__(self, route: Callable[[httpx.Request], httpx.Response]) -> None:
        self.route = route
        self.requests: List[httpx.Request] = []
        self.mints: List[Dict] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        if str(request.url) == SESSIONS:
            body = json.loads(request.content)
            self.mints.append(
                {"body": body, "authorization": request.headers.get("authorization")}
            )
            return httpx.Response(
                200,
                json={
                    "token": f"session-{len(self.mints)}",
                    "expires_at": 4_000_000_000,
                    "scopes": body["scopes"],
                },
            )
        return self.route(request)

    def calls(self) -> List[httpx.Request]:
        return [request for request in self.requests if str(request.url) != SESSIONS]


@pytest.fixture(autouse=True)
def no_api_url_env(monkeypatch):
    """The descriptor's URLs, whatever the developer's shell exports."""
    monkeypatch.delenv("ARETE_API_URL", raising=False)


def test_endpoints_come_from_options_then_arete_api_url_then_the_descriptor():
    deployment = DEFINITION.deployment
    assert protocol_client.resolve_protocol_endpoints(deployment, environ={}) == (
        DEPLOYMENT,
        API,
        SESSIONS,
    )
    local = {"ARETE_API_URL": " http://127.0.0.1:48705/ "}
    assert protocol_client.resolve_protocol_endpoints(deployment, environ=local) == (
        DEPLOYMENT,
        "http://127.0.0.1:48705/api",
        "http://127.0.0.1:48705/ws/sessions",
    )
    assert protocol_client.resolve_protocol_endpoints(
        deployment, api_url="http://127.0.0.1:4100/api/", deployment_id="dep_other", environ=local
    ) == ("dep_other", "http://127.0.0.1:4100/api", "http://127.0.0.1:4100/ws/sessions")
    assert protocol_client.resolve_protocol_endpoints(
        deployment, session_endpoint="http://s.test/ws/sessions", environ=local
    )[2] == "http://s.test/ws/sessions"


@pytest.mark.asyncio
async def test_arete_api_url_and_the_deployment_override_switch_what_is_called(monkeypatch):
    monkeypatch.setenv("ARETE_API_URL", "http://127.0.0.1:48705")
    seen: List[str] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(str(request.url))
        if request.url.path == "/ws/sessions":
            assert json.loads(request.content)["targetId"] == "dep_other"
            return httpx.Response(
                200, json={"token": "t", "expires_at": 4_000_000_000, "scopes": ["protocol:read"]}
            )
        return httpx.Response(200, json={"instances": [], "nextCursor": None})

    http = httpx.AsyncClient(transport=httpx.MockTransport(handler))
    client = ProtocolClient(
        DEFINITION,
        http_client=http,
        auth=AuthConfig(secret_key="a4_sk_test"),
        deployment_id="dep_other",
    )
    assert client.deployment_id == "dep_other"
    await client.list_instances()
    assert seen == [
        "http://127.0.0.1:48705/ws/sessions",
        "http://127.0.0.1:48705/api/protocol-deployments/dep_other/instances",
    ]


def make_client(route, **options) -> tuple:
    server = Server(route)
    http = httpx.AsyncClient(transport=httpx.MockTransport(server))
    options.setdefault("auth", AuthConfig(publishable_key="a4_pk_test"))
    return ProtocolClient(DEFINITION, http_client=http, **options), server


@pytest.mark.asyncio
async def test_prepare_mints_a_targeted_session_and_returns_the_intent():
    def route(request: httpx.Request) -> httpx.Response:
        assert request.method == "POST"
        assert str(request.url) == f"{API}/protocol-instances/inst%201/steps/contribute/prepare"
        assert request.headers["authorization"] == "Bearer session-1"
        body = json.loads(request.content)
        assert body["wallet"] == WALLET
        assert body["args"] == {"amount": "1000"}
        assert body["transactionFormat"] == "v0"
        assert body["idempotencyKey"] == "key-1"
        assert body["clientContext"] == {"requestId": "req"}
        return httpx.Response(
            200,
            json={"intent": INTENT, "transactions": [{"index": 0, "transactionBase64": "AA=="}]},
        )

    client, server = make_client(route)
    result = await client.prepare(
        "contribute",
        instance="inst 1",
        wallet=WALLET,
        args=decimal_args({"amount": 1000}, ("amount",)),
        transaction_format="v0",
        idempotency_key="key-1",
        client_context={"requestId": "req"},
    )
    assert isinstance(result, ProtocolPrepared)
    assert result.ok is True
    assert result.execution_id == "exec_1"
    assert result.transactions == [{"index": 0, "transactionBase64": "AA=="}]
    assert server.mints == [
        {
            "body": {
                "targetKind": "protocol-deployment",
                "targetId": DEPLOYMENT,
                "scopes": [
                    "protocol:prepare:contribute",
                    "protocol:prepare:refund",
                    "protocol:read",
                    "protocol:submit",
                ],
            },
            "authorization": "Bearer a4_pk_test",
        }
    ]
    await client.aclose()


@pytest.mark.asyncio
async def test_create_steps_post_to_the_deployment_and_secret_keys_prepare_any_step():
    def route(request: httpx.Request) -> httpx.Response:
        assert str(request.url) == f"{API}/protocol-deployments/{DEPLOYMENT}/steps/create/prepare"
        return httpx.Response(200, json={"intent": INTENT, "transactions": []})

    client, server = make_client(route, auth=AuthConfig(secret_key="a4_sk_test"))
    assert client.key_class == "secret"
    result = await client.prepare("create", wallet=WALLET, args={"deadline": "1"})
    assert result.ok
    assert server.mints[0]["body"]["scopes"] == [
        "protocol:prepare:*",
        "protocol:read",
        "protocol:submit",
    ]
    # Reads mint their own read-only session.
    await client.aclose()


@pytest.mark.asyncio
async def test_guard_rejections_are_results_not_errors():
    def route(request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            422,
            json={
                "rejected": "hard_cap_exceeded",
                "step": "contribute",
                "guard": "args.amount + instance.raised <= params.hardCap",
                "detail": {"raised": "10"},
            },
        )

    client, _ = make_client(route)
    result = await client.prepare("contribute", instance="inst_1", wallet=WALLET, args={})
    assert result == ProtocolRejection(
        rejected="hard_cap_exceeded",
        step="contribute",
        guard="args.amount + instance.raised <= params.hardCap",
        detail={"raised": "10"},
    )
    assert result.ok is False
    assert not result.is_access_refusal


@pytest.mark.asyncio
async def test_state_rejections_are_results_with_their_detail():
    def route(request: httpx.Request) -> httpx.Response:
        if "/steps/contribute/" in str(request.url):
            return httpx.Response(
                422,
                json={
                    "rejected": "participant_terminal",
                    "step": "contribute",
                    "detail": {"state": "left"},
                },
            )
        return httpx.Response(
            422,
            json={"rejected": "instance_terminal", "step": "refund", "detail": {"state": "closed"}},
        )

    client, _ = make_client(route)
    result = await client.prepare("contribute", instance="inst_1", wallet=WALLET, args={})
    assert result == ProtocolRejection(
        rejected="participant_terminal", step="contribute", detail={"state": "left"}
    )
    assert result.is_state_rejection
    assert not result.is_access_refusal
    assert is_protocol_state_rejection_code(result.rejected)
    assert not is_protocol_access_rejection_code(result.rejected)

    ended = await client.prepare("refund", instance="inst_1", wallet=WALLET)
    assert ended == ProtocolRejection(
        rejected="instance_terminal", step="refund", detail={"state": "closed"}
    )
    assert ended.is_state_rejection
    assert PROTOCOL_STATE_REJECTION_CODES == ("instance_terminal", "participant_terminal")


@pytest.mark.asyncio
async def test_access_refusals_from_the_api_and_the_session_mint_are_rejections():
    def route(request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            403, headers={"X-Error-Code": "STEP_NOT_IN_SESSION_SCOPE"}, json={"error": "no"}
        )

    client, _ = make_client(route)
    result = await client.prepare("contribute", instance="inst_1", wallet=WALLET)
    assert isinstance(result, ProtocolRejection)
    assert result.rejected == "step_not_in_session_scope"
    assert result.is_access_refusal

    # The session endpoint refuses to mint a session for a step.
    def refusing(request: httpx.Request) -> httpx.Response:
        if str(request.url) == SESSIONS:
            return httpx.Response(
                403, json={"error": "secret key required", "code": "step_requires_secret_key"}
            )
        raise AssertionError("no request after a refused mint")

    http = httpx.AsyncClient(transport=httpx.MockTransport(refusing))
    client = ProtocolClient(
        DEFINITION, auth=AuthConfig(publishable_key="a4_pk_test"), http_client=http
    )
    result = await client.prepare("contribute", instance="inst_1", wallet=WALLET)
    assert isinstance(result, ProtocolRejection)
    assert result.rejected == "step_requires_secret_key"


@pytest.mark.asyncio
async def test_publishable_clients_refuse_secret_steps_without_a_request():
    client, server = make_client(lambda request: httpx.Response(500))
    result = await client.prepare("create", wallet=WALLET)
    assert result == ProtocolRejection(rejected="step_requires_secret_key", step="create")
    unknown = await client.prepare("finalize", wallet=WALLET)
    assert unknown == ProtocolRejection(rejected="step_not_client_callable", step="finalize")
    assert server.requests == []


@pytest.mark.asyncio
async def test_unexpected_failures_raise_with_the_wire_code():
    client, _ = make_client(
        lambda request: httpx.Response(409, json={"error": "conflict", "code": "intent_conflict"})
    )
    with pytest.raises(HttpRequestError) as raised:
        await client.prepare("contribute", instance="inst_1", wallet=WALLET)
    assert raised.value.status == 409
    assert raised.value.code == "intent_conflict"
    assert "returned 409: conflict" in str(raised.value)


@pytest.mark.asyncio
async def test_an_expired_session_is_reminted_and_the_request_replayed_once():
    attempts: List[str] = []

    def route(request: httpx.Request) -> httpx.Response:
        attempts.append(request.headers["authorization"])
        if len(attempts) == 1:
            return httpx.Response(
                401, headers={"X-Error-Code": "token-expired"}, json={"error": "expired"}
            )
        return httpx.Response(200, json={"intent": INTENT, "transactions": []})

    client, server = make_client(route)
    result = await client.prepare("contribute", instance="inst_1", wallet=WALLET)
    assert result.ok
    assert attempts == ["Bearer session-1", "Bearer session-2"]
    assert len(server.mints) == 2

    # A 401 whose body names a refreshable code is replayed too; a second
    # refusal is raised, never replayed again.
    attempts.clear()

    def always(request: httpx.Request) -> httpx.Response:
        attempts.append(request.headers["authorization"])
        return httpx.Response(401, json={"error": "bad", "code": "token_invalid_signature"})

    client, server = make_client(always)
    with pytest.raises(HttpRequestError):
        await client.get_instance("inst_1")
    assert len(attempts) == 2
    assert server.mints[0]["body"]["scopes"] == ["protocol:read"]


@pytest.mark.asyncio
async def test_submit_sends_base64_once_and_never_resends_submitted_unknown():
    def route(request: httpx.Request) -> httpx.Response:
        assert str(request.url) == f"{API}/execution-intents/exec_1/submit"
        assert json.loads(request.content) == {
            "signedTransactions": ["already-base64", base64.b64encode(b"\x01\x02").decode()]
        }
        return httpx.Response(
            200,
            json={
                "executionId": "exec_1",
                "state": "submitted_unknown",
                "transactions": [{"index": 0, "signature": None, "state": "unknown"}],
            },
        )

    client, server = make_client(route)
    prepared = ProtocolPrepared(intent=INTENT, transactions=[])
    state = await client.submit(prepared, ["already-base64", b"\x01\x02"])
    assert state["state"] == "submitted_unknown"
    assert len(server.calls()) == 1

    # A server failure is raised as is, not retried.
    client, server = make_client(lambda request: httpx.Response(503, json={"error": "busy"}))
    with pytest.raises(HttpRequestError):
        await client.submit("exec_1", ["tx"])
    assert len(server.calls()) == 1


@pytest.mark.asyncio
async def test_reads_list_instances_participants_and_intents():
    def route(request: httpx.Request) -> httpx.Response:
        url = str(request.url)
        if url.startswith(f"{API}/protocol-deployments/{DEPLOYMENT}/instances"):
            assert request.url.params["state"] == "open"
            assert request.url.params["metadata.title"] == "Launch"
            assert request.url.params["limit"] == "5"
            return httpx.Response(200, json={"instances": [], "nextCursor": None})
        if url == f"{API}/protocol-instances/inst_1?wallet={WALLET}":
            return httpx.Response(
                200,
                json={
                    "id": "inst_1",
                    "metadata": {"title": "Launch"},
                    "metadataVersion": 3,
                    "allowedSteps": [{"step": "contribute", "allowed": True}],
                },
            )
        if url == f"{API}/protocol-instances/inst_1/participants/{WALLET}":
            return httpx.Response(200, json={"wallet": WALLET, "metadata": {"nick": "a"}})
        if url == f"{API}/execution-intents/exec_1":
            return httpx.Response(
                200, json={"executionId": "exec_1", "state": "completed", "transactions": []}
            )
        raise AssertionError(url)

    client, _ = make_client(route)
    page = await client.list_instances(state="open", metadata={"title": "Launch"}, limit=5)
    assert page == {"instances": [], "nextCursor": None}
    with pytest.raises(Exception, match="not filterable"):
        await client.list_instances(metadata={"notes": "x"})
    steps = await client.allowed_steps("inst_1", WALLET)
    assert steps == [{"step": "contribute", "allowed": True}]
    participant = await client.get_metadata("inst_1", participant=WALLET)
    assert participant.metadata == {"nick": "a"}
    assert (await client.get_intent("exec_1"))["state"] == "completed"


@pytest.mark.asyncio
async def test_wallet_actions_read_the_wallet_route_with_a_read_session():
    path = f"/api/protocol-deployments/{DEPLOYMENT}/wallets/{WALLET}/actions"
    full = {
        "deploymentId": DEPLOYMENT,
        "wallet": WALLET,
        "createSteps": [
            {
                "step": "create",
                "allowed": True,
                "scope": "create",
                "args": [
                    {"name": "deadline", "type": "string", "format": "unixTimestamp", "optional": False, "description": "Unix seconds"},
                    {"name": "memo", "type": "string", "optional": True},
                ],
            }
        ],
        "instances": [
            {
                "instanceId": "inst_1",
                "state": "open",
                "relation": ["participant", "role:admin", "creator"],
                "participantState": "contributed",
                "steps": [
                    {
                        "step": "contribute",
                        "allowed": False,
                        "reason": "raise_not_open",
                        "scope": "instance",
                        "args": [{"name": "amount", "type": "string", "format": "u64", "optional": False}],
                    }
                ],
            }
        ],
        "nextCursor": "c2",
    }
    # An older or sparser response: no optional fields anywhere.
    sparse = {
        "deploymentId": DEPLOYMENT,
        "wallet": WALLET,
        "instances": [{"instanceId": "inst_2", "state": "closed"}],
    }
    replies = [full, sparse]

    def route(request: httpx.Request) -> httpx.Response:
        assert request.method == "GET"
        assert request.url.path == path
        return httpx.Response(200, json=replies.pop(0))

    client, server = make_client(route)
    actions = await client.actions(WALLET, instance="inst_1", cursor="c1", limit=25)
    assert actions["nextCursor"] == "c2"
    assert actions["createSteps"][0]["scope"] == "create"
    assert actions["createSteps"][0]["args"][1] == {"name": "memo", "type": "string", "optional": True}
    instance = actions["instances"][0]
    assert instance["relation"] == ["participant", "role:admin", "creator"]
    assert instance["participantState"] == "contributed"
    assert instance["steps"][0]["reason"] == "raise_not_open"
    assert instance["steps"][0]["args"][0]["name"] == "amount"

    sparse_actions = await client.actions(WALLET)
    assert sparse_actions["createSteps"] == []
    assert sparse_actions["nextCursor"] is None
    assert sparse_actions["instances"] == [
        {
            "instanceId": "inst_2",
            "state": "closed",
            "relation": [],
            "participantState": None,
            "steps": [],
        }
    ]

    with pytest.raises(Exception, match="wallet must be a non-empty string"):
        await client.actions("")

    calls = server.calls()
    assert len(calls) == 2
    assert dict(calls[0].url.params) == {"instance": "inst_1", "cursor": "c1", "limit": "25"}
    assert calls[1].url.query == b""
    assert all(call.headers["authorization"] == "Bearer session-1" for call in calls)
    assert [mint["body"]["scopes"] for mint in server.mints] == [["protocol:read"]]


@pytest.mark.asyncio
async def test_metadata_updates_sign_the_proof_message():
    sent: List[Dict] = []

    def route(request: httpx.Request) -> httpx.Response:
        assert request.method == "PATCH"
        assert str(request.url) == f"{API}/protocol-instances/inst_1/metadata"
        sent.append(json.loads(request.content))
        return httpx.Response(
            200,
            json={"instanceId": "inst_1", "participantId": "p1", "version": 2, "metadata": {}},
        )

    client, _ = make_client(route)
    signed: List[bytes] = []

    async def sign(message: bytes) -> bytes:
        signed.append(message)
        return bytes(range(64))

    result = await client.update_metadata(
        "inst_1", {"nick": "ada"}, participant=WALLET, sign=sign, expires_at=None
    )
    assert result["version"] == 2
    lines = signed[0].decode().split("\n")
    assert lines[:4] == ["arete.protocol-metadata/v1", DEPLOYMENT, "inst_1", WALLET]
    proof = sent[0]["proof"]
    assert proof["wallet"] == WALLET
    assert decode_base58(proof["signature"]) == bytes(range(64))
    assert proof["expiresAt"] == int(lines[5])
    assert sent[0]["participant"] == WALLET
    assert sent[0]["patch"] == {"nick": "ada"}


def sse(*frames: str) -> bytes:
    return "".join(frames).encode()


@pytest.mark.asyncio
async def test_event_streams_resume_after_the_last_event_id(monkeypatch):
    delays: List[float] = []

    async def no_sleep(seconds: float) -> None:
        delays.append(seconds)

    monkeypatch.setattr(protocol_client, "_sleep", no_sleep)
    connections: List[httpx.Request] = []

    async def chunks(*parts: bytes):
        for part in parts:
            yield part

    def route(request: httpx.Request) -> httpx.Response:
        assert str(request.url) == f"{API}/protocol-instances/inst_1/events"
        connections.append(request)
        if len(connections) == 1:
            # Split across chunks, including inside a CRLF and a UTF-8 sequence.
            body = sse(
                "retry: 250\r\n",
                "id: 1\r\nevent: transition\r\ndata: {\"to\":\"open\"}\r\n\r",
                "\n: keepalive\n\n",
                "id: 2\nevent: emit\ndata: {\"event\":\"raise.succeeded\",\"id\":\"eé\"}\n\n",
            )
            return httpx.Response(200, content=chunks(body[:40], body[40:95], body[95:]))
        if len(connections) == 2:
            return httpx.Response(503, json={"error": "restarting"})
        if len(connections) == 3:
            return httpx.Response(
                200,
                content=chunks(
                    sse(
                        "event: error\ndata: {\"code\":\"session_expired\"}\n\n",
                        "id: 3\nevent: participant\ndata: {\"wallet\":\"w\"}\n\n",
                    )
                ),
            )
        return httpx.Response(200, content=chunks(sse("id: 4\nevent: observation\ndata: {}\n\n")))

    errors: List[Exception] = []
    client, server = make_client(route)
    events = []
    async with client.events("inst_1", on_error=errors.append) as stream:
        async for event in stream:
            events.append(event)
            if len(events) == 4:
                break
    assert [(event.type, event.id) for event in events] == [
        ("transition", "1"),
        ("emit", "2"),
        ("participant", "3"),
        ("observation", "4"),
    ]
    assert events[1].event == "raise.succeeded"
    assert events[1].data["id"] == "eé"
    assert "last-event-id" not in connections[0].headers
    assert connections[1].headers["last-event-id"] == "2"
    assert connections[2].headers["last-event-id"] == "2"
    assert connections[3].headers["last-event-id"] == "3"
    assert connections[0].headers["accept"] == "text/event-stream"
    # retry: 250 after the first stream ended, then 1s backoff after the 503.
    assert delays[:2] == [0.25, 1.0]
    assert len(errors) == 1 and isinstance(errors[0], HttpRequestError)
    # The expired-session error event forced a fresh session for connection 4.
    assert len(server.mints) == 2
    assert connections[3].headers["authorization"] == "Bearer session-2"
    assert stream.closed
    assert stream.last_event_id == "4"


@pytest.mark.asyncio
async def test_event_streams_end_on_refusals_the_api_will_repeat(monkeypatch):
    async def no_sleep(seconds: float) -> None:
        return None

    monkeypatch.setattr(protocol_client, "_sleep", no_sleep)
    client, server = make_client(
        lambda request: httpx.Response(404, json={"error": "no such instance", "code": "not_found"})
    )
    stream = client.events("missing", last_event_id="9")
    with pytest.raises(HttpRequestError) as raised:
        async for _ in stream:
            pass
    assert raised.value.status == 404
    assert stream.closed
    assert server.calls()[0].headers["last-event-id"] == "9"


@pytest.mark.asyncio
async def test_backoff_doubles_and_caps_at_thirty_seconds(monkeypatch):
    delays: List[float] = []
    client, _ = make_client(lambda request: httpx.Response(500))
    stream = client.events("inst_1")

    async def record(seconds: float) -> None:
        delays.append(seconds)
        if len(delays) == 7:
            await stream.aclose()

    monkeypatch.setattr(protocol_client, "_sleep", record)
    async for _ in stream:
        pass
    assert delays == [1.0, 2.0, 4.0, 8.0, 16.0, 30.0, 30.0]


def test_decimal_args_converts_named_integers_only():
    assert decimal_args({"a": 5, "b": 6, "c": [1, "2", True], "d": True}, ("a", "c", "d")) == {
        "a": "5",
        "b": 6,
        "c": ["1", "2", True],
        "d": True,
    }


@pytest.mark.asyncio
async def test_squads_onboarding_reads_the_proposer_and_checks_status():
    squads = f"{API}/protocol-deployments/{DEPLOYMENT}/squads"

    def route(request: httpx.Request) -> httpx.Response:
        url = request.url
        base = f"{url.scheme}://{url.host}{url.path}"
        if base == f"{squads}/proposer":
            return httpx.Response(
                200, json={"pubkey": "Proposer1", "permissions": "initiate", "rootKeyId": "root-1"}
            )
        if base == f"{squads}/status":
            return httpx.Response(
                200,
                json={
                    "ready": False,
                    "member": False,
                    "threshold": 2,
                    "multisig": "Multisig1",
                    "proposer": "Proposer1",
                    "problem": "not_member",
                },
            )
        raise AssertionError(f"unexpected {request.url}")

    client, server = make_client(route, auth=AuthConfig(secret_key="a4_sk_test"))
    proposer = await client.squads.proposer()
    assert proposer == {"pubkey": "Proposer1", "permissions": "initiate", "rootKeyId": "root-1"}
    assert not hasattr(client.squads, "add_proposer")
    status = await client.squads.status("Multisig1")
    assert status["problem"] == "not_member"
    assert status["mask"] is None
    calls = server.calls()
    assert [(call.method, call.url.path) for call in calls] == [
        ("GET", f"/api/protocol-deployments/{DEPLOYMENT}/squads/proposer"),
        ("GET", f"/api/protocol-deployments/{DEPLOYMENT}/squads/status"),
    ]
    assert calls[1].url.params["multisig"] == "Multisig1"
    await client.aclose()


@pytest.mark.asyncio
async def test_proposer_missing_system_events_are_delivered():
    data = {
        "id": 7,
        "deploymentId": DEPLOYMENT,
        "instanceId": "inst_1",
        "outboxId": 3,
        "step": "finalize",
        "multisig": "Multisig1",
        "proposer": "Proposer1",
        "problem": "not_member",
        "at": "2026-10-14T00:00:00Z",
    }

    async def chunks(*parts: bytes):
        for part in parts:
            yield part

    def route(request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            content=chunks(
                sse(f"id: 1-2-3-7\nevent: arete.squads.proposerMissing\ndata: {json.dumps(data)}\n\n")
            ),
        )

    client, _ = make_client(route)
    async with client.events("inst_1") as stream:
        async for event in stream:
            break
    assert event.type == SQUADS_PROPOSER_MISSING_EVENT
    assert event.id == "1-2-3-7"
    assert event.data == data
    await client.aclose()


# ---------------------------------------------------------------------------
# Live instance handles


import asyncio  # noqa: E402
from dataclasses import replace as _replace  # noqa: E402
from types import SimpleNamespace  # noqa: E402

from arete.protocol import (  # noqa: E402
    ProtocolInstanceHandle,
    ProtocolInstanceSnapshot,
    ProtocolLiveStack,
)

INSTANCE_URL = f"{API}/protocol-instances/inst_1"


def instance_detail(revision: int, presale: str) -> Dict:
    return {
        "id": "inst_1",
        "deploymentId": DEPLOYMENT,
        "revision": revision,
        "state": "succeeded" if revision > 3 else "open",
        "terminal": False,
        "visibility": "shared",
        "accounts": {"presale": presale},
        "roles": {},
        "createdByWallet": WALLET,
        "createdAt": "2026-10-13T00:00:00Z",
        "updatedAt": "2026-10-13T00:00:00Z",
        "releaseHash": "r",
        "allowedSteps": [],
    }


def handle_route(events: str):
    """Revision 3 with presale P1 on the first read, revision 4 with P2
    after; the event stream sends ``events`` per connection."""
    reads: List[httpx.Request] = []

    def route(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/events"):
            return httpx.Response(
                200, headers={"content-type": "text/event-stream"}, content=events.encode()
            )
        assert str(request.url).split("?")[0] == INSTANCE_URL
        reads.append(request)
        detail = instance_detail(3, "P1") if len(reads) == 1 else instance_detail(4, "P2")
        return httpx.Response(200, json=detail)

    return route, reads


@pytest.fixture
def parked_reconnects(monkeypatch):
    """Event streams never reconnect after a connection ends."""

    async def park(_seconds: float) -> None:
        await asyncio.Event().wait()

    monkeypatch.setattr(protocol_client, "_sleep", park)


async def wait_until(predicate, what: str) -> None:
    for _ in range(200):
        if predicate():
            return
        await asyncio.sleep(0.01)
    raise AssertionError(f"timed out waiting for {what}")


@pytest.mark.asyncio
async def test_instance_handles_load_and_refresh_on_events(parked_reconnects):
    route, reads = handle_route(
        'id: 1\nevent: transition\ndata: {"from":"open","to":"succeeded"}\n\n'
        "id: 2\nevent: participant\ndata: {}\n\n"
    )
    event_gate = asyncio.Event()

    class GatedEvents(httpx.AsyncByteStream):
        def __init__(self, body):
            self.body = body

        async def __aiter__(self):
            await event_gate.wait()
            yield self.body

    def gated_route(request):
        response = route(request)
        if request.url.path.endswith("/events"):
            return httpx.Response(200, headers=response.headers, stream=GatedEvents(response.content))
        return response

    client, _server = make_client(gated_route)
    async with client.instance_handle("inst_1", wallet=WALLET) as handle:
        assert isinstance(handle, ProtocolInstanceHandle)
        snapshots = handle.__aiter__()
        first = await snapshots.__anext__()
        assert isinstance(first, ProtocolInstanceSnapshot)
        assert first.instance["revision"] == 3
        assert first.live is None  # no live stack in the definition

        changes: List[int] = []
        handle.subscribe(lambda: changes.append(handle.current.instance["revision"]))
        event_gate.set()
        updated = await asyncio.wait_for(snapshots.__anext__(), 2)
        assert updated.instance["revision"] == 4
        assert handle.error is None
        await asyncio.sleep(0.05)
        # Two events, one read already running for the first: at most one
        # more read after it.
        assert 2 <= len(reads) <= 3
        assert all(request.url.params["wallet"] == WALLET for request in reads)
        assert changes and changes[-1] == 4

        refreshed = await handle.refresh()
        assert refreshed.instance["revision"] == 4
    assert handle.closed
    assert not client._handles
    with pytest.raises(Exception, match="closed"):
        await handle.refresh()
    # Iteration ends once the handle closes.
    assert [snapshot async for snapshot in snapshots] == []
    await client.aclose()


@pytest.mark.asyncio
async def test_instance_handles_raise_when_the_first_read_fails(parked_reconnects):
    client, _server = make_client(
        lambda request: httpx.Response(404, json={"code": "instance_not_found"})
    )
    with pytest.raises(HttpRequestError) as raised:
        await client.instance_handle("inst_1")
    assert raised.value.status == 404
    assert not client._handles
    await client.aclose()


@pytest.mark.asyncio
async def test_instance_handles_close_with_the_client(parked_reconnects):
    route, _reads = handle_route("")
    client, _server = make_client(route)
    handle = await client.instance_handle("inst_1")
    assert handle.current.instance["state"] == "open"
    await client.aclose()
    assert handle.closed


class FakeStack:
    """A connected stack in the TypeScript shape: views.<Entity>.<mode>.use(key)."""

    def __init__(self) -> None:
        self.asked: List[str] = []
        self.queues: Dict[str, "asyncio.Queue"] = {}

        def use(key: str):
            self.asked.append(key)
            queue: asyncio.Queue = asyncio.Queue()
            self.queues[key] = queue

            async def rows():
                while True:
                    yield await queue.get()

            return rows()

        self.views = SimpleNamespace(
            TokenRaiseInstance=SimpleNamespace(state=SimpleNamespace(use=use))
        )


@pytest.mark.asyncio
async def test_instance_handles_follow_the_live_stack_row(parked_reconnects):
    route, _reads = handle_route("id: 1\nevent: transition\ndata: {}\n\n")
    stack = FakeStack()
    definition = _replace(
        DEFINITION,
        live_stack=ProtocolLiveStack(
            stack="raiseStack", view="TokenRaiseInstance/state", key_account="presale"
        ),
    )
    server = Server(route)
    client = ProtocolClient(
        definition,
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(server)),
        auth=AuthConfig(publishable_key="a4_pk_test"),
        stacks={"raiseStack": stack},
    )
    assert client.stacks == {"raiseStack": stack}
    handle = await client.instance_handle("inst_1")
    try:
        # The first read keys the row by P1; the event's re-read moves it to P2.
        await wait_until(lambda: stack.asked == ["P1", "P2"], "the re-keyed row")
        stack.queues["P2"].put_nowait({"totalDeposit": "42"})
        await wait_until(lambda: handle.current.live is not None, "the live row")
        assert handle.current.live == {"totalDeposit": "42"}
        assert handle.current.instance["accounts"]["presale"] == "P2"
        # A re-read with the same key account keeps the row.
        await handle.refresh()
        assert stack.asked == ["P1", "P2"]
        assert handle.current.live == {"totalDeposit": "42"}
    finally:
        await handle.aclose()
        await client.aclose()


def test_live_rows_of_generated_stacks_match_the_view_wire_id():
    from arete.protocol.handle import live_view_rows
    from arete.subscription import SubscriptionRegistry
    from arete.views import StateViewHandle, ViewDef, ViewsNamespace, state_view_by_path

    parse = lambda row: row  # noqa: E731
    views = ViewsNamespace(
        SubscriptionRegistry.__new__(SubscriptionRegistry),
        {
            "token_raise_instance": {
                "state": ViewDef(
                    mode="state",
                    view="TokenRaiseInstance/state",
                    key_fields=("presale",),
                    parser=parse,
                )
            }
        },
    )
    handle = state_view_by_path(views, "TokenRaiseInstance/state")
    assert isinstance(handle, StateViewHandle)
    assert handle.view == "TokenRaiseInstance/state"
    assert handle._parser is parse
    assert handle._key_fields == ()
    assert live_view_rows(SimpleNamespace(), "X/state", "k") is None
    assert live_view_rows(SimpleNamespace(views=SimpleNamespace()), "X/state", "k") is None


@pytest.mark.asyncio
async def test_receipts_are_reads_and_verify():
    from pathlib import Path

    from arete.protocol import verify_receipt, verify_receipt_on_chain

    vectors = json.loads(
        (
            Path(__file__).resolve().parents[3]
            / "test-vectors/protocol-client/execution-receipt-v1.json"
        ).read_text(encoding="utf-8")
    )
    receipt = vectors["verifyCases"][0]["receipt"]

    def route(request: httpx.Request) -> httpx.Response:
        url = str(request.url)
        if url == f"{API}/protocol-receipts/keys":
            return httpx.Response(200, json=vectors["keySet"])
        if url == f"{API}/protocol-receipts/{receipt['signature']}":
            return httpx.Response(200, json=receipt)
        if url == f"{API}/protocol-receipts/missing":
            return httpx.Response(
                404, json={"error": "no receipt", "code": "protocol-receipt-not-found"}
            )
        raise AssertionError(url)

    client, _ = make_client(route)
    fetched = await client.get_receipt(receipt["signature"])
    keys = await client.get_receipt_keys()
    assert verify_receipt(fetched, keys).key_id == "test-receipt-1"
    assert verify_receipt_on_chain(fetched, vectors["onChainCases"][0]["transaction"]).valid
    with pytest.raises(HttpRequestError) as raised:
        await client.get_receipt("missing")
    assert raised.value.status == 404


@pytest.mark.asyncio
async def test_closing_during_start_cancels_the_read_without_starting_streams(monkeypatch):
    client, _ = make_client(lambda _request: httpx.Response(500))
    entered = asyncio.Event()
    cancelled = asyncio.Event()
    reads = 0

    async def blocked_read(*args, **kwargs):
        nonlocal reads
        reads += 1
        entered.set()
        try:
            await asyncio.Event().wait()
        finally:
            cancelled.set()

    monkeypatch.setattr(client, "get_instance", blocked_read)
    handle = client.instance_handle("inst_1")
    first = asyncio.create_task(handle.start())
    await entered.wait()
    second = asyncio.create_task(handle.start())
    await asyncio.sleep(0)
    # The client owns startup too, so closing it cancels an unfinished handle.
    await client.aclose()
    results = await asyncio.wait_for(asyncio.gather(first, second, return_exceptions=True), 1)
    assert all(isinstance(result, asyncio.CancelledError) for result in results)
    assert reads == 1
    assert cancelled.is_set()
    assert handle.closed
    assert not client._handles
    assert not handle._tasks
    assert handle._starting is None
    await handle.aclose()


@pytest.mark.asyncio
async def test_live_rows_clear_on_key_changes_and_removal(parked_reconnects):
    detail = instance_detail(3, "P1")
    stack = FakeStack()

    def route(request):
        if request.url.path.endswith("/events"):
            return httpx.Response(200, headers={"content-type": "text/event-stream"}, content=b"")
        return httpx.Response(200, json=detail)

    client = ProtocolClient(
        _replace(DEFINITION, live_stack=ProtocolLiveStack(stack="raiseStack", view="TokenRaiseInstance/state", key_account="presale")),
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(Server(route))),
        auth=AuthConfig(publishable_key="a4_pk_test"), stacks={"raiseStack": stack},
    )
    handle = await client.instance_handle("inst_1")
    seen = []
    handle.subscribe(lambda: seen.append(handle.current))
    try:
        await wait_until(lambda: "P1" in stack.queues, "first subscription")
        stack.queues["P1"].put_nowait({"totalDeposit": "11"})
        await wait_until(lambda: handle.current.live is not None, "first row")
        await handle.refresh()
        assert handle.current.live == {"totalDeposit": "11"}
        detail = instance_detail(4, "P2")
        await handle.refresh()
        assert handle.current.live is None
        stack.queues["P1"].put_nowait({"totalDeposit": "stale"})
        await wait_until(lambda: "P2" in stack.queues, "second subscription")
        assert handle.current.live is None
        stack.queues["P2"].put_nowait({"totalDeposit": "22"})
        await wait_until(lambda: handle.current.live is not None, "second row")
        detail = {**detail, "accounts": {}}
        await handle.refresh()
        assert handle.current.live is None
        stack.queues["P2"].put_nowait({"totalDeposit": "stale"})
        await asyncio.sleep(0)
        assert handle.current.live is None
        assert all(s.live is None or s.live["totalDeposit"] == ("11" if s.instance["accounts"].get("presale") == "P1" else "22") for s in seen)
    finally:
        await client.aclose()


@pytest.mark.asyncio
@pytest.mark.parametrize("context_entry", [False, True])
@pytest.mark.parametrize("read_completed", [False, True])
async def test_cancelled_startup_callers_leave_no_unclaimed_handle(monkeypatch, context_entry, read_completed):
    client, _ = make_client(lambda _request: httpx.Response(500))
    entered = asyncio.Event()
    read_cancelled = asyncio.Event()
    handle = client.instance_handle("inst_1")

    async def read(*args, **kwargs):
        entered.set()
        if read_completed:
            # Cancel after the read completes but before its waiter resumes.
            asyncio.get_running_loop().call_soon(caller.cancel)
            return instance_detail(3, "P1")
        try:
            await asyncio.Event().wait()
        finally:
            read_cancelled.set()

    monkeypatch.setattr(client, "get_instance", read)

    async def enter():
        if context_entry:
            async with handle:
                pytest.fail("cancelled context entry returned a handle")
        else:
            await handle
            pytest.fail("cancelled await returned a handle")

    caller = asyncio.create_task(enter())
    await entered.wait()
    if not read_completed:
        caller.cancel()
    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(caller, 1)
    assert handle.closed
    assert not client._handles
    assert not handle._tasks
    assert handle._starting is None
    assert handle._startup_waiters == 0
    if not read_completed:
        assert read_cancelled.is_set()
    await client.aclose()


@pytest.mark.asyncio
async def test_cancelling_one_startup_waiter_keeps_the_other_alive(monkeypatch, parked_reconnects):
    route, _ = handle_route("")
    client, _ = make_client(route)
    entered = asyncio.Event()
    release = asyncio.Event()
    calls = 0

    async def read(*args, **kwargs):
        nonlocal calls
        calls += 1
        entered.set()
        await release.wait()
        return instance_detail(3, "P1")

    monkeypatch.setattr(client, "get_instance", read)
    handle = client.instance_handle("inst_1")
    first = asyncio.create_task(handle.start())
    await entered.wait()
    second = asyncio.create_task(handle.start())
    await asyncio.sleep(0)
    first.cancel()
    with pytest.raises(asyncio.CancelledError):
        await first
    assert not handle.closed
    assert not second.done()
    release.set()
    assert await asyncio.wait_for(second, 1) is handle
    assert calls == 1
    assert handle._startup_waiters == 0
    assert handle in client._handles
    assert handle.current.instance["revision"] == 3
    await client.aclose()
