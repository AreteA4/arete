"""Shared HTTP auth + fetch machinery.

Mirror of the HTTP-token half of ``typescript/core/src/connection.ts`` and Rust
``arete_sdk::http``. Every HTTP surface (chain, transactions, program reads,
gateway) goes through :class:`HttpAuthClient` so token strategy, targeted
tokens, refresh-replay-once, and the predispatch-marker gate live in one place.

The predispatch marker is a **response** header. The server stamps every
transaction response with ``X-Arete-Upstream-Attempted: true|false``
(``rust/arete-server/src/http/transactions.rs`` ``transaction_response`` and
``http_health.rs``) and exposes it to browsers via
``Access-Control-Expose-Headers``; ``Access-Control-Allow-Headers`` is only
``Authorization, Content-Type``, so it is not accepted as a *request* header.
Clients only read it: ``false`` is the server's proof that it never dispatched
upstream, which is what makes replaying a ``send`` safe. TS
(``client.ts:843``, ``solana-gateway.ts:182``) and Rust
(``arete-a4-sdk/src/http.rs:909``) do exactly that and send nothing.
"""

from __future__ import annotations

import asyncio
import inspect
import json
import time
from collections import OrderedDict
from dataclasses import dataclass, field
from typing import Any, Dict, Mapping, Optional, Sequence, Set, Tuple

import httpx

from arete.auth import (
    TOKEN_REFRESH_BUFFER_SECONDS,
    AuthConfig,
    AuthErrorCode,
    AuthToken,
    build_token_endpoint_request_body,
    parse_jwt_expiry,
    request_token_from_endpoint,
    resolve_token_endpoint,
    should_refresh_token,
)
from arete.errors import AreteError, AuthError

try:  # Seam: arete.errors.HttpRequestError is provided by the core error module.
    from arete.errors import HttpRequestError
except ImportError:  # pragma: no cover - fallback until the core module lands

    class HttpRequestError(AreteError):  # type: ignore[no-redef]
        """HTTP request failure carrying the structured wire error body."""

        def __init__(
            self,
            message: str,
            *,
            code: Optional[str] = None,
            retryable: bool = False,
            retry_after: Optional[float] = None,
            suggested_action: Optional[str] = None,
            docs_url: Optional[str] = None,
            status: Optional[int] = None,
            body: Any = None,
        ) -> None:
            super().__init__(message)
            self.message = message
            self.code = code
            self.retryable = retryable
            self.retry_after = retry_after
            self.suggested_action = suggested_action
            self.docs_url = docs_url
            self.status = status
            self.body = body


UPSTREAM_ATTEMPTED_HEADER = "X-Arete-Upstream-Attempted"
ERROR_CODE_HEADER = "X-Error-Code"
MAX_HTTP_AUTH_TOKEN_STATES = 32
DEFAULT_READ_SCOPE = "read"

PROGRAM_READ_BINDING_KIND = "program-read-binding"
SOLANA_GATEWAY_BINDING_KIND = "solana-gateway-binding"
_TARGET_KINDS = (PROGRAM_READ_BINDING_KIND, SOLANA_GATEWAY_BINDING_KIND)


def normalize_scopes(scopes: Optional[Sequence[str]]) -> Tuple[str, ...]:
    """Dedupe + sort scopes: the canonical order used for request bodies,
    cache identities, and coverage checks."""
    return tuple(sorted(set(scopes or ())))


def _is_send_scope(scope: str) -> bool:
    return scope == "send" or scope.endswith(":send")


@dataclass(frozen=True)
class AuthTokenTarget:
    """Targeted-token request key.

    ``kind`` is ``"program-read-binding"`` or ``"solana-gateway-binding"``;
    ``target_id`` is the ``prb_…`` / ``sgb_…`` binding id; ``release_hash``
    applies to program-read targets only.
    """

    kind: str
    target_id: str
    release_hash: Optional[str] = None
    scopes: tuple = field(default_factory=tuple)  # sorted scope strings

    def _validate(self) -> None:
        complete = (
            self.kind in _TARGET_KINDS
            and bool(self.target_id)
            and (
                (self.kind == PROGRAM_READ_BINDING_KIND and self.release_hash)
                or (self.kind == SOLANA_GATEWAY_BINDING_KIND and self.release_hash is None)
            )
        )
        if not complete:
            raise AreteError(
                "Targeted authentication requires a complete supported target identity"
            )


def _targeted_identity(target: AuthTokenTarget, sorted_scopes: Sequence[str]) -> str:
    # Mirror of the TS identity: JSON [kind, id, releaseHash ?? null, scopes].
    return json.dumps(
        [target.kind, target.target_id, target.release_hash, list(sorted_scopes)],
        separators=(",", ":"),
    )


def _now() -> int:
    return int(time.time())


def _is_expiring(expires_at: Optional[int]) -> bool:
    if not expires_at:
        return False
    return _now() >= expires_at - TOKEN_REFRESH_BUFFER_SECONDS


@dataclass
class _CachedToken:
    token: str
    expires_at: Optional[int] = None


@dataclass
class _SharedTokenState:
    token: Optional[str] = None
    expires_at: Optional[int] = None
    scopes: Set[str] = field(default_factory=set)
    requested_scopes: Set[str] = field(default_factory=set)

    def valid_token_covering(self, required: Sequence[str]) -> Optional[str]:
        if self.token is None or _is_expiring(self.expires_at):
            return None
        if all(scope in self.scopes for scope in required):
            return self.token
        return None

    def clear(self) -> None:
        # Requested scopes survive a clear (TS clearTokenState).
        self.token = None
        self.expires_at = None
        self.scopes.clear()


def _parse_wire_error_code(raw: Optional[str]) -> Optional[AuthErrorCode]:
    if not raw:
        return None
    return AuthErrorCode.from_wire(raw.strip().replace("_", "-"))


def _decode_json_body(content: bytes) -> Any:
    if not content:
        return None
    try:
        return json.loads(content)
    except (ValueError, UnicodeDecodeError):
        return None


def _new_http_request_error(status: int, url: str, body: Any) -> HttpRequestError:
    parsed = body if isinstance(body, dict) else {}
    code = parsed.get("code") if isinstance(parsed.get("code"), str) else None
    message = None
    for key in ("message", "error"):
        if isinstance(parsed.get(key), str) and parsed[key]:
            message = parsed[key]
            break
    if message is None:
        message = f"HTTP request to '{url}' failed ({status})"
    fields: Dict[str, Any] = {
        "code": code,
        "retryable": parsed.get("retryable") is True,
        "retry_after": parsed.get("retry_after", parsed.get("retryAfter")),
        "suggested_action": parsed.get("suggested_action", parsed.get("suggestedAction")),
        "docs_url": parsed.get("docs_url", parsed.get("docsUrl")),
    }
    # Defensive construction: the error class is owned by arete.errors and its
    # exact __init__ may differ; guarantee the attributes either way.
    try:
        error = HttpRequestError(message, status=status, body=body, **fields)
    except TypeError:
        try:
            error = HttpRequestError(message, **fields)
        except TypeError:
            error = HttpRequestError(message)
    for name, value in {"status": status, "body": body, "message": message, **fields}.items():
        if getattr(error, name, None) in (None, False) and value is not None:
            setattr(error, name, value)
    return error


class HttpAuthClient:
    """Authed JSON fetch with token strategy + targeted-token LRU (cap 32).

    Strategy order: explicit token > provider > token_endpoint > hosted
    default. On 401: refresh the token and replay the request exactly once —
    except that ``send``-scoped requests replay only when the failed response
    carries the ``X-Arete-Upstream-Attempted: false`` predispatch marker (see
    sdk-core-api.md §2.5). Nothing is added to the outbound request; the
    marker is the server's response-side proof of non-dispatch.
    """

    def __init__(
        self,
        *,
        auth: Optional[AuthConfig] = None,
        websocket_url: Optional[str] = None,
        http_client: Optional[Any] = None,  # httpx.AsyncClient, injectable for tests
    ) -> None:
        self._auth = auth
        self._websocket_url = websocket_url
        self._owns_http_client = http_client is None
        self._http: httpx.AsyncClient = http_client or httpx.AsyncClient()
        self._shared = _SharedTokenState()
        self._targeted: "OrderedDict[str, _CachedToken]" = OrderedDict()
        self._mint_lock = asyncio.Lock()

    async def request_json(
        self,
        method: str,
        url: str,
        *,
        json_body: Any = None,
        params: Optional[Mapping[str, Any]] = None,
        headers: Optional[Mapping[str, str]] = None,
        target: Optional[AuthTokenTarget] = None,
        scopes: Optional[Sequence[str]] = None,
    ) -> Any:
        """Perform an authenticated request, returning decoded JSON.

        ``null`` bodies decode to ``None``. Non-2xx responses raise
        :class:`arete.errors.HttpRequestError` with the structured error body
        (``code``, ``message``, ``retryable``, ``retry_after``,
        ``suggested_action``, ``docs_url``) attached when present.
        """
        effective_scopes = self._effective_scopes(target, scopes)
        # `send` scope only gates the replay below; nothing is added to the
        # outbound request (the marker is a response header — see module docs).
        send_scoped = any(_is_send_scope(scope) for scope in effective_scopes)

        request_headers: Dict[str, str] = dict(headers or {})

        response = await self._attempt(
            method, url, json_body, params, request_headers, target, effective_scopes, False
        )
        if 200 <= response.status_code < 300:
            return _decode_json_body(response.content)

        code = _parse_wire_error_code(response.headers.get(ERROR_CODE_HEADER))
        body = _decode_json_body(response.content)
        if code is None and isinstance(body, dict) and isinstance(body.get("code"), str):
            code = _parse_wire_error_code(body["code"])
        refresh_worthy = (
            should_refresh_token(code)
            if code is not None
            else response.status_code == 401
        )
        # TS client.ts:843 / Rust http.rs:909 — the server's proof that the
        # request never reached upstream, read off the response.
        explicitly_not_dispatched = (
            response.headers.get(UPSTREAM_ATTEMPTED_HEADER) == "false"
        )
        if refresh_worthy and (not send_scoped or explicitly_not_dispatched):
            self._invalidate(target, effective_scopes)
            response = await self._attempt(
                method, url, json_body, params, request_headers, target, effective_scopes, True
            )
            if 200 <= response.status_code < 300:
                return _decode_json_body(response.content)
            body = _decode_json_body(response.content)

        raise _new_http_request_error(response.status_code, url, body)

    async def get_token(
        self,
        *,
        target: Optional[AuthTokenTarget] = None,
        scopes: Optional[Sequence[str]] = None,
        force_refresh: bool = False,
    ) -> Optional[str]:
        """Resolve a bearer token per the strategy order (None when unauthenticated)."""
        effective_scopes = self._effective_scopes(target, scopes)
        if target is not None:
            target._validate()
            return await self._targeted_token(target, effective_scopes, force_refresh)
        return await self._shared_token(effective_scopes, force_refresh)

    async def aclose(self) -> None:
        if self._owns_http_client:
            await self._http.aclose()

    # ------------------------------------------------------------------
    # internals

    def _effective_scopes(
        self, target: Optional[AuthTokenTarget], scopes: Optional[Sequence[str]]
    ) -> Tuple[str, ...]:
        combined = list(target.scopes if target is not None else ()) + list(scopes or ())
        return normalize_scopes(combined) or (DEFAULT_READ_SCOPE,)

    def _invalidate(
        self, target: Optional[AuthTokenTarget], sorted_scopes: Sequence[str]
    ) -> None:
        if target is not None:
            self._targeted.pop(_targeted_identity(target, sorted_scopes), None)
        else:
            self._shared.clear()

    def clear_tokens(self) -> None:
        """Clear every cached token (untargeted and targeted)."""
        self._shared.clear()
        self._targeted.clear()

    def _token_endpoint(self) -> Optional[str]:
        return resolve_token_endpoint(self._auth, self._websocket_url)

    async def _mint(
        self, target: Optional[AuthTokenTarget], scopes: Sequence[str]
    ) -> Optional[AuthToken]:
        auth = self._auth
        if auth is not None and auth.token:
            return AuthToken(token=auth.token)
        if auth is not None and auth.get_token:
            provider = auth.get_token
            try:
                result = await self._call_provider(provider, target, scopes)
            except AuthError:
                raise
            except Exception as e:
                raise AuthError(
                    f"Failed to get authentication token: {e}",
                    AuthErrorCode.AUTH_REQUIRED,
                ) from e
            return self._coerce_token(result)
        endpoint = self._token_endpoint()
        if endpoint is not None:
            body = build_token_endpoint_request_body(
                websocket_url=self._websocket_url,
                scopes=list(scopes),
                target_kind=target.kind if target is not None else None,
                target_id=target.target_id if target is not None else None,
                program_release_hash=target.release_hash if target is not None else None,
            )
            return await request_token_from_endpoint(self._http, endpoint, auth, body)
        return None

    async def _call_provider(
        self,
        provider: Any,
        target: Optional[AuthTokenTarget],
        scopes: Sequence[str],
    ) -> Any:
        """Call a token provider; request-aware providers (accepting one
        positional argument) receive the TS-shaped ``AuthTokenRequest``."""
        try:
            accepts_request = bool(
                [
                    p
                    for p in inspect.signature(provider).parameters.values()
                    if p.kind
                    in (p.POSITIONAL_ONLY, p.POSITIONAL_OR_KEYWORD, p.VAR_POSITIONAL)
                ]
            )
        except (TypeError, ValueError):
            accepts_request = False
        if accepts_request:
            request: Dict[str, Any] = {"scopes": list(scopes)}
            if target is not None:
                request["targetKind"] = target.kind
                request["targetId"] = target.target_id
                if target.release_hash is not None:
                    request["programReleaseHash"] = target.release_hash
            return await provider(request)
        return await provider()

    @staticmethod
    def _coerce_token(result: Any) -> AuthToken:
        if isinstance(result, AuthToken):
            return result
        if isinstance(result, str):
            return AuthToken(token=result)
        if isinstance(result, Mapping):
            expires_at = result.get("expires_at") or result.get("expiresAt")
            scopes = result.get("scopes")
            return AuthToken(
                token=result.get("token") or "",
                expires_at=int(expires_at) if expires_at else None,
                scopes=list(scopes) if isinstance(scopes, (list, tuple)) else None,
            )
        raise AuthError(
            "Authentication provider returned an unsupported token value",
            AuthErrorCode.TOKEN_INVALID_FORMAT,
        )

    @staticmethod
    def _finalize(
        minted: AuthToken, requested_scopes: Sequence[str]
    ) -> Tuple[str, Optional[int], Set[str], bool]:
        token = (minted.token or "").strip()
        if not token:
            raise AuthError(
                "Authentication provider returned an empty token",
                AuthErrorCode.TOKEN_INVALID_FORMAT,
            )
        explicit = minted.scopes is not None
        granted = set(minted.scopes if explicit else requested_scopes)
        expires_at = minted.expires_at or parse_jwt_expiry(token)
        if _is_expiring(expires_at):
            raise AuthError(
                "Authentication token is expired", AuthErrorCode.TOKEN_EXPIRED
            )
        return token, expires_at, granted, explicit

    @staticmethod
    def _scope_coverage_error(required: Sequence[str]) -> AuthError:
        return AuthError(
            "Authentication token was not granted required scopes: "
            + ", ".join(required),
            AuthErrorCode.AUTH_REQUIRED,
        )

    async def _shared_token(
        self, required_scopes: Sequence[str], force_refresh: bool
    ) -> Optional[str]:
        self._shared.requested_scopes.update(required_scopes)
        if not force_refresh:
            cached = self._shared.valid_token_covering(required_scopes)
            if cached is not None:
                return cached

        async with self._mint_lock:
            if not force_refresh:
                cached = self._shared.valid_token_covering(required_scopes)
                if cached is not None:
                    return cached
            # Refreshes mint the union of granted and requested scopes.
            fetch_scopes = tuple(
                sorted(self._shared.scopes | self._shared.requested_scopes)
            )
            minted = await self._mint(None, fetch_scopes)
            if minted is None:
                return None
            token, expires_at, granted, _ = self._finalize(minted, fetch_scopes)
            self._shared.token = token
            self._shared.expires_at = expires_at
            self._shared.scopes = granted
            if not all(scope in granted for scope in required_scopes):
                raise self._scope_coverage_error(required_scopes)
            return token

    async def _targeted_token(
        self,
        target: AuthTokenTarget,
        required_scopes: Sequence[str],
        force_refresh: bool,
    ) -> Optional[str]:
        identity = _targeted_identity(target, required_scopes)

        def cached_token() -> Optional[str]:
            state = self._targeted.get(identity)
            if state is None or _is_expiring(state.expires_at):
                return None
            self._targeted.move_to_end(identity)  # LRU touch
            return state.token

        if not force_refresh:
            token = cached_token()
            if token is not None:
                return token

        async with self._mint_lock:
            if not force_refresh:
                token = cached_token()
                if token is not None:
                    return token
            minted = await self._mint(target, required_scopes)
            if minted is None:
                return None
            token, expires_at, granted, explicit = self._finalize(
                minted, required_scopes
            )
            if explicit and not all(scope in granted for scope in required_scopes):
                raise self._scope_coverage_error(required_scopes)
            self._targeted.pop(identity, None)
            self._targeted[identity] = _CachedToken(token=token, expires_at=expires_at)
            while len(self._targeted) > MAX_HTTP_AUTH_TOKEN_STATES:
                self._targeted.popitem(last=False)
            return token

    async def _attempt(
        self,
        method: str,
        url: str,
        json_body: Any,
        params: Optional[Mapping[str, Any]],
        headers: Mapping[str, str],
        target: Optional[AuthTokenTarget],
        scopes: Sequence[str],
        force_refresh: bool,
    ) -> httpx.Response:
        token = await self.get_token(
            target=target, scopes=scopes, force_refresh=force_refresh
        )
        request_headers = dict(headers)
        if token:
            request_headers["Authorization"] = f"Bearer {token}"
        try:
            return await self._http.request(
                method.upper(),
                url,
                json=json_body,
                params=dict(params) if params else None,
                headers=request_headers,
            )
        except httpx.HTTPError as e:
            raise _new_http_request_error(
                0, url, {"message": f"HTTP request to '{url}' failed: {e}"}
            ) from e


def derive_http_endpoint(websocket_url: str) -> str:
    """Derive the HTTP base endpoint from a ws(s):// stack URL (TS deriveHttpEndpoint)."""
    from urllib.parse import urlsplit, urlunsplit

    def fallback(value: str) -> str:
        lower = value.lower()
        if lower.startswith("wss:"):
            return "https:" + value[4:]
        if lower.startswith("ws:"):
            return "http:" + value[3:]
        return value

    try:
        parts = urlsplit(websocket_url)
        scheme = parts.scheme.lower()
        if not scheme or not parts.netloc:
            return fallback(websocket_url)
        if scheme == "ws":
            parts = parts._replace(scheme="http")
        elif scheme == "wss":
            parts = parts._replace(scheme="https")
        rendered = urlunsplit(parts)
    except ValueError:
        return fallback(websocket_url)
    return rendered[:-1] if rendered.endswith("/") else rendered
