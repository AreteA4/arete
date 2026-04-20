"""Authentication support for Arete Python SDK."""

import asyncio
import base64
import json
import logging
import time
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Callable, Dict, Optional, Coroutine
import aiohttp

from arete.errors import AreteError, AuthError

logger = logging.getLogger(__name__)

TOKEN_REFRESH_BUFFER_SECONDS = 60
MIN_REFRESH_DELAY_SECONDS = 1
DEFAULT_QUERY_PARAMETER = "hs_token"
DEFAULT_HOSTED_TOKEN_ENDPOINT = "https://api.arete.run/ws/sessions"
HOSTED_WEBSOCKET_SUFFIX = ".stack.arete.run"


class TokenTransport(Enum):
    """How the websocket token is sent to the server."""

    QUERY = "query"
    BEARER = "bearer"


@dataclass
class AuthToken:
    """Represents an authentication token with optional expiry."""

    token: str
    expires_at: Optional[int] = None  # Unix timestamp in seconds

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
    # Server errors
    INTERNAL_ERROR = "internal_error"

    @classmethod
    def from_wire(cls, error_code: str) -> "AuthErrorCode":
        """Parse a kebab-case or snake_case error code string."""
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
            # Also support snake_case variants
            "token_missing": cls.TOKEN_MISSING,
            "token_expired": cls.TOKEN_EXPIRED,
            "token_invalid_signature": cls.TOKEN_INVALID_SIGNATURE,
            "token_invalid_format": cls.TOKEN_INVALID_FORMAT,
        }
        return code_map.get(error_code.lower(), cls.INTERNAL_ERROR)


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
    3. API key - for server-side use (can be secret or publishable key)
    4. Publishable key - for browser/client use with hosted Arete Cloud
    5. Custom token endpoint - for self-hosted token servers

    For server-side code, use `from_api_key()` or pass `publishable_key=`
    (which accepts any API key, not just publishable ones):

        auth = AuthConfig.from_api_key("hspk_...")  # or "hssk_..."
        auth = AuthConfig(publishable_key="hspk_...")  # same thing

    For browser/client code, use publishable_key directly:

        auth = AuthConfig(publishable_key="hspk_...")  # must be publishable

    Using static token:

        auth = AuthConfig(token="static_token_here")

    Using custom token provider:

        async def get_token():
            return AuthToken(token="...", expires_at=1234567890)
        auth = AuthConfig(get_token=get_token)
    """

    token: Optional[str] = None
    publishable_key: Optional[str] = None
    token_endpoint: Optional[str] = None
    get_token: Optional[TokenProvider] = None
    token_transport: TokenTransport = TokenTransport.QUERY
    token_endpoint_headers: Dict[str, str] = field(default_factory=dict)
    token_endpoint_credentials: Optional[str] = None  # 'omit', 'same-origin', 'include'

    @classmethod
    def from_api_key(cls, api_key: str, **kwargs) -> "AuthConfig":
        """Create AuthConfig from an API key.

        Use this for server-side code where the key could be either a
        secret key or a publishable key. For browser/client code, use
        the constructor with publishable_key=... directly.

        Args:
            api_key: The API key (can be secret or publishable)
            **kwargs: Additional auth config options

        Example:
            auth = AuthConfig.from_api_key("hspk_...")
            auth = AuthConfig.from_api_key("hssk_...", token_transport=TokenTransport.BEARER)
        """
        return cls(publishable_key=api_key, **kwargs)

    def __post_init__(self):
        # Validate that at most one auth strategy is specified
        strategies = sum(
            [
                1 if self.token else 0,
                1 if self.get_token else 0,
                1 if (self.publishable_key or self.token_endpoint) else 0,
            ]
        )
        if strategies > 1:
            logger.warning(
                "Multiple auth strategies specified. Priority: token > get_token > token_endpoint/publishable_key"
            )


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
        return host.lower().endswith(HOSTED_WEBSOCKET_SUFFIX)
    except Exception:
        return False


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

    def __init__(self, websocket_url: str, config: Optional[AuthConfig] = None):
        self.websocket_url = websocket_url
        self.config = config
        self._current_token: Optional[str] = None
        self._token_expiry: Optional[int] = None
        self._refresh_timer: Optional[asyncio.Task] = None
        self._http_session: Optional[aiohttp.ClientSession] = None

    def _get_http_session(self) -> aiohttp.ClientSession:
        """Get or create HTTP session for token requests."""
        if self._http_session is None or self._http_session.closed:
            self._http_session = aiohttp.ClientSession()
        return self._http_session

    async def close(self):
        """Cleanup resources."""
        if self._refresh_timer and not self._refresh_timer.done():
            self._refresh_timer.cancel()
            try:
                await self._refresh_timer
            except asyncio.CancelledError:
                pass
        if self._http_session and not self._http_session.closed:
            await self._http_session.close()

    def has_refreshable_auth(self) -> bool:
        """Check if auth strategy supports token refresh."""
        if self.config is None:
            return False
        # Token provider and token endpoint both support refresh
        return (
            self.config.get_token is not None
            or self.config.token_endpoint is not None
            or (
                self.config.publishable_key is not None
                and is_hosted_arete_websocket_url(self.websocket_url)
            )
        )

    def _get_token_endpoint(self) -> Optional[str]:
        """Determine token endpoint URL."""
        if self.config is None:
            return None

        if self.config.token_endpoint:
            return self.config.token_endpoint

        # For hosted Arete URLs, use default endpoint if publishable key provided
        if self.config.publishable_key and is_hosted_arete_websocket_url(
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
                    "Hosted Arete websocket connections require an API key, "
                    "auth.get_token, auth.token_endpoint, or auth.token",
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
                "Hosted Arete websocket connections require authentication",
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
        session = self._get_http_session()

        headers: Dict[str, str] = {
            "Content-Type": "application/json",
        }

        # Add custom endpoint headers
        if self.config and self.config.token_endpoint_headers:
            headers.update(self.config.token_endpoint_headers)

        # Add publishable key as Authorization header if present
        if self.config and self.config.publishable_key:
            headers["Authorization"] = f"Bearer {self.config.publishable_key}"

        payload = {"websocket_url": self.websocket_url}

        try:
            async with session.post(
                endpoint, headers=headers, json=payload
            ) as response:
                # Check for error code in headers
                error_code_header = response.headers.get("X-Error-Code")

                if not response.ok:
                    body = await response.text()

                    # Try to parse error from body
                    error_code = None
                    error_message = body or response.reason

                    try:
                        error_data = json.loads(body)
                        if isinstance(error_data, dict):
                            error_code_str = error_data.get("code") or error_code_header
                            if error_code_str:
                                error_code = AuthErrorCode.from_wire(error_code_str)
                            error_message = error_data.get("error") or error_message
                    except json.JSONDecodeError:
                        pass

                    if error_code is None:
                        # Infer from status code
                        if response.status == 429:
                            error_code = AuthErrorCode.QUOTA_EXCEEDED
                        else:
                            error_code = AuthErrorCode.AUTH_REQUIRED

                    raise AuthError(
                        f"Token endpoint returned {response.status}: {error_message}",
                        error_code,
                        {
                            "status": response.status,
                            "wire_error_code": error_code_header,
                        },
                    )

                data = await response.json()

                token = data.get("token")
                if not token:
                    raise AuthError(
                        "Token endpoint did not return a token",
                        AuthErrorCode.TOKEN_INVALID_FORMAT,
                    )

                # Support both camelCase and snake_case for expires_at
                expires_at = data.get("expires_at") or data.get("expiresAt")
                if expires_at:
                    expires_at = int(expires_at)

                return AuthToken(token=token, expires_at=expires_at)

        except aiohttp.ClientError as e:
            raise AuthError(
                f"Token endpoint request failed: {e}", AuthErrorCode.INTERNAL_ERROR
            ) from e

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
    """Parse error code from WebSocket close reason (e.g., 'token-expired: Token has expired')."""
    if not reason:
        return None

    # Try to extract error code from format "error-code: message"
    if ":" in reason:
        code_part = reason.split(":", 1)[0].strip()
        return AuthErrorCode.from_wire(code_part)

    # Check for common patterns
    reason_lower = reason.lower()
    if (
        "expired" in reason_lower
        or "invalid" in reason_lower
        or "token" in reason_lower
    ):
        return AuthErrorCode.TOKEN_EXPIRED

    return None
