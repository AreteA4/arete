"""Exception hierarchy for the Arete Python SDK.

Rooted at :class:`AreteError`. Core/shared errors live here; module-specific
errors (chain, transactions, reads, execution) subclass :class:`AreteError`
in their own modules.
"""

from __future__ import annotations

from typing import Any, Optional


class AreteError(Exception):
    """Root of the Arete exception hierarchy.

    ``code`` is a machine-readable error code (string or enum with ``value``);
    ``details`` carries structured context (wire frames, response bodies, the
    original exception, ...).
    """

    def __init__(self, message: str, code: Any = None, details: Any = None) -> None:
        super().__init__(message)
        self.message = message
        self.code = code
        self.details = details

    def __str__(self) -> str:
        if self.code is not None:
            code = self.code.value if hasattr(self.code, "value") else self.code
            return f"[{code}] {self.message}"
        return self.message


class AreteConnectionError(AreteError):
    """WebSocket transport failures (connect, reconnect, send, close)."""


class SubscriptionError(AreteError):
    """Protocol v2 subscription setup / lease / refresh failures."""


class AuthError(AreteError):
    """Authentication failures with optional machine-readable error code."""


class ProcessedSlotTimeoutError(AreteError):
    """Raised when ``wait_for_processed_slot`` times out."""

    def __init__(self, target_slot: int, processed_slot: Optional[int]) -> None:
        super().__init__(
            f"Timed out waiting for Arete to process slot {target_slot}",
            "PROCESSED_SLOT_TIMEOUT",
        )
        self.target_slot = target_slot
        self.processed_slot = processed_slot


class HttpRequestError(AreteError):
    """Non-2xx HTTP response carrying the structured Arete error body.

    The structured body fields (``code``, ``message``, ``retryable``,
    ``retry_after``, ``suggested_action``, ``docs_url``, ``fatal``) are
    attached when the server provides them; ``status`` is the HTTP status and
    ``body`` is the raw decoded response body when available.
    """

    def __init__(
        self,
        message: str,
        *,
        code: Optional[str] = None,
        status: Optional[int] = None,
        retryable: bool = False,
        retry_after: Optional[float] = None,
        suggested_action: Optional[str] = None,
        docs_url: Optional[str] = None,
        fatal: bool = False,
        body: Any = None,
        details: Any = None,
    ) -> None:
        super().__init__(message, code, details)
        self.status = status
        self.retryable = retryable
        self.retry_after = retry_after
        self.suggested_action = suggested_action
        self.docs_url = docs_url
        self.fatal = fatal
        self.body = body
