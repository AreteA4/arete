"""HyperStack Python SDK - Real-time data synchronization with authentication support."""

from hyperstack.client import HyperStackClient
from hyperstack.store import Store, Update, SharedStore
from hyperstack.types import (
    Entity,
    StackDefinition,
    ViewDef,
    ViewGroup,
    state_view,
    list_view,
    SortOrder,
    SortConfig,
    SubscribedFrame,
    try_parse_subscribed_frame,
    ConnectionState,
)
from hyperstack.views import TypedViews
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
    "SharedStore",
    # Types
    "Entity",
    "StackDefinition",
    "ViewDef",
    "ViewGroup",
    "state_view",
    "list_view",
    "SortOrder",
    "SortConfig",
    "SubscribedFrame",
    "try_parse_subscribed_frame",
    "ConnectionState",
    "TypedViews",
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
