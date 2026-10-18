"""Webhook signature verification for emitted protocol events.

Port of ``typescript/core/src/protocol/webhooks.ts``. A spec's ``emit``
reactions publish named events; a webhook endpoint subscribed to them
receives a signed POST whose payload carries ids and states only.

Signature scheme (``v1``):

- ``arete-webhook-id``: the delivery id, stable across retries;
- ``arete-webhook-timestamp``: Unix seconds when the delivery was signed;
- ``arete-webhook-signature``: one or more space-separated ``v1=<hex>``
  entries, each a lowercase hex HMAC-SHA256 over ``f"{timestamp}.{body}"``
  keyed with the endpoint secret. Several entries appear while a secret is
  being rotated; any one matching is enough.
"""

from __future__ import annotations

import hashlib
import hmac
import json
import re
import time
from dataclasses import dataclass
from typing import Any, Generic, Mapping, Optional, TypeVar, Union

WEBHOOK_ID_HEADER = "arete-webhook-id"
WEBHOOK_TIMESTAMP_HEADER = "arete-webhook-timestamp"
WEBHOOK_SIGNATURE_HEADER = "arete-webhook-signature"
WEBHOOK_TOLERANCE_SECONDS = 300

WEBHOOK_ERROR_CODES = (
    "missing-header",
    "invalid-timestamp",
    "timestamp-out-of-tolerance",
    "invalid-signature",
    "invalid-payload",
)

_TIMESTAMP = re.compile(r"0|[1-9][0-9]{0,15}")

TPayload = TypeVar("TPayload")
BytesOrStr = Union[str, bytes, bytearray]


class WebhookVerificationError(Exception):
    """A webhook delivery failed verification; ``code`` is one of
    :data:`WEBHOOK_ERROR_CODES`."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


@dataclass(frozen=True)
class VerifiedWebhook(Generic[TPayload]):
    id: str
    timestamp: int
    payload: TPayload


def _to_bytes(value: BytesOrStr) -> bytes:
    return value.encode("utf-8") if isinstance(value, str) else bytes(value)


def _header(headers: Any, name: str) -> Optional[str]:
    """Case-insensitive header lookup over a mapping (``dict``,
    ``httpx.Headers``, framework header objects) or any ``.get`` object."""
    value: Any = None
    if isinstance(headers, Mapping):
        value = headers.get(name)
        if value is None:
            for key in headers.keys():
                if isinstance(key, str) and key.lower() == name:
                    value = headers[key]
                    break
    elif callable(getattr(headers, "get", None)):
        value = headers.get(name)
    if isinstance(value, (list, tuple)):
        return " ".join(str(item) for item in value)
    if isinstance(value, bytes):
        return value.decode("latin-1")
    return value if isinstance(value, str) else None


def sign_webhook(secret: BytesOrStr, timestamp: int, body: BytesOrStr) -> str:
    """The ``v1`` signature for a body: lowercase hex HMAC-SHA256 over
    ``f"{timestamp}.{body}"``."""
    message = f"{timestamp}.".encode("utf-8") + _to_bytes(body)
    return hmac.new(_to_bytes(secret), message, hashlib.sha256).hexdigest()


def verify_webhook(
    secret: BytesOrStr,
    body: BytesOrStr,
    headers: Any,
    *,
    now: Optional[int] = None,
    tolerance_seconds: int = WEBHOOK_TOLERANCE_SECONDS,
) -> VerifiedWebhook[Any]:
    """Verify a webhook delivery and return its parsed payload.

    Pass the raw request body exactly as received (``str`` or ``bytes``),
    not re-serialized JSON. Raises :class:`WebhookVerificationError` on any
    failure.
    """
    delivery_id = _header(headers, WEBHOOK_ID_HEADER)
    timestamp_text = _header(headers, WEBHOOK_TIMESTAMP_HEADER)
    signature_text = _header(headers, WEBHOOK_SIGNATURE_HEADER)
    if not delivery_id:
        raise WebhookVerificationError("missing-header", f"missing {WEBHOOK_ID_HEADER}")
    if not timestamp_text:
        raise WebhookVerificationError("missing-header", f"missing {WEBHOOK_TIMESTAMP_HEADER}")
    if not signature_text:
        raise WebhookVerificationError("missing-header", f"missing {WEBHOOK_SIGNATURE_HEADER}")
    if not _TIMESTAMP.fullmatch(timestamp_text):
        raise WebhookVerificationError("invalid-timestamp", "the timestamp is not Unix seconds")
    timestamp = int(timestamp_text)
    current = int(time.time()) if now is None else now
    if abs(current - timestamp) > tolerance_seconds:
        raise WebhookVerificationError(
            "timestamp-out-of-tolerance", "the delivery is outside the allowed clock skew"
        )
    expected = sign_webhook(secret, timestamp, body)
    matched = False
    for entry in signature_text.split():
        if entry.startswith("v1="):
            candidate = entry[3:].encode("utf-8", "replace")
            matched = hmac.compare_digest(candidate, expected.encode("ascii")) or matched
    if not matched:
        raise WebhookVerificationError("invalid-signature", "no v1 signature matches")
    try:
        payload = json.loads(_to_bytes(body).decode("utf-8"))
    except (ValueError, UnicodeDecodeError) as error:
        raise WebhookVerificationError("invalid-payload", "the body is not JSON") from error
    return VerifiedWebhook(id=delivery_id, timestamp=timestamp, payload=payload)
