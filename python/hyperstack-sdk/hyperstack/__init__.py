"""HyperStack Python SDK - Real-time data synchronization with authentication support."""

from hyperstack.client import HyperStackClient
from hyperstack.store import Store, Update
from hyperstack.types import (
    SortOrder,
    SortConfig,
    SubscribedFrame,
    try_parse_subscribed_frame,
    ConnectionState,
)
from hyperstack.auth import (
    AuthConfig,
    AuthToken,
    AuthErrorCode,
    TokenProvider,
    TokenTransport,
)
from hyperstack.errors import (
    HyperStackError,
    ConnectionError,
    SubscriptionError,
    ParseError,
    TimeoutError,
    AuthError,
)

__version__ = "0.1.0"

__all__ = [
    # Client
    "HyperStackClient",
    "Store",
    "Update",
    # Types
    "SortOrder",
    "SortConfig",
    "SubscribedFrame",
    "try_parse_subscribed_frame",
    "ConnectionState",
    # Auth
    "AuthConfig",
    "AuthToken",
    "AuthErrorCode",
    "TokenProvider",
    "TokenTransport",
    # Errors
    "HyperStackError",
    "ConnectionError",
    "SubscriptionError",
    "ParseError",
    "TimeoutError",
    "AuthError",
]
