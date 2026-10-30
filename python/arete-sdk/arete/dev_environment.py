"""The dev environment switch (``docs/internal/dev-environment-switch.md``).

One switch decides where an installed SDK reads and sends:

- **Production** (the default): the generated descriptors as they are. With
  the switch unset nothing here runs and no dev input is read.
- **Dev through the edge**: every surface goes to the loopback edge
  ``a4 dev up`` serves, with no credential. Requests name their installed
  identity in ``X-Arete-Dev-Identity``; the edge resolves it.
- **Dev direct**: servers, CI and agents with a secret key resolve their
  installed identities against the environment
  (``POST /api/dev/environments/{env}/resolve``) and mint environment tokens.

A surface the environment can't serve raises :class:`DevEnvironmentError`
with the wire code, the message, and ``fix`` plus every other detail in
``details``. No dev request ever falls back to production.
"""

from __future__ import annotations

import asyncio
import json
import os
import re
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Any, Callable, Dict, List, Mapping, Optional, Tuple
from urllib.parse import quote

from .errors import AreteError

ARETE_ENVIRONMENT_ENV = "ARETE_ENVIRONMENT"
ARETE_DEV_URL_ENV = "ARETE_DEV_URL"
ARETE_DEV_ENV_ENV = "ARETE_DEV_ENV"
DEV_SESSION_FILE = ".arete/dev/session.json"
DEV_EDGE_HEADER = "X-Arete-Dev-Edge"
DEV_IDENTITY_HEADER = "X-Arete-Dev-Identity"
DEV_EDGE_WELL_KNOWN_PATH = "/.well-known/arete-dev-edge"
DEFAULT_API_ORIGIN = "https://api.arete.run"
DEFAULT_SESSION_ENDPOINT = "https://api.arete.run/ws/sessions"

NOT_CONFIGURED = "devenv-not-configured"
EDGE_UNREACHABLE = "devenv-edge-unreachable"
STACK_NOT_LOADED = "devenv-stack-not-loaded"
PROGRAM_RELEASE_MISMATCH = "devenv-program-release-mismatch"
PROTOCOL_NOT_DEPLOYED = "devenv-protocol-not-deployed"
PROTOCOL_NOT_OWNED = "devenv-protocol-not-owned"
SURFACE_UNAVAILABLE = "devenv-surface-unavailable"
ENVIRONMENT_TOKEN_REQUIRED = "environment-token-required"
ENVIRONMENT_TOKEN_REFUSED = "environment-token-refused"
INVALID_CONFIG = "INVALID_CONFIG"

_NOT_CONFIGURED_FIX = (
    "Run the app with `a4 dev run -- <command>`, start `a4 dev up`, or set "
    "`ARETE_DEV_ENV` with a secret key."
)
_EDGE_UNREACHABLE_FIX = "Start `a4 dev up` in the project (it serves the edge)."


class DevEnvironmentError(AreteError):
    """A dev refusal. ``code`` is the wire code; ``details`` holds ``fix``
    and every other detail field of the answer."""

    def __init__(
        self,
        message: str,
        code: str,
        fix: Optional[str] = None,
        details: Optional[Mapping[str, Any]] = None,
        status: Optional[int] = None,
    ) -> None:
        merged: Dict[str, Any] = dict(details or {})
        if fix is not None:
            merged["fix"] = fix
        super().__init__(message, code, merged)
        #: HTTP status of the answer, when one was received.
        self.http_status = status

    @property
    def fix(self) -> Optional[str]:
        return self.details.get("fix")

    def __str__(self) -> str:
        text = f"[{self.code}] {self.message}"
        return f"{text}. {self.fix}" if self.fix else text


def is_dev_refusal_code(code: Any) -> bool:
    return isinstance(code, str) and (
        code.startswith("devenv-")
        or code in (ENVIRONMENT_TOKEN_REQUIRED, ENVIRONMENT_TOKEN_REFUSED)
    )


def not_configured(message: str) -> DevEnvironmentError:
    return DevEnvironmentError(message, NOT_CONFIGURED, _NOT_CONFIGURED_FIX)


def edge_unreachable(edge_url: str, detail: str) -> DevEnvironmentError:
    return DevEnvironmentError(
        f"{edge_url} does not answer as the `a4 dev up` edge: {detail}",
        EDGE_UNREACHABLE,
        _EDGE_UNREACHABLE_FIX,
    )


def surface_unavailable(message: str, fix: str) -> DevEnvironmentError:
    return DevEnvironmentError(message, SURFACE_UNAVAILABLE, fix)


def stack_not_loaded(stack: str) -> DevEnvironmentError:
    return DevEnvironmentError(
        f"Stack '{stack}' was not installed from the registry, so it has no release "
        "to resolve in a dev environment",
        STACK_NOT_LOADED,
        "Install the stack with `a4 install stack` and run `a4 dev up`.",
    )


def _invalid(message: str) -> AreteError:
    return AreteError(message, INVALID_CONFIG)


# ---------------------------------------------------------------------------
# The switch


def dev(url: Optional[str] = None, env: Optional[str] = None) -> Dict[str, str]:
    """The ``environment`` option for dev mode: ``dev()`` takes the target
    from the variables or the session file, ``dev(url=...)`` names an edge,
    ``dev(env=...)`` an environment (direct mode)."""
    option: Dict[str, str] = {"kind": "dev"}
    if url is not None:
        option["url"] = url
    if env is not None:
        option["env"] = env
    return option


@dataclass(frozen=True)
class SwitchResult:
    """What the switch decided: ``mode`` is ``production`` or ``dev``; in dev
    mode ``via`` is ``edge`` (``edge_url``) or ``direct`` (``environment``)."""

    mode: str = "production"
    via: Optional[str] = None
    edge_url: Optional[str] = None
    environment: Optional[str] = None

    @property
    def is_dev(self) -> bool:
        return self.mode == "dev"

    def to_dict(self) -> Dict[str, Any]:
        if self.mode == "production":
            return {"mode": "production", "devInputsRead": False}
        if self.via == "edge":
            return {"mode": "dev", "via": "edge", "edgeUrl": self.edge_url}
        return {"mode": "dev", "via": "direct", "environment": self.environment}


PRODUCTION = SwitchResult()


def _parse_option(option: Any) -> Optional[Tuple[Optional[str], Optional[str]]]:
    """``None`` for production, else ``(url, env)``."""
    if option == "production":
        return None
    if isinstance(option, Mapping):
        if option.get("kind") != "dev":
            raise _invalid("environment.kind must be 'dev' (or pass the string 'production')")
        url = env = None
        for name, value in option.items():
            if name == "kind":
                continue
            if name not in ("url", "env"):
                raise _invalid(
                    f"environment has an unknown member '{name}'; it takes kind, url and env"
                )
            if not isinstance(value, str):
                raise _invalid(f"environment.{name} must be a string")
            if name == "url":
                url = value
            else:
                env = value
        return url, env
    raise _invalid("environment must be 'production' or {'kind': 'dev', 'url'?, 'env'?}")


def resolve_switch(
    option: Any = None,
    *,
    read_var: Callable[[str], Optional[str]],
    session_edge_url: Callable[[], Optional[str]],
    has_secret_key: Callable[[], bool],
    browser: bool = False,
) -> SwitchResult:
    """Decide the switch (contract §2.3). Each lookup is only called when
    the answer needs it, so production reads no dev input."""
    if option is not None:
        parsed = _parse_option(option)
        if parsed is None:
            return PRODUCTION
        url, env = parsed
    else:
        if browser:
            return PRODUCTION
        value = read_var(ARETE_ENVIRONMENT_ENV)
        if value in (None, "", "production"):
            return PRODUCTION
        if value != "dev":
            raise _invalid(
                f"{ARETE_ENVIRONMENT_ENV} is '{value}'; it must be 'production' or 'dev'"
            )
        url = env = None

    if url is not None:
        return _edge(url, "environment.url")
    if env is not None:
        return _direct(env, "environment.env", browser, has_secret_key)
    if not browser:
        url = read_var(ARETE_DEV_URL_ENV)
        if url is not None:
            return _edge(url, ARETE_DEV_URL_ENV)
        env = read_var(ARETE_DEV_ENV_ENV)
        if env is not None:
            return _direct(env, ARETE_DEV_ENV_ENV, browser, has_secret_key)
        url = session_edge_url()
        if url is not None:
            return _edge(url, DEV_SESSION_FILE)
    raise not_configured("Dev mode is on, but no edge URL or environment is set")


def _edge(url: str, source: str) -> SwitchResult:
    normalized, reason = normalize_edge_url(url)
    if normalized is None:
        raise _invalid(
            f"{source} '{url}' is not an edge URL: {reason}. It must be http(s)://, a "
            "loopback host (127.0.0.1, [::1] or localhost) and an explicit port"
        )
    return SwitchResult(mode="dev", via="edge", edge_url=normalized)


def _direct(
    env: str, source: str, browser: bool, has_secret_key: Callable[[], bool]
) -> SwitchResult:
    if not is_environment_reference(env):
        raise _invalid(
            f"{source} '{env}' is not an environment id (denv_…) or name (a-z, 0-9 and -)"
        )
    if browser:
        raise not_configured("Direct mode needs a secret key and never runs in a browser")
    if not has_secret_key():
        raise not_configured(
            f"{source} names an environment, but no secret key was found to resolve it with"
        )
    return SwitchResult(mode="dev", via="direct", environment=env)


def session_edge_url_from(start: Path) -> Optional[str]:
    """``edgeUrl`` of the nearest ``.arete/dev/session.json`` at or above ``start``."""
    for directory in (start, *start.parents):
        candidate = directory / DEV_SESSION_FILE
        if candidate.is_file():
            try:
                value = json.loads(candidate.read_text())
            except (OSError, ValueError):
                return None
            url = value.get("edgeUrl") if isinstance(value, dict) else None
            return url if isinstance(url, str) and url else None
    return None


def has_secret_key(auth: Any) -> bool:
    if auth is None:
        return False
    if getattr(auth, "secret_key", None):
        return True
    from .auth import classify_api_key

    key = getattr(auth, "publishable_key", None)
    return bool(key) and classify_api_key(key) == "secret"


def resolve_switch_from_process(option: Any = None, auth: Any = None) -> SwitchResult:
    """The switch from the process: variables, the nearest session file and
    whether ``auth`` (already resolved, or a callable resolving it, called
    only when direct mode needs to know) holds a secret key."""
    return resolve_switch(
        option,
        read_var=lambda name: os.environ.get(name),
        session_edge_url=lambda: session_edge_url_from(Path.cwd()),
        has_secret_key=lambda: has_secret_key(auth() if callable(auth) else auth),
    )


_PORT = re.compile(r"^[0-9]{1,5}$")


def normalize_edge_url(url: str) -> Tuple[Optional[str], str]:
    """``(scheme://host:port, "")``, or ``(None, why it is not an edge URL)``."""
    if url.startswith("http://"):
        scheme, rest = "http", url[len("http://") :]
    elif url.startswith("https://"):
        scheme, rest = "https", url[len("https://") :]
    else:
        return None, "the scheme is not http or https"
    end = min([i for i in (rest.find("/"), rest.find("?"), rest.find("#")) if i >= 0] or [len(rest)])
    authority, remainder = rest[:end], rest[end:]
    if remainder not in ("", "/"):
        return None, "it has a path, query or fragment"
    if "@" in authority:
        return None, "it has user information"
    if authority.startswith("["):
        close = authority.find("]")
        if close < 0:
            return None, "the host is malformed"
        host, after = authority[: close + 1], authority[close + 1 :]
        port = after[1:] if after.startswith(":") else None
    elif ":" in authority:
        host, port = authority.rsplit(":", 1)
    else:
        host, port = authority, None
    host = host.lower()
    if host not in ("127.0.0.1", "[::1]", "localhost"):
        return None, "the host is not loopback"
    if port is None:
        return None, "it has no explicit port"
    if not _PORT.match(port) or not 0 < int(port) <= 65535:
        return None, "the port is malformed"
    return f"{scheme}://{host}:{int(port)}", ""


_ENV_ID = re.compile(r"^denv_[0-7][0-9a-hjkmnp-tv-z]{25}$")
_ENV_NAME = re.compile(r"^[a-z0-9-]{1,63}$")


def is_environment_reference(value: str) -> bool:
    return bool(_ENV_ID.match(value) or _ENV_NAME.match(value))


# ---------------------------------------------------------------------------
# Identities, resolve answers and refusals


def identity_header(request: Mapping[str, Any]) -> str:
    """The compact JSON an edge request carries in ``X-Arete-Dev-Identity``."""
    return json.dumps(request, separators=(",", ":"))


def gateway_identity(binding: Any) -> Dict[str, str]:
    return {"solanaGatewayBindingId": binding.solana_gateway_binding_id}


def program_read_identity(program_id: Optional[str], binding: Any, release: Any) -> Dict[str, str]:
    identity: Dict[str, str] = {}
    if program_id:
        identity["programId"] = program_id
    identity["programReadBindingId"] = binding.program_read_binding_id
    identity["programReleaseHash"] = release.program_release_hash
    return identity


def stack_identity(release: Any) -> Dict[str, str]:
    return {"stackManifestHash": release.stack_manifest_hash, "liveAlias": release.live_alias}


def protocol_identity(
    package: str, protocol_release_hash: str, deployment_id: Optional[str]
) -> Dict[str, str]:
    identity = {"package": package, "protocolReleaseHash": protocol_release_hash}
    if deployment_id:
        identity["deploymentId"] = deployment_id
    return identity


_LABELS = {
    "chain": "Chain reads",
    "transactions": "Transactions",
    "programReads": "Program Read",
    "stacks": "The stack",
    "protocols": "The protocol",
}


def surface_target(answer: Mapping[str, Any], surface: str, index: int = 0) -> Dict[str, Any]:
    """The target for ``surface`` (``chain``, ``transactions``,
    ``programReads``, ``stacks``, ``protocols``), or raise its refusal."""
    value = answer.get(surface)
    if isinstance(value, list):
        value = value[index] if index < len(value) else None
    label = _LABELS.get(surface, surface)
    if not isinstance(value, Mapping):
        raise surface_unavailable(
            f"{label} is not available in this environment: resolve gave no answer for it",
            "Update a4 and the SDK, then run `a4 dev up`.",
        )
    status = value.get("status")
    if status == "target":
        return dict(value)
    if status == "refused" and isinstance(value.get("code"), str):
        details = {
            name: item
            for name, item in value.items()
            if name not in ("status", "code", "message", "fix")
        }
        raise DevEnvironmentError(
            value.get("message") or value["code"], value["code"], value.get("fix"), details
        )
    raise surface_unavailable(f"{label} has an unreadable resolve answer", "Update a4 and the SDK.")


def target_str(target: Mapping[str, Any], name: str) -> str:
    value = target.get(name)
    if not isinstance(value, str) or not value:
        raise surface_unavailable(f"The resolve answer names no {name}", "Update a4 and the SDK.")
    return value


def stack_query_base(target: Mapping[str, Any], stack: str) -> str:
    """Where a stack target answers HTTP queries; a stream-only stack fails
    queries (not the stream) with ``devenv-surface-unavailable``."""
    url = target.get("httpUrl")
    if isinstance(url, str) and url:
        return url
    raise surface_unavailable(
        f"The environment's '{stack}' stack serves its stream only, not HTTP queries",
        "Read the stack through its views (WebSocket) in dev mode.",
    )


_ENVELOPE = ("schemaVersion", "error", "message", "code", "retryable", "fix", "requestId")


def parse_refusal(
    status: int, header_code: Optional[str], body: Any
) -> Optional[DevEnvironmentError]:
    """A dev refusal from a service answer: the API problem envelope plus
    ``fix``, or Program Read's ``{"error": {"code", "detail": {...}}}``.
    ``None`` unless the code is a dev refusal code."""
    if isinstance(body, (bytes, bytearray)):
        try:
            body = json.loads(body) if body else None
        except ValueError:
            body = None
    header_code = header_code.strip() if isinstance(header_code, str) and header_code.strip() else None
    if isinstance(body, Mapping) and isinstance(body.get("error"), Mapping):
        inner = body["error"]
        code = header_code or inner.get("code")
        detail = inner.get("detail") if isinstance(inner.get("detail"), Mapping) else {}
        message = detail.get("message") or inner.get("message") or code
        fix = detail.get("fix")
        details = {k: v for k, v in detail.items() if k not in ("message", "fix")}
    else:
        obj = body if isinstance(body, Mapping) else {}
        code = header_code or obj.get("code")
        message = obj.get("error") if isinstance(obj.get("error"), str) else obj.get("message")
        message = message or code
        fix = obj.get("fix")
        details = {k: v for k, v in obj.items() if k not in _ENVELOPE}
    if not is_dev_refusal_code(code):
        return None
    return DevEnvironmentError(str(message), code, fix, details, status)


def refusal_of_response(response: Any) -> Optional[DevEnvironmentError]:
    """The dev refusal an ``httpx.Response`` carries, if any."""
    if 200 <= response.status_code < 300:
        return None
    return parse_refusal(
        response.status_code, response.headers.get("X-Error-Code"), response.content
    )


# ---------------------------------------------------------------------------
# Edge


def _encode_segment(value: str) -> str:
    return quote(value, safe="!~*'():")


def edge_stack_query(edge_url: str, release: Any) -> Tuple[str, Dict[str, str]]:
    return (
        f"{edge_url.rstrip('/')}/stacks/{_encode_segment(release.live_alias)}",
        {"stackManifestHash": release.stack_manifest_hash},
    )


def edge_stack_websocket_url(edge_url: str, release: Any) -> str:
    base, query = edge_stack_query(edge_url, release)
    if base.startswith("https://"):
        base = "wss://" + base[len("https://") :]
    elif base.startswith("http://"):
        base = "ws://" + base[len("http://") :]
    return f"{base}?stackManifestHash={query['stackManifestHash']}"


def edge_protocol_api_url(edge_url: str) -> str:
    return f"{edge_url.rstrip('/')}/api"


def edge_protocol_segment(deployment_id: Optional[str], package: str) -> str:
    if deployment_id:
        return _encode_segment(deployment_id)
    return f"package:{_encode_segment(package)}"


@dataclass(frozen=True)
class EdgePolicy:
    """How every request of one surface goes through the edge."""

    edge_url: str
    identity: Optional[str] = None
    params: Mapping[str, str] = field(default_factory=dict)

    @classmethod
    def for_identity(cls, edge_url: str, request: Optional[Mapping[str, Any]]) -> "EdgePolicy":
        return cls(edge_url, identity_header(request) if request is not None else None)

    def headers(self) -> Dict[str, str]:
        return {DEV_IDENTITY_HEADER: self.identity} if self.identity else {}

    def check(self, response: Any) -> None:
        if not is_edge_answer(response.headers):
            raise edge_unreachable(self.edge_url, "the answer has no X-Arete-Dev-Edge header")

    def send_error(self, error: BaseException) -> DevEnvironmentError:
        return edge_unreachable(self.edge_url, str(error) or type(error).__name__)


def is_edge_answer(headers: Mapping[str, str]) -> bool:
    value = headers.get(DEV_EDGE_HEADER)
    return isinstance(value, str) and value.strip() == "1"


async def check_edge(http: Any, edge_url: str) -> None:
    """``GET <edge>/.well-known/arete-dev-edge``: ``200``, the marker and
    ``{"edge": 1}``, or :data:`EDGE_UNREACHABLE`."""
    import httpx

    url = f"{edge_url.rstrip('/')}{DEV_EDGE_WELL_KNOWN_PATH}"
    try:
        response = await http.get(url)
    except httpx.HTTPError as error:
        raise edge_unreachable(edge_url, str(error) or type(error).__name__) from error
    try:
        body = response.json()
    except ValueError:
        body = None
    if (
        response.status_code == 200
        and is_edge_answer(response.headers)
        and isinstance(body, Mapping)
        and body.get("edge") == 1
    ):
        return
    raise edge_unreachable(
        edge_url,
        f"{DEV_EDGE_WELL_KNOWN_PATH} answered {response.status_code} without the edge marker",
    )


# ---------------------------------------------------------------------------
# Direct


def api_origin() -> str:
    origin = (os.environ.get("ARETE_API_URL") or "").strip().rstrip("/")
    return origin or DEFAULT_API_ORIGIN


def resolve_url(origin: str, environment: str) -> str:
    return f"{origin.rstrip('/')}/api/dev/environments/{_encode_segment(environment)}/resolve"


class DirectResolver:
    """Calls resolve for one client with its secret key and keeps every
    answer for the life of the client."""

    def __init__(self, environment: str, auth: Any, http: Any) -> None:
        self.environment = environment
        self.url = resolve_url(api_origin(), environment)
        self._auth = auth
        self._http = http
        self._cache: Dict[str, Dict[str, Any]] = {}
        self._pending: Dict[str, "asyncio.Future[Dict[str, Any]]"] = {}

    @property
    def auth(self) -> Any:
        return self._auth

    @property
    def http(self) -> Any:
        return self._http

    async def resolve(self, request: Mapping[str, Any]) -> Dict[str, Any]:
        """The answer for ``request``, resolved once. Concurrent first uses
        of one identity share one request; other identities don't wait."""
        key = identity_header(request)
        if key in self._cache:
            return self._cache[key]
        pending = self._pending.get(key)
        if pending is None:
            pending = asyncio.ensure_future(self._fetch(request))
            self._pending[key] = pending

            # Settled whether or not a caller is still waiting: a success
            # is kept, a failure is forgotten so the next use asks again.
            def settle(task: "asyncio.Future[Dict[str, Any]]", key: str = key) -> None:
                if self._pending.get(key) is task:
                    del self._pending[key]
                if not task.cancelled() and task.exception() is None:
                    self._cache.setdefault(key, task.result())

            pending.add_done_callback(settle)
        return await asyncio.shield(pending)

    async def _fetch(self, request: Mapping[str, Any]) -> Dict[str, Any]:
        from .auth import api_key_for_endpoint

        secret = api_key_for_endpoint(self._auth, self.url)
        if not secret:
            raise not_configured("Direct mode has no secret key to resolve with")
        response = await self._http.post(
            self.url,
            json=dict(request),
            headers={"Authorization": f"Bearer {secret}"},
        )
        if not 200 <= response.status_code < 300:
            refusal = refusal_of_response(response)
            if refusal is not None:
                raise refusal
            from .http import _decode_json_body, _new_http_request_error

            raise _new_http_request_error(
                response.status_code, self.url, _decode_json_body(response.content)
            )
        return response.json()

    def prime(self, request: Mapping[str, Any], answer: Dict[str, Any]) -> None:
        """Keep ``answer`` as the answer for ``request`` (a part of a larger
        answer already received)."""
        self._cache.setdefault(identity_header(request), answer)

    def token_auth(self, session_endpoint: str) -> Any:
        """Secret-key auth minting environment tokens at ``session_endpoint``."""
        return replace(
            self._auth,
            token=None,
            get_token=None,
            token_endpoint=session_endpoint,
            dev_environment=self.environment,
        )


@dataclass
class DevMode:
    """A client's dev mode, carried to every transport it builds."""

    switch: SwitchResult
    resolver: Optional[DirectResolver] = None

    @property
    def edge_url(self) -> Optional[str]:
        return self.switch.edge_url

    @classmethod
    def of(cls, switch: SwitchResult, auth: Any, http: Any) -> Optional["DevMode"]:
        if not switch.is_dev:
            return None
        if switch.via == "direct":
            if not has_secret_key(auth):
                raise not_configured("Direct mode needs a secret key")
            return cls(switch, DirectResolver(switch.environment or "", auth, http))
        return cls(switch)


__all__: List[str] = [
    "ARETE_DEV_ENV_ENV",
    "ARETE_DEV_URL_ENV",
    "ARETE_ENVIRONMENT_ENV",
    "DevEnvironmentError",
    "SwitchResult",
    "dev",
    "resolve_switch",
    "resolve_switch_from_process",
    "parse_refusal",
]
