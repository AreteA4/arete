"""Authentication support for Arete Python SDK."""

from __future__ import annotations

import asyncio
import base64
import json
import logging
import os
import time
from dataclasses import dataclass, field, replace
from enum import Enum
from typing import (
    TYPE_CHECKING,
    Any,
    Callable,
    Coroutine,
    Dict,
    Iterable,
    List,
    Mapping,
    Optional,
    Sequence,
)

import httpx

from arete._a4_profile import ProfileKey
from arete._a4_profile import read_profile_key as read_profile_key_default
from arete.errors import AreteError, AuthError

if TYPE_CHECKING:  # arete.stack imports this module through arete.gateway.
    from arete.stack import StackRelease

logger = logging.getLogger(__name__)

TOKEN_REFRESH_BUFFER_SECONDS = 60
MIN_REFRESH_DELAY_SECONDS = 1
DEFAULT_QUERY_PARAMETER = "hs_token"
DEFAULT_HOSTED_TOKEN_ENDPOINT = "https://api.arete.run/ws/sessions"
ARETE_API_KEY_ENV = "ARETE_API_KEY"
"""Environment variable that supplies ``secret_key`` when no auth is set."""

_PUBLISHABLE_KEY_PREFIXES = ("a4_pk_", "hspk_")
_SECRET_KEY_PREFIXES = ("a4_sk_", "a4_ak_", "hsk_")
_CREATE_PUBLISHABLE_HINT = (
    "create one with `a4 auth keys create-publishable --origin <scheme://host[:port]>`"
)
_warned: set = set()


def classify_api_key(key: str) -> str:
    """Return ``"publishable"``, ``"secret"`` (secret or agent key) or
    ``"unknown"`` from an API key's prefix. Unknown keys are not validated."""
    trimmed = key.strip()
    if trimmed.startswith(_PUBLISHABLE_KEY_PREFIXES):
        return "publishable"
    if trimmed.startswith(_SECRET_KEY_PREFIXES):
        return "secret"
    return "unknown"


def _warn_once(warning_id: str, message: str) -> None:
    if warning_id in _warned:
        return
    _warned.add(warning_id)
    logger.warning(message)
HOSTED_WEBSOCKET_SUFFIX = ".stack.arete.run"
HOSTED_WEBSOCKET_SUFFIXES_ENV = "ARETE_HOSTED_WEBSOCKET_SUFFIXES"

_extra_hosted_suffixes: Optional[List[str]] = None


def _normalise_hosted_suffix(entry: str) -> Optional[str]:
    trimmed = entry.strip().rstrip(".").lower()
    if not trimmed:
        return None
    return trimmed if trimmed.startswith(".") else f".{trimmed}"


def set_hosted_websocket_suffixes(suffixes: Optional[Iterable[str]]) -> None:
    """Replace the extra hosted suffixes, or pass ``None`` to read the env again.

    A deployment can be served on a hostname outside the default suffix. A
    client that does not recognise it mints no session, connects without a
    token, and the server answers 401 - which reads as a credential problem
    rather than an unknown hostname.
    """
    global _extra_hosted_suffixes
    if suffixes is None:
        _extra_hosted_suffixes = None
        return
    normalised = []
    for entry in suffixes:
        suffix = _normalise_hosted_suffix(entry)
        if suffix is not None and suffix not in normalised:
            normalised.append(suffix)
    _extra_hosted_suffixes = normalised


def hosted_websocket_suffixes() -> List[str]:
    """The default suffix plus any configured extras."""
    extras = _extra_hosted_suffixes
    if extras is None:
        extras = []
        for entry in os.environ.get(HOSTED_WEBSOCKET_SUFFIXES_ENV, "").split(","):
            suffix = _normalise_hosted_suffix(entry)
            if suffix is not None and suffix not in extras:
                extras.append(suffix)
    return [HOSTED_WEBSOCKET_SUFFIX, *extras]


def is_hosted_websocket_host(host: str) -> bool:
    """True when ``host`` is served by hosted Arete under any known suffix."""
    normalised = host.rstrip(".").lower()
    return any(normalised.endswith(suffix) for suffix in hosted_websocket_suffixes())


class TokenTransport(Enum):
    """How the websocket token is sent to the server."""

    QUERY = "query"
    BEARER = "bearer"


@dataclass
class AuthToken:
    """Represents an authentication token with optional expiry.

    ``scopes`` optionally records the scopes the token was granted (targeted
    tokens); ``None`` means "assume the requested scopes were granted".
    """

    token: str
    expires_at: Optional[int] = None  # Unix timestamp in seconds
    scopes: Optional[List[str]] = None

    def is_expiring(self, buffer_seconds: int = TOKEN_REFRESH_BUFFER_SECONDS) -> bool:
        """Check if token is expired or about to expire."""
        if self.expires_at is None:
            return False
        return time.time() >= self.expires_at - buffer_seconds


class AuthErrorCode(Enum):
    """Machine-readable error codes for authentication failures."""

    # Token validation errors
    TOKEN_MISSING = "token_missing"
    TOKEN_EXPIRED = "token_expired"
    TOKEN_INVALID_SIGNATURE = "token_invalid_signature"
    TOKEN_INVALID_FORMAT = "token_invalid_format"
    TOKEN_INVALID_ISSUER = "token_invalid_issuer"
    TOKEN_INVALID_AUDIENCE = "token_invalid_audience"
    TOKEN_MISSING_CLAIM = "token_missing_claim"
    TOKEN_KEY_NOT_FOUND = "token_key_not_found"
    # Origin and security errors
    ORIGIN_MISMATCH = "origin_mismatch"
    ORIGIN_REQUIRED = "origin_required"
    ORIGIN_NOT_ALLOWED = "origin_not_allowed"
    AUTH_REQUIRED = "auth_required"
    MISSING_AUTHORIZATION_HEADER = "missing_authorization_header"
    INVALID_AUTHORIZATION_FORMAT = "invalid_authorization_format"
    INVALID_API_KEY = "invalid_api_key"
    EXPIRED_API_KEY = "expired_api_key"
    USER_NOT_FOUND = "user_not_found"
    SECRET_KEY_REQUIRED = "secret_key_required"
    DEPLOYMENT_ACCESS_DENIED = "deployment_access_denied"
    # Rate limiting and quota errors
    RATE_LIMIT_EXCEEDED = "rate_limit_exceeded"
    WEBSOCKET_SESSION_RATE_LIMIT_EXCEEDED = "websocket_session_rate_limit_exceeded"
    CONNECTION_LIMIT_EXCEEDED = "connection_limit_exceeded"
    SUBSCRIPTION_LIMIT_EXCEEDED = "subscription_limit_exceeded"
    SNAPSHOT_LIMIT_EXCEEDED = "snapshot_limit_exceeded"
    EGRESS_LIMIT_EXCEEDED = "egress_limit_exceeded"
    QUOTA_EXCEEDED = "quota_exceeded"
    # Static token errors
    INVALID_STATIC_TOKEN = "invalid_static_token"
    # Stack version errors: the session endpoint no longer serves, or never
    # served, the stack version the client was generated for. Terminal.
    STACK_VERSION_RETIRED = "stack_version_retired"
    STACK_VERSION_UNKNOWN = "stack_version_unknown"
    # Server errors
    INTERNAL_ERROR = "internal_error"

    @classmethod
    def from_wire(cls, error_code: str) -> "AuthErrorCode":
        """Parse a kebab-case or snake_case error code string.

        A code this SDK does not know maps to ``INTERNAL_ERROR``; use
        :meth:`from_wire_known` to tell the two apart.
        """
        known = cls.from_wire_known(error_code)
        return known if known is not None else cls.INTERNAL_ERROR

    @classmethod
    def from_wire_known(cls, error_code: str) -> Optional["AuthErrorCode"]:
        """Like :meth:`from_wire`, but ``None`` for a code this SDK does not know."""
        code_map = {
            "token-missing": cls.TOKEN_MISSING,
            "token-expired": cls.TOKEN_EXPIRED,
            "token-invalid-signature": cls.TOKEN_INVALID_SIGNATURE,
            "token-invalid-format": cls.TOKEN_INVALID_FORMAT,
            "token-invalid-issuer": cls.TOKEN_INVALID_ISSUER,
            "token-invalid-audience": cls.TOKEN_INVALID_AUDIENCE,
            "token-missing-claim": cls.TOKEN_MISSING_CLAIM,
            "token-key-not-found": cls.TOKEN_KEY_NOT_FOUND,
            "origin-mismatch": cls.ORIGIN_MISMATCH,
            "origin-required": cls.ORIGIN_REQUIRED,
            "origin-not-allowed": cls.ORIGIN_NOT_ALLOWED,
            "rate-limit-exceeded": cls.RATE_LIMIT_EXCEEDED,
            "websocket-session-rate-limit-exceeded": cls.WEBSOCKET_SESSION_RATE_LIMIT_EXCEEDED,
            "connection-limit-exceeded": cls.CONNECTION_LIMIT_EXCEEDED,
            "subscription-limit-exceeded": cls.SUBSCRIPTION_LIMIT_EXCEEDED,
            "snapshot-limit-exceeded": cls.SNAPSHOT_LIMIT_EXCEEDED,
            "egress-limit-exceeded": cls.EGRESS_LIMIT_EXCEEDED,
            "invalid-static-token": cls.INVALID_STATIC_TOKEN,
            "internal-error": cls.INTERNAL_ERROR,
            "auth-required": cls.AUTH_REQUIRED,
            "missing-authorization-header": cls.MISSING_AUTHORIZATION_HEADER,
            "invalid-authorization-format": cls.INVALID_AUTHORIZATION_FORMAT,
            "invalid-api-key": cls.INVALID_API_KEY,
            "expired-api-key": cls.EXPIRED_API_KEY,
            "user-not-found": cls.USER_NOT_FOUND,
            "secret-key-required": cls.SECRET_KEY_REQUIRED,
            "deployment-access-denied": cls.DEPLOYMENT_ACCESS_DENIED,
            "quota-exceeded": cls.QUOTA_EXCEEDED,
            "stack-version-retired": cls.STACK_VERSION_RETIRED,
            "stack-version-unknown": cls.STACK_VERSION_UNKNOWN,
            # Also support snake_case variants
            "token_missing": cls.TOKEN_MISSING,
            "token_expired": cls.TOKEN_EXPIRED,
            "token_invalid_signature": cls.TOKEN_INVALID_SIGNATURE,
            "token_invalid_format": cls.TOKEN_INVALID_FORMAT,
        }
        return code_map.get(error_code.strip().lower())


def should_refresh_token(error_code: AuthErrorCode) -> bool:
    """Determine if the error indicates the client should fetch a new token."""
    refresh_codes = {
        AuthErrorCode.TOKEN_EXPIRED,
        AuthErrorCode.TOKEN_INVALID_SIGNATURE,
        AuthErrorCode.TOKEN_INVALID_FORMAT,
        AuthErrorCode.TOKEN_INVALID_ISSUER,
        AuthErrorCode.TOKEN_INVALID_AUDIENCE,
        AuthErrorCode.TOKEN_KEY_NOT_FOUND,
    }
    return error_code in refresh_codes


def is_stack_version_refusal(error_code: Optional[AuthErrorCode]) -> bool:
    """The session endpoint refused the client's stack version: no retry,
    token refresh or reconnect can change the answer."""
    return error_code in (
        AuthErrorCode.STACK_VERSION_RETIRED,
        AuthErrorCode.STACK_VERSION_UNKNOWN,
    )


def should_retry_error(error_code: AuthErrorCode) -> bool:
    """Determine if the error indicates the client should retry the same request."""
    retry_codes = {
        AuthErrorCode.RATE_LIMIT_EXCEEDED,
        AuthErrorCode.WEBSOCKET_SESSION_RATE_LIMIT_EXCEEDED,
        AuthErrorCode.INTERNAL_ERROR,
    }
    return error_code in retry_codes


# Type alias for token provider function
TokenProvider = Callable[[], Coroutine[Any, Any, AuthToken]]


@dataclass
class AuthConfig:
    """Configuration for Arete authentication.

    Supports multiple authentication strategies:
    1. Static token - for server-side use with pre-minted tokens
    2. Token provider function - custom async function that returns tokens
    3. Secret key - an agent key (``a4_ak_...``) or secret key
       (``a4_sk_...``) for servers, agents and local scripts
    4. Publishable key - an origin-bound key (``a4_pk_...``) for code
       shipped to a browser
    5. Custom token endpoint - for self-hosted token servers

    For servers, agents and scripts, pass ``secret_key=``, or set no auth at
    all and rely on ``ARETE_API_KEY`` or your ``a4`` login:

        auth = AuthConfig(secret_key="a4_sk_...")  # or an agent key "a4_ak_..."

    Publishable keys are for anything shipped to a browser; create one with
    ``a4 auth keys create-publishable --origin <scheme://host[:port]>``:

        auth = AuthConfig(publishable_key="a4_pk_...")

    ``from_api_key()`` picks the right field from the key's prefix.

    Using static token:

        auth = AuthConfig(token="static_token_here")

    Using custom token provider:

        async def get_token():
            return AuthToken(token="...", expires_at=1234567890)
        auth = AuthConfig(get_token=get_token)
    """

    # Credential fields stay out of repr() so logging a config never prints
    # key or token material (matching the Rust SDK's masked Debug output).
    token: Optional[str] = field(default=None, repr=False)
    publishable_key: Optional[str] = field(default=None, repr=False)
    token_endpoint: Optional[str] = None
    get_token: Optional[TokenProvider] = None
    token_transport: TokenTransport = TokenTransport.QUERY
    token_endpoint_headers: Dict[str, str] = field(default_factory=dict, repr=False)
    token_endpoint_credentials: Optional[str] = None  # 'omit', 'same-origin', 'include'
    secret_key: Optional[str] = field(default=None, repr=False)
    """Agent key (``a4_ak_...``) or secret key (``a4_sk_...``). Server-side
    only; defaults to ``ARETE_API_KEY``, then the active ``a4`` CLI login's
    key, when no auth option is set."""

    @classmethod
    def from_api_key(cls, api_key: str, **kwargs) -> "AuthConfig":
        """Create AuthConfig from an API key of either class.

        Publishable keys (``a4_pk_...``) become ``publishable_key``; agent,
        secret and unrecognised keys become ``secret_key``.

        Args:
            api_key: The API key
            **kwargs: Additional auth config options

        Example:
            auth = AuthConfig.from_api_key("a4_sk_...")
            auth = AuthConfig.from_api_key("a4_ak_...", token_transport=TokenTransport.BEARER)
        """
        if classify_api_key(api_key) == "publishable":
            return cls(publishable_key=api_key, **kwargs)
        return cls(secret_key=api_key, **kwargs)

    @property
    def api_key(self) -> Optional[str]:
        """The key sent as the token endpoint bearer credential, if any."""
        return self.secret_key or self.publishable_key

    def __post_init__(self):
        # Error and warning text never includes key material.
        if self.secret_key is not None:
            if not self.secret_key.strip():
                raise AreteError("auth.secret_key is empty", "INVALID_CONFIG")
            if classify_api_key(self.secret_key) == "publishable":
                raise AreteError(
                    "auth.secret_key was given a publishable key (a4_pk_...). Pass it "
                    "as auth.publishable_key instead; auth.secret_key takes an agent "
                    "key (a4_ak_...) or secret key (a4_sk_...).",
                    "INVALID_CONFIG",
                )
            if self.publishable_key is not None:
                raise AreteError(
                    "Set either auth.secret_key (servers and scripts) or "
                    "auth.publishable_key (browsers), not both.",
                    "INVALID_CONFIG",
                )
        if (
            self.publishable_key is not None
            and classify_api_key(self.publishable_key) == "secret"
        ):
            _warn_once(
                "secret-in-publishable",
                "auth.publishable_key was given a secret-class key (a4_sk_... or "
                "a4_ak_...). Pass it as auth.secret_key instead, or set "
                "ARETE_API_KEY. Publishable keys are for code shipped to a "
                f"browser; {_CREATE_PUBLISHABLE_HINT}.",
            )

        # Validate that at most one auth strategy is specified
        strategies = sum(
            [
                1 if self.token else 0,
                1 if self.get_token else 0,
                1 if (self.api_key or self.token_endpoint) else 0,
            ]
        )
        if strategies > 1:
            logger.warning(
                "Multiple auth strategies specified. Priority: token > get_token > token_endpoint/secret_key/publishable_key"
            )


def _has_explicit_auth(config: Optional[AuthConfig]) -> bool:
    return config is not None and bool(
        config.token
        or config.get_token
        or config.token_endpoint
        or config.publishable_key
        or config.secret_key
    )


NO_API_KEY_HINT = (
    "No Arete API key found. Run `a4 auth login` (or `a4 auth signup` for an "
    "agent), or set ARETE_API_KEY, or pass AuthConfig(secret_key=...)."
)
"""Appended when a request that carried no API key is refused. It names the
commands and options that supply one, never where credentials are stored."""


def _with_secret_key(config: Optional[AuthConfig], key: str) -> AuthConfig:
    if config is None:
        return AuthConfig(secret_key=key)
    return replace(config, secret_key=key)


def resolve_auth_config(
    config: Optional[AuthConfig],
    read_profile_key: Optional[Callable[[], "ProfileKey"]] = None,
) -> Optional[AuthConfig]:
    """Apply the server-side credential chain.

    When no auth option is set (no token, token provider, token endpoint,
    publishable key or secret key), a non-empty ``ARETE_API_KEY`` supplies
    ``secret_key``; without it, the agent or secret key from the active ``a4``
    CLI login does. A publishable key in either place is ignored.
    """
    if _has_explicit_auth(config):
        return config
    env_key = (os.environ.get(ARETE_API_KEY_ENV) or "").strip()
    if env_key:
        if classify_api_key(env_key) != "publishable":
            return _with_secret_key(config, env_key)
        _warn_once(
            "publishable-in-env",
            f"{ARETE_API_KEY_ENV} holds a publishable key (a4_pk_...) and was "
            "ignored. Set it to an agent key (a4_ak_...) or secret key "
            "(a4_sk_...), or pass the publishable key as auth.publishable_key.",
        )

    profile = (read_profile_key or read_profile_key_default)()
    if profile.ambiguous:
        _warn_once(
            "ambiguous-a4-profile",
            "More than one a4 login profile holds a key; not choosing one. Set "
            f"ARETE_PROFILE (for example `agent`) or {ARETE_API_KEY_ENV}.",
        )
    if profile.key and classify_api_key(profile.key) == "secret":
        return _with_secret_key(config, profile.key)
    return config


def parse_jwt_expiry(token: str) -> Optional[int]:
    """Parse the exp claim from a JWT token."""
    try:
        parts = token.split(".")
        if len(parts) != 3:
            return None

        payload = parts[1]
        # Add padding if needed
        padding_needed = 4 - len(payload) % 4
        if padding_needed != 4:
            payload += "=" * padding_needed

        decoded = base64.urlsafe_b64decode(payload.encode("utf-8"))
        data = json.loads(decoded.decode("utf-8"))
        exp = data.get("exp")
        return int(exp) if isinstance(exp, (int, float)) else None
    except Exception:
        return None


def is_hosted_arete_websocket_url(websocket_url: str) -> bool:
    """Check if URL is a hosted Arete Cloud URL."""
    try:
        from urllib.parse import urlparse

        host = urlparse(websocket_url).hostname or ""
        return is_hosted_websocket_host(host)
    except Exception:
        return False


def resolve_token_endpoint(
    config: Optional[AuthConfig], websocket_url: Optional[str]
) -> Optional[str]:
    """Determine the token endpoint for a configuration + stack URL.

    Explicit ``token_endpoint`` wins; hosted Arete stack URLs fall back to the
    hosted default endpoint (anonymous minting is allowed without a key).
    """
    if config is not None and config.token_endpoint:
        return config.token_endpoint
    if websocket_url and is_hosted_arete_websocket_url(websocket_url):
        return DEFAULT_HOSTED_TOKEN_ENDPOINT
    return None


def build_token_endpoint_request_body(
    *,
    websocket_url: Optional[str],
    scopes: Sequence[str],
    target_kind: Optional[str] = None,
    target_id: Optional[str] = None,
    program_release_hash: Optional[str] = None,
    stack_release: Optional["StackRelease"] = None,
) -> Dict[str, Any]:
    """Build the token endpoint POST body.

    Untargeted: ``{"websocket_url", "scopes"[, "stackManifestHash",
    "liveAlias"]}`` — the served stack version is named only when
    ``stack_release`` is given. Targeted (program-read-binding /
    solana-gateway-binding): ``{"targetKind", "targetId", "scopes"[,
    "programReleaseHash"]}``; a stack release is never added to it.
    """
    if target_kind is not None:
        body: Dict[str, Any] = {
            "targetKind": target_kind,
            "targetId": target_id,
            "scopes": list(scopes),
        }
        if program_release_hash is not None:
            body["programReleaseHash"] = program_release_hash
        return body
    body = {"websocket_url": websocket_url or "", "scopes": list(scopes)}
    body.update(stack_release_fields(stack_release))
    return body


def stack_release_fields(stack_release: Optional["StackRelease"]) -> Dict[str, str]:
    """The camelCase session request fields naming a served stack version."""
    if stack_release is None:
        return {}
    return {
        "stackManifestHash": stack_release.stack_manifest_hash,
        "liveAlias": stack_release.live_alias,
    }


def _non_empty_string(value: Any) -> Optional[str]:
    return value if isinstance(value, str) and value.strip() else None


def _stack_version_refusal_error(
    status: int,
    error_code: AuthErrorCode,
    message: str,
    error_data: Any,
    wire_error_code: Optional[str],
) -> AuthError:
    """An :class:`AuthError` naming the replacement and how to install it."""
    data = error_data if isinstance(error_data, dict) else {}
    replacement = data.get("replacement")
    replacement = replacement if isinstance(replacement, dict) else {}
    replacement_version = _non_empty_string(replacement.get("version"))
    replacement_hash = _non_empty_string(replacement.get("stackManifestHash"))
    upgrade_command = _non_empty_string(data.get("upgradeCommand"))
    retired_at = _non_empty_string(data.get("retiredAt"))

    text = f"Token endpoint returned {status}: {message}"
    guidance = []
    if replacement_version or replacement_hash:
        guidance.append(f"Replacement: {replacement_version or replacement_hash}.")
    if upgrade_command:
        guidance.append(f"Upgrade with: {upgrade_command}")
    if guidance:
        text = text.rstrip()
        separator = " " if text.endswith((".", "!", "?")) else ". "
        text = f"{text}{separator}{' '.join(guidance)}"

    return AuthError(
        text,
        error_code,
        {
            "status": status,
            "wire_error_code": wire_error_code,
            "replacement": {
                key: value
                for key, value in (
                    ("version", replacement_version),
                    ("stack_manifest_hash", replacement_hash),
                )
                if value is not None
            }
            or None,
            "upgrade_command": upgrade_command,
            "retired_at": retired_at,
        },
        replacement_version=replacement_version,
        replacement_stack_manifest_hash=replacement_hash,
        upgrade_command=upgrade_command,
        retired_at=retired_at,
    )


async def request_token_from_endpoint(
    http_client: httpx.AsyncClient,
    endpoint: str,
    config: Optional[AuthConfig],
    body: Mapping[str, Any],
) -> AuthToken:
    """POST the token endpoint and parse ``{token, expires_at[, scopes]}``.

    Sends ``Authorization: Bearer <secret or publishable key>`` plus any
    configured endpoint headers. Raises :class:`AuthError` on failure.
    """
    headers: Dict[str, str] = {"Content-Type": "application/json"}
    if config is not None:
        if config.api_key:
            headers["Authorization"] = f"Bearer {config.api_key}"
        headers.update(config.token_endpoint_headers or {})

    try:
        response = await http_client.post(endpoint, headers=headers, json=dict(body))
    except httpx.HTTPError as e:
        raise AuthError(
            f"Token endpoint request failed: {e}", AuthErrorCode.INTERNAL_ERROR
        ) from e

    error_code_header = response.headers.get("X-Error-Code")
    if response.status_code < 200 or response.status_code >= 300:
        raw = response.text
        error_code = None
        error_message = raw or response.reason_phrase
        error_data = None
        try:
            error_data = json.loads(raw)
            if isinstance(error_data, dict):
                error_code_str = error_data.get("code") or error_code_header
                if error_code_str:
                    error_code = AuthErrorCode.from_wire(str(error_code_str))
                error_message = error_data.get("error") or error_message
        except json.JSONDecodeError:
            pass
        if error_code is None:
            if error_code_header:
                error_code = AuthErrorCode.from_wire(error_code_header)
            elif response.status_code == 429:
                error_code = AuthErrorCode.QUOTA_EXCEEDED
            else:
                error_code = AuthErrorCode.AUTH_REQUIRED
        if is_stack_version_refusal(error_code):
            raise _stack_version_refusal_error(
                response.status_code,
                error_code,
                error_message,
                error_data,
                error_code_header,
            )
        if response.status_code == 401 and not (config is not None and config.api_key):
            error_message = f"{error_message}. {NO_API_KEY_HINT}"
        raise AuthError(
            f"Token endpoint returned {response.status_code}: {error_message}",
            error_code,
            {"status": response.status_code, "wire_error_code": error_code_header},
        )

    data = response.json()
    token = data.get("token") if isinstance(data, dict) else None
    if not token:
        raise AuthError(
            "Token endpoint did not return a token",
            AuthErrorCode.TOKEN_INVALID_FORMAT,
        )
    expires_at = data.get("expires_at") or data.get("expiresAt")
    scopes = data.get("scopes")
    return AuthToken(
        token=token,
        expires_at=int(expires_at) if expires_at else None,
        scopes=list(scopes) if isinstance(scopes, list) else None,
    )


def build_websocket_url(
    websocket_url: str,
    token: Optional[str] = None,
    transport: TokenTransport = TokenTransport.QUERY,
) -> str:
    """Build WebSocket URL with authentication.

    For query transport, adds token as query parameter.
    For bearer transport, returns URL unchanged (token sent in headers).
    """
    if transport == TokenTransport.BEARER or token is None:
        return websocket_url

    from urllib.parse import urlparse, parse_qs, urlencode, urlunparse

    parsed = urlparse(websocket_url)
    query_params = parse_qs(parsed.query)
    query_params[DEFAULT_QUERY_PARAMETER] = [token]

    new_query = urlencode(query_params, doseq=True)
    return urlunparse(
        (
            parsed.scheme,
            parsed.netloc,
            parsed.path,
            parsed.params,
            new_query,
            parsed.fragment,
        )
    )


class AuthState:
    """Manages authentication state and token lifecycle.

    This is an internal class used by WebSocketManager to handle:
    - Token fetching from endpoints
    - Token caching and expiry
    - Automatic refresh scheduling
    """

    def __init__(
        self,
        websocket_url: str,
        config: Optional[AuthConfig] = None,
        stack_release: Optional["StackRelease"] = None,
    ):
        self.websocket_url = websocket_url
        self.config = resolve_auth_config(config)
        # Served stack version named in the session request, when the stack
        # definition carries one. Without it the request is unchanged.
        self.stack_release = stack_release
        self._current_token: Optional[str] = None
        self._token_expiry: Optional[int] = None
        self._refresh_timer: Optional[asyncio.Task] = None
        self._http_session: Optional[httpx.AsyncClient] = None

    def _get_http_session(self) -> httpx.AsyncClient:
        """Get or create HTTP client for token requests."""
        if self._http_session is None or self._http_session.is_closed:
            self._http_session = httpx.AsyncClient()
        return self._http_session

    async def close(self):
        """Cleanup resources."""
        if self._refresh_timer and not self._refresh_timer.done():
            self._refresh_timer.cancel()
            try:
                await self._refresh_timer
            except asyncio.CancelledError:
                pass
        if self._http_session and not self._http_session.is_closed:
            await self._http_session.aclose()

    def has_refreshable_auth(self) -> bool:
        """Check if auth strategy supports token refresh."""
        if self.config is None:
            return False
        # Token provider and token endpoint both support refresh
        return (
            self.config.get_token is not None
            or self.config.token_endpoint is not None
            or (
                self.config.api_key is not None
                and is_hosted_arete_websocket_url(self.websocket_url)
            )
        )

    def _get_token_endpoint(self) -> Optional[str]:
        """Determine token endpoint URL."""
        if self.config is None:
            return None

        if self.config.token_endpoint:
            return self.config.token_endpoint

        # For hosted Arete URLs, use default endpoint if an API key is provided
        if self.config.api_key and is_hosted_arete_websocket_url(
            self.websocket_url
        ):
            return DEFAULT_HOSTED_TOKEN_ENDPOINT

        return None

    async def resolve_token(self, force_refresh: bool = False) -> Optional[str]:
        """Get current token or fetch a new one.

        Returns the token string, or None if no auth configured.
        Raises AuthError if token fetching fails.
        """
        # Return cached token if valid and not forcing refresh
        if not force_refresh and self._current_token is not None:
            if self._token_expiry is None or not self._is_token_expiring():
                return self._current_token

        # Determine auth strategy
        if self.config is None:
            if is_hosted_arete_websocket_url(self.websocket_url):
                raise AuthError(
                    f"{NO_API_KEY_HINT} Hosted Arete websocket connections need "
                    "an API key or one of auth.publishable_key, auth.get_token, "
                    "auth.token_endpoint or auth.token.",
                    AuthErrorCode.AUTH_REQUIRED,
                )
            return None

        # Priority 1: Static token
        if self.config.token:
            return self._set_token(AuthToken(token=self.config.token))

        # Priority 2: Token provider function
        if self.config.get_token:
            try:
                token = await self.config.get_token()
                return self._set_token(token)
            except Exception as e:
                raise AuthError(
                    f"Failed to get authentication token: {e}",
                    AuthErrorCode.AUTH_REQUIRED,
                ) from e

        # Priority 3: Token endpoint (custom or hosted)
        token_endpoint = self._get_token_endpoint()
        if token_endpoint:
            try:
                token = await self._fetch_token_from_endpoint(token_endpoint)
                return self._set_token(token)
            except Exception as e:
                if isinstance(e, AuthError):
                    raise
                raise AuthError(
                    f"Failed to fetch authentication token from endpoint: {e}",
                    AuthErrorCode.AUTH_REQUIRED,
                ) from e

        # No auth strategy matched
        if is_hosted_arete_websocket_url(self.websocket_url):
            raise AuthError(
                f"{NO_API_KEY_HINT} Hosted Arete websocket connections require "
                "authentication.",
                AuthErrorCode.AUTH_REQUIRED,
            )

        return None

    def _set_token(self, token: AuthToken) -> str:
        """Store token and extract expiry from JWT if not provided."""
        if not token.token or not token.token.strip():
            raise AuthError(
                "Authentication provider returned an empty token",
                AuthErrorCode.TOKEN_INVALID_FORMAT,
            )

        self._current_token = token.token.strip()

        # Use explicit expiry or parse from JWT
        self._token_expiry = token.expires_at or parse_jwt_expiry(self._current_token)

        # Check if already expired
        if self._token_expiry and self._is_token_expiring():
            raise AuthError(
                "Authentication token is expired", AuthErrorCode.TOKEN_EXPIRED
            )

        return self._current_token

    def _is_token_expiring(
        self, buffer_seconds: int = TOKEN_REFRESH_BUFFER_SECONDS
    ) -> bool:
        """Check if current token is expired or about to expire."""
        if self._token_expiry is None:
            return False
        return time.time() >= self._token_expiry - buffer_seconds

    def clear_token(self):
        """Clear cached token state (e.g., after auth error)."""
        self._current_token = None
        self._token_expiry = None

    async def _fetch_token_from_endpoint(self, endpoint: str) -> AuthToken:
        """Fetch token from token endpoint."""
        return await request_token_from_endpoint(
            self._get_http_session(),
            endpoint,
            self.config,
            {
                "websocket_url": self.websocket_url,
                **stack_release_fields(self.stack_release),
            },
        )

    def get_refresh_delay(self) -> Optional[float]:
        """Calculate delay until token refresh is needed.

        Returns seconds until refresh, or None if no refresh needed.
        """
        if not self.has_refreshable_auth():
            return None

        if self._token_expiry is None:
            return None

        refresh_at = self._token_expiry - TOKEN_REFRESH_BUFFER_SECONDS
        delay = max(MIN_REFRESH_DELAY_SECONDS, refresh_at - time.time())
        return delay

    async def schedule_refresh(
        self, callback: Callable[[], Coroutine[Any, Any, None]]
    ) -> Optional[asyncio.Task]:
        """Schedule a token refresh callback.

        Returns the scheduled task, or None if no refresh needed.
        """
        delay = self.get_refresh_delay()
        if delay is None:
            return None

        async def refresh_task():
            await asyncio.sleep(delay)
            await callback()

        # Cancel any existing timer
        if self._refresh_timer and not self._refresh_timer.done():
            self._refresh_timer.cancel()

        self._refresh_timer = asyncio.create_task(refresh_task())
        return self._refresh_timer


def parse_error_code_from_close_reason(reason: str) -> Optional[AuthErrorCode]:
    """Parse the error code a WebSocket close reason starts with.

    The server closes with ``"<code>: <message>"`` (for example
    ``"token-expired: Token has expired"``), or with the bare code. A colon
    prefix that is not a wire code is kept as ``INTERNAL_ERROR`` so it is
    still treated as a coded close.

    A free-form reason is never guessed at: one that merely mentions
    "token", "invalid" or "expired" is not a token expiry, and reading it as
    one would drop a valid token and reconnect without backing off.
    """
    if not reason:
        return None

    if ":" in reason:
        code_part = reason.split(":", 1)[0].strip()
        return AuthErrorCode.from_wire(code_part)

    return AuthErrorCode.from_wire_known(reason)
