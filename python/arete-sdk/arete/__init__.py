"""Arete Python SDK - Real-time data synchronization with authentication support."""

from arete.client import AreteClient
from arete.store import Store, Update, RichUpdate
from arete.types import (
    SortOrder,
    SortConfig,
    SubscribedFrame,
    SnapshotFrame,
    SnapshotEntity,
    try_parse_subscribed_frame,
    ConnectionState,
)
from arete.errors import SocketIssue
from arete.auth import (
    AuthConfig,
    AuthToken,
    AuthErrorCode,
    TokenProvider,
    TokenTransport,
)
from arete.errors import (
    AreteError,
    ConnectionError,
    SubscriptionError,
    ParseError,
    TimeoutError,
    AuthError,
)

__version__ = "0.1.0"

__all__ = [
    # Client
    "AreteClient",
    "Store",
    "Update",
    "RichUpdate",
    # Types
    "SortOrder",
    "SortConfig",
    "SubscribedFrame",
    "SnapshotFrame",
    "SnapshotEntity",
    "try_parse_subscribed_frame",
    "ConnectionState",
    "SocketIssue",
    # Auth
    "AuthConfig",
    "AuthToken",
    "AuthErrorCode",
    "TokenProvider",
    "TokenTransport",
    # Errors
    "AreteError",
    "ConnectionError",
    "SubscriptionError",
    "ParseError",
    "TimeoutError",
    "AuthError",
]
