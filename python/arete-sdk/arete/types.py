import gzip
import json
from enum import Enum
from typing import Any, Dict, List, Optional, Union
from dataclasses import dataclass, field


GZIP_MAGIC = bytes([0x1F, 0x8B])


class Mode(str, Enum):
    STATE = "state"
    APPEND = "append"
    LIST = "list"


class Operation(str, Enum):
    CREATE = "create"
    UPSERT = "upsert"
    PATCH = "patch"
    DELETE = "delete"
    SNAPSHOT = "snapshot"
    SUBSCRIBED = "subscribed"


class SortOrder(str, Enum):
    ASC = "asc"
    DESC = "desc"


@dataclass
class SortConfig:
    field: List[str]
    order: SortOrder

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "SortConfig":
        return cls(
            field=data["field"],
            order=SortOrder(data["order"]),
        )


@dataclass
class SubscribedFrame:
    op: str
    view: str
    mode: Mode
    sort: Optional[SortConfig] = None

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "SubscribedFrame":
        sort_data = data.get("sort")
        return cls(
            op=data["op"],
            view=data["view"],
            mode=Mode(data["mode"]),
            sort=SortConfig.from_dict(sort_data) if sort_data else None,
        )

    @classmethod
    def from_message(cls, data: Union[bytes, str]) -> "SubscribedFrame":
        parsed = parse_message(data)
        return cls.from_dict(parsed)

    @staticmethod
    def is_subscribed_frame(data: Dict[str, Any]) -> bool:
        return data.get("op") == "subscribed"


def try_parse_subscribed_frame(data: Union[bytes, str]) -> Optional["SubscribedFrame"]:
    """Try to parse a message as a SubscribedFrame. Returns None if not a subscribed frame."""
    try:
        parsed = parse_message(data)
        if SubscribedFrame.is_subscribed_frame(parsed):
            return SubscribedFrame.from_dict(parsed)
        return None
    except Exception:
        return None


class ConnectionState(str, Enum):
    DISCONNECTED = "disconnected"
    CONNECTING = "connecting"
    CONNECTED = "connected"
    ERROR = "error"
    RECONNECTING = "reconnecting"


def is_gzip(data: bytes) -> bool:
    return len(data) >= 2 and data[:2] == GZIP_MAGIC


def parse_message(data: Union[bytes, str]) -> Dict[str, Any]:
    if isinstance(data, bytes):
        if is_gzip(data):
            decompressed = gzip.decompress(data)
            return json.loads(decompressed.decode("utf-8"))
        data = data.decode("utf-8")

    return json.loads(data)


@dataclass
class Frame:
    mode: str
    entity: str
    op: str
    key: str
    data: Dict[str, Any]
    append: List[str] = field(default_factory=list)

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "Frame":
        return cls(
            mode=data["mode"],
            entity=data.get("export") or data.get("entity", ""),
            op=data["op"],
            key=data.get("key", ""),
            data=data.get("data", {}),
            append=data.get("append", []),
        )

    @classmethod
    def from_message(cls, data: Union[bytes, str]) -> "Frame":
        parsed = parse_message(data)
        return cls(
            mode=parsed["mode"],
            entity=parsed.get("export") or parsed.get("entity", ""),
            op=parsed["op"],
            key=parsed.get("key", ""),
            data=parsed.get("data", {}),
            append=parsed.get("append", []),
        )


@dataclass
class Subscription:
    view: str
    key: Optional[str] = None
    partition: Optional[str] = None

    def to_dict(self) -> Dict[str, Any]:
        result: Dict[str, Any] = {"type": "subscribe", "view": self.view}
        if self.key is not None:
            result["key"] = self.key
        if self.partition is not None:
            result["partition"] = self.partition
        return result

    def sub_key(self) -> str:
        return f"{self.view}:{self.key or '*'}"


@dataclass
class Unsubscription:
    view: str
    key: Optional[str] = None

    def to_dict(self) -> Dict[str, Any]:
        result: Dict[str, Any] = {"type": "unsubscribe", "view": self.view}
        if self.key is not None:
            result["key"] = self.key
        return result

    def sub_key(self) -> str:
        return f"{self.view}:{self.key or '*'}"
