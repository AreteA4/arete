"""The shared vectors in ``test-vectors/dev-environment-switch/v1/``.

``targets.json`` runs end to end: each surface is built as a client builds
it and its requests are captured by a mock transport.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Dict, List, Optional
from urllib.parse import parse_qsl, urlsplit

import httpx
import pytest


from arete import _dev_client
from arete.auth import AuthConfig
from arete.dev_environment import (
    DEV_IDENTITY_HEADER,
    DevEnvironmentError,
    DevMode,
    SwitchResult,
    check_edge,
    parse_refusal,
    resolve_switch,
    stack_query_base,
    surface_target,
)
from arete.errors import AreteError
from arete.gateway import (
    HostedSolanaGatewayBindings,
    create_hosted_solana_gateway_transports,
    dev_gateway_transports,
)
from arete.http import HttpAuthClient
from arete.program_read_transport import (
    HttpProgramReadTransport,
    ProgramReadRequest,
    program_read_descriptor_from_wire,
)
from arete.protocol import ProtocolClient
from arete.protocol.types import ProtocolDefinition, ProtocolDeployment
from arete.stack import ProgramDef, StackDef, StackEndpoints, StackRelease

VECTORS = (
    Path(__file__).resolve().parents[3] / "test-vectors" / "dev-environment-switch" / "v1"
)
SECRET = "a4_sk_vector"
FAR = 4_102_444_800  # 2100-01-01


def load(name: str) -> Dict[str, Any]:
    data = json.loads((VECTORS / name).read_text())
    assert data["version"] == 1
    return data


def case_ids(data: Dict[str, Any]) -> List[str]:
    return [case["name"] for case in data["cases"]]


# ---------------------------------------------------------------------------
# switch.json

DEV_INPUTS = ("ARETE_DEV_URL", "ARETE_DEV_ENV")


def run_switch(input: Dict[str, Any]):
    read: List[str] = []
    variables = input["variables"]

    def read_var(name: str) -> Optional[str]:
        assert not input["browser"], f"a browser read {name}"
        read.append(name)
        return variables.get(name)

    def session_edge_url() -> Optional[str]:
        read.append("sessionFile")
        return (input["sessionFile"] or {}).get("edgeUrl")

    result = resolve_switch(
        input["option"],
        read_var=read_var,
        session_edge_url=session_edge_url,
        has_secret_key=lambda: bool(input["secretKey"]),
        browser=bool(input["browser"]),
    )
    return result, [name for name in read if name in DEV_INPUTS or name == "sessionFile"]


SWITCH = load("switch.json")


@pytest.mark.parametrize("case", SWITCH["cases"], ids=case_ids(SWITCH))
def test_switch(case: Dict[str, Any]) -> None:
    expect = case["expect"]
    if "error" in expect:
        with pytest.raises(AreteError) as raised:
            run_switch(case["input"])
        assert raised.value.code == expect["error"]["code"]
        return
    result, dev_reads = run_switch(case["input"])
    assert result.to_dict() == expect
    if expect.get("devInputsRead") is False:
        assert dev_reads == []


# ---------------------------------------------------------------------------
# targets.json


class Recorder:
    """A mock transport answering every request, recording each."""

    def __init__(self, resolve_answer: Optional[Dict[str, Any]] = None) -> None:
        self.requests: List[httpx.Request] = []
        self.resolve_answer = resolve_answer

    def handler(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        path = request.url.path
        if path.endswith("/ws/sessions"):
            return httpx.Response(200, json={"token": "t", "expires_at": FAR})
        if path.endswith("/resolve"):
            return httpx.Response(200, json=self.resolve_answer)
        if path.endswith("/instances"):
            return httpx.Response(
                200, json={"items": [], "nextCursor": None}, headers={"X-Arete-Dev-Edge": "1"}
            )
        return httpx.Response(200, json=None, headers={"X-Arete-Dev-Edge": "1"})

    def client(self) -> httpx.AsyncClient:
        return httpx.AsyncClient(transport=httpx.MockTransport(self.handler))

    def take(self) -> List[httpx.Request]:
        taken, self.requests = self.requests, []
        return taken


def body(request: httpx.Request) -> Any:
    return json.loads(request.content)


def split(requests: List[httpx.Request]):
    sessions = [r for r in requests if r.url.path.endswith("/ws/sessions")]
    resolves = [r for r in requests if r.url.path.endswith("/resolve")]
    others = [r for r in requests if r not in sessions and r not in resolves]
    return sessions, resolves, others


def surface_observation(requests: List[httpx.Request], endpoint_of) -> Dict[str, Any]:
    sessions, _, others = split(requests)
    assert others, "the surface sent its request"
    request = others[0]
    identity = request.headers.get(DEV_IDENTITY_HEADER)
    return {
        "endpoint": endpoint_of(request),
        "tokenRequest": body(sessions[0]) if sessions else None,
        "sessionEndpoint": str(sessions[0].url) if sessions else None,
        "identityHeader": json.loads(identity) if identity else None,
        "authorization": request.headers.get("Authorization"),
    }


def origin_of(prefix: str):
    def endpoint(request: httpx.Request) -> str:
        url = str(request.url)
        return url[: url.index(prefix)]

    return endpoint


def stack_def(installed: Dict[str, Any]) -> StackDef:
    return StackDef(
        name=installed["stack"]["name"],
        endpoints=StackEndpoints(
            ws=installed["stack"]["endpoints"]["ws"], http=installed["stack"]["endpoints"]["http"]
        ),
        programs={
            "klend": ProgramDef(name="klend", program_id=installed["programs"]["klend"]["programId"])
        },
        program_reads={
            "klend": program_read_descriptor_from_wire(installed["programReads"]["klend"])
        },
        gateway=HostedSolanaGatewayBindings.from_dict(installed["gateway"]),
        release=StackRelease(
            stack_manifest_hash=installed["stack"]["release"]["stackManifestHash"],
            live_alias=installed["stack"]["release"]["liveAlias"],
        ),
    )


def protocol_def(installed: Dict[str, Any]) -> ProtocolDefinition:
    protocol = installed["protocol"]
    deployment = protocol["deployment"] or {
        "deploymentId": "",
        "access": "private",
        "apiUrl": "https://api.arete.run/api",
        "sessionEndpoint": "https://api.arete.run/ws/sessions",
    }
    return ProtocolDefinition(
        name="tokenRaise",
        version="1.0.0",
        package=protocol["package"],
        package_release_hash="arete:h1:package-release:sha256:" + "9" * 64,
        protocol_spec_hash="arete:h1:protocol-spec:sha256:" + "a" * 64,
        protocol_release_hash=protocol["protocolReleaseHash"],
        deployment=ProtocolDeployment(
            deployment_id=deployment["deploymentId"],
            access=deployment["access"],
            api_url=deployment["apiUrl"],
            session_endpoint=deployment["sessionEndpoint"],
        ),
        instance_visibility="shared",
    )


async def observe(
    installed: Dict[str, Any], mode: Dict[str, Any], answer: Optional[Dict[str, Any]]
) -> Dict[str, Any]:
    recorder = Recorder(answer)
    http = recorder.client()
    stack = stack_def(installed)
    switch = SwitchResult(
        mode=mode["mode"],
        via=mode.get("via"),
        edge_url=mode.get("edgeUrl"),
        environment=mode.get("environment"),
    )
    secret = AuthConfig(secret_key=SECRET)
    public = AuthConfig(publishable_key="a4_pk_vector")
    observed: Dict[str, Any] = {}
    resolve_requests: List[httpx.Request] = []

    dev = DevMode.of(switch, secret, http)
    if dev is None:
        transports = create_hosted_solana_gateway_transports(
            stack.gateway, auth=public, http_client=http
        )
        chain, transactions = transports.chain, transports.transactions
        descriptor = stack.program_reads["klend"]
        program = HttpProgramReadTransport.hosted(
            descriptor.binding,
            descriptor.release,
            HttpAuthClient(
                auth=AuthConfig(
                    publishable_key="a4_pk_vector",
                    token_endpoint=descriptor.binding.auth.session_endpoint,
                ),
                http_client=http,
            ),
        )
        websocket_url = stack.endpoints.ws
        query_base = stack.endpoints.http
        query_http = HttpAuthClient(
            auth=public,
            websocket_url=websocket_url,
            http_client=http,
            stack_release=stack.release,
        )
    else:
        wiring = await _dev_client.prepare(stack, dev, overrides={}, http_client=http)
        resolve_requests += split(recorder.take())[1]
        chain, transactions = wiring.chain, wiring.transactions
        program = wiring.program_transport(
            "klend", stack.programs["klend"], stack.program_reads["klend"]
        )
        websocket_url = wiring.websocket_url
        query_base = wiring.query_base
        query_http = wiring.query_http

    recorder.take()
    try:
        await chain.lamports("11111111111111111111111111111111")
    except Exception:
        pass  # the mock's answer is not a balance; the request is what counts
    observed["chain"] = surface_observation(recorder.take(), origin_of("/chain/"))
    try:
        await transactions.get_latest_blockhash()
    except Exception:
        pass  # the mock's answer is not a blockhash; the request is what counts
    observed["transactions"] = surface_observation(recorder.take(), origin_of("/transactions/"))
    await program.read(ProgramReadRequest(operation="fetch", account="Reserve", address="x"))

    requests = recorder.take()
    resolve_requests += split(requests)[1]
    read = surface_observation(requests, origin_of("/v1/releases/"))
    read["pathReleaseHash"] = (
        str(split(requests)[2][0].url).split("/v1/releases/")[1].split("/")[0]
    )
    observed["programReads"] = {"klend": read}

    token = await query_http.get_token(scopes=["read"])
    sessions = split(recorder.take())[0]
    await query_http.request_json("GET", f"{query_base}/query")
    query_request = split(recorder.take())[2][0]
    observed["stack"] = {
        "websocketUrl": websocket_url,
        "queryBaseUrl": str(query_request.url).split("?")[0][: -len("/query")],
        "queryParams": dict(parse_qsl(urlsplit(str(query_request.url)).query)),
        "sessionRequired": token is not None,
        "sessionRequest": body(sessions[0]) if sessions else None,
        "sessionEndpoint": str(sessions[0].url) if sessions else None,
        "identityHeader": None,
        "authorization": query_request.headers.get("Authorization"),
    }

    environment: Any = "production"
    if switch.via == "edge":
        environment = {"kind": "dev", "url": switch.edge_url}
    elif switch.via == "direct":
        environment = {"kind": "dev", "env": switch.environment}
    client = ProtocolClient(
        protocol_def(installed), auth=secret, http_client=http, environment=environment
    )
    await client.list_instances()
    requests = recorder.take()
    sessions, resolves, others = split(requests)
    resolve_requests += resolves
    instances = str(others[0].url)
    api_url = instances.split("/protocol-deployments/")[0]
    segment = instances.split("/protocol-deployments/")[1].split("/")[0]
    identity = others[0].headers.get(DEV_IDENTITY_HEADER)
    observed["protocol"] = {
        "apiUrl": api_url,
        "sessionEndpoint": str(sessions[0].url) if sessions else None,
        "deploymentId": (
            body(sessions[0])["targetId"] if sessions else (client.deployment_id or None)
        ),
        "deploymentPathSegment": segment,
        "instancesUrl": instances,
        "identityHeader": json.loads(identity) if identity else None,
        "authorization": others[0].headers.get("Authorization"),
    }

    merged: Dict[str, Any] = {}
    for resolve in resolve_requests:
        merged.update(body(resolve))
    urls = {str(resolve.url) for resolve in resolve_requests}
    keys = {resolve.headers.get("Authorization") for resolve in resolve_requests}
    assert len(urls) <= 1 and len(keys) <= 1
    observed["resolveRequest"] = (
        {
            "url": urls.pop(),
            "authorization": "secret-key" if keys.pop() == f"Bearer {SECRET}" else None,
            "body": merged,
        }
        if resolve_requests
        else None
    )
    await http.aclose()
    return observed


TARGETS = load("targets.json")


@pytest.mark.parametrize("case", TARGETS["cases"], ids=case_ids(TARGETS))
@pytest.mark.asyncio
async def test_targets(case: Dict[str, Any], monkeypatch: pytest.MonkeyPatch) -> None:
    for name in ("ARETE_ENVIRONMENT", "ARETE_DEV_URL", "ARETE_DEV_ENV", "ARETE_API_URL"):
        monkeypatch.delenv(name, raising=False)
    installed = TARGETS["installed"][case["input"]["installed"]]
    observed = await observe(installed, case["input"]["mode"], case["input"]["resolveResponse"])
    for surface, expected in case["expect"].items():
        got = observed[surface]
        if surface == "programReads":
            got, expected = got["klend"], expected["klend"]
        if surface != "resolveRequest" and isinstance(got, dict) and "authorization" in got:
            # tokenRequest null means no Authorization header is sent.
            authorization = got.pop("authorization")
            if expected.get("tokenRequest", expected.get("sessionRequest", 0)) is None:
                assert authorization is None, f"{surface} sent a credential"
        if surface == "transactions" and got.get("tokenRequest") and expected.get("tokenRequest"):
            # Each transaction route mints for its own scope (inspect or
            # send), a subset of the binding's.
            minted = got["tokenRequest"]["scopes"]
            assert set(minted) <= set(expected["tokenRequest"]["scopes"])
            got["tokenRequest"]["scopes"] = expected["tokenRequest"]["scopes"]
        assert got == expected, surface


# ---------------------------------------------------------------------------
# refusals.json


def assert_refusal(error: BaseException, expect: Dict[str, Any]) -> None:
    assert isinstance(error, AreteError), repr(error)
    assert error.code == expect["code"], str(error)
    if not isinstance(error, DevEnvironmentError):
        return
    if "message" in expect:
        assert error.message == expect["message"]
    if "fix" in expect:
        assert error.fix == expect["fix"]
    if "details" in expect:
        details = {k: v for k, v in error.details.items() if k != "fix"}
        assert details == expect["details"]


def answering(status: int, headers: Dict[str, str], content: Any) -> httpx.AsyncClient:
    def handler(request: httpx.Request) -> httpx.Response:
        if isinstance(content, str):
            return httpx.Response(status, text=content, headers=headers)
        return httpx.Response(status, json=content, headers=headers)

    return httpx.AsyncClient(transport=httpx.MockTransport(handler))


def refusing_connections() -> httpx.AsyncClient:
    def handler(request: httpx.Request) -> httpx.Response:
        raise httpx.ConnectError("connection refused", request=request)

    return httpx.AsyncClient(transport=httpx.MockTransport(handler))


async def first_edge_request(http: httpx.AsyncClient, edge_url: str) -> BaseException:
    installed = TARGETS["installed"]["default"]
    transports = dev_gateway_transports(
        HostedSolanaGatewayBindings.from_dict(installed["gateway"]),
        DevMode(SwitchResult(mode="dev", via="edge", edge_url=edge_url)),
        None,
        http_client=http,
    )
    with pytest.raises(Exception) as raised:
        await transports.chain.lamports("11111111111111111111111111111111")
    return raised.value


SURFACES = {
    "chain": ("chain", 0),
    "transactions": ("transactions", 0),
    "stack": ("stacks", 0),
    "protocol": ("protocols", 0),
    "programReads.klend": ("programReads", 0),
}

REFUSALS = load("refusals.json")


@pytest.mark.parametrize("case", REFUSALS["cases"], ids=case_ids(REFUSALS))
@pytest.mark.asyncio
async def test_refusals(case: Dict[str, Any]) -> None:
    input, expect = case["input"], case["expect"]
    source = input["source"]
    if source == "switch":
        with pytest.raises(AreteError) as raised:
            run_switch(input["switch"])
        assert_refusal(raised.value, expect)
    elif source == "edge":
        edge_url = input["mode"]["edgeUrl"]
        answer = input["edgeAnswer"]
        http = (
            refusing_connections()
            if answer["kind"] == "connection-refused"
            else answering(answer["status"], answer["headers"], answer["body"])
        )
        if answer.get("request") == "GET /.well-known/arete-dev-edge":
            # The check before a socket: it fails, so none opens.
            with pytest.raises(AreteError) as raised:
                await check_edge(http, edge_url)
            error: BaseException = raised.value
        else:
            error = await first_edge_request(http, edge_url)
        assert_refusal(error, expect)
    elif source == "resolve":
        answer = input["resolveResponse"]
        failing = expect["failingSurface"]
        with pytest.raises(AreteError) as raised:
            if failing == "stackQuery":
                stack_query_base(surface_target(answer, "stacks"), "klend")
            else:
                surface_target(answer, *SURFACES[failing])
        assert_refusal(raised.value, expect)
        if expect.get("otherSurfacesUsable"):
            for name, (surface, index) in SURFACES.items():
                if name != failing:
                    surface_target(answer, surface, index)
    elif source == "service":
        response = input["response"]
        header = next(
            (v for k, v in response["headers"].items() if k.lower() == "x-error-code"), None
        )
        refusal = parse_refusal(response["status"], header, response["body"])
        assert refusal is not None
        assert_refusal(refusal, expect)
        if input["mode"].get("via") == "edge":
            http = answering(response["status"], response["headers"], response["body"])
            assert_refusal(
                await first_edge_request(http, input["mode"]["edgeUrl"]), expect
            )
    else:  # pragma: no cover
        raise AssertionError(source)


@pytest.mark.asyncio
async def test_production_reads_no_dev_input_and_sends_no_environment(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """With the switch unset the protocol client's session request is the
    one it always sent."""
    for name in ("ARETE_ENVIRONMENT", "ARETE_DEV_URL", "ARETE_DEV_ENV"):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("ARETE_DEV_URL", "not read")
    recorder = Recorder()
    client = ProtocolClient(
        protocol_def(TARGETS["installed"]["default"]),
        auth=AuthConfig(secret_key=SECRET),
        http_client=recorder.client(),
    )
    assert client.environment == "production"
    await client.list_instances()
    sessions, _, _ = split(recorder.take())
    assert "environment" not in body(sessions[0])


@pytest.mark.asyncio
async def test_arete_environment_variable_routes_a_client_through_the_edge(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from arete import Arete

    monkeypatch.setenv("ARETE_ENVIRONMENT", "dev")
    monkeypatch.setenv("ARETE_DEV_URL", "http://127.0.0.1:8911/")
    recorder = Recorder()
    http = recorder.client()
    stack = stack_def(TARGETS["installed"]["default"])
    client = await Arete.connect(stack, transport="http", http_client=http)
    assert client.environment == "dev"
    try:
        await client.chain.lamports("11111111111111111111111111111111")
    except Exception:
        pass
    (request,) = recorder.take()
    assert str(request.url).startswith("http://127.0.0.1:8911/chain/")
    assert "Authorization" not in request.headers
    assert json.loads(request.headers[DEV_IDENTITY_HEADER]) == {
        "chain": {"solanaGatewayBindingId": stack.gateway.chain.solana_gateway_binding_id}
    }


@pytest.mark.asyncio
async def test_an_unknown_switch_value_is_a_configuration_error(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from arete import Arete

    monkeypatch.setenv("ARETE_ENVIRONMENT", "staging")
    stack = stack_def(TARGETS["installed"]["default"])
    with pytest.raises(AreteError) as raised:
        await Arete.connect(stack, transport="http", http_client=Recorder().client())
    assert raised.value.code == "INVALID_CONFIG"


@pytest.mark.asyncio
async def test_a_production_client_reports_production(monkeypatch: pytest.MonkeyPatch) -> None:
    from arete import Arete

    monkeypatch.delenv("ARETE_ENVIRONMENT", raising=False)
    stack = stack_def(TARGETS["installed"]["default"])
    client = await Arete.connect(stack, transport="http", http_client=Recorder().client())
    assert client.environment == "production"


@pytest.mark.asyncio
async def test_a_refused_stack_leaves_the_other_surfaces_working(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Edge mode with a stack that has no release: the stack refuses with
    devenv-stack-not-loaded, chain reads still go to the edge."""
    from dataclasses import replace

    from arete import Arete

    monkeypatch.setenv("ARETE_ENVIRONMENT", "dev")
    monkeypatch.setenv("ARETE_DEV_URL", "http://127.0.0.1:8911")
    recorder = Recorder()
    stack = replace(stack_def(TARGETS["installed"]["default"]), release=None)
    client = await Arete.connect(stack, http_client=recorder.client())
    with pytest.raises(AreteError) as raised:
        await client.connect_socket()
    assert raised.value.code == "devenv-stack-not-loaded"
    try:
        await client.chain.lamports("11111111111111111111111111111111")
    except Exception:
        pass
    assert any(str(r.url).startswith("http://127.0.0.1:8911/chain/") for r in recorder.take())


@pytest.mark.asyncio
async def test_a_deferred_socket_still_checks_the_edge(monkeypatch: pytest.MonkeyPatch) -> None:
    from arete import Arete

    monkeypatch.setenv("ARETE_ENVIRONMENT", "dev")
    monkeypatch.setenv("ARETE_DEV_URL", "http://127.0.0.1:8911")
    # Not an edge: no marker on any answer.
    http = answering(200, {}, {"result": 1})
    stack = stack_def(TARGETS["installed"]["default"])
    client = await Arete.connect(stack, http_client=http, auto_connect=False)
    with pytest.raises(AreteError) as raised:
        await client.connect_socket()
    assert raised.value.code == "devenv-edge-unreachable"


@pytest.mark.asyncio
async def test_a_refused_stack_session_keeps_its_code() -> None:
    from arete.auth import AuthState

    refusal = {
        "schemaVersion": 1,
        "error": "not loaded",
        "code": "devenv-stack-not-loaded",
        "retryable": False,
        "fix": "the fix",
    }
    state = AuthState(
        "wss://s1.devstack.arete.run",
        AuthConfig(token_endpoint="https://api.arete.run/ws/sessions", secret_key=SECRET),
    )
    state._http_session = answering(409, {}, refusal)
    with pytest.raises(DevEnvironmentError) as raised:
        await state.resolve_token()
    assert raised.value.code == "devenv-stack-not-loaded"
    assert raised.value.fix == "the fix"


@pytest.mark.asyncio
async def test_a_composition_session_shares_dev_transports(monkeypatch: pytest.MonkeyPatch) -> None:
    from arete import create_session

    monkeypatch.setenv("ARETE_ENVIRONMENT", "dev")
    monkeypatch.setenv("ARETE_DEV_URL", "http://127.0.0.1:8911")
    recorder = Recorder()
    stack = stack_def(TARGETS["installed"]["default"])
    session = await create_session(
        stacks={"klend": stack},
        mode="composition",
        transport="http",
        http_client=recorder.client(),
    )
    assert session.environment == "dev"
    recorder.take()
    try:
        await session.chain.lamports("11111111111111111111111111111111")
    except Exception:
        pass
    (request,) = recorder.take()
    assert str(request.url).startswith("http://127.0.0.1:8911/chain/")
    assert "Authorization" not in request.headers


@pytest.mark.asyncio
async def test_a_dev_client_closes_the_http_client_it_made(monkeypatch: pytest.MonkeyPatch) -> None:
    from arete import Arete

    monkeypatch.setenv("ARETE_ENVIRONMENT", "dev")
    monkeypatch.setenv("ARETE_DEV_URL", "http://127.0.0.1:8911")
    stack = stack_def(TARGETS["installed"]["default"])
    client = await Arete.connect(stack, transport="http")
    owned = client._owned_dev_http
    assert owned is not None and not owned.is_closed
    await client.aclose()
    assert owned.is_closed
