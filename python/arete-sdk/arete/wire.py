"""WebSocket protocol v2 wire types.

Mirror of ``typescript/core/src/frame.ts`` and the frame halves of
``types.ts``: client envelopes, server frames, gzip detection, ``seq``
parsing/comparison, and u64 decimal-string helpers.

Envelope fields are camelCase on the wire; entity payloads are snake_case and
pass through untransformed.
"""

from __future__ import annotations

import gzip
import json
import re
import unicodedata
from dataclasses import dataclass
from typing import Any, Mapping, Optional, Tuple, Union

from arete.errors import AreteError

PROTOCOL_VERSION = 2

GZIP_MAGIC = b"\x1f\x8b"

FRAME_MODES = frozenset({"state", "append", "list"})
LIVE_OPS = frozenset({"upsert", "patch", "remove", "delete"})
QUERY_FIELDS = (
    "view",
    "key",
    "partition",
    "filters",
    "take",
    "skip",
    "after",
    "snapshotLimit",
)

_ASCII_DIGITS = re.compile(r"^[0-9]+$")

U64_MAX = 2**64 - 1


# ---------------------------------------------------------------------------
# u64 decimal-string helpers
# ---------------------------------------------------------------------------


def format_u64(value: int) -> str:
    """Format a native int as the wire's u64 decimal string."""
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError("u64 values must be int")
    if value < 0 or value > U64_MAX:
        raise ValueError(f"u64 value out of range: {value}")
    return str(value)


def parse_u64(value: str) -> int:
    """Parse a wire u64 decimal string into a native int."""
    if not isinstance(value, str) or not _ASCII_DIGITS.match(value):
        raise ValueError(f"invalid u64 decimal string: {value!r}")
    parsed = int(value)
    if parsed > U64_MAX:
        raise ValueError(f"u64 value out of range: {value}")
    return parsed


# ---------------------------------------------------------------------------
# Seq: "<slot>:<index>" — slot compares numerically, index lexicographically
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Seq:
    """Parsed ``_seq`` cursor. ``slot`` is None when the prefix is not numeric."""

    slot: Optional[int]
    index: str
    raw: str


def parse_seq(value: str) -> Seq:
    parts = value.split(":")
    slot_text = parts[0] if parts else ""
    index = parts[1] if len(parts) > 1 else ""
    slot = int(slot_text) if _ASCII_DIGITS.match(slot_text) else None
    return Seq(slot=slot, index=index, raw=value)


def seq_slot(value: Any) -> Optional[int]:
    """Extract the numeric slot from a seq string, or None."""
    if not isinstance(value, str):
        return None
    slot_text = value.split(":", 1)[0]
    return int(slot_text) if _ASCII_DIGITS.match(slot_text) else None


def compare_seq(left: str, right: str) -> int:
    """Compare two seq strings: slot numerically, then index lexicographically."""
    left_parsed = parse_seq(left)
    right_parsed = parse_seq(right)
    if left_parsed.slot is not None and right_parsed.slot is not None:
        if left_parsed.slot != right_parsed.slot:
            return -1 if left_parsed.slot < right_parsed.slot else 1
    if left_parsed.index == right_parsed.index:
        return 0
    return -1 if left_parsed.index < right_parsed.index else 1


# ---------------------------------------------------------------------------
# Client envelopes (every message carries protocolVersion: 2)
# ---------------------------------------------------------------------------


def subscribe_envelope(
    subscription_id: str,
    query: Mapping[str, Any],
    snapshot_enabled: bool = True,
) -> dict:
    return {
        "type": "subscribe",
        "protocolVersion": PROTOCOL_VERSION,
        "subscriptionId": subscription_id,
        "query": dict(query),
        "snapshot": {"enabled": snapshot_enabled},
    }


def unsubscribe_envelope(subscription_id: str) -> dict:
    return {
        "type": "unsubscribe",
        "protocolVersion": PROTOCOL_VERSION,
        "subscriptionId": subscription_id,
    }


def ping_envelope() -> dict:
    return {"type": "ping", "protocolVersion": PROTOCOL_VERSION}


def refresh_auth_envelope(token: str) -> dict:
    return {
        "type": "refresh_auth",
        "protocolVersion": PROTOCOL_VERSION,
        "token": token,
    }


# ---------------------------------------------------------------------------
# Server frames
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class SortConfig:
    field: Tuple[str, ...]
    order: str  # 'asc' | 'desc'


@dataclass(frozen=True)
class SubscribedFrame:
    subscription_id: str
    query: Mapping[str, Any]  # effective query, wire (camelCase) field names
    mode: str  # 'state' | 'append' | 'list'
    sort: Optional[SortConfig] = None


@dataclass(frozen=True)
class UnsubscribedFrame:
    subscription_id: str


@dataclass(frozen=True)
class SnapshotEntity:
    key: str
    data: Any


@dataclass(frozen=True)
class SnapshotFrame:
    subscription_id: str
    snapshot_id: str
    authoritative: bool
    mode: str
    entity: str
    data: Tuple[SnapshotEntity, ...]
    complete: bool
    key: Optional[str] = None


@dataclass(frozen=True)
class EntityFrame:
    subscription_id: str
    mode: str
    entity: str
    op: str  # 'upsert' | 'patch' | 'remove' | 'delete'
    key: str
    data: Any
    append: Tuple[str, ...] = ()
    seq: Optional[str] = None


@dataclass(frozen=True)
class ErrorFrame:
    """Structured protocol/subscription error envelope."""

    subscription_id: Optional[str]
    code: str
    fatal: bool
    error: Optional[str] = None
    message: Optional[str] = None
    retryable: Optional[bool] = None
    retry_after: Optional[float] = None
    suggested_action: Optional[str] = None
    docs_url: Optional[str] = None


Frame = Union[SubscribedFrame, UnsubscribedFrame, SnapshotFrame, EntityFrame, ErrorFrame]


# ---------------------------------------------------------------------------
# Update taxonomy delivered to consumers
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Update:
    """Raw view update: op is 'upsert' | 'patch' | 'remove' | 'delete'.

    ``data`` is the full entity on upsert, the partial patch on patch, and
    None on remove/delete.
    """

    op: str
    key: str
    data: Any = None


@dataclass(frozen=True)
class RichUpdate:
    """Update with before/after diffs.

    ``type`` is 'created' | 'updated' | 'removed' | 'deleted'. ``data`` is set
    on created; ``before``/``after`` (and the raw ``patch`` when applicable) on
    updated; ``last_known`` on removed/deleted.
    """

    type: str
    key: str
    data: Any = None
    before: Any = None
    after: Any = None
    patch: Any = None
    last_known: Any = None


# ---------------------------------------------------------------------------
# Validation + parsing
# ---------------------------------------------------------------------------


def is_gzip_data(data: bytes) -> bool:
    return len(data) >= 2 and data[:2] == GZIP_MAGIC


def is_valid_subscription_id(value: Any) -> bool:
    if not isinstance(value, str) or len(value) == 0 or value.strip() != value:
        return False
    if any(unicodedata.category(ch) == "Cc" for ch in value):
        return False
    return len(value.encode("utf-8")) <= 128


def _is_record(value: Any) -> bool:
    return isinstance(value, dict)


def _is_mode(value: Any) -> bool:
    return isinstance(value, str) and value in FRAME_MODES


def _is_sort(value: Any) -> bool:
    if not _is_record(value):
        return False
    field = value.get("field")
    return (
        isinstance(field, list)
        and all(isinstance(entry, str) for entry in field)
        and value.get("order") in ("asc", "desc")
    )


def _is_positive_int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


def _is_non_negative_int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


def is_valid_query(value: Any) -> bool:
    if not _is_record(value):
        return False
    view = value.get("view")
    if not isinstance(view, str) or len(view) == 0:
        return False
    if any(key not in QUERY_FIELDS for key in value):
        return False
    if "key" in value and not isinstance(value["key"], str):
        return False
    if "partition" in value and not isinstance(value["partition"], str):
        return False
    if "filters" in value and not _is_record(value["filters"]):
        return False
    if "take" in value and not _is_positive_int(value["take"]):
        return False
    if "skip" in value and not _is_non_negative_int(value["skip"]):
        return False
    if "after" in value and not isinstance(value["after"], str):
        return False
    if "snapshotLimit" in value and not _is_positive_int(value["snapshotLimit"]):
        return False
    return True


def is_valid_frame(frame: Any) -> bool:
    if not _is_record(frame) or frame.get("protocolVersion") != PROTOCOL_VERSION:
        return False

    if frame.get("type") == "error":
        subscription_id = frame.get("subscriptionId")
        return (
            (subscription_id is None or is_valid_subscription_id(subscription_id))
            and isinstance(frame.get("code"), str)
            and isinstance(frame.get("fatal"), bool)
            and ("message" not in frame or isinstance(frame["message"], str))
            and ("error" not in frame or isinstance(frame["error"], str))
            and ("retryable" not in frame or isinstance(frame["retryable"], bool))
        )

    if not is_valid_subscription_id(frame.get("subscriptionId")):
        return False
    op = frame.get("op")
    if not isinstance(op, str):
        return False
    if op == "unsubscribed":
        return True
    if op == "subscribed":
        return (
            is_valid_query(frame.get("query"))
            and _is_mode(frame.get("mode"))
            and ("sort" not in frame or _is_sort(frame["sort"]))
        )
    if not _is_mode(frame.get("mode")) or not isinstance(frame.get("entity"), str):
        return False
    if op == "snapshot":
        snapshot_id = frame.get("snapshotId")
        data = frame.get("data")
        return (
            isinstance(snapshot_id, str)
            and len(snapshot_id) > 0
            and isinstance(frame.get("authoritative"), bool)
            and isinstance(frame.get("complete"), bool)
            and ("key" not in frame or isinstance(frame["key"], str))
            and isinstance(data, list)
            and all(
                _is_record(entry) and isinstance(entry.get("key"), str) and "data" in entry
                for entry in data
            )
        )
    return (
        op in LIVE_OPS
        and isinstance(frame.get("key"), str)
        and "data" in frame
        and ("seq" not in frame or isinstance(frame["seq"], str))
        and (
            "append" not in frame
            or (
                isinstance(frame["append"], list)
                and all(isinstance(entry, str) for entry in frame["append"])
            )
        )
    )


def _frame_from_dict(frame: Mapping[str, Any]) -> Frame:
    if frame.get("type") == "error":
        return ErrorFrame(
            subscription_id=frame.get("subscriptionId"),
            code=frame["code"],
            fatal=frame["fatal"],
            error=frame.get("error"),
            message=frame.get("message"),
            retryable=frame.get("retryable"),
            retry_after=frame.get("retry_after"),
            suggested_action=frame.get("suggested_action"),
            docs_url=frame.get("docs_url"),
        )
    op = frame["op"]
    subscription_id = frame["subscriptionId"]
    if op == "unsubscribed":
        return UnsubscribedFrame(subscription_id=subscription_id)
    if op == "subscribed":
        sort = frame.get("sort")
        return SubscribedFrame(
            subscription_id=subscription_id,
            query=frame["query"],
            mode=frame["mode"],
            sort=SortConfig(field=tuple(sort["field"]), order=sort["order"]) if sort else None,
        )
    if op == "snapshot":
        return SnapshotFrame(
            subscription_id=subscription_id,
            snapshot_id=frame["snapshotId"],
            authoritative=frame["authoritative"],
            mode=frame["mode"],
            entity=frame["entity"],
            key=frame.get("key"),
            data=tuple(
                SnapshotEntity(key=entry["key"], data=entry["data"]) for entry in frame["data"]
            ),
            complete=frame["complete"],
        )
    return EntityFrame(
        subscription_id=subscription_id,
        mode=frame["mode"],
        entity=frame["entity"],
        op=op,
        key=frame["key"],
        data=frame.get("data"),
        append=tuple(frame.get("append") or ()),
        seq=frame.get("seq"),
    )


def decode_frame_payload(data: Union[str, bytes, bytearray, memoryview]) -> Any:
    """Decode a raw WebSocket message (text, binary, or gzip binary) to JSON."""
    if isinstance(data, str):
        return json.loads(data)
    raw = bytes(data)
    if is_gzip_data(raw):
        raw = gzip.decompress(raw)
    return json.loads(raw.decode("utf-8"))


def parse_frame(data: Union[str, bytes, bytearray, memoryview]) -> Frame:
    """Parse and validate one protocol v2 server frame.

    Raises :class:`AreteError` (code ``INVALID_FRAME``) on anything that is
    not a valid v2 frame.
    """
    try:
        frame = decode_frame_payload(data)
    except (ValueError, OSError) as exc:
        raise AreteError("Invalid WebSocket protocol v2 frame", "INVALID_FRAME", exc) from exc
    if not is_valid_frame(frame):
        raise AreteError("Invalid WebSocket protocol v2 frame", "INVALID_FRAME", frame)
    return _frame_from_dict(frame)


def frame_slot(frame: Frame) -> Optional[int]:
    """The highest chain slot carried by a frame, for processed-slot tracking."""
    if isinstance(frame, (SubscribedFrame, UnsubscribedFrame, ErrorFrame)):
        return None
    if isinstance(frame, SnapshotFrame):
        latest: Optional[int] = None
        for entity in frame.data:
            slot = seq_slot(entity.data.get("_seq")) if isinstance(entity.data, dict) else None
            if slot is not None and (latest is None or slot > latest):
                latest = slot
        return latest
    slot = seq_slot(frame.seq)
    if slot is not None:
        return slot
    if isinstance(frame.data, dict):
        return seq_slot(frame.data.get("_seq"))
    return None
