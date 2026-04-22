from __future__ import annotations
from dataclasses import dataclass
from typing import TYPE_CHECKING, Optional

if TYPE_CHECKING:
    from arete.auth import AuthErrorCode


@dataclass
class SocketIssue:
    """Structured error pushed by the server over the WebSocket."""
    error: str
    message: str
    code: Optional[AuthErrorCode] = None
    retryable: bool = False
    retry_after: Optional[float] = None
    suggested_action: Optional[str] = None
    docs_url: Optional[str] = None
    fatal: bool = False

    @classmethod
    def from_dict(cls, data: dict) -> "SocketIssue":
        from arete.auth import AuthErrorCode
        raw_code = data.get("code", "")
        normalized = raw_code.upper().replace("-", "_")
        code = None
        try:
            code = AuthErrorCode(normalized)
        except (ValueError, KeyError):
            pass
        return cls(
            error=data.get("error", ""),
            message=data.get("message", ""),
            code=code,
            retryable=data.get("retryable", False),
            retry_after=data.get("retryAfter"),
            suggested_action=data.get("suggestedAction"),
            docs_url=data.get("docsUrl"),
            fatal=data.get("fatal", False),
        )


class AreteError(Exception):
    """Base exception for all Arete errors"""

    pass


class ConnectionError(AreteError):
    """WebSocket connection issues"""

    pass


class SubscriptionError(AreteError):
    """Subscription setup/management failures"""

    pass


class ParseError(AreteError):
    """Entity parsing failures"""

    pass


class TimeoutError(AreteError):
    """Operation timeouts"""

    pass


class AuthError(AreteError):
    """Authentication failures with optional error code"""

    def __init__(self, message: str, code=None, details=None):
        super().__init__(message)
        self.code = code
        self.details = details

    def __str__(self):
        if self.code:
            return f"[{self.code.value if hasattr(self.code, 'value') else self.code}] {super().__str__()}"
        return super().__str__()
