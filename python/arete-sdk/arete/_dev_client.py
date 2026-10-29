"""How a stack client is wired in dev mode
(``docs/internal/dev-environment-switch.md`` §3).

:func:`prepare` runs once at connect: in direct mode it resolves every
identity the client was built with (gateway, stack, hosted program reads)
in one request. The result names where each surface goes; a surface that
fails carries its refusal and raises it when used, sending nothing.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Callable, Dict, List, Mapping, Optional

from arete.dev_environment import (
    DEFAULT_SESSION_ENDPOINT,
    DevMode,
    EdgePolicy,
    edge_stack_query,
    edge_stack_websocket_url,
    gateway_identity,
    program_read_identity,
    stack_identity,
    stack_not_loaded,
    stack_query_base,
    surface_target,
    surface_unavailable,
    target_str,
)
from arete.errors import AreteError
from arete.gateway import dev_gateway_transports
from arete.http import AuthTokenTarget, HttpAuthClient
from arete.program_read_transport import (
    LOCAL_HTTP,
    HttpProgramReadTransport,
    ProgramReadDescriptor,
)
from arete.stack import ProgramDef, StackDef, StackRelease

REFUSED_ENDPOINT = "http://127.0.0.1:9"


@dataclass
class DevWiring:
    mode: DevMode
    #: The stack socket URL, or the error opening it raises.
    websocket_url: Optional[str]
    socket_error: Optional[BaseException]
    socket_auth: Any
    socket_release: Optional[StackRelease]
    #: Edge: check the edge before the socket opens.
    edge_url: Optional[str]
    query_base: str
    query_http: HttpAuthClient
    chain: Any
    transactions: Any
    http_client: Any
    program_transport: Callable[[str, ProgramDef, Optional[ProgramReadDescriptor]], Any] = field(
        repr=False, default=None  # type: ignore[assignment]
    )


async def prepare(
    stack: StackDef,
    mode: DevMode,
    *,
    overrides: Mapping[str, ProgramReadDescriptor],
    http_client: Any,
) -> DevWiring:
    release = stack.release
    answer: Optional[Dict[str, Any]] = None
    hosted_reads: List[str] = []
    for name in stack.programs:
        descriptor = overrides.get(name) or stack.program_reads.get(name)
        if descriptor is not None and descriptor.transport_kind != LOCAL_HTTP:
            hosted_reads.append(name)

    def read_identity(name: str) -> Dict[str, str]:
        descriptor = overrides.get(name) or stack.program_reads[name]
        return program_read_identity(
            stack.programs[name].program_id, descriptor.binding, descriptor.release
        )

    if mode.resolver is not None:
        request: Dict[str, Any] = {}
        if stack.gateway is not None:
            request["chain"] = gateway_identity(stack.gateway.chain)
            request["transactions"] = gateway_identity(stack.gateway.transactions)
        if hosted_reads:
            request["programReads"] = [read_identity(name) for name in hosted_reads]
        if release is not None:
            request["stacks"] = [stack_identity(release)]
        if request:
            answer = await mode.resolver.resolve(request)
            # Each program read is used on its own: keep its answer under
            # its one-identity request.
            for index, name in enumerate(hosted_reads):
                items = answer.get("programReads")
                if isinstance(items, list) and index < len(items):
                    mode.resolver.prime(
                        {"programReads": [read_identity(name)]},
                        {"programReads": [items[index]]},
                    )

    transports = dev_gateway_transports(stack.gateway, mode, answer, http_client=http_client)

    def refused_http(error: BaseException) -> HttpAuthClient:
        return HttpAuthClient(auth=None, http_client=http_client, refused=error)

    websocket_url: Optional[str] = None
    socket_error: Optional[BaseException] = None
    socket_auth: Any = None
    socket_release: Optional[StackRelease] = None
    query_base = REFUSED_ENDPOINT
    query_http: HttpAuthClient
    if release is None:
        socket_error = stack_not_loaded(stack.name)
        query_http = refused_http(socket_error)
    elif mode.resolver is None:
        assert mode.edge_url is not None
        websocket_url = edge_stack_websocket_url(mode.edge_url, release)
        query_base, params = edge_stack_query(mode.edge_url, release)
        query_http = HttpAuthClient(
            auth=None, http_client=http_client, edge=EdgePolicy(mode.edge_url, None, params)
        )
    else:
        try:
            if answer is None:
                raise stack_not_loaded(stack.name)
            target = surface_target(answer, "stacks")
            websocket_url = target_str(target, "websocketUrl")
            socket_release = StackRelease(
                stack_manifest_hash=target_str(target, "stackManifestHash"),
                live_alias=release.live_alias,
            )
            socket_auth = mode.resolver.token_auth(DEFAULT_SESSION_ENDPOINT)
        except AreteError as error:
            websocket_url = None
            socket_error = error
            query_http = refused_http(error)
        else:
            query_http = HttpAuthClient(
                auth=socket_auth,
                websocket_url=websocket_url,
                http_client=http_client,
                stack_release=socket_release,
            )
            try:
                query_base = stack_query_base(target, stack.name)
            except AreteError as error:
                query_http = refused_http(error)

    def program_transport(
        name: str, definition: ProgramDef, descriptor: Optional[ProgramReadDescriptor]
    ) -> Any:
        if descriptor is None or descriptor.transport_kind == LOCAL_HTTP:
            return HttpProgramReadTransport(
                endpoint=REFUSED_ENDPOINT,
                release=descriptor.release if descriptor is not None else None,  # type: ignore[arg-type]
                http=refused_http(
                    surface_unavailable(
                        f"Program '{name}' has no hosted Program Read binding, which a dev "
                        "environment needs",
                        "Install the program from the registry (`a4 install program`) to "
                        "read it in a dev environment.",
                    )
                ),
            )
        binding = descriptor.binding
        identity = program_read_identity(definition.program_id, binding, descriptor.release)
        if mode.resolver is None:
            assert mode.edge_url is not None
            return HttpProgramReadTransport(
                endpoint=mode.edge_url,
                release=descriptor.release,
                http=HttpAuthClient(
                    auth=None,
                    http_client=http_client,
                    edge=EdgePolicy.for_identity(mode.edge_url, {"programReads": [identity]}),
                ),
            )
        return HttpProgramReadTransport(
            endpoint=binding.endpoint,
            release=descriptor.release,
            http=HttpAuthClient(
                auth=mode.resolver.token_auth(binding.auth.session_endpoint),
                websocket_url=None,
                http_client=http_client,
            ),
            target=AuthTokenTarget(
                kind="program-read-binding",
                target_id=binding.program_read_binding_id,
                release_hash=descriptor.release.program_release_hash,
                scopes=("read",),
            ),
            direct=(mode.resolver, identity),
        )

    return DevWiring(
        mode=mode,
        websocket_url=websocket_url,
        socket_error=socket_error,
        socket_auth=socket_auth,
        socket_release=socket_release,
        edge_url=mode.edge_url if mode.resolver is None else None,
        query_base=query_base,
        query_http=query_http,
        chain=transports.chain,
        transactions=transports.transactions,
        http_client=http_client,
        program_transport=program_transport,
    )
