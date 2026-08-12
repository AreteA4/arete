"""``program-read-http/v1`` transport and descriptor types.

Python projection of ``typescript/core/src/program-read-transport.ts`` plus
the descriptor validation from ``typescript/core/src/client.ts``
(``validateProgramReadDescriptor``). Rust sibling:
``arete_sdk::program_read_transport``. Wire fixtures:
``typescript/core/src/program-read-contract-v1.fixture.json``.

- ``GET  <endpoint>/v1/releases/<release>/accounts/<Account>/<address>`` —
  fetch one account (``null`` body means missing).
- ``GET  …/<address>/exists`` → ``{"exists": bool}``.
- ``POST …/accounts/<Account>`` with ``{"addresses": […]}`` → per-address
  ``ok``/``missing``/``error`` items.

All HTTP goes through :class:`arete.http.HttpAuthClient` (``request_json``),
which owns token strategy and refresh-replay-once; hosted bindings forward a
targeted :class:`arete.http.AuthTokenTarget` per request.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Any, ClassVar, Mapping, Optional, Tuple, Union
from urllib.parse import quote, urlsplit

from arete.http import AuthTokenTarget
from arete.read import (
    READ_SCOPES,
    ProgramReadRequest,
    _coded_error,
    coerce_read_request_error,
)

PROGRAM_READ_CONTRACT_VERSION = "program-read-http/v1"

LOCAL_HTTP = "local-http"
HOSTED_BINDING = "hosted-binding"

_BINDING_ID_RE = re.compile(r"^prb_[A-Za-z0-9_-]{32}$")


# ---------------------------------------------------------------------------
# Descriptor types (TS `types.ts`, camelCase on the wire)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ProgramReleaseReference:
    """Generated release identity for a program (TS ``ProgramReleaseReference``)."""

    program_release_hash: str
    program_spec_hash: str


@dataclass(frozen=True)
class HttpAuthMetadata:
    """Public, non-secret token acquisition metadata (TS ``HttpAuthMetadata``)."""

    session_endpoint: str
    target_kind: str
    target_id: str
    required: Optional[bool] = None
    mode: Optional[str] = None
    jwks_url: Optional[str] = None
    token_transport: Optional[str] = None
    audience: Optional[str] = None
    scopes: Optional[Tuple[str, ...]] = None
    accepted_key_classes: Optional[Tuple[str, ...]] = None


@dataclass(frozen=True)
class ProgramReadBinding:
    """One generated, non-inheriting hosted program read binding."""

    endpoint: str
    program_read_binding_id: str
    auth: HttpAuthMetadata


@dataclass(frozen=True)
class LocalHttpTransportDef:
    """Local transport addressing the connect-time HTTP endpoint."""

    endpoint_source: str = "connect-http-url"
    kind: ClassVar[str] = LOCAL_HTTP


@dataclass(frozen=True)
class HostedBindingTransportDef:
    """Hosted transport addressing the generated binding endpoint."""

    binding: ProgramReadBinding
    kind: ClassVar[str] = HOSTED_BINDING


@dataclass(frozen=True)
class ProgramReadDescriptor:
    """Generated release identity with one explicit, non-inheriting read transport."""

    release: ProgramReleaseReference
    transport: Union[LocalHttpTransportDef, HostedBindingTransportDef]

    @property
    def transport_kind(self) -> str:
        return self.transport.kind

    @property
    def binding(self) -> Optional[ProgramReadBinding]:
        if isinstance(self.transport, HostedBindingTransportDef):
            return self.transport.binding
        return None


# ---------------------------------------------------------------------------
# Wire (de)serialization — the exact TS-generated JSON shape
# ---------------------------------------------------------------------------


def program_read_descriptor_from_wire(data: Mapping[str, Any]) -> ProgramReadDescriptor:
    release_data = data.get("release")
    release_data = release_data if isinstance(release_data, Mapping) else {}
    release = ProgramReleaseReference(
        program_release_hash=release_data.get("programReleaseHash", ""),
        program_spec_hash=release_data.get("programSpecHash", ""),
    )
    transport_data = data.get("transport")
    if not isinstance(transport_data, Mapping):
        raise _coded_error("Program read descriptor requires a transport", "INVALID_CONFIG")
    kind = transport_data.get("kind")
    if kind == LOCAL_HTTP:
        if transport_data.get("endpointSource") != "connect-http-url":
            raise _coded_error(
                "Local HTTP transport must use endpointSource 'connect-http-url'",
                "INVALID_CONFIG",
            )
        return ProgramReadDescriptor(release=release, transport=LocalHttpTransportDef())
    if kind == HOSTED_BINDING:
        binding_data = transport_data.get("binding")
        binding_data = binding_data if isinstance(binding_data, Mapping) else {}
        auth_data = binding_data.get("auth")
        auth_data = auth_data if isinstance(auth_data, Mapping) else {}
        auth = HttpAuthMetadata(
            session_endpoint=auth_data.get("sessionEndpoint", ""),
            target_kind=auth_data.get("targetKind", ""),
            target_id=auth_data.get("targetId", ""),
            required=auth_data.get("required"),
            mode=auth_data.get("mode"),
            jwks_url=auth_data.get("jwksUrl"),
            token_transport=auth_data.get("tokenTransport"),
            audience=auth_data.get("audience"),
            scopes=tuple(auth_data["scopes"]) if auth_data.get("scopes") is not None else None,
            accepted_key_classes=(
                tuple(auth_data["acceptedKeyClasses"])
                if auth_data.get("acceptedKeyClasses") is not None
                else None
            ),
        )
        binding = ProgramReadBinding(
            endpoint=binding_data.get("endpoint", ""),
            program_read_binding_id=binding_data.get("programReadBindingId", ""),
            auth=auth,
        )
        return ProgramReadDescriptor(
            release=release, transport=HostedBindingTransportDef(binding=binding)
        )
    raise _coded_error(
        "Program read descriptor has an unsupported transport", "INVALID_CONFIG"
    )


def program_read_descriptor_to_wire(descriptor: ProgramReadDescriptor) -> dict:
    release = {
        "programReleaseHash": descriptor.release.program_release_hash,
        "programSpecHash": descriptor.release.program_spec_hash,
    }
    if isinstance(descriptor.transport, LocalHttpTransportDef):
        return {
            "release": release,
            "transport": {"kind": LOCAL_HTTP, "endpointSource": "connect-http-url"},
        }
    binding = descriptor.transport.binding
    auth: dict = {
        "sessionEndpoint": binding.auth.session_endpoint,
        "targetKind": binding.auth.target_kind,
        "targetId": binding.auth.target_id,
    }
    optional_fields = {
        "required": binding.auth.required,
        "mode": binding.auth.mode,
        "jwksUrl": binding.auth.jwks_url,
        "tokenTransport": binding.auth.token_transport,
        "audience": binding.auth.audience,
        "scopes": list(binding.auth.scopes) if binding.auth.scopes is not None else None,
        "acceptedKeyClasses": (
            list(binding.auth.accepted_key_classes)
            if binding.auth.accepted_key_classes is not None
            else None
        ),
    }
    auth.update({key: value for key, value in optional_fields.items() if value is not None})
    return {
        "release": release,
        "transport": {
            "kind": HOSTED_BINDING,
            "binding": {
                "endpoint": binding.endpoint,
                "programReadBindingId": binding.program_read_binding_id,
                "auth": auth,
            },
        },
    }


# ---------------------------------------------------------------------------
# Descriptor validation (TS `validateProgramReadDescriptor`)
# ---------------------------------------------------------------------------


def _is_non_empty(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _is_secure_or_loopback_http_url(value: str) -> bool:
    try:
        parts = urlsplit(value)
        host = parts.hostname
    except ValueError:
        return False
    if not host:
        return False
    if parts.scheme == "https":
        return True
    return parts.scheme == "http" and host in ("localhost", "127.0.0.1", "::1")


def validate_program_read_descriptor(
    program_name: str, descriptor: ProgramReadDescriptor
) -> None:
    """Fail-closed descriptor validation; raises ``AreteError`` (``INVALID_CONFIG``)."""
    release = getattr(descriptor, "release", None)
    if (
        release is None
        or not _is_non_empty(getattr(release, "program_release_hash", None))
        or not _is_non_empty(getattr(release, "program_spec_hash", None))
    ):
        raise _coded_error(
            f"Program '{program_name}' read descriptor requires a complete release",
            "INVALID_CONFIG",
        )
    transport = getattr(descriptor, "transport", None)
    if transport is None:
        raise _coded_error(
            f"Program '{program_name}' read descriptor requires a transport",
            "INVALID_CONFIG",
        )
    kind = getattr(transport, "kind", None)
    if kind == LOCAL_HTTP:
        if getattr(transport, "endpoint_source", None) != "connect-http-url":
            raise _coded_error(
                f"Program '{program_name}' local HTTP transport must use "
                "endpointSource 'connect-http-url'",
                "INVALID_CONFIG",
            )
        return
    if kind != HOSTED_BINDING:
        raise _coded_error(
            f"Program '{program_name}' read descriptor has an unsupported transport",
            "INVALID_CONFIG",
        )
    binding = getattr(transport, "binding", None)
    auth = getattr(binding, "auth", None)
    valid = (
        binding is not None
        and _is_secure_or_loopback_http_url(binding.endpoint)
        and _BINDING_ID_RE.match(binding.program_read_binding_id) is not None
        and auth is not None
        and auth.target_kind == "program-read-binding"
        and auth.target_id == binding.program_read_binding_id
        and _is_secure_or_loopback_http_url(auth.session_endpoint)
    )
    if not valid:
        raise _coded_error(
            f"Program '{program_name}' hosted binding requires secure endpoints, "
            "a canonical binding ID, and matching program-read-binding auth metadata",
            "INVALID_CONFIG",
        )


# ---------------------------------------------------------------------------
# Release-addressed request paths (TS `requestPath` / `appendUrl`)
# ---------------------------------------------------------------------------


def _encode_uri_component(value: str) -> str:
    # JS encodeURIComponent: unreserved set A-Z a-z 0-9 - _ . ! ~ * ' ( )
    return quote(value, safe="!~*'()")


def request_path(release: ProgramReleaseReference, request: ProgramReadRequest) -> str:
    """Release-addressed path; ``%3A`` is restored to ``:`` so typed hashes stay readable."""
    release_hash = _encode_uri_component(release.program_release_hash).replace("%3A", ":")
    root = f"/v1/releases/{release_hash}/accounts/{_encode_uri_component(request.account)}"
    if request.operation == "fetch_many":
        return root
    address_path = f"{root.rstrip('/')}/{_encode_uri_component(request.address or '')}"
    return f"{address_path}/exists" if request.operation == "exists" else address_path


def _append_url(base: str, path: str) -> str:
    return f"{base.rstrip('/')}/{path.lstrip('/')}"


# ---------------------------------------------------------------------------
# Transports
# ---------------------------------------------------------------------------


class UnavailableProgramReadTransport:
    """Transport that fails every read (TS ``kind: 'unavailable'``)."""

    def __init__(self, message: str) -> None:
        self._message = message

    async def read(self, request: ProgramReadRequest) -> Any:
        raise _coded_error(self._message, "INVALID_CONFIG")


class HttpProgramReadTransport:
    """Release-addressed ``program-read-http/v1`` transport (local + hosted)."""

    def __init__(
        self,
        *,
        endpoint: str,
        release: ProgramReleaseReference,
        http: Any,
        target: Optional[AuthTokenTarget] = None,
    ) -> None:
        self._endpoint = endpoint
        self._release = release
        self._http = http
        self._target = target

    @classmethod
    def local_http(
        cls, connect_http_url: str, release: ProgramReleaseReference, http: Any
    ) -> "HttpProgramReadTransport":
        """Local transport on the connect-time HTTP endpoint; no targeted token."""
        return cls(endpoint=connect_http_url, release=release, http=http)

    @classmethod
    def hosted(
        cls,
        binding: ProgramReadBinding,
        release: ProgramReleaseReference,
        http: Any,
        *,
        authenticated: bool = True,
    ) -> "HttpProgramReadTransport":
        """Hosted transport on the binding endpoint with a targeted token.

        Whether a token materializes is :class:`arete.http.HttpAuthClient`'s
        decision per the strategy order; pass ``authenticated=False`` to skip
        the target entirely (binding ``auth.required == false`` with no
        runtime auth configured — TS ``hostedAuthConfig``).
        """
        target = (
            AuthTokenTarget(
                kind="program-read-binding",
                target_id=binding.program_read_binding_id,
                release_hash=release.program_release_hash,
                scopes=READ_SCOPES,
            )
            if authenticated
            else None
        )
        return cls(endpoint=binding.endpoint, release=release, http=http, target=target)

    @classmethod
    def from_descriptor(
        cls,
        program_name: str,
        descriptor: ProgramReadDescriptor,
        http: Any,
        *,
        connect_http_url: Optional[str] = None,
    ) -> "HttpProgramReadTransport":
        """Validate a descriptor and build the matching transport."""
        validate_program_read_descriptor(program_name, descriptor)
        if descriptor.transport_kind == LOCAL_HTTP:
            if not _is_non_empty(connect_http_url):
                raise _coded_error(
                    f"Program '{program_name}' local HTTP reads require a "
                    "connect-time HTTP endpoint",
                    "INVALID_CONFIG",
                )
            return cls.local_http(connect_http_url, descriptor.release, http)
        binding = descriptor.binding
        assert binding is not None
        return cls.hosted(binding, descriptor.release, http)

    @property
    def endpoint(self) -> str:
        return self._endpoint

    @property
    def release(self) -> ProgramReleaseReference:
        return self._release

    @property
    def target(self) -> Optional[AuthTokenTarget]:
        return self._target

    async def read(self, request: ProgramReadRequest) -> Any:
        """Execute one read, returning the raw JSON wire value (``null`` → ``None``).

        Non-2xx responses raised by the HTTP layer are translated to
        :class:`arete.read.ReadRequestError` with the ``X-Error-Code`` header
        (or nested/top-level body code) preserved; refresh-replay-once for
        targeted tokens lives inside ``HttpAuthClient``.
        """
        path = request_path(self._release, request)
        url = _append_url(self._endpoint, path)
        if request.operation == "fetch_many":
            method = "POST"
            json_body: Any = {"addresses": list(request.addresses or ())}
        else:
            method = "GET"
            json_body = None
        try:
            return await self._http.request_json(
                method,
                url,
                json_body=json_body,
                target=self._target,
                scopes=READ_SCOPES,
            )
        except Exception as error:
            read_error = coerce_read_request_error(error, path, nested_body_code=True)
            if read_error is None:
                raise
            raise read_error from error
