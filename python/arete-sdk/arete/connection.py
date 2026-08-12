"""WebSocket protocol v2 connection manager.

Mirror of the WS half of ``typescript/core/src/connection.ts`` over the
``websockets`` library: connect/reconnect with backoff, JSON ping keepalive,
``refresh_auth`` scheduling via :mod:`arete.auth`, resubscription of active
leases with stable subscription ids, structured socket issues, frame dispatch,
and processed-slot tracking.
"""

from __future__ import annotations

import asyncio
import json
import logging
from dataclasses import dataclass
from typing import Any, Awaitable, Callable, Dict, List, Mapping, Optional, Set, Tuple

from arete.auth import (
    AuthConfig,
    AuthErrorCode,
    AuthState,
    TokenTransport,
    build_websocket_url,
    parse_error_code_from_close_reason,
    should_refresh_token,
)
from arete.errors import (
    AreteConnectionError,
    AreteError,
    ProcessedSlotTimeoutError,
    SubscriptionError,
)
from arete.subscription import Subscription
from arete.wire import (
    Frame,
    frame_slot,
    parse_frame,
    ping_envelope,
    refresh_auth_envelope,
    unsubscribe_envelope,
)

logger = logging.getLogger(__name__)

DEFAULT_RECONNECT_INTERVALS = (1.0, 2.0, 4.0, 8.0, 16.0)
DEFAULT_MAX_RECONNECT_ATTEMPTS = 5
DEFAULT_PING_INTERVAL_SECONDS = 15.0

CONNECTION_STATES = ("disconnected", "connecting", "connected", "reconnecting", "error")

_RATE_LIMIT_CODES = {
    AuthErrorCode.RATE_LIMIT_EXCEEDED,
    AuthErrorCode.CONNECTION_LIMIT_EXCEEDED,
}


@dataclass(frozen=True)
class SocketIssue:
    """Structured non-frame socket problem (v2 error envelope or legacy)."""

    error: str
    message: str
    code: str
    retryable: bool
    fatal: bool
    retry_after: Optional[float] = None
    suggested_action: Optional[str] = None
    docs_url: Optional[str] = None
    subscription_id: Optional[str] = None


def _is_refresh_auth_response(value: Any) -> bool:
    return (
        isinstance(value, dict)
        and isinstance(value.get("success"), bool)
        and "op" not in value
        and "entity" not in value
        and "mode" not in value
    )


def _is_socket_issue_message(value: Any) -> bool:
    return (
        isinstance(value, dict)
        and value.get("type") == "error"
        and isinstance(value.get("code"), str)
        and isinstance(value.get("fatal"), bool)
        and ("message" not in value or isinstance(value["message"], str))
        and ("error" not in value or isinstance(value["error"], str))
        and ("retryable" not in value or isinstance(value["retryable"], bool))
    )


async def _default_connect(url: str, headers: Optional[Mapping[str, str]]) -> Any:
    from websockets.asyncio.client import connect

    return await connect(url, additional_headers=dict(headers) if headers else None)


class ConnectionManager:
    """Protocol v2 WebSocket manager.

    Registered frame handlers (:meth:`on_frame`) receive every parsed
    :class:`arete.wire.Frame` — wire the internal store's ``handle_frame``
    and anything else that needs raw frames. ``connect_factory`` is
    injectable for tests: ``async (url, headers) -> websocket``.
    """

    def __init__(
        self,
        websocket_url: Optional[str],
        *,
        auth: Optional[AuthConfig] = None,
        auth_state: Optional[Any] = None,
        auto_reconnect: bool = True,
        reconnect_intervals: Optional[Tuple[float, ...]] = None,
        max_reconnect_attempts: int = DEFAULT_MAX_RECONNECT_ATTEMPTS,
        ping_interval_seconds: float = DEFAULT_PING_INTERVAL_SECONDS,
        connect_factory: Optional[
            Callable[[str, Optional[Mapping[str, str]]], Awaitable[Any]]
        ] = None,
    ) -> None:
        self._websocket_url = websocket_url or None
        if auth_state is not None:
            self._auth_state = auth_state
        elif auth is not None:
            if websocket_url is None:
                raise AreteError(
                    "Authentication requires a WebSocket URL", "INVALID_CONFIG"
                )
            self._auth_state = AuthState(websocket_url, auth)
        else:
            self._auth_state = None
        self._auto_reconnect = auto_reconnect
        self._reconnect_intervals = tuple(reconnect_intervals or DEFAULT_RECONNECT_INTERVALS)
        self._max_reconnect_attempts = max_reconnect_attempts
        self._ping_interval_seconds = ping_interval_seconds
        self._connect_factory = connect_factory or _default_connect

        self._state = "disconnected"
        self._ws: Optional[Any] = None
        self._run_task: Optional["asyncio.Task[None]"] = None
        self._closing = False
        self._reconnect_attempts = 0
        self._refresh_rotation_pending = False
        self._ever_connected = False

        self._connect_waiters: List["asyncio.Future[None]"] = []
        self._outbound: Optional["asyncio.Queue[Optional[str]]"] = None
        self._sender_task: Optional["asyncio.Task[None]"] = None
        self._ping_task: Optional["asyncio.Task[None]"] = None
        self._token_refresh_task: Optional["asyncio.Task[None]"] = None

        self._active: Dict[str, Subscription] = {}
        self._queued: Dict[str, Subscription] = {}

        self._frame_handlers: Set[Callable[[Frame], None]] = set()
        self._state_handlers: Set[Callable[[str, Optional[str]], None]] = set()
        self._issue_handlers: Set[Callable[[SocketIssue], None]] = set()

        self._processed_slot: Optional[int] = None
        self._slot_waiters: List[Tuple[int, "asyncio.Future[int]"]] = []

    # -- lifecycle ---------------------------------------------------------

    @property
    def connection_state(self) -> str:
        return self._state

    def is_connected(self) -> bool:
        return self._state == "connected" and self._ws is not None

    async def connect(self) -> None:
        """Connect (or wait for the in-flight attempt). Resolves on open."""
        self._require_websocket_url()
        if self.is_connected():
            return
        loop = asyncio.get_running_loop()
        waiter: "asyncio.Future[None]" = loop.create_future()
        self._connect_waiters.append(waiter)
        if self._run_task is None or self._run_task.done():
            self._closing = False
            self._run_task = asyncio.create_task(self._run())
        try:
            await waiter
        finally:
            if waiter in self._connect_waiters:
                self._connect_waiters.remove(waiter)

    async def disconnect(self) -> None:
        self._closing = True
        self._reject_connect_waiters(AreteConnectionError(
            "WebSocket connection attempt was cancelled", "CONNECTION_CANCELLED"
        ))
        task = self._run_task
        self._run_task = None
        ws = self._ws
        if ws is not None:
            try:
                await ws.close()
            except Exception:
                pass
        if task is not None and not task.done():
            task.cancel()
            try:
                await task
            except (asyncio.CancelledError, Exception):
                pass
        self._teardown_connection_tasks()
        self._ws = None
        self._set_state("disconnected")

    # -- observation hooks -------------------------------------------------

    def on_frame(self, handler: Callable[[Frame], None]) -> Callable[[], None]:
        self._frame_handlers.add(handler)
        return lambda: self._frame_handlers.discard(handler)

    def on_connection_state_change(
        self, handler: Callable[[str, Optional[str]], None]
    ) -> Callable[[], None]:
        self._state_handlers.add(handler)
        return lambda: self._state_handlers.discard(handler)

    def on_socket_issue(self, handler: Callable[[SocketIssue], None]) -> Callable[[], None]:
        self._issue_handlers.add(handler)
        return lambda: self._issue_handlers.discard(handler)

    # -- subscriptions -----------------------------------------------------

    def subscribe(self, subscription: Subscription) -> None:
        self._require_websocket_url()
        subscription_id = subscription.subscription_id
        existing = self._active.get(subscription_id) or self._queued.get(subscription_id)
        if existing is not None:
            if existing.to_wire() != subscription.to_wire():
                raise SubscriptionError(
                    f"subscriptionId '{subscription_id}' is already registered locally",
                    "DUPLICATE_SUBSCRIPTION_ID",
                )
            return

        if self.is_connected():
            self._active[subscription_id] = subscription
            self._send_json(subscription.to_wire())
        else:
            self._queued[subscription_id] = subscription

    def unsubscribe(self, subscription_id: str) -> None:
        self._queued.pop(subscription_id, None)
        if subscription_id in self._active:
            del self._active[subscription_id]
            if self.is_connected():
                self._send_json(unsubscribe_envelope(subscription_id))

    def refresh(self, subscription: Subscription) -> None:
        self._require_websocket_url()
        subscription_id = subscription.subscription_id
        existing = self._active.get(subscription_id) or self._queued.get(subscription_id)
        if existing is None:
            raise SubscriptionError(
                f"Cannot refresh inactive subscription '{subscription_id}'",
                "SUBSCRIPTION_NOT_FOUND",
            )
        if existing.to_wire() != subscription.to_wire():
            raise SubscriptionError(
                "Cannot change a subscription while refreshing it", "INVALID_SUBSCRIPTION"
            )
        self.unsubscribe(subscription_id)
        self.subscribe(existing)

    # -- processed slot cursor --------------------------------------------

    @property
    def processed_slot(self) -> Optional[int]:
        return self._processed_slot

    async def wait_for_processed_slot(
        self, slot: int, *, timeout: Optional[float] = None
    ) -> int:
        if isinstance(slot, bool) or not isinstance(slot, int) or slot < 0:
            raise ValueError("slot must be a non-negative integer")
        if self._processed_slot is not None and self._processed_slot >= slot:
            return self._processed_slot
        loop = asyncio.get_running_loop()
        waiter: "asyncio.Future[int]" = loop.create_future()
        entry = (slot, waiter)
        self._slot_waiters.append(entry)
        try:
            if timeout is None:
                return await waiter
            try:
                return await asyncio.wait_for(waiter, timeout)
            except asyncio.TimeoutError:
                raise ProcessedSlotTimeoutError(slot, self._processed_slot) from None
        finally:
            if entry in self._slot_waiters:
                self._slot_waiters.remove(entry)

    # -- internals: run loop ----------------------------------------------

    async def _run(self) -> None:
        recovering = False
        try:
            while not self._closing:
                self._set_state("reconnecting" if recovering else "connecting")

                try:
                    token = await self._resolve_token()
                except Exception as exc:
                    error = exc if isinstance(exc, AreteError) else AreteConnectionError(
                        f"Failed to get token: {exc}", "CONNECTION_ERROR", exc
                    )
                    if recovering and self._auto_reconnect:
                        self._set_state("reconnecting", str(error))
                        if await self._backoff_or_give_up():
                            continue
                        self._reject_connect_waiters(error)
                        return
                    self._set_state("error", str(error))
                    self._reject_connect_waiters(error)
                    return

                url = self._build_auth_url(token)
                headers = self._auth_headers(token)
                try:
                    ws = await self._connect_factory(url, headers)
                except Exception as exc:
                    error = AreteConnectionError(
                        "Failed to create WebSocket connection", "CONNECTION_ERROR", exc
                    )
                    if recovering and self._auto_reconnect:
                        self._set_state("reconnecting", str(error))
                        if await self._backoff_or_give_up():
                            continue
                        self._reject_connect_waiters(error)
                        return
                    self._set_state("error", str(error))
                    self._reject_connect_waiters(error)
                    return

                self._ws = ws
                self._reconnect_attempts = 0
                recovering = True
                self._ever_connected = True
                self._start_connection_tasks(ws)
                self._set_state("connected")
                self._resolve_connect_waiters()
                self._resubscribe_active()
                self._flush_subscription_queue()

                try:
                    async for message in ws:
                        self._handle_raw_message(message)
                except asyncio.CancelledError:
                    raise
                except Exception:
                    pass  # Connection closed; close code examined below.
                finally:
                    self._teardown_connection_tasks()
                    self._ws = None

                if self._closing:
                    return

                close_code = getattr(ws, "close_code", None)
                close_reason = getattr(ws, "close_reason", None) or ""

                if self._refresh_rotation_pending:
                    self._refresh_rotation_pending = False
                    if not self._auto_reconnect:
                        self._set_state(
                            "error",
                            "WebSocket closed for token refresh and automatic "
                            "reconnection is disabled",
                        )
                        return
                    continue  # Reconnect immediately with the fresh token.

                error_code = parse_error_code_from_close_reason(close_reason)
                if close_code == 1008 or error_code is not None:
                    if error_code is not None and should_refresh_token(error_code):
                        if self._auth_state is not None:
                            self._auth_state.clear_token()
                        if not self._auto_reconnect:
                            self._set_state(
                                "error",
                                "Authentication refresh requires reconnection, but "
                                f"automatic reconnection is disabled: {close_reason or close_code}",
                            )
                            return
                        continue  # Retry immediately with a fresh token.
                    if error_code in _RATE_LIMIT_CODES:
                        self._set_state("error", f"Rate limit exceeded: {close_reason}")
                        return

                if not self._auto_reconnect:
                    detail = (
                        f"{close_code}: {close_reason}" if close_reason else f"code {close_code}"
                    )
                    self._set_state(
                        "error",
                        f"WebSocket closed ({detail}) and automatic reconnection is disabled",
                    )
                    return
                if not await self._backoff_or_give_up():
                    return
        finally:
            if self._closing:
                self._set_state("disconnected")
            # Never leave a connect() call hanging when the loop exits.
            self._reject_connect_waiters(AreteConnectionError(
                "WebSocket connection attempt failed", "CONNECTION_ERROR"
            ))

    async def _backoff_or_give_up(self) -> bool:
        """Sleep the next backoff interval; False when attempts are exhausted."""
        if self._reconnect_attempts >= self._max_reconnect_attempts:
            self._set_state(
                "error", f"Max reconnection attempts ({self._reconnect_attempts}) reached"
            )
            return False
        self._set_state("reconnecting")
        index = min(self._reconnect_attempts, len(self._reconnect_intervals) - 1)
        self._reconnect_attempts += 1
        await asyncio.sleep(self._reconnect_intervals[index])
        return True

    # -- internals: per-connection tasks ----------------------------------

    def _start_connection_tasks(self, ws: Any) -> None:
        self._outbound = asyncio.Queue()
        self._sender_task = asyncio.create_task(self._sender_loop(ws, self._outbound))
        self._ping_task = asyncio.create_task(self._ping_loop())
        self._schedule_token_refresh()

    def _teardown_connection_tasks(self) -> None:
        for task in (self._sender_task, self._ping_task, self._token_refresh_task):
            if task is not None and not task.done():
                task.cancel()
        self._sender_task = None
        self._ping_task = None
        self._token_refresh_task = None
        self._outbound = None

    async def _sender_loop(self, ws: Any, queue: "asyncio.Queue[Optional[str]]") -> None:
        while True:
            payload = await queue.get()
            if payload is None:
                return
            try:
                await ws.send(payload)
            except Exception:
                return  # Socket is gone; reconnect will resubscribe active leases.

    async def _ping_loop(self) -> None:
        while True:
            await asyncio.sleep(self._ping_interval_seconds)
            self._send_json(ping_envelope())

    def _send_json(self, payload: Mapping[str, Any]) -> None:
        if self._outbound is not None:
            self._outbound.put_nowait(json.dumps(payload, separators=(",", ":")))

    def _resubscribe_active(self) -> None:
        for subscription in self._active.values():
            self._send_json(subscription.to_wire())

    def _flush_subscription_queue(self) -> None:
        queued = list(self._queued.values())
        self._queued.clear()
        for subscription in queued:
            self.subscribe(subscription)

    # -- internals: auth ---------------------------------------------------

    async def _resolve_token(self, force_refresh: bool = False) -> Optional[str]:
        if self._auth_state is None:
            return None
        return await self._auth_state.resolve_token(force_refresh)

    def _token_transport(self) -> TokenTransport:
        config = getattr(self._auth_state, "config", None)
        transport = getattr(config, "token_transport", None)
        return transport if isinstance(transport, TokenTransport) else TokenTransport.QUERY

    def _build_auth_url(self, token: Optional[str]) -> str:
        url = self._require_websocket_url()
        return build_websocket_url(url, token, self._token_transport())

    def _auth_headers(self, token: Optional[str]) -> Optional[Dict[str, str]]:
        if token and self._token_transport() == TokenTransport.BEARER:
            return {"Authorization": f"Bearer {token}"}
        return None

    def _schedule_token_refresh(self) -> None:
        if self._token_refresh_task is not None and not self._token_refresh_task.done():
            self._token_refresh_task.cancel()
            self._token_refresh_task = None
        if self._auth_state is None:
            return
        delay = self._auth_state.get_refresh_delay()
        if delay is None:
            return
        self._token_refresh_task = asyncio.create_task(self._refresh_token_after(delay))

    async def _refresh_token_after(self, delay: float) -> None:
        await asyncio.sleep(delay)
        try:
            token = await self._resolve_token(force_refresh=True)
            if token and self.is_connected():
                self._send_json(refresh_auth_envelope(token))
        except Exception as exc:
            logger.warning("Background token refresh failed: %s", exc)
        self._schedule_token_refresh()

    def _handle_refresh_auth_response(self, message: Mapping[str, Any]) -> None:
        if message.get("success"):
            self._schedule_token_refresh()
            return
        error = message.get("error")
        if self._auth_state is not None and isinstance(error, str) \
                and should_refresh_token(AuthErrorCode.from_wire(error)):
            self._auth_state.clear_token()
        self._rotate_connection_for_token_refresh()

    def _rotate_connection_for_token_refresh(self) -> None:
        ws = self._ws
        if ws is None or self._refresh_rotation_pending:
            return
        if not self._auto_reconnect:
            self._set_state(
                "error",
                "Token refresh requires a new connection, but automatic "
                "reconnection is disabled",
            )
            asyncio.ensure_future(self._close_socket(ws))
            return
        self._refresh_rotation_pending = True
        self._set_state("reconnecting")
        asyncio.ensure_future(self._close_socket(ws))

    @staticmethod
    async def _close_socket(ws: Any) -> None:
        try:
            await ws.close(1000, "token refresh")
        except Exception:
            pass

    # -- internals: inbound messages --------------------------------------

    def _handle_raw_message(self, message: Any) -> None:
        try:
            if isinstance(message, (bytes, bytearray, memoryview)):
                self._dispatch_frame(parse_frame(message))
                return
            parsed = json.loads(message)
            if _is_refresh_auth_response(parsed):
                self._handle_refresh_auth_response(parsed)
                return
            if _is_socket_issue_message(parsed):
                if parsed.get("protocolVersion") == 2:
                    frame = parse_frame(message)
                    self._dispatch_frame(frame)
                    if parsed.get("subscriptionId") is None or parsed["fatal"]:
                        self._handle_socket_issue(parsed)
                else:
                    self._handle_socket_issue(parsed)
                return
            self._dispatch_frame(parse_frame(message))
        except Exception:
            self._set_state("error", "Failed to parse frame from server")

    def _handle_socket_issue(self, message: Mapping[str, Any]) -> None:
        issue = SocketIssue(
            error=message.get("error") or message["code"],
            message=message.get("message") or message.get("error") or message["code"],
            code=message["code"],
            retryable=bool(message.get("retryable", False)),
            retry_after=message.get("retry_after"),
            suggested_action=message.get("suggested_action"),
            docs_url=message.get("docs_url"),
            fatal=message["fatal"],
            subscription_id=message.get("subscriptionId"),
        )
        for handler in list(self._issue_handlers):
            handler(issue)
        if issue.fatal:
            self._set_state("error", issue.message)

    def _dispatch_frame(self, frame: Frame) -> None:
        slot = frame_slot(frame)
        if slot is not None:
            self._note_processed_slot(slot)
        for handler in list(self._frame_handlers):
            handler(frame)

    def _note_processed_slot(self, slot: int) -> None:
        if self._processed_slot is not None and slot <= self._processed_slot:
            return
        self._processed_slot = slot
        for entry in list(self._slot_waiters):
            target, waiter = entry
            if slot >= target and not waiter.done():
                waiter.set_result(slot)

    # -- internals: state --------------------------------------------------

    def _require_websocket_url(self) -> str:
        if self._websocket_url is None:
            raise AreteError(
                "WebSocket transport is disabled (client was connected with "
                'transport: "http"); views and subscriptions are unavailable',
                "WEBSOCKET_DISABLED",
            )
        return self._websocket_url

    def _set_state(self, state: str, error: Optional[str] = None) -> None:
        self._state = state
        for handler in list(self._state_handlers):
            handler(state, error)

    def _resolve_connect_waiters(self) -> None:
        for waiter in self._connect_waiters:
            if not waiter.done():
                waiter.set_result(None)

    def _reject_connect_waiters(self, error: BaseException) -> None:
        for waiter in self._connect_waiters:
            if not waiter.done():
                waiter.set_exception(error)
