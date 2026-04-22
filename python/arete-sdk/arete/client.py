"""Arete client with authentication support."""

import asyncio
import json
import logging
from typing import Dict, List, Optional, Callable

from arete.websocket import WebSocketManager
from arete.store import Store, Mode
from arete.types import Subscription, Unsubscription, Frame, SnapshotFrame, SubscribedFrame
from arete.auth import AuthConfig
from arete.errors import SocketIssue

logger = logging.getLogger(__name__)


def parse_mode(view: str) -> Mode:
    """Parse view mode from view path."""
    if view.endswith("/state"):
        return Mode.STATE
    elif view.endswith("/list"):
        return Mode.LIST
    elif view.endswith("/append"):
        return Mode.APPEND
    else:
        return Mode.LIST  # Default to list mode


class AreteClient:
    """Arete WebSocket client with real-time data synchronization.

    Supports authentication via API keys (server-side), publishable keys (browser),
    static tokens, or custom token providers.

    Examples:
        # Server-side with any API key (secret or publishable)
        auth = AuthConfig.from_api_key("hspk_...")  # or "hssk_..."
        client = AreteClient("wss://demo.stack.arete.run", auth=auth)

        # Browser/client with publishable key only
        auth = AuthConfig(publishable_key="hspk_...")
        client = AreteClient("wss://demo.stack.arete.run", auth=auth)

        # Using static token
        auth = AuthConfig(token="static_token_here")
        client = AreteClient("wss://example.com", auth=auth)

        # Using async context manager
        async with AreteClient(url, auth=auth) as client:
            store = client.subscribe("Entity/list")
            # ... use store
    """

    def __init__(
        self,
        url: str,
        reconnect_intervals: Optional[List[int]] = None,
        ping_interval: int = 15,
        on_connect: Optional[Callable] = None,
        on_disconnect: Optional[Callable] = None,
        on_error: Optional[Callable] = None,
        on_socket_issue: Optional[Callable[["SocketIssue"], None]] = None,
        auth: Optional[AuthConfig] = None,
    ):
        """
        Initialize Arete client.

        Args:
            url: WebSocket server URL
            reconnect_intervals: List of wait intervals (in seconds) between reconnection attempts.
                Defaults to [1, 2, 4, 8, 16].
            ping_interval: Seconds between keep-alive ping messages. Defaults to 15.
            on_connect: Optional callback invoked when connection is established
            on_disconnect: Optional callback invoked when connection is closed
            on_error: Optional callback invoked when an error occurs
            on_socket_issue: Optional callback for structured socket issues from server
            auth: Optional authentication configuration. Required for hosted Arete URLs.
        """
        self.url = url
        self._stores: Dict[str, Store] = {}
        self._pending_subs: List[Subscription] = []
        self._user_on_connect = on_connect
        self._last_error: Optional[Exception] = None

        self.ws_manager = WebSocketManager(
            url=url,
            reconnect_intervals=reconnect_intervals,
            ping_interval=ping_interval,
            on_connect=self._on_connect,
            on_disconnect=on_disconnect,
            on_error=on_error,
            on_socket_issue=on_socket_issue,
            auth=auth,
        )
        self.ws_manager.set_message_handler(self._on_message)

    async def connect(self) -> None:
        """Connect to Arete server."""
        await self.ws_manager.connect()

    async def disconnect(self) -> None:
        """Disconnect from server."""
        await self.ws_manager.disconnect()

    async def __aenter__(self):
        await self.connect()
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb):
        await self.disconnect()

    def is_connected(self) -> bool:
        return self.ws_manager.is_running and self.ws_manager.ws is not None

    @property
    def last_error(self) -> Optional[Exception]:
        return self._last_error

    def clear_store(self) -> None:
        """Wipe all cached data. Use when switching user context."""
        for store in self._stores.values():
            store.clear()
        self._stores.clear()
        self._pending_subs.clear()

    def subscribe(
        self,
        view: str,
        key: Optional[str] = None,
        parser: Optional[Callable] = None,
        filters: Optional[Dict] = None,
        take: Optional[int] = None,
        skip: Optional[int] = None,
        with_snapshot: Optional[bool] = None,
        after: Optional[str] = None,
    ) -> Store:
        """
        Subscribe to updates for the specified view (and optional key) on the Arete server.

        Args:
            view: The view to subscribe to, in the format 'Entity/mode'.
            key: An optional key to filter the subscription to a specific entity or item.
            parser: An optional parser function to transform raw data into custom types.

        Returns:
            Store: A Store instance that provides access to real-time updates for the subscribed view.
        """
        if "/" not in view:
            raise ValueError(f"Invalid view '{view}'. Expected: Entity/mode")

        mode = parse_mode(view)
        store = Store(mode=mode, parser=parser, view=view)

        store_key = f"{view}:{key or '*'}"
        self._stores[store_key] = store

        sub = Subscription(
            view=view, key=key, filters=filters,
            take=take, skip=skip, with_snapshot=with_snapshot, after=after,
        )
        if self.ws_manager.is_running:
            asyncio.create_task(self._send_sub(sub))
        else:
            self._pending_subs.append(sub)

        return store

    async def _on_connect(self) -> None:
        """Send queued subscriptions on connect."""
        while self._pending_subs:
            await self._send_sub(self._pending_subs.pop(0))

        if self._user_on_connect:
            await self._user_on_connect()

    async def _send_sub(self, sub: Subscription) -> None:
        """Send subscription to server."""
        if not self.ws_manager.ws or not self.ws_manager.is_running:
            return

        try:
            await self.ws_manager.ws.send(json.dumps(sub.to_dict()))
            logger.info(f"Subscribed: {sub.view}")
        except Exception as e:
            logger.error(f"Subscribe failed: {e}")

    async def unsubscribe(self, view: str, key: Optional[str] = None) -> None:
        """Unsubscribe from a view."""
        store_key = f"{view}:{key or '*'}"
        self._stores.pop(store_key, None)

        if not self.ws_manager.ws or not self.ws_manager.is_running:
            return

        try:
            unsub = Unsubscription(view=view, key=key)
            await self.ws_manager.ws.send(json.dumps(unsub.to_dict()))
            logger.info(f"Unsubscribed: {view}")
        except Exception as e:
            logger.error(f"Unsubscribe failed: {e}")

    async def _on_message(self, message) -> None:
        try:
            from arete.types import parse_message
            parsed = parse_message(message)
            op = parsed.get("op", "")

            # subscribed confirmation
            if op == "subscribed":
                sub_frame = SubscribedFrame.from_dict(parsed)
                store = self._find_store_by_view(sub_frame.view)
                if store:
                    if sub_frame.sort:
                        store.set_sort_config(sub_frame.sort)
                    await store.handle_frame(
                        Frame(mode=sub_frame.mode.value, entity="", op="subscribed", key="", data={})
                    )
                return

            # batch snapshot
            if SnapshotFrame.is_snapshot_frame(parsed):
                snap = SnapshotFrame.from_dict(parsed)
                store = self._find_store_by_view(snap.view)
                if store:
                    await store.apply_snapshot(snap.entities, snap.complete)
                return

            # socket issue from server
            if op == "socket_issue":
                issue = SocketIssue.from_dict(parsed)
                logger.warning("Socket issue: %s — %s", issue.error, issue.message)
                if self.ws_manager.on_socket_issue:
                    self.ws_manager.on_socket_issue(issue)
                return

            # entity frame
            frame = Frame.from_dict(parsed)
            logger.debug("Frame: entity=%s op=%s key=%s", frame.entity, frame.op, frame.key)
            view = frame.entity
            for store_key in (f"{view}:{frame.key}", f"{view}:*"):
                store = self._stores.get(store_key)
                if store:
                    await store.handle_frame(frame)

        except Exception as e:
            self._last_error = e
            logger.error("Message error: %s", e, exc_info=True)

    def _find_store_by_view(self, view: str) -> Optional[Store]:
        """Find first store whose view matches (ignores key suffix)."""
        for store_key, store in self._stores.items():
            if store_key.startswith(f"{view}:"):
                return store
        return None
